# Secret checks before publication

Before publishing the **current PR branch**, commit the complete candidate and
run the local secret gate against the fetched, intended PR base, followed by this
repository's normal validation. Push exactly the scanned HEAD.
A change to any outgoing commit requires a fresh scan. This gate scans committed
history; it does not certify uncommitted files. It performs no push or PR write.

```sh
python3 -I scripts/secret_gate.py
# Explicit non-default PR target or destination tip (fetch it first):
python3 -I scripts/secret_gate.py --base refs/remotes/origin/release
# Initial publication, empty remote or orphan history, explicitly:
python3 -I scripts/secret_gate.py --all-history
# Offline regression checks (no GitGuardian calls):
python3 -I tests/test_secret_gate.py -q
```

Use `python` instead of `python3` on native Windows. Python 3.11+, Git, and
ggshield on PATH are required. Require ggshield **>=1.56.1,<2**; this gate was tested with **1.56.1**.
Install ggshield through the approved development-tool environment. Use existing
native ggshield authentication or an approved process-injected
`GITGUARDIAN_API_KEY`; never put credentials in the repository. For a private
GitGuardian instance, explicitly set its approved `GITGUARDIAN_INSTANCE`.
The gate never logs in, creates a key, or requests additional access.

By default the gate queries the configured `origin` remote's **current default
HEAD**, resolves that commit locally, and scans `base..HEAD`. It does not assume
`main`/`master`, use a feature branch's already-pushed upstream as the PR base,
or infer a base from `HEAD~1`. Use `--remote NAME` for a different configured
remote. If the remote tip isn't available locally, fetch it and rerun. A missing
base, failed remote lookup, grafted history or shallow clone blocks; fetch complete history first.
An explicit `--base` trusts the selected locally available ref; fetch it before use.

A PR scan certifies only commits outside its selected base that are reachable
from the current HEAD. It does not certify other branches or tags. Before pushing
other refs, multiple refs, or rewritten history, use `--pre-push` with the complete
Git update stream, normally through a composed pre-push hook. If the intended
publication is not covered by the scan, stop; do not reuse its clean result.

`--pre-push` reads Git's standard four-column ref updates from stdin, validates
all updates, then scans each `old..new` range. Force pushes include all commits
unique to the new tip. A new ref scans **all reachable history**, including root
commits; it can cost more API quota. Multiple updates and lightweight commit tags are supported. Annotated tags are
blocked with exit 20 because commit-range scanning does not cover annotation
text. Rerunning this gate still exits 20, even after an external annotation scan.
This gate does not offer a bypass. Unknown tips and non-commit tags block. Deletes have no content.
No hooks or Git configuration are installed automatically; compose existing hooks
rather than replacing them. Agent instructions require the gate even without hooks.

