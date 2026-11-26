//! The definitions the front end keys on, found the way the compiler finds them, never by a printed name
//! (`docs/orcl/front-end.md`: printed names follow re-exports, so core's `Duration` prints as `orcl_std::Duration`):
//! core's `Option` and `PartialOrd`, `PartialEq` by their lang items, `Duration`, `Ord` and `Default` by their
//! diagnostic items, `orcl_std::Program` by its crate and its own name. A call into core is an operation of the
//! closed list when the function it calls is one of the list's, known by the trait or the inherent impl that
//! defines it and the function's own name there, at the types the list takes.
use orcl_interp::{BinOp, CoreOp, Int};
use rustc_middle::ty::{self as rty, TyCtxt};
use rustc_public::mir::mono::Instance;
use rustc_public::rustc_internal;
use rustc_public::ty::{
    AdtDef, AssocContainer, AssocKind, FloatTy, FnDef, GenericArgKind, GenericArgs, IntTy, RigidTy, Span, Ty, TyKind,
    UintTy,
};
use rustc_public::{Crate, CrateDef};
use rustc_span::Symbol;
use rustc_span::def_id::{CrateNum, DefId, LOCAL_CRATE};
use rustc_span::hygiene::{ExpnKind, MacroKind};

use crate::error::Location;

/// The standard library's crate: with the program's own, the program's crates, whose functions are lowered.
pub(crate) const STD: &str = "orcl_std";

/// What a scalar of the closed list is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scalar {
    Bool,
    Int(Int),
    F32,
    F64,
    Duration,
}

/// The definitions of core and `orcl-std` the front end knows.
pub(crate) struct Defs<'tcx> {
    pub(crate) tcx: TyCtxt<'tcx>,
    option: Option<DefId>,
    duration: Option<DefId>,
    ord: Option<DefId>,
    partial_ord: Option<DefId>,
    partial_eq: Option<DefId>,
    default: Option<DefId>,
}

pub(crate) const fn int_of(t: IntTy) -> Option<Int> {
    match t {
        IntTy::I8 => Some(Int::I8),
        IntTy::I16 => Some(Int::I16),
        IntTy::I32 => Some(Int::I32),
        IntTy::I64 => Some(Int::I64),
        IntTy::Isize => Some(Int::Isize),
        IntTy::I128 => None,
    }
}

pub(crate) const fn uint_of(t: UintTy) -> Option<Int> {
    match t {
        UintTy::U8 => Some(Int::U8),
        UintTy::U16 => Some(Int::U16),
        UintTy::U32 => Some(Int::U32),
        UintTy::U64 => Some(Int::U64),
        UintTy::Usize => Some(Int::Usize),
        UintTy::U128 => None,
    }
}

/// An integer type as the compiler has it inside, for an inherent impl's own type.
fn internal_int(ty: rty::Ty<'_>) -> Option<Int> {
    match ty.kind() {
        rty::Int(rty::IntTy::I8) => Some(Int::I8),
        rty::Int(rty::IntTy::I16) => Some(Int::I16),
        rty::Int(rty::IntTy::I32) => Some(Int::I32),
        rty::Int(rty::IntTy::I64) => Some(Int::I64),
        rty::Int(rty::IntTy::Isize) => Some(Int::Isize),
        rty::Uint(rty::UintTy::U8) => Some(Int::U8),
        rty::Uint(rty::UintTy::U16) => Some(Int::U16),
        rty::Uint(rty::UintTy::U32) => Some(Int::U32),
        rty::Uint(rty::UintTy::U64) => Some(Int::U64),
        rty::Uint(rty::UintTy::Usize) => Some(Int::Usize),
        _ => None,
    }
}

