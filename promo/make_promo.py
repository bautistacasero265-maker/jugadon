#!/usr/bin/env python3
"""Video promocional vertical (1080x1920, ~30 s) de Jugadón, para TikTok / Reels / Shorts.

Todo se genera acá: imágenes con PIL, música y efectos sintetizados con numpy, voz con Piper
(si se pasa --voice; sin voz sale igual, en silencio, para probar). Se arma con ffmpeg.

Estructura de video viral: gancho en los primeros 2 s, cortes rápidos (una idea por escena),
subtítulos grandes palabra por palabra, "zoom punch" y destellos en cada corte, efectos de sonido,
música con caída ("drop") después del gancho y llamado a la acción al final.
"""
import argparse
import math
import os
import random
import subprocess
import wave

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(ROOT)
W, H, FPS, SR = 1080, 1920, 30, 48000

ORANGE = (255, 138, 0)
PINK = (255, 46, 110)
RED = (240, 52, 72)
GREEN = (46, 214, 120)
WHITE = (255, 255, 255)
MUTED = (170, 165, 195)
BG1 = (15, 14, 21)
BG2 = (34, 16, 44)
YELLOW = (255, 214, 0)

BLACK_F = os.path.join(ROOT, "fonts", "InterDisplay-Black.otf")
BOLD_F = os.path.join(ROOT, "fonts", "InterDisplay-Bold.otf")
_fonts = {}


def font(size, black=True):
    k = (size, black)
    if k not in _fonts:
        _fonts[k] = ImageFont.truetype(BLACK_F if black else BOLD_F, size)
    return _fonts[k]


# (texto que dice la voz, subtítulo en pantalla, escena)
# La voz lee la pronunciación ("efe nueve"); el subtítulo muestra cómo se escribe ("F9").
SCRIPT = [
    ("¿Hiciste la jugada de tu vida... y no estabas grabando?", "¿Hiciste la jugada de tu vida... y NO estabas grabando?", "hook"),
    ("Con Jugadón, apretás efe nueve y se guardan los últimos treinta segundos.", "Con Jugadón apretás F9 y se guardan los últimos 30 segundos", "f9"),
    ("Lo comprime tu placa de video, así el juego no pierde efe pe ese.", "Lo comprime tu placa de video: el juego no pierde FPS", "gpu"),
    ("Y en reposo no consume casi nada. Está hecho en Rost.", "En reposo no consume casi nada. Hecho en Rust", "light"),
    ("Todos tus clips quedan ordenados, listos para compartir.", "Todos tus clips ordenados, listos para compartir", "gallery"),
    ("Los recortás mientras los ves, y con un toque copiás un link que abre cualquiera.", "Recortalos mientras los ves y copiá un link que abre cualquiera", "features"),
    ("Con efe ocho grabás la pantalla, o una sola ventana.", "Con F8 grabás la pantalla o una sola ventana", "f8"),
    ("Es gratis. Sin cuentas, y sin publicidad.", "Gratis. Sin cuentas y sin publicidad", "free"),
    ("Bajalo ya. El link está en la bio.", "Bajalo ya: link en la bio", "cta"),
]


# ---------------------------------------------------------------- voz

def synth_voice(args):
    """Devuelve una lista de arrays float32 a 48 kHz (uno por línea) o None si no hay voz."""
    if not args.voice:
        return None
    out = []
    os.makedirs(args.tmp, exist_ok=True)
    for i, (say, _, _) in enumerate(SCRIPT):
        path = os.path.join(args.tmp, f"voz_{i}.wav")
        cmd = [args.piper, "--model", args.voice, "--output_file", path, "--length_scale", "0.9", "--sentence_silence", "0.05"]
        subprocess.run(cmd, input=say.encode("utf-8"), check=True)
        with wave.open(path) as w:
            sr, n = w.getframerate(), w.getnframes()
            a = np.frombuffer(w.readframes(n), dtype=np.int16).astype(np.float32) / 32768.0
        # a 48 kHz y sin silencios en las puntas
        a = np.interp(np.arange(0, len(a), sr / SR), np.arange(len(a)), a).astype(np.float32)
        nz = np.where(np.abs(a) > 0.01)[0]
        if len(nz):
            a = a[max(0, nz[0] - 800): nz[-1] + 2400]
        out.append(a)
    return out


