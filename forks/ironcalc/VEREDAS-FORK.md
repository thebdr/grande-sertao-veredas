# IronCalc inside Veredas: Ábaco's fork, its record

This folder is Ábaco's copy of [IronCalc](https://github.com/ironcalc/IronCalc) (MIT OR
Apache-2.0; `LICENSE-MIT` and `LICENSE-Apache-2.0` here and in `base/` and `xlsx/` are upstream's,
kept verbatim): the engine (`base`, crate `ironcalc_base`) and its file layer (`xlsx`, crate
`ironcalc`), the two crates `veredas-abaco` builds on (`docs/abaco-plan.md`, "The fork").

| | |
|---|---|
| Upstream | `https://github.com/ironcalc/IronCalc.git` |
| Copied from | upstream `main` at `15ecfecb7cb9a50f7427911298d267023157fc45` (2026-11-02), crates `ironcalc_base` 0.9.1 and `ironcalc` 0.9.1; the trees of `base` (`1aaecb5`) and `xlsx` (`1556e78`) checked equal to upstream's on 2026-11-22 |
| What is carried | `base/`, `xlsx/`, upstream's licences and README, `Cargo.lock`; the root manifest's members cut to the two crates |
| Not carried | upstream's bindings (Python, WebAssembly, Node), web app, docs, generators and CI |
| History | a copy, not a subtree: upstream's history is upstream's (`docs/questions.md` Q340) |

## How it sits in the suite
- It lives at the repository's root, under `forks/`, and the suite's root manifest excludes it:
  these are not the suite's crates, so law L1 lets them out of the workspace, and they are no
  `veredas-*` crate, so nothing of the suite dodges the laws. It cannot live inside Ábaco's own
  folder: cargo makes a crate beneath a listed member's folder a member whatever `exclude` says
  (`docs/abaco-plan.md`, "Traps"). `veredas-abaco` reaches it by path, never through crates.io.
- This folder is a cargo workspace of its own for upstream's tests:
  `cargo test --locked --manifest-path forks/ironcalc/Cargo.toml`. Ábaco's component test runs
  them, so they run with Ábaco's own tests.
- A newer upstream is brought in by diff, as a step of its own, measured by the oracle
  (`tools/abaco-oracle`) before and after.
- A change is made upstream-shaped, one commit each, and listed below. It goes upstream as a patch
  from a clone of upstream; opening the pull request is the architect's act.

## Local changes
Each was first made in Veredas 1's copy (its `crates/ironcalc`, 2026-11-03 to 2026-11-07) and
arrives here as one commit, in the same order. Test data that named the workbooks of a real plant
is replaced by neutral names on the way (a linked book is `Linked.xlsx`, its sheet `ZONES`).

| # | Change | Where | Upstream |
|---|---|---|---|
| 1 | **External workbook references** `[1]Sheet!A1` and `'[1]Sheet name'!A1` (cells and ranges, A1 and R1C1): lexer tokens and parser nodes kept as written and re-emitted as written; `Workbook.external_books` (`external::ExternalBook`: target, sheets, cached cells, defined names); evaluation through a pluggable `external::ExternalResolver` on the model: `CachedResolver` (the file's `sheetDataSet`, Excel's behaviour with the linked book closed; the default) and `HostResolver` (an application hook). Unresolvable is `#REF!`, never `#ERROR!`. An external defined name (`[1]Book!Name`) is refused by the lexer. | `base/src/external.rs`, `base/src/expressions/{token,lexer,parser}`, `base/src/model.rs` | none |
| 2 | **External links survive load and save.** Import reads `workbook.xml` `<externalReferences>` (order = `[n]`), each `xl/externalLinks/externalLinkN.xml` (`externalBook` target via the part's `.rels`, `xxl21:absoluteUrl`, `sheetNames`, `definedNames`, the `sheetDataSet` cache; a listed cell without `<v>` is empty), DDE/OLE links as index placeholders. Export writes the parts, their `.rels`, the content-type overrides, the `workbook.xml.rels` relationships and `<externalReferences>` between `<sheets>` and `<definedNames>`. `XmlNode::child` made available outside tests. | `xlsx/src/{import,export}/external_links.rs`, `xlsx/src/import/{mod,workbook,worksheets,xml}.rs`, `xlsx/src/export/{mod,workbook,workbook_xml_rels}.rs` | none |
| 3 | **`LinkedResolver`, the linked strategy** (`ironcalc::import::LinkedResolver`): opens each external book from disk with `load_from_xlsx` (relative targets against the workbook's folder, URLs by file name, `file:///`), evaluates it once, reads live values; nested links get a resolver with one less `depth` (default 2), the chain of files being opened cuts cycles; a book that cannot be opened is reported (`LinkStatus`, `preload`) and read from the cache (or `#REF!` with the fallback off). | `xlsx/src/import/linked.rs` | none |
