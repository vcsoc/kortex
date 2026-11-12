"""Local freezer unit tests; these do not invoke Docker or prove Linux runtime."""
import hashlib
import json
import os
from pathlib import Path
import socket
import tempfile
import unittest
from unittest.mock import patch
import subprocess

from linux_validation import JOURNEY, SCRIPT, container, local_endpoint, mount, pinned_snapshot, source_inputs, test_evidence


class SourceSnapshotTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kortex-freezer-unit-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name).resolve()
        for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE", "README.md", "KORTEX_SPECIFICATION.md"):
            (self.root / name).write_text("inert freezer unit fixture")
        (self.root / "crates/example/src").mkdir(parents=True)
        (self.root / "fixtures").mkdir()
        (self.root / "crates/example/src/lib.rs").write_text("// inert source; never executed")

    def test_exact_relative_bytes_and_determinism(self):
        first = source_inputs(self.root)
        self.assertEqual(first, source_inputs(self.root))
        self.assertEqual(len(first), 7)
        self.assertEqual(first["crates/example/src/lib.rs"], b"// inert source; never executed")
        (self.root / "crates/example/src/lib.rs").write_text("// edited")
        self.assertNotEqual(first, source_inputs(self.root))

    def test_caches_git_metadata_and_node_modules_are_not_copied(self):
        for name in ("target", ".git", ".kortex", "node_modules", "__pycache__"):
            directory = self.root / "crates/example" / name
            directory.mkdir()
            (directory / "not-an-input").write_text("must not enter snapshot")
        self.assertEqual(len(source_inputs(self.root)), 7)

    @unittest.skipUnless(os.name == "posix", "native symlink privilege fixture")
    def test_file_and_directory_aliases_are_refused(self):
        outside = self.root / "outside"
        outside.write_text("outside input")
        alias = self.root / "crates/example/src/alias.rs"
        alias.symlink_to(outside)
        with self.assertRaises(ValueError):
            source_inputs(self.root)
        alias.unlink()
        alias.symlink_to(self.root / "fixtures", target_is_directory=True)
        with self.assertRaises(ValueError):
            source_inputs(self.root)

    def test_oversized_sparse_source_is_refused_before_reading(self):
        path = self.root / "fixtures/oversized.bin"
        with path.open("wb") as output:
            output.truncate(16 * 1024 * 1024 + 1)
        with self.assertRaises(ValueError):
            source_inputs(self.root)

    def pin(self):
        digest = hashlib.sha256()
        for name, content in sorted(source_inputs(self.root).items()):
            digest.update(name.encode() + b"\0" + content + b"\0")
        return digest.hexdigest()

    def test_snapshot_reuse_does_not_copy_or_change_mtimes(self):
        path = self.root / "crates/example/src/lib.rs"
        before = path.stat().st_mtime_ns
        first = pinned_snapshot(self.root, self.pin(), 7)
        self.assertEqual(first, pinned_snapshot(self.root, self.pin(), 7))
        self.assertEqual(path.stat().st_mtime_ns, before)
        self.assertEqual(first[0], self.root)

    def test_hash_and_count_pins_refuse_wrong_or_changed_source(self):
        pin = self.pin()
        for digest, count in (("0" * 64, 7), (pin, 8), ("bad", 7)):
            with self.assertRaises(ValueError):
                pinned_snapshot(self.root, digest, count)
        (self.root / "crates/example/src/lib.rs").write_text("// changed")
        with self.assertRaises(ValueError):
            pinned_snapshot(self.root, pin, 7)

    @unittest.skipUnless(os.name == "posix", "native symlink fixture")
    def test_top_level_source_directory_alias_is_refused(self):
        (self.root / "fixtures").rmdir()
        (self.root / "fixtures").symlink_to(self.root / "crates", target_is_directory=True)
        with self.assertRaises(ValueError):
            source_inputs(self.root)


