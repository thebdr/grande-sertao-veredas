# Security

## Reporting a vulnerability

Report a vulnerability in Veredas 2 privately, through GitHub's private vulnerability reporting:
on this repository's page, open **Security**, then **Report a vulnerability**. The report stays
between you and the repository's owners while a fix is made. Please do not open a public issue,
discussion or pull request for it.

A report helps most when it names:
- the component and its version, or the suite release (a release's manifest,
  `releases/<release>.toml`, names every component's version, and each release bundle carries
  its component list, `sbom.cdx.json`);
- what you did, what happened, and what you expected;
- anything that limits who can reach it (a local user, the plant's network, the internet).

## Fixes and versions

Each component carries a version of its own (`docs/veredas-plan.md`, "Versioning"). A fix ships in
a new suite release, every component it changes at a new version, and the release's manifest names
them. No release has been published yet (`docs/questions.md` Q436): until one is, a fix lands on
`main`.

## What a release lists

Each release lists what it is made of in CycloneDX form (`docs/alforge-plan.md`, "The component
list"): every component with its version and licence, and every crate its components may compile on
the release's target, each by its package URL and checksum, so a scanner can match them against
published advisories. `alforge components <commit>` prints the same list for any commit.
