# Genera assets/jugadon.ico (ícono del exe y de la bandeja) y assets/banner.png (README).
# Uso: python3 assets/make_assets.py   (requiere Pillow)
from PIL import Image, ImageDraw, ImageFont, ImageFilter
import os

HERE = os.path.dirname(os.path.abspath(__file__))
TOP, BOTTOM = (255, 46, 99), (255, 154, 0)  # rosa fuerte -> naranja


def gradient(size, c1, c2):
    g = Image.new("RGB", (1, 256))
    for y in range(256):
        t = y / 255
        g.putpixel((0, y), tuple(int(a + (b - a) * t) for a, b in zip(c1, c2)))
    return g.resize((size, size))


def icon(size):
    s = size * 4  # supersampling
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    mask = Image.new("L", (s, s), 0)
    small = size <= 24
    pad = 0 if small else int(s * 0.04)
    ImageDraw.Draw(mask).rounded_rectangle([pad, pad, s - pad - 1, s - pad - 1], radius=int(s * 0.24), fill=255)
    img.paste(gradient(s, TOP, BOTTOM), (0, 0), mask)
    d = ImageDraw.Draw(img)
    # triángulo de play blanco, apenas corrido a la derecha para que se vea centrado
    w = s * (0.50 if small else 0.44)
    cx, cy = s * 0.54, s * 0.5
    tri = [(cx - w * 0.45, cy - w * 0.55), (cx - w * 0.45, cy + w * 0.55), (cx + w * 0.55, cy)]
    if not small:
        sh = Image.new("RGBA", (s, s), (0, 0, 0, 0))
        ImageDraw.Draw(sh).polygon([(x, y + s * 0.02) for x, y in tri], fill=(120, 0, 30, 110))
        img.alpha_composite(sh.filter(ImageFilter.GaussianBlur(s * 0.02)))
        # líneas de velocidad
        for i, (yy, ll) in enumerate([(-0.16, 0.13), (0.0, 0.18), (0.16, 0.13)]):
            y = cy + yy * s
            x2 = cx - w * 0.45 - s * 0.05
            d.rounded_rectangle([x2 - ll * s, y - s * 0.022, x2, y + s * 0.022], radius=int(s * 0.022), fill=(255, 255, 255, 230))
    d.polygon(tri, fill=(255, 255, 255, 255))
    return img.resize((size, size), Image.LANCZOS)


sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
imgs = [icon(n) for n in sizes]
imgs[-1].save(os.path.join(HERE, "jugadon.ico"), sizes=[(n, n) for n in sizes], append_images=imgs[:-1])
icon(512).save(os.path.join(HERE, "icon.png"))

# Banner del README
W, H = 1280, 400
b = Image.new("RGBA", (W, H), (18, 16, 26, 255))
glow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
ImageDraw.Draw(glow).ellipse([60, 40, 400, 380], fill=(255, 80, 80, 90))
b.alpha_composite(glow.filter(ImageFilter.GaussianBlur(60)))
b.alpha_composite(icon(260), (90, 70))
font = "/usr/share/fonts/opentype/inter/InterDisplay-Black.otf"
if not os.path.exists(font):
    font = "/usr/share/fonts/opentype/inter/InterDisplay-Bold.otf"
d = ImageDraw.Draw(b)
title = ImageFont.truetype(font, 128)
sub = ImageFont.truetype("/usr/share/fonts/opentype/inter/InterDisplay-SemiBold.otf", 34)
# título con degradé
tw = d.textbbox((0, 0), "Jugadón", font=title)
tmask = Image.new("L", (W, H), 0)
ImageDraw.Draw(tmask).text((410, 95), "Jugadón", font=title, fill=255)
grad = Image.new("RGB", (W, H))
for x in range(W):
    t = min(max((x - 410) / max(tw[2], 1), 0), 1)
    col = tuple(int(a + (c - a) * t) for a, c in zip(TOP, BOTTOM))
    ImageDraw.Draw(grad).line([(x, 0), (x, H)], fill=col)
b.paste(grad, (0, 0), tmask)
d.text((414, 250), "Apretá F9 y guardá la jugada. Sin trabar el juego.", font=sub, fill=(235, 235, 245))
b.convert("RGB").save(os.path.join(HERE, "banner.png"), optimize=True)
print("ok")
