#!/usr/bin/env python3
"""Render one exported .c/.p pair, without reconstructing its layout from an image.

Requires matplotlib. PADS Default Font is approximated with DejaVu Sans;
coordinates, rotations, justification, text heights and visibility come from .c.
This is a file preview, not a PADS editor screenshot.
"""
import argparse
import json
import math
import shlex
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.font_manager import FontProperties
from matplotlib.patches import Circle, PathPatch
from matplotlib.textpath import TextPath
from matplotlib.transforms import Affine2D, Bbox


def read_symbol(c_path, p_path, reference, unit=0):
    data = [line for line in c_path.read_text().splitlines() if line.strip()]
    assert data[0] == "*PADS-LIBRARY-SCH-DECALS-V9*"
    starts = [i - 1 for i, line in enumerate(data) if line.startswith("TIMESTAMP ")]
    end = starts[unit + 1] if unit + 1 < len(starts) else len(data) - 1
    lines = iter(data[starts[unit]:end])
    header = next(lines).split()
    name = header[0]
    nheight, nameheight = float(header[3]), float(header[5])
    label_count, piece_count, text_count, term_count, visibility = map(int, header[7:12])
    for _ in range(3):
        next(lines)  # timestamp + number/name fonts
    texts, pieces = [], []
    for _ in range(label_count):
        cols, field = shlex.split(next(lines)), next(lines)
        if field == "REF-DES" and not visibility & 1:
            value = reference
        elif field in ("PARTTYPE", "PART-TYPE") and not visibility & 2:
            value = next(line.split()[0] for line in p_path.read_text().splitlines()
                         if line and not line.startswith("*"))
        else:
            continue
        just = int(cols[3])
        # Attribute labels have their own justification encoding (PADS spec p15).
        horizontal = 1 if just & 4 else (2 if just & 1 else 0)
        vertical = 1 if just & 8 else (2 if just & 2 else 0)
        texts.append((float(cols[0]), float(cols[1]), float(cols[2]) / 10,
                      float(cols[4]), horizontal, vertical, value, field))
    for _ in range(piece_count):
        cols = next(lines).split()
        points = [tuple(map(float, next(lines).split())) for _ in range(int(cols[1]))]
        assert all(len(p) == 2 for p in points), "arc segments are not supported"
        pieces.append((cols[0], float(cols[2]), int(cols[4]), points))
    for _ in range(text_count):
        cols, value = shlex.split(next(lines)), next(lines)
        just = int(cols[7])
        assert int(cols[6]) == 0, "mirrored free text is not supported"
        texts.append((float(cols[0]), float(cols[1]), float(cols[2]), float(cols[4]),
                      just & 15, just >> 4, value, "pin name"))
    part_lines = p_path.read_text().splitlines()
    gate = [i for i, line in enumerate(part_lines) if line.startswith("GATE ")][unit]
    assert part_lines[gate + 1] == name, "part gate must reference this decal"
    pins = [line.split() for line in part_lines[gate + 2:gate + 2 + term_count]]
    assert len(pins) == term_count
    for pin in pins:
        terminal = next(lines)
        assert terminal.startswith("T")
        cols = terminal[1:].split()
        assert len(cols) == 13 and next(lines).startswith("P")
        x, y = map(float, cols[:2])
        # PIN is the standard Logic library decal: a solid 200 mil local +X
        # shaft. Fail closed for missing/custom references, never silently draw
        # a plausible preview for an unresolved dependency such as literal '0'.
        assert cols[12] == "PIN", f"Unresolved/unsupported pin decal: {cols[12]}"
        rotation, mirror = map(int, cols[2:4])
        assert (rotation, mirror) in ((0, 0), (0, 2), (90, 0), (90, 4))
        def pin_local(dx, dy):
            if rotation == 90:
                dx, dy = -dy, dx
            return (-dx if mirror & 2 else dx, -dy if mirror & 4 else dy)
        sx, sy = pin_local(200, 0)
        pieces.append(("OPEN", 10, -1, [(x, y), (x + sx, y + sy)]))
        for offset, height, value, kind, hidden in [
            (4, nheight, pin[0], "pin number", visibility & 16),
            (8, nameheight, pin[3], "pin name", visibility & 24),
        ]:
            if hidden:
                continue
            dx, dy, angle = map(float, cols[offset:offset + 3])
            dx, dy = pin_local(dx, dy)
            angle = (angle + rotation) % 180  # readable pin label orientation
            just = int(cols[offset + 3])
            texts.append((x + dx, y + dy, angle, height, just & 15, just >> 4, value, kind))
    assert next(lines, None) is None, "unexpected trailing decal records"
    return pieces, texts


