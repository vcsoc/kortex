import hashlib
import io
import os
import pathlib
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import install_private
import zipfile

from install_private import install


class InstallPrivateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.destination = self.root / "bin"
        self.destination.mkdir()
        self.binary = "kortex.exe" if os.name == "nt" else "kortex"

    def tar(self, entries):
        data = io.BytesIO()
        with tarfile.open(fileobj=data, mode="w:gz") as archive:
            for name, content, kind in entries:
                info = tarfile.TarInfo(name)
                info.type = kind
                info.size = len(content) if kind == tarfile.REGTYPE else 0
                archive.addfile(info, io.BytesIO(content) if info.size else None)
        path = self.root / "archive.tar.gz"
        path.write_bytes(data.getvalue())
        return path, hashlib.sha256(data.getvalue()).hexdigest()

    def test_tar_install_and_no_overwrite(self):
        path, digest = self.tar([("private/" + self.binary, b"inert binary bytes", tarfile.REGTYPE)])
        target = install(path, digest, self.destination)
        self.assertEqual(target.read_bytes(), b"inert binary bytes")
        with self.assertRaises(FileExistsError):
            install(path, digest, self.destination)
        self.assertEqual(target.read_bytes(), b"inert binary bytes")

    def test_hash_mismatch_and_duplicate_or_symlink_refuse(self):
        for entries in [
            [("private/" + self.binary, b"one", tarfile.REGTYPE), ("other/" + self.binary, b"two", tarfile.REGTYPE)],
            [("private/" + self.binary, b"", tarfile.SYMTYPE)],
        ]:
            path, digest = self.tar(entries)
            with self.assertRaises(ValueError):
                install(path, digest, self.destination)
        path, _ = self.tar([("private/" + self.binary, b"one", tarfile.REGTYPE)])
        with self.assertRaises(ValueError):
            install(path, "0" * 64, self.destination)
        self.assertFalse((self.destination / self.binary).exists())

    def test_tar_quickstart_installed_and_existing_guide_preserved(self):
        path, digest = self.tar([("private/" + self.binary, b"inert", tarfile.REGTYPE),
                                 ("private/docs/QUICKSTART.html", b"offline guide", tarfile.REGTYPE)])
        guide = self.destination / "kortex-quickstart.html"
        guide.write_bytes(b"existing")
        with self.assertRaises(FileExistsError):
            install(path, digest, self.destination)
        self.assertEqual(guide.read_bytes(), b"existing")
        self.assertFalse((self.destination / self.binary).exists())
        guide.unlink()
        install(path, digest, self.destination)
        self.assertEqual(guide.read_bytes(), b"offline guide")

    def test_explicit_browser_open_uses_local_file_url(self):
        path, digest = self.tar([("private/" + self.binary, b"inert", tarfile.REGTYPE),
                                 ("private/docs/QUICKSTART.html", b"offline guide", tarfile.REGTYPE)])
        with patch("sys.argv", ["install_private.py", str(path), "--sha256", digest,
                                 "--destination", str(self.destination), "--open-guide"]):
            with patch("install_private.webbrowser.open", return_value=True) as browser:
                self.assertEqual(install_private.main(), 0)
                browser.assert_called_once_with((self.destination / "kortex-quickstart.html").resolve().as_uri())

    def test_zip_and_unrelated_traversal_not_extracted(self):
        data = io.BytesIO()
        with zipfile.ZipFile(data, "w") as archive:
            archive.writestr("private/" + self.binary, b"inert zip binary")
            archive.writestr("../../outside", b"not extracted")
        path = self.root / "archive.zip"
        path.write_bytes(data.getvalue())
        target = install(path, hashlib.sha256(data.getvalue()).hexdigest(), self.destination)
        self.assertEqual(target.read_bytes(), b"inert zip binary")
        self.assertFalse((self.root / "outside").exists())


if __name__ == "__main__":
    unittest.main()
