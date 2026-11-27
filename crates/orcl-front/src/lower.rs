//! The lowering: from the crate's `orcl_std::Program` implementation, every function it reaches in the program's
//! crates, each a body of the interpreter's, with the types they hold and the source files their spans name.
//!
//! The program's first three functions are its entries, in this order and by these names, `P` its state's name as the
//! file gives it (`crate::names`): `<P as Program>::scan`, `<P as Default>::default` and
//! `<P as Program>::safe_outputs`. A host computes the first state and the safe outputs by calling the last two,
//! until the program file carries them (`docs/questions.md` Q231: computed when the program is lowered, in a format
//! past 1).
use std::collections::{HashMap, VecDeque};

use orcl_interp::{Function, FunctionId, Program, Type, TypeId};
use rustc_middle::ty::TyCtxt;
use rustc_public::CrateDef;
use rustc_public::mir::mono::{Instance, InstanceKind};
use rustc_public::ty::{GenericArgKind, GenericArgs, Span, Ty};

use crate::defs::Defs;
use crate::error::{Refusal, Refused};
use crate::types::Key;

/// Everything lowered so far.
pub(crate) struct Lowerer<'tcx> {
    pub(crate) defs: Defs<'tcx>,
    pub(crate) types: Vec<Type>,
    /// Each type's size in the interpreter's slots, as its loader lays it out.
    pub(crate) slots: Vec<u32>,
    pub(crate) keys: HashMap<Key, TypeId>,
    pub(crate) memo: HashMap<Ty, TypeId>,
    pub(crate) functions: Vec<Option<Function>>,
    pub(crate) instances: HashMap<Instance, FunctionId>,
    queue: VecDeque<(FunctionId, Instance)>,
    files: Vec<String>,
    file_ids: HashMap<String, u32>,
}

impl<'tcx> Lowerer<'tcx> {
    fn new(tcx: TyCtxt<'tcx>) -> Lowerer<'tcx> {
        Lowerer {
            defs: Defs::new(tcx),
            types: Vec::new(),
            slots: Vec::new(),
            keys: HashMap::new(),
            memo: HashMap::new(),
            functions: Vec::new(),
            instances: HashMap::new(),
            queue: VecDeque::new(),
            files: Vec::new(),
            file_ids: HashMap::new(),
        }
    }

    pub(crate) fn refuse(&self, what: Refused, name: impl Into<String>, at: Span) -> Refusal {
        Refusal { what, name: name.into(), at: Some(self.defs.location(at)) }
    }

    /// A span as the interpreter's: its file's index in the program, its line and its column.
    pub(crate) fn span(&mut self, at: Span) -> orcl_interp::Span {
        let location = self.defs.location(at);
        let next = u32::try_from(self.files.len()).unwrap_or(u32::MAX);
        let file = *self.file_ids.entry(location.file.clone()).or_insert(next);
        if file == next {
            self.files.push(location.file);
        }
        orcl_interp::Span { file, line: location.line, column: location.column }
    }

    /// The function a call reaches, numbered on first sight and lowered in turn.
    pub(crate) fn function_of(&mut self, instance: Instance) -> FunctionId {
        if let Some(id) = self.instances.get(&instance) {
            return *id;
        }
        let id = FunctionId(u32::try_from(self.functions.len()).unwrap_or(u32::MAX));
        self.functions.push(None);
        self.instances.insert(instance, id);
        self.queue.push_back((id, instance));
        id
    }

    fn walk(&mut self, names: &[(Instance, String)]) -> Result<(), Refusal> {
        let named: HashMap<Instance, String> = names.iter().cloned().collect();
        while let Some((id, instance)) = self.queue.pop_front() {
            let name = named.get(&instance).cloned().unwrap_or_else(|| instance.trimmed_name());
            let function = self.function(instance, name)?;
            if let Some(slot) = self.functions.get_mut(id.0 as usize) {
                *slot = Some(function);
            }
        }
        Ok(())
    }

    fn finish(self) -> Program {
        Program { types: self.types, functions: self.functions.into_iter().flatten().collect(), files: self.files }
    }
}

fn no_program(name: impl Into<String>) -> Refusal {
    Refusal { what: Refused::Program, name: name.into(), at: None }
}

/// The entries' names, `p` the state's.
pub(crate) fn entry_names(p: &str) -> [String; 3] {
    [format!("<{p} as Program>::scan"), format!("<{p} as Default>::default"), format!("<{p} as Program>::safe_outputs")]
}

/// The crate's program: its type and its three entries, resolved.
fn entries(lowerer: &mut Lowerer<'_>) -> Result<(TypeId, Vec<(Instance, String)>), Refusal> {
    let impls: Vec<_> = rustc_public::local_crate()
        .trait_impls()
        .into_iter()
        .filter(|i| lowerer.defs.is_program(i.trait_impl().value.def_id.def_id()))
        .collect();
    let [program] = impls.as_slice() else {
        return Err(no_program(format!("{} implementations of orcl_std::Program", impls.len())));
    };
    let trait_ref = program.trait_impl().value;
    let this = trait_ref.self_ty();
    let at = program.span();
    if !program.generics_of().params.is_empty() {
        return Err(lowerer.refuse(Refused::Program, format!("a generic program, {this}"), at));
    }
    let state = lowerer.ty(this, at)?;
    let args = GenericArgs(vec![GenericArgKind::Type(this)]);
    let program_trait = trait_ref.def_id.def_id();
    let scan = lowerer.defs.resolve_trait_fn(program_trait, "scan", &args);
    let safe = lowerer.defs.resolve_trait_fn(program_trait, "safe_outputs", &args);
    let default = lowerer.defs.resolve_default(this);
    let (Some(scan), Some(default), Some(safe)) = (scan, default, safe) else {
        return Err(lowerer.refuse(Refused::Program, format!("no `Default` of {this}"), at));
    };
    for instance in [scan, default, safe] {
        if instance.kind != InstanceKind::Item {
            return Err(lowerer.refuse(Refused::Program, instance.trimmed_name(), at));
        }
    }
    let [scan_name, default_name, safe_name] = entry_names(&lowerer.name_of(state));
    Ok((state, vec![(scan, scan_name), (default, default_name), (safe, safe_name)]))
}

/// Lowers the crate the compiler holds into the interpreter's program, or says what it refuses.
pub(crate) fn lower(tcx: TyCtxt<'_>) -> Result<Program, Refusal> {
    let mut lowerer = Lowerer::new(tcx);
    let (state, names) = entries(&mut lowerer)?;
    for (instance, _) in &names {
        lowerer.function_of(*instance);
    }
    lowerer.walk(&names)?;
    lowerer.name_apart(state);
    Ok(lowerer.finish())
}
