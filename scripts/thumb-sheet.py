#!/usr/bin/env python3
"""Side-by-side sheet: Figma's stored thumbnail (left) vs figstudio's render (right).

Usage:
  scripts/thumb-sheet.py [-o sheet.png] [--scale 1] [file.fig ...]

Defaults to every file in tests/fixtures. Needs Pillow (pip install pillow) and a release
build of the CLI (cargo build --release -p figstudio-cli). Look at the sheet yourself:
a passing test suite does not mean the picture is right.

Differences that are expected (not bugs) are listed in docs/TESTING.md.
"""

import argparse
import pathlib
import subprocess
import sys
import tempfile

try:
    from PIL import Image, ImageDraw
except ImportError:
    sys.exit("thumb-sheet: needs Pillow (pip install pillow)")

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLI = ROOT / "target" / "release" / "figstudio-cli"
COL_W, ROW_H = 560, 260


def fit(im):
    s = min(COL_W / im.width, ROW_H / im.height)
    return im.resize((max(1, int(im.width * s)), max(1, int(im.height * s))))


def run(*args):
    p = subprocess.run([str(CLI), *map(str, args)], capture_output=True, text=True)
    return p.returncode, (p.stdout + p.stderr).strip()


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("files", nargs="*", type=pathlib.Path)
    ap.add_argument("-o", "--out", type=pathlib.Path, default=pathlib.Path("thumb-sheet.png"))
    ap.add_argument("--scale", default="1")
    a = ap.parse_args()
    if not CLI.exists():
        sys.exit(f"thumb-sheet: {CLI} missing; run cargo build --release -p figstudio-cli")
    files = a.files or sorted((ROOT / "tests" / "fixtures").glob("*.fig"))
    rows = []
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        for i, f in enumerate(files):
            ours, theirs = tmp / f"{i}.png", tmp / f"{i}_thumb.png"
            code, out = run("render", f, "-o", ours, "--scale", a.scale)
            stats = out.splitlines()[-1] if out else ""
            print(f"{f.name}: {stats}" if code == 0 else f"{f.name}: render failed: {out}")
            has_thumb = run("thumbnail", f, "-o", theirs)[0] == 0
            row = Image.new("RGB", (COL_W * 2 + 30, ROW_H + 22), "white")
            d = ImageDraw.Draw(row)
            d.text((5, 4), f"{f.name} | Figma thumbnail", fill="black")
            d.text((COL_W + 25, 4), "figstudio" if code == 0 else "figstudio: RENDER FAILED", fill="black")
            if has_thumb:
                row.paste(fit(Image.open(theirs).convert("RGB")), (5, 20))
            else:
                d.text((5, 40), "(no stored thumbnail)", fill="gray")
            if code == 0:
                row.paste(fit(Image.open(ours).convert("RGB")), (COL_W + 25, 20))
            rows.append(row)
    if not rows:
        sys.exit("thumb-sheet: no files")
    sheet = Image.new("RGB", (rows[0].width, sum(r.height for r in rows)), "white")
    y = 0
    for r in rows:
        sheet.paste(r, (0, y))
        y += r.height
    sheet.save(a.out)
    print(f"wrote {a.out} {sheet.size[0]}x{sheet.size[1]}")


if __name__ == "__main__":
    main()