class EvidenceTests(unittest.TestCase):
    def success(self):
        languages = "\n".join(f"P1_CLI language={name} discovered=3 persisted=true query_check=true context_exact_callers=1"
                              for name in ("java", "csharp", "c", "cpp", "ruby", "php"))
        return ("VALIDATION_PHASE=workspace BEGIN\ntest result: ok. 12 passed; 0 failed; 1 ignored;\n"
                "VALIDATION_PHASE=workspace EXIT=0\nVALIDATION_PHASE=journey BEGIN\n"
                + languages + f"\ntest {JOURNEY} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n"
                "VALIDATION_PHASE=journey EXIT=0\n")

    def test_actual_test_and_all_six_languages_required(self):
        evidence = test_evidence(self.success())
        self.assertTrue(evidence["workspace_passed"])
        self.assertEqual(evidence["workspace_passed_tests"], 12)
        self.assertTrue(evidence["six_language_cli_journey_passed"])
        self.assertFalse(test_evidence(self.success().replace("language=php", "language=notphp"))
                         ["six_language_cli_journey_passed"])

    def test_failed_target_counts_and_nested_helper_are_not_double_counted(self):
        text = ("VALIDATION_PHASE=workspace BEGIN\nRunning tests/helper.rs\n"
                "test result: ok. 1 passed; 0 failed; 0 ignored;\n"
                "test result: ok. 12 passed; 0 failed; 0 ignored;\n"
                "Running tests/broken.rs\ntest result: FAILED. 45 passed; 1 failed; 0 ignored;\n"
                "VALIDATION_PHASE=workspace EXIT=101\n")
        evidence = test_evidence(text)
        self.assertFalse(evidence["workspace_passed"])
        self.assertEqual(evidence["workspace_passed_tests"], 57)
        self.assertEqual(evidence["workspace_failed_tests"], 1)
        self.assertEqual(evidence["workspace_summarized_targets"], 2)

    def test_zero_filtered_truncated_and_nonzero_are_not_passes(self):
        for text in (self.success().replace("1 passed;", "0 passed;"),
                     self.success().replace(f"test {JOURNEY} ... ok", "test other ... ok"),
                     self.success().replace("journey EXIT=0", "journey EXIT=124"),
                     self.success().replace("journey EXIT=0", "")):
            self.assertFalse(test_evidence(text)["six_language_cli_journey_passed"])
        for text in (self.success().replace("12 passed", "0 passed"),
                     self.success().replace("workspace EXIT=0", "workspace EXIT=101")):
            self.assertFalse(test_evidence(text)["workspace_passed"])

    def test_remote_and_non_socket_endpoints_refused(self):
        for endpoint in ("tcp://127.0.0.1:2375", "ssh://host", "unix://relative", "unix:///dev/null"):
            with self.assertRaises((ValueError, FileNotFoundError)):
                local_endpoint({"Endpoints": {"docker": {"Host": endpoint}}})

    @unittest.skipUnless(hasattr(socket, "AF_UNIX"), "Unix socket required")
    def test_local_unix_socket_accepted(self):
        with tempfile.TemporaryDirectory(prefix="linux-socket-") as directory:
            path = str(Path(directory) / "docker.sock")
            with socket.socket(socket.AF_UNIX) as server:
                server.bind(path)
                self.assertEqual(local_endpoint({"Endpoints": {"docker": {"Host": "unix://" + path}}}), "unix://" + path)

    def test_ambiguous_mounts_refused(self):
        with self.assertRaises(ValueError):
            mount("bind", "/tmp/source,readonly=false", "/src", True)
        self.assertIn("readonly", mount("bind", "/source", "/src", True)[1])

    def test_commands_are_offline_locked_and_journey_is_exact(self):
        self.assertIn("--workspace --all-features --locked --offline --no-fail-fast", SCRIPT)
        self.assertIn("-- --ignored --exact --nocapture", SCRIPT)
        self.assertIn("KORTEX_TEST_CLI=/build/debug/kortex", SCRIPT)
        self.assertNotIn("cargo install", SCRIPT)


class ContainerLifecycleTests(unittest.TestCase):
    def test_timeout_cleanup_and_offline_nonroot_resource_flags(self):
        with tempfile.TemporaryDirectory() as directory:
            run = Path(directory)
            with patch("linux_validation.subprocess.check_output", side_effect=[b"id", json.dumps([{}]).encode()]) as output:
                with patch("linux_validation.subprocess.run", side_effect=[subprocess.TimeoutExpired("docker start", 90), subprocess.CompletedProcess([], 0)]) as execute:
                    with self.assertRaises(subprocess.TimeoutExpired):
                        container(["docker", "--host", "unix:///trusted.sock"], run, "owner-token", "own-container", [], "true", 60)
            command = output.call_args_list[0].args[0]
            for flag in ("--pull=never", "--network=none", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--user=501:20", "--cpus=4", "--memory=8g", "--memory-swap=8g", "--pids-limit=1024"):
                self.assertIn(flag, command)
            self.assertNotIn("--cap-add=CHOWN", command)
            self.assertIn("timeout", command)
            self.assertIn("--kill-after=10s", command)
            self.assertEqual(execute.call_args_list[0].kwargs["timeout"], 90)
            self.assertEqual(execute.call_args_list[1].args[0][-3:], ["rm", "--force", "own-container"])


if __name__ == "__main__":
    unittest.main()
