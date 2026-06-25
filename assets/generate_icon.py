"""Generate Period icon assets (PNG and ICO) matching the site amber theme."""
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
SIZE = 256

# Transparent background.
image = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
draw = ImageDraw.Draw(image)

# Amber accent palette.
RING_COLOR = (217, 119, 6, 255)       # #d97706
DOT_COLOR = (180, 83, 9, 255)         # #b45309

# Thick outer ring.
draw.ellipse([28, 28, 228, 228], outline=RING_COLOR, width=32)
# Solid central dot.
draw.ellipse([100, 100, 156, 156], fill=DOT_COLOR)

# Save PNG.
image.save(HERE / "period.png", "PNG")

# Save ICO with multiple sizes.
icon_sizes = [16, 24, 32, 48, 64, 128, 256]
ico_images = [image.resize((s, s), Image.Resampling.LANCZOS) for s in icon_sizes]
ico_images[0].save(HERE / "period.ico", sizes=[(s, s) for s in icon_sizes])
print("Generated period.png and period.ico")
