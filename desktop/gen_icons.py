#!/usr/bin/env python3
"""Generate tray template icons (36x36, black+alpha) and the 1024px app icon
source for filelink desktop. Pure stdlib: hand-rolled PNG encoder + 4x
supersampled shape rasterizer."""

import math
import struct
import zlib
from pathlib import Path

OUT = Path(__file__).parent / "src-tauri" / "icons"
SS = 4  # supersample factor


# ---------- PNG encoding ----------

def write_png(path: Path, w: int, h: int, rgba):
    def chunk(typ: bytes, data: bytes) -> bytes:
        return (struct.pack(">I", len(data)) + typ + data
                + struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF))

    raw = bytearray()
    for y in range(h):
        raw.append(0)
        for x in range(w):
            r, g, b, a = rgba[x, y]
            raw += bytes((r, g, b, a))
    data = (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
            + chunk(b"IEND", b""))
    path.write_bytes(data)


# ---------- shape rasterization (alpha buffers, supersampled space) ----------

def seg_dist(px, py, ax, ay, bx, by):
    vx, vy = bx - ax, by - ay
    wx, wy = px - ax, py - ay
    L2 = vx * vx + vy * vy
    t = 0.0 if L2 == 0 else max(0.0, min(1.0, (wx * vx + wy * vy) / L2))
    dx, dy = wx - t * vx, wy - t * vy
    return math.hypot(dx, dy)


class Canvas:
    def __init__(self, size):
        self.size = size
        self.a = [0.0] * (size * size)

    def stroke(self, a, b, r):
        s = self.size
        (ax, ay), (bx, by) = a, b
        for y in range(s):
            for x in range(s):
                d = seg_dist(x + 0.5, y + 0.5, ax, ay, bx, by)
                cov = max(0.0, min(1.0, r - d + 0.5))
                if cov > 0:
                    i = y * s + x
                    self.a[i] = min(1.0, max(self.a[i], cov))

    def arc(self, c, radius, thick, a0, a1):
        """Angles in degrees, math convention (CCW from +x), y flipped for screen."""
        s = self.size
        cx, cy = c
        for y in range(s):
            for x in range(s):
                px, py = x + 0.5 - cx, cy - (y + 0.5)
                d = abs(math.hypot(px, py) - radius)
                ang = math.degrees(math.atan2(py, px)) % 360
                lo, hi = a0 % 360, a1 % 360
                inside = (lo <= ang <= hi) if lo <= hi else (ang >= lo or ang <= hi)
                if inside:
                    cov = max(0.0, min(1.0, thick / 2 - d + 0.5))
                    if cov > 0:
                        i = y * s + x
                        self.a[i] = min(1.0, max(self.a[i], cov))

    def cut(self, other):
        for i, v in enumerate(other.a):
            self.a[i] = max(0.0, self.a[i] - v)

    def save(self, path, color=(0, 0, 0)):
        s = self.size // SS
        buf = {}
        for y in range(s):
            for x in range(s):
                acc = 0.0
                for dy in range(SS):
                    for dx in range(SS):
                        acc += self.a[(y * SS + dy) * self.size + x * SS + dx]
                alpha = round(acc / (SS * SS) * 255)
                buf[x, y] = (*color, alpha)
        write_png(path, s, s, buf)


def glyph_idle():
    """Upload glyph: up arrow rising out of a tray."""
    c = Canvas(36 * SS)
    r = 2.0 * SS
    # tray
    c.stroke((7 * SS, 22 * SS), (7 * SS, 27 * SS), r)
    c.stroke((29 * SS, 22 * SS), (29 * SS, 27 * SS), r)
    c.stroke((7 * SS, 27 * SS), (29 * SS, 27 * SS), r)
    # arrow
    c.stroke((18 * SS, 8.5 * SS), (18 * SS, 21 * SS), r)
    c.stroke((12.5 * SS, 13.5 * SS), (18 * SS, 8 * SS), r)
    c.stroke((23.5 * SS, 13.5 * SS), (18 * SS, 8 * SS), r)
    return c


