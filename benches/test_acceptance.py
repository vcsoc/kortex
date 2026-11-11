"""Harness-only tests; mocked commands are not application acceptance evidence."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "kortex_acceptance_harness", Path(__file__).with_name("acceptance.py")
)
harness = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(harness)


class AcceptanceHarnessTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kortex-harness-unit-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.binary = self.root / "mock-binary"
        self.binary.write_bytes(b"controlled unit-test data, never executed")
        self.output = self.root / "report.json"
        self.executed_paths = set()

    def arguments(self, strict=False):
        words = ["acceptance.py", "--binary", str(self.binary), "--files", "2",
                 "--functions", "2", "--search-runs", "1", "--context-runs", "1",
                 "--impact-runs", "1", "--startup-runs", "1", "--updates", "1",
                 "--output", str(self.output)]
        return words + (["--enforce-targets"] if strict else [])

    def command(self, argv, **kwargs):
        executable = Path(argv[0])
        self.assertNotEqual(executable, self.binary)
        self.assertEqual(executable.read_bytes(), self.binary.read_bytes())
        self.executed_paths.add(executable)
        operation = argv[3]
        if operation == "index":
            value = {"files": 2, "parsed": 1, "reused": 1}
        elif operation == "query":
            value = {"data": {"items": [{"symbol": {"name": argv[4]}}]}}
        elif operation == "context":
            value = {"data": {"symbol": {"name": argv[4]}}}
        elif operation == "impact":
            value = {"data": {"target": {"name": argv[4]}, "direct": 1}}
        else:
            value = {"version": "mock-only"}
        return subprocess.CompletedProcess(argv, 0, json.dumps(value), "")

    def invoke(self, command=None, strict=False):
        with patch.object(sys, "argv", self.arguments(strict)), \
             patch.object(harness.subprocess, "run", side_effect=command or self.command), \
             contextlib.redirect_stdout(io.StringIO()):
            return harness.main()

    def report(self):
        return json.loads(self.output.read_text())

    def test_small_completed_run_is_not_reference_acceptance(self):
        self.assertEqual(self.invoke(), 0)
        report = self.report()
        self.assertTrue(report["runs_complete"])
        self.assertFalse(report["acceptance_sample_sizes"])
        self.assertEqual(report["process_commands_completed"]["index"], 2)
        self.assertEqual(report["requested_samples"]["query"], 1)
        self.assertEqual(len(self.executed_paths), 1)
        self.assertTrue(all(not path.exists() for path in self.executed_paths))

    def test_strict_mode_refuses_small_samples(self):
        self.assertEqual(self.invoke(strict=True), 1)
        self.assertFalse(self.report()["acceptance_sample_sizes"])

    def test_timeout_records_phase_completion_and_available_rss(self):
        def timeout(argv, **kwargs):
            if argv[3] == "index":
                raise subprocess.TimeoutExpired(argv, 0.01)
            return self.command(argv, **kwargs)

        with self.assertRaises(subprocess.TimeoutExpired):
            self.invoke(timeout)
        report = self.report()
        self.assertFalse(report["runs_complete"])
        self.assertFalse(report["acceptance_sample_sizes"])
        self.assertEqual(report["process_commands_completed"], {"version": 1})
        self.assertEqual(report["last_command"]["operation"], "index")
        self.assertIsNone(report["last_command"]["exit_code"])
        self.assertGreaterEqual(report["last_command"]["elapsed_ms"], 0)
        self.assertEqual(report["error"]["type"], "TimeoutExpired")
        self.assertIn("peak_child_rss_bytes", report)
        self.assertIn("rss_scope", report)

    def test_binary_change_during_copy_is_refused(self):
        def changed_copy(source, destination):
            Path(destination).write_bytes(b"concurrently replaced executable")

        with patch.object(harness.shutil, "copy2", side_effect=changed_copy):
            with self.assertRaisesRegex(RuntimeError, "binary changed"):
                self.invoke()
        self.assertFalse(self.report()["runs_complete"])
        self.assertEqual(self.report()["process_commands_completed"], {})
        self.assertEqual(self.executed_paths, set())

    def test_wrong_context_or_impact_target_cannot_pass_fast(self):
        for operation in ("context", "impact"):
            with self.subTest(operation=operation):
                def wrong_target(argv, **kwargs):
                    result = self.command(argv, **kwargs)
                    if argv[3] == operation:
                        value = json.loads(result.stdout)
                        key = "symbol" if operation == "context" else "target"
                        value["data"][key]["name"] = "wrong_symbol"
                        result.stdout = json.dumps(value)
                    return result

                with self.assertRaisesRegex(AssertionError, "wrong fixture symbol"):
                    self.invoke(wrong_target)
                self.assertFalse(self.report()["runs_complete"])

    def test_missing_known_impact_caller_cannot_pass_fast(self):
        def missing_caller(argv, **kwargs):
            result = self.command(argv, **kwargs)
            if argv[3] == "impact":
                value = json.loads(result.stdout)
                value["data"]["direct"] = 0
                result.stdout = json.dumps(value)
            return result

        with self.assertRaisesRegex(AssertionError, "omitted a known fixture caller"):
            self.invoke(missing_caller)
        self.assertFalse(self.report()["runs_complete"])

    def test_process_failure_keeps_exit_code(self):
        def failed_index(argv, **kwargs):
            if argv[3] == "index":
                return subprocess.CompletedProcess(argv, 7, "", "controlled failure")
            return self.command(argv, **kwargs)

        with self.assertRaisesRegex(RuntimeError, "exited 7"):
            self.invoke(failed_index)
        self.assertEqual(self.report()["last_command"]["exit_code"], 7)
        self.assertNotIn("index", self.report()["process_commands_completed"])
        self.assertFalse(self.report()["runs_complete"])


if __name__ == "__main__":
    unittest.main()