# ---------------------------------------------------------------- audio: música y efectos

def env_exp(n, k):
    return np.exp(-np.arange(n) / SR * k).astype(np.float32)


def kick():
    n = int(SR * 0.35)
    t = np.arange(n) / SR
    f = 50 + 110 * np.exp(-t * 30)
    ph = np.cumsum(2 * np.pi * f / SR)
    return (np.sin(ph) * env_exp(n, 9)).astype(np.float32)


def noise(n, seed=0):
    return np.random.default_rng(seed).uniform(-1, 1, n).astype(np.float32)


def hat():
    n = int(SR * 0.05)
    x = noise(n, 1)
    x = np.diff(x, prepend=0)  # agudo
    return x * env_exp(n, 70) * 0.35


def clap():
    n = int(SR * 0.2)
    x = noise(n, 2)
    x = x - np.convolve(x, np.ones(8) / 8, mode="same")
    return x * env_exp(n, 22) * 0.55


def tone(freq, dur, kind="saw", vol=1.0):
    n = int(SR * dur)
    t = np.arange(n) / SR
    if kind == "sin":
        x = np.sin(2 * np.pi * freq * t)
    else:
        x = 2 * ((t * freq) % 1) - 1
    return (x * vol).astype(np.float32)


def lowpass(x, a):
    y = np.empty_like(x)
    acc = 0.0
    for i in range(len(x)):  # simple, alcanza para unos segundos de audio
        acc += a * (x[i] - acc)
        y[i] = acc
    return y


def add(buf, x, at, vol=1.0):
    i = int(at * SR)
    if i >= len(buf):
        return
    j = min(len(buf), i + len(x))
    buf[i:j] += x[: j - i] * vol


def whoosh():
    n = int(SR * 0.4)
    x = noise(n, 3)
    t = np.arange(n) / n
    a = 0.02 + 0.5 * t  # se abre el filtro
    y = np.empty_like(x)
    acc = 0.0
    for i in range(n):
        acc += a[i] * (x[i] - acc)
        y[i] = acc
    return y * np.sin(np.pi * t) * 0.6


def riser(dur):
    n = int(SR * dur)
    t = np.arange(n) / SR
    f = 200 * (6 ** (t / dur))
    ph = np.cumsum(2 * np.pi * f / SR)
    return (np.sin(ph) * 0.15 + noise(n, 4) * 0.08 * (t / dur)) * (t / dur)


def ding():
    # el mismo "tin-tin" que hace la app al guardar
    a = tone(880, 0.07, "sin") * env_exp(int(SR * 0.07), 8)
    b = tone(1318.5, 0.22, "sin") * env_exp(int(SR * 0.22), 9)
    return np.concatenate([a, b]) * 0.5


def keyclick():
    n = int(SR * 0.04)
    return noise(n, 5) * env_exp(n, 120) * 0.6


def boom():
    n = int(SR * 1.2)
    t = np.arange(n) / SR
    f = 40 + 60 * np.exp(-t * 6)
    ph = np.cumsum(2 * np.pi * f / SR)
    return np.sin(ph) * env_exp(n, 2.5) * 0.9


