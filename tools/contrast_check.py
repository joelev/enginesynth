"""Measure the contrast of the key deck from pixels, not from the stylesheet or by eye.

usage: contrast_check.py [base-url] [--out dir]
  base-url defaults to http://127.0.0.1:47623/live/ ; crops go to out.nosync/first-run/contrast/.

For 1440x900 and 1280x720 (rendered at 2x, as on a MacBook) it screenshots the loading screen
(ghost keys) and the running app held on the limiter in neutral (the floor at its liveliest behind
the deck), and samples:
  deck          a pixel of the deck between the number row and the keys under it
  key face      the mean of a patch of each cap clear of its legends
  key edge      the cap's outline, at the middle of its top edge
  key letter    the brightest pixel inside the letter's box (the core of the glyph)
  sub-legend    the brightest pixel inside the small legend's box
  digits        the same for the unlit number keys
  lever         the knob's centre and the brightest pixel of the D and N marks
  gear numeral  its brightest pixel against its window
Ratios are WCAG contrast ratios, (L1 + 0.05) / (L2 + 0.05) on relative luminance. Targets:
letter to face 7, sub-legend to face 4.5,
face to deck 1.25, edge to deck 3, lever marks and knob to deck 4.5, ghost letters to deck 3.
Exits 1 if any target is missed.
"""
import argparse
import glob
import io
import os
import sys

from PIL import Image
from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument('base', nargs='?', default='http://127.0.0.1:47623/live/')
ap.add_argument('--out', default=os.path.join(ROOT, 'out.nosync', 'first-run', 'contrast'))
A = ap.parse_args()
os.makedirs(A.out, exist_ok=True)
DSF = 2
KEYS = ['#btnDown', '#btnUp', '#btnAuto', '#btnBrake', '#btnMode']
RECTS = """() => { const r = (s) => { const e = document.querySelector(s); if (!e) return null; const b = e.getBoundingClientRect(); return [b.left, b.top, b.width, b.height]; };
  const o = {}; for (const s of %s) o[s] = r(s); return o; }"""


def lum(p):
    def lin(c):
        c /= 255
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
    return 0.2126 * lin(p[0]) + 0.7152 * lin(p[1]) + 0.0722 * lin(p[2])


def ratio(a, b):
    hi, lo = max(a, b), min(a, b)
    return (hi + 0.05) / (lo + 0.05)


class Shot:
    def __init__(self, page):
        self.im = Image.open(io.BytesIO(page.screenshot())).convert('RGB')

    def mean(self, x, y, w, h):
        px = [self.im.getpixel((int(i), int(j))) for i in range(int(x * DSF), int((x + w) * DSF)) for j in range(int(y * DSF), int((y + h) * DSF))]
        return sum(lum(p) for p in px) / len(px)

    def brightest(self, x, y, w, h):
        return max(lum(self.im.getpixel((i, j))) for i in range(int(x * DSF), int((x + w) * DSF)) for j in range(int(y * DSF), int((y + h) * DSF)))

    def at(self, x, y):
        return lum(self.im.getpixel((int(x * DSF), int(y * DSF))))

    def crop(self, x, y, w, h, path):
        self.im.crop((int(x * DSF), int(y * DSF), int((x + w) * DSF), int((y + h) * DSF))).save(path)


rows, misses = [], []


def row(size, state, what, value, target):
    ok = value >= target
    rows.append(f'| {size} | {state} | {what} | {value:.2f} | {target:g} | {"ok" if ok else "MISS"} |')
    if not ok:
        misses.append(f'{size} {state} {what}: {value:.2f} < {target:g}')


