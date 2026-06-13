#!/usr/bin/env python3
"""Generate tray icon PNGs for HQ Sync menubar app.

Creates 4 tray-state PNGs from the official HQ SVG mark at @1x (38x22) and
@2x (76x44). Icons are monochrome black on transparent; macOS treats them as
template images and auto-inverts for light/dark menu bars.
"""

from PIL import Image
import os
import shutil
import subprocess
import tempfile

ICON_DIR = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "icons")
SOURCE_SVG = os.path.join(ICON_DIR, "source", "HQ.svg")
STATE_NAMES = ("tray-idle", "tray-syncing", "tray-error", "tray-conflict")
CANVAS_1X = (38, 22)
CANVAS_2X = (76, 44)


def render_source_svg():
    """Rasterize the source SVG with macOS' built-in image renderer."""
    if not os.path.exists(SOURCE_SVG):
        raise FileNotFoundError(f"Missing source SVG: {SOURCE_SVG}")
    if not shutil.which("sips"):
        raise RuntimeError("sips is required to rasterize SVG tray icons on macOS")

    tmpdir = tempfile.TemporaryDirectory()
    path = os.path.join(tmpdir.name, "hq-source.png")
    subprocess.run(
        ["sips", "-s", "format", "png", SOURCE_SVG, "--out", path],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return tmpdir, Image.open(path).convert("RGBA")


def make_template_logo(source, canvas_size):
    """Fit the HQ logo into a menu-bar canvas and convert it to template black."""
    canvas_w, canvas_h = canvas_size
    padding_y = max(1, round(canvas_h * 0.09))
    max_h = canvas_h - padding_y * 2
    max_w = canvas_w
    src_w, src_h = source.size
    scale = min(max_w / src_w, max_h / src_h)
    logo_w = max(1, round(src_w * scale))
    logo_h = max(1, round(src_h * scale))
    logo = source.resize((logo_w, logo_h), Image.Resampling.LANCZOS)

    alpha = logo.getchannel("A")
    template_logo = Image.new("RGBA", logo.size, (0, 0, 0, 0))
    template_logo.putalpha(alpha)

    img = Image.new("RGBA", canvas_size, (0, 0, 0, 0))
    img.alpha_composite(
        template_logo,
        ((canvas_w - logo_w) // 2, (canvas_h - logo_h) // 2),
    )
    return img


def generate_icon(source, name, canvas_size):
    """Generate a single icon PNG."""
    img = make_template_logo(source, canvas_size)
    path = os.path.join(ICON_DIR, name)
    img.save(path, "PNG")
    print(f"  Created {name} ({canvas_size[0]}x{canvas_size[1]})")


def main():
    os.makedirs(ICON_DIR, exist_ok=True)
    print(f"Generating tray icons from {SOURCE_SVG}...")

    tmpdir, source = render_source_svg()
    try:
        for name in STATE_NAMES:
            generate_icon(source, f"{name}.png", CANVAS_1X)
            generate_icon(source, f"{name}@2x.png", CANVAS_2X)
    finally:
        tmpdir.cleanup()

    print("Done. 8 tray icons generated.")


if __name__ == "__main__":
    main()