def render(c_path, p_path, destination, reference, show_bounds, unit=0, pixels_per_mil=None):
    pieces, texts = read_symbol(c_path, p_path, reference, unit)
    color, background = "#00e8ee", "#000000"
    fig, ax = plt.subplots()
    fig.patch.set_facecolor(background)
    ax.set_facecolor(background)
    boxes, text_boxes = [], []
    # PADS 9.4+ explicit line styles. Reject ambiguous legacy values instead of
    # silently depicting them as solid, as the first preview implementation did.
    line_styles = {-1: "solid", -2: "dashed", -3: "dotted", -4: "dashdot",
                   -5: (0, (6, 2, 1, 2, 1, 2)), 0: "dotted"}
    for kind, width, style, points in pieces:
        if style not in line_styles:
            raise ValueError(f"Unverified legacy PADS line style {style}; cannot render it as solid")
        linestyle = line_styles[style]
        if kind == "CIRCLE":
            (x1, y1), (x2, y2) = points
            radius = math.hypot(x2 - x1, y2 - y1) / 2
            x, y = (x1 + x2) / 2, (y1 + y2) / 2
            ax.add_patch(Circle((x, y), radius, fill=False, color=color, linewidth=0.8, linestyle=linestyle))
            boxes.append(Bbox.from_extents(x - radius, y - radius, x + radius, y + radius))
        else:
            assert kind in ("OPEN", "CLOSED"), kind
            xs, ys = zip(*points)
            ax.plot(xs, ys, color=color, linewidth=max(0.6, width * 0.10),
                    linestyle=linestyle, solid_capstyle="butt")
            boxes.append(Bbox.from_extents(min(xs), min(ys), max(xs), max(ys)))
    font = FontProperties(family="DejaVu Sans")
    cap_height = TextPath((0, 0), "H", prop=font, size=1).get_extents().height
    for x, y, angle, height, horizontal, vertical, text, kind in texts:
        if not text:
            continue
        glyphs = TextPath((0, 0), text, prop=font, size=height / cap_height)
        bounds = glyphs.get_extents()
        assert horizontal in (0, 1, 2) and vertical in (0, 1, 2)
        transform = (Affine2D()
                     .translate(-bounds.x0 - bounds.width * horizontal / 2,
                                -bounds.y0 - bounds.height * vertical / 2)
                     .rotate_deg(angle).translate(x, y))
        path = transform.transform_path(glyphs)
        box = path.get_extents()
        text_boxes.append((text, kind, box))
        boxes.append(box)
        ax.add_patch(PathPatch(path, facecolor=color, edgecolor="none"))
        if show_bounds:
            ax.plot([box.x0, box.x1, box.x1, box.x0, box.x0],
                    [box.y0, box.y0, box.y1, box.y1, box.y0], color="#3333ff", linewidth=0.5)
    collisions = []
    for i, (a, ak, ab) in enumerate(text_boxes):
        for b, bk, bb in text_boxes[i + 1:]:
            if ab.overlaps(bb):
                collisions.append([a, ak, b, bk])
    extent = Bbox.union(boxes)
    margin = 110
    xmin, xmax = extent.x0 - margin, extent.x1 + margin
    ymin, ymax = extent.y0 - margin, extent.y1 + margin
    width = math.ceil((xmax - xmin) * pixels_per_mil) if pixels_per_mil else 760
    height = round(width * (ymax - ymin) / (xmax - xmin))
    fig.set_size_inches(width / 120, height / 120)
    ax.set_position([0, 0, 1, 1])
    ax.set_xlim(xmin, xmax)
    ax.set_ylim(ymin, ymax)
    ax.set_aspect("equal")
    ax.axis("off")
    destination.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(destination.with_suffix(".png"), dpi=120, facecolor=background)
    fig.savefig(destination.with_suffix(".svg"), facecolor=background)
    plt.close(fig)
    report = {"source": str(c_path), "part_type": str(p_path), "preview_only": True,
              "font": "DejaVu Sans approximation of PADS Default Font",
              "unit": unit + 1, "text_count": len(text_boxes), "text_bbox_overlaps": collisions,
              "line_styles": sorted({piece[2] for piece in pieces}),
              "pin_decal_dependency": "PADS Logic standard library PIN (200 mil)",
              "native_pin_transforms": "0/0 left, 0/2 right, 90/0 bottom, 90/4 top",
              "pixels_per_mil": width / (xmax - xmin),
              "text_heights_mil": {kind: sorted({height for _, _, _, height, _, _, _, k in texts if k == kind})
                                   for kind in sorted({t[-1] for t in texts})},
              "image_size": [width, height]}
    destination.with_suffix(".json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    if collisions:
        raise SystemExit("text bounding boxes overlap")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("decal", type=Path)
    parser.add_argument("part", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--reference", default="U?")
    parser.add_argument("--bounds", action="store_true")
    parser.add_argument("--unit", type=int, default=1, help="1-based gate index")
    parser.add_argument("--pixels-per-mil", type=float, help="fixed physical scale for before/after comparisons")
    args = parser.parse_args()
    render(args.decal, args.part, args.output, args.reference, args.bounds, args.unit - 1, args.pixels_per_mil)
