"""Package already resized PNGs into an ICO container; no image processing."""
import pathlib
import struct
import sys

source, target = map(pathlib.Path, sys.argv[1:])
sizes = (16, 24, 32, 48, 64, 128, 256)
images = [(size, (source / f"{size}.png").read_bytes()) for size in sizes]
offset = 6 + 16 * len(images)
entries = []
for size, data in images:
    entries.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
    offset += len(data)
target.write_bytes(struct.pack("<HHH", 0, 1, len(images)) + b"".join(entries) + b"".join(data for _, data in images))
