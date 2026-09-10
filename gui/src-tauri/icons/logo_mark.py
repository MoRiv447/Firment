"""Generate every brand PNG from the mark's geometry.

The mark is three skewed bars, derived exactly from the site's CSS (see
gui/src-tauri/icons/logo.svg for the derivation). It is drawn here rather than
traced from a bitmap so every size is generated from the same numbers, and so
the source of truth stays the CSS instead of a 1024px PNG someone once exported.

Outputs, all written to gui/src-tauri/icons/ (then mirrored to the two public
directories by the caller):

  logo-{32,64,128,256,512}.png    transparent glyph, acid green  (dark grounds)
  logo-w-{32,64,128,256,512}.png  transparent glyph, dark green (light grounds)
  app-tile.png                    1024x1024 rounded tile, green on charcoal
  32x32.png / 128x128.png / 128x128@2x.png   the tauri bundle icons, same tile
  icon.ico / icon.icns            multi-frame, built from app-tile.png

`logo-w-512.png` did not exist before: the white set stopped at 256 while the
dark set went to 512, so the two families could not be swapped one for one.

`icon.png` is written here as an OPAQUE square on purpose. round_icon.py owns
that path and its input contract is an opaque square (it replaces the alpha
channel wholesale with a rounded mask, then writes back to the same path), so
handing it a finished rounded tile would mask it twice.

Run order matters: this script WRITES 32x32.png / 128x128.png / 128x128@2x.png
as charcoal tiles, and round_icon.py then OVERWRITES them from icon.png. If you
run round_icon.py, run this script again afterwards so the bundle icons are the
tiles rather than white squares.

    python gui/src-tauri/icons/logo_mark.py
"""
from __future__ import annotations

import io
import math
import struct
from pathlib import Path

from PIL import Image, ImageDraw

ICON_DIR = Path(r"D:\OldStudy66\Firment\gui\src-tauri\icons")

# The glyph, in the 26x30 CSS box, on a supersampled grid.
BAR_HEIGHT = 6.0
BARS = [(3.0, 25.0), (12.0, 19.0), (21.0, 8.0)]  # (top, width)
SKEW_DEG = -13.0
GLYPH_W = 26.0
GLYPH_H = 30.0

SS = 4  # supersample factor: Pillow has no antialiased polygon fill

ACID = (0xB4, 0xF7, 0x79, 255)
INK_ACID = (0x3B, 0x6D, 0x11, 255)
LIGHT_GLYPH = (0x4D, 0x7C, 0x0F, 255)
TILE_BG = (0x18, 0x18, 0x1B, 255)  # `bg` from the design tokens, not pure black


def _bar_polygon(top: float, width: float) -> list[tuple[float, float]]:
    """One bar as a quadrilateral, skewed about the centre of the 26x30 box.

    `transform: skew(-13deg)` with the default `transform-origin: 50% 50%`
    pivots on (13, 15), so each corner moves by
    `-tan(13deg) * (y - 15)` in x. Doing it per corner is what keeps the three
    bars parallel AND correctly offset from each other; a single shear of the
    bounding box would not.
    """
    tan = math.tan(math.radians(-SKEW_DEG))
    cx, cy = GLYPH_W / 2, GLYPH_H / 2

    def shear(x: float, y: float) -> tuple[float, float]:
        return (x - tan * (y - cy), y)

    bottom = top + BAR_HEIGHT
    return [
        shear(0.0, top),
        shear(width, top),
        shear(width, bottom),
        shear(0.0, bottom),
    ]


def glyph_bbox() -> tuple[float, float, float, float]:
    """Bounding box of the whole mark after the skew (it leans left at the
    top, so the tight box is not the 26x30 CSS box)."""
    xs: list[float] = []
    ys: list[float] = []
    for top, width in BARS:
        for x, y in _bar_polygon(top, width):
            xs.append(x)
            ys.append(y)
    return min(xs), min(ys), max(xs), max(ys)


def _bars_area() -> float:
    return sum(w * BAR_HEIGHT for _, w in BARS)


def _bars_centroid_x() -> float:
    """x of the mark's area centroid, after the skew.

    This is the number the optical nudge should follow: the drawn box is easy to
    centre, but what the eye centres on is the ink's mass. The skew moves it and
    so does the 25/19/8 taper -- the wide bar is at the TOP, where the lean
    displaces it furthest left.
    """
    total = 0.0
    weighted = 0.0
    for top, width in BARS:
        area = width * BAR_HEIGHT
        xs = [x for x, _ in _bar_polygon(top, width)]
        total += area
        weighted += area * (min(xs) + max(xs)) / 2
    return weighted / total


