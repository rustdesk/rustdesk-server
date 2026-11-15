"""Fail-closed ggshield gate for committed PR history or Git pre-push input."""

import sys

# This bootstrap uses only the built-in sys module before enforcing isolation.
if __name__ == "__main__" and not sys.flags.isolated:
    sys.stderr.write(
        "UNAVAILABLE: run this gate with Python -I; publication blocked.\n"
    )
    sys.exit(21)

import argparse
import os
import re
import shutil
import stat
import subprocess
import tempfile
from pathlib import Path

MINIMUM_PYTHON = (3, 11)
GITDIR_PREFIX = "gitdir:"


class GateError(Exception):
    """A safe, classified publication-blocking failure."""

    def __init__(self, code, message):
        """Retain the exit classification and a value-free diagnostic."""
        self.code = code
        self.message = message


def metadata_path(path, anchor):
    """Confine metadata access, including symlinks, to its allowed Git directory."""
    try:
        anchor = anchor.resolve()
        path = path.absolute()
        if any(
            p.is_symlink() for p in (path, *path.parents) if p.is_relative_to(anchor)
        ):
            raise ValueError("metadata symlink")
        resolved = path.resolve()
        relative = resolved.relative_to(anchor)
        if any(
            p.is_symlink()
            for p in (resolved, *resolved.parents)
            if p.is_relative_to(anchor)
        ):
            raise ValueError("unresolved metadata symlink")
    except (ValueError, OSError, RuntimeError) as error:
        raise GateError(
            20, "BASE_ERROR: Git metadata escapes its allowed directory."
        ) from error
    return anchor / relative


def metadata_text(path, anchor):
    """Read only a bounded, confined Git metadata record; never display it."""
    path = metadata_path(path, anchor)
    try:
        if not path.is_file():
            raise GateError(20, "BASE_ERROR: missing or non-regular Git metadata.")
        with open(path, "rb", opener=metadata_opener) as stream:
            if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                raise GateError(20, "BASE_ERROR: non-regular Git metadata.")
            data = stream.read(8193)
    except (OSError, ValueError) as error:
        raise GateError(20, "BASE_ERROR: unreadable Git metadata.") from error
    if len(data) > 8192:
        raise GateError(20, "BASE_ERROR: oversized Git metadata record.")
    if b"\0" in data:
        raise GateError(20, "BASE_ERROR: invalid Git metadata path.")
    try:
        return data.decode("utf-8").strip()
    except UnicodeError as error:
        raise GateError(
            20, "BASE_ERROR: unsupported Git metadata path encoding."
        ) from error


def metadata_opener(path, flags):
    """Avoid blocking on a FIFO swapped in before descriptor validation."""
    return os.open(
        path, flags | getattr(os, "O_NONBLOCK", 0) | getattr(os, "O_NOFOLLOW", 0)
    )


def linked_roots(common):
    """Read registered worktree pointers inside this Git metadata directory."""
    roots = set()
    registry = metadata_path(common / "worktrees", common)
    if not registry.exists():
        return roots
    if not registry.is_dir():
        raise GateError(20, "BASE_ERROR: non-directory worktree registry.")
    try:
        entries = list(registry.iterdir())
    except OSError as error:
        raise GateError(20, "BASE_ERROR: unreadable worktree registry.") from error
    for entry in entries:
        entry = metadata_path(entry, common)
        if not entry.is_dir():
            raise GateError(20, "BASE_ERROR: non-directory worktree registration.")
        pointer = entry / "gitdir"
        try:
            target = (pointer.parent / metadata_text(pointer, common)).resolve()
        except (OSError, ValueError, RuntimeError) as error:
            raise GateError(
                20, "BASE_ERROR: invalid linked-worktree metadata."
            ) from error
        if target.name != ".git":
            raise GateError(20, "BASE_ERROR: unsupported linked-worktree metadata.")
        roots.add(target.parent)
    return roots


