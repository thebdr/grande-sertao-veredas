//! The names the program's file gives its structures, enums and functions, made once the program is lowered: the
//! source's own, trimmed as `rustc_public` trims them (`Timer`, `Avg<4>`, `Ton::scan`, `<Timer as Default>::default`),
//! unless two distinct items the file holds would share one. Then each of those is named by its path from its crate's
//! root (`fast::Timer` and `slow::Timer`, `fast::step` and `slow::step`; `orcl-std`'s `timers::Ton` beside a program's
//! own `Ton`), and by its crate's name and that path where two paths still meet; a function shows the types it names
//! as the file names them, inside arrays, tuples and references too (`<fast::Timer as Default>::default`,
//! `fast::Timer::bump`, `pass::<fast::Timer>`, `tag::<[fast::Timer; 2]>`). The types a function's name shows count
//! among the items whose names meet though the file holds no value of them, so two instances of a generic at marker
//! types of one name are named apart by the markers' paths (`rate::<a::Marker>`, `rate::<b::Marker>`). Core's
//! `Option` and `Duration` keep their names, `Option` the one the interpreter looks for: a structure or an enum of the
//! program's crates named `Option` is always named by its path (`mine::Option`; one at the crate's root by the
//! crate's name, `edge::Option`), and one named `Duration` too where the file holds core's or a name shows it.
use std::collections::{HashMap, HashSet};

use orcl_interp::{Type, TypeId};
use rustc_middle::ty::print::ForceTrimmedGuard;
use rustc_middle::ty::{self as rty, GenericArg, GenericArgKind, TypingEnv};
use rustc_public::CrateDef;
use rustc_public::mir::mono::Instance;
use rustc_public::rustc_internal;
use rustc_public::ty::{RigidTy, TyKind};
use rustc_span::def_id::DefId;

use crate::defs::Defs;
use crate::lower::{Lowerer, entry_names};
use crate::types::{Key, const_name};

/// The most a name is qualified: its crate's name, then its path from the crate's root. One step less is the path
/// alone, none the trimmed name.
const CRATE: u8 = 2;

/// An item a structure's or an enum's name stands for: a definition of the program's crates, one of core's two whose
/// names are fixed, or another of core's a function's name shows (`PhantomData`), by its trimmed name.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Item {
    Adt(DefId),
    Fixed(&'static str),
    Foreign(DefId),
}