# Vertical breathing room (top and bottom).
PERIMETER_MARGIN = 0.06
# Horizontal breathing room (left and right). Larger than the vertical one
# because the mark is 30.5 wide by 24 tall -- 27% wider than it is tall -- so a
# square frame is horizontal-tight, and equal margins would put the top bar's
# right end at the edge.
SIDE_MARGIN = 0.10
# Slack reserved for the optical nudge, so the nudge eats margin rather than
# pushing the mark past it.
OPTICAL_SLACK = 0.06
# How much of the centroid centring to apply. 0 = pure bounding-box centring,
# 1 = pure centroid centring (which over-corrects this mark; see plan_layout).
OPTICAL_BLEND = 0.5


def plan_layout(size_big: float) -> tuple[float, float, float]:
    """(scale, pad_x, pad_y) for the mark inside a `size_big` square.

    Shared by both renderers so the glyph and the app tile cannot drift apart.
    """
    x0, y0, x1, y1 = glyph_bbox()
    gw, gh = x1 - x0, y1 - y0

    # Two DIFFERENT budgets, one per axis. Reusing the side budget for the
    # height (or the reverse) silently shrinks the mark -- that mistake is what
    # fed a "width" scale into the height and pushed the ink off the canvas.
    avail_w = size_big * (1 - SIDE_MARGIN * 2 - OPTICAL_SLACK)
    avail_h = size_big * (1 - PERIMETER_MARGIN * 2)
    scale = min(avail_w / gw, avail_h / gh)

    slack_x = size_big - gw * scale
    # Nudge toward putting the ink's CENTROID where the bounding box's centre
    # is, but only halfway: full centroid centring over-corrects a mark made of
    # solid parallelograms (it measured 5px left / 2px right at 32px, i.e. the
    # opposite imbalance). Half of it removes the geometric "reads right-heavy"
    # without pushing the bar ends off centre.
    nudge = (_bars_centroid_x() - x0 - gw / 2) * scale * OPTICAL_BLEND
    return scale, slack_x / 2 - nudge, (size_big - gh * scale) / 2


def render_glyph(size: int, color: tuple[int, int, int, int]) -> Image.Image:
    """The mark alone on transparency, fitted and optically centred in `size`."""
    big = size * SS
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    scale, pad_x, pad_y = plan_layout(big)
    x0, y0, _, _ = glyph_bbox()

    for top, width in BARS:
        pts = [((x - x0) * scale + pad_x, (y - y0) * scale + pad_y) for x, y in _bar_polygon(top, width)]
        draw.polygon(pts, fill=color)

    return img.resize((size, size), Image.LANCZOS)


def render_tile(size: int, glyph_color: tuple[int, int, int, int]) -> Image.Image:
    """The app tile: rounded charcoal square, mark centred inside."""
    big = size * SS
    tile = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    ImageDraw.Draw(tile).rounded_rectangle(
        [(0, 0), (big - 1, big - 1)],
        radius=int(big * 0.2237),  # iOS app-icon radius, same as round_icon.py
        fill=TILE_BG,
    )

    # Same layout as the standalone mark, just inside a smaller box so the tile
    # keeps a margin around it. Sharing plan_layout is the point: the two
    # renderers cannot drift into different optical corrections.
    inner = big * 0.76
    scale, pad_x, pad_y = plan_layout(inner)
    pad_x += (big - inner) / 2
    pad_y += (big - inner) / 2
    x0, y0, _, _ = glyph_bbox()

    draw = ImageDraw.Draw(tile)
    for top, width in BARS:
        pts = [((x - x0) * scale + pad_x, (y - y0) * scale + pad_y) for x, y in _bar_polygon(top, width)]
        draw.polygon(pts, fill=glyph_color)

    return tile.resize((size, size), Image.LANCZOS)


# Alpha at or above this counts as ink when measuring framing. LANCZOS leaves a
# 1-2px fringe of alpha < 8 around a sharp corner; treating that as ink measured
# a 32px favicon as having a 0px margin when its solid mark sat 6px in.
INK_ALPHA = 32


def ink_bbox(img: Image.Image) -> tuple[int, int, int, int]:
    """Bounding box of the MARK, ignoring the resampling fringe."""
    mask = img.split()[-1].point(lambda a: 255 if a >= INK_ALPHA else 0)
    box = mask.getbbox()
    if box is None:
        raise SystemExit("generated image is empty")
    return box


def glyph_bbox_in(img: Image.Image) -> tuple[int, int, int, int]:
    """Bounding box of the GLYPH inside a tile.

    A tile is fully opaque, so alpha cannot separate mark from ground the way it
    does for the transparent glyphs. The mark is a light green on a near-black
    ground, so the green channel is the discriminator.
    """
    green = img.convert("RGB").split()[1].point(lambda g: 255 if g > 100 else 0)
    box = green.getbbox()
    if box is None:
        raise SystemExit("no glyph found in the tile")
    return box


