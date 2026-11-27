#!/usr/bin/env python3
"""Generate crisp optical-sized Windows ICO + 32x32 PNG icons without touching macOS .icns.

Outputs strictly to:
  - src-tauri/icons/icon.ico
  - packaging/app_icon.ico
  - src-tauri/icons/32x32.png

Never modifies src-tauri/icons/icon.icns, icon.png, 128x128.png, or 128x128@2x.png.
"""

from __future__ import annotations

import argparse
import math
import struct
import zlib
from pathlib import Path
from typing import Iterable, Sequence

REPO_ROOT = Path(__file__).resolve().parents[1]

WINDOWS_ICO_SIZES: tuple[int, ...] = (16, 20, 24, 32, 40, 48, 64, 96, 128, 256)
DIB_ICO_MAX_SIZE = 128

# Canonical C-2 mark geometry inside a square viewBox (-1, -8.5, 117, 117)
VB_X = -1.0
VB_Y = -8.5
VB_SIZE = 117.0

# Colors (sRGB)
MARK_RGB = (0x14, 0x14, 0x13)
PLATE_RGB = (0xFF, 0xFF, 0xFF)
BORDER_RGB = (0xD4, 0xD4, 0xD8)

# 11 full blades from ui/public/app-symbol.svg (for >= 64px)
FULL_BLADES: tuple[tuple[tuple[float, float], ...], ...] = (
    ((41.00, 20.00), (52.06, 18.90), (75.0, 18.90), (75.0, 21.10), (52.06, 21.10)),
    ((50.00, 26.00), (58.50, 24.95), (75.0, 24.95), (75.0, 27.05), (58.50, 27.05)),
    ((58.00, 32.00), (63.78, 31.00), (75.0, 31.00), (75.0, 33.00), (63.78, 33.00)),
    ((45.00, 38.00), (55.20, 36.85), (75.0, 36.85), (75.0, 39.15), (55.20, 39.15)),
    ((23.00, 44.00), (35.79, 42.70), (75.0, 42.70), (75.0, 45.30), (35.79, 45.30)),
    ((1.00, 50.00), (15.90, 48.40), (75.0, 48.40), (75.0, 51.60), (15.90, 51.60)),
    ((23.00, 56.00), (35.79, 54.70), (75.0, 54.70), (75.0, 57.30), (35.79, 57.30)),
    ((45.00, 62.00), (55.20, 60.85), (75.0, 60.85), (75.0, 63.15), (55.20, 63.15)),
    ((58.00, 68.00), (63.78, 67.00), (75.0, 67.00), (75.0, 69.00), (63.78, 69.00)),
    ((50.00, 74.00), (58.50, 72.95), (75.0, 72.95), (75.0, 75.05), (58.50, 75.05)),
    ((41.00, 80.00), (52.06, 78.90), (75.0, 78.90), (75.0, 81.10), (52.06, 81.10)),
)


def _cubic(
    p0: tuple[float, float],
    p1: tuple[float, float],
    p2: tuple[float, float],
    p3: tuple[float, float],
    steps: int = 40,
) -> list[tuple[float, float]]:
    pts: list[tuple[float, float]] = []
    for i in range(1, steps + 1):
        t = i / steps
        mt = 1.0 - t
        x = (
            mt**3 * p0[0]
            + 3 * mt**2 * t * p1[0]
            + 3 * mt * t**2 * p2[0]
            + t**3 * p3[0]
        )
        y = (
            mt**3 * p0[1]
            + 3 * mt**2 * t * p1[1]
            + 3 * mt * t**2 * p2[1]
            + t**3 * p3[1]
        )
        pts.append((x, y))
    return pts


def _build_orbit_polygon() -> list[tuple[float, float]]:
    pts: list[tuple[float, float]] = [(59.0, 5.0)]
    pts.extend(_cubic((59.0, 5.0), (28.5, 32.5), (10.5, 70.0), (18.2, 84.8)))
    pts.extend(_cubic((18.2, 84.8), (25.2, 96.8), (54.5, 82.0), (77.8, 51.2)))
    pts.append((74.4, 47.8))
    pts.extend(_cubic((74.4, 47.8), (53.2, 75.2), (27.2, 87.2), (22.5, 79.0)))
    pts.extend(_cubic((22.5, 79.0), (17.5, 70.0), (33.0, 34.0), (59.0, 5.0)))
    return pts


