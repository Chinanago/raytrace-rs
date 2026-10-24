"""Subset the bundled Noto fonts for the GUI.

Usage (fonttools is only needed when regenerating the bundled fonts):
    uv run --no-project --with fonttools python scripts/subset_fonts.py SOURCE_DIR

SOURCE_DIR holds the unmodified upstream files:
    NotoSansJP-Regular.otf, NotoSansJP-Bold.otf  (notofonts/noto-cjk, Sans/SubsetOTF/JP)
    NotoSansMono-Regular.ttf                      (notofonts/notofonts.github.io, fonts/NotoSansMono/hinted/ttf)

Japanese text keeps JIS X 0208 plus the cp932 (NEC/IBM) extensions and every
non-ASCII character used in the repository sources, so UI strings never fall
back to tofu. Glyphs outside this set fall back to egui's default fonts.
"""
import sys
from pathlib import Path

from fontTools import subset

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "crates" / "raytrace-gui" / "assets" / "fonts"

# Latin, Greek (λ, μ, θ in optics labels), punctuation, units and math symbols.
COMMON = [
    (0x20, 0x7E), (0xA0, 0xFF), (0x370, 0x3FF), (0x2000, 0x206F), (0x2070, 0x209F),
    (0x2100, 0x214F), (0x2150, 0x218F), (0x2190, 0x21FF), (0x2200, 0x22FF), (0x2300, 0x23FF),
    (0x2460, 0x24FF), (0x2500, 0x257F), (0x25A0, 0x25FF), (0x2600, 0x26FF), (0x2700, 0x27BF),
]
JAPANESE = [(0x3000, 0x303F), (0x3040, 0x309F), (0x30A0, 0x30FF), (0x31F0, 0x31FF), (0xFF00, 0xFFEF)]


def ranges(spans):
    return {c for lo, hi in spans for c in range(lo, hi + 1)}


def cp932_characters():
    chars = set()
    for lead in list(range(0x81, 0xA0)) + list(range(0xE0, 0xFD)):
        for trail in list(range(0x40, 0x7F)) + list(range(0x80, 0xFD)):
            try:
                chars.update(ord(c) for c in bytes([lead, trail]).decode("cp932"))
            except UnicodeDecodeError:
                pass
    return chars


def source_characters():
    chars = set()
    for pattern in ("crates/**/*.rs", "src/**/*.rs", "examples/**/*.rs", "examples/*.json"):
        for path in ROOT.glob(pattern):
            chars.update(ord(c) for c in path.read_text(encoding="utf-8") if ord(c) > 0x7E)
    return chars


def write(source, target, codepoints):
    options = subset.Options()
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    options.notdef_outline = True
    options.glyph_names = False
    options.hinting = True
    font = subset.load_font(str(source), options)
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=sorted(codepoints))
    subsetter.subset(font)
    target.parent.mkdir(parents=True, exist_ok=True)
    subset.save_font(font, str(target), options)
    print(f"{target.relative_to(ROOT)}: {target.stat().st_size / 1e6:.2f} MB")


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    source = Path(sys.argv[1])
    common = ranges(COMMON)
    japanese = common | ranges(JAPANESE) | cp932_characters() | source_characters()
    for name in ("NotoSansJP-Regular.otf", "NotoSansJP-Bold.otf"):
        write(source / name, OUT / name, japanese)
    write(source / "NotoSansMono-Regular.ttf", OUT / "NotoSansMono-Regular.ttf", common)


if __name__ == "__main__":
    main()
