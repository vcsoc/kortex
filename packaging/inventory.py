"""Deterministic local dependency inventory; declarations are not legal approval."""
import hashlib
from pathlib import PurePosixPath
import urllib.parse


def purl(ecosystem, name, version):
    encoded = urllib.parse.quote(name, safe="/" if ecosystem == "npm" else "")
    return f"pkg:{ecosystem}/{encoded}@{urllib.parse.quote(version, safe='')}"


def proprietary_notice(text):
    return [{"license": {"name": "Kortex proprietary/confidential notice; no public use grant",
                         "text": {"contentType": "text/plain", "content": text}}}]


def dependency_licenses(value):
    if isinstance(value, str) and value:
        return {"licenses": [{"expression": value}]}
    return {"properties": [{"name": "kortex:license-review", "value": "required; declaration missing"}]}


def cargo_inventory(metadata, notice):
    members = set(metadata["workspace_members"])
    refs, components, seen = {}, [], set()
    for package in metadata["packages"]:
        ref = purl("cargo", package["name"], package["version"])
        if ref in seen:
            raise ValueError("ambiguous same-name/version Cargo sources require provenance review")
        seen.add(ref)
        refs[package["id"]] = ref
        component = {"type": "library", "bom-ref": ref, "name": package["name"],
                     "version": package["version"], "purl": ref}
        if package["id"] in members:
            if package.get("license") or package.get("publish") != [] or not package.get("license_file"):
                raise ValueError("first-party Cargo metadata violates proprietary/nonpublishable policy")
            component["licenses"] = proprietary_notice(notice)
            component["properties"] = [{"name": "kortex:first-party", "value": "private; publishing disabled"}]
        else:
            component.update(dependency_licenses(package.get("license")))
        components.append(component)
    dependencies = [{"ref": refs[node["id"]],
                     "dependsOn": sorted({refs[dep["pkg"]] for dep in node["deps"]})}
                    for node in metadata["resolve"]["nodes"]]
    return components, dependencies


def npm_inventory(manifest, lock, notice):
    if manifest.get("private") is not True:
        raise ValueError("first-party frontend must be private")
    if lock.get("lockfileVersion") not in (2, 3):
        raise ValueError("unsupported frontend lockfile schema")
    packages = lock.get("packages", {})
    root = packages.get("")
    if not root or root.get("name") != manifest["name"] or root.get("version") != manifest["version"]:
        raise ValueError("frontend manifest and locked identity disagree")
    refs, components = {}, []
    for key, package in sorted(packages.items()):
        path = PurePosixPath(key)
        if key and (path.is_absolute() or ".." in path.parts or "\\" in key or str(path) != key):
            raise ValueError("unsafe frontend package location")
        if package.get("link"):
            raise ValueError("linked frontend dependency requires explicit provenance review")
        name = package.get("name") or key.rsplit("node_modules/", 1)[-1]
        version = package.get("version")
        if not name or not isinstance(version, str):
            raise ValueError("frontend dependency lacks a locked name/version")
        identifier = purl("npm", name, version)
        ref = identifier + "#" + urllib.parse.quote(key or "root", safe="")
        refs[key] = ref
        component = {"type": "application" if not key else "library", "bom-ref": ref,
                     "purl": identifier, "name": name, "version": version}
        if not key:
            component["licenses"] = proprietary_notice(notice)
        else:
            component.update(dependency_licenses(package.get("license")))
        properties = component.setdefault("properties", [])
        properties.append({"name": "kortex:npm-scope", "value": "development/build only" if package.get("dev") else "runtime candidate"})
        if package.get("integrity"):
            properties.append({"name": "kortex:npm-lock-integrity", "value": package["integrity"]})
        components.append(component)

    def resolve(key, name):
        if not name or ".." in PurePosixPath(name).parts or "\\" in name or name.startswith("/"):
            raise ValueError("unsafe frontend dependency name")
        location = PurePosixPath(key)
        for parent in (location, *location.parents):
            if parent.name == "node_modules":
                continue
            candidate = str(parent / "node_modules" / name)
            if candidate in refs:
                return refs[candidate]
        return None

    dependencies = []
    for key, package in sorted(packages.items()):
        names = set()
        for field in ("dependencies", "optionalDependencies", "peerDependencies"):
            names.update(package.get(field, {}))
        if not key:
            names.update(package.get("devDependencies", {}))
        targets, missing = set(), []
        for name in sorted(names):
            target = resolve(key, name)
            if target is None:
                missing.append(name)
            else:
                targets.add(target)
        if missing:
            component = next(item for item in components if item["bom-ref"] == refs[key])
            component["properties"].append({"name": "kortex:unresolved-lock-dependencies", "value": ", ".join(missing)})
        dependencies.append({"ref": refs[key], "dependsOn": sorted(targets)})
    return components, dependencies


def build_inventory(metadata, cargo_lock, notice, frontend=None):
    components, dependencies = cargo_inventory(metadata, notice)
    properties = [{"name": "kortex:Cargo.lock:sha256", "value": hashlib.sha256(cargo_lock).hexdigest()},
                  {"name": "kortex:distribution-policy", "value": "proprietary/private; public distribution prohibited"},
                  {"name": "kortex:inventory-scope", "value": "all first-party Cargo features plus build/test dependencies; superset, not exact binary composition"},
                  {"name": "kortex:legal-approval", "value": "not established by inventory"}]
    if frontend is not None:
        manifest, lock, raw_lock = frontend
        npm_components, npm_dependencies = npm_inventory(manifest, lock, notice)
        components.extend(npm_components)
        dependencies.extend(npm_dependencies)
        properties.append({"name": "kortex:frontend-lock:sha256", "value": hashlib.sha256(raw_lock).hexdigest()})
        properties.append({"name": "kortex:frontend-scope", "value": "locked runtime and development/build dependencies; dev scanners are not asserted bundled"})
    return {"bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
            "metadata": {"tools": [{"vendor": "Kortex", "name": "locked-local-inventory", "version": "0.1.0"}],
                         "properties": properties},
            "components": sorted(components, key=lambda item: item["bom-ref"]),
            "dependencies": sorted(dependencies, key=lambda item: item["ref"])}