def _build_star_polygon(size: int) -> list[tuple[float, float]]:
    # At tiny sizes (<=48px), slightly broaden the vertical/left waist of the star
    # so anti-aliasing keeps a solid dark core instead of hairline gaps.
    waist_dx = 1.6 if size <= 24 else (0.9 if size <= 48 else 0.0)
    left_x = 63.0 - waist_dx * 1.4
    pts: list[tuple[float, float]] = [(75.0, 4.0)]
    pts.extend(_cubic((75.0, 4.0), (76.4 + waist_dx, 30.0), (84.5, 46.8 - waist_dx), (114.0, 50.0)))
    pts.extend(_cubic((114.0, 50.0), (84.5, 53.2 + waist_dx), (76.4 + waist_dx, 70.0), (75.0, 96.0)))
    pts.extend(_cubic((75.0, 96.0), (73.6 - waist_dx, 70.0), (69.2 - waist_dx, 54.5 + waist_dx), (left_x, 50.0)))
    pts.extend(_cubic((left_x, 50.0), (69.2 - waist_dx, 45.5 - waist_dx), (73.6 - waist_dx, 30.0), (75.0, 4.0)))
    return pts


def _offset_polygon(poly: Sequence[tuple[float, float]], expand_vb: float) -> list[tuple[float, float]]:
    """Expand a closed polygon outward along its vertex normals by `expand_vb`."""
    if expand_vb <= 0.0:
        return list(poly)
    n = len(poly)
    # Compute signed area to orient outward normal cleanly
    area2 = sum(
        poly[i][0] * poly[(i + 1) % n][1] - poly[(i + 1) % n][0] * poly[i][1]
        for i in range(n)
    )
    sign = 1.0 if area2 >= 0 else -1.0
    out: list[tuple[float, float]] = []
    for i in range(n):
        p_prev = poly[(i - 1) % n]
        p_curr = poly[i]
        p_next = poly[(i + 1) % n]
        dx1, dy1 = p_curr[0] - p_prev[0], p_curr[1] - p_prev[1]
        dx2, dy2 = p_next[0] - p_curr[0], p_next[1] - p_curr[1]
        l1 = math.hypot(dx1, dy1) or 1.0
        l2 = math.hypot(dx2, dy2) or 1.0
        # Outward unit normals in SVG (y down): clockwise polygon has area2 < 0
        nx1, ny1 = sign * (dy1 / l1), sign * (-dx1 / l1)
        nx2, ny2 = sign * (dy2 / l2), sign * (-dx2 / l2)
        nx, ny = nx1 + nx2, ny1 + ny2
        nl = math.hypot(nx, ny)
        if nl < 1e-6:
            nx, ny = nx1, ny1
        else:
            nx, ny = nx / nl, ny / nl
        out.append((p_curr[0] + nx * expand_vb, p_curr[1] + ny * expand_vb))
    return out


