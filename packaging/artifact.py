#!/usr/bin/env python3
"""Build private development archives; all public packaging is prohibited."""
import argparse
import gzip
import hashlib
import io
import json
import os
import pathlib
import subprocess
import tarfile
import zipfile

parser = argparse.ArgumentParser()
parser.add_argument("--development", action="store_true", help="Private smoke archive, not an approved release")
parser.add_argument("--binary", default="target/release/kortex.exe" if os.name == "nt" else "target/release/kortex")
parser.add_argument("--output", default="target/artifacts")
args = parser.parse_args()
if not args.development:
    parser.error("PUBLIC_RELEASE_DISABLED: Kortex is proprietary and private; no public use or distribution is authorized. Legacy approval variables cannot override this policy.")
binary = pathlib.Path(args.binary)
if not binary.is_file():
    parser.error(f"build the release binary first: {binary}")
subprocess.run(["python3" if os.name != "nt" else "python", "packaging/sbom.py"], check=True)
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"]))
host = next(line.split(": ", 1)[1] for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines() if line.startswith("host: "))
version = next(p["version"] for p in metadata["packages"] if p["name"] == "kortex-cli")
name = f"kortex-{version}-{host}" + ("-development" if args.development else "")
files = {"kortex.exe" if os.name == "nt" else "kortex": binary.read_bytes(),
         "README.md": pathlib.Path("README.md").read_bytes(),
         "kortex.cdx.json": pathlib.Path("target/kortex.cdx.json").read_bytes()}
license_path = pathlib.Path("LICENSE" if pathlib.Path("LICENSE").exists() else "LICENSE_PENDING.md")
files[license_path.name] = license_path.read_bytes()
for path in sorted(pathlib.Path("docs").iterdir()):
    if path.is_file() and path.suffix in (".md", ".json", ".txt"):
        files[f"docs/{path.name}"] = path.read_bytes()
for package in metadata["packages"]:
    if package["source"] is None:
        continue
    root = pathlib.Path(package["manifest_path"]).parent
    for pattern in ("LICENSE*", "COPYING*", "NOTICE*"):
        for path in sorted(root.glob(pattern)):
            if path.is_file():
                files[f"third-party/{package['name']}-{package['version']}/{path.name}"] = path.read_bytes()
output = pathlib.Path(args.output)
output.mkdir(parents=True, exist_ok=True)
if os.name == "nt":
    artifact = output / f"{name}.zip"
    with zipfile.ZipFile(artifact, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path, data in sorted(files.items()):
            entry = zipfile.ZipInfo(f"{name}/{path}", date_time=(1980, 1, 1, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.external_attr = 0o644 << 16
            archive.writestr(entry, data)
else:
    artifact = output / f"{name}.tar.gz"
    with artifact.open("wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", mtime=0, filename="") as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            for path, data in sorted(files.items()):
                entry = tarfile.TarInfo(f"{name}/{path}")
                entry.size = len(data)
                entry.mode = 0o755 if path == "kortex" else 0o644
                entry.mtime = 0
                archive.addfile(entry, io.BytesIO(data))
digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
(output / f"{artifact.name}.sha256").write_text(f"{digest}  {artifact.name}\n")
print(f"Created {artifact} ({digest}); public_release_approved={not args.development}")