All commits in the selected range reach `ggshield secret scan commit-range`,
including merged side history and content subsequently removed. There is no hook
commit-count cap. Native ggshield detector/file limits still apply (including
binary/oversize content); this is not an attestation that every byte was scanned.
See the official [commit-range](https://docs.gitguardian.com/ggshield-docs/reference/secret/scan/commit-range)
and [configuration](https://docs.gitguardian.com/ggshield-docs/configuration) references.

Only exit **0 / CLEAN** means a successful scanner run. Other outcomes block:

| Exit | Outcome |
| --- | --- |
| 1 | Findings; remediate before publication |
| 2 | Invalid gate arguments |
| 3 | Authentication failed/unavailable; owner must restore approved native auth |
| 4 | Scan/transport/server/timeout error, or HEAD/base changed between initial resolution and final verification |
| 10 | Skipped: empty range or deletion-only input; no clean scan claimed |
| 20 | Invalid/unavailable base, Git operation, history or hook input |
| 21 | Missing/untrusted executable, unsupported Python, or unverifiable/unsupported ggshield version |

No skip or fail-open switch is provided. Strict temporary scanner configuration
ignores local/global exclusions and exit-zero settings; dotenv loading and scanner
log files are disabled, and all secrets are requested. Credentials stay in their
native storage/process environment. Raw findings and diagnostics are suppressed,
including source context; investigate findings locally in an approved private
terminal. Do not paste secret values into chat, tickets or logs.

GitGuardian App/server-side checks, CI secret scans, Gitleaks, branch protection,
and existing approval controls remain required and unchanged. Local scanning adds
an earlier gate; it does not replace them. Existing offline CI entry points remain
offline; agent guidance requires this additional gate. Use the local wrapper where
one is documented; otherwise invoke the Python command explicitly.

Tool executables must resolve outside the checkout through absolute PATH entries.
The gate rejects checkout-owned tools and Windows implicit current-directory
lookup; use an approved user-managed tool environment outside the repository.
Follow this repository's higher Python minimum when applicable.


Always invoke the gate with Python isolated mode (`-I`); an unisolated CLI
returns exit 21 before importing the rest of the gate. Ambient Git location
overrides and Python import/home overrides are removed. Scanner processes run
from a temporary directory with Git explicitly pinned to the intended worktree.
Tool lookup excludes this repository and its linked worktrees, while approved
external installations may be Git-managed. Standard `.git`, submodule and linked-worktree
layouts are supported; unsupported metadata layouts block with exit 20.
For an unsupported ref spelling, resolve and pass its full commit SHA.


For submodules, tool lookup also excludes the enclosing checkout that owns its
Git metadata. The scan remains pinned to the submodule worktree and commits.


Metadata reads are bounded and confined to Git administrative directories;
symlinks escaping those directories block. PR mode rechecks its selected base
after scanning. This narrows the race window; rerun against the exact current
destination before publication, with server checks as the backstop.


Inherited `GIT_EXEC_PATH` is removed from Git and scanner children so it cannot
select a checkout-owned transport helper. Native `pre-push` targets, where
present, consume the complete four-column Git ref-update stream on standard
input. No input returns SKIPPED (exit 10) and blocks publication.

Relative worktree registration pointers resolve against their own directory.
Tool exclusion includes linked worktrees of the enclosing metadata owner for
submodules. Metadata symlink loops return BASE_ERROR (exit 20). Git and scanner
children discard GIT_EXEC_PATH, GIT_SSH and GIT_SSH_COMMAND overrides and Python
startup variables; native SSH agent/config authentication is retained.


Explicit `--base` accepts only a fully qualified ref (for example,
`refs/remotes/origin/main`) or a full commit SHA. Short names such as
`origin/main` block with exit 20 because a local branch can shadow them.
Unsupported metadata path encodings also block with BASE_ERROR (exit 20).


Git/scanner credential boundary: Git children launched directly by this gate do not receive ggshield credential
variables. ggshield receives the approved API key/instance selectors, and Git
processes launched by ggshield can inherit them.
Git trace and executable proxy overrides are removed. Existing native Git
configuration and credential helpers remain trusted prerequisites. Explicit
global/system config selectors must resolve to approved absolute paths outside
this checkout and its related worktrees; unsafe selectors block without falling
back to another credential context. This gate is not an operating-system sandbox.

Metadata reads reject non-regular records, NUL path bytes and missing registered
worktree pointers. The opener is nonblocking where supported and validates its
descriptor. Grafts symlinks are confined to Git metadata. An empty explicit base
blocks rather than selecting a different destination.


Askpass selectors follow the same external absolute-path rule as explicit Git
configuration selectors; unsafe values block without changing authentication
contexts. Native approved external askpass programs remain available. Git metadata
symlinks, malformed registrations and unreadable registries block. Metadata opens
use O_NOFOLLOW where supported; platforms without it retain pre-open confinement
and descriptor checks. Concurrent replacement of parent directories is outside
the gate isolation guarantee.

The Python interpreter is a trusted bootstrap prerequisite. Use an approved
installation outside all repository checkouts. On Windows, use its absolute path
for direct invocations (for example, `& $env:MTG_SECRET_GATE_PYTHON -I scripts/secret_gate.py`).
Set that variable to the existing approved installation; it is not a credential.


Tool candidates are checked against their own Git association as well as the
registered roots. A stale or rewritten registration cannot admit a scanner from
a linked checkout. Obsolete registrations are left untouched; this gate does not
prune other tasks worktree metadata.