def _scanline_coverage(
    polygons: Sequence[Sequence[tuple[float, float]]],
    width: int,
    height: int,
    ss: int = 8,
) -> list[list[float]]:
    """Compute exact [0, 1] pixel coverage via scanline edge intersections at `ss` subpixel rows."""
    cov = [[0.0] * width for _ in range(height)]
    # Precompute bounding boxes of polygons in pixel coordinates
    poly_edges: list[tuple[float, float, list[tuple[float, float, float, float]]]] = []
    for poly in polygons:
        if len(poly) < 3:
            continue
        ys = [p[1] for p in poly]
        min_y, max_y = min(ys), max(ys)
        edges: list[tuple[float, float, float, float]] = []
        n = len(poly)
        for i in range(n):
            x0, y0 = poly[i]
            x1, y1 = poly[(i + 1) % n]
            if abs(y1 - y0) < 1e-9:
                continue
            if y0 < y1:
                edges.append((y0, y1, x0, (x1 - x0) / (y1 - y0)))
            else:
                edges.append((y1, y0, x1, (x0 - x1) / (y0 - y1)))
        poly_edges.append((min_y, max_y, edges))

    inv_ss = 1.0 / ss
    for py in range(height):
        # Quick skip if row is completely outside all polygons
        if not any(py + 1.0 >= min_y and py <= max_y for min_y, max_y, _ in poly_edges):
            continue
        row_sums = [0.0] * width
        for sy in range(ss):
            y = py + (sy + 0.5) * inv_ss
            intervals: list[tuple[float, float]] = []
            for min_y, max_y, edges in poly_edges:
                if y < min_y or y > max_y:
                    continue
                xs: list[float] = []
                for y0, y1, x0, slope in edges:
                    if y0 <= y < y1:
                        xs.append(x0 + (y - y0) * slope)
                if len(xs) >= 2:
                    xs.sort()
                    for k in range(0, len(xs) - 1, 2):
                        a, b = xs[k], xs[k + 1]
                        if b > a:
                            intervals.append((a, b))
            if not intervals:
                continue
            # Merge overlapping intervals across polygons (union)
            intervals.sort()
            merged: list[tuple[float, float]] = [intervals[0]]
            for a, b in intervals[1:]:
                prev_a, prev_b = merged[-1]
                if a <= prev_b:
                    merged[-1] = (prev_a, max(prev_b, b))
                else:
                    merged.append((a, b))
            for a, b in merged:
                x_start = max(0, int(math.floor(a)))
                x_end = min(width - 1, int(math.floor(b)))
                for px in range(x_start, x_end + 1):
                    left = max(a, float(px))
                    right = min(b, float(px + 1))
                    if right > left:
                        row_sums[px] += right - left
        cov_row = cov[py]
        for px in range(width):
            val = row_sums[px] * inv_ss
            cov_row[px] = 1.0 if val > 1.0 else val
    return cov


def _pixel_snapped_micro_blades(
    size: int,
    vb_to_px,
    px_to_vb,
) -> list[list[tuple[float, float]]]:
    """Generate pixel-row-snapped horizontal radar blades for <= 48px optical sizing."""
    star_vb_x = 75.0
    if size <= 20:
        # 3 crisp pixel-aligned bars with pure-white air gaps
        spec = (
            (31.0, 41.0, 29.0),
            (50.0, 4.0, 50.0),
            (69.0, 41.0, 71.0),
        )
        bar_h_px = 1 if size == 16 else 2
        if size == 16:
            # 16px: pick explicit distinct pixel rows centered around 50
            rows = (
                (5, 1, 40.0),
                (7, 2, 4.0),
                (10, 1, 40.0),
            )
            polys: list[list[tuple[float, float]]] = []
            for y_px, h_px, tip_vb_x in rows:
                x_tip_px, _ = vb_to_px(tip_vb_x, 50.0)
                x_end_px, _ = vb_to_px(star_vb_x, 50.0)
                polys.append([
                    (x_tip_px, y_px + h_px * 0.5),
                    (x_tip_px + h_px * 1.4, float(y_px)),
                    (x_end_px, float(y_px)),
                    (x_end_px, float(y_px + h_px)),
                    (x_tip_px + h_px * 1.4, float(y_px + h_px)),
                ])
            return polys
        else:
            # 20px: 3 crisp 2px bars with clear 2px white gaps
            rows = (
                (5, 2, 42.0),
                (9, 2, 4.0),
                (13, 2, 42.0),
            )
            polys = []
            for y_px, h_px, tip_vb_x in rows:
                x_tip_px, _ = vb_to_px(tip_vb_x, 50.0)
                x_end_px, _ = vb_to_px(star_vb_x, 50.0)
                polys.append([
                    (x_tip_px, y_px + h_px * 0.5),
                    (x_tip_px + h_px * 1.3, float(y_px)),
                    (x_end_px, float(y_px)),
                    (x_end_px, float(y_px + h_px)),
                    (x_tip_px + h_px * 1.3, float(y_px + h_px)),
                ])
            return polys

    # 24px, 32px, 40px, 48px: 5 pixel-snapped horizontal bars (distinct optical meteor tail)
    vb_ys_and_tips = (
        (24.0, 44.0),
        (37.0, 29.0),
        (50.0, 3.0),
        (63.0, 29.0),
        (76.0, 44.0),
    )
    h_px = 1 if size == 24 else (2 if size <= 40 else 2)
    polys = []
    used_rows: set[int] = set()
    for idx, (cy_vb, tip_vb_x) in enumerate(vb_ys_and_tips):
        x_tip_px, cy_px = vb_to_px(tip_vb_x, cy_vb)
        x_end_px, _ = vb_to_px(star_vb_x, cy_vb)
        # Center bar gets 1px extra height on 24/48px for focal hierarchy
        cur_h = 2 if (size == 24 and idx == 2) else (3 if (size >= 40 and idx == 2) else h_px)
        y_top = int(round(cy_px - cur_h * 0.5))
        # Ensure minimum 1px white air gap from previous bar
        while any(r in used_rows for r in range(y_top - 1, y_top + cur_h + 1)):
            y_top += 1
        for r in range(y_top, y_top + cur_h):
            used_rows.add(r)
        taper = max(1.5, cur_h * 1.6)
        polys.append([
            (x_tip_px, y_top + cur_h * 0.5),
            (x_tip_px + taper, float(y_top)),
            (x_end_px, float(y_top)),
            (x_end_px, float(y_top + cur_h)),
            (x_tip_px + taper, float(y_top + cur_h)),
        ])
    return polys


