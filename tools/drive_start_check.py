"""Every engine, started the way the page now starts it (in Drive, automatic, at rest): does it idle
with the throttle shut, and does it pull away when the throttle is held? Also times the silent gap
when stepping from one engine to the next with the throttle held.

usage: drive_start_check.py [base-url] [gap]
  base-url defaults to http://127.0.0.1:47623/live/ ; ENGINES=id1,id2 limits the run.
  With "gap" it only measures the engine-switch gap (v8_cross -> i4, v8_cross -> bmw_s70_2_v12, i4 -> v8_cross).
"""
import glob
import os
import sys

from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
args = [a for a in sys.argv[1:] if a != 'gap']
BASE = args[0] if args else 'http://127.0.0.1:47623/live/'
ids = [l.strip() for l in open(os.path.join(ROOT, 'engines', 'defs', 'ORDER')) if l.strip()]
if os.environ.get('ENGINES'):
    ids = [i for i in ids if i in os.environ['ENGINES'].split(',')]
ST = "() => { const S = __app, f = S.engine.frame, sp = S.header.spec; return { rpm: f[2], gear: f[5], speed: f[6], idle: sp.idle, redline: sp.redline, mode: S.mode, kind: S.header.kind || sp.kind || '', finite: Array.prototype.every.call(f, Number.isFinite) }; }"
# Silent gap: the longest run of near-silence in the page's output between asking for the switch and
# two seconds later, sampled every 3 ms from the analyser the page already has.
GAP = """async (name) => {
  const S = __app, an = S.analyser, buf = new Float32Array(256), log = [];
  const t0 = performance.now();
  const iv = setInterval(() => { an.getFloatTimeDomainData(buf); let q = 0; for (const v of buf) q += v * v; log.push([performance.now() - t0, Math.sqrt(q / buf.length)]); }, 3);
  await new Promise((r) => setTimeout(r, 300));
  const before = log.reduce((a, x) => a + x[1], 0) / log.length, tAsk = performance.now() - t0;
  const done = S.pick(name);
  let tFrame = null;
  const w = setInterval(() => { if (tFrame === null && S.engine.engine === name && S.engine.header === S.header && S.engine.frame) tFrame = performance.now() - t0 - tAsk; }, 2);
  await done; await new Promise((r) => setTimeout(r, 2000));
  clearInterval(iv); clearInterval(w);
  let gap = 0, run = null;
  for (const [t, v] of log) { if (t < tAsk) continue; if (v < 0.02 * before) { if (run === null) run = t; gap = Math.max(gap, t - run); } else run = null; }
  return { gap_ms: Math.round(gap), first_frame_ms: tFrame === null ? null : Math.round(tFrame), level_before: +before.toFixed(3) };
}"""
exe = sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
bad = []
with sync_playwright() as pw:
    b = pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required', '--use-angle=metal'], **({"executable_path": exe[-1]} if exe else {}))
    if 'gap' in sys.argv:
        for a, c in [('v8_cross', 'i4'), ('v8_cross', 'bmw_s70_2_v12'), ('i4', 'v8_cross')]:
            pg = b.new_page(viewport={'width': 1280, 'height': 720})
            pg.goto(f'{BASE}?engine={a}&orbit=0&firstrun=0')
            pg.wait_for_selector('#btnStart:not([disabled])', timeout=30000)
            pg.click('#btnStart')
            pg.wait_for_function('() => __app.started && __app.engine.frame', timeout=20000)
            pg.wait_for_timeout(1500)
            pg.keyboard.down('Space'); pg.wait_for_timeout(1500)
            print(f'{a} -> {c} with the throttle held:', pg.evaluate(GAP, c))
            pg.close()
    else:
        for e in ids:
            pg = b.new_page(viewport={'width': 960, 'height': 600})
            errs = []
            pg.on('pageerror', lambda x: errs.append(str(x)[:200]))
            try:
                pg.goto(f'{BASE}?engine={e}&orbit=0&firstrun=0&waves=0')
                pg.wait_for_selector('#btnStart:not([disabled])', timeout=30000)
                pg.click('#btnStart')
                pg.wait_for_function('() => __app.started && __app.engine.frame', timeout=20000)
                pg.wait_for_timeout(2500)
                idle = pg.evaluate(ST)
                pg.keyboard.down('Space'); pg.wait_for_timeout(3000)
                go = pg.evaluate(ST)
                pg.keyboard.up('Space')
                ok_idle = idle['finite'] and idle['mode'] == 'drive' and idle['gear'] == 1 and idle['speed'] < 0.5 and 0.6 * idle['idle'] < idle['rpm'] < 1.6 * idle['idle']
                ok_go = go['finite'] and go['speed'] > 3
                print(f"{'ok  ' if ok_idle and ok_go else 'FAIL'} {e:28s} {idle['kind'] or 'piston':10s} idle in Drive {idle['rpm']:6.0f} rpm (target {idle['idle']:.0f}), {idle['speed']:.2f} m/s; after 3 s of throttle {go['speed'] * 3.6:5.0f} km/h, gear {go['gear']:.0f}", flush=True)
                if not (ok_idle and ok_go) or errs:
                    bad.append((e, idle, go, errs[:2]))
            except Exception as x:
                bad.append((e, type(x).__name__, errs[:2])); print('FAIL', e, type(x).__name__, flush=True)
            finally:
                pg.close()
        print(f'{len(ids)} engines; failures: {len(bad)}')
        for f in bad:
            print('  FAIL', f)
    b.close()
sys.exit(1 if bad else 0)
