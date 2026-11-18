"""Look at and time the sound picture on the floor of the live app (viz/wave-studies.js) in headless Chromium.

usage: wave_studies.py time [engine ...]     frame time with the floor on against off (the "Sound waves" button), 1920x1080
       wave_studies.py shots [engine ...]    a still at idle, about 3500 rpm and on the limiter, per engine
BASE=http://127.0.0.1:47623/live/ by default (serve the folder first). Output: out.nosync/wave-studies/.
--bare hides the interface in the stills.

States: idle (throttle closed), mid (a small controller holds about 3500 rpm, or 45% of the
redline on engines that cannot reach it), limit (throttle wide open, on the limiter). All in neutral.
"""
import glob
import os
import sys
import time

from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, 'out.nosync', 'wave-studies')
BASE = os.environ.get('BASE', 'http://127.0.0.1:47623/live/')
BARE = '--bare' in sys.argv   # hide the interface, leaving the 3D picture

# hold a state: the page's own pedal path (S.pedalDrag / S.pedalVal), driven by a small controller
HOLD_JS = """(state) => {
  const S = window.__app;
  clearInterval(window.__hold);
  S.pedalDrag = state !== 'idle';
  S.pedalVal = state === 'limit' ? 1 : 0;
  if (state === 'blip') {   // throttle blips: half a second open, 0.7 s closed
    S.pedalDrag = true;
    const t0 = performance.now();
    window.__hold = setInterval(() => { S.pedalVal = ((performance.now() - t0) % 1200) < 500 ? 0.9 : 0; }, 16);
    return;
  }
  if (state !== 'mid') return;
  let thr = 0.08, integ = 0.08;
  window.__hold = setInterval(() => {
    const f = S.engine && S.engine.frame; if (!f) return;
    const red = S.engine.header.spec.redline, want = Math.min(3500, 0.45 * red) < 2500 ? 0.45 * red : 3500;
    const e = (want - f[2]) / red;
    integ = Math.min(0.6, Math.max(0.0, integ + 0.02 * e));
    thr = Math.min(0.7, Math.max(0, integ + 0.5 * e));
    S.pedalVal = thr;
  }, 16);
}"""


def launch(pw):
    exe = sorted(glob.glob(os.path.expanduser(
        "~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
    return pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required', '--use-angle=metal'],
                              **({"executable_path": exe[-1]} if exe else {}))


BARE_JS = """() => { const gl = document.getElementById('gl');
  for (const el of document.querySelectorAll('body *')) if (el !== gl && !el.contains(gl) && !gl.contains(el)) el.style.visibility = 'hidden'; }"""


def open_page(browser, engine, query='', size=(1280, 720)):
    for attempt in range(3):
        try:
            return open_page_once(browser, engine, query, size)
        except Exception as x:  # the plain test server sometimes drops a connection
            if attempt == 2:
                raise
            time.sleep(1)


def open_page_once(browser, engine, query='', size=(1280, 720)):
    pg = browser.new_page(viewport={'width': size[0], 'height': size[1]}, device_scale_factor=1)
    errs = []
    pg.on('pageerror', lambda x: errs.append(str(x)[:300]))
    pg.on('console', lambda m: errs.append(m.text[:300]) if m.type == 'error' and '404' not in m.text and 'favicon' not in m.text else None)
    pg.goto(f'{BASE}?engine={engine}&orbit=0&ui=0{("&" + query) if query else ""}')
    pg.wait_for_selector('#btnStart:not([disabled])', timeout=20000)
    pg.click('#btnStart')
    pg.wait_for_function('() => window.__app && window.__app.engine && window.__app.engine.frame', timeout=20000)
    if BARE:
        pg.evaluate(BARE_JS)
    return pg, errs


def hold(pg, state, settle=None):
    pg.evaluate(HOLD_JS, state)
    pg.wait_for_timeout(int(1000 * (settle if settle is not None else {'idle': 2.5, 'mid': 5.0, 'limit': 4.0, 'blip': 6.0}[state])))


TIME_JS = """async () => {
  const S = window.__app, sc = S.scene, gl = sc.renderer.getContext(), px = new Uint8Array(4);
  const ts = [[], []];
  for (let round = 0; round < 10; round++) for (let m = 0; m < 2; m++) {
    sc.setWaves(m === 1);
    for (let i = 0; i < 25; i++) {
      await new Promise(requestAnimationFrame);
      const t0 = performance.now();
      sc.update(S.engine.frame); sc.render(); gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
      if (i >= 5) ts[m].push(performance.now() - t0);
    }
  }
  return ts.map((t) => { t.sort((a, b) => a - b); return { median: t[t.length >> 1], p90: t[Math.floor(0.9 * t.length)], n: t.length }; });
}"""


def timing(engines):
    """Scene update + render + a one-pixel readback (so the GPU has finished), floor off and on, interleaved."""
    with sync_playwright() as pw:
        b = launch(pw)
        print('| engine | state | floor | median ms | p90 ms |')
        print('|---|---|---|---|---|')
        for e in engines:
            pg, errs = open_page(b, e, size=(1920, 1080))
            for st in ['idle', 'limit']:
                hold(pg, st)
                for label, r in zip(['off', 'on'], pg.evaluate(TIME_JS)):
                    print(f"| {e} | {st} | {label} | {r['median']:.2f} | {r['p90']:.2f} |")
            if errs:
                print('console:', errs[:3])
            pg.close()
        b.close()


def shots(engines):
    os.makedirs(OUT, exist_ok=True)
    with sync_playwright() as pw:
        b = launch(pw)
        for e in engines:
            pg, errs = open_page(b, e)
            for st in ['idle', 'mid', 'limit']:
                hold(pg, st)
                path = os.path.join(OUT, f'floor_{e}_{st}.png')
                pg.screenshot(path=path)
                print(path, errs[:2])
            pg.close()
        b.close()


if __name__ == '__main__':
    a = [x for x in sys.argv[1:] if not x.startswith('--')]
    cmd = a[0] if a else 'shots'
    if cmd == 'time':
        timing(a[1:] or ['v8_cross', 'bmw_s70_2_v12'])
    else:
        shots(a[1:] or ['v8_cross', 'i4', 'wankel4_pp'])