def check_ico_has_frames(path: Path, expect: int) -> None:
    """Assert an ICO really carries `expect` frames.

    There is history here: a PNG-in-ICO build was silently dropped by the
    resource compiler, the exe ended up with RT_ICON=0, and Windows fell back to
    a square white placeholder. A header check is cheap insurance against
    regenerating an icon that looks fine in the repo and broken after a build.
    """
    data = path.read_bytes()
    if len(data) < 6:
        raise SystemExit(f"{path.name}: too short to be an ICO")
    reserved, kind, count = struct.unpack("<HHH", data[:6])
    if (reserved, kind, count) != (0, 1, expect):
        raise SystemExit(
            f"{path.name}: header says reserved={reserved} type={kind} frames={count}, expected 0/1/{expect}"
        )
    print(f"  ico ok      {path.name}: {count} frames, {len(data)} B")


def check_framing(name: str, img: Image.Image, *, tile: bool = False) -> None:
    """Fail loudly if a generated asset is mis-framed.

    Eyeballing a 32px favicon is not a review, and the first pass of this
    script shipped a mark whose top bar ran into the right edge. Measure it.
    """
    size = img.size[0]
    left, top, right, bottom = (glyph_bbox_in(img) if tile else ink_bbox(img))
    gaps = {
        "left": left,
        "right": size - right,
        "top": top,
        "bottom": size - bottom,
    }
    smallest = min(gaps.values())
    # Scaled: 32px simply cannot hold a 1px margin as a percentage the way a
    # 512 can, and demanding it would reject every small favicon.
    need = max(1.0, size * 0.03)
    if smallest < need:
        raise SystemExit(
            f"{name}: {min(gaps, key=gaps.get)} margin is {smallest}px "
            f"(need {need:.1f}px) — the mark is clipping"
        )
    # The horizontal gaps must sit within a narrow band of each other; a wide
    # spread means the mark is visibly off-centre.
    spread = abs(gaps["left"] - gaps["right"]) / size
    if spread > 0.08:
        raise SystemExit(
            f"{name}: horizontal margins differ by {spread:.1%} "
            f"(left {gaps['left']}, right {gaps['right']}) — mark is off-centre"
        )
    print(
        f"  framing ok  ink {right - left}x{bottom - top} in {size}px  "
        f"margins L{gaps['left']} R{gaps['right']} T{gaps['top']} B{gaps['bottom']}"
    )


def render_opaque_square(size: int, glyph_color: tuple[int, int, int, int]) -> Image.Image:
    """The mark on an OPAQUE square, for round_icon.py's input contract.

    round_icon.py reads icon.png as an opaque square, replaces the alpha channel
    with a rounded mask, and writes icon.png back. Feeding it the finished rounded
    tile instead would apply the mask a second time to an already-transparent
    image. So icon.png gets the flat square here and round_icon.py owns the rest.
    """
    big = size * SS
    img = Image.new("RGBA", (big, big), (255, 255, 255, 255))
    draw = ImageDraw.Draw(img)
    inner = big * 0.70
    scale, pad_x, pad_y = plan_layout(inner)
    off = (big - inner) / 2
    pad_x += off
    pad_y += off
    x0, y0, _, _ = glyph_bbox()

    for top, width in BARS:
        pts = [((x - x0) * scale + pad_x, (y - y0) * scale + pad_y) for x, y in _bar_polygon(top, width)]
        draw.polygon(pts, fill=glyph_color)

    return img.resize((size, size), Image.LANCZOS)


def write(img: Image.Image, name: str, *, tile: bool = False) -> None:
    check_framing(name, img, tile=tile)
    path = ICON_DIR / name
    img.save(path, format="PNG", optimize=True)
    print(f"wrote {name:22} {img.size[0]:>4}x{img.size[1]:<4} {path.stat().st_size:>7} B")


def build_bmp_ico(frames: list[Image.Image]) -> bytes:
    """Multi-frame ICO, each frame a 32-bit BMP DIB (BGRA, bottom-up).

    Copied from round_icon.py rather than imported: it is the encoding winres
    actually embeds. A PNG-in-ICO was silently dropped there once (the built exe
    had RT_ICON=0 and Windows fell back to a square placeholder).
    """
    out = io.BytesIO()
    out.write(struct.pack("<HHH", 0, 1, len(frames)))

    blobs: list[tuple[int, int, bytes]] = []
    for im in frames:
        w, h = im.size
        rgba = im.convert("RGBA").transpose(Image.FLIP_TOP_BOTTOM)
        raw = rgba.tobytes()
        px = bytearray()
        for i in range(0, len(raw), 4):
            r, g, b, a = raw[i], raw[i + 1], raw[i + 2], raw[i + 3]
            px += bytes((b, g, r, a))
        bih = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, len(px), 0, 0, 0, 0)
        blobs.append((w, h, bih + bytes(px)))

    offset = 6 + 16 * len(blobs)
    for w, h, blob in blobs:
        out.write(struct.pack("<BBBBHHII", 0 if w >= 256 else w, 0 if h >= 256 else h, 0, 0, 1, 32, len(blob), offset))
        offset += len(blob)
    for _, _, blob in blobs:
        out.write(blob)
    return out.getvalue()


