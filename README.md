# veredas

Veredas 2: tools for watching, testing and driving industrial plants. A Rust workspace of
independently versioned components with Portuguese names, each a role nobody else covers, all
meeting the outside world through one gateway.

The name is from *Grande Sertão: Veredas*; the veredas are the paths through wild terrain, plural
because the suite follows many at once. Veredas 1 (the `Veredas` repo) grew from one log trailer
into sixteen crates on one version; this repo draws the borders before the suite grows further.
The two lines never merge.

## The components

| Component | Meaning | Role |
|---|---|---|
| `veredas-core` | the one English name | every class and library more than one component needs; a library, never an executable, depending on no other component |
| `veredas-portal` | gateway | the single point of exchange with the outside world: the data plane, the session store, SSE and REST, the gated write surface, the desktop shell |
| `veredas-controlador` | controller | a softPLC running ORCL programs; the only production component past portal, and only on its real-time path |
| `veredas-maquinar` | to machine | simulated hardware, built before the consumer that needs it; a test tool that owns its sockets |
| `veredas-aranha` | spider | the industrial network scanner, for setup and fault-finding, never for production |
| `veredas-comissionador` | commissioner | predefined checks on portal data, live or replayed, ending in an acceptance report |
| `veredas-abaco` | abacus | the engineering spreadsheet on a fork of IronCalc, with Python macros; `acerto` is its first extension |
| `veredas-supervisao` | supervision | the unified web interface, 2D and 3D, every value through portal |
| `veredas-wms` | later | a warehouse management system, after its own study |

None exists as code yet: each arrives in its own session through the five conditions below, and
is parked today as a pending delta in the contract.

## Orientation

- `docs/veredas-plan.md` is the plan: versioning, naming, the five conditions for a new member,
  the architecture, each component's scope and rules. It wins on intent.
- `.zen/contract.md` is the living contract: what is proven, by which test. It wins on status.
- `tools/leis` holds the suite's laws as tests (`cargo test -p leis`): the rules of the plan that
  a machine can check from the shape of the workspace.
- `docs/design/` holds the design references (mockups) for comissionador and supervisão;
  `docs/veredas-2-review.html` and `.pptx` are the deck of the 2026-11-05 review.
- `docs/questions.md` records the questions raised mid-build and the option taken.

## Adding a member

1. A role nobody covers, in one sentence, in `docs/<nome>-plan.md`, before the code.
2. Anything a second component could use is designed for `veredas-core` from the start.
3. All exchange with the outside world goes through `veredas-portal` (the test tools and
   controlador's real-time path excepted).
4. Any hardware it talks to has a simulator in `veredas-maquinar`, built before or alongside it.
5. A Portuguese `veredas-<nome>`, versioned on its own from core's current MAJOR.

## Build and test

    cargo build --workspace
    cargo nextest run --workspace

Rust stable, edition 2024. `cargo install cargo-nextest --locked` once on a fresh machine; plain
`cargo test --workspace` works too.
