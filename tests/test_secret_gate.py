"""Offline regression tests: real Git histories and a fake scanner, never an API."""

import contextlib
import importlib.util
import io
import os
import shlex
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

GATE = Path(__file__).resolve().parents[1] / "scripts" / "secret_gate.py"
SPEC = importlib.util.spec_from_file_location("secret_gate", GATE)
secret_gate = importlib.util.module_from_spec(SPEC)
sys.modules["secret_gate"] = secret_gate
SPEC.loader.exec_module(secret_gate)
FAKE = """import os, subprocess, sys
sys.excepthook = lambda *_: os._exit(97)
if sys.flags.optimize:
    raise RuntimeError("fixture requires enabled assertions")
from pathlib import Path
args = sys.argv[1:]
assert not Path.cwd().resolve().is_relative_to(
    Path(os.environ["GIT_WORK_TREE"]).resolve()
)
if args == ["--version"]:
    print("ggshield, version " + os.environ.get("GATE_VERSION", "1.56.1"))
    sys.exit(0)
if os.environ.get("GATE_BREAK_FIXTURE"):
    raise RuntimeError("fixture precondition failure")
assert not any(
    Path(entry).resolve().is_relative_to(Path.cwd().resolve())
    for entry in os.get_exec_path()
)
assert "--all-secrets" in args
assert os.environ["GITGUARDIAN_EXIT_ZERO"] == "false"
assert os.environ["GITGUARDIAN_DONT_LOAD_ENV"] == "1"
assert "GITGUARDIAN_LOG_FILE" not in os.environ
assert not any(name in os.environ for name in (
    "PYTHONPATH", "PYTHONHOME", "PYTHONUSERBASE", "PYTHONSTARTUP", "PYTHONINSPECT",
    "GIT_EXEC_PATH", "GIT_SSH", "GIT_SSH_COMMAND", "GIT_PROXY_COMMAND",
    "GIT_TRACE2_EVENT", "GIT_TRACE2_ENV_VARS",
))
assert "ignored_paths" not in Path(args[args.index("--config-path")+1]).read_text()
revision = args[-1]
with open(os.environ["GATE_CALLS"], "a", encoding="utf-8") as calls:
    calls.write(revision + "\\n")
if "GATE_CODE" in os.environ:
    print("sensitive-output-must-stay-private")
    sys.exit(int(os.environ["GATE_CODE"]))
commits = subprocess.check_output(["git", "rev-list", revision], text=True)
for sha in commits.splitlines():
    diff = subprocess.check_output(["git", "show", sha], text=True)
    if "synthetic-secret-marker" in diff:
        sys.exit(1)
sys.exit(0)
"""

PYTHON_FLOOR_CASES = (((3, 10), 21), ((3, 11), 0), ((3, 12), 0))


