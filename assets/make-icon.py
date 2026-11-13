"""Booth's app icon: the NoFaceTube hood, with Booth's waveform where Outbox has its envelope.

Writes `assets/booth-icon-1024.png` (the master), then the Android adaptive layers, which Tauri
cannot produce correctly on its own. Run it from the repo root after changing the mark:

    python assets/make-icon.py
    cd apps/native && pnpm tauri icon ../../assets/booth-icon-1024.png
    python assets/make-icon.py --android-only     # redo the layers Tauri overwrote

Sources (outside this repo, from the NoFaceTube brand folder):
  scripts/brand/outbox-icon-1024.png   the sibling app's icon: same crop, rounding and red
  apps/native/public/booth-logo.png    Booth's own logo; the waveform's column heights come from it

Android draws both icon layers on a 108dp canvas but shows only the central 72dp, so `tauri icon`'s
full-bleed foreground gets zoomed and the hood is cut. We place the mark inside the safe zone, let
the launcher's mask do the rounding, and draw the themed (monochrome) icon as the waveform alone.
"""
import sys
import pathlib
import numpy as np
from PIL import Image, ImageFilter

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUTBOX = pathlib.Path(r"C:/Projects/nofacetube/scripts/brand/outbox-icon-1024.png")
LOGO = ROOT / "apps/native/public/booth-logo.png"
MASTER = ROOT / "assets/booth-icon-1024.png"
RES = ROOT / "apps/native/src-tauri/gen/android/app/src/main/res"
DPI = {"mdpi": 108, "hdpi": 162, "xhdpi": 216, "xxhdpi": 324, "xxxhdpi": 432}
N = 1024
GLYPH = (434, 711, 307, 716)        # the envelope's box in the Outbox icon: y0, y1, x0, x1


def waveform():
    """The column heights of the waveform in Booth's 144px logo, 0..1."""
    b = np.asarray(Image.open(LOGO).convert("RGB")).astype(float)
    red = b[:, :, 0] - (b[:, :, 1] + b[:, :, 2]) / 2
    y0, y1 = 52, 76
    base = np.concatenate([red[y0:y0 + 3, :], red[y1 - 3:y1, :]]).mean(0)
    sig = np.clip(red[y0:y1, :] - base[None, :], 0, None)
    peak = sig.max()
    cols = np.nonzero(sig.max(0) > peak * 0.50)[0]      # 0.5 keeps the bars, drops the rim glow
    half = []
    for x in range(cols[0], cols[-1] + 1):
        prof = sig[:, x]
        on = np.nonzero(prof > max(prof.max() * 0.5, peak * 0.14))[0]
        half.append(((on[-1] - on[0] + 1) / 2) if len(on) else 0.0)
    return np.clip(np.array(half) / np.percentile(half, 97), 0, 1.0)


def bars(canvas, half, cx, cy, width, peak, min_half):
    """A mask with one bar per column, tallest in the middle, the way the logo's waveform runs."""
    m = np.zeros((canvas, canvas), np.float32)
    pitch = width / len(half)
    for i, hh in enumerate(half):
        xa = cx - width / 2 + i * pitch
        ia = int(round(xa))
        ib = max(int(round(xa + pitch * 0.60)), ia + 1)
        hp = max(peak * hh, min_half)
        m[int(round(cy - hp)):int(round(cy + hp)), ia:ib] = 1.0
    return m


def taper(n, lo, lo_full, hi_full, hi):
    """A window that is 1 over the middle and eases to 0, so a fill leaves no edge."""
    v = np.zeros(n)
    v[lo_full:hi_full] = 1.0
    v[lo:lo_full] = 0.5 - 0.5 * np.cos(np.linspace(0, np.pi, lo_full - lo))
    v[hi_full:hi] = 0.5 + 0.5 * np.cos(np.linspace(0, np.pi, hi - hi_full))
    return v


def master(half):
    """The 1024 icon: erase the sibling app's glyph out of the visor, draw the waveform there."""
    a = np.asarray(Image.open(OUTBOX).convert("RGB")).astype(float)
    gy0, gy1, gx0, gx1 = GLYPH
    py0, py1 = gy0 - 120, gy1 + 120
    fill = a.copy()
    top = a[py0 - 10:py0, :, :].mean(0)
    bot = a[py1:py1 + 10, :, :].mean(0)
    t = np.linspace(0, 1, py1 - py0)[:, None, None]
    fill[py0:py1, :, :] = top[None] * (1 - t) + bot[None] * t
    m = (taper(N, gy0 - 130, gy0 - 55, gy1 + 55, gy1 + 130)[:, None]
         * taper(N, 212, gx0 - 40, gx1 + 40, 812)[None, :]).astype(np.float32)
    a = a * (1 - m[:, :, None]) + fill * m[:, :, None]

    core = bars(N, half, (gx0 + gx1) / 2, (gy0 + gy1) / 2, width=452, peak=76, min_half=4.0)
    core_img = Image.fromarray((core * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(1.1))
    out = a.copy()
    red = np.array([248.0, 6.0, 6.0])               # the family's red, sampled off the envelope
    for blur, k in ((48, 0.75), (14, 0.85)):
        gm = np.asarray(core_img.filter(ImageFilter.GaussianBlur(blur))).astype(float)[:, :, None] / 255.0
        out = out + red[None, None] * gm * k
    cm = np.asarray(core_img).astype(float)[:, :, None] / 255.0
    out = out * (1 - cm) + np.array([255.0, 40.0, 38.0])[None, None] * cm
    Image.fromarray(np.clip(out, 0, 255).astype(np.uint8)).save(MASTER)
    print("wrote", MASTER)


def android(half):
    icon = Image.open(MASTER).convert("RGBA")
    for dpi, n in DPI.items():
        art = round(n * 72 / 108)                   # the safe zone: 72dp of the 108dp canvas
        layer = Image.new("RGBA", (n, n), (0, 0, 0, 0))
        layer.paste(icon.resize((art, art), Image.LANCZOS), ((n - art) // 2, (n - art) // 2))
        layer.save(RES / f"mipmap-{dpi}" / "ic_launcher_foreground.png")

        m = Image.fromarray((bars(n, half, n / 2, n / 2, n * 0.50, n * 0.105, n * 0.006) * 255).astype(np.uint8))
        if n > 160:
            m = m.filter(ImageFilter.GaussianBlur(0.5))
        mono = Image.new("RGBA", (n, n), (255, 255, 255, 0))
        mono.putalpha(m)
        mono.save(RES / f"mipmap-{dpi}" / "ic_launcher_monochrome.png")

    (RES / "values" / "ic_launcher_background.xml").write_text(
        '<?xml version="1.0" encoding="utf-8"?>\n<resources>\n'
        '  <color name="ic_launcher_background">#000000</color>\n</resources>\n'
    )
    (RES / "mipmap-anydpi-v26" / "ic_launcher.xml").write_text(
        '<?xml version="1.0" encoding="utf-8"?>\n'
        '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
        '  <background android:drawable="@color/ic_launcher_background"/>\n'
        '  <foreground android:drawable="@mipmap/ic_launcher_foreground"/>\n'
        '  <monochrome android:drawable="@mipmap/ic_launcher_monochrome"/>\n'
        '</adaptive-icon>\n'
    )
    print("wrote the Android layers")


if __name__ == "__main__":
    half = waveform()
    if "--android-only" not in sys.argv:
        master(half)
    android(half)
