#!/usr/bin/env python3
"""Generate tray icon PNGs for HQ Sync menubar app.

Creates 4 states (idle, syncing, error, conflict) at @1x (22x22) and @2x (44x44).
Icons are monochrome black on transparent — macOS treats them as template images
and auto-inverts for dark mode.
"""

from PIL import Image, ImageDraw
import os

ICON_DIR = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "icons")


def draw_h_letterform(draw, size):
    """Draw a simple geometric 'H' letterform centered in the icon."""
    # Proportional sizing
    s = size
    pad = max(2, s // 5)
    stroke = max(2, s // 8)
    half = s // 2

    # Left vertical bar
    draw.rectangle([pad, pad, pad + stroke - 1, s - pad - 1], fill="black")
    # Right vertical bar
    draw.rectangle([s - pad - stroke, pad, s - pad - 1, s - pad - 1], fill="black")
    # Horizontal crossbar
    draw.rectangle([pad, half - stroke // 2, s - pad - 1, half + stroke // 2], fill="black")


def draw_sync_arrows(draw, size):
    """Draw circular arrow overlay (two curved arrows) for syncing state."""
    s = size
    cx, cy = s // 2, s // 2
    r = max(3, s // 3)

    # Draw two small arrow indicators at top-right and bottom-left
    arrow_size = max(2, s // 8)

    # Top arc arrow (right side)
    for i in range(max(2, r)):
        angle_offset = i * 0.15
        x = int(cx + r * 0.7 + arrow_size * 0.3)
        y = int(cy - r * 0.3 + i)
        if 0 <= x < s and 0 <= y < s:
            draw.point((x, y), fill="black")

    # Small arrows as triangles
    # Top-right arrow
    ax, ay = cx + r, cy - arrow_size
    draw.polygon([(ax, ay), (ax + arrow_size, ay + arrow_size // 2), (ax, ay + arrow_size)], fill="black")
    # Bottom-left arrow
    ax, ay = cx - r, cy + arrow_size
    draw.polygon([(ax, ay), (ax - arrow_size, ay - arrow_size // 2), (ax, ay - arrow_size)], fill="black")

    # Arc segments (simplified as lines)
    import math
    for angle in range(0, 180, 8):
        rad = math.radians(angle)
        x = int(cx + r * math.cos(rad))
        y = int(cy - r * math.sin(rad))
        if 0 <= x < s and 0 <= y < s:
            draw.point((x, y), fill="black")
    for angle in range(180, 360, 8):
        rad = math.radians(angle)
        x = int(cx + r * math.cos(rad))
        y = int(cy - r * math.sin(rad))
        if 0 <= x < s and 0 <= y < s:
            draw.point((x, y), fill="black")


def draw_dot_badge(draw, size, color):
    """Draw a small colored dot badge in the bottom-right corner."""
    s = size
    dot_r = max(2, s // 6)
    cx = s - dot_r - 1
    cy = s - dot_r - 1
    draw.ellipse([cx - dot_r, cy - dot_r, cx + dot_r, cy + dot_r], fill=color)


def generate_icon(name, size, draw_fn):
    """Generate a single icon PNG."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    draw_fn(draw, size)
    path = os.path.join(ICON_DIR, name)
    img.save(path, "PNG")
    print(f"  Created {name} ({size}x{size})")


def make_idle(draw, size):
    draw_h_letterform(draw, size)


def make_syncing(draw, size):
    draw_h_letterform(draw, size)
    draw_sync_arrows(draw, size)


def make_error(draw, size):
    draw_h_letterform(draw, size)
    draw_dot_badge(draw, size, "black")  # monochrome — macOS template will handle coloring


def make_conflict(draw, size):
    draw_h_letterform(draw, size)
    # Amber/conflict: use a slightly different badge — a small diamond shape
    s = size
    dot_r = max(2, s // 6)
    cx = s - dot_r - 1
    cy = s - dot_r - 1
    draw.polygon([
        (cx, cy - dot_r),
        (cx + dot_r, cy),
        (cx, cy + dot_r),
        (cx - dot_r, cy),
    ], fill="black")


def main():
    os.makedirs(ICON_DIR, exist_ok=True)
    print("Generating tray icons...")

    states = {
        "tray-idle": make_idle,
        "tray-syncing": make_syncing,
        "tray-error": make_error,
        "tray-conflict": make_conflict,
    }

    for name, draw_fn in states.items():
        generate_icon(f"{name}.png", 22, draw_fn)
        generate_icon(f"{name}@2x.png", 44, draw_fn)

    print("Done! 8 tray icons generated.")


if __name__ == "__main__":
    main()
