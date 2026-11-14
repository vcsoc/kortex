#!/usr/bin/env python3
"""Install an explicitly supplied private Kortex development archive.

This offline helper does not authorize distribution or trust a checksum from the
archive itself. The expected SHA-256 must come from a separate trusted channel.
"""
import argparse
import hashlib
import os
import pathlib
import re
import stat
import sys
import tarfile
import zipfile
import webbrowser

MAX_ARCHIVE = 512 * 1024 * 1024
MAX_BINARY = 256 * 1024 * 1024
MAX_GUIDE = 1024 * 1024


def install(archive, expected_sha256, destination):
    if not re.fullmatch(r"[0-9a-fA-F]{64}", expected_sha256):
        raise ValueError("expected SHA-256 must be 64 hexadecimal characters")
    archive = pathlib.Path(archive)
    destination = pathlib.Path(destination)
    if archive.is_symlink() or not archive.is_file() or archive.stat().st_size > MAX_ARCHIVE:
        raise ValueError("archive must be a bounded regular file, not an alias")
    # Hold the same bytes for hashing and parsing: no preflight/ambient reopen.
    with archive.open("rb") as source:
        data = source.read(MAX_ARCHIVE + 1)
    if len(data) > MAX_ARCHIVE or hashlib.sha256(data).hexdigest() != expected_sha256.lower():
        raise ValueError("archive SHA-256 mismatch or archive exceeds limit")
    import io
    binary_name = "kortex.exe" if os.name == "nt" else "kortex"
    payload = None
    guide = None
    if zipfile.is_zipfile(io.BytesIO(data)):
        with zipfile.ZipFile(io.BytesIO(data)) as package:
            matches = [m for m in package.infolist() if len(pathlib.PurePosixPath(m.filename).parts) == 2 and pathlib.PurePosixPath(m.filename).name == binary_name]
            if len(matches) != 1 or matches[0].file_size > MAX_BINARY or matches[0].is_dir() or stat.S_ISLNK(matches[0].external_attr >> 16):
                raise ValueError("archive must contain one bounded regular Kortex binary")
            with package.open(matches[0]) as member:
                payload = member.read(MAX_BINARY + 1)
            guides = [m for m in package.infolist() if len(pathlib.PurePosixPath(m.filename).parts) == 3 and pathlib.PurePosixPath(m.filename).parts[1:] == ("docs", "QUICKSTART.html")]
            if len(guides) > 1 or any(g.is_dir() or stat.S_ISLNK(g.external_attr >> 16) or g.file_size > MAX_GUIDE for g in guides):
                raise ValueError("archive contains invalid or duplicate quick-start pages")
            if guides:
                with package.open(guides[0]) as member:
                    guide = member.read(MAX_GUIDE + 1)
    else:
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as package:
            matches = [m for m in package if len(pathlib.PurePosixPath(m.name).parts) == 2 and pathlib.PurePosixPath(m.name).name == binary_name]
            if len(matches) != 1 or not matches[0].isfile() or matches[0].size > MAX_BINARY:
                raise ValueError("archive must contain one bounded regular Kortex binary")
            member = package.extractfile(matches[0])
            if member is None:
                raise ValueError("binary could not be read")
            with member:
                payload = member.read(MAX_BINARY + 1)
            guides = [m for m in package.getmembers() if len(pathlib.PurePosixPath(m.name).parts) == 3 and pathlib.PurePosixPath(m.name).parts[1:] == ("docs", "QUICKSTART.html")]
            if len(guides) > 1 or any(not g.isfile() or g.size > MAX_GUIDE for g in guides):
                raise ValueError("archive contains invalid or duplicate quick-start pages")
            if guides:
                member = package.extractfile(guides[0])
                if member is None:
                    raise ValueError("quick-start page could not be read")
                with member:
                    guide = member.read(MAX_GUIDE + 1)
    if guide is not None and len(guide) > MAX_GUIDE:
        raise ValueError("quick-start page exceeds limit")
    if not payload or len(payload) > MAX_BINARY:
        raise ValueError("binary is empty or exceeds limit")
    # The destination is explicitly chosen and must already exist; never replace
    # an existing executable or alter a user's PATH/configuration implicitly.
    if not destination.is_dir() or destination.is_symlink():
        raise ValueError("destination must be an existing directory, not an alias")
    target = destination / binary_name
    guide_target = destination / "kortex-quickstart.html"
    if guide is not None and (guide_target.exists() or guide_target.is_symlink()):
        raise FileExistsError(f"quick-start page already exists: {guide_target}")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    fd = os.open(target, flags, 0o700)
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(payload)
    except BaseException:
        # Do not remove a path after a failure: another actor may have replaced it.
        raise
    if guide is not None:
        # Exclusive creation refuses both existing files and final-component aliases.
        with os.fdopen(os.open(guide_target, flags, 0o600), "wb") as output:
            output.write(guide)
    return target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=pathlib.Path)
    parser.add_argument("--sha256", required=True, help="SHA-256 obtained through a trusted independent channel")
    parser.add_argument("--destination", required=True, type=pathlib.Path, help="existing user-owned directory")
    parser.add_argument("--open-guide", action="store_true", help="open the installed offline guide in the default browser")
    args = parser.parse_args()
    try:
        target = install(args.archive, args.sha256, args.destination)
    except (OSError, ValueError, tarfile.TarError, zipfile.BadZipFile) as error:
        print(f"Installation failed: {error}", file=sys.stderr)
        return 1
    print(f"Installed {target}. Add its directory to PATH if needed; run kortex init in your project.")
    guide = args.destination / "kortex-quickstart.html"
    if guide.is_file():
        print(f"Offline quick start: {guide}")
        if args.open_guide and not webbrowser.open(guide.resolve().as_uri()):
            print("Browser could not be opened; open the quick-start path manually.", file=sys.stderr)
    elif args.open_guide:
        print("This archive has no quick-start page.", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
