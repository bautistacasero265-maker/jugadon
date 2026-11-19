#!/usr/bin/env python3
"""Video promocional vertical (1080x1920, 60 fps, ~25 s) de Jugadón, para TikTok / Reels / Shorts.

Se genera todo acá, en la nube:
- voz neural argentina (edge-tts, es-AR) con el momento exacto de cada palabra; si falla, Piper.
- imágenes con PIL, música y efectos sintetizados con numpy, armado con ffmpeg.

Lo que lo hace "adictivo" (retención):
- gancho con freno: kill-feed de un ACE, se congela con "scratch" de disco y el sello "NO GRABABA".
- algo cambia cada 1-2 s: cortes con glitch, golpes con onda expansiva, confeti, emojis que saltan.
- zoom que late con el beat, barra de progreso arriba, subtítulos gigantes palabra por palabra
  (exactos con la voz) con las palabras clave en color.
- final que vuelve al principio (loop): la gente lo mira dos veces.
"""
import argparse
import asyncio
import math
import os
import random
import re
import subprocess
import unicodedata
import wave

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(ROOT)
W, H, FPS, SR = 1080, 1920, 60, 48000
BPM = 90

ORANGE = (255, 138, 0)
PINK = (255, 46, 110)
RED = (240, 52, 72)
GREEN = (46, 214, 120)
WHITE = (255, 255, 255)
MUTED = (170, 165, 195)
YELLOW = (255, 214, 0)
BLUE = (64, 150, 255)
BG1 = (12, 11, 18)
BG2 = (36, 14, 46)

BLACK_F = os.path.join(ROOT, "fonts", "InterDisplay-Black.otf")
BOLD_F = os.path.join(ROOT, "fonts", "InterDisplay-Bold.otf")
EMOJI_F = "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf"
_fonts = {}
_emoji = {}


def font(size, black=True):
    k = (size, black)
    if k not in _fonts:
        _fonts[k] = ImageFont.truetype(BLACK_F if black else BOLD_F, max(4, size))
    return _fonts[k]


def emoji(ch, size):
    """Emoji a color (Noto). Se dibuja una vez a 109 px y se escala."""
    if not os.path.exists(EMOJI_F):
        return None
    if ch not in _emoji:
        f = ImageFont.truetype(EMOJI_F, 109)
        im = Image.new("RGBA", (160, 160), (0, 0, 0, 0))
        ImageDraw.Draw(im).text((10, 10), ch, font=f, embedded_color=True)
        _emoji[ch] = im.crop(im.getbbox() or (0, 0, 160, 160))
    e = _emoji[ch]
    s = max(1, int(size))
    return e.resize((s, int(s * e.height / e.width)), Image.LANCZOS)


# (lo que dice la voz, escena). La voz lee el texto tal cual (la voz neural pronuncia bien "F9", "FPS").
SCRIPT = [
    ("¡Pará! ¿Hiciste la jugada de tu vida… y no la grabaste?", "hook"),
    ("Esto se arregla con una sola tecla.", "fix"),
    ("Apretás F9 y se guardan los últimos treinta segundos.", "f9"),
    ("¿Y los FPS? Ni se enteran: lo hace tu placa de video.", "gpu"),
    ("Pesa menos que una foto: ciento ocho kilobytes.", "light"),
    ("Tocás el clip y lo recortás ahí mismo…", "edit"),
    ("…y copiás un link que abre cualquiera.", "link"),
    ("Gratis. Sin cuentas. Sin vueltas.", "free"),
    ("Bajalo ya: el link está en la bio.", "cta"),
]
# palabras que van en color y más grandes en los subtítulos
KEYWORDS = {"pará", "jugada", "grabaste", "tecla", "f9", "treinta", "fps", "placa", "foto", "kilobytes", "recortás", "link", "gratis", "cuentas", "vueltas", "bio", "ya"}


# ---------------------------------------------------------------- voz

def wav_to_array(path):
    with wave.open(path) as w:
        sr, n, ch = w.getframerate(), w.getnframes(), w.getnchannels()
        a = np.frombuffer(w.readframes(n), dtype=np.int16).astype(np.float32) / 32768.0
    if ch > 1:
        a = a.reshape(-1, ch).mean(axis=1)
    if sr != SR:
        a = np.interp(np.arange(0, len(a), sr / SR), np.arange(len(a)), a).astype(np.float32)
    return a


async def _edge(text, voice, rate, pitch, mp3):
    import edge_tts
    words = []
    c = edge_tts.Communicate(text, voice, rate=rate, pitch=pitch, boundary="WordBoundary")
    with open(mp3, "wb") as f:
        async for ch in c.stream():
            if ch["type"] == "audio":
                f.write(ch["data"])
            elif ch["type"] == "WordBoundary":
                words.append((ch["offset"] / 1e7, ch["text"]))
    return words


def _norm(w):
    w = unicodedata.normalize("NFD", w.lower())
    return "".join(c for c in w if c.isalnum())


def _tokens(line):
    return [t for t in re.findall(r"[\wÁÉÍÓÚáéíóúñÑü]+", line) if _norm(t)]


def synth_voice(args):
    """Toda la locución de UNA sola vez (sin cortes entre frases, entonación natural).
    Devuelve (audio float32 48 kHz, [(segundo, palabra, número de frase)]) o None (sin voz, para probar)."""
    if args.no_voice:
        return None
    os.makedirs(args.tmp, exist_ok=True)
    full = " ".join(line for line, _ in SCRIPT)
    wav = os.path.join(args.tmp, "voz.wav")
    bounds = None
    try:
        mp3 = os.path.join(args.tmp, "voz.mp3")
        bounds = asyncio.run(_edge(full, args.edge_voice, args.rate, args.pitch, mp3))
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", mp3, "-ac", "1", "-ar", str(SR), wav], check=True)
    except Exception as e:  # sin internet o servicio caído: Piper (más robótica, pero anda offline)
        if not args.voice:
            raise
        print("edge-tts no anduvo, uso Piper:", e)
        subprocess.run([args.piper, "--model", args.voice, "--output_file", wav, "--length_scale", "0.85", "--sentence_silence", "0.05"], input=full.encode(), check=True)
    a = wav_to_array(wav)
    nz = np.where(np.abs(a) > 0.015)[0]
    cut0 = max(0, nz[0] - 900) if len(nz) else 0
    a = a[cut0:(nz[-1] + 4000 if len(nz) else len(a))]
    expected = [(tok, i) for i, (line, _) in enumerate(SCRIPT) for tok in _tokens(line)]
    words = []
    if bounds:
        bounds = [(t - cut0 / SR, w) for t, w in bounds]
        # se empareja cada palabra esperada con la que dijo la voz (por si separa distinto los números)
        j = 0
        for tok, line in expected:
            for k in range(j, min(j + 4, len(bounds))):
                if _norm(bounds[k][1]) == _norm(tok) or _norm(tok).startswith(_norm(bounds[k][1])):
                    words.append((max(0.0, bounds[k][0]), bounds[k][1], line))
                    j = k + 1
                    break
    if len({w[2] for w in words}) < len(SCRIPT):
        # respaldo: repartir por letras
        span = len(a) / SR
        wt = [len(t) + 2 for t, _ in expected]
        acc, words = 0, []
        for (tok, line), k in zip(expected, wt):
            words.append((span * acc / sum(wt), tok, line))
            acc += k
    return a, words


