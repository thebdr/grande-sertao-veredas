# grande-sertao-veredas (Veredas 2)

The successor suite to Veredas 1 (the `Veredas` repository: its own history, never merged here).
A Rust workspace; each component is a crate with a version of its own. Read this, then the plan.

## Authority, in order
1. `docs/veredas-plan.md` wins on **intent** (roles, rules, borders). Change it in the same commit
   as the decision that changes it.
2. `.zen/contract.md` wins on **status**: a capability is what its clause says, proven by the test
   the clause links. One or two sentences per clause and a pointer to the plan, never a copy of its
   prose.
3. `tools/leis` **enforces** the structural rules as cargo tests (laws L1–L7). A law changes only
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
- Every TOML file that is not a cargo manifest, and every TOML block of a reference in `docs/`,
  states a `veredas-*` `format` and a `version` from 1; portal migrates or refuses, never guesses. L7
- Prose: fixture first (its simulator in maquinar before or alongside); writes stay gated.

## Adding a member
1. `docs/<nome>-plan.md`: the one-sentence role nobody covers, scope, rules, what it absorbs.
2. Its `P-xxx` in the contract becomes `C-xxx` clauses as capabilities land (`zen-contract`).
3. `crates/veredas-<nome>` at `<core MAJOR>.0.0-alpha.1`, listed in `Cargo.toml`.
4. Its hardware simulated in `veredas-maquinar` first.
5. `cargo test -p leis` green, then the component's own tests.
6. `zen-reconcile` before the turn ends: contract, test, docs and code agree.

## The zen loop here
- Record a run (the real command; its exit code is the verdict):
  `node <plugin>/hooks/zen-record-run.js --clause C-00x -- cargo test -p leis --test l<x>_<law> --test real_tree`
  where `<plugin>` is the implication-machine directory the session-start index prints. One test
  file per law, so a change to one law re-stales only its clause's records.
- A behavioral clause gets a `zen-refuter` pass, then
  `node <plugin>/hooks/zen-record-refute.js --clause C-00x --verdict holds|trivial|refuted`, then
  `zen verify C-00x` flips the status.
- `.zen/evidence/runs/` and `refutes/` are committed; `audit.jsonl` is not. Record on a clean tree,
  at a commit that holds the test.

## Build, test, commit
- `cargo build --workspace` · `cargo nextest run --workspace`, run bare, never piped (a pipe hides
  the exit code). Green on the development machine (Linux) is the gate; Windows is tested in a VM
  with a clone of its own. GitHub runs the suite too on every push (`.github/workflows/leis.yml`),
  a clean-checkout check no push waits for (the architect, 2026-11-22). L6 reads git history, so
  the laws need a repository with its commits.
- Typed errors in library code, no `unwrap`/`expect` on I/O paths; tests may unwrap.
- Blocked on ambiguity: write the question to `docs/questions.md`, take the most reversible
  option, mark it `// DECISION PENDING`, continue.
- Commits go to `main` directly (no PR workflow), under "Commit and push" below.
- No tool that waits on a person or on the desktop app's window: a page is probed headless.
- Component traps, recipes and harness hooks go to `docs/<nome>-plan.md` and `.zen/evidence/`,
  never here. This file stays one screen; Veredas 1's grew to 42 KB.

## Several sessions at once (the architect, 2026-11-22)
- One member session per component: its main developer, the only writer of its crate, its plan
  and its clauses. One orchestrating session starts them, checks every "done" against git and the
  contract, and carries the questions to the architect; a member never asks the architect.
- Each session has a git worktree and a `target/` of its own. Its commits reach `main` rebased,
  never merged: `git fetch origin`, `git rebase origin/main`, the suite, `git push origin
  HEAD:main`; a refused push is rebased and pushed again. A conflict in a shared file keeps both
  sides' entries whole.
- Shared files (`Cargo.toml`, `Cargo.lock`, the contract, `docs/questions.md`, the suite plan,
  `tools/leis`, this file) change in small commits pushed at once. What another member owns is
  asked for through the orchestrator, never edited; so is a new `P-xxx`.
- A new clause or question takes the next free number of its member's block, so no two members
  claim one number: supervisão C-055/Q190, core C-070/Q100, portal C-090/Q130, maquinar
  C-110/Q160, aranha C-130/Q280, controlador (with ORCL) C-150/Q220, comissionador C-200/Q310,
  ábaco C-220/Q340, oficina C-250/Q370, releases C-290/Q430; a block ends where the next begins.
  Suite-wide ones run on from C-300 and Q46.
- Before a push a member runs the laws and every test its change can reach:
  `cargo nextest run --workspace -E 'rdeps(veredas-<nome>) | package(leis)'` (for core, the whole
  suite); the orchestrator runs the whole suite on `main` as pushes land. A run that holds
  maquinar's or supervisão's tests (they measure time and drive browsers) goes one at a time on
  the machine, built first so the lock is held for the tests alone: the command with `--no-run`,
  then `flock /tmp/veredas-suite.lock cargo nextest run ...` (the lock keeps the exit code).

## Commit and push (this repository's policy: it wins over a session's global rules)
- Commit every finished change and push it, without asking: Git is made for mistakes to be pushed,
  and a commit made by these rules is taken back with one `git revert`. In doubt, push. A turn
  that ends on "commit?", "push?" or "pushable when you want" has a defect.
- A commit is one finished change: it builds, the suite is green, and the docs the repository
  keeps for it (its contract clause and plan among them) moved with it. One logical change per
  commit; a cosmetic and a semantic change split when they can. Its message: a title naming the
  component or area, what changed and why, then the attribution trailer the session is given.
- Never committed: work that does not build or has a red suite (no WIP commit); half a feature,
  unless the half stands on its own; files that are not the repository's (tool folders, scratch
  outputs, session notes such as `HANDOFF.md`, secrets, build outputs, IDE files, anything over a
  few MB that is not a fixture); someone else's uncommitted work, which is named and left.
- Stage by path (`git add <paths>`), never `git add -A`: the repository is public, and a revert
  does not unpublish a secret or a session note. Say what a commit left out of the tree.
- Push to `origin` as soon as the change in hand is committed: what is fit to commit is fit to
  push. Name the commits pushed in that turn. One thing holds a push, someone else's unpushed
  commits on the branch: say so in one line and carry on with the work.
- A pushed mistake is answered by a new commit, a fix or a `git revert`; after a red run of the
  suite, here or on GitHub, it is the next push. Never force-push, rewrite pushed history or amend
  a pushed commit.
