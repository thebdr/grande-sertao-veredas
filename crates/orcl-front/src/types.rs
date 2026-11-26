//! The program's types: each of rustc's types the program holds, as one of ORCL's, once each; anything else refused
//! by name where the program first holds it.
use orcl_interp::{Field, Int, Type, TypeId, Variant};
use rustc_public::{CrateDef, CrateDefType};
use rustc_public::mir::mono::Instance;
use rustc_public::ty::{AdtDef, AdtKind, FloatTy, GenericArgKind, GenericArgs, RigidTy, Span, Ty, TyConst, TyKind};

use crate::defs::{Defs, int_of, uint_of};
use crate::error::{Refusal, Refused};
use crate::lower::Lowerer;

/// A type as the interpreter tells it from others: a structure or an enum by its definition and its arguments, the
/// rest by their shape.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Bool,
    Int(Int),
    F32,
    F64,
    Duration,
    Never,
    Tuple(Vec<TypeId>),
    Array(TypeId, u32),
    Ref(TypeId, bool),
    Adt(AdtDef, Vec<ArgKey>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ArgKey {
    Ty(TypeId),
    Const(TyConst),
}

impl Lowerer<'_> {
    /// The type `ty` is in the program, lowered on first sight; `at` names where the program holds it, for a
    /// refusal.
    pub(crate) fn ty(&mut self, ty: Ty, at: Span) -> Result<TypeId, Refusal> {
        if let Some(id) = self.memo.get(&ty) {
            return Ok(*id);
        }
        let id = self.lower_ty(ty, at)?;
        self.memo.insert(ty, id);
        Ok(id)
    }

    pub(crate) fn get(&self, id: TypeId) -> Option<&Type> {
        self.types.get(id.0 as usize)
    }

    /// The slots a value of the type takes, as the interpreter's loader lays it out.
    pub(crate) fn slots_of(&self, id: TypeId) -> u32 {
        self.slots.get(id.0 as usize).copied().unwrap_or(0)
    }

    /// Refuses `usize` and `isize` where the program declares them: ORCL has neither (`docs/orcl/language`), the
    /// compiler's own temporaries (an index, a discriminant) excepted.
    pub(crate) fn no_word(&self, id: TypeId, at: Span) -> Result<TypeId, Refusal> {
        match self.get(id) {
            Some(Type::Int(Int::Usize)) => Err(self.refuse(Refused::Type, "usize", at)),
            Some(Type::Int(Int::Isize)) => Err(self.refuse(Refused::Type, "isize", at)),
            _ => Ok(id),
        }
    }

    /// The name a type has in the program: a structure's or an enum's own, with its arguments.
    pub(crate) fn name_of(&self, id: TypeId) -> String {
        match self.get(id) {
            Some(Type::Bool) => "bool".into(),
            Some(Type::Int(t)) => match t {
                Int::I8 => "i8",
                Int::I16 => "i16",
                Int::I32 => "i32",
                Int::I64 => "i64",
                Int::Isize => "isize",
                Int::U8 => "u8",
                Int::U16 => "u16",
                Int::U32 => "u32",
                Int::U64 => "u64",
                Int::Usize => "usize",
            }
            .into(),
            Some(Type::F32) => "f32".into(),
            Some(Type::F64) => "f64".into(),
            Some(Type::Duration) => "Duration".into(),
            Some(Type::Struct { name, .. } | Type::Enum { name, .. }) => name.clone(),
            Some(Type::Array { element, len }) => format!("[{}; {len}]", self.name_of(*element)),
            Some(Type::Ref { to, mutable: true }) => format!("&mut {}", self.name_of(*to)),
            Some(Type::Ref { to, mutable: false }) => format!("&{}", self.name_of(*to)),
            None => "?".into(),
        }
    }

    fn intern(&mut self, key: Key, ty: Type, slots: u32) -> TypeId {
        if let Some(id) = self.keys.get(&key) {
            return *id;
        }
        let id = TypeId(u32::try_from(self.types.len()).unwrap_or(u32::MAX));
        self.types.push(ty);
        self.slots.push(slots);
        self.keys.insert(key, id);
        id
    }

    fn lower_ty(&mut self, ty: Ty, at: Span) -> Result<TypeId, Refusal> {
        let refused = |s: &Self| s.refuse(Refused::Type, format!("{ty}"), at);
        let TyKind::RigidTy(rigid) = ty.kind() else { return Err(refused(self)) };
        match rigid {
            RigidTy::Bool => Ok(self.intern(Key::Bool, Type::Bool, 1)),
            RigidTy::Int(t) => match int_of(t) {
                Some(t) => Ok(self.intern(Key::Int(t), Type::Int(t), 1)),
                None => Err(refused(self)),
            },
            RigidTy::Uint(t) => match uint_of(t) {
                Some(t) => Ok(self.intern(Key::Int(t), Type::Int(t), 1)),
                None => Err(refused(self)),
            },
            RigidTy::Float(FloatTy::F32) => Ok(self.intern(Key::F32, Type::F32, 1)),
            RigidTy::Float(FloatTy::F64) => Ok(self.intern(Key::F64, Type::F64, 1)),
            // `!`, the type of a loop's value that never comes: an enum of no variant, which nothing can build
            RigidTy::Never => Ok(self.intern(
                Key::Never,
                Type::Enum { name: "!".into(), discriminant: Int::Isize, variants: Vec::new() },
                1,
            )),
            RigidTy::Tuple(parts) => {
                let mut ids = Vec::with_capacity(parts.len());
                for part in parts {
                    ids.push(self.ty(part, at)?);
                }
                let names: Vec<String> = ids.iter().map(|id| self.name_of(*id)).collect();
                let name = match names.as_slice() {
                    [one] => format!("({one},)"),
                    _ => format!("({})", names.join(", ")),
                };
                let fields: Vec<Field> =
                    ids.iter().enumerate().map(|(i, id)| Field { name: i.to_string(), ty: *id }).collect();
                let slots = ids.iter().fold(0u32, |n, id| n.saturating_add(self.slots_of(*id)));
                Ok(self.intern(Key::Tuple(ids), Type::Struct { name, fields }, slots))
            }
            RigidTy::Array(element, len) => {
                let len = len.eval_target_usize().ok().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| refused(self))?;
                let element = self.ty(element, at)?;
                self.no_word(element, at)?;
                let slots = self.slots_of(element).saturating_mul(len);
                Ok(self.intern(Key::Array(element, len), Type::Array { element, len }, slots))
            }
            RigidTy::Ref(_, to, mutability) => {
                let to = self.ty(to, at)?;
                let mutable = mutability == rustc_public::mir::Mutability::Mut;
                Ok(self.intern(Key::Ref(to, mutable), Type::Ref { to, mutable }, 1))
            }
            RigidTy::Adt(def, args) => self.adt(ty, def, &args, at),
            _ => Err(refused(self)),
        }
    }

    fn adt(&mut self, ty: Ty, def: AdtDef, args: &GenericArgs, at: Span) -> Result<TypeId, Refusal> {
        if self.defs.is_duration(def) {
            return Ok(self.intern(Key::Duration, Type::Duration, 2));
        }
        let option = self.defs.is_option(def);
        if !option && !Defs::program_crate(&def.krate()) || def.kind() == AdtKind::Union {
            return Err(self.refuse(Refused::Type, format!("{ty}"), at));
        }
        if !Instance::resolve_drop_in_place(ty).is_empty_shim() {
            return Err(self.refuse(Refused::DropGlue, format!("{ty}"), at));
        }
        let mut keys = Vec::new();
        let mut names = Vec::new();
        for arg in &args.0 {
            match arg {
                GenericArgKind::Type(t) => {
                    let id = self.ty(*t, at)?;
                    names.push(self.name_of(id));
                    keys.push(ArgKey::Ty(id));
                }
                GenericArgKind::Const(c) => {
                    names.push(format!("{}", c.eval_target_usize().unwrap_or(0)));
                    keys.push(ArgKey::Const(c.clone()));
                }
                GenericArgKind::Lifetime(_) => {}
            }
        }
        let key = Key::Adt(def, keys);
        if let Some(id) = self.keys.get(&key) {
            return Ok(*id);
        }
        let base = if option { "Option".to_string() } else { def.trimmed_name() };
        let name = if option || names.is_empty() { base } else { format!("{base}<{}>", names.join(", ")) };
        // the type is numbered before its fields are lowered, so a field that refers back to it finds it
        let id = self.intern(key, Type::Struct { name: name.clone(), fields: Vec::new() }, 0);
        let mut variants = Vec::new();
        let mut largest = 0u32;
        for v in def.variants_iter() {
            let mut fields = Vec::new();
            let mut size = 0u32;
            for f in v.fields() {
                // a field's type is refused where the field is declared
                let fid = self.ty(f.ty_with_args(args), f.span())?;
                self.no_word(fid, f.span())?;
                size = size.saturating_add(self.slots_of(fid));
                fields.push(Field { name: f.name.clone(), ty: fid });
            }
            largest = largest.max(size);
            variants.push((v, fields, size));
        }
        let (lowered, slots) = match def.kind() {
            AdtKind::Struct => {
                let fields = variants.into_iter().next().map(|(_, f, _)| f).unwrap_or_default();
                (Type::Struct { name, fields }, largest)
            }
            _ => {
                let discriminant = ty.kind().discriminant_ty().ok_or_else(|| self.refuse(Refused::Type, format!("{ty}"), at))?;
                let discriminant = self.ty(discriminant, at)?;
                let Some(Type::Int(d)) = self.get(discriminant).cloned() else {
                    return Err(self.refuse(Refused::Type, format!("{ty}"), at));
                };
                let mut out = Vec::with_capacity(variants.len());
                let mut size = 1u32;
                for (v, fields, s) in variants {
                    let value = def.discriminant_for_variant(v.idx()).val;
                    out.push(Variant { name: v.name(), discriminant: d.wrap(value as i128), fields });
                    size = size.max(s.saturating_add(1));
                }
                (Type::Enum { name, discriminant: d, variants: out }, size)
            }
        };
        if let Some(t) = self.types.get_mut(id.0 as usize) {
            *t = lowered;
        }
        if let Some(s) = self.slots.get_mut(id.0 as usize) {
            *s = slots;
        }
        Ok(id)
    }
}