impl<'tcx> Defs<'tcx> {
    pub(crate) fn new(tcx: TyCtxt<'tcx>) -> Defs<'tcx> {
        let lang = tcx.lang_items();
        let diagnostic = |name: &str| tcx.get_diagnostic_item(Symbol::intern(name));
        Defs {
            tcx,
            option: lang.option_type(),
            duration: diagnostic("Duration"),
            ord: diagnostic("Ord"),
            partial_ord: lang.partial_ord_trait(),
            partial_eq: lang.partial_eq_trait(),
            default: diagnostic("Default"),
        }
    }

    fn internal(&self, def: rustc_public::DefId) -> DefId {
        rustc_internal::internal(self.tcx, def)
    }

    fn program_crate_num(&self, krate: CrateNum) -> bool {
        krate == LOCAL_CRATE || self.tcx.crate_name(krate).as_str() == STD
    }

    /// Whether a crate is one of the program's: its own, or `orcl-std`.
    pub(crate) fn program_crate(krate: &Crate) -> bool {
        krate.is_local || krate.name == STD
    }

    pub(crate) fn is_option(&self, adt: AdtDef) -> bool {
        self.option.is_some_and(|d| d == self.internal(adt.def_id()))
    }

    pub(crate) fn is_duration(&self, adt: AdtDef) -> bool {
        self.duration.is_some_and(|d| d == self.internal(adt.def_id()))
    }

    /// Whether a trait is `orcl_std::Program`: the trait of that name in the standard library's crate.
    pub(crate) fn is_program(&self, def: rustc_public::DefId) -> bool {
        let d = self.internal(def);
        self.tcx.crate_name(d.krate).as_str() == STD && self.tcx.item_name(d).as_str() == "Program"
    }

    /// A trait's function of that own name, at `args`, resolved.
    pub(crate) fn resolve_trait_fn(&self, def: rustc_public::DefId, name: &str, args: &GenericArgs) -> Option<Instance> {
        let tr = self.internal(def);
        let f = self.tcx.associated_item_def_ids(tr).iter().copied().find(|f| self.tcx.item_name(*f).as_str() == name)?;
        let f = FnDef(rustc_internal::stable(f));
        Instance::resolve(f, args).ok()
    }

    /// `Default::default` at `ty`, resolved.
    pub(crate) fn resolve_default(&self, ty: Ty) -> Option<Instance> {
        let tr = self.default?;
        self.resolve_trait_fn(rustc_internal::stable(tr), "default", &GenericArgs(vec![GenericArgKind::Type(ty)]))
    }

    /// What scalar of the list `ty` is, if any.
    pub(crate) fn scalar(&self, ty: Ty) -> Option<Scalar> {
        match ty.kind() {
            TyKind::RigidTy(RigidTy::Bool) => Some(Scalar::Bool),
            TyKind::RigidTy(RigidTy::Int(t)) => int_of(t).map(Scalar::Int),
            TyKind::RigidTy(RigidTy::Uint(t)) => uint_of(t).map(Scalar::Int),
            TyKind::RigidTy(RigidTy::Float(FloatTy::F32)) => Some(Scalar::F32),
            TyKind::RigidTy(RigidTy::Float(FloatTy::F64)) => Some(Scalar::F64),
            TyKind::RigidTy(RigidTy::Adt(adt, _)) if self.is_duration(adt) => Some(Scalar::Duration),
            _ => None,
        }
    }

    fn is_option_arg(&self, args: &GenericArgs, i: usize) -> bool {
        match args.0.get(i) {
            Some(GenericArgKind::Type(t)) => matches!(t.kind(), TyKind::RigidTy(RigidTy::Adt(adt, _)) if self.is_option(adt)),
            _ => false,
        }
    }

    fn type_arg(&self, args: &GenericArgs, i: usize) -> Option<Scalar> {
        match args.0.get(i)? {
            GenericArgKind::Type(t) => self.scalar(*t),
            _ => None,
        }
    }

    /// The operation of the closed list a call of `def` at `args` is, if it is one: `def` a function of core, of
    /// one of the traits or inherent impls the list names, at the types it takes.
    pub(crate) fn core_op(&self, def: FnDef, args: &GenericArgs) -> Option<CoreOp> {
        let did = self.internal(def.def_id());
        if self.tcx.crate_name(did.krate).as_str() != "core" {
            return None;
        }
        let item = def.associated_item()?;
        let AssocKind::Fn { name, .. } = &item.kind else { return None };
        let parent = self.internal(def.def_id().parent()?);
        let name = name.as_str();
        match item.container {
            AssocContainer::Trait => {
                let is = |t: Option<DefId>| t == Some(parent);
                if is(self.default) && name == "default" && self.is_option_arg(args, 0) {
                    // `<Option<T> as Default>::default`: `None`, which the list's `Default` gives an `Option` too
                    return Some(CoreOp::Default);
                }
                let this = self.type_arg(args, 0)?;
                if is(self.ord) {
                    match (name, this) {
                        ("min", Scalar::Int(t)) => Some(CoreOp::Min(t)),
                        ("max", Scalar::Int(t)) => Some(CoreOp::Max(t)),
                        ("min", Scalar::Duration) => Some(CoreOp::DurationMin),
                        ("max", Scalar::Duration) => Some(CoreOp::DurationMax),
                        _ => None,
                    }
                } else if is(self.partial_eq) || is(self.partial_ord) {
                    let op = match name {
                        "eq" if is(self.partial_eq) => BinOp::Eq,
                        "ne" if is(self.partial_eq) => BinOp::Ne,
                        "lt" if is(self.partial_ord) => BinOp::Lt,
                        "le" if is(self.partial_ord) => BinOp::Le,
                        "gt" if is(self.partial_ord) => BinOp::Gt,
                        "ge" if is(self.partial_ord) => BinOp::Ge,
                        _ => return None,
                    };
                    let rhs = self.type_arg(args, 1)?;
                    (this == Scalar::Duration && rhs == Scalar::Duration).then_some(CoreOp::DurationCompare(op))
                } else if is(self.default) && name == "default" {
                    Some(CoreOp::Default)
                } else {
                    None
                }
            }
            AssocContainer::InherentImpl => {
                // an inherent impl of core names its own type plainly (`i32`, `Duration`, `Option<T>`): nothing to normalize
                let own = self.tcx.type_of(parent).instantiate_identity().skip_normalization();
                if let Some(t) = internal_int(own) {
                    return match name {
                        "saturating_add" => Some(CoreOp::SaturatingAdd(t)),
                        "saturating_sub" => Some(CoreOp::SaturatingSub(t)),
                        "saturating_mul" => Some(CoreOp::SaturatingMul(t)),
                        "wrapping_add" => Some(CoreOp::WrappingAdd(t)),
                        "wrapping_sub" => Some(CoreOp::WrappingSub(t)),
                        "wrapping_mul" => Some(CoreOp::WrappingMul(t)),
                        "checked_shl" => Some(CoreOp::CheckedShl(t)),
                        "checked_shr" => Some(CoreOp::CheckedShr(t)),
                        _ => None,
                    };
                }
                let rty::Adt(adt, _) = own.kind() else { return None };
                if Some(adt.did()) == self.duration {
                    match name {
                        "from_millis" => Some(CoreOp::DurationFromMillis),
                        "from_secs" => Some(CoreOp::DurationFromSecs),
                        "saturating_sub" => Some(CoreOp::DurationSaturatingSub),
                        _ => None,
                    }
                } else if Some(adt.did()) == self.option {
                    match name {
                        "get_or_insert" => Some(CoreOp::OptionGetOrInsert),
                        "is_none" => Some(CoreOp::OptionIsNone),
                        "is_some" => Some(CoreOp::OptionIsSome),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            AssocContainer::TraitImpl(_) => None,
        }
    }

    /// The name a refusal gives a call: a trait's function as `<Self as Trait>::name` at the types it is called at,
    /// any other as the compiler names the function it resolves to.
    pub(crate) fn call_name(&self, def: FnDef, args: &GenericArgs, resolved: Option<&Instance>) -> String {
        let fallback = || resolved.map_or_else(|| def.name(), Instance::name);
        let (Some(item), Some(parent)) = (def.associated_item(), def.def_id().parent()) else { return fallback() };
        match (&item.kind, &item.container, args.0.first()) {
            (AssocKind::Fn { name, .. }, AssocContainer::Trait, Some(GenericArgKind::Type(this))) => {
                format!("<{this} as {}>::{name}", parent.trimmed_name())
            }
            _ => fallback(),
        }
    }

    /// The span a source location names: past the expansion of a macro defined outside the program's crates (core's
    /// `matches!`), to where the program calls it; a derive's expansion and the program's own macros keep their
    /// spans, which name the program's lines.
    pub(crate) fn source(&self, span: Span) -> Span {
        let mut sp: rustc_span::Span = rustc_internal::internal(self.tcx, span);
        for _ in 0..64 {
            if !sp.from_expansion() {
                break;
            }
            let data = sp.ctxt().outer_expn_data();
            let foreign_bang = matches!(data.kind, ExpnKind::Macro(MacroKind::Bang, _))
                && !data.macro_def_id.is_some_and(|d| self.program_crate_num(d.krate));
            if !foreign_bang {
                break;
            }
            sp = data.call_site;
        }
        rustc_internal::stable(sp)
    }

    pub(crate) fn location(&self, span: Span) -> Location {
        let sp = self.source(span);
        let lines = sp.get_lines();
        Location {
            file: sp.get_filename(),
            line: u32::try_from(lines.start_line).unwrap_or(u32::MAX),
            column: u32::try_from(lines.start_col).unwrap_or(u32::MAX),
        }
    }
}