# ---------------------------------------------------------------- audio

def env_exp(n, k):
    return np.exp(-np.arange(n) / SR * k).astype(np.float32)


def noise(n, seed=0):
    return np.random.default_rng(seed).uniform(-1, 1, n).astype(np.float32)


def sweep(f0, f1, dur, curve=30):
    n = int(SR * dur)
    t = np.arange(n) / SR
    f = f1 + (f0 - f1) * np.exp(-t * curve)
    return np.sin(np.cumsum(2 * np.pi * f / SR)).astype(np.float32)


def kick():
    n = int(SR * 0.32)
    return sweep(170, 48, 0.32) * env_exp(n, 9)


def hat(seed=1):
    n = int(SR * 0.045)
    return np.diff(noise(n, seed), prepend=0) * env_exp(n, 75) * 0.32


def clap():
    n = int(SR * 0.22)
    x = noise(n, 2)
    x = x - np.convolve(x, np.ones(10) / 10, mode="same")
    return x * env_exp(n, 20) * 0.6


def onepole(x, a):
    # filtro pasabajos rápido (vectorizado por bloques no hace falta: es poco audio)
    from itertools import accumulate
    return np.fromiter(accumulate(x, lambda acc, v: acc + a * (v - acc)), dtype=np.float32, count=len(x))


def add(buf, x, at, vol=1.0):
    i = int(at * SR)
    if i < 0:
        x = x[-i:]
        i = 0
    if i >= len(buf) or len(x) == 0:
        return
    j = min(len(buf), i + len(x))
    buf[i:j] += x[: j - i] * vol


def whoosh(dur=0.35):
    n = int(SR * dur)
    t = np.arange(n) / n
    y = onepole(noise(n, 3), 0.15) * np.sin(np.pi * t) ** 2
    return y * 1.4


def riser(dur):
    n = int(SR * dur)
    t = np.arange(n) / SR
    f = 180 * (8 ** (t / dur))
    x = np.sin(np.cumsum(2 * np.pi * f / SR)) * 0.12 + onepole(noise(n, 4), 0.3) * 0.25 * (t / dur)
    return (x * (t / dur) ** 1.5).astype(np.float32)


def boom():
    n = int(SR * 1.3)
    return sweep(120, 34, 1.3, 5) * env_exp(n, 2.4) * 0.95


def scratch():
    """Freno de disco: barrido que baja rápido."""
    n = int(SR * 0.45)
    t = np.arange(n) / SR
    f = 900 * np.exp(-t * 7) + 60
    x = np.sign(np.sin(np.cumsum(2 * np.pi * f / SR))) * 0.3 + noise(n, 7) * 0.2
    return onepole(x.astype(np.float32), 0.25) * env_exp(n, 4) * 1.2


def ding():
    a = np.sin(2 * np.pi * 880 * np.arange(int(SR * 0.07)) / SR) * env_exp(int(SR * 0.07), 8)
    b = np.sin(2 * np.pi * 1318.5 * np.arange(int(SR * 0.25)) / SR) * env_exp(int(SR * 0.25), 9)
    return np.concatenate([a, b]).astype(np.float32) * 0.55


def pop():
    n = int(SR * 0.08)
    return sweep(300, 900, 0.08, 40) * env_exp(n, 40) * 0.5


def click():
    n = int(SR * 0.03)
    return noise(n, 5) * env_exp(n, 140) * 0.7


def rhodes(freq, dur, vol=1.0):
    """Piano eléctrico suave (tipo lo-fi): fundamental + armónicos que se apagan + trémolo leve."""
    n = int(SR * dur)
    t = np.arange(n) / SR
    x = np.sin(2 * np.pi * freq * t) + 0.35 * np.sin(2 * np.pi * freq * 2 * t) * np.exp(-t * 6) + 0.12 * np.sin(2 * np.pi * freq * 3 * t) * np.exp(-t * 10)
    x *= (1 - np.exp(-t * 200)) * np.exp(-t * 1.1) * (1 + 0.15 * np.sin(2 * np.pi * 4.5 * t))
    return (x * vol).astype(np.float32)


def bell(freq, dur):
    n = int(SR * dur)
    t = np.arange(n) / SR
    x = np.sin(2 * np.pi * freq * t) + 0.4 * np.sin(2 * np.pi * freq * 3.01 * t) * np.exp(-t * 8)
    return (x * (1 - np.exp(-t * 300)) * np.exp(-t * 4)).astype(np.float32)


def soft_snare(seed):
    n = int(SR * 0.25)
    x = onepole(noise(n, seed), 0.35) * env_exp(n, 16) * 0.7 + sweep(240, 180, 0.25, 20) * env_exp(n, 25) * 0.4
    return x.astype(np.float32)


