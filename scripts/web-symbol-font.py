#!/usr/bin/env python3
"""Build the browser's four lazy outline-only symbol/emoji fallbacks (fontTools)."""
import hashlib
import os
from pathlib import Path
import shutil
import sys


def build(out):
    try:
        from fontTools import subset
        from fontTools.ttLib import TTFont
        from fontTools.varLib.instancer import instantiateVariableFont
    except ImportError as error:
        raise ValueError("fontTools is missing; install it with 'uv tool install fonttools'") from error

    home = Path.home()
    folders = [Path(p) for p in [
        "/usr/share/fonts/noto", "/usr/share/fonts/truetype/noto",
        "/usr/share/fonts/opentype/noto", "/usr/share/fonts/google-noto",
        str(home / ".local/share/fonts/noto-emoji"), str(home / ".local/share/fonts"),
        str(home / "Library/Fonts"), "/Library/Fonts",
    ]]
    specs = [
        ("symbols", "FIGSTUDIO_SYMBOLS_FONT", ["NotoSansSymbols-Regular.ttf"], "Noto Sans Symbols"),
        ("symbols2", "FIGSTUDIO_SYMBOLS2_FONT", ["NotoSansSymbols2-Regular.ttf"], "Noto Sans Symbols 2"),
        ("math", "FIGSTUDIO_MATH_FONT", ["NotoSansMath-Regular.ttf"], "Noto Sans Math"),
        ("emoji", "FIGSTUDIO_EMOJI_FONT", ["NotoEmoji-Regular.ttf", "NotoEmoji-Variable.ttf", "NotoEmoji[wght].ttf"], "Noto Emoji"),
    ]
    sources = []
    for name, env, filenames, family in specs:
        override = os.environ.get(env)
        source = Path(override) if override else next(
            (folder / f for folder in folders for f in filenames if (folder / f).is_file()), None)
        if source is None or not source.is_file():
            raise ValueError(f"missing {family}; install it or set {env}")
        sources.append((name, source, family))

    ranges = [(0x20A0, 0x20CF), (0x2100, 0x214F), (0x2190, 0x2BFF),
              (0x1F000, 0x1FAFF), (0x1FB00, 0x1FBFF)]
    unicodes = {c for lo, hi in ranges for c in range(lo, hi + 1)}
    cache = Path(os.environ.get("CARGO_TARGET_DIR", "target")) / "web-font"
    cache.mkdir(parents=True, exist_ok=True)
    out.mkdir(parents=True, exist_ok=True)
    licences = []
    for name, source, family in sources:
        font = TTFont(source)
        actual = font["name"].getDebugName(1)
        if actual != family or "glyf" not in font:
            raise ValueError(f"{name}: expected outline-only {family}, not a bitmap colour face")
        if "open font license" not in (font["name"].getDebugName(13) or "").lower():
            raise ValueError(f"{name}: source must declare the SIL Open Font License")
        licences.append(f"{name}.ttf: {actual}\n{font['name'].getDebugName(0)}")
        expected = set(font.getBestCmap()) & unicodes
        dest = cache / f"{name}.ttf"
        marker = cache / f"{name}.ttf.source"
        fingerprint = hashlib.sha256(Path(__file__).read_bytes() + source.read_bytes()).hexdigest()
        if not dest.is_file() or not marker.is_file() or marker.read_text() != fingerprint:
            if "fvar" in font:
                font = instantiateVariableFont(font, {a.axisTag: a.defaultValue for a in font["fvar"].axes})
            options = subset.Options()
            options.hinting = False
            options.layout_features = []  # egui draws scalars, not shaped emoji ligatures.
            options.name_IDs = ["*"]
            options.name_languages = ["*"]
            subsetter = subset.Subsetter(options=options)
            subsetter.populate(unicodes=unicodes)
            subsetter.subset(font)
            part = dest.with_suffix(".ttf.part")
            font.save(part)
            part.replace(dest)
            marker.write_text(fingerprint)
        font.close()
        with TTFont(dest) as check:
            if set(check.getBestCmap()) != expected or "glyf" not in check:
                raise ValueError(f"{name}: subset lost source coverage or outlines")
            if any(table in check for table in ["sbix", "CBDT", "CBLC", "COLR"]):
                raise ValueError(f"{name}: browser fallback must be monochrome outlines")
        shutil.copyfile(dest, out / dest.name)
        print(f"web-font: {dest.name} {dest.stat().st_size} bytes")
    # Full OFL text, plus the actual sources' copyright notices; no machine-local paths.
    licence = Path(__file__).with_name("font-OFL.txt").read_text()
    (out / "symbols-LICENSE.txt").write_text("\n\n".join(licences) + "\n\n" + licence)


if __name__ == "__main__":
    try:
        build(Path(sys.argv[1]))
    except (ValueError, OSError) as error:
        if os.environ.get("FIGSTUDIO_ALLOW_MISSING_SYMBOLS") == "1":
            print(f"web-font: {error}; symbol fallback omitted (explicit local opt-out)", file=sys.stderr)
        else:
            sys.exit(f"web-font: {error}; FIGSTUDIO_ALLOW_MISSING_SYMBOLS=1 permits a local build without symbols")
