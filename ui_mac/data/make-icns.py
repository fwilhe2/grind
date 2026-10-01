#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# make-icns.py — Grind.icns from grind.svg, on any machine with ImageMagick (M10).
#
#   ui_mac/data/make-icns.py            # writes ui_mac/data/Grind.icns beside this script
#
# An .icns is a header and a run of (four-character tag, length, PNG) entries, one per size the
# Finder, the Dock and the app switcher ask for; macOS has read PNG payloads in every one of these
# tags since 10.7. Packing it here rather than with `iconutil` on the runner means the icon is a
# checked-in file a reviewer can open, built the same way as ui_win32's grind.ico, and the bundle
# step copies it.

import pathlib
import struct
import subprocess
import tempfile

HERE = pathlib.Path(__file__).resolve().parent

# Tag and pixel size: the 1x and 2x sizes of 16, 32, 128, 256 and 512 points.
SIZES = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),
    (b"icp6", 64),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic13", 256),
    (b"ic09", 512),
    (b"ic14", 512),
    (b"ic10", 1024),
]


def png(size: int, work: pathlib.Path) -> bytes:
    out = work / f"{size}.png"
    subprocess.run(
        [
            "magick",
            "-background",
            "none",
            "-density",
            "384",
            str(HERE / "grind.svg"),
            "-resize",
            f"{size}x{size}",
            # No timestamps or profiles, so the same SVG packs to the same bytes.
            "-strip",
            "-define",
            "png:exclude-chunks=date,time",
            f"PNG32:{out}",
        ],
        check=True,
    )
    return out.read_bytes()


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        work = pathlib.Path(tmp)
        cache: dict[int, bytes] = {}
        body = b""
        for tag, size in SIZES:
            data = cache.setdefault(size, png(size, work))
            body += tag + struct.pack(">I", 8 + len(data)) + data
    icns = b"icns" + struct.pack(">I", 8 + len(body)) + body
    (HERE / "Grind.icns").write_bytes(icns)
    print(f"Grind.icns: {len(icns)} bytes, {len(SIZES)} entries")


if __name__ == "__main__":
    main()
