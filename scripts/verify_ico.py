"""Independent check of an exported .ico, using Pillow rather than our own parser.

usage: python -I scripts/verify_ico.py out.ico --expect 16,32,256 [--opaque-corners]

Reads the raw ICONDIR (so we see exactly what was written, not what Pillow picks), decodes
every entry, and asserts: exact size set, 32 bpp, PNG payload, corner alpha 0, centre alpha 255.
"""
import io
import struct
import sys

from PIL import Image


def main() -> int:
    args = sys.argv[1:]
    path = args[0]
    expect = sorted(int(v) for v in args[args.index("--expect") + 1].split(","))
    opaque_corners = "--opaque-corners" in args  # shape "none"

    data = open(path, "rb").read()
    reserved, kind, count = struct.unpack_from("<HHH", data, 0)
    assert (reserved, kind) == (0, 1), "not an icon file"

    sizes, ok = [], True
    for i in range(count):
        w, h, _colors, _res, planes, bpp, size, off = struct.unpack_from("<BBBBHHII", data, 6 + i * 16)
        w, h = w or 256, h or 256
        blob = data[off : off + size]
        is_png = blob.startswith(b"\x89PNG\r\n\x1a\n")
        img = Image.open(io.BytesIO(blob)).convert("RGBA")
        n = img.width
        corners = [img.getpixel(p)[3] for p in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)]]
        centre = img.getpixel((n // 2, n // 2))[3]
        want_corner = 255 if opaque_corners else 0
        good = is_png and bpp == 32 and planes == 1 and img.size == (w, h) and all(
            c == want_corner for c in corners) and centre == 255
        ok &= good
        sizes.append(w)
        print(f"  {w:>3}x{h:<3} bpp={bpp} png={is_png} corners_alpha={corners} centre_alpha={centre} {'OK' if good else 'FAIL'}")

    sizes_ok = sorted(sizes) == expect
    print(f"  sizes={sorted(sizes)} expected={expect} {'OK' if sizes_ok else 'FAIL'}")
    return 0 if ok and sizes_ok else 1


if __name__ == "__main__":
    sys.exit(main())