def music(total, drop_at, bpm=124):
    """Beat electrónico simple: arranca suave (gancho), cae fuerte en el drop."""
    buf = np.zeros(int(total * SR) + SR, dtype=np.float32)
    beat = 60 / bpm
    k, h, c = kick(), hat(), clap()
    # bajo: La menor, Fa, Do, Sol (dos compases cada uno)
    roots = [55.0, 43.65, 65.41, 49.0]
    t = 0.0
    i = 0
    while t < total:
        after = t >= drop_at
        bar_pos = i % 4
        if after:
            add(buf, k, t, 0.9)
            if bar_pos in (1, 3):
                add(buf, c, t, 0.6)
            root = roots[(i // 8) % 4]
            b = tone(root, beat * 0.9) * env_exp(int(SR * beat * 0.9), 3)
            add(buf, b, t, 0.25)
            add(buf, h, t + beat / 2, 1.0)
        else:
            # gancho: solo un pad tenso
            if bar_pos == 0:
                pad = tone(110, beat * 4, "sin") * 0.12 + tone(164.8, beat * 4, "sin") * 0.08
                add(buf, pad, t, 1.0)
        t += beat
        i += 1
    # suavizar el bajo/saw
    buf = lowpass(buf, 0.35)
    return buf[: int(total * SR)]


# ---------------------------------------------------------------- dibujo

def ease_out(x):
    x = max(0.0, min(1.0, x))
    return 1 - (1 - x) ** 3


def ease_back(x):
    x = max(0.0, min(1.0, x))
    c = 1.7
    return 1 + (c + 1) * (x - 1) ** 3 + c * (x - 1) ** 2


def gradient_bg():
    y = np.linspace(0, 1, H)[:, None]
    top = np.array(BG1, dtype=np.float32)
    bot = np.array(BG2, dtype=np.float32)
    g = top * (1 - y[..., None]) + bot * y[..., None]
    g = np.repeat(g, W, axis=1)
    return Image.fromarray(g.astype(np.uint8), "RGB")


def glow(color, r):
    s = r * 2
    im = Image.new("RGBA", (s, s), color + (0,))
    d = ImageDraw.Draw(im)
    d.ellipse((r // 2, r // 2, s - r // 2, s - r // 2), fill=color + (150,))
    return im.filter(ImageFilter.GaussianBlur(r // 3))


BG = None
GLOWS = None


def background(t, tint=None):
    im = BG.copy()
    for k, (g, ox, oy, sp) in enumerate(GLOWS):
        x = int(W / 2 + math.sin(t * sp + k * 2) * 380 - g.width / 2 + ox)
        y = int(H / 2 + math.cos(t * sp * 0.8 + k) * 600 - g.height / 2 + oy)
        im.paste(g, (x, y), g)
    if tint:
        ov = Image.new("RGB", (W, H), tint[0])
        im = Image.blend(im, ov, tint[1])
    return im


def text_center(d, y, s, size, fill=WHITE, black=True, stroke=0, stroke_fill=(0, 0, 0)):
    f = font(size, black)
    w = d.textlength(s, font=f)
    d.text(((W - w) / 2, y), s, font=f, fill=fill, stroke_width=stroke, stroke_fill=stroke_fill)
    return w


def rounded(d, box, r, fill, outline=None, width=0):
    d.rounded_rectangle(box, r, fill=fill, outline=outline, width=width)


def gradient_text(img, y, s, size, c1=ORANGE, c2=PINK):
    """Texto con degradé naranja→rosa (el color de la marca)."""
    f = font(size)
    tmp = ImageDraw.Draw(img)
    w = int(tmp.textlength(s, font=f))
    bbox = f.getbbox(s)
    h = bbox[3] + 10
    mask = Image.new("L", (w + 20, h + 20), 0)
    ImageDraw.Draw(mask).text((10, 10), s, font=f, fill=255)
    grad = np.zeros((h + 20, w + 20, 3), dtype=np.uint8)
    xs = np.linspace(0, 1, w + 20)[None, :, None]
    grad[:] = (np.array(c1) * (1 - xs) + np.array(c2) * xs).astype(np.uint8)
    gi = Image.fromarray(grad, "RGB")
    shadow = mask.filter(ImageFilter.GaussianBlur(14))
    x = (W - w - 20) // 2
    img.paste((0, 0, 0), (x, int(y) + 8), shadow)
    img.paste(gi, (x, int(y)), mask)


def keycap(img, cx, cy, label, pressed, size=300, color=ORANGE):
    d = ImageDraw.Draw(img)
    off = 26 if pressed else 0
    x0, y0 = cx - size // 2, cy - size // 2
    rounded(d, (x0, y0 + 40, x0 + size, y0 + size + 40), 48, (60, 30, 20))
    rounded(d, (x0, y0 + off, x0 + size, y0 + size + off), 48, color)
    rounded(d, (x0 + 22, y0 + 18 + off, x0 + size - 22, y0 + size - 50 + off), 36, tuple(min(255, c + 30) for c in color))
    f = font(int(size * 0.42))
    w = d.textlength(label, font=f)
    d.text((cx - w / 2, y0 + size * 0.22 + off), label, font=f, fill=WHITE)


def toast(img, y, title, sub, icon):
    d = ImageDraw.Draw(img)
    x0, x1 = 90, W - 90
    sh = Image.new("RGBA", (x1 - x0 + 80, 260), (0, 0, 0, 0))
    ImageDraw.Draw(sh).rounded_rectangle((40, 40, x1 - x0 + 40, 220), 40, fill=(0, 0, 0, 160))
    sh = sh.filter(ImageFilter.GaussianBlur(18))
    img.paste(sh, (x0 - 40, int(y) - 30), sh)
    rounded(d, (x0, y, x1, y + 180), 40, (36, 32, 50), outline=(80, 70, 110), width=3)
    ic = icon.resize((110, 110), Image.LANCZOS)
    img.paste(ic, (x0 + 34, int(y) + 35), ic)
    d.text((x0 + 170, y + 34), title, font=font(52), fill=WHITE)
    d.text((x0 + 172, y + 100), sub, font=font(38, False), fill=MUTED)
    # tilde verde
    cx, cy = x1 - 90, y + 90
    d.ellipse((cx - 44, cy - 44, cx + 44, cy + 44), fill=GREEN)
    d.line((cx - 20, cy + 2, cx - 4, cy + 18, cx + 22, cy - 16), fill=WHITE, width=12, joint="curve")


def chip(d, cx, y, s, size=44, fill=(44, 38, 62), color=WHITE):
    f = font(size, False)
    w = d.textlength(s, font=f)
    rounded(d, (cx - w / 2 - 34, y, cx + w / 2 + 34, y + size + 40), (size + 40) // 2, fill)
    d.text((cx - w / 2, y + 16), s, font=f, fill=color)


# ---------------------------------------------------------------- escenas (t = segundos desde que empezó, p = 0..1)

def sc_hook(img, t, p):
    d = ImageDraw.Draw(img)
    # mira de un juego
    cx, cy = W // 2, 640
    r = 150 + 10 * math.sin(t * 6)
    d.ellipse((cx - r, cy - r, cx + r, cy + r), outline=(255, 255, 255, 180), width=8)
    for a, b in [((cx - r - 70, cy), (cx - r + 50, cy)), ((cx + r - 50, cy), (cx + r + 70, cy)), ((cx, cy - r - 70), (cx, cy - r + 50)), ((cx, cy + r - 50), (cx, cy + r + 70))]:
        d.line((a, b), fill=WHITE, width=10)
    d.ellipse((cx - 14, cy - 14, cx + 14, cy + 14), fill=RED)
    s = ease_back(t / 0.35)
    if s > 0:
        text_center(d, 920, "LA JUGADA", int(120 * min(s, 1.15)), stroke=8)
        text_center(d, 1060, "DE TU VIDA", int(120 * min(ease_back((t - 0.15) / 0.35), 1.15)) if t > 0.15 else 1, stroke=8)
    if p > 0.55:
        k = ease_back((p - 0.55) / 0.12)
        # sello rojo "NO GRABABA" torcido
        stamp = Image.new("RGBA", (900, 260), (0, 0, 0, 0))
        sd = ImageDraw.Draw(stamp)
        sd.rounded_rectangle((10, 10, 890, 250), 30, outline=RED + (255,), width=16)
        f = font(130)
        w = sd.textlength("NO GRABABA", font=f)
        sd.text(((900 - w) / 2, 50), "NO GRABABA", font=f, fill=RED + (255,))
        sz = max(1, int(900 * (2.2 - 1.2 * k)))
        stamp = stamp.resize((sz, int(sz * 260 / 900))).rotate(-8, expand=True, resample=Image.BICUBIC)
        img.paste(stamp, ((W - stamp.width) // 2, 1240 - stamp.height // 2 + 130), stamp)


def sc_f9(img, t, p, icon):
    keycap(img, W // 2, 760, "F9", 0.35 < t < 0.65)
    if t > 0.55:
        y = 260 - 420 * (1 - ease_out((t - 0.55) / 0.35))
        toast(img, y, "Clip guardado", "los últimos 30 segundos", icon)
    if t > 0.7:
        d = ImageDraw.Draw(img)
        # barra de "últimos 30 s" llenándose
        k = ease_out((t - 0.7) / 0.9)
        rounded(d, (140, 1120, W - 140, 1170), 25, (44, 38, 62))
        x1 = 140 + (W - 280) * k
        if x1 > 190:
            rounded(d, (140, 1120, x1, 1170), 25, ORANGE)
        text_center(d, 1190, "-30 s  →  ahora", 46, MUTED, black=False)


def sc_gpu(img, t, p):
    d = ImageDraw.Draw(img)
    # contador de FPS que no baja
    fps = 144 if t > 0.3 else int(144 * ease_out(t / 0.3))
    gradient_text(img, 380, f"{fps}", 330, GREEN, (120, 255, 190))
    text_center(d, 720, "FPS", 120, GREEN)
    # gráfico plano
    pts = []
    for i in range(40):
        x = 140 + i * (W - 280) / 39
        y = 1010 + math.sin(i * 1.7 + t * 4) * 6
        pts.append((x, y))
    k = int(len(pts) * ease_out(t / 0.8))
    if k > 1:
        d.line(pts[:k], fill=GREEN, width=10, joint="curve")
    chip(d, W // 2, 1110, "el video lo comprime la placa", 46)
    if t > 0.6:
        chip(d, W // 2, 1230, "GPU ✓   ·   procesador libre", 46, fill=(30, 70, 50), color=(170, 255, 200))


def sc_light(img, t, p):
    d = ImageDraw.Draw(img)
    k = ease_out(t / 0.7)
    gradient_text(img, 420, f"{int(108 * k)} KB", 230)
    text_center(d, 720, "de RAM en reposo", 70, WHITE, black=False)
    if t > 0.5:
        s = ease_back((t - 0.5) / 0.3)
        f = int(90 * max(0.01, s))
        rounded(d, (W / 2 - 330, 950, W / 2 + 330, 1110), 80, (52, 34, 30), outline=ORANGE, width=6)
        text_center(d, 975, "HECHO EN RUST", max(10, int(f * 0.8)), ORANGE)


def sc_gallery(img, t, p, shot):
    # captura real de la ventana, en un marco, con zoom lento
    z = 1.0 + 0.12 * p
    sw = int(1000 * z)
    sh = int(sw * shot.height / shot.width)
    im = shot.resize((sw, sh), Image.LANCZOS)
    x = (W - sw) // 2
    y = 520 - int(40 * p)
    frame = Image.new("RGBA", (sw + 24, sh + 24), (0, 0, 0, 0))
    ImageDraw.Draw(frame).rounded_rectangle((0, 0, sw + 23, sh + 23), 36, fill=(255, 138, 0, 255))
    img.paste(frame, (x - 12, y - 12), frame)
    mask = Image.new("L", (sw, sh), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, sw - 1, sh - 1), 28, fill=255)
    img.paste(im, (x, y), mask)
    d = ImageDraw.Draw(img)
    text_center(d, 260, "MIS CLIPS", 110, WHITE)


def sc_features(img, t, p):
    d = ImageDraw.Draw(img)
    items = [("Ver y recortar", "mientras lo mirás, al instante", ORANGE),
             ("Copiar link", "lo abre cualquiera, sin la app", PINK),
             ("Achicar para Discord", "menos de 10 MB, sin Nitro", (88, 101, 242)),
             ("Favoritos y nombres", "\"ace en Valorant\"", (255, 196, 0))]
    for i, (a, b, c) in enumerate(items):
        k = ease_back((t - i * 0.32) / 0.3)
        if k <= 0:
            continue
        y = 300 + i * 270
        x0 = 110 - int(700 * (1 - min(k, 1)))
        rounded(d, (x0, y, x0 + W - 220, y + 220), 44, (36, 32, 50), outline=c, width=6)
        d.ellipse((x0 + 40, y + 60, x0 + 140, y + 160), fill=c)
        d.text((x0 + 180, y + 42), a, font=font(66), fill=WHITE)
        d.text((x0 + 182, y + 128), b, font=font(44, False), fill=MUTED)


def sc_f8(img, t, p):
    keycap(img, W // 2, 620, "F8", 0.25 < t < 0.55, color=RED)
    d = ImageDraw.Draw(img)
    if t > 0.5:
        on = int(t * 2.5) % 2 == 0
        if on:
            d.ellipse((250, 1000, 330, 1080), fill=RED)
        secs = int((t - 0.5) * 6)
        d.text((360, 985), f"REC  00:{secs:02d}", font=font(100), fill=WHITE)
        chip(d, W // 2, 1170, "pantalla completa  ·  o una sola ventana", 44)


def sc_free(img, t, p):
    d = ImageDraw.Draw(img)
    s = ease_back(t / 0.35)
    if s > 0:
        gradient_text(img, 470, "GRATIS", max(10, int(230 * min(s, 1.1))))
    for i, w_ in enumerate(["sin cuentas", "sin publicidad", "sin marca de agua"]):
        k = ease_out((t - 0.4 - i * 0.25) / 0.3)
        if k > 0:
            chip(d, W // 2, 860 + i * 150 + int(40 * (1 - k)), "✓  " + w_, 52, fill=(30, 70, 50), color=(170, 255, 200))


def sc_cta(img, t, p, icon):
    d = ImageDraw.Draw(img)
    s = ease_back(t / 0.4)
    sz = max(1, int(380 * min(s, 1.1)))
    ic = icon.resize((sz, sz), Image.LANCZOS)
    img.paste(ic, ((W - sz) // 2, 360 + (380 - sz) // 2), ic)
    if t > 0.25:
        gradient_text(img, 800, "JUGADÓN", 170)
    if t > 0.45:
        text_center(d, 1010, "clips de juegos para PCs de pocos recursos", 42, MUTED, black=False)
        pulse = 1 + 0.06 * math.sin(t * 9)
        bw, bh = int(720 * pulse), int(150 * pulse)
        rounded(d, ((W - bw) / 2, 1120, (W + bw) / 2, 1120 + bh), bh // 2, ORANGE)
        text_center(d, 1120 + bh / 2 - 42, "DESCARGALO GRATIS", int(62 * pulse))
        # flecha hacia abajo (link en la bio)
        ay = 1340 + 18 * math.sin(t * 7)
        d.polygon([(W / 2 - 50, ay), (W / 2 + 50, ay), (W / 2, ay + 60)], fill=WHITE)


# ---------------------------------------------------------------- subtítulos palabra por palabra

def caption(img, words, idx, pop):
    """Muestra un grupo de hasta 3 palabras; la que se está diciendo va en amarillo y más grande."""
    d = ImageDraw.Draw(img)
    g0 = (idx // 3) * 3
    group = words[g0:g0 + 3]
    sizes = []
    for j, w_ in enumerate(group):
        sz = 84 if g0 + j == idx else 76
        if g0 + j == idx:
            sz = int(sz * (1 + 0.15 * (1 - pop)))
        sizes.append(sz)
    gap = 26
    widths = [d.textlength(w_.upper(), font=font(s_)) for w_, s_ in zip(group, sizes)]
    total = sum(widths) + gap * (len(group) - 1)
    if total > W - 100:  # que entre siempre
        k = (W - 100) / total
        sizes = [int(s_ * k) for s_ in sizes]
        widths = [d.textlength(w_.upper(), font=font(s_)) for w_, s_ in zip(group, sizes)]
        total = sum(widths) + gap * (len(group) - 1)
    x = (W - total) / 2
    for j, (w_, s_, wd) in enumerate(zip(group, sizes, widths)):
        col = YELLOW if g0 + j == idx else WHITE
        d.text((x, 1560 - s_ / 2), w_.upper(), font=font(s_), fill=col, stroke_width=9, stroke_fill=(0, 0, 0))
        x += wd + gap


# ---------------------------------------------------------------- armado

def main():
    global BG, GLOWS
    ap = argparse.ArgumentParser()
    ap.add_argument("--voice", help="modelo .onnx de Piper (sin esto: sin voz, para probar)")
    ap.add_argument("--piper", default="piper")
    ap.add_argument("--out", default=os.path.join(REPO, "promo", "out", "Jugadon-promo.mp4"))
    ap.add_argument("--tmp", default="/tmp/jugadon_promo")
    ap.add_argument("--frames", help="solo guardar estos cuadros como png (ej: 10,80,200)")
    args = ap.parse_args()
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    os.makedirs(args.tmp, exist_ok=True)
    random.seed(1)

    BG = gradient_bg()
    GLOWS = [(glow(ORANGE, 420), -200, -300, 0.5), (glow(PINK, 480), 200, 300, 0.4), (glow((120, 60, 255), 360), 0, 0, 0.3)]
    icon = Image.open(os.path.join(REPO, "assets", "icon.png")).convert("RGBA")
    shot = Image.open(os.path.join(REPO, "assets", "captura_ventana.png")).convert("RGB")

    voices = synth_voice(args)
    # duración de cada escena: lo que dura la voz + un respiro (sin voz: por cantidad de letras)
    durs = []
    for i, (say, cap, _) in enumerate(SCRIPT):
        v = len(voices[i]) / SR if voices else len(say) * 0.055
        durs.append(max(2.0, v + 0.35))
    starts = np.cumsum([0] + durs[:-1]).tolist()
    total = sum(durs) + 0.6
    print(f"duración total: {total:.1f} s")

    # ---------- audio
    drop = starts[1]
    mus = music(total, drop) * 0.55
    voice_track = np.zeros_like(mus)
    sfx = np.zeros_like(mus)
    for i, st in enumerate(starts):
        if voices:
            add(voice_track, voices[i], st + 0.12, 1.0)
        if i > 0:
            add(sfx, whoosh(), st - 0.15, 0.7)
    add(sfx, boom(), 0.0, 0.7)
    add(sfx, riser(drop - 0.3), 0.3, 0.6)
    add(sfx, boom(), drop, 0.8)
    add(sfx, keyclick(), starts[1] + 0.35, 1.0)
    add(sfx, ding(), starts[1] + 0.6, 1.0)
    add(sfx, keyclick(), starts[6] + 0.25, 1.0)
    add(sfx, ding(), starts[8] + 0.3, 0.8)
    # la música baja cuando habla la voz ("ducking")
    if voices:
        envv = np.abs(voice_track)
        k = int(SR * 0.15)
        envv = np.convolve(envv, np.ones(k) / k, mode="same")
        duck = 1 - 0.6 * np.clip(envv * 8, 0, 1)
        mus *= duck
    mix = mus + voice_track * 1.1 + sfx
    mix /= max(1.0, np.max(np.abs(mix)) / 0.95)
    wav = os.path.join(args.tmp, "mix.wav")
    with wave.open(wav, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes((mix * 32767).astype(np.int16).tobytes())

    # ---------- palabras con su momento (proporcional a las letras de la voz)
    timed = []
    for i, (say, cap, _) in enumerate(SCRIPT):
        words = cap.split()
        weights = [len(x) + 2 for x in words]
        span = durs[i] - 0.35
        acc = 0
        for wd, wt in zip(words, weights):
            timed.append((starts[i] + 0.12 + span * acc / sum(weights), i, wd))
            acc += wt

    # ---------- video
    only = set(int(x) for x in args.frames.split(",")) if args.frames else None
    nframes = int(total * FPS)
    proc = None
    if only is None:
        proc = subprocess.Popen(
            ["ffmpeg", "-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}", "-r", str(FPS), "-i", "-",
             "-i", wav, "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k",
             "-shortest", "-movflags", "+faststart", args.out],
            stdin=subprocess.PIPE)
    for fi in range(nframes):
        if only is not None and fi not in only:
            continue
        t = fi / FPS
        si = max(i for i, s in enumerate(starts) if s <= t)
        lt = t - starts[si]
        p = min(1.0, lt / durs[si])
        kind = SCRIPT[si][2]
        tint = (RED, 0.12 + 0.08 * math.sin(t * 10)) if kind == "hook" and p > 0.55 else None
        img = background(t, tint)
        if kind == "hook":
            sc_hook(img, lt, p)
        elif kind == "f9":
            sc_f9(img, lt, p, icon)
        elif kind == "gpu":
            sc_gpu(img, lt, p)
        elif kind == "light":
            sc_light(img, lt, p)
        elif kind == "gallery":
            sc_gallery(img, lt, p, shot)
        elif kind == "features":
            sc_features(img, lt, p)
        elif kind == "f8":
            sc_f8(img, lt, p)
        elif kind == "free":
            sc_free(img, lt, p)
        else:
            sc_cta(img, lt, p, icon)
        # subtítulo
        cur = [x for x in timed if x[0] <= t and x[1] == si]
        if cur and kind != "cta":
            words = [x[2] for x in timed if x[1] == si]
            idx = len(cur) - 1
            pop = min(1.0, (t - cur[-1][0]) / 0.12)
            caption(img, words, idx, pop)
        # logo chico arriba (marca de agua de la promo)
        if kind != "cta":
            ic = icon.resize((72, 72), Image.LANCZOS)
            img.paste(ic, (60, 90), ic)
            ImageDraw.Draw(img).text((148, 98), "Jugadón", font=font(48), fill=WHITE)
        # zoom "punch" al empezar cada escena + temblor en el gancho
        z = 1 + 0.07 * math.exp(-lt * 9)
        if kind == "hook" and p > 0.55:
            z += 0.02
        if z > 1.001:
            cw, ch = int(W / z), int(H / z)
            dx = dy = 0
            if kind == "hook" and p > 0.55:
                dx, dy = random.randint(-14, 14), random.randint(-14, 14)
            x0 = max(0, min(W - cw, (W - cw) // 2 + dx))
            y0 = max(0, min(H - ch, (H - ch) // 2 + dy))
            img = img.crop((x0, y0, x0 + cw, y0 + ch)).resize((W, H), Image.BILINEAR)
        # destello blanco en el corte
        if si > 0 and lt < 0.08:
            img = Image.blend(img, Image.new("RGB", (W, H), WHITE), 0.55 * (1 - lt / 0.08))
        if only is not None:
            img.save(os.path.join(args.tmp, f"frame_{fi}.png"))
        else:
            proc.stdin.write(img.tobytes())
    if proc:
        proc.stdin.close()
        proc.wait()
        if proc.returncode != 0:
            raise SystemExit("ffmpeg falló")
        print("listo:", args.out)


if __name__ == "__main__":
    main()
