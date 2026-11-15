# Repository Instructions

## Rust Rules

- In Rust code, do not introduce `unwrap()` or `expect()`.
- Allowed exceptions:
- Tests may use `unwrap()` or `expect()` when it keeps the test focused and readable.
- Lock acquisition may use `unwrap()` only when the locking API makes that the practical option and the failure mode is poison handling rather than normal control flow.
- Outside those exceptions, propagate errors, handle them explicitly, or use safer fallbacks instead of `unwrap()` and `expect()`.

## Editing Hygiene

- Do not introduce formatting-only changes.
- Do not run repository-wide formatters or reflow unrelated code unless the
  user explicitly asks for formatting.
- Keep diffs limited to semantic changes required for the task.

## Secret gate before publication

Before publishing the current PR branch, commit the complete candidate and run
`python3 -I scripts/secret_gate.py --base refs/remotes/origin/master` (native Windows: `python -I`). Require exit 0;
missing tooling/authentication, scan errors and skipped scans block publication.
Then run the existing repository gates. Repeat after changing outgoing commits.
Push exactly the scanned HEAD. This PR scan covers the current branch against its
fetched intended base; other refs, multi-ref pushes and rewritten history require
`--pre-push` with Git's complete ref-update stream. Stop if any pushed ref is
uncovered. See [secret scanning](SECRET-SCANNING.md) for base selection, new branches,
pre-push input and prerequisites. Preserve GitGuardian App, CI and Gitleaks checks.

For a non-default PR target, fetch its destination and pass that exact ref to
`--base`; the remote default is not a substitute for a release/maintenance base.

The secret gate requires Python 3.11+ and approved ggshield >=1.56.1,<2.

Explicit bases require fully qualified refs such as `refs/remotes/origin/master`
or full commit SHAs; ambiguous short names block with exit 20.
