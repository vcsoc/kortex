"""Regression guards for the owner's private, closed-source distribution policy."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent


class PrivatePolicyTests(unittest.TestCase):
    def test_public_packaging_cannot_be_enabled_by_legacy_approval(self):
        self.assertTrue((ROOT / "LICENSE").is_file())
        for approval in ("true", "false", ""):
            with self.subTest(approval=approval), tempfile.TemporaryDirectory() as output:
                env = dict(os.environ, KORTEX_RELEASE_APPROVED=approval)
                result = subprocess.run(
                    [sys.executable, str(ROOT / "packaging/artifact.py"), "--output", output],
                    cwd=ROOT, env=env, capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("PUBLIC_RELEASE_DISABLED", result.stderr)
                self.assertEqual(list(Path(output).iterdir()), [])

    def test_first_party_packages_cannot_publish_or_assert_public_license(self):
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]
        self.assertIs(workspace["publish"], False)
        self.assertNotIn("license", workspace)
        self.assertEqual(workspace["license-file"], "LICENSE")
        manifests = list((ROOT / "crates").glob("*/Cargo.toml"))
        self.assertGreaterEqual(len(manifests), 13)
        for manifest in manifests:
            with self.subTest(manifest=manifest.relative_to(ROOT)):
                package = tomllib.loads(manifest.read_text())["package"]
                publish = package.get("publish")
                if isinstance(publish, dict) and publish.get("workspace") is True:
                    publish = workspace["publish"]
                self.assertIs(publish, False)
                self.assertNotIn("license", package)
                self.assertEqual(package.get("license-file"), {"workspace": True})

    def test_every_hosted_job_requires_private_repo_and_owner_approval(self):
        workflows = list((ROOT / ".github/workflows").glob("*.yml"))
        workflows += list((ROOT / ".github/workflows").glob("*.yaml"))
        self.assertTrue(workflows)
        required = "    if: github.event.repository.private == true && vars.KORTEX_PRIVATE_VALIDATION_APPROVED == 'true'"
        for workflow in workflows:
            text = workflow.read_text()
            self.assertIn("jobs:\n", text)
            jobs = text.split("jobs:\n", 1)[1]
            headers = list(re.finditer(r"(?m)^  [a-zA-Z_][a-zA-Z0-9_-]*:[ \t]*$", jobs))
            self.assertTrue(headers, workflow)
            for index, header in enumerate(headers):
                end = headers[index + 1].start() if index + 1 < len(headers) else len(jobs)
                body = jobs[header.end():end]
                with self.subTest(workflow=workflow.name, job=header.group().strip()):
                    self.assertIn(required, body.splitlines())

    def test_external_hosted_actions_are_pinned_to_commit_hashes(self):
        workflows = list((ROOT / ".github/workflows").glob("*.yml"))
        workflows += list((ROOT / ".github/workflows").glob("*.yaml"))
        for workflow in workflows:
            actions = re.findall(r"\buses:\s*([^\s#]+)", workflow.read_text())
            self.assertTrue(actions, workflow)
            for action in actions:
                if not action.startswith("./"):
                    with self.subTest(workflow=workflow.name, action=action):
                        self.assertRegex(action, r"^[^@\s]+@[0-9a-f]{40}$")

    def test_explorer_package_is_private_when_present(self):
        manifest = ROOT / "web/package.json"
        if not manifest.is_file():
            self.skipTest("explorer package has not been created yet")
        package = json.loads(manifest.read_text())
        self.assertIs(package.get("private"), True)
        self.assertIn(package.get("license"), (None, "UNLICENSED", "LicenseRef-Proprietary"))


if __name__ == "__main__":
    unittest.main()
