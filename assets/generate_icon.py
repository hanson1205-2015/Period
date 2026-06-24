"""Generate Period icon assets (PNG and ICO) from a simple vector description."""
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
SIZE = 256

# Deep blue to purple gradient background, approximated with bands.
image = Image.new("RGBA", (SIZE, SIZE), (30, 58, 138, 255))
draw = ImageDraw.Draw(image)

# Draw smooth vertical gradient from top-left blue to bottom-right purple.
for y in range(SIZE):
    for x in range(SIZE):
        t = (x + y) / (2 * SIZE)
        r = int(30 + (88 - 30) * t)
        g = int(58 + (28 - 58) * t)
        b = int(138 + (135 - 138) * t)
        image.putpixel((x, y), (r, g, b, 255))

# Rounded rectangle clip (approximated with a large rounded rectangle overlay).
mask = Image.new("L", (SIZE, SIZE), 0)
mask_draw = ImageDraw.Draw(mask)
mask_draw.rounded_rectangle((0, 0, SIZE, SIZE), radius=48, fill=255)
image.putalpha(mask)

# Subtle circular ring.
draw = ImageDraw.Draw(image)
draw.ellipse([40, 40, 216, 216], outline=(255, 255, 255, 38), width=8)

# The "period" dot with a light blue to purple gradient, approximated.
for y in range(SIZE):
    for x in range(SIZE):
        dx = x - 128
        dy = y - 172
        dist = (dx * dx + dy * dy) ** 0.5
        if dist <= 28:
            t = (dx + 28) / 56
            r = int(96 + (192 - 96) * t)
            g = int(165 + (132 - 165) * t)
            b = int(250 + (252 - 250) * t)
            image.putpixel((x, y), (r, g, b, 255))

# Save PNG.
image.save(HERE / "period.png", "PNG")

# Save ICO with multiple sizes.
icon_sizes = [16, 24, 32, 48, 64, 128, 256]
ico_images = [image.resize((s, s), Image.Resampling.LANCZOS) for s in icon_sizes]
ico_images[0].save(HERE / "period.ico", sizes=[(s, s) for s in icon_sizes])
print("Generated period.png and period.ico")
