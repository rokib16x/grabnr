"""Generates every grabnr brand asset from one definition of the "Chunk g" mark.

The mark: a lowercase g on a blue tile. Its bowl is drawn in three pieces, like a file arriving in chunks over
several links; the last piece lands in lime. Wordmarks are Plus Jakarta Sans ExtraBold, converted to outlines.

Needs fontTools + brotli (to read the woff2 font) and the `resvg` CLI (to render PNGs):
    python3 -m venv .venv && .venv/bin/pip install fonttools brotli && cargo install resvg
    .venv/bin/python assets/brand/make_brand.py [path/to/resvg]
Run from the repository root.
"""

import math
import os
import shutil
import subprocess
import sys

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

ROOT = os.getcwd()
BRAND = os.path.join(ROOT, "assets", "brand")
RESVG = sys.argv[1] if len(sys.argv) > 1 else shutil.which("resvg") or "resvg"
FONT = os.path.join(ROOT, "node_modules", "@fontsource-variable", "plus-jakarta-sans", "files", "plus-jakarta-sans-latin-wght-normal.woff2")

BLUE, BLUE_DEEP, LIME, NAVY, WHITE = "#2563eb", "#1d4ed8", "#bef264", "#0f172a", "#ffffff"
TAGLINE = "Download faster over every network connection at once"


# --- The mark (256 x 256 design grid) ------------------------------------------------------------
def arc(cx, cy, r, a0, a1):
    x0, y0 = cx + r * math.cos(math.radians(a0)), cy + r * math.sin(math.radians(a0))
    x1, y1 = cx + r * math.cos(math.radians(a1)), cy + r * math.sin(math.radians(a1))
    large = 1 if (a1 - a0) % 360 > 180 else 0
    return f"M{x0:.2f} {y0:.2f}A{r} {r} 0 {large} 1 {x1:.2f} {y1:.2f}"


def glyph(main=WHITE, accent=LIME):
    """The g itself: a bowl in three pieces and a round-capped stem with its hook.

    The bowl's gaps sit at the top, lower left and bottom, all clear of the stem, so every cut is visible and no
    cut lands under the stem (which would leave slivers). The right side of the bowl runs under the stem in one piece.
    """
    s = 'fill="none" stroke-width="30" stroke-linejoin="round"'
    return (f'<g {s} stroke-linecap="butt"><path d="{arc(114, 104, 46, 156, 248)}" stroke="{accent}"/>'
            f'<path d="{arc(114, 104, 46, 262, 448)}" stroke="{main}"/><path d="{arc(114, 104, 46, 100, 142)}" stroke="{main}"/></g>'
            f'<path {s} stroke-linecap="round" stroke="{main}" d="M166 60V170C166 206 138 216 114 216C98 216 86 211 77 202"/>')


def tile(bg_top=BLUE, bg_bottom=BLUE_DEEP, main=WHITE, accent=LIME, uid="t"):
    """The g on its rounded tile, in a 256 grid. A soft top-to-bottom gradient gives it depth at large sizes."""
    return (f'<defs><linearGradient id="{uid}" x1="0" y1="0" x2="0.35" y2="1"><stop offset="0" stop-color="{bg_top}"/>'
            f'<stop offset="1" stop-color="{bg_bottom}"/></linearGradient></defs>'
            f'<rect width="256" height="256" rx="58" fill="url(#{uid})"/>' + glyph(main, accent))


def svg(w, h, body, bg=None):
    back = f'<rect width="{w}" height="{h}" fill="{bg}"/>' if bg else ""
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">{back}{body}</svg>\n'


def place(body, x, y, size):
    return f'<g transform="translate({x:.2f} {y:.2f}) scale({size / 256:.5f})">{body}</g>'


# --- Text as outlines ----------------------------------------------------------------------------
_fonts = {}


def font(weight):
    if weight not in _fonts:
        f = TTFont(FONT)
        _fonts[weight] = instantiateVariableFont(f, {"wght": weight})
    return _fonts[weight]