def measure(page, size, state):
    sels = KEYS + [k + ' b' for k in KEYS] + [k + ' i' for k in KEYS] + ['#kb', '#pedal', '#pedal .num:nth-child(8)', '#gear', '#lever', '#lever .lv-knob', '#lever .lv-d', '#lever .lv-n', '#kLetters', '.k-deck']
    R = page.evaluate(RECTS % repr(sels))
    S = Shot(page)
    nums, letters = R['#pedal'], R['#kLetters']
    # the deck, in the gap between the number row and the keys under it (several points, the lightest of them: the worst case)
    gy = (nums[1] + nums[3] + letters[1]) / 2
    deck = max(S.at(nums[0] + nums[2] * f, gy) for f in (0.15, 0.35, 0.5, 0.65, 0.85))
    dk = R['.k-deck']
    S.crop(dk[0] - 8, dk[1] - 8, dk[2] + 16, min(dk[3], page.viewport_size['height'] - dk[1]) + 8, os.path.join(A.out, f'deck-{state.split(",")[0]}{"-limiter" if "limiter" in state else ""}-{size}-2x.png'))
    if state == 'loading':
        worst = min(ratio(S.brightest(*R[k + ' b']), deck) for k in KEYS)
        row(size, state, 'ghost key letter : deck (dimmest of 5)', worst, 3)
        row(size, state, 'ghost digit : deck', ratio(S.brightest(*R['#pedal .num:nth-child(8)']), deck), 3)
        sp = page.evaluate(RECTS % repr(['#btnStart']))['#btnStart']
        row(size, state, 'space bar face : deck', ratio(S.mean(sp[0] + sp[2] * 0.2, sp[1] + sp[3] * 0.3, 20, 6), deck), 7)
        return
    face, edge, letter, sub = [], [], [], []
    for k in KEYS:
        x, y, w, h = R[k]
        f = S.mean(x + w * 0.62, y + h * 0.4, w * 0.25, h * 0.2)
        face.append(ratio(f, deck))
        edge.append(ratio(max(S.at(x + w * 0.5, y + 0.5), S.at(x + w * 0.5, y + 0.25)), deck))
        letter.append(ratio(S.brightest(*R[k + ' b']), f))
        sub.append(ratio(S.brightest(*R[k + ' i']), f))
    row(size, state, 'key letter : key face (worst of Q E A S D)', min(letter), 7)
    row(size, state, 'sub-legend : key face (worst of 5)', min(sub), 4.5)
    row(size, state, 'key face : deck (worst of 5)', min(face), 1.25)
    row(size, state, 'key edge : deck (worst of 5)', min(edge), 3)
    n = R['#pedal .num:nth-child(8)']   # key 8: unlit while idling
    nf = S.mean(n[0] + 3, n[1] + n[3] * 0.3, 6, n[3] * 0.4)
    if 'limiter' in state:   # full throttle: the whole row is lit (dark digit on the light face)
        darkest = min(lum(S.im.getpixel((i, j))) for i in range(int((n[0] + n[2] * 0.3) * DSF), int((n[0] + n[2] * 0.7) * DSF)) for j in range(int((n[1] + 4) * DSF), int((n[1] + n[3] - 4) * DSF)))
        row(size, state, 'lit number digit : its lit key face', ratio(darkest, nf), 7)
    else:
        row(size, state, 'number key face : deck', ratio(nf, deck), 1.25)
        row(size, state, 'number key edge : deck', ratio(max(S.at(n[0] + n[2] * 0.5, n[1] + 0.5), S.at(n[0] + n[2] * 0.5, n[1] + 0.25)), deck), 3)
        row(size, state, 'number digit : its key face', ratio(S.brightest(n[0] + n[2] * 0.3, n[1] + 4, n[2] * 0.4, n[3] - 8), nf), 7)
    kn = R['#lever .lv-knob']
    row(size, state, 'lever knob : deck', ratio(S.at(kn[0] + kn[2] / 2, kn[1] + kn[3] / 2), deck), 4.5)
    row(size, state, 'lever D mark : deck', ratio(S.brightest(*R['#lever .lv-d']), deck), 4.5)
    row(size, state, 'lever N mark : deck', ratio(S.brightest(*R['#lever .lv-n']), deck), 4.5)
    g = R['#gear']
    row(size, state, 'gear numeral : its window', ratio(S.brightest(*g), S.mean(g[0] + 4, g[1] + 4, 8, 8)), 7)


exe = sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
with sync_playwright() as pw:
    b = pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required', '--use-angle=metal'], **({"executable_path": exe[-1]} if exe else {}))
    for w, h in [(1440, 900), (1280, 720)]:
        ctx = b.new_context(viewport={'width': w, 'height': h}, device_scale_factor=DSF)
        page = ctx.new_page()
        page.goto(A.base + '?engine=v8_cross&orbit=0&firstrun=1')
        page.wait_for_selector('#btnStart:not([disabled])', timeout=30000)
        page.wait_for_timeout(1500)
        measure(page, f'{w}x{h}', 'loading')
        page.click('#btnStart')
        page.wait_for_function('() => __app.started && __app.engine.frame', timeout=20000)
        page.keyboard.press('KeyD')            # neutral: held on the limiter the floor is at its brightest, and the lever's N mark is the lit one
        page.keyboard.down('KeyW')
        page.wait_for_timeout(5000)
        measure(page, f'{w}x{h}', 'running, neutral, limiter')
        page.keyboard.up('KeyW')
        page.keyboard.press('KeyD')
        page.wait_for_timeout(1200)
        measure(page, f'{w}x{h}', 'running, Drive, idle')
        ctx.close()
    b.close()
print('| size | state | pair | measured | target | |')
print('|---|---|---|---|---|---|')
print('\n'.join(rows))
print('\nmisses:', misses or 'none')
sys.exit(1 if misses else 0)
