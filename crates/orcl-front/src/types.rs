//! The program's types: each of rustc's types the program holds, as one of ORCL's, once each; anything else refused
//! by name where the program first holds it.
use std::collections::HashSet;

use orcl_interp::{Field, Int, Type, TypeId, Variant};
use rustc_public::{CrateDef, CrateDefType};
use rustc_public::mir::mono::Instance;
use rustc_public::ty::{AdtDef, AdtKind, FloatTy, GenericArgKind, GenericArgs, RigidTy, Span, Ty, TyConst, TyConstKind, TyKind};

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

    /// Refuses `usize` and `isize` where the program declares them, wherever they stand in the type it declares: ORCL
    /// has neither (`docs/orcl/language`), the compiler's own temporaries (an index, a discriminant) excepted. The
    /// refusal names the innermost type, `usize` or `isize`, at `at`, the program's declaration.
    pub(crate) fn no_word(&self, id: TypeId, at: Span) -> Result<TypeId, Refusal> {
        match self.word_in(id) {
            Some(word) => Err(self.refuse(Refused::Type, word, at)),
            None => Ok(id),
        }
    }

    /// The first `usize` or `isize` a type holds, in its fields' declared order: the type itself, a structure's or a
    /// tuple's field, a variant's payload (an enum's discriminant is the compiler's), an array's element, a
    /// reference's pointee. A type still being lowered holds none yet; its fields are checked as they are lowered.
    fn word_in(&self, id: TypeId) -> Option<&'static str> {
        let mut seen = HashSet::new();
        let mut todo = vec![id];
        while let Some(id) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            match self.get(id) {
                Some(Type::Int(Int::Usize)) => return Some("usize"),
                Some(Type::Int(Int::Isize)) => return Some("isize"),
                Some(Type::Struct { fields, .. }) => todo.extend(fields.iter().rev().map(|f| f.ty)),
                Some(Type::Enum { variants, .. }) => {
                    todo.extend(variants.iter().rev().flat_map(|v| v.fields.iter().rev().map(|f| f.ty)));
                }
                Some(Type::Array { element, .. }) => todo.push(*element),
                Some(Type::Ref { to, .. }) => todo.push(*to),
                _ => {}
            }
        }
        None
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
            // an `Option` keeps the name the interpreter looks for, and shows its payload where it is displayed
            Some(Type::Enum { name, variants, .. }) if name == "Option" => {
                match variants.get(1).and_then(|some| some.fields.first()) {
                    Some(payload) => format!("Option<{}>", self.name_of(payload.ty)),
                    None => name.clone(),
                }
            }
            Some(Type::Struct { name, .. } | Type::Enum { name, .. }) => name.clone(),
            Some(Type::Array { element, len }) => format!("[{}; {len}]", self.name_of(*element)),
            Some(Type::Ref { to, mutable: true }) => format!("&mut {}", self.name_of(*to)),
            Some(Type::Ref { to, mutable: false }) => format!("&{}", self.name_of(*to)),
            None => "?".into(),
        }
    }

    /// A structure's or an enum's name: `base`, then its arguments' names (`Avg<4>`, `Hold<Option<u8>>`).
    pub(crate) fn adt_name(&self, base: &str, args: &[ArgKey]) -> String {
        let names: Vec<String> = args
            .iter()
            .map(|a| match a {
                ArgKey::Ty(id) => self.name_of(*id),
                ArgKey::Const(c) => const_name(c),
            })
            .collect();
        if names.is_empty() { base.to_string() } else { format!("{base}<{}>", names.join(", ")) }
    }

    /// A tuple's name, its parts' names in their order: `(u8,)` for one part.
    pub(crate) fn tuple_name(&self, parts: &[TypeId]) -> String {
        let names: Vec<String> = parts.iter().map(|id| self.name_of(*id)).collect();
        match names.as_slice() {
            [one] => format!("({one},)"),
            _ => format!("({})", names.join(", ")),
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
                let name = self.tuple_name(&ids);
                let fields: Vec<Field> =
                    ids.iter().enumerate().map(|(i, id)| Field { name: i.to_string(), ty: *id }).collect();
                let slots = ids.iter().fold(0u32, |n, id| n.saturating_add(self.slots_of(*id)));
                Ok(self.intern(Key::Tuple(ids), Type::Struct { name, fields }, slots))
            }
            RigidTy::Array(element, len) => {
                let len = len.eval_target_usize().ok().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| refused(self))?;
                // an element's `usize` is refused where the program declares the array, as any other `usize` it holds
                let element = self.ty(element, at)?;
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
        for arg in &args.0 {
            match arg {
                GenericArgKind::Type(t) => keys.push(ArgKey::Ty(self.ty(*t, at)?)),
                GenericArgKind::Const(c) => keys.push(ArgKey::Const(c.clone())),
                GenericArgKind::Lifetime(_) => {}
            }
        }
        // its trimmed name: once the program is lowered, `name_apart` names it by its path if another item the file
        // holds has that name (`crate::names`)
        let name = if option { "Option".to_string() } else { self.adt_name(&def.trimmed_name(), &keys) };
        let key = Key::Adt(def, keys);
        if let Some(id) = self.keys.get(&key) {
            return Ok(*id);
        }
        // the type is numbered before its fields are lowered, so a field that refers back to it finds it
        let id = self.intern(key, Type::Struct { name: name.clone(), fields: Vec::new() }, 0);
        let mut variants = Vec::new();
        let mut largest = 0u32;
        for v in def.variants_iter() {
            let mut fields = Vec::new();
            let mut size = 0u32;
            for f in v.fields() {
                // a field's type is refused where the field is declared; `Option`'s field is core's, its payload
                // refused where the program declares the `Option`
                let fid = self.ty(f.ty_with_args(args), f.span())?;
                if !option {
                    self.no_word(fid, f.span())?;
                }
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

/// A const generic argument's value as a type's name shows it: a `bool`, a signed or an unsigned integer read from its
/// bytes by its type, a `usize` evaluated, `?` for what is none of these.
pub(crate) fn const_name(c: &TyConst) -> String {
    if let TyConstKind::Value(ty, alloc) = c.kind() {
        let value = match ty.kind() {
            TyKind::RigidTy(RigidTy::Bool) => alloc.read_bool().map(|b| b.to_string()).ok(),
            // read as unsigned and sign-extended from the constant's own width: `read_int` gives `-1000` as
            // `4294966296` for an `i32`
            TyKind::RigidTy(RigidTy::Int(_)) => alloc.read_uint().map(|v| sign_extended(v, alloc.bytes.len()).to_string()).ok(),
            TyKind::RigidTy(RigidTy::Uint(_)) => alloc.read_uint().map(|v| v.to_string()).ok(),
            _ => None,
        };
        if let Some(value) = value {
            return value;
        }
    }
    c.eval_target_usize().map_or_else(|_| "?".to_string(), |v| v.to_string())
}

/// `v`, the bytes of a signed integer `bytes` wide read as unsigned, as the signed value they hold.
fn sign_extended(v: u128, bytes: usize) -> i128 {
    let bits = bytes.saturating_mul(8);
    if bits == 0 || bits >= 128 {
        return v as i128;
    }
    let shift = 128u32.saturating_sub(u32::try_from(bits).unwrap_or(128));
    ((v << shift) as i128) >> shift
}
