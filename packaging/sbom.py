#!/usr/bin/env python3
"""Generate a local CycloneDX 1.5 inventory from Cargo's locked resolution.

This records declared licenses; it is not a legal approval or vulnerability audit.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import urllib.parse

parser = argparse.ArgumentParser()
parser.add_argument("--output", default="target/kortex.cdx.json")
args = parser.parse_args()
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"]))
refs = {}
components = []
for package in metadata["packages"]:
    name = package["name"]
    version = package["version"]
    purl = f"pkg:cargo/{urllib.parse.quote(name, safe='')}@{urllib.parse.quote(version, safe='')}"
    refs[package["id"]] = purl
    component = {"type": "library", "bom-ref": purl, "name": name, "version": version, "purl": purl}
    if package.get("license"):
        component["licenses"] = [{"expression": package["license"]}]
    else:
        component["properties"] = [{"name": "kortex:license-review", "value": "owner-pending"}]
    components.append(component)
dependencies = []
for node in metadata["resolve"]["nodes"]:
    dependencies.append({"ref": refs[node["id"]], "dependsOn": sorted({refs[d["pkg"]] for d in node["deps"]})})
bom = {
    "bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
    "metadata": {"tools": [{"vendor": "Kortex", "name": "locked-cargo-inventory", "version": "0.1.0"}],
                 "properties": [{"name": "kortex:Cargo.lock:sha256", "value": hashlib.sha256(pathlib.Path("Cargo.lock").read_bytes()).hexdigest()},
                                {"name": "kortex:release-approval", "value": "pending"}]},
    "components": sorted(components, key=lambda c: c["bom-ref"]),
    "dependencies": sorted(dependencies, key=lambda d: d["ref"]),
}
output = pathlib.Path(args.output)
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(bom, indent=2) + "\n")
print(f"Wrote {len(components)} components to {output}; declarations require review.")
