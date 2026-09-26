#!/usr/bin/env python3
"""Generate the EXIF orientation fixtures for wmde-viewer's loader tests.

Every image is split in two halves, left and right, stored as the sensor saw it, with EXIF
Orientation 6 ("rotate 90 degrees clockwise to view"). Viewed correctly the left half ends
up on top. The embedded EXIF thumbnail uses other colours than its main image, so a test
can tell which of the two it was handed.
"""
import io
import struct
import sys
from pathlib import Path

from PIL import Image

OUT = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).parent)
RED, BLUE = (255, 0, 0), (0, 0, 255)
GREEN, YELLOW = (0, 255, 0), (255, 255, 0)


def halves(w, h, left, right):
    im = Image.new("RGB", (w, h), right)
    im.paste(left, (0, 0, w // 2, h))
    return im


def exif_orientation_6():
    ex = Image.Exif()
    ex[0x0112] = 6
    return ex


# 1. JPEG, no embedded thumbnail: the turbojpeg full path and the DCT-scaled thumbnail path.
halves(64, 32, RED, BLUE).save(OUT / "orientation_6.jpg", quality=95, exif=exif_orientation_6())

# 2. PNG with an eXIf chunk: the zune path.
halves(64, 32, RED, BLUE).save(OUT / "orientation_6.png", exif=exif_orientation_6())

# 3. JPEG with an IFD1 thumbnail: the embedded-thumbnail path used for camera photos.
thumb = io.BytesIO()
halves(32, 16, GREEN, YELLOW).save(thumb, format="JPEG", quality=95)
thumb = thumb.getvalue()

# Little-endian TIFF: header, IFD0 {Orientation, next -> IFD1}, IFD1 {JPEGInterchangeFormat,
# JPEGInterchangeFormatLength}, then the thumbnail bytes. Offsets count from the TIFF header.
ifd0_off = 8
ifd0 = struct.pack("<H", 1) + struct.pack("<HHIHH", 0x0112, 3, 1, 6, 0)
ifd1_off = ifd0_off + len(ifd0) + 4
ifd1_size = 2 + 2 * 12 + 4
thumb_off = ifd1_off + ifd1_size
ifd0 += struct.pack("<I", ifd1_off)
ifd1 = (
    struct.pack("<H", 2)
    + struct.pack("<HHII", 0x0201, 4, 1, thumb_off)
    + struct.pack("<HHII", 0x0202, 4, 1, len(thumb))
    + struct.pack("<I", 0)
)
tiff = b"II*\x00" + struct.pack("<I", ifd0_off) + ifd0 + ifd1 + thumb
app1 = b"Exif\x00\x00" + tiff
segment = b"\xff\xe1" + struct.pack(">H", len(app1) + 2) + app1

main = io.BytesIO()
halves(256, 128, RED, BLUE).save(main, format="JPEG", quality=95)
main = main.getvalue()
assert main[:2] == b"\xff\xd8"
(OUT / "orientation_6_exif_thumb.jpg").write_bytes(main[:2] + segment + main[2:])

for name in ("orientation_6.jpg", "orientation_6.png", "orientation_6_exif_thumb.jpg"):
    im = Image.open(OUT / name)
    print(name, im.size, "orientation", im.getexif().get(0x0112), (OUT / name).stat().st_size, "bytes")
