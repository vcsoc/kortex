"""Real filesystem bundle-input tests, not application/runtime acceptance."""
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest

from bundle_inputs import bounded_regular_bytes, collect_notices, collect_web_assets


class BundleInputTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kortex-private-bundle-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def assets(self):
        root = self.root / "dist"
        root.mkdir()
        (root / "index.html").write_text('<script type="module" src="/assets/app.js"></script>')
        (root / "assets").mkdir()
        (root / "assets/app.js").write_text('console.log("fixture");')
        return root

    def test_binary_assets_preserve_exact_bytes_and_hashes(self):
        root = self.assets()
        content = b"\x89PNG\r\n\x1a\n\x00\xff\x80"
        (root / "assets/logo.png").write_bytes(content)
        result = collect_web_assets(root)
        self.assertEqual(result["web-dist/assets/logo.png"], content)
        manifest = json.loads(result["web-assets-manifest.json"])
        record = manifest["files"]["web-dist/assets/logo.png"]
        self.assertEqual(record, {"bytes": len(content), "sha256": hashlib.sha256(content).hexdigest()})
        self.assertEqual(result, collect_web_assets(root))

    def test_source_maps_and_invalid_text_are_refused(self):
        for filename, content in (("app.js.map", b"{}"), ("bad.js", b"\xff")):
            with self.subTest(filename=filename):
                root = self.assets()
                path = root / filename
                path.write_bytes(content)
                with self.assertRaises((ValueError, UnicodeDecodeError)):
                    collect_web_assets(root)
                path.unlink()
                (root / "assets/app.js").unlink()
                (root / "assets").rmdir()
                (root / "index.html").unlink()
                root.rmdir()

    def test_runtime_name_case_and_nul_text_policy_is_preserved(self):
        root = self.assets()
        for filename, content in (("nul.txt", b"a\0b"), ("unicode-\u00e9.js", b"x"),
                                  ("bad+name.js", b"x"), ("upper.JS", b"x")):
            path = root / filename
            path.write_bytes(content)
            with self.subTest(filename=filename), self.assertRaises(ValueError):
                collect_web_assets(root)
            path.unlink()

    def test_missing_index_and_oversized_file_are_refused(self):
        root = self.assets()
        (root / "index.html").unlink()
        with self.assertRaisesRegex(ValueError, "index.html"):
            collect_web_assets(root)
        (root / "index.html").write_text("fixture")
        with (root / "large.js").open("wb") as output:
            output.truncate(2 * 1024 * 1024 + 1)
        with self.assertRaises(ValueError):
            collect_web_assets(root)

    def test_entry_and_depth_limits_apply_to_directories_too(self):
        root = self.assets()
        current = root
        for _ in range(9):
            current = current / "deep"
            current.mkdir()
        with self.assertRaisesRegex(ValueError, "entry/depth"):
            collect_web_assets(root)

    @unittest.skipUnless(os.name == "posix", "symlink fixture requires native supported privilege")
    def test_symlinks_and_special_files_are_refused(self):
        root = self.assets()
        outside = self.root / "outside.js"
        outside.write_text("must never be bundled")
        link = root / "leak.js"
        link.symlink_to(outside)
        with self.assertRaises(ValueError):
            collect_web_assets(root)
        with self.assertRaises(ValueError):
            bounded_regular_bytes(link, 1024)
        link.unlink()
        fifo = root / "pipe.js"
        os.mkfifo(fifo)
        with self.assertRaises(ValueError):
            bounded_regular_bytes(fifo, 1024)
        with self.assertRaises(ValueError):
            collect_web_assets(root)

    def notice_fixture(self):
        root = self.root / "notices-project"
        catalog = root / "packaging/notices/grammar-1.0.0"
        catalog.mkdir(parents=True)
        content = b"Verbatim third-party notice fixture."
        (catalog / "LICENSE").write_bytes(content)
        crate = root / "registry/grammar"
        crate.mkdir(parents=True)
        (crate / ".cargo_vcs_info.json").write_text(json.dumps({"git": {"sha1": "abc123"}}))
        item = {"package": "grammar", "version": "1.0.0", "file": "grammar-1.0.0/LICENSE",
                "upstream_commit": "abc123", "sha256": hashlib.sha256(content).hexdigest()}
        index = catalog.parent / "index.json"
        index.write_text(json.dumps({"notices": [item]}))
        metadata = {"workspace_members": [], "packages": [{"id": "grammar-registry", "source": "registry",
                    "name": "grammar", "version": "1.0.0", "manifest_path": str(crate / "Cargo.toml")}]}
        return root, metadata, catalog, crate

    def test_supplemental_notice_is_tied_to_published_revision_and_hash(self):
        root, metadata, catalog, crate = self.notice_fixture()
        files = collect_notices(metadata, root)
        self.assertEqual(files["third-party/grammar-1.0.0/LICENSE"], (catalog / "LICENSE").read_bytes())
        (crate / ".cargo_vcs_info.json").write_text(json.dumps({"git": {"sha1": "different"}}))
        with self.assertRaisesRegex(ValueError, "VCS revision"):
            collect_notices(metadata, root)

    def test_changed_notice_bytes_are_refused(self):
        root, metadata, catalog, _ = self.notice_fixture()
        (catalog / "LICENSE").write_text("changed content")
        with self.assertRaisesRegex(ValueError, "hash mismatch"):
            collect_notices(metadata, root)

    def test_core_only_notices_do_not_require_node_modules(self):
        root, metadata, _, _ = self.notice_fixture()
        web = root / "web"
        web.mkdir()
        (web / "package-lock.json").write_text(json.dumps({"packages": {
            "node_modules/missing": {"version": "1.0.0"}}}))
        self.assertTrue(collect_notices(metadata, root))
        with self.assertRaisesRegex(ValueError, "install locked"):
            collect_notices(metadata, root, include_frontend=True)


class ExternalFrontendTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="kortex-external-frontend-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "source"
        self.web = self.root / "installed-web"
        for path in (self.project / "web", self.project / "packaging/notices", self.web):
            path.mkdir(parents=True)
        (self.project / "packaging/notices/index.json").write_text('{"notices":[]}')
        for root in (self.project / "web", self.web):
            (root / "package.json").write_text('{"private":true}')
            (root / "package-lock.json").write_text(json.dumps({"packages": {
                "node_modules/example": {"version": "1.0.0"}}}))
        package = self.web / "node_modules/example"
        package.mkdir(parents=True)
        (package / "LICENSE").write_text("Exact external dependency notice")
        self.metadata = {"workspace_members": [], "packages": []}

    def test_matching_external_install_supplies_notices_without_source_mutation(self):
        files = collect_notices(self.metadata, self.project, True, self.web)
        self.assertEqual(files["third-party/npm/example-1.0.0/LICENSE"], b"Exact external dependency notice")
        self.assertFalse((self.project / "web/node_modules").exists())

    def test_external_manifest_or_lock_mismatch_is_refused(self):
        for name in ("package.json", "package-lock.json"):
            path = self.web / name
            original = path.read_bytes()
            path.write_bytes(original + b" ")
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "differs from frozen"):
                collect_notices(self.metadata, self.project, True, self.web)
            path.write_bytes(original)

    def test_external_install_without_frontend_bundle_is_refused(self):
        with self.assertRaisesRegex(ValueError, "external frontend"):
            collect_notices(self.metadata, self.project, False, self.web)

    @unittest.skipUnless(os.name == "posix", "symlink fixture requires native privilege")
    def test_external_root_and_dependency_aliases_are_refused(self):
        alias = self.root / "alias"
        alias.symlink_to(self.web, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "external frontend"):
            collect_notices(self.metadata, self.project, True, alias)
        package = self.web / "node_modules/example"
        (package / "LICENSE").unlink()
        package.rmdir()
        package.symlink_to(self.project / "web", target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "escapes frontend"):
            collect_notices(self.metadata, self.project, True, self.web)


if __name__ == "__main__":
    unittest.main()
