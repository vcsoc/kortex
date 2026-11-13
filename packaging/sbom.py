#!/usr/bin/env python3
"""Generate a private CycloneDX 1.5 Cargo/frontend dependency superset.

Declared licenses are not legal approval, audit success or exact binary composition.
No model downloads or source uploads occur. Cargo metadata resolution is offline.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

from inventory import build_inventory


ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", default="target/kortex.cdx.json")
    parser.add_argument("--project-root", type=Path, default=ROOT,
                        help="Explicit frozen project; outputs remain separately selected")
    args = parser.parse_args()
    root = args.project_root.resolve(strict=True)
    output = Path(args.output).absolute()
    if root != ROOT.resolve() and output.resolve().is_relative_to(root):
        raise ValueError("external frozen project requires output outside its source tree")
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--offline", "--all-features", "--format-version", "1"], cwd=root
    ))
    license_path = root / "LICENSE"
    notice = license_path.read_text(encoding="utf-8")
    members = set(metadata["workspace_members"])
    for package in metadata["packages"]:
        if package["id"] not in members:
            continue
        path = Path(package["license_file"] or "")
        if not path.is_absolute():
            path = Path(package["manifest_path"]).parent / path
        if path.resolve() != license_path.resolve():
            raise ValueError("first-party package does not use the owner's restrictive LICENSE")
    frontend = None
    manifest_path = root / "web/package.json"
    lock_path = root / "web/package-lock.json"
    if manifest_path.exists() or lock_path.exists():
        raw_lock = lock_path.read_bytes()
        frontend = (json.loads(manifest_path.read_bytes()), json.loads(raw_lock), raw_lock)
    bom = build_inventory(metadata, (root / "Cargo.lock").read_bytes(), notice, frontend)
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=output.parent, delete=False) as stream:
            temporary = Path(stream.name)
            json.dump(bom, stream, indent=2)
            stream.write("\n")
        os.replace(temporary, output)
        temporary = None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    print(f"Wrote {len(bom['components'])} locked Cargo/npm components to {output}; declaration inventory only.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
