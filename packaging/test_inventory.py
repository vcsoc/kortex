"""Inventory-format regressions using metadata fixtures, not an application audit."""
import copy
import json
import unittest

from inventory import build_inventory, cargo_inventory, npm_inventory, purl


NOTICE = "Proprietary and confidential. No public use or distribution grant."


def cargo_fixture():
    return {"workspace_members": ["local"], "packages": [
        {"id": "local", "name": "kortex-core", "version": "0.1.0", "license": None,
         "license_file": "LICENSE", "publish": []},
        {"id": "registry", "name": "fixture-dependency", "version": "1.2.3", "license": "MIT"}],
        "resolve": {"nodes": [{"id": "local", "deps": [{"pkg": "registry"}]},
                              {"id": "registry", "deps": []}]}}


def npm_fixture():
    manifest = {"name": "kortex-private-explorer", "version": "0.1.0", "private": True}
    lock = {"lockfileVersion": 3, "packages": {
        "": {**manifest, "dependencies": {"react": "19.3.1", "@scope/ui": "1.0.0"},
             "devDependencies": {"scanner": "2.0.0"}},
        "node_modules/react": {"version": "19.3.1", "license": "MIT", "integrity": "sha512-fixture"},
        "node_modules/@scope/ui": {"version": "1.0.0", "license": "MIT", "dependencies": {"react": "19.3.1"}},
        "node_modules/scanner": {"version": "2.0.0", "license": "MPL-2.0", "dev": True}}}
    return manifest, lock


class InventoryTests(unittest.TestCase):
    def test_owner_decision_is_not_labelled_pending_or_public(self):
        bom = build_inventory(cargo_fixture(), b"locked", NOTICE)
        first = next(component for component in bom["components"] if component["name"] == "kortex-core")
        self.assertEqual(first["licenses"][0]["license"]["text"]["content"], NOTICE)
        self.assertNotIn("expression", first["licenses"][0])
        self.assertNotIn("owner-pending", json.dumps(bom))
        self.assertIn("superset", json.dumps(bom["metadata"]))
        self.assertEqual(bom["dependencies"][1]["dependsOn"], ["pkg:cargo/fixture-dependency@1.2.3"])

    def test_public_or_publishable_first_party_is_refused(self):
        for field, value in (("license", "MIT"), ("publish", None), ("license_file", None)):
            metadata = cargo_fixture()
            metadata["packages"][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                cargo_inventory(metadata, NOTICE)

    def test_unknown_third_party_license_requires_review_not_owner_decision(self):
        metadata = cargo_fixture()
        metadata["packages"][1]["license"] = None
        components, _ = cargo_inventory(metadata, NOTICE)
        self.assertEqual(components[1]["properties"][0]["value"], "required; declaration missing")

    def test_duplicate_cargo_source_identity_is_not_silently_collapsed(self):
        metadata = cargo_fixture()
        duplicate = copy.deepcopy(metadata["packages"][1])
        duplicate["id"] = "different-source"
        metadata["packages"].append(duplicate)
        with self.assertRaisesRegex(ValueError, "ambiguous"):
            cargo_inventory(metadata, NOTICE)

    def test_scoped_npm_names_and_peer_resolution(self):
        manifest, lock = npm_fixture()
        components, edges = npm_inventory(manifest, lock, NOTICE)
        scoped = next(item for item in components if item["name"] == "@scope/ui")
        self.assertEqual(scoped["purl"], "pkg:npm/%40scope/ui@1.0.0")
        targets = next(edge["dependsOn"] for edge in edges if edge["ref"] == scoped["bom-ref"])
        self.assertEqual(len(targets), 1)
        self.assertIn("pkg:npm/react@19.3.1", targets[0])
        scanner = next(item for item in components if item["name"] == "scanner")
        self.assertIn({"name": "kortex:npm-scope", "value": "development/build only"}, scanner["properties"])

    def test_nested_dependency_resolves_nearest_locked_installation(self):
        manifest, lock = npm_fixture()
        lock["packages"]["node_modules/@scope/ui/node_modules/react"] = {"version": "18.0.0", "license": "MIT"}
        components, edges = npm_inventory(manifest, lock, NOTICE)
        scoped = next(item for item in components if item["name"] == "@scope/ui")
        target = next(edge["dependsOn"] for edge in edges if edge["ref"] == scoped["bom-ref"])[0]
        self.assertIn("react@18.0.0", target)

    def test_linked_or_escaping_frontend_dependencies_are_refused(self):
        for key, package in (("../outside", {"version": "1"}),
                             ("node_modules/linked", {"link": True, "version": "1"})):
            manifest, lock = npm_fixture()
            lock["packages"][key] = package
            with self.subTest(key=key), self.assertRaises(ValueError):
                npm_inventory(manifest, lock, NOTICE)

    def test_manifest_lock_mismatch_and_public_frontend_are_refused(self):
        manifest, lock = npm_fixture()
        manifest["private"] = False
        with self.assertRaises(ValueError):
            npm_inventory(manifest, lock, NOTICE)
        manifest["private"] = True
        lock["packages"][""]["version"] = "different"
        with self.assertRaises(ValueError):
            npm_inventory(manifest, lock, NOTICE)

    def test_frontend_and_cargo_hashes_are_deterministic(self):
        manifest, lock = npm_fixture()
        arguments = (cargo_fixture(), b"cargo lock", NOTICE, (manifest, lock, b"npm lock"))
        first = build_inventory(*arguments)
        self.assertEqual(first, build_inventory(*arguments))
        self.assertEqual(len(first["components"]), 6)
        self.assertTrue(any(item["name"] == "kortex-private-explorer" for item in first["components"]))
        self.assertEqual(purl("cargo", "some-crate", "1.0.0"), "pkg:cargo/some-crate@1.0.0")


if __name__ == "__main__":
    unittest.main()
