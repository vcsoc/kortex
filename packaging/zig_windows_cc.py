#!/usr/bin/env python3
"""Development-only real Zig/Clang driver; never emulates compiler success."""
import os
from pathlib import Path
import sys

RUST_TARGET = "x86_64-pc-windows-gnu"
ZIG_TARGET = "x86_64-windows-gnu"


def mapped_arguments(arguments):
    result = []
    has_target = False
    target_value = False
    for word in arguments:
        if target_value:
            result.append(ZIG_TARGET if word == RUST_TARGET else word)
            target_value = False
        elif word in ("-target", "--target"):
            result.append(word)
            has_target = True
            target_value = True
        elif word.startswith(("-target=", "--target=")):
            flag, value = word.split("=", 1)
            result.append(flag + "=" + (ZIG_TARGET if value == RUST_TARGET else value))
            has_target = True
        else:
            result.append(word)
    return result if has_target else ["-target", ZIG_TARGET, *result]


def main():
    tools = Path(__file__).resolve().parent.parent / "target/tools"
    zig = str(tools / "zig-aarch64-macos-0.16.0/zig")
    # exec preserves the real compiler's output, signals and failure status.
    os.execv(zig, [zig, "cc", *mapped_arguments(sys.argv[1:])])


if __name__ == "__main__":
    main()
