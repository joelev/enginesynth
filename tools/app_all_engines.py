"""Load every engine in the web app (headless Chromium) and report any that fail to draw or start.

usage: app_all_engines.py [base-url] [--pages]
  With no argument it serves this folder and uses /live/.
  --pages  base-url is a built site's root (e.g. https://joeljohnson.com/enginesynth/): load each
           engine's own page e/<id>/ instead of ?engine=<id>, and check the address stays there.
  ENGINES=id1,id2 limits the run to those engines.
The plain test server sometimes drops a connection under load, so a load that fails with a
connection error is retried (up to three attempts); other failures are reported at once.
"""
import glob
import os
import socket
import subprocess
import sys
import time

from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ids = [l.strip() for l in open(os.path.join(ROOT, 'engines', 'defs', 'ORDER')) if l.strip()]
PAGES = '--pages' in sys.argv
sys.argv = [a for a in sys.argv if a != '--pages']
only = os.environ.get('ENGINES')
if only:
    ids = [i for i in ids if i in only.split(',')]

STATE_JS = """() => {
  const S = window.__app || {}; const g = S.engine;
  return { name: S.name, frame: !!(g && g.frame), rpm: g && g.frame ? g.frame[2] : 0,
           finite: !!(g && g.frame) && Array.prototype.every.call(g.frame, Number.isFinite) };
}"""


def try_engine(browser, base, e):
    """Returns None on success, or (engine, what happened, console errors)."""
    pg = browser.new_page(viewport={'width': 1280, 'height': 720})
    errs = []
    pg.on('pageerror', lambda x: errs.append(str(x)[:200]))
    pg.on('console', lambda m: errs.append(m.text[:200]) if m.type == 'error' and '404' not in m.text and 'favicon' not in m.text else None)
    try:
        if PAGES:
            pg.goto(f'{base}e/{e}/?orbit=0')
        else:
            sep = '&' if '?' in base else '?'
            pg.goto(f'{base}{sep}engine={e}&orbit=0')
        pg.wait_for_selector('#btnStart:not([disabled])', timeout=20000)
        pg.click('#btnStart')
        pg.wait_for_timeout(900)
        st = pg.evaluate(STATE_JS)
        if PAGES and f'/e/{e}/' not in pg.url:
            errs.append(f'address moved to {pg.url}')
        if st['name'] != e or not st['frame'] or not st['finite'] or errs:
            return (e, st, errs[:2])
        return None
    except Exception as x:  # report whatever went wrong
        return (e, type(x).__name__, errs[:2])
    finally:
        pg.close()


srv = None
if len(sys.argv) > 1:
    base = sys.argv[1]
    if PAGES and not base.endswith('/'):
        base += '/'
else:
    if PAGES:
        sys.exit('--pages needs the address of a built site (the development server has no engine pages)')
    port = 47643
    s = socket.socket()
    if s.connect_ex(('127.0.0.1', port)) == 0:
        sys.exit(f'port {port} is in use')
    s.close()
    srv = subprocess.Popen([sys.executable, '-m', 'http.server', str(port), '--bind', '127.0.0.1'], cwd=ROOT,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(0.8)
    base = f'http://127.0.0.1:{port}/live/'

bad = []
try:
    exe = sorted(glob.glob(os.path.expanduser(
        "~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
    with sync_playwright() as pw:
        b = pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required', '--use-angle=metal'],
                               **({"executable_path": exe[-1]} if exe else {}))
        for e in ids:
            failed = None
            for attempt in range(3):
                failed = try_engine(b, base, e)
                dropped = failed is not None and any('ERR_CONNECTION' in str(x) or 'ERR_EMPTY' in str(x) for x in failed[2])
                if not dropped:
                    break
            if failed:
                bad.append(failed)
        b.close()
finally:
    if srv:
        srv.terminate()

print(f'{len(ids)} engines loaded; failures: {len(bad)}')
for f in bad:
    print('  FAIL', f)
sys.exit(1 if bad else 0)