def checkout_roots():
    """Read this checkout's standard Git metadata without executing a tool."""
    cwd = Path.cwd().resolve()
    root = next((p for p in (cwd, *cwd.parents) if (p / ".git").exists()), None)
    if root is None:
        raise GateError(20, "BASE_ERROR: run from the intended Git worktree.")
    marker = root / ".git"
    gitdir = marker
    if marker.is_file():
        text = metadata_text(marker, root)
        if not text.startswith(GITDIR_PREFIX) or "\n" in text:
            raise GateError(20, "BASE_ERROR: unsupported Git worktree metadata.")
        gitdir = (root / text[len(GITDIR_PREFIX) :].strip()).resolve()
    gitdir = gitdir.resolve()
    owner = next((p for p in (gitdir, *gitdir.parents) if p.name == ".git"), None)
    if owner is None:
        raise GateError(20, "BASE_ERROR: unsupported Git metadata layout.")
    common_file = metadata_path(gitdir / "commondir", owner)
    common = (
        metadata_path(gitdir / metadata_text(common_file, owner), owner)
        if common_file.exists()
        else gitdir
    )
    if not (common / "objects").is_dir():
        raise GateError(20, "BASE_ERROR: unsupported Git metadata layout.")
    return (
        {root, owner.parent, common, gitdir}
        | linked_roots(common)
        | linked_roots(owner)
    )


def checkout_owned(path, roots):
    """Reject code owned by this repository or its linked worktrees."""
    if any(path.is_relative_to(root) for root in roots):
        return True
    for directory in (path, *path.parents):
        marker = directory / ".git"
        if marker.is_file():
            text = metadata_text(marker, directory)
            if not text.startswith(GITDIR_PREFIX) or "\n" in text:
                raise GateError(20, "BASE_ERROR: unsupported tool-checkout metadata.")
            gitdir = (directory / text[len(GITDIR_PREFIX) :].strip()).resolve()
            if any(gitdir.is_relative_to(root) for root in roots):
                return True
    return False


def command_environment():
    """Bind children to the current checkout and approved import directories."""
    env = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith(("PYTHON", "GITGUARDIAN_", "GG_", "GIT_TRACE"))
    }
    for name in (
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
        "GIT_CEILING_DIRECTORIES",
        "GIT_DISCOVERY_ACROSS_FILESYSTEM",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_EXEC_PATH",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_PROXY_COMMAND",
        "GIT_CURL_VERBOSE",
        "PYTHONPATH",
        "PYTHONHOME",
        "PYTHONUSERBASE",
        "PYTHONSTARTUP",
        "PYTHONINSPECT",
    ):
        env.pop(name, None)
    for name in (
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "GIT_ASKPASS",
        "SSH_ASKPASS",
    ):
        value = env.get(name)
        if value is not None and os.path.normcase(value) != os.path.normcase(
            os.devnull
        ):
            path = Path(value)
            if not path.is_absolute() or checkout_owned(
                path.resolve(), checkout_roots()
            ):
                raise GateError(
                    20,
                    "BASE_ERROR: Git config/askpass override requires an approved absolute path "
                    "outside the checkout.",
                )
            env[name] = str(path.resolve())
    env["PYTHONSAFEPATH"] = "1"
    return env


def executable(name):
    """Resolve approved tools outside the checkout, including on Windows."""
    if name not in {"git", "ggshield"}:
        return None
    roots = checkout_roots()
    os.environ["NODEFAULTCURRENTDIRECTORYINEXEPATH"] = "1"
    directories = [
        str(Path(entry).resolve())
        for entry in os.get_exec_path()
        if Path(entry).is_absolute()
        and not checkout_owned(Path(entry).resolve(), roots)
    ]
    os.environ["PATH"] = os.pathsep.join(directories)
    suffixes = [""]
    if os.name == "nt":
        suffixes += os.environ.get("PATHEXT", ".COM;.EXE;.BAT;.CMD").split(os.pathsep)
    for directory in directories:
        for suffix in suffixes:
            # A path containing a directory bypasses Python 3.11's implicit cwd lookup.
            found = shutil.which(str(Path(directory) / (name + suffix)))
            if found and not checkout_owned(Path(found).resolve(), roots):
                return str(Path(found).resolve())
    return None


def git(*args):
    """Run Git without interactive authentication or replacement objects."""
    if not args or not all(re.fullmatch(r"[\w./@{}^~+:=,-]+", arg) for arg in args):
        raise GateError(20, "BASE_ERROR: invalid Git argument.")
    tool = executable("git")
    if not tool:
        raise GateError(21, "UNAVAILABLE: install approved Git outside the checkout.")
    env = dict(
        command_environment(),
        GIT_NO_REPLACE_OBJECTS="1",
        GIT_GRAFT_FILE=os.devnull,
        GIT_TERMINAL_PROMPT="0",
        GCM_INTERACTIVE="Never",
    )
    result = subprocess.run(
        [tool, *args], check=False, env=env, capture_output=True, text=True, timeout=90
    )
    if result.returncode:
        raise GateError(
            20,
            "BASE_ERROR: Git operation failed; check refs/access and fetch the base.",
        )
    return result.stdout.strip()


