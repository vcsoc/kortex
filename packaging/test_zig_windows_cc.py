"""Argument-adapter unit tests, not Windows execution or compiler acceptance."""
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location(
    "kortex_zig_windows_cc", Path(__file__).with_name("zig_windows_cc.py")
)
driver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(driver)


class ZigCompilerArgumentTests(unittest.TestCase):
    def test_default_target_is_injected_for_compiler_family_probes(self):
        self.assertEqual(driver.mapped_arguments(["-E", "probe.c"]),
                         ["-target", driver.ZIG_TARGET, "-E", "probe.c"])

    def test_vendor_qualified_target_is_mapped_in_both_forms(self):
        for flag in ("-target", "--target"):
            with self.subTest(flag=flag):
                self.assertEqual(driver.mapped_arguments([flag, driver.RUST_TARGET, "-c", "a.c"]),
                                 [flag, driver.ZIG_TARGET, "-c", "a.c"])
                self.assertEqual(driver.mapped_arguments([flag + "=" + driver.RUST_TARGET, "a.c"]),
                                 [flag + "=" + driver.ZIG_TARGET, "a.c"])

    def test_other_explicit_targets_are_not_falsely_relabelled(self):
        self.assertEqual(driver.mapped_arguments(["--target=aarch64-linux-gnu", "a.c"]),
                         ["--target=aarch64-linux-gnu", "a.c"])

    def test_source_paths_and_flags_remain_exact(self):
        words = ["-c", "a path/x86_64-pc-windows-gnu/source.c", "-DNAME=a value", "-o", "a.o"]
        self.assertEqual(driver.mapped_arguments(words)[2:], words)

    def test_missing_target_value_is_not_silently_repaired(self):
        self.assertEqual(driver.mapped_arguments(["-target"]), ["-target"])

    def test_existing_zig_target_is_preserved_without_duplication(self):
        words = ["-target", driver.ZIG_TARGET, "-c", "a.c"]
        self.assertEqual(driver.mapped_arguments(words), words)


if __name__ == "__main__":
    unittest.main()
