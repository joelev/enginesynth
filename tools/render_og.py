"""Render the link-preview image (live/og.jpg, 1200x630, JPEG quality 88) from the running app in headless Chromium.
usage: render_og.py [engine] [out.png]     (serves this folder itself on a free port; default v8_cross)
Starts the engine, holds the throttle so the pipes glow, hides the controls and keeps the title."""
import glob, os, socket, subprocess, sys, time
from playwright.sync_api import sync_playwright
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
engine = sys.argv[1] if len(sys.argv) > 1 else 'v8_cross'
out = os.path.abspath(sys.argv[2]) if len(sys.argv) > 2 else os.path.join(ROOT, 'live', 'og.jpg')
s = socket.socket(); s.bind(('127.0.0.1', 0)); port = s.getsockname()[1]; s.close()
srv = subprocess.Popen([sys.executable, '-m', 'http.server', str(port), '--bind', '127.0.0.1'], cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    time.sleep(0.8)
    exe = sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
    with sync_playwright() as pw:
        b = pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required', '--use-angle=metal'], **({"executable_path": exe[-1]} if exe else {}))
        page = b.new_page(viewport={'width': 1200, 'height': 630}, device_scale_factor=1)
        page.goto(f'http://127.0.0.1:{port}/live/?engine={engine}&orbit=0&ui=0&quality=light')
        page.wait_for_selector('#btnStart:not([disabled])', timeout=30000)
        page.click('#btnStart'); page.wait_for_timeout(2500)
        page.add_style_tag(content='#dash,#topright,#btnWorkshop,#keys,#banner,#espec,.engbtn-go,#heavy{display:none!important} #top{padding:34px 40px} #title{font-size:17px} #elabel{font-size:36px}')
        # the camera presets leave room at the bottom for the dashboard, which is hidden here: re-centre and move in
        page.evaluate("""() => { const sc = __app.scene, v = sc.views.engine, c = sc.camera, t = sc.controls.target;
            t.copy(v.target); t.y += 0.11; c.position.copy(v.pos).sub(v.target).multiplyScalar(0.72).add(t); sc.controls.update(); }""")
        page.keyboard.down('Space'); page.wait_for_timeout(900)
        page.screenshot(path=out, type='jpeg', quality=88)
        page.keyboard.up('Space')
        b.close()
    print(out)
finally:
    srv.terminate()