def text(s, x, y, size, color, weight=800, tracking=-0.035):
    """Return (svg path, width) for `s` with its baseline at y, in Plus Jakarta Sans at `weight`."""
    f = font(weight)
    upm, cmap, gs, hmtx = f["head"].unitsPerEm, f.getBestCmap(), f.getGlyphSet(), f["hmtx"]
    k = size / upm
    pen = SVGPathPen(gs)
    cx = 0.0
    for ch in s:
        name = cmap[ord(ch)]
        gs[name].draw(TransformPen(pen, (k, 0, 0, -k, x + cx, y)))
        cx += hmtx[name][0] * k + tracking * size
    width = cx - tracking * size
    return f'<path fill="{color}" d="{pen.getCommands()}"/>', width


def cap_height(size, weight=800):
    os2 = font(weight)["OS/2"]
    return os2.sCapHeight * size / font(weight)["head"].unitsPerEm


# --- Compositions --------------------------------------------------------------------------------
def lockup(w, h, word_color, tile_size, word_size, uid):
    """Tile on the left, wordmark to its right, scaled down if needed so the pair fits 96% of the width, centred."""
    _, ww = text("grabnr", 0, 0, word_size, word_color)
    gap = tile_size * 0.26
    k = min(1.0, w * 0.96 / (tile_size + gap + ww))
    tile_size, word_size, gap, ww = tile_size * k, word_size * k, gap * k, ww * k
    x = (w - (tile_size + gap + ww)) / 2
    ty = (h - tile_size) / 2
    xh = font(800)["OS/2"].sxHeight * word_size / font(800)["head"].unitsPerEm
    baseline = h / 2 + xh / 2  # optically centre the x-height on the tile
    path, _ = text("grabnr", x + tile_size + gap, baseline, word_size, word_color)
    return place(tile(uid=uid), x, ty, tile_size) + path


def stacked(w, h, word_color, tagline=None, tag_color=None):
    t = 512
    body = place(tile(uid="s"), (w - t) / 2, 40, t)
    _, ww = text("grabnr", 0, 0, 240, word_color)
    word, _ = text("grabnr", (w - ww) / 2, 40 + t + 230, 240, word_color)
    body += word
    if tagline:
        _, tw = text(tagline, 0, 0, 44, tag_color, 500, -0.01)
        tag, _ = text(tagline, (w - tw) / 2, 40 + t + 340, 44, tag_color, 500, -0.01)
        body += tag
    return body


def banner(w, h, bg, word_color, tag_color, uid, tag_size=34):
    """Wide banner: tile, then the wordmark with the tagline under it, scaled to fit and centred."""
    ts = h * 0.5
    word_size = ts * 0.78
    for _ in range(20):  # shrink until the whole block fits in 86% of the width
        word_size = ts * 0.78
        _, ww = text("grabnr", 0, 0, word_size, word_color)
        _, tw = text(TAGLINE, 0, 0, tag_size, tag_color, 500, -0.01)
        gap = ts * 0.24
        block_w = ts + gap + max(ww, tw)
        if block_w <= w * 0.86:
            break
        ts *= 0.94
        tag_size *= 0.94
    f = font(800)
    upm = f["head"].unitsPerEm
    asc, desc = f["OS/2"].sxHeight * word_size / upm, -f["hhea"].descent * word_size / upm * 0.8
    # Vertical block: x-height of the word, its descender, a gap, then the tagline's cap height.
    tag_cap = cap_height(tag_size, 500)
    block_h = asc + desc + tag_size * 0.55 + tag_cap
    top = (h - block_h) / 2
    word_base = top + asc + word_size * 0.0
    tag_base = word_base + desc + tag_size * 0.55 + tag_cap
    x = (w - block_w) / 2
    y = (h - ts) / 2
    tx = x + ts + gap
    word, _ = text("grabnr", tx, word_base, word_size, word_color)
    tag, _ = text(TAGLINE, tx + word_size * 0.03, tag_base, tag_size, tag_color, 500, -0.01)
    return svg(w, h, place(tile(uid=uid), x, y, ts) + word + tag, bg)


def write(path, content):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(content)