def _rounded_rect_sdf(px: float, py: float, x0: float, y0: float, w: float, h: float, r: float) -> float:
    cx = x0 + w * 0.5
    cy = y0 + h * 0.5
    hx = w * 0.5 - r
    hy = h * 0.5 - r
    dx = max(abs(px - cx) - hx, 0.0)
    dy = max(abs(py - cy) - hy, 0.0)
    return math.hypot(dx, dy) - r


def render_windows_icon(size: int) -> list[int]:
    """Render an RGBA pixel buffer (length = size*size*4) for the given Windows icon size."""
    # Full-bleed Windows rounded plate (88-94% active canvas area, zero macOS Dock outer margin)
    pad = 0.0 if size <= 24 else (1.0 if size <= 48 else round(size * 0.03))
    plate_w = size - 2.0 * pad
    rx = max(3.0, round(plate_w * 0.20))

    # Map C-2 viewBox cleanly onto the full-bleed plate
    margin_ratio = 0.055 if size <= 48 else 0.075
    inner_x0 = pad + plate_w * margin_ratio
    inner_w = plate_w * (1.0 - 2.0 * margin_ratio)
    scale = inner_w / VB_SIZE

    def vb_to_px(vx: float, vy: float) -> tuple[float, float]:
        return (inner_x0 + (vx - VB_X) * scale, inner_x0 + (vy - VB_Y) * scale)

    def px_to_vb(px: float, py: float) -> tuple[float, float]:
        return ((px - inner_x0) / scale + VB_X, (py - inner_x0) / scale + VB_Y)

    # Vector shapes transformed to pixel space
    orbit_vb = _build_orbit_polygon()
    if size <= 20:
        orbit_vb = _offset_polygon(orbit_vb, 1.45)
    elif size <= 48:
        orbit_vb = _offset_polygon(orbit_vb, 1.05)

    star_vb = _build_star_polygon(size)
    polys_px: list[list[tuple[float, float]]] = [
        [vb_to_px(x, y) for x, y in orbit_vb],
        [vb_to_px(x, y) for x, y in star_vb],
    ]

    if size <= 48:
        polys_px.extend(_pixel_snapped_micro_blades(size, vb_to_px, px_to_vb))
    else:
        for blade in FULL_BLADES:
            polys_px.append([vb_to_px(x, y) for x, y in blade])

    mark_cov = _scanline_coverage(polys_px, size, size, ss=8)

    # Composite crisp white rounded-square plate + subtle 1px border + dark C-2 mark
    border_w = 1.0 if size >= 24 else 0.75
    rgba = bytearray(size * size * 4)
    ss_plate = 4
    inv_ss2 = 1.0 / (ss_plate * ss_plate)
    for py in range(size):
        for px in range(size):
            # Supersample rounded plate alpha & border mix
            plate_alpha_acc = 0.0
            border_acc = 0.0
            for sy in range(ss_plate):
                sy_f = py + (sy + 0.5) / ss_plate
                for sx in range(ss_plate):
                    sx_f = px + (sx + 0.5) / ss_plate
                    d = _rounded_rect_sdf(sx_f, sy_f, pad, pad, plate_w, plate_w, rx)
                    if d <= 0.0:
                        plate_alpha_acc += 1.0
                        if d > -border_w:
                            border_acc += 1.0
            plate_a = plate_alpha_acc * inv_ss2
            border_f = border_acc * inv_ss2

            # Mix plate white with border gray (18% opacity border on white base)
            base_r = PLATE_RGB[0] * (1.0 - border_f * 0.55) + BORDER_RGB[0] * (border_f * 0.55)
            base_g = PLATE_RGB[1] * (1.0 - border_f * 0.55) + BORDER_RGB[1] * (border_f * 0.55)
            base_b = PLATE_RGB[2] * (1.0 - border_f * 0.55) + BORDER_RGB[2] * (border_f * 0.55)

            m = mark_cov[py][px]
            # Clip mark cleanly inside plate
            m_eff = min(1.0, m)
            r = int(round(base_r * (1.0 - m_eff) + MARK_RGB[0] * m_eff))
            g = int(round(base_g * (1.0 - m_eff) + MARK_RGB[1] * m_eff))
            b = int(round(base_b * (1.0 - m_eff) + MARK_RGB[2] * m_eff))
            a = int(round(plate_a * 255.0))

            idx = (py * size + px) * 4
            rgba[idx] = max(0, min(255, r))
            rgba[idx + 1] = max(0, min(255, g))
            rgba[idx + 2] = max(0, min(255, b))
            rgba[idx + 3] = max(0, min(255, a))

    return list(rgba)