def commit(ref):
    """Resolve a ref to an immutable commit before selecting history."""
    if not re.fullmatch(r"(?!-)[\w./@{}^~+=,-]+", ref):
        raise GateError(
            20, "BASE_ERROR: unsupported commit reference; pass a full commit SHA."
        )
    sha = git("rev-parse", "--verify", "--end-of-options", ref + "^{commit}")
    if not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", sha):
        raise GateError(20, "BASE_ERROR: reference did not resolve to one commit.")
    return sha


def remote_base(remote):
    """Resolve the configured remote default tip without guessing a branch."""
    if not re.fullmatch(r"(?!-)[\w./@+=,-]+", remote):
        raise GateError(20, "BASE_ERROR: invalid remote name.")
    if remote not in git("remote").splitlines():
        raise GateError(
            20, "BASE_ERROR: select a configured remote or an explicit --base."
        )
    for line in git("ls-remote", "--symref", "--", remote, "HEAD").splitlines():
        parts = line.split()
        if (
            len(parts) == 2
            and parts[1] == "HEAD"
            and re.fullmatch(r"[0-9a-f]{40,64}", parts[0])
        ):
            return commit(parts[0])
    raise GateError(
        20,
        "BASE_ERROR: no remote default; supply --base or --all-history.",
    )


def base_commit(ref):
    """Require a full destination ref or commit SHA instead of an ambiguous name."""
    if not re.fullmatch(r"[0-9a-fA-F]{40}|[0-9a-fA-F]{64}", ref):
        if not ref.startswith("refs/"):
            raise GateError(20, "BASE_ERROR: --base requires a full ref or commit SHA.")
        git("check-ref-format", ref)
    return commit(ref)


def active_grafts(common):
    """Read confined graft metadata with the documented failure classification."""
    grafts = metadata_path(common / "info/grafts", common)
    try:
        return grafts.exists() and grafts.stat().st_size
    except OSError as error:
        raise GateError(20, "BASE_ERROR: unreadable graft metadata.") from error


def ranges(args, stream, head):
    """Select complete PR or pre-push commit ranges."""
    common = Path(git("rev-parse", "--git-common-dir"))
    if active_grafts(common) or os.environ.get("GIT_GRAFT_FILE"):
        raise GateError(
            20, "BASE_ERROR: grafted history is unsupported; restore complete history."
        )
    if git("rev-parse", "--is-shallow-repository") != "false":
        raise GateError(
            20, "BASE_ERROR: shallow history; fetch --unshallow before scanning."
        )
    if not args.pre_push:
        base = base_commit(args.base) if args.base is not None else None
        if not base and not args.all_history:
            base = remote_base(args.remote)
        return [f"{base}..{head}" if base else head]
    result = []
    for line in stream:
        revision = push_range(line)
        if revision:
            result.append(revision)
    return list(dict.fromkeys(result))


def push_range(line):
    """Parse one complete Git pre-push update into its outgoing range."""
    fields = line.split()
    if len(fields) != 4 or not all(
        re.fullmatch(r"[0-9a-f]{40,64}", fields[i]) for i in (1, 3)
    ):
        raise GateError(20, "BASE_ERROR: malformed Git pre-push input.")
    _, local, _, old = fields
    if set(local) == {"0"}:
        return None
    if git("cat-file", "-t", local) == "tag":
        raise GateError(
            20,
            "BASE_ERROR: annotated tags require an approved annotation scan; publication blocked.",
        )
    head = commit(local)
    # New refs include root history, without a hook commit limit or HEAD~1 guess.
    return head if set(old) == {"0"} else f"{commit(old)}..{head}"


