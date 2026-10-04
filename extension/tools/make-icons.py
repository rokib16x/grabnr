#!/usr/bin/env python3
"""Generate icons/icon{16,32,48,128}.png: blue rounded square, white down arrow.
Pure stdlib (struct + zlib). Supersampled for anti-aliasing."""
import os, struct, zlib

BLUE = (0x25, 0x63, 0xEB)
WHITE = (255, 255, 255)
SS = 4  # supersampling factor


def in_rounded_square(x, y, r):  # unit square [0,1], corner radius r
    cx = min(max(x, r), 1 - r)
    cy = min(max(y, r), 1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def in_arrow(x, y):
    # shaft
    if 0.42 <= x <= 0.58 and 0.20 <= y <= 0.55:
        return True
    # head: triangle pointing down, apex (0.5, 0.78), base y=0.50 from x 0.24..0.76
    if 0.50 <= y <= 0.78:
        half = 0.26 * (0.78 - y) / 0.28
        return abs(x - 0.5) <= half
    return False


def render(size):
    rows = []
    for py in range(size):
        row = bytearray([0])  # filter type 0
        for px in range(size):
            acc = [0, 0, 0, 0]
            for sy in range(SS):
                for sx in range(SS):
                    x = (px + (sx + 0.5) / SS) / size
                    y = (py + (sy + 0.5) / SS) / size
                    if not in_rounded_square(x, y, 0.22):
                        continue
                    c = WHITE if in_arrow(x, y) else BLUE
                    acc[0] += c[0]; acc[1] += c[1]; acc[2] += c[2]; acc[3] += 255
            n = SS * SS
            a = acc[3] // n
            if acc[3]:
                row += bytes((acc[0] * 255 // acc[3], acc[1] * 255 // acc[3], acc[2] * 255 // acc[3], a))
            else:
                row += bytes((0, 0, 0, 0))
        rows.append(bytes(row))
    return b''.join(rows)


def chunk(tag, data):
    body = tag + data
    return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body) & 0xFFFFFFFF)


def png(size):
    ihdr = struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', ihdr)
            + chunk(b'IDAT', zlib.compress(render(size), 9)) + chunk(b'IEND', b''))


if __name__ == '__main__':
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'icons')
    os.makedirs(out, exist_ok=True)
    for s in (16, 32, 48, 128):
        with open(os.path.join(out, f'icon{s}.png'), 'wb') as f:
            f.write(png(s))
        print('wrote', f'icon{s}.png')
