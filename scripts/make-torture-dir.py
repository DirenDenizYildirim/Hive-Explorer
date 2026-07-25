#!/usr/bin/env python3
"""Generate a directory designed to break a file manager.

Everything in here corresponds to a stability requirement in Hive's README QA
checklist: deep nesting, names that are not valid UTF-8, names containing
newlines, broken symlinks, a symlink loop, a very large flat directory, a
zero-byte file, and a sparse multi-gigabyte file.

The sparse file is created with a seek-and-truncate, so it costs almost no disk
space despite reporting several gigabytes.

Usage:
    ./scripts/make-torture-dir.py [DIR]       # default: /tmp/hive-torture
    ./scripts/make-torture-dir.py DIR --big   # include the 50k-file directory

Only stdlib is used; there is no dependency to install.
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
from pathlib import Path

FLAT_FILE_COUNT = 50_000
DEEP_NESTING_DEPTH = 60
SPARSE_SIZE_BYTES = 4 * 1024**3  # 4 GiB, sparse


def fresh(root: Path) -> None:
    if root.exists():
        print(f"removing existing {root}")
        shutil.rmtree(root)
    root.mkdir(parents=True)


def weird_names(root: Path) -> None:
    """Names that are legal on Linux and hostile to naive code."""
    d = root / "weird-names"
    d.mkdir()

    (d / "spaces in the name.txt").write_text("ok\n")
    (d / "two\nlines.txt").write_text("a filename containing a newline\n")
    (d / "tab\there.txt").write_text("a filename containing a tab\n")
    (d / "quote'and\"quote.txt").write_text("ok\n")
    (d / "back\\slash.txt").write_text("ok\n")
    (d / "-leading-dash.txt").write_text("ok\n")
    (d / "--looks-like-a-flag").write_text("ok\n")
    (d / ".hidden-dotfile").write_text("ok\n")
    (d / "trailing-tilde~").write_text("looks like an editor backup\n")
    (d / "emoji-🐝-name.txt").write_text("ok\n")
    (d / "Straße-Übung-日本語.txt").write_text("non-ascii\n")
    (d / ("very-long-" + "x" * 200 + ".txt")).write_text("near NAME_MAX\n")
    (d / "case").write_text("lowercase\n")
    (d / "CASE").write_text("uppercase — same name on a case-insensitive fs\n")

    # Not valid UTF-8. Filenames on Linux are bytes; this must not make the
    # entry disappear from the listing or crash the sorter.
    for raw in (b"invalid-\xff\xfe-utf8.txt", b"\x80\x81\x82-leading-garbage"):
        try:
            os.close(os.open(os.path.join(os.fsencode(d), raw), os.O_CREAT | os.O_WRONLY, 0o644))
        except OSError as error:  # pragma: no cover - filesystem dependent
            print(f"  skipped {raw!r}: {error}", file=sys.stderr)


def symlinks(root: Path) -> None:
    d = root / "symlinks"
    d.mkdir()

    (d / "real-target.txt").write_text("the target\n")
    (d / "good-link.txt").symlink_to("real-target.txt")
    (d / "broken-link.txt").symlink_to("does-not-exist.txt")
    (d / "broken-absolute").symlink_to("/nonexistent/path/entirely")

    # A two-node symlink loop. Resolving this naively never terminates.
    (d / "loop-a").symlink_to("loop-b")
    (d / "loop-b").symlink_to("loop-a")

    # A directory symlink pointing at its own parent — a recursive walk that
    # follows symlinks will spin here forever.
    (d / "self-parent").symlink_to("..", target_is_directory=True)

    real_dir = d / "real-dir"
    real_dir.mkdir()
    (real_dir / "inside.txt").write_text("ok\n")
    (d / "dir-link").symlink_to("real-dir", target_is_directory=True)


def sizes(root: Path) -> None:
    d = root / "sizes"
    d.mkdir()

    (d / "zero-byte.txt").touch()
    (d / "one-byte.txt").write_bytes(b"x")
    (d / "small.txt").write_text("hello\n" * 100)
    (d / "medium.bin").write_bytes(os.urandom(4 * 1024 * 1024))

    # Sparse: reports 4 GiB, occupies almost nothing.
    sparse = d / "sparse-4gib.bin"
    with open(sparse, "wb") as handle:
        handle.truncate(SPARSE_SIZE_BYTES)


def deep_nesting(root: Path) -> None:
    path = root / "deep"
    path.mkdir()
    for level in range(DEEP_NESTING_DEPTH):
        path = path / f"level-{level:02d}"
        path.mkdir()
    (path / "bottom.txt").write_text("you made it\n")


def permissions(root: Path) -> None:
    d = root / "permissions"
    d.mkdir()

    (d / "readable.txt").write_text("ok\n")

    unreadable_dir = d / "no-access-dir"
    unreadable_dir.mkdir()
    (unreadable_dir / "secret.txt").write_text("cannot list me\n")
    unreadable_dir.chmod(0o000)

    unreadable_file = d / "no-read-file.txt"
    unreadable_file.write_text("cannot read me\n")
    unreadable_file.chmod(0o000)


def many_files(root: Path, count: int) -> None:
    d = root / f"many-files-{count}"
    d.mkdir()
    # Mixed-width numbering so natural sort ordering is actually exercised:
    # file2 must sort before file10.
    for index in range(count):
        (d / f"file{index}.txt").write_bytes(b"")
    print(f"  created {count} files in {d}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dir", nargs="?", default="/tmp/hive-torture")
    parser.add_argument(
        "--big",
        action="store_true",
        help=f"also create a directory of {FLAT_FILE_COUNT} files (slow)",
    )
    parser.add_argument(
        "--clean",
        action="store_true",
        help="remove the directory and exit",
    )
    args = parser.parse_args()

    root = Path(args.dir).expanduser().resolve()

    if args.clean:
        # chmod back first, or rmtree cannot descend into permissions/.
        for path in root.rglob("*"):
            try:
                path.chmod(0o755)
            except OSError:
                pass
        shutil.rmtree(root, ignore_errors=True)
        print(f"removed {root}")
        return 0

    fresh(root)
    print(f"building torture directory at {root}")

    weird_names(root)
    print("  weird names")
    symlinks(root)
    print("  symlinks, broken links, and a loop")
    sizes(root)
    print("  zero-byte, small, and sparse 4 GiB files")
    deep_nesting(root)
    print(f"  {DEEP_NESTING_DEPTH} levels of nesting")
    permissions(root)
    print("  unreadable file and directory")

    # 2000 entries is the thumbnail auto-disable threshold; 2500 crosses it
    # without the wait that 50k involves.
    many_files(root, 2_500)
    if args.big:
        many_files(root, FLAT_FILE_COUNT)

    print()
    print(f"done: {root}")
    print("clean up with:")
    print(f"  {sys.argv[0]} {root} --clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