class SecretGateTests(unittest.TestCase):
    """Exercise publication blocking with real Git and a fake scanner."""

    def setUp(self):
        """Create a disposable repository, bare remote and synthetic scanner."""
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.env = {
            k: v
            for k, v in os.environ.items()
            if not k.startswith(("GIT_", "GITGUARDIAN_", "GG_"))
        }
        self.env.update(
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_AUTHOR_NAME="Fixture",
            GIT_AUTHOR_EMAIL="fixture@example.invalid",
            GIT_COMMITTER_NAME="Fixture",
            GIT_COMMITTER_EMAIL="fixture@example.invalid",
        )
        self.git("init", "-q", "-b", "main")
        self.base = self.commit("base")
        self.git("clone", "--bare", "-q", ".", str(self.root / "remote.git"))
        self.git("remote", "add", "origin", str(self.root / "remote.git"))
        self.git("switch", "-q", "-c", "feature")
        self.head = self.commit("feature")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        fake = self.bin / "ggshield"
        (self.bin / "fake.py").write_text(FAKE)
        if os.name == "nt":
            fake.with_suffix(".cmd").write_text(
                f'@"{sys.executable}" -I "{self.bin / "fake.py"}" %*\n'
            )
        else:
            fake.write_text(
                f"#!/bin/sh\nexec {shlex.quote(sys.executable)} -I "
                f'{shlex.quote(str(self.bin / "fake.py"))} "$@"\n'
            )
            fake.chmod(0o700)
        self.calls = self.root / "calls"
        self.env.update(
            PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
            GATE_CALLS=str(self.calls),
        )

    def git(self, *args):
        """Run Git without interactive authentication or replacement objects."""
        return subprocess.check_output(
            ["git", *args],
            cwd=self.repo,
            env=self.env,
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()

    def commit(self, content):
        """Resolve a ref to an immutable commit before selecting history."""
        (self.repo / "file.txt").write_text(content)
        self.git("add", "file.txt")
        self.git("commit", "-q", "-m", "fixture", "--no-gpg-sign")
        return self.git("rev-parse", "HEAD")

    def run_gate(self, *args, input_text=None):
        """Execute the gate without inheriting real scanner credentials."""
        return subprocess.run(
            [sys.executable, "-I", str(GATE), *args],
            check=False,
            cwd=self.repo,
            env=self.env,
            input=input_text,
            text=True,
            capture_output=True,
        )

    def test_new_branch_without_upstream_uses_remote_default(self):
        """Prove new branch without upstream uses remote default."""
        result = self.run_gate()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("CLEAN:", result.stdout)
        self.assertEqual(
            self.calls.read_text().splitlines(), [f"{self.base}..{self.head}"]
        )

    def test_feature_upstream_does_not_hide_pr_commits(self):
        """Prove feature upstream does not hide pr commits."""
        self.git("update-ref", "refs/remotes/origin/feature", self.head)
        self.git("config", "branch.feature.remote", "origin")
        self.git("config", "branch.feature.merge", "refs/heads/feature")
        self.assertEqual(self.run_gate().returncode, 0)
        self.assertEqual(
            self.calls.read_text().splitlines(), [f"{self.base}..{self.head}"]
        )

    def test_added_then_removed_secret_is_in_history(self):
        """Prove added then removed secret is in history."""
        first = self.commit("synthetic-secret-marker")
        self.commit("feature")
        self.assertEqual(self.git("diff", self.head, "HEAD"), "")
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 1)
        self.assertIn(first, self.git("rev-list", self.calls.read_text().strip()))
        self.assertNotIn("synthetic-secret-marker", result.stdout + result.stderr)

    def test_non_main_base_and_detached_head(self):
        """Prove non main base and detached head."""
        self.git("checkout", "-q", "--detach")
        self.assertEqual(self.run_gate("--base", self.base).returncode, 0)

    def test_missing_base_is_not_clean(self):
        """Prove missing base is not clean."""
        self.assertEqual(self.run_gate("--base", "missing").returncode, 20)
        self.assertFalse(self.calls.exists())

    def test_empty_range_is_skipped(self):
        """Prove empty range is skipped."""
        result = self.run_gate("--base", self.head)
        self.assertEqual(result.returncode, 10)
        self.assertIn("SKIPPED:", result.stderr)
        self.assertFalse(self.calls.exists())

    def test_auth_scan_errors_and_output_redaction(self):
        """Prove auth scan errors and output redaction."""
        for scanner_code, gate_code in ((1, 1), (3, 3), (2, 4), (4, 4), (128, 4)):
            with self.subTest(scanner_code=scanner_code):
                self.env["GATE_CODE"] = str(scanner_code)
                result = self.run_gate("--base", self.base)
                self.assertEqual(result.returncode, gate_code)
                self.assertNotIn("sensitive-output", result.stdout + result.stderr)

    def test_fail_open_environment_and_dotenv_cannot_bypass(self):
        """Prove fail open environment and dotenv cannot bypass."""
        self.env.update(GITGUARDIAN_EXIT_ZERO="true", GITGUARDIAN_LOG_FILE="unsafe.log")
        (self.repo / ".env").write_text("GITGUARDIAN_EXIT_ZERO=true\n")
        self.assertEqual(self.run_gate("--base", self.base).returncode, 0)
        self.assertFalse((self.repo / "unsafe.log").exists())

    def test_missing_scanner_blocks(self):
        """Prove missing scanner blocks."""
        with (
            patch("secret_gate.executable", side_effect=["git", None]),
            patch("secret_gate.commit", return_value=self.head),
            patch("secret_gate.ranges", return_value=[self.head]),
            patch("secret_gate.git", return_value="1"),
        ):
            self.assertEqual(secret_gate.main(["--base", self.base]), 21)

    def test_missing_git_blocks(self):
        """An absent Git executable must not claim a successful scan."""
        with patch("secret_gate.executable", return_value=None):
            self.assertEqual(secret_gate.main(["--base", self.base]), 21)

    def test_shallow_history_blocks(self):
        """Prove shallow history blocks."""
        (self.repo / ".git/shallow").write_text(self.base + "\n")
        self.assertEqual(self.run_gate("--base", self.base).returncode, 20)

    def test_no_remote_requires_explicit_choice_and_includes_root(self):
        """Prove no remote requires explicit choice and includes root."""
        self.git("remote", "remove", "origin")
        self.assertEqual(self.run_gate().returncode, 20)
        self.assertEqual(self.run_gate("--all-history").returncode, 0)
        self.assertEqual(self.calls.read_text().splitlines(), [self.head])
        self.assertIn(self.base, self.git("rev-list", self.calls.read_text().strip()))

    def test_remote_base_object_must_be_fetched(self):
        """Prove remote base object must be fetched."""
        stale = self.root / "stale"
        self.git(
            "clone", "--no-hardlinks", "-q", str(self.root / "remote.git"), str(stale)
        )
        self.git("push", "-q", "origin", "HEAD:main")
        result = subprocess.run(
            [sys.executable, "-I", str(GATE)],
            check=False,
            cwd=stale,
            env=self.env,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 20)
        self.assertFalse(self.calls.exists())

    def test_short_base_cannot_be_shadowed_by_a_local_branch(self):
        """Use the complete destination ref when a local branch shadows origin."""
        self.git("fetch", "origin")
        self.commit("synthetic-secret-marker")
        hidden = self.commit("removed synthetic marker")
        self.commit("extra clean feature")
        self.git("branch", "origin/main", hidden)
        short = self.run_gate("--base", "origin/main")
        self.assertEqual(short.returncode, 20, short.stderr)
        self.assertFalse(self.calls.exists())
        exact = self.run_gate("--base", "refs/remotes/origin/main")
        self.assertEqual(exact.returncode, 1, exact.stderr)
        self.assertTrue(self.calls.exists())

    def test_invalid_metadata_encoding_is_a_base_error(self):
        """Unsupported path bytes block with the documented metadata outcome."""
        directory = self.repo / ".git/worktrees/invalid-encoding"
        directory.mkdir(parents=True)
        (directory / "gitdir").write_bytes(b"../bad-\xff/.git\n")
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertFalse(self.calls.exists())

    def test_prepush_multiple_refs_and_new_branch(self):
        """Prove prepush multiple refs and new branch."""
        updates = f"refs/heads/feature {self.head} refs/heads/feature {self.base}\n"
        updates += f"refs/heads/other {self.head} refs/heads/other {'0' * 40}\n"
        result = self.run_gate("--pre-push", input_text=updates)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.calls.read_text().splitlines(),
            [f"{self.base}..{self.head}", self.head],
        )

    def test_prepush_with_unborn_checkout_scans_the_pushed_ref(self):
        """Pushing an existing ref does not depend on the checked-out HEAD."""
        self.git("switch", "--orphan", "unborn")
        updates = f"refs/heads/feature {self.head} refs/heads/feature {self.base}\n"
        result = self.run_gate("--pre-push", input_text=updates)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.calls.read_text().splitlines(), [f"{self.base}..{self.head}"]
        )

    def test_grafted_history_blocks(self):
        """Legacy grafts must not hide a secret-bearing ancestor."""
        self.commit("synthetic-secret-marker")
        final = self.commit("clean-again")
        (self.repo / ".git/info/grafts").write_text(f"{final} {self.head}\n")
        self.assertEqual(self.run_gate("--base", self.base).returncode, 20)
        self.assertFalse(self.calls.exists())

    def test_environment_grafted_history_blocks(self):
        """An environment-selected graft file must not hide history."""
        grafts = self.root / "external-grafts"
        grafts.write_text(f"{self.head}\n")
        self.env["GIT_GRAFT_FILE"] = str(grafts)
        self.assertEqual(self.run_gate("--base", self.base).returncode, 20)
        self.assertFalse(self.calls.exists())

    def test_prepush_delete_malformed_and_unknown_tip(self):
        """Prove prepush delete malformed and unknown tip."""
        cases = [
            (f"(delete) {'0' * 40} refs/heads/old {self.base}\n", 10),
            ("bad\n", 20),
            (f"refs/heads/feature {self.head} refs/heads/feature {'f' * 40}\n", 20),
        ]
        for updates, expected in cases:
            self.assertEqual(
                self.run_gate("--pre-push", input_text=updates).returncode, expected
            )

    def test_changed_head_does_not_claim_clean(self):
        """Prove changed head does not claim clean."""
        with (
            patch("secret_gate.executable", return_value="tool"),
            patch("secret_gate.commit", side_effect=[self.head, self.base]),
            patch("secret_gate.ranges", return_value=[self.head]),
            patch("secret_gate.git", return_value="1"),
            patch("secret_gate.scan", return_value=0),
        ):
            self.assertEqual(secret_gate.main(["--base", self.base]), 4)

    def test_timeout_does_not_claim_clean(self):
        """Prove timeout does not claim clean."""
        with (
            patch("secret_gate.executable", return_value="tool"),
            patch("secret_gate.commit", return_value=self.head),
            patch("secret_gate.ranges", return_value=[self.head]),
            patch("secret_gate.git", return_value="1"),
            patch(
                "secret_gate.scan",
                side_effect=subprocess.TimeoutExpired("scanner", 1800),
            ),
        ):
            self.assertEqual(secret_gate.main(["--base", self.base]), 4)

    def test_force_push_scans_unique_new_history(self):
        """Prove force push scans unique new history."""
        self.git("switch", "-q", "-c", "rewritten", self.base)
        rewritten = self.commit("rewritten")
        updates = f"refs/heads/feature {rewritten} refs/heads/feature {self.head}\n"
        self.assertEqual(self.run_gate("--pre-push", input_text=updates).returncode, 0)
        self.assertEqual(
            self.calls.read_text().splitlines(), [f"{self.head}..{rewritten}"]
        )

    def test_merged_side_branch_commits_are_included(self):
        """Prove merged side branch commits are included."""
        self.git("switch", "-q", "-c", "side", self.base)
        secret = self.commit("synthetic-secret-marker")
        self.commit("side-clean")
        self.git("switch", "-q", "feature")
        # Resolve this synthetic merge without dropping the side branch ancestry.
        self.git("merge", "--no-ff", "-s", "ours", "-m", "fixture merge", "side")
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 1)
        self.assertIn(secret, self.git("rev-list", self.calls.read_text().strip()))

    def test_option_like_ref_rejected_before_git(self):
        """Reject agent-supplied option arguments before command invocation."""
        with patch("secret_gate.git") as command:
            with self.assertRaises(secret_gate.GateError):
                secret_gate.commit("--help")
            command.assert_not_called()

    def test_option_like_remote_rejected_before_git(self):
        """Reject option-like remote names even if a repo config lists one."""
        with patch("secret_gate.git", return_value="--heads") as command:
            with self.assertRaises(secret_gate.GateError):
                secret_gate.remote_base("--heads")
            command.assert_not_called()

    def test_git_boundary_rejects_unexpected_argument_characters(self):
        """Prevent arbitrary agent arguments from reaching the Git process."""
        with patch("secret_gate.subprocess.run") as command:
            with self.assertRaises(secret_gate.GateError):
                secret_gate.git("ls-remote", "--symref", "bad;command", "HEAD")
            command.assert_not_called()

    def test_replace_refs_cannot_hide_outgoing_commits(self):
        """Prove replace refs cannot hide outgoing commits."""
        self.commit("synthetic-secret-marker")
        final = self.commit("clean-again")
        self.git("replace", "--graft", final, self.head)
        self.assertEqual(self.run_gate("--base", self.base).returncode, 1)

    def test_checkout_executable_cannot_claim_clean(self):
        """A checkout-owned decoy must not replace the approved scanner."""
        self.commit("synthetic-secret-marker")
        decoy = self.repo / ("ggshield.cmd" if os.name == "nt" else "ggshield")
        decoy.write_text("@exit /b 0\n" if os.name == "nt" else "#!/bin/sh\nexit 0\n")
        decoy.chmod(0o700)
        self.env["PATH"] = str(self.repo) + os.pathsep + self.env["PATH"]
        # Older Windows lookup can reject the decoy rather than find the next tool.
        expected = (1,)
        self.assertIn(self.run_gate("--base", self.base).returncode, expected)

    def test_implicit_current_directory_tool_is_rejected(self):
        """Reject a tool returned through Windows' implicit cwd lookup."""
        with (
            patch.dict(os.environ),
            patch("secret_gate.Path.cwd", return_value=self.repo),
            patch(
                "secret_gate.shutil.which", return_value=str(self.repo / "ggshield.cmd")
            ),
        ):
            self.assertIsNone(secret_gate.executable("ggshield"))

    def test_git_exec_path_cannot_load_checkout_transport_helpers(self):
        """An inherited helper directory must not execute checkout code."""
        helpers = self.repo / "transport-helpers"
        helpers.mkdir()
        marker = self.root / "helper-executed"
        program = helpers / "helper.py"
        program.write_text(
            "import sys\nfrom pathlib import Path\n"
            f"Path({str(marker)!r}).write_text('executed')\n"
            "for command in sys.stdin:\n"
            "    if command.strip() == 'capabilities':\n"
            "        print('', flush=True)\n"
            "    elif command.startswith('list'):\n"
            f"        print({self.base!r} + ' HEAD\\n', flush=True)\n"
        )
        helper = helpers / "git-remote-gateprobe"
        if os.name == "nt":
            helper.with_suffix(".cmd").write_text(
                f'@"{sys.executable}" -I "{program}" %*\n'
            )
        else:
            helper.write_text(
                f"#!/bin/sh\nexec {shlex.quote(sys.executable)} -I "
                f'{shlex.quote(str(program))} "$@"\n'
            )
            helper.chmod(0o700)
        self.git("remote", "set-url", "origin", "gateprobe::fixture")
        self.env["GIT_EXEC_PATH"] = str(helpers)
        result = self.run_gate()
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertFalse(marker.exists())
        self.assertFalse(self.calls.exists())

    def test_transport_command_overrides_are_removed(self):
        """Keep native SSH agent authentication without injected executables."""
        env = {
            "GIT_EXEC_PATH": "unsafe",
            "GIT_SSH": "unsafe",
            "GIT_SSH_COMMAND": "unsafe",
            "GIT_PROXY_COMMAND": "unsafe",
            "SSH_AUTH_SOCK": "native-agent",
        }
        with patch.dict(os.environ, env, clear=True):
            clean = secret_gate.command_environment()
        self.assertFalse(
            any(
                name in clean
                for name in (
                    "GIT_EXEC_PATH",
                    "GIT_SSH",
                    "GIT_SSH_COMMAND",
                    "GIT_PROXY_COMMAND",
                )
            )
        )
        self.assertEqual(clean["SSH_AUTH_SOCK"], "native-agent")

    def test_relative_linked_pointer_excludes_sibling_decoy(self):
        """Model Git's relative registration format on older Git versions too."""
        self.commit("synthetic-secret-marker")
        linked = self.root / "relative-linked"
        self.git("worktree", "add", "--detach", str(linked), "HEAD")
        pointer = next((self.repo / ".git/worktrees").glob("*/gitdir"))
        pointer.write_text(os.path.relpath(linked / ".git", pointer.parent) + "\n")
        self.scanner_decoy(linked)
        self.env["PATH"] = str(linked) + os.pathsep + self.env["PATH"]
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertTrue(self.calls.exists())

    def test_metadata_symlink_loop_is_classified(self):
        """A metadata loop blocks without a traceback or findings classification."""
        loop = self.repo / ".git/commondir"
        try:
            loop.symlink_to(loop.name)
        except OSError:
            self.skipTest("symlink creation is unavailable")
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertFalse(self.calls.exists())

    def scanner_decoy(self, directory):
        """Create a version-compatible no-scan decoy to prove tool exclusion."""
        decoy = directory / ("ggshield.cmd" if os.name == "nt" else "ggshield")
        if os.name == "nt":
            decoy.write_text(
                '@if "%~1"=="--version" echo ggshield, version 1.56.1\n@exit /b 0\n'
            )
        else:
            decoy.write_text(
                '#!/bin/sh\nif [ "$1" = "--version" ]; then\n'
                'echo "ggshield, version 1.56.1"\nfi\nexit 0\n'
            )
        decoy.chmod(0o700)

    def test_nested_submodule_excludes_enclosing_linked_worktree(self):
        """Protect both an enclosing linked checkout and its nested submodule."""
        self.commit("synthetic-secret-marker")
        parent = self.root / "parent"
        parent.mkdir()
        self.git("-C", str(parent), "init", "-q")
        self.git(
            "-C",
            str(parent),
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            str(self.repo),
            "modules/child",
        )
        self.git("-C", str(parent), "commit", "-q", "-m", "fixture", "--no-gpg-sign")
        linked = self.root / "parent-linked"
        self.git("-C", str(parent), "worktree", "add", "--detach", str(linked), "HEAD")
        self.git(
            "-C",
            str(linked),
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
        )
        self.scanner_decoy(linked)
        child = linked / "modules/child"
        self.env["PATH"] = str(linked) + os.pathsep + self.env["PATH"]
        result = subprocess.run(
            [sys.executable, "-I", str(GATE), "--base", self.base],
            cwd=child,
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertTrue(self.calls.exists())

    def test_fake_scanner_assertions_survive_python_optimization(self):
        """Optimized parent environments must not weaken the scanner fixture."""
        self.env["PYTHONOPTIMIZE"] = "1"
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.calls.exists())

    def test_git_environment_cannot_select_a_different_repository(self):
        """Scan the current checkout despite ambient repository selectors."""
        self.commit("synthetic-secret-marker")
        self.env.update(
            GIT_DIR=str(self.root / "remote.git"),
            GIT_WORK_TREE=str(self.root),
            GIT_COMMON_DIR=str(self.root / "remote.git"),
        )
        self.assertEqual(self.run_gate("--base", self.base).returncode, 1)

    def test_scanner_python_import_path_cannot_load_checkout_code(self):
        """Check both scanner invocations reject checkout import overrides."""
        marker = self.repo / "shadow-imported"
        (self.repo / "sitecustomize.py").write_text(
            f"from pathlib import Path\nPath({str(marker)!r}).touch()\n"
        )
        self.env["PYTHONPATH"] = (
            str(self.repo) + os.pathsep + self.env.get("PYTHONPATH", "")
        )
        # Run the gate without startup imports, then set the scanner's input.
        program = (
            "import os, runpy, sys; "
            f"os.environ['PYTHONPATH'] = {self.env['PYTHONPATH']!r}; "
            f"sys.argv = [{str(GATE)!r}, '--base', {self.base!r}]; "
            f"runpy.run_path({str(GATE)!r}, run_name='__main__')"
        )
        clean_env = dict(self.env)
        clean_env.pop("PYTHONPATH", None)
        result = subprocess.run(
            [sys.executable, "-I", "-c", program],
            cwd=self.repo,
            env=clean_env,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(marker.exists())

    def test_linked_worktree_rejects_tools_from_main_checkout(self):
        """A main-checkout decoy cannot become a linked worktree's scanner."""
        self.commit("synthetic-secret-marker")
        linked = self.root / "linked"
        self.git("worktree", "add", "--detach", str(linked), "HEAD")
        decoy = self.repo / ("ggshield.cmd" if os.name == "nt" else "ggshield")
        decoy.write_text("@exit /b 0\n" if os.name == "nt" else "#!/bin/sh\nexit 0\n")
        decoy.chmod(0o700)
        env = dict(self.env, PATH=str(self.repo) + os.pathsep + self.env["PATH"])
        result = subprocess.run(
            [sys.executable, "-I", str(GATE), "--base", self.base],
            cwd=linked,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        expected = (1,)
        self.assertIn(result.returncode, expected, result.stderr)

    def test_annotated_tags_are_blocked_before_scanner_success(self):
        """Never claim coverage for unscanned annotation text."""
        self.git("tag", "-a", "-m", "synthetic-secret-marker", "release")
        tag = self.git("rev-parse", "release")
        result = self.run_gate(
            "--pre-push",
            input_text=f"refs/tags/release {tag} refs/tags/release {'0' * 40}\n",
        )
        self.assertEqual(result.returncode, 20)
        self.assertIn("annotated tags", result.stderr)
        self.assertFalse(self.calls.exists())

    def test_valid_equal_ref_and_at_remote_are_supported(self):
        """Support ordinary configured names without accepting shell syntax."""
        self.git("branch", "fix=api", self.base)
        self.git("branch", "fix,api", self.base)
        self.git("remote", "add", "team@code", str(self.root / "remote.git"))
        self.assertEqual(self.run_gate("--base", "refs/heads/fix=api").returncode, 0)
        self.assertEqual(self.run_gate("--remote", "team@code").returncode, 0)
        self.git("remote", "add", "team+mirror", str(self.root / "remote.git"))
        self.assertEqual(self.run_gate("--remote", "team+mirror").returncode, 0)
        self.assertEqual(self.run_gate("--base", "refs/heads/fix,api").returncode, 0)
        self.git("remote", "add", "team=code", str(self.root / "remote.git"))
        self.assertEqual(self.run_gate("--remote", "team=code").returncode, 0)

    def test_scan_uses_the_original_resolved_head(self):
        """A ref change cannot switch the immutable commit sent to the scanner."""
        with (
            patch("secret_gate.executable", return_value="tool"),
            patch("secret_gate.commit", side_effect=[self.head, self.base, self.head]),
            patch(
                "secret_gate.git", side_effect=[str(self.repo / ".git"), "false", "1"]
            ),
            patch("secret_gate.scan", return_value=0) as scanner,
        ):
            self.assertEqual(secret_gate.main(["--all-history"]), 4)
            scanner.assert_called_once_with([(self.head, 1)], "tool")

    def test_explicit_release_base_includes_default_branch_history(self):
        """A release PR must scan commits already present on the default branch."""
        self.commit("synthetic-secret-marker")
        self.git("push", "-q", "origin", "HEAD:main")
        (self.repo / "other.txt").write_text("unrelated change")
        self.git("add", "other.txt")
        self.git("commit", "-q", "-m", "fixture", "--no-gpg-sign")
        self.assertEqual(self.run_gate().returncode, 0)
        self.assertEqual(self.run_gate("--base", self.base).returncode, 1)

    def test_unsupported_python_blocks_before_tool_lookup(self):
        """An unsupported gate runtime must not start a publication scan."""
        with (
            patch("secret_gate.sys.version_info", (3, 10, 20)),
            patch("secret_gate.executable") as lookup,
        ):
            self.assertEqual(secret_gate.main(["--all-history"]), 21)
            lookup.assert_not_called()

    def test_metadata_symlink_escape_blocks_before_scanning(self):
        """A linked-worktree pointer must not read outside Git metadata."""
        external = self.root / "external-metadata"
        external.write_text(str(self.root / "elsewhere/.git"))
        pointer = self.repo / ".git/worktrees/escape/gitdir"
        pointer.parent.mkdir(parents=True)
        try:
            pointer.symlink_to(external)
        except OSError:
            if os.name == "nt":
                self.skipTest("File symlinks are unavailable to this Windows test user")
            raise
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertIn("metadata escapes", result.stderr)
        self.assertFalse(self.calls.exists())

    def test_remote_base_change_blocks_after_scanning(self):
        """Do not claim clean against a remote base that changed during the scan."""
        previous = Path.cwd()
        try:
            os.chdir(self.repo)
            with (
                patch.dict(os.environ, self.env, clear=True),
                patch("secret_gate.remote_base", side_effect=[self.base, self.head]),
                patch("secret_gate.scan", return_value=0),
            ):
                self.assertEqual(secret_gate.main([]), 4)
        finally:
            os.chdir(previous)

    def test_fixture_failure_is_not_reported_as_findings(self):
        """A fixture error must not satisfy a secret-detection assertion."""
        self.env["GATE_BREAK_FIXTURE"] = "1"
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 4)
        self.assertFalse(self.calls.exists())

    def test_unrelated_git_managed_tool_installation_is_allowed(self):
        """Approved external installations can themselves be Git-managed."""
        self.git("init", "-q", str(self.bin))
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_linked_submodule_worktree_uses_its_metadata_owner(self):
        """Support submodule metadata without accepting its main-worktree decoy."""
        self.commit("synthetic-secret-marker")
        parent = self.root / "parent"
        parent.mkdir()
        self.git("-C", str(parent), "init", "-q")
        self.git(
            "-C",
            str(parent),
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            str(self.repo),
            "modules/child",
        )
        child = parent / "modules/child"
        linked = self.root / "submodule-linked"
        self.git("-C", str(child), "worktree", "add", "--detach", str(linked), "HEAD")
        decoy = child / ("ggshield.cmd" if os.name == "nt" else "ggshield")
        decoy.write_text("@exit /b 0\n" if os.name == "nt" else "#!/bin/sh\nexit 0\n")
        decoy.chmod(0o700)
        env = dict(self.env, PATH=str(child) + os.pathsep + self.env["PATH"])
        result = subprocess.run(
            [sys.executable, "-I", str(GATE), "--base", self.base],
            cwd=linked,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertTrue(self.calls.exists())

    def test_gate_startup_cannot_import_checkout_shadow_modules(self):
        """The documented isolated command ignores a sibling stdlib decoy."""
        scripts = self.repo / "scripts"
        scripts.mkdir()
        copied_gate = scripts / "secret_gate.py"
        copied_gate.write_text(GATE.read_text())
        (scripts / "argparse.py").write_text(
            "import sys\nprint('CLEAN: decoy')\nsys.exit(0)\n"
        )
        for isolated, expected in ((False, 21), (True, 0)):
            command = [sys.executable, *(["-I"] if isolated else []), str(copied_gate)]
            result = subprocess.run(
                [*command, "--base", self.base],
                cwd=self.repo,
                env=self.env,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, expected, result.stderr)
            self.assertNotIn("CLEAN: decoy", result.stdout)

    def test_unsupported_scanner_version_blocks(self):
        """Unknown scanner contracts must not yield a clean publication result."""
        self.env["GATE_VERSION"] = "1.55.0"
        self.assertEqual(self.run_gate("--base", self.base).returncode, 21)
        self.assertFalse(self.calls.exists())

    def test_empty_explicit_base_does_not_fall_back(self):
        """An empty wrapper argument must not select a different destination."""
        result = self.run_gate("--base", "")
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertFalse(self.calls.exists())

    def test_git_tracing_does_not_receive_scanner_credentials(self):
        """A synthetic credential reaches the scanner without a Git trace file."""
        trace = self.root / "git-trace.json"
        marker = "synthetic-auth-context-marker"
        self.env.update(
            GITGUARDIAN_API_KEY=marker,
            GIT_TRACE2_EVENT=str(trace),
            GIT_TRACE2_ENV_VARS="GITGUARDIAN_API_KEY",
        )
        fake = self.bin / "fake.py"
        fake.write_text(
            fake.read_text().replace(
                "args = sys.argv[1:]",
                f"assert os.environ.get('GITGUARDIAN_API_KEY') == {marker!r}\nargs = sys.argv[1:]",
            )
        )
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.calls.exists())
        self.assertFalse(trace.exists())

    def test_nonregular_metadata_cannot_block_the_gate(self):
        """An owned FIFO fixture must produce a classified error promptly."""
        if not hasattr(os, "mkfifo"):
            self.skipTest("FIFO creation is unavailable")
        pointer = self.repo / ".git/worktrees/fifo/gitdir"
        pointer.parent.mkdir(parents=True)
        os.mkfifo(pointer)
        result = subprocess.run(
            [sys.executable, "-I", str(GATE), "--base", self.base],
            cwd=self.repo,
            env=self.env,
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
        )
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertFalse(self.calls.exists())

    def test_missing_worktree_pointer_cannot_admit_a_decoy(self):
        """Incomplete registration must not turn a sibling into a trusted tool."""
        self.commit("synthetic-secret-marker")
        linked = self.root / "missing-pointer-linked"
        self.git("worktree", "add", "--detach", str(linked), "HEAD")
        next((self.repo / ".git/worktrees").glob("*/gitdir")).unlink()
        self.scanner_decoy(linked)
        self.env["PATH"] = str(linked) + os.pathsep + self.env["PATH"]
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertFalse(self.calls.exists())

    def test_escaping_grafts_symlink_is_not_accepted(self):
        """Even an empty external grafts target is outside approved metadata."""
        target = self.root / "external-empty"
        target.touch()
        grafts = self.repo / ".git/info/grafts"
        try:
            grafts.symlink_to(target)
        except OSError:
            self.skipTest("symlink creation is unavailable")
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertFalse(self.calls.exists())

    def test_malformed_metadata_is_classified_before_scanning(self):
        """NUL path bytes are metadata errors, without raw diagnostics."""
        pointer = self.repo / ".git/worktrees/invalid-path-record/gitdir"
        pointer.parent.mkdir(parents=True)
        pointer.write_bytes(b"bad\0path/.git\n")
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertFalse(self.calls.exists())
        self.assertNotIn("Traceback", result.stderr)

    def test_checkout_config_override_cannot_redirect_the_base(self):
        """A checkout config cannot hide history by redirecting the destination."""
        self.commit("synthetic-secret-marker")
        self.commit("removed marker")
        alternate = self.root / "alternate.git"
        self.git("clone", "--bare", "-q", ".", str(alternate))
        self.commit("extra clean feature")
        config = self.repo / "untrusted-global.cfg"
        config.write_text(
            f'[url "{alternate.as_posix()}"]\n'
            f"\tinsteadOf = {(self.root / 'remote.git').as_posix()}\n"
        )
        self.env["GIT_CONFIG_GLOBAL"] = str(config)
        result = self.run_gate()
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertFalse(self.calls.exists())

    def test_non_directory_registration_cannot_admit_a_decoy(self):
        """A corrupted registration must not hide a sibling-owned scanner."""
        self.commit("synthetic-secret-marker")
        linked = self.root / "file-registration-linked"
        self.git("worktree", "add", "--detach", str(linked), "HEAD")
        entry = next((self.repo / ".git/worktrees").iterdir())
        entry.rename(self.root / "saved-registration")
        entry.write_text("corrupted registration")
        self.scanner_decoy(linked)
        self.env["PATH"] = str(linked) + os.pathsep + self.env["PATH"]
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertFalse(self.calls.exists())

    def test_dangling_registry_symlink_cannot_admit_a_decoy(self):
        """A replaced registry is not equivalent to no linked worktrees."""
        self.commit("synthetic-secret-marker")
        linked = self.root / "dangling-registry-linked"
        self.git("worktree", "add", "--detach", str(linked), "HEAD")
        registry = self.repo / ".git/worktrees"
        registry.rename(self.root / "saved-registry")
        try:
            registry.symlink_to(
                self.repo / ".git/missing-registry", target_is_directory=True
            )
        except OSError:
            self.skipTest("symlink creation is unavailable")
        self.scanner_decoy(linked)
        self.env["PATH"] = str(linked) + os.pathsep + self.env["PATH"]
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertFalse(self.calls.exists())

    @unittest.skipUnless(
        os.name == "posix" and os.geteuid() != 0, "POSIX permissions required"
    )
    def test_unreadable_registry_is_a_base_error(self):
        """Unreadable registration directories have the metadata classification."""
        registry = self.repo / ".git/worktrees"
        registry.mkdir()
        registry.chmod(0)
        try:
            result = self.run_gate("--base", self.base)
        finally:
            registry.chmod(0o700)
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertFalse(self.calls.exists())

    def test_non_regular_commondir_does_not_fall_back(self):
        """Existing malformed common-directory records cannot be ignored."""
        (self.repo / ".git/commondir").mkdir()
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 20, result.stderr)
        self.assertFalse(self.calls.exists())

    @unittest.skipUnless(
        hasattr(os, "O_NOFOLLOW"), "No-follow descriptor support required"
    )
    def test_metadata_open_rejects_a_replaced_final_symlink(self):
        """A final path swapped after confinement must not read outside data."""
        pointer = self.repo / ".git/pointer"
        pointer.write_text("original metadata")
        outside = self.root / "external-pointer"
        outside.write_text("synthetic external metadata")
        opener = secret_gate.metadata_opener

        def replace_then_open(path, flags):
            pointer.unlink()
            pointer.symlink_to(outside)
            return opener(path, flags)

        with (
            patch.object(secret_gate, "metadata_opener", side_effect=replace_then_open),
            self.assertRaises(secret_gate.GateError) as error,
        ):
            secret_gate.metadata_text(pointer, self.repo / ".git")
        self.assertEqual(error.exception.code, 20)

    def test_askpass_selection_requires_external_absolute_paths(self):
        """Reject checkout helpers without changing approved native authentication."""
        for name in ("GIT_ASKPASS", "SSH_ASKPASS"):
            for value in ("relative-helper", str(self.repo / "helper")):
                with self.subTest(name=name, value=value):
                    self.env[name] = value
                    result = self.run_gate("--base", self.base)
                    self.assertEqual(result.returncode, 20, result.stderr)
                    self.assertFalse(self.calls.exists())
            self.env.pop(name)
        approved = self.root / "approved-askpass"
        approved.write_text("synthetic native helper fixture")
        self.env.update(GIT_ASKPASS=str(approved), SSH_ASKPASS=str(approved))
        fake = self.bin / "fake.py"
        checks = (
            f"assert os.environ['GIT_ASKPASS'] == {str(approved.resolve())!r}\n"
            f"assert os.environ['SSH_ASKPASS'] == {str(approved.resolve())!r}\n"
        )
        fake.write_text(
            fake.read_text().replace(
                "args = sys.argv[1:]", checks + "args = sys.argv[1:]"
            )
        )
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.calls.exists())

    def test_rewritten_registry_pointer_cannot_admit_a_linked_scanner(self):
        """The candidate's own Git association excludes an omitted sibling."""
        self.commit("synthetic-secret-marker")
        linked = self.root / "rewritten-pointer-linked"
        self.git("worktree", "add", "--detach", str(linked), "HEAD")
        pointer = next((self.repo / ".git/worktrees").glob("*/gitdir"))
        pointer.write_text(str(self.root / "missing-worktree/.git") + "\n")
        self.scanner_decoy(linked)
        self.env["PATH"] = str(linked) + os.pathsep + self.env["PATH"]
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertTrue(self.calls.exists())

    def test_duplicate_registry_target_cannot_admit_a_linked_scanner(self):
        """An alias to another registration cannot hide the tool's actual owner."""
        self.commit("synthetic-secret-marker")
        linked = self.root / "aliased-pointer-linked"
        other = self.root / "other-linked-checkout"
        self.git("worktree", "add", "--detach", str(linked), "HEAD")
        self.git("worktree", "add", "--detach", str(other), "HEAD")
        registry = self.repo / ".git/worktrees"
        pointer = next(
            p
            for p in registry.glob("*/gitdir")
            if Path(p.read_text().strip()).resolve() == (linked / ".git").resolve()
        )
        pointer.write_text(str(other / ".git") + "\n")
        self.scanner_decoy(linked)
        self.env["PATH"] = str(linked) + os.pathsep + self.env["PATH"]
        result = self.run_gate("--base", self.base)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertTrue(self.calls.exists())

    def test_repository_python_floor_is_enforced(self):
        """Exercise the CLI outcome immediately below and at the approved floor."""
        previous = Path.cwd()
        try:
            os.chdir(self.repo)
            for version, expected in PYTHON_FLOOR_CASES:
                with (
                    self.subTest(version=version),
                    patch.dict(os.environ, self.env, clear=True),
                    patch.object(sys, "version_info", (*version, 0)),
                    contextlib.redirect_stdout(io.StringIO()),
                    contextlib.redirect_stderr(io.StringIO()),
                ):
                    self.assertEqual(secret_gate.main(["--base", self.base]), expected)
        finally:
            os.chdir(previous)


if __name__ == "__main__":
    unittest.main()