def png(src, out, w, h=None):
    h = h or w
    subprocess.run([RESVG, "-w", str(w), "-h", str(h), src, out], check=True)


# --- Write everything ----------------------------------------------------------------------------
def main():
    mark = svg(256, 256, tile())
    mono = svg(256, 256, f'<g transform="translate(6 -4)">{glyph("#000", "#000")}</g>')
    app = svg(1024, 1024, place(tile(), 100, 100, 824))
    app_dark = svg(1024, 1024, place(tile("#1e293b", NAVY, WHITE, LIME, "d"), 100, 100, 824))
    files = {
        "assets/brand/grabnr-mark.svg": mark,
        "assets/brand/grabnr-mark-mono.svg": mono,
        "assets/brand/grabnr-app-icon.svg": app,
        "assets/brand/grabnr-app-icon-dark.svg": app_dark,
        "assets/brand/grabnr-logo.svg": svg(1200, 880, stacked(1200, 880, NAVY)),
        "assets/brand/grabnr-logo-dark.svg": svg(1200, 880, stacked(1200, 880, WHITE), NAVY),
        "assets/brand/grabnr-logo-tagline.svg": svg(1200, 1000, stacked(1200, 1000, NAVY, TAGLINE, "#475569"), WHITE),
        "assets/brand/readme-banner.svg": banner(1280, 320, "#f5f8ff", NAVY, "#475569", "b"),
        "assets/brand/github-social-preview.svg": banner(1280, 640, NAVY, WHITE, "#94a3b8", "g", 38),
        "assets/brand/github-social-preview-light.svg": banner(1280, 640, "#f5f8ff", NAVY, "#475569", "l", 38),
        "site/assets/grabnr-mark.svg": mark,
        "site/assets/grabnr-lockup.svg": svg(1330, 470, lockup(1330, 470, NAVY, 400, 330, "k")),
        "site/assets/grabnr-lockup-dark.svg": svg(1330, 470, lockup(1330, 470, WHITE, 400, 330, "k")),
        "public/grabnr.svg": mark,
    }
    for rel, content in files.items():
        write(os.path.join(ROOT, rel), content)

    b = lambda n: os.path.join(BRAND, n)  # noqa: E731
    for size in (16, 32, 48, 64, 128, 180, 192, 256, 512, 1024):
        png(b("grabnr-mark.svg"), b(f"grabnr-mark-{size}.png"), size)
    png(b("grabnr-app-icon.svg"), b("grabnr-app-icon-1024.png"), 1024)
    png(b("grabnr-app-icon-dark.svg"), b("grabnr-app-icon-dark-1024.png"), 1024)
    png(b("grabnr-logo.svg"), b("grabnr-logo.png"), 2400, 1760)
    png(b("grabnr-logo-dark.svg"), b("grabnr-logo-on-dark.png"), 2400, 1760)
    png(b("grabnr-logo-tagline.svg"), b("grabnr-logo-tagline.png"), 2400, 2000)
    png(b("grabnr-logo-tagline.svg"), b("grabnr-logo-on-white.png"), 2400, 2000)
    png(b("readme-banner.svg"), b("readme-banner.png"), 2560, 640)
    png(b("github-social-preview.svg"), b("github-social-preview.png"), 1280, 640)
    png(b("github-social-preview-light.svg"), b("github-social-preview-light.png"), 1280, 640)
    png(b("github-social-preview.svg"), os.path.join(ROOT, "site/assets/og.png"), 1280, 640)
    for size in (32, 180):
        png(b("grabnr-mark.svg"), os.path.join(ROOT, f"site/assets/grabnr-mark-{size}.png"), size)
    for size in (16, 32, 48, 128):
        png(b("grabnr-mark.svg"), os.path.join(ROOT, f"extension/icons/icon{size}.png"), size)
    # Menu bar: a template image (black + alpha) that macOS tints for light and dark menu bars.
    png(b("grabnr-mark-mono.svg"), os.path.join(ROOT, "src-tauri/icons/tray.png"), 41)
    print(f"wrote {len(files)} SVGs and the PNG set")


if __name__ == "__main__":
    main()