def glyph_spin(frame):
    c = Canvas(36 * SS)
    start = 90 + frame * 45  # rotate clockwise across frames
    c.arc((18 * SS, 17.5 * SS), 10.5 * SS, 3.6 * SS, start, start + 110)
    return c


def glyph_check():
    c = Canvas(36 * SS)
    r = 2.4 * SS
    c.stroke((8.5 * SS, 18 * SS), (15 * SS, 24.5 * SS), r)
    c.stroke((15 * SS, 24.5 * SS), (27.5 * SS, 11 * SS), r)
    return c


def glyph_warn():
    """Filled rounded triangle with an exclamation cut out."""
    c = Canvas(36 * SS)
    s = SS
    # filled triangle via many strokes between edge points (round joins)
    A, B, C = (18, 7.5), (29.5, 27.5), (6.5, 27.5)
    edges = [(A, B), (B, C), (C, A)]
    for (p, q) in edges:
        steps = 40
        for i in range(steps):
            t0, t1 = i / steps, (i + 1) / steps
            c.stroke((p[0] + (q[0] - p[0]) * t0, p[1] + (q[1] - p[1]) * t0),
                     (p[0] + (q[0] - p[0]) * t1, p[1] + (q[1] - p[1]) * t1), 1.8 * s)
    # cut exclamation: bar + dot
    cut = Canvas(36 * SS)
    cut.stroke((18 * s, 13.5 * s), (18 * s, 21 * s), 1.7 * s)
    cut.stroke((18 * s, 24.2 * s), (18 * s, 24.3 * s), 1.9 * s)
    c.cut(cut)
    return c


def app_icon():
    """1024 rounded-rect gradient + white upload glyph, macOS Big Sur layout."""
    S = 1024
    buf = {}
    m, rr = 100, 185  # margin and corner radius
    gx0, gy0 = (0.30, 0.36, 0.94), (0.16, 0.22, 0.72)  # indigo gradient
    for y in range(S):
        t = y / S
        gr = tuple(gx0[k] + (gy0[k] - gx0[k]) * t for k in range(3))
        for x in range(S):
            inside = (m + rr <= x or x <= S - m - rr or True) and (m <= x < S - m and m <= y < S - m)
            if not inside:
                buf[x, y] = (0, 0, 0, 0)
                continue
            # rounded corner mask
            cx = min(max(x, m + rr), S - m - rr)
            cy = min(max(y, m + rr), S - m - rr)
            if math.hypot(x - cx, y - cy) > rr and (cx != x or cy != y):
                buf[x, y] = (0, 0, 0, 0)
                continue
            buf[x, y] = (round(gr[0] * 255), round(gr[1] * 255), round(gr[2] * 255), 255)
    # white glyph: reuse tray-glyph geometry scaled x24 from the 36-unit design
    scale, off = 24, 256 + 80
    g = Canvas(36 * SS)
    r = 2.0 * g.size / 36
    g.stroke((7, 22), (7, 27), r)
    g.stroke((29, 22), (29, 27), r)
    g.stroke((7, 27), (29, 27), r)
    g.stroke((18, 8.5), (18, 21), r)
    g.stroke((12.5, 13.5), (18, 8), r)
    g.stroke((23.5, 13.5), (18, 8), r)
    for y in range(g.size):
        for x in range(g.size):
            a = g.a[y * g.size + x]
            if a <= 0.01:
                continue
            dx = round(x / SS * scale) + off
            dy = round(y / SS * scale) + off
            if 0 <= dx < S and 0 <= dy < S:
                pr, pg, pb, pa = buf[dx, dy]
                na = max(pa / 255, min(1.0, a))
                buf[dx, dy] = (255, 255, 255, round(na * 255))
    write_png(OUT / "appicon.png", S, S, buf)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    glyph_idle().save(OUT / "tray-idle.png")
    for f in range(8):
        glyph_spin(f).save(OUT / f"tray-spin-{f}.png")
    glyph_check().save(OUT / "tray-check.png")
    glyph_warn().save(OUT / "tray-warn.png")
    app_icon()
    print("icons written to", OUT)


if __name__ == "__main__":
    main()