PUBLIC_DIRS = [
    Path(r"D:\OldStudy66\Firment\gui\public\icons"),
    Path(r"D:\OldStudy66\Firment\web\public"),
]

# Names shared by all three trees. `app-tile.png` is deliberately absent: it is
# the build source for the bundle icons and nothing serves it at runtime.
MIRRORED = [
    *[f"logo-{s}.png" for s in (32, 64, 128, 256, 512)],
    *[f"logo-w-{s}.png" for s in (32, 64, 128, 256, 512)],
]


def mirror_public() -> None:
    """Copy the shared PNGs so the three trees stay byte-identical.

    They were byte-identical before this change (verified across 25 files) and
    the point of mirroring here rather than by hand is that a regeneration
    cannot leave one of them stale -- which is exactly what a manual copy
    eventually does.
    """
    for dest in PUBLIC_DIRS:
        if not dest.is_dir():
            print(f"skip (missing): {dest}")
            continue
        for name in MIRRORED:
            src = ICON_DIR / name
            if not src.exists():
                raise SystemExit(f"nothing to mirror for {name}")
            (dest / name).write_bytes(src.read_bytes())
        print(f"mirrored {len(MIRRORED)} files -> {dest}")


def check_mirrors() -> None:
    import hashlib

    def digest(p: Path) -> str:
        return hashlib.sha256(p.read_bytes()).hexdigest()[:12]

    for name in MIRRORED:
        heads = {digest(d / name) for d in [ICON_DIR, *PUBLIC_DIRS] if (d / name).exists()}
        if len(heads) != 1:
            raise SystemExit(f"{name}: the three trees disagree ({heads})")
    print(f"  mirrors ok  {len(MIRRORED)} files identical across {1 + len(PUBLIC_DIRS)} trees")


def main() -> None:
    for size in (32, 64, 128, 256, 512):
        write(render_glyph(size, ACID), f"logo-{size}.png")
    for size in (32, 64, 128, 256, 512):
        write(render_glyph(size, LIGHT_GLYPH), f"logo-w-{size}.png")

    tile = render_tile(1024, ACID)
    write(tile, "app-tile.png", tile=True)
    for name, size in (("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256)):
        write(render_tile(size, ACID), name, tile=True)

    # round_icon.py's input: an opaque square it will mask into a rounded tile.
    # Dark mark on white, which is what that script's alpha replacement assumes.
    icon = render_opaque_square(1024, LIGHT_GLYPH)
    icon.save(ICON_DIR / "icon.png", format="PNG", optimize=True)
    print(f"wrote icon.png              1024x1024 {(ICON_DIR / 'icon.png').stat().st_size:>7} B  (opaque; round_icon.py adds the corners)")

    # icon.ico: the same frame set round_icon.py used.
    ico_sizes = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    frames = [render_tile(s[0], ACID) for s in ico_sizes]
    (ICON_DIR / "icon.ico").write_bytes(build_bmp_ico(frames))
    print(f"wrote icon.ico               sizes={[f.size for f in frames]}")
    check_ico_has_frames(ICON_DIR / "icon.ico", len(ico_sizes))

    # icon.icns: Apple's container needs a 512@2x (1024) and a 256@2x (512)
    # that the old file did not have, so the icon would go blurry in the Dock.
    icns_types = [
        (b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128),
        (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024), (b"ic11", 32),
        (b"ic12", 64), (b"ic13", 256), (b"ic14", 512),
    ]
    chunks = b""
    for kind, size in icns_types:
        buf = io.BytesIO()
        render_tile(size, ACID).save(buf, format="PNG", optimize=True)
        blob = buf.getvalue()
        chunks += kind + struct.pack(">I", len(blob) + 8) + blob
    icns = b"icns" + struct.pack(">I", len(chunks) + 8) + chunks
    (ICON_DIR / "icon.icns").write_bytes(icns)
    print(f"wrote icon.icns              {len(icns)} B, {len(icns_types)} frames")

    mirror_public()
    check_mirrors()

if __name__ == "__main__":
    main()
