"""Frozen project output confinement guards; no application acceptance claims."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import sbom

ROOT = Path(__file__).resolve().parent.parent


class FrozenProjectTests(unittest.TestCase):
    def test_sbom_refuses_output_inside_frozen_project_before_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source"
            source.mkdir()
            with patch.object(sys, "argv", ["sbom.py", "--project-root", str(source),
                                           "--output", str(source / "generated.json")]):
                with patch("sbom.subprocess.check_output") as metadata:
                    with self.assertRaisesRegex(ValueError, "outside its source tree"):
                        sbom.main()
                    metadata.assert_not_called()
            self.assertEqual(list(source.iterdir()), [])

    def test_artifact_refuses_frozen_output_without_creating_it(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source"
            source.mkdir()
            result = subprocess.run([sys.executable, str(ROOT / "packaging/artifact.py"),
                                     "--development", "--project-root", str(source),
                                     "--output", str(source / "artifacts")],
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertIn("outside its source tree", result.stderr)
            self.assertEqual(list(source.iterdir()), [])

    def test_explicit_project_never_bypasses_public_release_refusal(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "artifacts"
            result = subprocess.run([sys.executable, str(ROOT / "packaging/artifact.py"),
                                     "--project-root", temporary, "--output", str(output)],
                                    env=dict(os.environ, KORTEX_RELEASE_APPROVED="true"),
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertIn("PUBLIC_RELEASE_DISABLED", result.stderr)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
