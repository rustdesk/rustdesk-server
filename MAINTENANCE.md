# Maintaining the MTG RustDesk fork

`master` is the default source branch. `codex/websocket-registration` maintains
Quick Support's WebSocket transport and image. Changes to one branch do not
qualify the other; each has its own dependency lockfile and common-library patch.

## Local source checks

Initialize recursive submodules, then run these commands with Rust 1.91.1 and
the repository's native build prerequisites installed:

```sh
python3 -m unittest discover -s tests -p 'test_*.py'
python3 scripts/apply_common_patch.py
cargo +1.91.1 check --locked --workspace
cargo +1.91.1 fmt --all -- --check
cargo +1.91.1 clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo +1.91.1 test --locked --workspace --all-features
```

The native check generates `src/version.rs` for the formatting check. The common
patch deliberately modifies submodule files locally; its manifest records the
exact before/after content. Never commit an unpublished replacement submodule
revision. When updating that revision, regenerate and review the downstream patch
and manifest as described in `patches/hbb-common/README.md`.

## Automated evidence

Repository validation checks Rust formatting, diagnostics, tests, measured
coverage, lockfile and auditable-binary dependencies, and CycloneDX inventories.
An audit build omits debug symbols to stay within the auditor's binary-size limit;
its embedded dependency metadata remains mandatory. Both complete lockfile and
compiled-binary audits run without advisory ignores.

CodeQL covers Rust, Actions, and Python. The legacy Tauri server-management GUI
and its separate vulnerable Rust dependency graph have been retired.
Review extraction diagnostics as well as finding counts: a successful upload with
incomplete extraction does not qualify source analysis.

The Rust Sonar adapter uses actual LCOV and Clippy JSON produced from the exact
candidate head, plus real Python helper coverage from the same checkout.
The consuming job verifies head, base, patch-manifest digest, and
report digests. Fork pull requests receive no scanning credentials. Generated
version/protobuf build output is excluded from the LCOV import; authored server
and common-library source remain covered by analysis.

A new Sonar project is initialized from original `master` commit
`a7736be5e40f85bfc141120dce587e836e5d4b80`, with its actual tests and coverage.
The maintained branch is separately initialized from
`5997e8c1f90b1af62e7e6e293d79cdc0b938d9d1` before its PRs are analyzed, so
its changes are compared with that branch rather than the newer default source.
Long-lived branch gates also need an explicit new-code definition. Bind each
branch to the analysis of its original revision above through SonarCloud's
`api/project_analyses/set_baseline` API (`project`, `branch`, `analysis`). Verify
the selected analysis revision first, and read back a `manual_baseline` gate period
at that original analysis date. Do not advance this baseline past unvalidated
changes to obtain a passing gate. A PR diff can omit patched submodule changes;
the long-lived branch gate must independently validate their measured coverage.

Existing backlog is report-only; this initialization is not a clean quality-gate
attestation or a merge/deployment candidate. Candidate scans await their own gate.
The final admission job fails when required evidence fails, is skipped, or is
unavailable; a skipped downstream scanner cannot become a passing admission check.
Before relying on Sonar for admission, verify project/analysis/head/base identity,
actual indexed files and imported coverage, effective exclusions, new-code policy,
small-change behavior, and supported branch/PR analysis. Missing evidence is pending.

Dependabot checks Cargo, Actions, and submodules on both maintained branches.
Minor/patch updates are grouped; major updates remain separate for deliberate
review. Security updates remain eligible immediately.

## Image and delivery boundaries

The maintained branch builds and exercises the patched BusyBox applets, RustDesk
protocol, and S6 lifecycle before registry authentication. Publication inventories
the tested image and attaches build provenance and its SPDX SBOM to that image's
immutable digest. Confirm registry attestations after publication; workflow source
alone is not evidence that an attestation exists.

The default branch has no Quick Support image; its image-check disposition is
explicitly inapplicable. A merged source change or published image does not prove
an ACA deployment or Windows customer/technician acceptance. Those have separate
authorization and runtime verification requirements.

## Visible dependency debt

RustSec warnings for legacy unmaintained crates remain visible, including
`sodiumoxide`, `dlopen_derive`, `ansi_term`, and `atty`; the default branch also
retains `bincode` 1.x. The `atty` unsoundness advisory concerns its Windows code.
These warnings are not vulnerability-free or cross-platform compatibility claims.
Replacing cryptographic or serialized formats requires explicit compatibility
work and tests; do not suppress advisories to make that work disappear.