def scan(revisions, executable):
    # Keep native authentication, but prevent config/dotenv/fail-open/logging
    # settings from changing this gate. Never persist or forward findings.
    """Run strict ggshield scans and suppress sensitive scanner diagnostics."""
    env = {
        k: v
        for k, v in command_environment().items()
        if not k.startswith(("GITGUARDIAN_", "GG_"))
    }
    for key in ("GITGUARDIAN_API_KEY", "GITGUARDIAN_INSTANCE"):
        if key in os.environ:
            env[key] = os.environ[key]
    env.update(
        GITGUARDIAN_DONT_LOAD_ENV="1",
        GITGUARDIAN_EXIT_ZERO="false",
        GITGUARDIAN_FAIL_ON_SERVER_ERROR="true",
        GIT_NO_REPLACE_OBJECTS="1",
        GIT_GRAFT_FILE=os.devnull,
        GIT_TERMINAL_PROMPT="0",
        GCM_INTERACTIVE="Never",
        GIT_DIR=git("rev-parse", "--absolute-git-dir"),
        GIT_WORK_TREE=git("rev-parse", "--show-toplevel"),
    )
    with tempfile.TemporaryDirectory(prefix="secret-gate-") as directory:
        version = subprocess.run(
            [executable, "--version"],
            check=False,
            env=env,
            cwd=directory,
            capture_output=True,
            text=True,
            timeout=30,
        )
        match = re.fullmatch(
            r"ggshield, version (\d+)\.(\d+)\.(\d+)\s*", version.stdout
        )
        if version.returncode or not match:
            raise GateError(
                21, "UNAVAILABLE: cannot verify the approved ggshield version."
            )
        if not (1, 56, 1) <= tuple(map(int, match.groups())) < (2, 0, 0):
            raise GateError(21, "UNAVAILABLE: ggshield >=1.56.1,<2 is required.")
        config = Path(directory) / "config.yaml"
        config.write_text(
            "version: 2\nexit_zero: false\nsecret:\n  show_secrets: false\n",
            encoding="utf-8",
        )
        for revision, count in revisions:
            print(f"SCANNING: {count} commit(s), {revision}", flush=True)
            # Even censored findings can include sensitive source context.
            result = subprocess.run(
                [
                    executable,
                    "--no-check-for-updates",
                    "--config-path",
                    str(config),
                    "secret",
                    "scan",
                    "commit-range",
                    "--all-secrets",
                    revision,
                ],
                check=False,
                env=env,
                cwd=directory,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=1800,
            )
            if result.returncode == 1:
                raise GateError(
                    1,
                    "FINDINGS: secrets detected; remediate before publication.",
                )
            if result.returncode == 3:
                raise GateError(
                    3,
                    "AUTH_FAILED: restore approved native ggshield authentication.",
                )
            if result.returncode:
                raise GateError(
                    4,
                    f"SCAN_ERROR: scanner exit {result.returncode}; publication blocked.",
                )
    return 0


def verify_snapshot(args, head, selected):
    """Reject a changed candidate or destination after the scan completes."""
    if not args.pre_push and head != commit("HEAD"):
        raise GateError(
            4,
            "SCAN_ERROR: HEAD changed; rerun for the new candidate.",
        )
    if not args.pre_push and not args.all_history and ".." in selected[0]:
        scanned_base = selected[0].split("..", 1)[0]
        current_base = (
            base_commit(args.base)
            if args.base is not None
            else remote_base(args.remote)
        )
        if current_base != scanned_base:
            raise GateError(
                4, "SCAN_ERROR: base changed; rerun for the new destination."
            )


def main(argv=None, stream=None):
    """Run the publication gate and return its classified exit status."""
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--base",
        help="fully qualified destination ref or full commit SHA",
    )
    mode.add_argument(
        "--all-history",
        action="store_true",
        help="scan all HEAD history for an initial/orphan publication",
    )
    mode.add_argument(
        "--pre-push",
        action="store_true",
        help="read Git ref updates from stdin; performs no push",
    )
    parser.add_argument(
        "--remote",
        default="origin",
        help="configured remote for the default PR base lookup",
    )
    args = parser.parse_args(argv)
    try:
        if sys.version_info < MINIMUM_PYTHON:
            raise GateError(
                21,
                "UNAVAILABLE: use supported Python "
                f"{MINIMUM_PYTHON[0]}.{MINIMUM_PYTHON[1]} or newer.",
            )
        if not executable("git"):
            raise GateError(21, "UNAVAILABLE: install Git before running this gate.")
        head = None if args.pre_push else commit("HEAD")
        selected = ranges(args, sys.stdin if stream is None else stream, head)
        revisions = [(r, int(git("rev-list", "--count", r, "--"))) for r in selected]
        revisions = [(r, n) for r, n in revisions if n]
        if not revisions:
            raise GateError(
                10, "SKIPPED: no outgoing commits; no clean scan is claimed."
            )
        scanner = executable("ggshield")
        if not scanner:
            raise GateError(
                21,
                "UNAVAILABLE: install approved ggshield; publication blocked.",
            )
        scan(revisions, scanner)
        verify_snapshot(args, head, selected)
        print(
            f"CLEAN: ggshield passed for {sum(n for _, n in revisions)} commit scan(s)."
        )
        return 0
    except GateError as error:
        print(error.message, file=sys.stderr)
        return error.code
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired):
        print(
            "SCAN_ERROR: command failed or timed out; publication blocked.",
            file=sys.stderr,
        )
        return 4


if __name__ == "__main__":
    sys.exit(main())