def encode_png(width: int, height: int, rgba: Sequence[int]) -> bytes:
    """Encode an RGBA pixel sequence into a valid RGBA PNG."""
    raw_rows = bytearray()
    stride = width * 4
    for y in range(height):
        raw_rows.append(0)  # Filter type 0 (None)
        row_start = y * stride
        raw_rows.extend(rgba[row_start : row_start + stride])

    def chunk(tag: bytes, payload: bytes) -> bytes:
        crc = zlib.crc32(tag + payload) & 0xFFFFFFFF
        return struct.pack(">I", len(payload)) + tag + payload + struct.pack(">I", crc)

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    idat = zlib.compress(bytes(raw_rows), level=9)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", idat)
        + chunk(b"IEND", b"")
    )


def encode_bmp_dib_32bpp(width: int, height: int, rgba: Sequence[int]) -> bytes:
    """Encode an ICO-embedded 32-bit BGRA BITMAPINFOHEADER + XOR + AND mask."""
    header = struct.pack(
        "<IiiHHIIiiII",
        40,  # biSize
        width,
        height * 2,  # biHeight includes XOR + AND mask
        1,  # biPlanes
        32,  # biBitCount
        0,  # BI_RGB uncompressed
        0,  # biSizeImage
        0,
        0,
        0,
        0,
    )
    xor_mask = bytearray()
    and_stride = ((width + 31) // 32) * 4
    and_mask = bytearray()
    for y in range(height - 1, -1, -1):
        and_row = bytearray(and_stride)
        for x in range(width):
            idx = (y * width + x) * 4
            r, g, b, a = rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]
            xor_mask.extend((b, g, r, a))
            if a == 0:
                and_row[x // 8] |= 1 << (7 - (x % 8))
        and_mask.extend(and_row)
    return header + bytes(xor_mask) + bytes(and_mask)


def build_ico(frames: Sequence[tuple[int, Sequence[int]]]) -> bytes:
    """Build multi-resolution Windows ICO (16..128px 32bpp BMP DIB, 256px PNG)."""
    entries: list[tuple[int, bytes]] = []
    for size, rgba in frames:
        if size <= DIB_ICO_MAX_SIZE:
            payload = encode_bmp_dib_32bpp(size, size, rgba)
        else:
            payload = encode_png(size, size, rgba)
        entries.append((size, payload))

    header = struct.pack("<HHH", 0, 1, len(entries))
    dir_bytes = bytearray()
    offset = 6 + len(entries) * 16
    for size, payload in entries:
        w_byte = 0 if size >= 256 else size
        h_byte = 0 if size >= 256 else size
        dir_bytes.extend(
            struct.pack(
                "<BBBBHHII",
                w_byte,
                h_byte,
                0,
                0,
                1,
                32,
                len(payload),
                offset,
            )
        )
        offset += len(payload)

    return header + bytes(dir_bytes) + b"".join(payload for _, payload in entries)


def render_contact_sheet(
    frames: Sequence[tuple[int, Sequence[int]]],
    out_path: Path,
) -> None:
    """Render 1x and 4x zoomed icon previews on Windows Light (#f3f3f3) & Dark (#1c1c1c) taskbars."""
    # Canvas layout: top row light (#f3f3f3), bottom row dark (#1c1c1c)
    cols_w = sum(max(32, s) + 12 for s, _ in frames) + 40
    scale_sizes = [s for s, _ in frames if s <= 64]
    zoom_w = sum(s * 4 + 12 for s in scale_sizes) + 40
    width = max(cols_w, zoom_w)
    row_h = 500
    height = row_h * 2
    canvas = bytearray(width * height * 4)

    def fill_rect(x0: int, y0: int, rw: int, rh: int, rgb: tuple[int, int, int]) -> None:
        r, g, b = rgb
        for y in range(max(0, y0), min(height, y0 + rh)):
            for x in range(max(0, x0), min(width, x0 + rw)):
                i = (y * width + x) * 4
                canvas[i], canvas[i + 1], canvas[i + 2], canvas[i + 3] = r, g, b, 255

    def blit_scaled(src_s: int, rgba: Sequence[int], dst_x: int, dst_y: int, factor: int) -> None:
        for sy in range(src_s):
            for sx in range(src_s):
                si = (sy * src_s + sx) * 4
                sr, sg, sb, sa = rgba[si], rgba[si + 1], rgba[si + 2], rgba[si + 3]
                if sa == 0:
                    continue
                af = sa / 255.0
                for dy in range(factor):
                    py = dst_y + sy * factor + dy
                    if not (0 <= py < height):
                        continue
                    for dx in range(factor):
                        px = dst_x + sx * factor + dx
                        if not (0 <= px < width):
                            continue
                        di = (py * width + px) * 4
                        canvas[di] = int(canvas[di] * (1.0 - af) + sr * af)
                        canvas[di + 1] = int(canvas[di + 1] * (1.0 - af) + sg * af)
                        canvas[di + 2] = int(canvas[di + 2] * (1.0 - af) + sb * af)

    fill_rect(0, 0, width, row_h, (0xF3, 0xF3, 0xF3))
    fill_rect(0, row_h, width, row_h, (0x1C, 0x1C, 0x1C))

    for theme_idx in range(2):
        base_y = theme_idx * row_h
        # Row A: 1x native sizes
        cx = 20
        for s, rgba in frames:
            blit_scaled(s, rgba, cx, base_y + 20, 1)
            cx += s + 12
        # Row B: 4x zoomed small/mid sizes (16..48) placed cleanly below 256px icon
        cx = 20
        for s, rgba in frames:
            if s <= 48:
                blit_scaled(s, rgba, cx, base_y + 290, 4)
                cx += s * 4 + 12

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_bytes(encode_png(width, height, canvas))


def main(argv: Iterable[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Generate crisp Windows ICO + 32x32 PNG icons.")
    parser.add_argument(
        "--preview",
        type=Path,
        default=None,
        help="Optional output PNG path for light/dark taskbar visual preview sheet.",
    )
    args = parser.parse_args(list(argv) if argv is not None else None)

    frames: list[tuple[int, list[int]]] = [
        (size, render_windows_icon(size)) for size in WINDOWS_ICO_SIZES
    ]
    ico_bytes = build_ico(frames)

    tauri_ico = REPO_ROOT / "src-tauri" / "icons" / "icon.ico"
    packaging_ico = REPO_ROOT / "packaging" / "app_icon.ico"
    png32_path = REPO_ROOT / "src-tauri" / "icons" / "32x32.png"

    tauri_ico.write_bytes(ico_bytes)
    packaging_ico.write_bytes(ico_bytes)

    frame_32 = next(rgba for size, rgba in frames if size == 32)
    png32_path.write_bytes(encode_png(32, 32, frame_32))

    if args.preview is not None:
        render_contact_sheet(frames, args.preview)

    print(
        f"Generated Windows icons: {len(frames)} frames "
        f"({', '.join(str(s) for s in WINDOWS_ICO_SIZES)}), "
        f"ico_bytes={len(ico_bytes)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
