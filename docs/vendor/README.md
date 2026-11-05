# Vendor documents (gitignored)

Proprietary protocol documents live here on each dev machine and never enter git history
(`docs/questions.md` Q6).

Expected files:

- **`genibelt-protocol.txt`**: the GENIBELT / Master Sort Control host interface specification,
  the authority behind `veredas_core::telegram` (the framing, the typed catalog in `genibelt`;
  code comments cite its section numbers). Veredas 1 keeps a copy in its own `docs/vendor/`.

A machine without these files builds and tests fine: the constants are in code, pinned by tests.