/// What a function's name shows of its parent: nothing (a free function), its inherent impl's own type, or the trait
/// it belongs to at its types.
enum Shape<'tcx> {
    Free,
    Inherent(rty::Ty<'tcx>),
    Trait(rty::TraitRef<'tcx>),
}

impl<'tcx> Lowerer<'tcx> {
    /// Names apart what the file holds, everything lowered: the structures and enums, then the functions, the
    /// entries by `state`'s name.
    pub(crate) fn name_apart(&mut self, state: TypeId) {
        let levels = self.type_levels();
        if !levels.is_empty() {
            self.rename_types(&levels);
        }
        self.rename_functions(state, &levels);
    }

    /// A definition's name, trimmed (`level` 0), by its path from its crate's root (1) or by its crate's name and
    /// that path (2).
    fn qualified(&self, def: DefId, level: u8) -> String {
        let tcx = self.defs.tcx;
        if level == 0 {
            return rustc_internal::stable(def).trimmed_name();
        }
        let path: Vec<String> =
            tcx.def_path(def).data.iter().filter_map(|d| d.data.get_opt_name()).map(|s| s.to_string()).collect();
        let path = path.join("::");
        if level >= CRATE { format!("{}::{path}", tcx.crate_name(def.krate)) } else { path }
    }

    /// The structures and enums named past their trimmed names, and how far: each of two distinct items whose names
    /// meet, the file's own and those a function's name shows (a marker type, an array's element), core's `Option`
    /// counted always (the interpreter knows it by that name) and core's `Duration` where the file holds it or a name
    /// shows it, a step further until no two meet.
    fn type_levels(&self) -> HashMap<DefId, u8> {
        let mut items = HashSet::from([Item::Fixed("Option")]);
        for key in self.keys.keys() {
            match key {
                Key::Duration => {
                    items.insert(Item::Fixed("Duration"));
                }
                Key::Adt(def, _) if !self.defs.is_option(*def) => {
                    items.insert(Item::Adt(rustc_internal::internal(self.defs.tcx, def.def_id())));
                }
                _ => {}
            }
        }
        let tcx = self.defs.tcx;
        for instance in self.instances.keys() {
            let Some((own_args, shape)) = self.shown(rustc_internal::internal(tcx, *instance)) else { continue };
            for arg in shown_args(own_args, &shape).iter().flat_map(|a| a.walk()) {
                if let GenericArgKind::Type(t) = arg.kind()
                    && let TyKind::RigidTy(RigidTy::Adt(def, _)) = rustc_internal::stable(t).kind()
                {
                    items.insert(if self.defs.is_option(def) {
                        Item::Fixed("Option")
                    } else if self.defs.is_duration(def) {
                        Item::Fixed("Duration")
                    } else if Defs::program_crate(&def.krate()) {
                        Item::Adt(rustc_internal::internal(tcx, def.def_id()))
                    } else {
                        Item::Foreign(rustc_internal::internal(tcx, def.def_id()))
                    });
                }
            }
        }
        let mut levels: HashMap<DefId, u8> = HashMap::new();
        loop {
            let mut named: HashMap<String, Vec<Item>> = HashMap::new();
            for item in &items {
                let name = match item {
                    Item::Adt(def) => self.qualified(*def, levels.get(def).copied().unwrap_or(0)),
                    Item::Fixed(name) => (*name).to_string(),
                    Item::Foreign(def) => self.qualified(*def, 0),
                };
                named.entry(name).or_default().push(*item);
            }
            let mut further = false;
            for item in named.values().filter(|meet| meet.len() > 1).flatten() {
                if let Item::Adt(def) = item {
                    let level = levels.entry(*def).or_insert(0);
                    if *level < CRATE {
                        *level += 1;
                        further = true;
                    }
                }
            }
            if !further {
                return levels;
            }
        }
    }

    /// Every structure's, enum's and tuple's name made again by `levels`, in the order the types were numbered: a
    /// type's arguments and a tuple's parts are numbered before it, so their names are already made.
    fn rename_types(&mut self, levels: &HashMap<DefId, u8>) {
        let made: HashMap<TypeId, Key> = self.keys.iter().map(|(key, id)| (*id, key.clone())).collect();
        for n in 0..self.types.len() {
            let id = TypeId(u32::try_from(n).unwrap_or(u32::MAX));
            let name = match made.get(&id) {
                Some(Key::Adt(def, args)) if !self.defs.is_option(*def) => {
                    let def = rustc_internal::internal(self.defs.tcx, def.def_id());
                    self.adt_name(&self.qualified(def, levels.get(&def).copied().unwrap_or(0)), args)
                }
                Some(Key::Tuple(parts)) => self.tuple_name(parts),
                _ => continue,
            };
            if let Some(Type::Struct { name: old, .. } | Type::Enum { name: old, .. }) = self.types.get_mut(n) {
                *old = name;
            }
        }
    }

    /// Every function's name: the entries' made from the state's; any other's kept as `rustc_public` trimmed it,
    /// unless it shows a type named past its trimmed name, or two distinct functions' names meet, each of those then
    /// made with its own path (a free function's, its impl's own type's and its trait's, where they are the program's)
    /// a step further until no two meet.
    fn rename_functions(&mut self, state: TypeId, levels: &HashMap<DefId, u8>) {
        let tcx = self.defs.tcx;
        let count = self.functions.len();
        let mut instances: Vec<Option<Instance>> = vec![None; count];
        for (instance, id) in &self.instances {
            if let Some(slot) = instances.get_mut(id.0 as usize) {
                *slot = Some(*instance);
            }
        }
        let defs: Vec<Option<DefId>> =
            instances.iter().map(|i| i.map(|i| rustc_internal::internal(tcx, i).def_id())).collect();
        let entries = entry_names(&self.name_of(state));
        let mut own = vec![0u8; count];
        loop {
            let names: Vec<Option<String>> = (0..count)
                .map(|n| match entries.get(n) {
                    Some(entry) => Some(entry.clone()),
                    None => {
                        let instance = instances.get(n).copied().flatten()?;
                        self.function_name(instance, own.get(n).copied().unwrap_or(0), levels)
                    }
                })
                .collect();
            let mut meeting: HashMap<&str, Vec<usize>> = HashMap::new();
            for (n, made) in names.iter().enumerate() {
                let old = self.functions.get(n).and_then(|f| f.as_ref()).map(|f| f.name.as_str());
                if let Some(name) = made.as_deref().or(old) {
                    meeting.entry(name).or_default().push(n);
                }
            }
            let mut further = false;
            for meet in meeting.values() {
                let distinct: HashSet<Option<DefId>> = meet.iter().map(|n| defs.get(*n).copied().flatten()).collect();
                if distinct.len() < 2 {
                    continue;
                }
                // the entries keep their names: the others are named apart from them
                for n in meet.iter().filter(|n| **n >= entries.len()) {
                    if let Some(level) = own.get_mut(*n)
                        && *level < CRATE
                    {
                        *level += 1;
                        further = true;
                    }
                }
            }
            if !further {
                for (function, name) in self.functions.iter_mut().zip(names) {
                    if let (Some(function), Some(name)) = (function, name) {
                        function.name = name;
                    }
                }
                return;
            }
        }
    }

    /// A function's name made, its own path qualified to `own` and the types it shows as the file names them; `None`
    /// when it shows no type named past its trimmed name and `own` is 0, so its trimmed name stands.
    fn function_name(&self, instance: Instance, own: u8, levels: &HashMap<DefId, u8>) -> Option<String> {
        let tcx = self.defs.tcx;
        let instance: rty::Instance<'tcx> = rustc_internal::internal(tcx, instance);
        let def = instance.def_id();
        let (own_args, shape) = self.shown(instance)?;
        let shown = shown_args(own_args, &shape);
        let renamed = shown.iter().flat_map(|a| a.walk()).any(|a| match a.kind() {
            GenericArgKind::Type(t) => matches!(t.kind(), rty::Adt(adt, _) if levels.contains_key(&adt.did())),
            _ => false,
        });
        if own == 0 && !renamed {
            return None;
        }
        let name = tcx.item_name(def);
        let turbofish = self.args_named(own_args, levels).map(|a| format!("::<{a}>")).unwrap_or_default();
        Some(match shape {
            Shape::Free => format!("{}{turbofish}", self.qualified(def, own)),
            Shape::Inherent(ty) => format!("{}::{name}{turbofish}", self.ty_named(ty, levels, own, true)),
            Shape::Trait(tr) => {
                let trait_args = tcx.generics_of(tr.def_id).own_args_no_defaults(tcx, tr.args.as_slice());
                let trait_args = self.args_named(trait_args, levels).map(|a| format!("<{a}>")).unwrap_or_default();
                let at = if self.defs.program_crate_num(tr.def_id.krate) { own } else { 0 };
                let this = self.ty_named(tr.self_ty(), levels, own, false);
                format!("<{this} as {}{trait_args}>::{name}{turbofish}", self.qualified(tr.def_id, at))
            }
        })
    }

    /// What a function's name shows besides its own path: its own generic arguments (its defaults aside) and its
    /// parent's shape at the instance's types; `None` where those types cannot be made.
    fn shown(&self, instance: rty::Instance<'tcx>) -> Option<(&'tcx [GenericArg<'tcx>], Shape<'tcx>)> {
        let tcx = self.defs.tcx;
        let def = instance.def_id();
        let args = instance.args.as_tcx_slice();
        let env = TypingEnv::fully_monomorphized();
        let parents = tcx.mk_args(args.get(..tcx.generics_of(def).parent_count)?);
        let shape = if let Some(imp) = tcx.trait_impl_of_assoc(def) {
            Shape::Trait(tcx.try_instantiate_and_normalize_erasing_regions(parents, env, tcx.impl_trait_ref(imp)).ok()?)
        } else if let Some(imp) = tcx.inherent_impl_of_assoc(def) {
            Shape::Inherent(tcx.try_instantiate_and_normalize_erasing_regions(parents, env, tcx.type_of(imp)).ok()?)
        } else if let Some(tr) = tcx.trait_of_assoc(def) {
            Shape::Trait(rty::TraitRef::new_from_args(tcx, tr, parents))
        } else {
            Shape::Free
        };
        Some((tcx.generics_of(def).own_args_no_defaults(tcx, args), shape))
    }

    /// Generic arguments as a name shows them, its lifetimes aside; `None` for none.
    fn args_named(&self, args: &[GenericArg<'tcx>], levels: &HashMap<DefId, u8>) -> Option<String> {
        let names: Vec<String> = args
            .iter()
            .filter_map(|a| match a.kind() {
                GenericArgKind::Type(t) => Some(self.ty_named(t, levels, 0, false)),
                GenericArgKind::Const(c) => Some(const_name(&rustc_internal::stable(c))),
                GenericArgKind::Lifetime(_) => None,
            })
            .collect();
        (!names.is_empty()).then(|| names.join(", "))
    }

    /// A type as a function's name shows it: as the file names it, a structure or an enum of the program's crates
    /// qualified at least to `floor`, its arguments after `::` where it heads a path (`Avg::<4>::push`); a type the
    /// file does not hold made of its parts as the file names them (`[fast::Timer; 2]`, `(slow::Timer, u8)`,
    /// `&fast::Timer`, a slice, a raw pointer, `PhantomData<a::Marker>`), core's structures and enums by their trimmed
    /// names (none heads an inherent impl of the program's crates); anything else as `rustc_public` trims it.
    fn ty_named(&self, ty: rty::Ty<'tcx>, levels: &HashMap<DefId, u8>, floor: u8, head: bool) -> String {
        let tcx = self.defs.tcx;
        if let rty::Adt(adt, args) = ty.kind()
            && self.defs.program_crate_num(adt.did().krate)
        {
            let base = self.qualified(adt.did(), levels.get(&adt.did()).copied().unwrap_or(0).max(floor));
            return match self.args_named(args.as_slice(), levels) {
                Some(args) if head => format!("{base}::<{args}>"),
                Some(args) => format!("{base}<{args}>"),
                None => base,
            };
        }
        if let Some(id) = self.memo.get(&rustc_internal::stable(ty)) {
            return self.name_of(*id);
        }
        match ty.kind() {
            rty::Array(element, len) => {
                let len = len.try_to_target_usize(tcx).map_or_else(|| len.to_string(), |n| n.to_string());
                format!("[{}; {len}]", self.ty_named(*element, levels, 0, false))
            }
            rty::Slice(element) => format!("[{}]", self.ty_named(*element, levels, 0, false)),
            rty::Ref(_, to, mutability) => {
                format!("&{}{}", mutability.prefix_str(), self.ty_named(*to, levels, 0, false))
            }
            rty::RawPtr(to, mutability) => {
                format!("*{} {}", mutability.ptr_str(), self.ty_named(*to, levels, 0, false))
            }
            rty::Tuple(parts) => {
                let names: Vec<String> = parts.iter().map(|t| self.ty_named(t, levels, 0, false)).collect();
                match names.as_slice() {
                    [one] => format!("({one},)"),
                    _ => format!("({})", names.join(", ")),
                }
            }
            rty::Adt(adt, args) => {
                let base = self.qualified(adt.did(), 0);
                match self.args_named(tcx.generics_of(adt.did()).own_args_no_defaults(tcx, args.as_slice()), levels) {
                    Some(args) => format!("{base}<{args}>"),
                    None => base,
                }
            }
            _ => {
                let _trimmed = ForceTrimmedGuard::new();
                ty.to_string()
            }
        }
    }
}

/// The generic arguments a function's name shows: its own, then its inherent impl's own type or its trait's.
fn shown_args<'tcx>(own_args: &[GenericArg<'tcx>], shape: &Shape<'tcx>) -> Vec<GenericArg<'tcx>> {
    let mut shown: Vec<GenericArg<'tcx>> = own_args.to_vec();
    match shape {
        Shape::Free => {}
        Shape::Inherent(ty) => shown.push((*ty).into()),
        Shape::Trait(tr) => shown.extend(tr.args.iter()),
    }
    shown
}
