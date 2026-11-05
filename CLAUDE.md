# grande-sertao-veredas (Veredas 2)

The successor suite to Veredas 1 (`C:\Source\Repos\Veredas`: its own history, never merged here).
A Rust workspace; each component is a crate with a version of its own. Read this, then the plan.

## Authority, in order
1. `docs/veredas-plan.md` wins on **intent** (roles, rules, borders). Change it in the same commit
   as the decision that changes it.
2. `.zen/contract.md` wins on **status**: a capability is what its clause says, proven by the test
   the clause links. One or two sentences per clause and a pointer to the plan, never a copy of its
   prose.
3. `tools/leis` **enforces** the structural rules as cargo tests (laws L1–L6). A law changes only
   with the contract clause that states it.
4. There is no session log: the contract, `.zen/evidence/` and git are the record.

## The rules (the plan, one line each; the law that enforces it, or "prose")
- Members live under `crates/` (components) or `tools/` (repo tooling), each listed by path. L1
- A component is `veredas-<nome>`, Portuguese, lowercase ASCII, in a folder of its name; screens
  show the accented name (Ábaco, Supervisão). `orcl-*` keep their names; an extension (`acerto`)
  lives inside its host. L2
- SemVer per component from 2.0.0, no shared workspace version; a new component starts at core's
  current MAJOR. L3
- `veredas-core` depends on no other member, in any dependency table. L4
- Dependencies point one way: anything may use core; the apps use portal; the bench (maquinar)
  and the scanner (aranha) own their sockets and never pull portal; controlador embeds portal
  and aranha and runs ORCL. The map is `MAP` in `tools/leis/src/lib.rs`. L5
- A component's plan, `docs/<nome>-plan.md`, exists before its code. L6
- Prose: fixture first (its simulator in maquinar before or alongside); every plain-text config
  and project file states its schema version, portal migrates or refuses; writes stay gated.

## Adding a member
1. `docs/<nome>-plan.md`: the one-sentence role nobody covers, scope, rules, what it absorbs.
2. Its `P-xxx` in the contract becomes `C-xxx` clauses as capabilities land (`zen-contract`).
3. `crates/veredas-<nome>` at `<core MAJOR>.0.0-alpha.1`, listed in `Cargo.toml`.
4. Its hardware simulated in `veredas-maquinar` first.
5. `cargo test -p leis` green, then the component's own tests.
6. `zen-reconcile` before the turn ends: contract, test, docs and code agree.

## The zen loop here
- Record a run (the real command; its exit code is the verdict):
  `node <plugin>/hooks/zen-record-run.js --clause C-00x -- cargo test -p leis --test leis <test>`
  where `<plugin>` is the implication-machine directory the session-start index prints.
- A behavioral clause gets a `zen-refuter` pass, then
  `node <plugin>/hooks/zen-record-refute.js --clause C-00x --verdict holds|trivial|refuted`, then
  `zen verify C-00x` flips the status.
- `.zen/evidence/runs/` and `refutes/` are committed; `audit.jsonl` is not. Record on a clean tree,
  at a commit that holds the test.

## Build, test, commit
- `cargo build --workspace` · `cargo nextest run --workspace`, run bare, never piped (a pipe hides
  the exit code).
- Typed errors in library code, no `unwrap`/`expect` on I/O paths; tests may unwrap.
- Blocked on ambiguity: write the question to `docs/questions.md`, take the most reversible
  option, mark it `// DECISION PENDING`, continue.
- Commits go to `main` directly (one maintainer, no PR workflow); the global commit policy applies.
- Component traps, recipes and harness hooks go to `docs/<nome>-plan.md` and `.zen/evidence/`,
  never here. This file stays one screen; Veredas 1's grew to 42 KB.
