"""Open a deployed (or locally served) copy of the web app in headless Chromium and check it works.
usage: site_test.py <url> [screenshot.png]      or      site_test.py --local <folder-containing-enginesynth> [screenshot.png]"""
import glob, os, socket, subprocess, sys, time
from playwright.sync_api import sync_playwright
args = sys.argv[1:]; srv = None
if args[0] == '--local':
    port = 47641; s = socket.socket()
    if s.connect_ex(('127.0.0.1', port)) == 0: sys.exit(f'port {port} is in use')
    s.close()
    srv = subprocess.Popen([sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), 'serve.py'), str(port), args[1]], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(0.8); url = f'http://127.0.0.1:{port}/enginesynth/'; shot = args[2] if len(args) > 2 else None
else:
    url = args[0]; shot = args[1] if len(args) > 1 else None
try:
    exe = sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
    with sync_playwright() as pw:
        b = pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required', '--use-angle=metal'], **({"executable_path": exe[-1]} if exe else {}))
        page = b.new_page(viewport={'width': 1600, 'height': 900})
        logs, bad, reqs = [], [], []
        page.on('console', lambda m: logs.append(f'{m.type}: {m.text}') if m.type in ('error', 'warning') else None)
        page.on('pageerror', lambda e: logs.append(f'pageerror: {e}'))
        page.on('response', lambda r: (reqs.append(r.url), bad.append(f'{r.status} {r.url}') if r.status >= 400 else None))
        page.goto(url, wait_until='networkidle')
        page.wait_for_selector('#btnStart:not([disabled])', timeout=20000)
        page.click('#btnStart'); page.wait_for_timeout(2500)
        idle = page.inner_text('#rTorque') if page.query_selector('#rTorque') else '?'
        page.keyboard.down('Space'); page.wait_for_timeout(450); rev = page.inner_text('#rTorque') if page.query_selector('#rTorque') else '?'
        if shot: page.screenshot(path=shot)
        page.keyboard.up('Space'); page.wait_for_timeout(600)
        title = page.inner_text('#elabel') if page.query_selector('#elabel') else ''
        quality = page.inner_text('#quality') if page.query_selector('#quality') else ''
        b.close()
    outside = [u for u in reqs if not u.startswith(url.split('/enginesynth')[0] + '/enginesynth/')]
    print(f'{url}\n  title "{title}", {len(reqs)} requests, torque readout at idle {idle!r}, after holding Space {rev!r}, quality {quality}')
    print('  failed requests:', bad or 'none'); print('  requests outside the folder:', outside or 'none'); print('  console errors/warnings:', logs or 'none')
finally:
    if srv: srv.terminate()