def music(total, drop_at, beat):
    """Beat chill (lo-fi) de 90 BPM en estéreo, para que quede de fondo y resalte la voz:
    acordes de piano eléctrico, melodía pegadiza de campanita, batería relajada con swing,
    bajo grave y crujido de vinilo. Antes del drop: solo piano y vinilo (filtrados), después entra todo."""
    n = int(total * SR) + SR
    L = np.zeros(n, np.float32)
    R = np.zeros(n, np.float32)
    # Am7, Fmaj7, Cmaj7, G6
    chords = [(220.0, 261.6, 329.6, 392.0), (174.6, 220.0, 261.6, 329.6), (261.6, 329.6, 392.0, 493.9), (196.0, 246.9, 293.7, 329.6)]
    roots = [55.0, 43.65, 65.41, 49.0]
    hook = [659.3, 784.0, 880.0, 784.0, 659.3, 587.3, 659.3, 0]  # motivo de la melodía (corcheas)
    swing = beat * 0.12
    k = kick() * 0.8
    t, i = 0.0, 0
    while t < total:
        bar = (i // 4) % 4
        pos = i % 4
        after = t >= drop_at
        ch = chords[bar]
        if pos == 0 or pos == 2:
            # acorde: notas apenas escalonadas (como tocado a mano), abierto en estéreo
            for j, f in enumerate(ch):
                r = rhodes(f, beat * 2.2, 0.11 if pos == 0 else 0.07)
                add(L, r, t + j * 0.012, 1.0 if j % 2 == 0 else 0.6)
                add(R, r, t + j * 0.012, 0.6 if j % 2 == 0 else 1.0)
        if after:
            # batería relajada: bombo en 1 y "y del 2", caja suave en 2 y 4, hi-hats con swing
            if pos == 0:
                add(L, k, t, 1.0)
                add(R, k, t, 1.0)
            if pos == 1:
                add(L, k, t + beat / 2, 0.7)
                add(R, k, t + beat / 2, 0.7)
            if pos in (1, 3):
                sn = soft_snare(i)
                add(L, sn, t, 0.75)
                add(R, sn, t + 0.006, 0.75)
            add(L, hat(i) * 0.6, t, 0.5)
            add(R, hat(i + 77) * 0.6, t + beat / 2 + swing, 0.55)
            # bajo grave redondo
            if pos in (0, 1):
                m = int(SR * beat * 0.9)
                tt = np.arange(m) / SR
                b = np.sin(2 * np.pi * roots[bar] * tt) * (1 - np.exp(-tt * 60)) * np.exp(-tt * 1.5)
                add(L, b.astype(np.float32), t + (0 if pos == 0 else beat / 2), 0.4)
                add(R, b.astype(np.float32), t + (0 if pos == 0 else beat / 2), 0.4)
            # melodía pegadiza (cada dos compases, en corcheas)
            if (i // 4) % 2 == 0:
                for e in range(2):
                    f = hook[(pos * 2 + e) % len(hook)]
                    if f:
                        te = t + e * beat / 2 + (swing if e else 0)
                        bl = bell(f, beat * 0.9)
                        add(L, bl, te, 0.07)
                        add(R, bl, te + 0.01, 0.09)
        t += beat
        i += 1
    # vinilo: siseo muy bajo y crujidos sueltos
    rng = np.random.default_rng(9)
    hiss = onepole(noise(n, 11), 0.5) * 0.012
    crack = np.zeros(n, np.float32)
    for p in rng.integers(0, n - 200, int(total * 7)):
        crack[p:p + 60] += rng.uniform(0.02, 0.06) * np.exp(-np.arange(60) / 8)
    L += hiss + crack
    R += hiss + np.roll(crack, 300)
    # cálido: pasabajos suave; antes del drop, más cerrado (como escuchado de lejos)
    d = int(drop_at * SR)
    for X in (L, R):
        X[:d] = onepole(X[:d], 0.12)
        X[d:] = onepole(X[d:], 0.55)
    return np.stack([L, R], axis=1)[: int(total * SR)]


def save_wav(path, x):
    x = np.clip(x, -1, 1)
    ch = 1 if x.ndim == 1 else x.shape[1]
    with wave.open(path, "wb") as w:
        w.setnchannels(ch)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes((x * 32767).astype("<i2").tobytes())


# ---------------------------------------------------------------- dibujo

def ease_out(x):
    x = max(0.0, min(1.0, x))
    return 1 - (1 - x) ** 3


def ease_back(x):
    x = max(0.0, min(1.0, x))
    c = 2.2
    return 1 + (c + 1) * (x - 1) ** 3 + c * (x - 1) ** 2


def gradient_bg():
    y = np.linspace(0, 1, H)[:, None, None]
    g = np.array(BG1, np.float32) * (1 - y) + np.array(BG2, np.float32) * y
    return Image.fromarray(np.repeat(g, W, axis=1).astype(np.uint8), "RGB")


def glow(color, r):
    s = r * 2
    im = Image.new("RGBA", (s, s), color + (0,))
    ImageDraw.Draw(im).ellipse((r // 2, r // 2, s - r // 2, s - r // 2), fill=color + (150,))
    return im.filter(ImageFilter.GaussianBlur(r // 3))


BG = GLOWS = None


def background(t, hue_shift=0):
    im = BG.copy()
    for k, (g, sp) in enumerate(GLOWS):
        x = int(W / 2 + math.sin(t * sp + k * 2.1 + hue_shift) * 420 - g.width / 2)
        y = int(H / 2 + math.cos(t * sp * 0.7 + k * 1.3) * 700 - g.height / 2)
        im.paste(g, (x, y), g)
    return im


def tc(d, y, s, size, fill=WHITE, black=True, stroke=0):
    f = font(size, black)
    w = d.textlength(s, font=f)
    d.text(((W - w) / 2, y), s, font=f, fill=fill, stroke_width=stroke, stroke_fill=(0, 0, 0))


def rounded(d, box, r, fill, outline=None, width=0):
    d.rounded_rectangle(box, r, fill=fill, outline=outline, width=width)


def grad_text(img, y, s, size, c1=ORANGE, c2=PINK, x=None):
    f = font(size)
    w = int(ImageDraw.Draw(img).textlength(s, font=f))
    h = f.getbbox(s)[3] + 20
    mask = Image.new("L", (w + 40, h + 20), 0)
    ImageDraw.Draw(mask).text((20, 10), s, font=f, fill=255)
    xs = np.linspace(0, 1, w + 40)[None, :, None]
    grad = (np.array(c1) * (1 - xs) + np.array(c2) * xs).astype(np.uint8)
    gi = Image.fromarray(np.repeat(grad, h + 20, axis=0), "RGB")
    x0 = (W - w - 40) // 2 if x is None else x
    img.paste((0, 0, 0), (x0 + 6, int(y) + 10), mask.filter(ImageFilter.GaussianBlur(10)))
    img.paste(gi, (x0, int(y)), mask)


def paste_center(img, sub, cx, cy, scale=1.0, rot=0.0):
    if sub is None or scale <= 0.01:
        return
    s = sub.resize((max(1, int(sub.width * scale)), max(1, int(sub.height * scale))), Image.LANCZOS)
    if rot:
        s = s.rotate(rot, expand=True, resample=Image.BICUBIC)
    img.paste(s, (int(cx - s.width / 2), int(cy - s.height / 2)), s)


def sticker(img, ch, cx, cy, t0, t, size=170, rot=0):
    """Emoji que aparece con rebote."""
    if t < t0:
        return
    k = ease_back((t - t0) / 0.25)
    wob = math.sin((t - t0) * 9) * 6 * math.exp(-(t - t0) * 2)
    paste_center(img, emoji(ch, size), cx, cy, k, rot + wob)


def keycap(img, cx, cy, label, press, size=320, color=ORANGE):
    d = ImageDraw.Draw(img)
    off = int(28 * press)
    x0, y0 = cx - size // 2, cy - size // 2
    rounded(d, (x0, y0 + 44, x0 + size, y0 + size + 44), 52, tuple(int(c * 0.35) for c in color))
    rounded(d, (x0, y0 + off, x0 + size, y0 + size + off), 52, color)
    rounded(d, (x0 + 24, y0 + 20 + off, x0 + size - 24, y0 + size - 54 + off), 40, tuple(min(255, c + 35) for c in color))
    f = font(int(size * 0.42))
    w = d.textlength(label, font=f)
    d.text((cx - w / 2, y0 + size * 0.2 + off), label, font=f, fill=WHITE)


def toast(img, y, title, sub, icon, accent=GREEN):
    d = ImageDraw.Draw(img)
    x0, x1 = 80, W - 80
    sh = Image.new("RGBA", (x1 - x0 + 80, 280), (0, 0, 0, 0))
    ImageDraw.Draw(sh).rounded_rectangle((40, 40, x1 - x0 + 40, 230), 44, fill=(0, 0, 0, 170))
    img.paste(sh.filter(ImageFilter.GaussianBlur(18)), (x0 - 40, int(y) - 26), sh.filter(ImageFilter.GaussianBlur(18)))
    rounded(d, (x0, y, x1, y + 190), 44, (38, 33, 54), outline=(90, 80, 125), width=3)
    ic = icon.resize((116, 116), Image.LANCZOS)
    img.paste(ic, (x0 + 34, int(y) + 37), ic)
    d.text((x0 + 176, y + 36), title, font=font(56), fill=WHITE)
    d.text((x0 + 178, y + 106), sub, font=font(38, False), fill=MUTED)
    cx, cy = x1 - 92, y + 95
    d.ellipse((cx - 46, cy - 46, cx + 46, cy + 46), fill=accent)
    d.line((cx - 21, cy + 2, cx - 4, cy + 19, cx + 23, cy - 17), fill=WHITE, width=12, joint="curve")


def chip(d, cx, y, s, size=46, fill=(44, 38, 62), color=WHITE):
    f = font(size, False)
    w = d.textlength(s, font=f)
    rounded(d, (cx - w / 2 - 36, y, cx + w / 2 + 36, y + size + 42), (size + 42) // 2, fill)
    d.text((cx - w / 2, y + 17), s, font=f, fill=color)


# ---------------------------------------------------------------- efectos de pantalla

class FX:
    """Golpes (temblor + onda expansiva), confeti y destellos, todos con su momento."""

    def __init__(self):
        self.impacts = []  # (t, fuerza, cx, cy)
        self.bursts = []  # (t, cx, cy, partículas)

    def impact(self, t, strength=1.0, cx=W // 2, cy=H // 2):
        self.impacts.append((t, strength, cx, cy))

    def confetti(self, t, cx, cy, n=70, seed=0):
        rng = random.Random(seed)
        parts = []
        for _ in range(n):
            a = rng.uniform(0, 2 * math.pi)
            v = rng.uniform(900, 2200)
            parts.append((math.cos(a) * v, math.sin(a) * v - 900, rng.choice([ORANGE, PINK, YELLOW, BLUE, GREEN, WHITE]), rng.uniform(14, 30), rng.uniform(0, 6)))
        self.bursts.append((t, cx, cy, parts))

    def shake(self, t):
        dx = dy = 0.0
        for t0, s, _, _ in self.impacts:
            dt = t - t0
            if 0 <= dt < 0.4:
                a = 34 * s * math.exp(-dt * 10)
                dx += a * math.sin(dt * 90)
                dy += a * math.cos(dt * 77)
        return int(dx), int(dy)

    def draw(self, img, t):
        d = ImageDraw.Draw(img)
        for t0, s, cx, cy in self.impacts:
            dt = t - t0
            if 0 <= dt < 0.45:
                r = 80 + dt * 2600 * s
                wd = max(2, int(26 * (1 - dt / 0.45)))
                d.ellipse((cx - r, cy - r, cx + r, cy + r), outline=WHITE, width=wd)
        for t0, cx, cy, parts in self.bursts:
            dt = t - t0
            if 0 <= dt < 1.6:
                for vx, vy, col, sz, spin in parts:
                    x = cx + vx * dt * 0.6
                    y = cy + vy * dt * 0.6 + 1300 * dt * dt
                    if 0 <= x < W and 0 <= y < H:
                        a = spin + dt * 10
                        w_ = sz * abs(math.cos(a))
                        d.rectangle((x - w_ / 2, y - sz / 2, x + w_ / 2 + 2, y + sz / 2), fill=col)


def glitch(img, amount, seed):
    """Corte con glitch: canales corridos y franjas desplazadas."""
    rng = random.Random(seed)
    r, g, b = img.split()
    off = int(30 * amount)
    r = ImageChops.offset(r, -off, 0)
    b = ImageChops.offset(b, off, 0)
    out = Image.merge("RGB", (r, g, b))
    for _ in range(int(8 * amount)):
        y = rng.randint(0, H - 80)
        h = rng.randint(20, 120)
        dx = rng.randint(-120, 120)
        band = out.crop((0, y, W, y + h))
        out.paste(ImageChops.offset(band, dx, 0), (0, y))
    return out


# ---------------------------------------------------------------- escenas

def sc_hook(img, t, dur, ctx):
    d = ImageDraw.Draw(img)
    freeze = dur * 0.58
    tt = min(t, freeze)
    # kill feed de un ACE que se va armando
    names = ["xXSniperXx", "Pepe_99", "ElTanque", "NoobMaster", "ElUltimo"]
    for i, n in enumerate(names):
        t0 = 0.12 + i * 0.22
        if tt < t0:
            continue
        k = ease_out((tt - t0) / 0.15)
        y = 300 + i * 112
        x1 = W - 70
        x0 = x1 - 760 + int(300 * (1 - k))
        rounded(d, (x0, y, x1, y + 92), 18, (20, 18, 28), outline=RED if i == len(names) - 1 else (70, 60, 90), width=4)
        d.text((x0 + 28, y + 20), "VOS", font=font(46), fill=ORANGE)
        # mira
        cx, cy = x0 + 190, y + 46
        d.ellipse((cx - 22, cy - 22, cx + 22, cy + 22), outline=WHITE, width=5)
        d.line((cx - 34, cy, cx + 34, cy), fill=WHITE, width=5)
        d.line((cx, cy - 34, cx, cy + 34), fill=WHITE, width=5)
        paste_center(img, emoji("💀", 54), cx + 76, cy)
        d.text((cx + 120, y + 22), n, font=font(44, False), fill=WHITE)
    kills = sum(1 for i in range(5) if tt >= 0.12 + i * 0.22)
    if kills:
        s = ease_back((tt - (0.12 + (kills - 1) * 0.22)) / 0.2)
        label = "ACE!" if kills == 5 else f"x{kills}"
        f = int(220 * min(1.15, s))
        grad_text(img, 940, label, f, YELLOW, ORANGE)
    if t >= freeze:
        # se congela: blanco y negro, sello rojo y llanto
        g = img.convert("L").convert("RGB")
        img.paste(Image.blend(g, Image.new("RGB", (W, H), (60, 0, 10)), 0.35))
        k = ease_back((t - freeze) / 0.18)
        stamp = Image.new("RGBA", (940, 270), (0, 0, 0, 0))
        sd = ImageDraw.Draw(stamp)
        sd.rounded_rectangle((10, 10, 930, 260), 30, outline=RED + (255,), width=18)
        f = font(138)
        w = sd.textlength("NO GRABABA", font=f)
        sd.text(((940 - w) / 2, 50), "NO GRABABA", font=f, fill=RED + (255,))
        paste_center(img, stamp, W / 2, 1120, 2.4 - 1.4 * k, -9)
        sticker(img, "😭", W - 200, 1360, freeze + 0.15, t, 210, 10)


def sc_fix(img, t, dur, ctx):
    d = ImageDraw.Draw(img)
    # la tecla cae desde arriba y golpea
    land = 0.32
    y = -400 + (980 + 400) * ease_out(t / land) if t < land else 980
    keycap(img, W // 2, int(y), "F9", 1.0 if land <= t < land + 0.12 else 0.0, size=420)
    if t > land:
        s = ease_back((t - land) / 0.25)
        tc(d, 1290, "1 TECLA", int(150 * min(1.1, s)), YELLOW, stroke=10)
    sticker(img, "⚡", 230, 640, land + 0.1, t, 190, -12)
    sticker(img, "🎮", W - 230, 700, land + 0.25, t, 190, 12)


def sc_f9(img, t, dur, ctx):
    d = ImageDraw.Draw(img)
    # rebobinado: reloj que vuelve 30 s
    k = ease_out(t / 0.7)
    cx, cy, r = W // 2, 760, 230
    d.ellipse((cx - r, cy - r, cx + r, cy + r), outline=(80, 70, 110), width=16)
    d.arc((cx - r, cy - r, cx + r, cy + r), -90 - 180 * k, -90, fill=ORANGE, width=16)
    ang = math.radians(-90 - 180 * k)
    d.line((cx, cy, cx + math.cos(ang) * (r - 40), cy + math.sin(ang) * (r - 40)), fill=WHITE, width=14)
    d.ellipse((cx - 18, cy - 18, cx + 18, cy + 18), fill=WHITE)
    tc(d, 1030, f"-{int(30 * k)} s", 130, ORANGE)
    if t > 0.75:
        yy = 230 - 420 * (1 - ease_out((t - 0.75) / 0.3))
        toast(img, yy, "Clip guardado", "los últimos 30 segundos", ctx["icon"])
    sticker(img, "🔥", W - 190, 560, 0.9, t, 200, 14)


def gauge(img, cx, cy, r, value, vmax, t):
    d = ImageDraw.Draw(img)
    d.arc((cx - r, cy - r, cx + r, cy + r), 150, 390, fill=(60, 54, 84), width=40)
    frac = value / vmax
    d.arc((cx - r, cy - r, cx + r, cy + r), 150, 150 + 240 * frac, fill=GREEN, width=40)
    a = math.radians(150 + 240 * frac + math.sin(t * 30) * 0.6)
    d.line((cx, cy, cx + math.cos(a) * (r - 60), cy + math.sin(a) * (r - 60)), fill=WHITE, width=16)
    d.ellipse((cx - 26, cy - 26, cx + 26, cy + 26), fill=WHITE)


def sc_gpu(img, t, dur, ctx):
    d = ImageDraw.Draw(img)
    v = 144 * ease_out(t / 0.5)
    gauge(img, W // 2, 820, 330, v, 165, t)
    grad_text(img, 880, f"{int(v)}", 210, GREEN, (150, 255, 200))
    tc(d, 1110, "FPS", 90, GREEN)
    if t > 0.6:
        chip(d, W // 2, 1250, "la graba tu placa de video", 48, fill=(30, 70, 50), color=(170, 255, 200))
    sticker(img, "😎", W - 200, 420, 0.5, t, 200, 10)


def sc_light(img, t, dur, ctx):
    d = ImageDraw.Draw(img)
    # carrera de barras: una foto del celular contra Jugadón
    rows = [("📸", "una foto del celu", "3 MB", 1.0, (110, 100, 140)), ("🎮", "Jugadón en reposo", "108 KB", 0.036, ORANGE)]
    for i, (e, label, val, frac, col) in enumerate(rows):
        y = 520 + i * 330
        t0 = 0.1 + i * 0.45
        if t < t0:
            continue
        k = ease_out((t - t0) / 0.5)
        paste_center(img, emoji(e, 110), 150, y + 60)
        d.text((240, y), label, font=font(56), fill=WHITE)
        bw = max(30, int((W - 340) * frac * k))
        rounded(d, (240, y + 92, 240 + bw, y + 160), 34, col)
        d.text((240 + bw + 24 if frac < 0.5 else 270, y + 98), val, font=font(54), fill=WHITE if frac >= 0.5 else ORANGE)
    if t > 1.1:
        s = ease_back((t - 1.1) / 0.25)
        tc(d, 1230, "MENOS QUE UNA FOTO", int(84 * min(1.1, s)), YELLOW, stroke=8)
        sticker(img, "🤯", W - 150, 470, 1.25, t, 160, 12)


def cursor(img, x, y, down):
    pts = [(x, y), (x, y + 70), (x + 18, y + 54), (x + 32, y + 84), (x + 46, y + 77), (x + 32, y + 48), (x + 56, y + 48)]
    d = ImageDraw.Draw(img)
    d.polygon(pts, fill=WHITE, outline=(0, 0, 0))
    if down:
        d.ellipse((x - 40, y - 40, x + 40, y + 40), outline=ORANGE, width=8)


def sc_edit(img, t, dur, ctx):
    shot = ctx["shot"]
    d = ImageDraw.Draw(img)
    # la ventana real de la app, el mouse toca un clip y aparece el editor con las manijas
    sw = 1000
    sh = int(sw * shot.height / shot.width)
    im = shot.resize((sw, sh), Image.LANCZOS)
    x, y = (W - sw) // 2, 340
    mask = Image.new("L", (sw, sh), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, sw - 1, sh - 1), 30, fill=255)
    rounded(d, (x - 10, y - 10, x + sw + 10, y + sh + 10), 38, ORANGE)
    img.paste(im, (x, y), mask)
    # mouse hacia la segunda tarjeta
    tx, ty = x + int(sw * 0.62), y + int(sh * 0.30)
    k = ease_out(t / 0.5)
    cursor(img, int(W - 120 + (tx - (W - 120)) * k), int(y + sh + 120 + (ty - (y + sh + 120)) * k), 0.5 < t < 0.7)
    if t > 0.65:
        # editor: barra con manijas que se cierran
        e = ease_out((t - 0.65) / 0.6)
        py = y + sh + 70
        rounded(d, (60, py, W - 60, py + 270), 40, (30, 27, 42), outline=(90, 80, 125), width=3)
        d.text((100, py + 30), "✂  Recortar", font=font(54), fill=WHITE)
        bx0, bx1 = 110, W - 110
        rounded(d, (bx0, py + 140, bx1, py + 160), 10, (60, 54, 84))
        a = bx0 + int(260 * e)
        b = bx1 - int(220 * e)
        rounded(d, (a, py + 132, b, py + 168), 18, ORANGE)
        for hx in (a, b):
            rounded(d, (hx - 14, py + 112, hx + 14, py + 188), 10, YELLOW)
        d.text((100, py + 200), f"queda: 0:{int(30 - 15 * e):02d}", font=font(40, False), fill=MUTED)


def sc_link(img, t, dur, ctx):
    d = ImageDraw.Draw(img)
    toast(img, 200 - 360 * (1 - ease_out(t / 0.3)), "Link copiado", "lo abre cualquiera, sin la app", ctx["icon"], BLUE)
    # chat que reacciona
    bubbles = [(0.35, True, "litter.catbox.moe/jugada.mp4"), (0.75, False, "QUÉ JUGADAAA"), (1.05, False, "sos un crack")]
    for i, (t0, mine, msg) in enumerate(bubbles):
        if t < t0:
            continue
        k = ease_back((t - t0) / 0.22)
        f = font(50, not mine)
        w = d.textlength(msg, font=f) + 80
        y = 560 + i * 200
        x0 = W - 80 - w if mine else 80
        sub = Image.new("RGBA", (int(w), 150), (0, 0, 0, 0))
        sd = ImageDraw.Draw(sub)
        sd.rounded_rectangle((0, 0, w - 1, 149), 44, fill=(0, 132, 255, 255) if mine else (52, 46, 72, 255))
        sd.text((40, 44), msg, font=f, fill=WHITE)
        paste_center(img, sub, x0 + w / 2, y + 75, k)
    sticker(img, "🔥", 760, 860, 0.85, t, 120)
    sticker(img, "😂", 600, 1060, 1.15, t, 120)


def sc_free(img, t, dur, ctx):
    words = ctx["words"]
    # cada palabra fuerte cae en su momento exacto de la voz
    marks = [w[0] for w in words if w[1].lower().strip(".,!¡¿?…") in ("gratis", "cuentas", "vueltas")] or [0.0, dur * 0.35, dur * 0.7]
    labels = [("GRATIS", YELLOW, ORANGE, 230), ("SIN CUENTAS", WHITE, (200, 200, 220), 120), ("SIN VUELTAS", PINK, ORANGE, 120)]
    for i, (lab, c1, c2, size) in enumerate(labels):
        t0 = marks[min(i, len(marks) - 1)] - (0.15 if i else 0.0)
        if t < t0:
            continue
        s = ease_back((t - t0) / 0.2)
        grad_text(img, 420 + i * 300 + (0 if i == 0 else 60), lab, int(size * min(1.12, s)), c1, c2)
    sticker(img, "💸", 200, 300, 0.1, t, 160, -14)


def sc_cta(img, t, dur, ctx):
    d = ImageDraw.Draw(img)
    icon = ctx["icon"]
    s = ease_back(t / 0.35)
    paste_center(img, icon, W // 2, 560, 0.75 * min(1.1, s), math.sin(t * 3) * 4)
    if t > 0.2:
        grad_text(img, 820, "JUGADÓN", 180)
    if t > 0.4:
        pulse = 1 + 0.07 * math.sin(t * 10)
        bw, bh = int(780 * pulse), int(160 * pulse)
        top = 1080
        sub = Image.new("RGBA", (bw, bh), (0, 0, 0, 0))
        ImageDraw.Draw(sub).rounded_rectangle((0, 0, bw - 1, bh - 1), bh // 2, fill=ORANGE + (255,))
        img.paste(sub, ((W - bw) // 2, top), sub)
        tc(d, top + bh / 2 - 44, "DESCARGALO GRATIS", int(66 * pulse))
        ay = 1300 + 26 * abs(math.sin(t * 6))
        d.polygon([(W / 2 - 60, ay), (W / 2 + 60, ay), (W / 2, ay + 74)], fill=WHITE)
        tc(d, 1400, "link en la bio", 54, MUTED, black=False)
    sticker(img, "👇", W - 210, 1330, 0.5, t, 150)


SCENES = {"hook": sc_hook, "fix": sc_fix, "f9": sc_f9, "gpu": sc_gpu, "light": sc_light, "edit": sc_edit, "link": sc_link, "free": sc_free, "cta": sc_cta}


# ---------------------------------------------------------------- subtítulos

def caption(img, words, t):
    """Grupos de hasta 3 palabras; la que se dice ahora rebota y va en color (las clave, más grandes)."""
    cur = [i for i, (wt, _) in enumerate(words) if wt <= t]
    if not cur:
        return
    idx = cur[-1]
    g0 = (idx // 3) * 3
    group = words[g0:g0 + 3]
    d = ImageDraw.Draw(img)
    items = []
    for j, (wt, w_) in enumerate(group):
        clean = w_.strip(".,!¡¿?…:").lower()
        key = clean in KEYWORDS
        size = 104 if key else 90
        if g0 + j == idx:
            size = int(size * (1 + 0.22 * max(0.0, 1 - (t - wt) / 0.12)))
        col = (YELLOW if (g0 + j) % 2 == 0 else ORANGE) if g0 + j == idx else (PINK if key and g0 + j < idx else WHITE)
        if key and g0 + j == idx:
            col = YELLOW
        items.append((w_.upper().strip(), size, col))
    gap = 24
    widths = [d.textlength(s, font=font(z)) for s, z, _ in items]
    total = sum(widths) + gap * (len(items) - 1)
    if total > W - 80:
        k = (W - 80) / total
        items = [(s, int(z * k), c) for s, z, c in items]
        widths = [d.textlength(s, font=font(z)) for s, z, _ in items]
        total = sum(widths) + gap * (len(items) - 1)
    x = (W - total) / 2
    for (s, z, c), wd in zip(items, widths):
        d.text((x, 1530 - z / 2), s, font=font(z), fill=c, stroke_width=10, stroke_fill=(0, 0, 0))
        x += wd + gap


# ---------------------------------------------------------------- armado

def whip(img, amount, direction=1):
    """Transición de barrido: la escena nueva entra deslizándose con desenfoque de movimiento."""
    off = int(W * 0.22 * amount) * direction
    if off == 0:
        return img
    base = ImageChops.offset(img, off, 0)
    acc = np.asarray(base, np.float32)
    for k in (0.33, 0.66):
        acc += np.asarray(ImageChops.offset(img, int(off * (1 + k)), 0), np.float32)
    return Image.fromarray((acc / 3).astype(np.uint8))


def main():
    global BG, GLOWS
    ap = argparse.ArgumentParser()
    ap.add_argument("--no-voice", action="store_true", help="sin voz (para probar el armado)")
    ap.add_argument("--edge-voice", default="es-AR-TomasNeural")
    ap.add_argument("--rate", default="+14%")
    ap.add_argument("--pitch", default="+3Hz")
    ap.add_argument("--voice", help="modelo .onnx de Piper (respaldo si edge-tts no anda)")
    ap.add_argument("--piper", default="piper")
    ap.add_argument("--out", default=os.path.join(REPO, "promo", "out", "Jugadon-promo.mp4"))
    ap.add_argument("--tmp", default="/tmp/jugadon_promo")
    ap.add_argument("--frames", help="solo guardar estos cuadros como png (ej: 10,80,200)")
    args = ap.parse_args()
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    os.makedirs(args.tmp, exist_ok=True)
    random.seed(1)

    BG = gradient_bg()
    GLOWS = [(glow(ORANGE, 430), 0.5), (glow(PINK, 500), 0.4), (glow((120, 60, 255), 380), 0.3)]
    icon = Image.open(os.path.join(REPO, "assets", "icon.png")).convert("RGBA")
    shot = Image.open(os.path.join(REPO, "assets", "captura_ventana.png")).convert("RGB")

    voice = synth_voice(args)
    if voice:
        vaudio, words = voice
        vlen = len(vaudio) / SR
    else:
        # sin voz: tiempos inventados por cantidad de letras
        vaudio, words, acc = None, [], 0.0
        for i, (line, _) in enumerate(SCRIPT):
            for tok in _tokens(line):
                words.append((acc, tok, i))
                acc += 0.06 * (len(tok) + 2)
            acc += 0.15
        vlen = acc
    lead = 0.15  # la voz arranca un toque después del primer cuadro
    # cada escena empieza un poquito antes de su primera palabra (el corte "anticipa" lo que se dice)
    starts = [0.0]
    for i in range(1, len(SCRIPT)):
        first = min((w[0] for w in words if w[2] == i), default=starts[-1] + 2.0)
        starts.append(max(starts[-1] + 0.8, first + lead - 0.12))
    total = vlen + lead + 1.7  # el final queda un ratito en pantalla con el botón
    durs = [b_ - a_ for a_, b_ in zip(starts, starts[1:] + [total])]
    words_per = [[(w[0] + lead - starts[i], w[1]) for w in words if w[2] == i] for i in range(len(SCRIPT))]
    beat = 60 / BPM
    print(f"duración total: {total:.1f} s; escenas: " + ", ".join(f"{d:.1f}" for d in durs))

    fx = FX()
    hook_freeze = durs[0] * 0.58
    drop = starts[1]
    fx.impact(hook_freeze, 1.3)
    fx.impact(starts[1] + 0.32, 1.4, W // 2, 980)
    fx.confetti(starts[1] + 0.32, W // 2, 980, 80, 1)
    fx.confetti(starts[2] + 0.8, W // 2, 330, 70, 2)
    fx.impact(starts[4] + 1.1, 0.8)
    fx.confetti(starts[6] + 0.8, W // 2, 900, 60, 3)
    for wt, w_ in words_per[7]:
        if _norm(w_) in ("gratis", "cuentas", "vueltas"):
            fx.impact(starts[7] + wt, 1.0)
    fx.confetti(starts[8] + 0.3, W // 2, 560, 90, 4)

    # ---------- audio: tres pistas y mastering con ffmpeg
    mus = music(total, drop, beat)
    sfx = np.zeros(len(mus), np.float32)
    voice_track = np.zeros(len(mus), np.float32)
    if vaudio is not None:
        add(voice_track, vaudio, lead, 1.0)
    for st in starts[1:]:
        add(sfx, whoosh(), st - 0.2, 0.7)
    add(sfx, riser(drop), 0.0, 0.55)
    add(sfx, scratch(), hook_freeze - 0.05, 0.8)
    add(sfx, boom(), starts[1] + 0.32, 1.0)
    add(sfx, click(), starts[1] + 0.32, 1.0)
    add(sfx, ding(), starts[2] + 0.75, 1.0)
    add(sfx, ding(), starts[6] + 0.0, 0.9)
    for t0, st_, _, _ in fx.impacts:
        add(sfx, boom()[: int(SR * 0.5)] * 0.7, t0, st_ * 0.7)
    for i in range(5):
        add(sfx, pop(), 0.12 + i * 0.22, 0.6)
    for t0 in (0.35, 0.75, 1.05):
        add(sfx, pop(), starts[6] + t0, 0.7)
    # en el congelado del gancho la música no se corta: solo baja un poco (sin "parones")
    i0, i1 = int(hook_freeze * SR), int(drop * SR)
    mus[i0:i1] *= 0.55
    vpath, mpath, spath = (os.path.join(args.tmp, n) for n in ("v.wav", "m.wav", "s.wav"))
    save_wav(vpath, voice_track * 0.9)
    save_wav(mpath, mus / max(1.0, float(np.max(np.abs(mus))) / 0.9))
    save_wav(spath, sfx / max(1.0, float(np.max(np.abs(sfx))) / 0.9))
    wav = os.path.join(args.tmp, "mix.wav")
    # voz: limpia (pasaaltos), comprimida y con brillo; la música baja sola cuando habla (sidechain);
    # todo nivelado a -14 LUFS (lo que usan TikTok, Reels y Shorts)
    graph = (
        "[0:a]highpass=f=90,equalizer=f=250:t=q:w=1:g=-3,equalizer=f=3500:t=q:w=1.2:g=5,equalizer=f=9000:t=q:w=1:g=2,"
        "acompressor=threshold=-22dB:ratio=4:attack=4:release=90:makeup=6,aformat=channel_layouts=stereo,asplit=2[v1][v2];"
        "[1:a][v2]sidechaincompress=threshold=0.02:ratio=7:attack=6:release=320:makeup=1[m];"
        "[2:a]aformat=channel_layouts=stereo[s];"
        "[v1][m][s]amix=inputs=3:normalize=0:weights=1.25 0.6 0.95,"
        "acompressor=threshold=-12dB:ratio=2.5:attack=10:release=150,loudnorm=I=-14:TP=-1.5:LRA=8,alimiter=limit=0.84:level=false,aresample=48000"
    )
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", vpath, "-i", mpath, "-i", spath, "-filter_complex", graph, "-ar", "48000", "-ac", "2", wav], check=True)

    # ---------- video (60 cuadros por segundo)
    only = set(int(x) for x in args.frames.split(",")) if args.frames else None
    nframes = int(total * FPS)
    proc = None
    if only is None:
        proc = subprocess.Popen(
            ["ffmpeg", "-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}", "-r", str(FPS), "-i", "-",
             "-i", wav, "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "256k",
             "-shortest", "-movflags", "+faststart", args.out],
            stdin=subprocess.PIPE)
    for fi in range(nframes):
        if only is not None and fi not in only:
            continue
        t = fi / FPS
        si = max(i for i, s_ in enumerate(starts) if s_ <= t)
        lt = t - starts[si]
        kind = SCRIPT[si][1]
        img = background(t, si)
        ctx = {"icon": icon, "shot": shot, "words": words_per[si]}
        SCENES[kind](img, lt, durs[si], ctx)
        fx.draw(img, t)
        if kind != "cta":
            caption(img, words_per[si], lt)
        d = ImageDraw.Draw(img)
        if kind != "cta":
            ic = icon.resize((70, 70), Image.LANCZOS)
            img.paste(ic, (56, 64), ic)
            d.text((142, 72), "Jugadón", font=font(46), fill=WHITE)
        # cámara: siempre en movimiento suave + golpe al empezar la escena + latido con el beat + temblor.
        # Con transformación afín (subpíxel), así no hay saltitos de un píxel.
        z = 1.035 + 0.06 * math.exp(-lt * 8) + 0.008 * math.sin(t * 0.9)
        if t >= drop:
            ph = ((t - drop) % beat) / beat
            z += 0.02 * math.exp(-ph * 7)
        dx, dy = fx.shake(t)
        cx = W / 2 + math.sin(t * 0.7) * 10 + dx
        cy = H / 2 + math.cos(t * 0.55) * 14 + dy
        img = img.transform((W, H), Image.AFFINE, (1 / z, 0, cx - W / 2 / z, 0, 1 / z, cy - H / 2 / z), resample=Image.BILINEAR)
        # entrada de cada escena: barrido con desenfoque (al drop, glitch)
        if si > 0 and lt < 0.18:
            if si == 1:
                img = glitch(img, 1 - lt / 0.18, fi)
            else:
                img = whip(img, 1 - ease_out(lt / 0.18), 1 if si % 2 else -1)
        # barra de progreso arriba (retención)
        d = ImageDraw.Draw(img)
        d.rectangle((0, 0, W, 12), fill=(40, 34, 56))
        pw = int(W * t / total)
        if pw > 2:
            d.rectangle((0, 0, pw, 12), fill=ORANGE)
        # final: glitch que vuelve al principio (loop)
        if t > total - 0.15:
            img = glitch(img, (t - (total - 0.15)) / 0.15, fi)
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
