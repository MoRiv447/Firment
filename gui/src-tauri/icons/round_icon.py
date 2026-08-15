"""Generate a rounded-corner icon.png + a multi-frame PNG ICO.

Input:  gui/src-tauri/icons/icon.png    (any size, square)
Output: gui/src-tauri/icons/icon.png    (1024x1024, rounded, transparent corners)
        gui/src-tauri/icons/icon.ico    (7 frames at 16/24/32/48/64/128/256,
                                         each PNG-encoded so Vista+ reads the
                                         alpha channel for round corners)

Pillow's ICO writer only emits one frame, so we assemble the multi-frame ICO
ourselves: ICO header (6B) + N directory entries (16B each) + N PNG blobs.

Corner radius: 22.37% of the edge (iOS / macOS app-icon standard).
"""
from __future__ import annotations

import io
import struct
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ICON_DIR = Path(r"D:\OldStudy66\Firment\gui\src-tauri\icons")
SRC = ICON_DIR / "icon.png"
DST_PNG = ICON_DIR / "icon.png"
DST_ICO = ICON_DIR / "icon.ico"
SIZE = 1024
RADIUS = 0.2237  # iOS app-icon standard (~22%)
ICO_SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]


def rounded_alpha_mask(size: int, radius: float) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [(0, 0), (size - 1, size - 1)],
        radius=int(size * radius),
        fill=255,
    )
    return mask


def apply_rounded_alpha(img: Image.Image) -> Image.Image:
    """Replace the alpha channel with a rounded-square mask.

    The source icon.png is opaque (white background, black logo), so the
    original alpha is 255 everywhere — we just want corners to go
    transparent. Replacing the alpha wholesale with the rounded mask is
    simpler and correct in this case.
    """
    mask = rounded_alpha_mask(img.size[0], RADIUS)
    img.putalpha(mask)
    return img


def png_bytes(img: Image.Image) -> bytes:
    buf = io.BytesIO()
    img.save(buf, format="PNG", optimize=True)
    return buf.getvalue()


def build_multi_frame_ico(frames: list[Image.Image]) -> bytes:
    """Assemble an ICO with each frame encoded as a PNG blob."""
    blobs: list[tuple[int, int, bytes]] = []
    for im in frames:
        blobs.append((im.width, im.height, png_bytes(im)))

    out = io.BytesIO()
    out.write(struct.pack("<HHH", 0, 1, len(blobs)))  # reserved, type=icon, count

    data_offset = 6 + 16 * len(blobs)
    entries: list[tuple[int, int, int]] = []
    for w, h, blob in blobs:
        # width/height of 0 means 256 in ICO format
        ew = 0 if w >= 256 else w
        eh = 0 if h >= 256 else h
        entries.append((ew, eh, len(blob), data_offset))
        data_offset += len(blob)

    for ew, eh, size, off in entries:
        out.write(struct.pack("<BBBBHHII", ew, eh, 0, 0, 1, 32, size, off))

    for _, _, blob in blobs:
        out.write(blob)
    return out.getvalue()


def main() -> None:
    if not SRC.exists():
        sys.exit(f"missing: {SRC}")

    img = Image.open(SRC).convert("RGBA")
    if img.size != (SIZE, SIZE):
        img = img.resize((SIZE, SIZE), Image.LANCZOS)
    img = apply_rounded_alpha(img)

    img.save(DST_PNG, format="PNG", optimize=True)
    print(f"wrote {DST_PNG}  ({DST_PNG.stat().st_size} B)")

    ico_frames = [apply_rounded_alpha(Image.open(SRC).convert("RGBA").resize(s, Image.LANCZOS)) for s in ICO_SIZES]
    # Use the high-res rounded img as the 256 frame
    ico_frames[-1] = img.resize((256, 256), Image.LANCZOS).copy()

    DST_ICO.write_bytes(build_multi_frame_ico(ico_frames))
    print(f"wrote {DST_ICO}  ({DST_ICO.stat().st_size} B)  sizes={[im.size for im in ico_frames]}")

    # The tauri bundle icon list also includes 32x32.png / 128x128.png /
    # 128x128@2x.png — these are embedded in the exe resources / shortcuts,
    # so they must be rounded too or the shortcut icon stays square.
    for name, size in [("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256)]:
        out = apply_rounded_alpha(img.resize((size, size), Image.LANCZOS))
        out.save(ICON_DIR / name, format="PNG", optimize=True)
        print(f"wrote {ICON_DIR / name}  ({(ICON_DIR / name).stat().st_size} B)")


if __name__ == "__main__":
    main()