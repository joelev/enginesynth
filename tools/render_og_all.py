"""Render the link-preview images: live/og/<engine id>.jpg for every engine, and live/og.jpg.

usage: render_og_all.py [engine id ...] [--main-only]
  With no ids, every engine in engines/defs/ORDER and then live/og.jpg (the V8, without an engine name).

Each is 1200x630, drawn by the page itself in offline capture (?capture=1, as render_live_video.py):
the engine is started in Drive, left four seconds, then Space is held; the picture is taken 3.5 s
into that pull (engine glowing, floor lit by the spectrogram). The interface is hidden; the name
"Engine Synthesizer" and the engine's label are set bottom left, large enough for a thumbnail.
Rendered at 2x and scaled down; JPEG quality is lowered from 88 until the file is under 150 KB.
"""
import glob
import io
import os
import socket
import subprocess
import sys
import time

from PIL import Image
from playwright.sync_api import sync_playwright

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from capture_clock import CLOCK

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, 'live', 'og')
os.makedirs(OUT, exist_ok=True)
args = [a for a in sys.argv[1:] if not a.startswith('--')]
ids = args or [l.strip() for l in open(os.path.join(ROOT, 'engines', 'defs', 'ORDER')) if l.strip()]
jobs = ([] if '--main-only' in sys.argv else [(i, os.path.join(OUT, i + '.jpg'), None) for i in ids])
if not args or '--main-only' in sys.argv:
    jobs.append(('v8_cross', os.path.join(ROOT, 'live', 'og.jpg'), 'main'))
W, H, SPF = 1200, 630, 800
TOP = 0.84     # highest a pipe's centre line may reach, in normalised device coordinates (1 is the top edge)
CSS = """body > *:not(#gl):not(#vignette):not(#ogshade):not(#ogcard) { display: none !important; } * { cursor: none !important; }
#ogshade { position: fixed; left: 0; right: 0; bottom: 0; height: 330px; background: linear-gradient(to top, rgba(4,5,10,.92) 0%, rgba(4,5,10,.7) 45%, rgba(4,5,10,0) 100%); }
#ogcard { position: fixed; left: 46px; right: 46px; bottom: 40px; font-family: -apple-system, BlinkMacSystemFont, "Helvetica Neue", "Segoe UI", system-ui, sans-serif; }
#ogcard .k { font-size: 21px; letter-spacing: .34em; text-transform: uppercase; color: #ffb04a; margin-bottom: 14px; }
#ogcard .l { font-size: 52px; font-weight: 650; letter-spacing: -.02em; line-height: 1.06; color: #f4f6fa; text-wrap: balance; }
#ogcard .l.long { font-size: 42px; } #ogcard .l.title { font-size: 76px; letter-spacing: -.028em; }"""
CARD = """([kicker, text, cls]) => { const s = document.createElement('div'); s.id = 'ogshade'; const c = document.createElement('div'); c.id = 'ogcard';
  const k = document.createElement('div'); k.className = 'k'; k.textContent = kicker; const l = document.createElement('div'); l.className = 'l ' + cls;
  // a hyphenated word (i-AVLS, RX-7, two-rotor) never breaks at its hyphen
  text.split(/( )/).forEach((w) => { if (w.includes('-')) { const n = document.createElement('span'); n.style.whiteSpace = 'nowrap'; n.textContent = w; l.append(n); } else l.append(w); });
  c.append(k, l); document.body.append(s, c); }"""

s = socket.socket(); s.bind(('127.0.0.1', 0)); port = s.getsockname()[1]; s.close()
srv = subprocess.Popen([sys.executable, os.path.join(ROOT, 'tools', 'serve.py'), str(port), ROOT], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(0.8)
bad = []
try:
    exe = sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
    with sync_playwright() as pw:
        b = pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required', '--use-angle=metal'], **({"executable_path": exe[-1]} if exe else {}))
        for eng, path, kind in jobs:
            ctx = b.new_context(viewport={'width': W, 'height': H}, device_scale_factor=2)
            page = ctx.new_page()
            errs = []
            page.on('pageerror', lambda e: errs.append(str(e)[:200]))
            page.add_init_script(CLOCK)
            try:
                page.goto(f'http://127.0.0.1:{port}/live/?capture=1&engine={eng}&firstrun=0&preserve=1&quality=heavy')
                page.wait_for_selector('#btnStart:not([disabled])', timeout=60000)
                page.click('#btnStart')
                page.wait_for_function('() => __app.started && __app.engine && __app.engine.step', timeout=30000, polling=100)   # not on animation frames: the page's are held
                label = page.inner_text('#elabel')
                page.add_style_tag(content=CSS)
                if kind == 'main':
                    page.evaluate(CARD, ['Sound from simulated pressure waves · no recordings', 'Engine Synthesizer', 'title'])
                else:
                    page.evaluate(CARD, ['Engine Synthesizer', label, 'long' if len(label) > 40 else ''])
                for _ in range(240):
                    page.evaluate('(n) => { __cap.step(n); }', SPF)
                # the page's view for this engine (it leaves the lower third free, where the words are), a little closer; no orbit.
                # That view frames the engine's core only; where pipes (intake trumpets, a rotary's runner) would then reach
                # past TOP of the frame, step back until every pipe point is below it (at most to x1.25). Not for rotaries: their
                # intake runner rises far above the rotor, and leaving the frame there was the approved look (wankel4_pp).
                k = page.evaluate("""(top) => { const sc = __app.scene, c = sc.controls, cam = sc.camera; sc.setView(sc.viewName, true);
                  const d0 = cam.position.clone().sub(c.target); let k = 0.86;
                  const place = () => { cam.position.copy(d0).multiplyScalar(k).add(c.target); c.update(); cam.updateMatrixWorld(true); };
                  place();
                  while (sc.model.kind !== 'wankel' && k < 1.25 && sc.model.ptsAll.some((p) => p.clone().project(cam).y > top)) { k += 0.02; place(); }
                  sc.idleOrbit = false; c.autoRotate = false; return k; }""", TOP)
                page.keyboard.down('Space')
                for _ in range(210):
                    r = page.evaluate('(n) => __cap.step(n)', SPF)
                im = Image.open(io.BytesIO(page.screenshot(type='png'))).convert('RGB').resize((W, H), Image.LANCZOS)
                page.keyboard.up('Space')
                for q in range(88, 50, -3):
                    buf = io.BytesIO(); im.save(buf, 'JPEG', quality=q, optimize=True, progressive=True)
                    if buf.tell() <= 150_000: break
                open(path, 'wb').write(buf.getvalue())
                px = list(im.convert('L').resize((60, 32)).getdata())
                ok = not errs and r and r[7] and max(px) > 150 and sum(px) / len(px) > 8
                print(f"{'ok  ' if ok else 'FAIL'} {eng:28s} {buf.tell() / 1000:6.1f} KB q{q}  {r[0]:.0f} rpm gear {r[1]:.0f}  view x{k:.2f}  {label[:50]}", flush=True)
                if not ok: bad.append((eng, errs[:2]))
            except Exception as x:
                bad.append((eng, type(x).__name__, errs[:2])); print('FAIL', eng, type(x).__name__, flush=True)
            ctx.close()
        b.close()
finally:
    srv.terminate()
print(f'{len(jobs)} images; failures: {len(bad)}', bad or '')
sys.exit(1 if bad else 0)
