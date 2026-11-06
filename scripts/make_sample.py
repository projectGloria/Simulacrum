"""Generate a non-square, photo-ish test image (and the app-icon artwork) with Pillow.

usage: python -I scripts/make_sample.py out.png [width height]
"""
import sys

from PIL import Image, ImageDraw


def main() -> None:
    out = sys.argv[1]
    w, h = (int(sys.argv[2]), int(sys.argv[3])) if len(sys.argv) > 3 else (1600, 1200)
    img = Image.new("RGB", (w, h))
    px = img.load()
    for y in range(h):
        for x in range(w):
            # Diagonal indigo → teal gradient: smooth content makes resampling artefacts obvious.
            t = (x / w + y / h) / 2
            px[x, y] = (int(40 + 60 * t), int(30 + 170 * t), int(140 + 90 * (1 - t)))
    d = ImageDraw.Draw(img)
    s = min(w, h)
    cx, cy = w // 2, h // 2
    # A bold glyph-like mark centred in the square crop.
    d.ellipse([cx - s * 0.28, cy - s * 0.28, cx + s * 0.28, cy + s * 0.28], outline=(255, 255, 255), width=int(s * 0.06))
    d.rectangle([cx - s * 0.04, cy - s * 0.34, cx + s * 0.04, cy + s * 0.34], fill=(255, 214, 102))
    # Content at the far left/right is outside the centre square crop and must disappear.
    d.rectangle([0, 0, w * 0.08, h], fill=(255, 0, 0))
    d.rectangle([w * 0.92, 0, w, h], fill=(255, 0, 0))
    img.save(out)


if __name__ == "__main__":
    main()
