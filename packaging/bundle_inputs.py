"""Bounded owner-supplied build inputs for private development archives.

Inputs must be quiescent trusted build directories. Leaf checks do not claim
protection against malicious same-user replacement of initial ancestors.
"""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import stat
import urllib.parse

TEXT_ASSETS = {".html", ".js", ".mjs", ".css", ".json", ".svg", ".txt"}
BINARY_ASSETS = {".png", ".jpg", ".jpeg", ".webp", ".gif", ".ico", ".woff", ".woff2", ".ttf", ".otf"}


def bounded_regular_bytes(path, maximum):
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode) or before.st_size > maximum:
        raise ValueError(f"nonregular or oversized bundle input: {path.name}")
    flags = (os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
             | getattr(os, "O_BINARY", 0))
    with os.fdopen(os.open(path, flags), "rb") as source:
        opened = os.fstat(source.fileno())
        if not stat.S_ISREG(opened.st_mode) or (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino):
            raise ValueError("bundle input changed before opening")
        content = source.read(maximum + 1)
        after = os.fstat(source.fileno())
    if len(content) > maximum or (opened.st_size, opened.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
        raise ValueError("oversized or concurrently modified bundle input")
    return content


def collect_web_assets(directory):
    supplied = Path(directory)
    if supplied.is_symlink():
        raise ValueError("asset root must not be a symlink")
    root = supplied.resolve(strict=True)
    if not root.is_dir():
        raise ValueError("asset root must be a directory")
    pending, files, count, total = [root], {}, 0, 0
    while pending:
        parent = pending.pop()
        for path in sorted(parent.iterdir()):
            count += 1
            relative = path.relative_to(root)
            if count > 512 or len(relative.parts) > 8:
                raise ValueError("asset entry/depth budget exceeded")
            if any(any(not (c.isascii() and (c.isalnum() or c in "._-")) for c in part) for part in relative.parts):
                raise ValueError("unsafe asset name")
            metadata = path.lstat()
            if stat.S_ISDIR(metadata.st_mode):
                pending.append(path)
            elif stat.S_ISREG(metadata.st_mode):
                if path.suffix not in TEXT_ASSETS | BINARY_ASSETS:
                    raise ValueError("unsupported asset type; source maps are never bundled")
                content = bounded_regular_bytes(path, 2 * 1024 * 1024)
                if path.suffix in TEXT_ASSETS:
                    content.decode("utf-8")
                    if b"\0" in content:
                        raise ValueError("NUL in text asset refused by runtime policy")
                total += len(content)
                if total > 16 * 1024 * 1024:
                    raise ValueError("asset total-byte budget exceeded")
                files["web-dist/" + relative.as_posix()] = content
            else:
                raise ValueError("asset symlink or special file refused")
    if "web-dist/index.html" not in files:
        raise ValueError("asset bundle requires index.html")
    manifest = {"schema_version": 1, "scope": "explicit owner-supplied prebuilt assets; no runtime Node",
                "files": {key: {"bytes": len(value), "sha256": hashlib.sha256(value).hexdigest()}
                          for key, value in sorted(files.items())}}
    files["web-assets-manifest.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    return files


def collect_notices(metadata, project, include_frontend=False, frontend_root=None):
    files = {}
    members = set(metadata["workspace_members"])
    active = {(package["name"], package["version"]): package for package in metadata["packages"]}
    for package in metadata["packages"]:
        if package["id"] in members:
            continue
        root = Path(package["manifest_path"]).parent
        for path in sorted(root.iterdir()):
            if path.name.upper().startswith(("LICENSE", "COPYING", "NOTICE")) and not path.is_dir():
                files[f"third-party/{package['name']}-{package['version']}/{path.name}"] = bounded_regular_bytes(path, 1024 * 1024)
    # Some published grammar archives omit the required upstream notice. Use
    # reviewed verbatim files tied to their published crate's VCS commit.
    catalog_root = project / "packaging/notices"
    catalog = json.loads(bounded_regular_bytes(catalog_root / "index.json", 128 * 1024))
    for item in catalog["notices"]:
        package = active.get((item["package"], item["version"]))
        if package is None:
            continue
        relative = PurePosixPath(item["file"])
        if relative.is_absolute() or ".." in relative.parts or "\\" in item["file"]:
            raise ValueError("unsafe notice catalog path")
        original = Path(package["manifest_path"]).parent / ".cargo_vcs_info.json"
        revision = json.loads(bounded_regular_bytes(original, 64 * 1024))["git"]["sha1"]
        if revision != item["upstream_commit"]:
            raise ValueError("notice does not match the published crate's VCS revision")
        content = bounded_regular_bytes(catalog_root / Path(*relative.parts), 1024 * 1024)
        if hashlib.sha256(content).hexdigest() != item["sha256"]:
            raise ValueError("supplemental third-party notice hash mismatch")
        key = f"third-party/{item['package']}-{item['version']}/LICENSE"
        if key in files and files[key] != content:
            raise ValueError("conflicting third-party notices require review")
        files[key] = content
    lock_path = project / "web/package-lock.json"
    web_root = project / "web"
    if frontend_root is not None:
        supplied = Path(frontend_root)
        if not include_frontend or supplied.is_symlink() or not supplied.is_dir():
            raise ValueError("external frontend requires a regular installed dependency directory")
        web_root = supplied.resolve(strict=True)
        # An external offline install must describe exactly the frozen frontend.
        for name in ("package.json", "package-lock.json"):
            if bounded_regular_bytes(web_root / name, 8 * 1024 * 1024) != bounded_regular_bytes(project / "web" / name, 8 * 1024 * 1024):
                raise ValueError("external frontend manifest/lock differs from frozen source")
    if include_frontend and lock_path.exists():
        lock = json.loads(bounded_regular_bytes(lock_path, 8 * 1024 * 1024))
        for key, package in sorted(lock["packages"].items()):
            if not key:
                continue
            relative = PurePosixPath(key)
            if relative.is_absolute() or ".." in relative.parts or "\\" in key or package.get("link"):
                raise ValueError("unsafe or linked npm notice location")
            root = web_root / Path(*relative.parts)
            if not root.exists():
                if package.get("optional"):
                    continue  # Other-platform build tools are inventoried, not asserted installed/bundled.
                raise ValueError("install locked frontend dependencies before bundling notices")
            if root.is_symlink() or not root.resolve().is_relative_to(web_root.resolve()):
                raise ValueError("npm notice directory escapes frontend")
            name = urllib.parse.quote(package.get("name") or key.rsplit("node_modules/", 1)[-1], safe="")
            version = urllib.parse.quote(package["version"], safe="")
            for path in sorted(root.iterdir()):
                if path.name.upper().startswith(("LICENSE", "COPYING", "NOTICE")) and not path.is_dir():
                    files[f"third-party/npm/{name}-{version}/{path.name}"] = bounded_regular_bytes(path, 1024 * 1024)
    return files
