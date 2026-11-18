"""Checks for the public version of the web app, in Chromium, WebKit (desktop and iPhone profile)
and Firefox. Serves this folder itself on a free port. Prints PASS/FAIL per check; exit 1 on any FAIL.
usage: public_test.py [section ...]   sections: browsers watchdog unsupported links share throttle
                                       touch memory picture motion about predict   (default: all)
       public_test.py --shots <dir>   also save screenshots there"""
import glob, json, os, socket, subprocess, sys, time
from playwright.sync_api import sync_playwright
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
args = sys.argv[1:]
SHOTS = None
if '--shots' in args:
    i = args.index('--shots'); SHOTS = os.path.abspath(args[i + 1]); del args[i:i + 2]; os.makedirs(SHOTS, exist_ok=True)
SECTIONS = args or ['browsers', 'watchdog', 'unsupported', 'links', 'share', 'throttle', 'touch', 'memory', 'picture', 'motion', 'about', 'predict']
s = socket.socket(); s.bind(('127.0.0.1', 0)); PORT = s.getsockname()[1]; s.close()
srv = subprocess.Popen([sys.executable, os.path.join(ROOT, 'tools', 'serve.py'), str(PORT), ROOT])
BASE = f'http://127.0.0.1:{PORT}/live/'
FAILS = []
def check(ok, what, detail=''):
    print(('PASS ' if ok else 'FAIL ') + what + (f'  [{detail}]' if detail != '' else ''))
    if not ok: FAILS.append(what)
def shot(page, name):
    if SHOTS: page.screenshot(path=os.path.join(SHOTS, name + '.png'))
CHROME = sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
def chromium(pw):
    return pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required', '--use-angle=metal'], **({"executable_path": CHROME[-1]} if CHROME else {}))
def watch(page):
    logs = []
    page.on('console', lambda m: logs.append(f'{m.type}: {m.text[:300]}') if m.type in ('error', 'warning') else None)
    page.on('pageerror', lambda e: logs.append(f'pageerror: {e}'))
    return logs
def ready(page, q='', timeout=30000):
    page.goto(BASE + q)
    page.wait_for_selector('#btnStart:not([disabled])', timeout=timeout)
def start(page, tap=False):
    (page.tap if tap else page.click)('#btnStart')
    page.wait_for_function('() => __app.started && __app.engine && __app.engine.frame', timeout=20000)
APP = lambda page, js: page.evaluate(js)
STATE = """() => { const S = __app, e = S.engine; return { name: e && e.engine, level: e && e.level, load: e && e.stats.load, quality: e && e.stats.quality,
  rpm: e && e.frame ? e.frame[2] : null, ctx: S.ctx && S.ctx.state, rate: S.ctx && S.ctx.sampleRate, label: document.getElementById('elabel').textContent,
  notice: !document.getElementById('heavy').hidden, banner: document.getElementById('banner').hidden ? '' : document.getElementById('banner').textContent } }"""

def flow(pw, label, bt, launch, ctx_kw, tap):
    b = launch()
    ctx = b.new_context(**ctx_kw); page = ctx.new_page(); logs = watch(page)
    try:
        t0 = time.time(); ready(page); t_ready = time.time() - t0
        pre = APP(page, "() => ({ name: __app.name, label: document.getElementById('elabel').textContent, model: !!__app.scene.model })")
        check(pre['model'] and pre['label'], f'{label}: engine drawn before Start', f"{pre['name']}, ready in {t_ready:.1f} s")
        if label == 'webkit-iphone': shot(page, 'phone-start')
        start(page, tap); page.wait_for_timeout(1500)
        st = APP(page, STATE)
        check(st['ctx'] == 'running' and st['rate'] == 48000 and st['rpm'] and st['rpm'] > 200, f'{label}: starts, audio running at 48 kHz, engine idling', f"{st['ctx']} {st['rate']} Hz, {st['rpm'] and round(st['rpm'])} rpm")
        # the engine starts in Drive, so Space pulls away: revs rise, or (where the simulator runs far
        # slower than real time, as Playwright's Firefox does) the car is at least moving and the engine has not stalled
        page.keyboard.down('Space'); page.wait_for_timeout(2500); rev = APP(page, STATE)['rpm']; v = APP(page, '() => __app.engine.frame[6]'); page.keyboard.up('Space')
        check(rev > st['rpm'] + 800 or (v > 0.5 and rev > 300), f'{label}: Space pulls away', f"{round(st['rpm'])} -> {round(rev)} rpm, {v:.1f} m/s")
        if label == 'webkit-iphone': shot(page, 'phone-running')
        for name in ['flat6', 'wankel2', 'v10_f1']:
            (page.tap if tap else page.click)('#btnEngines'); page.wait_for_timeout(300)
            (page.tap if tap else page.click)(f'#brList .eng[data-name="{name}"]')
            page.wait_for_function(f"() => __app.engine.engine === '{name}' && __app.engine.frame && __app.header === __app.engine.header", timeout=15000)
            if page.is_visible('#brClose'): (page.tap if tap else page.click)('#brClose')
            page.wait_for_timeout(1200)
            sw = APP(page, STATE)
            check(sw['name'] == name and sw['rpm'] and sw['rpm'] > 200, f'{label}: switch to {name}', f"{sw['label']}, {round(sw['rpm'] or 0)} rpm, {sw['quality']}, load {sw['load']:.2f}")
        errs = [l for l in logs if not l.startswith('warning')]
        check(not errs, f'{label}: no console errors', '; '.join(logs) or 'console clean')
    except Exception as e:
        check(False, f'{label}: flow', f'{type(e).__name__}: {str(e)[:300]}')
    b.close()

with sync_playwright() as pw:
  try:
    time.sleep(0.6)
    if 'browsers' in SECTIONS:
        flow(pw, 'chromium', None, lambda: chromium(pw), {'viewport': {'width': 1440, 'height': 900}}, False)
        flow(pw, 'webkit-desktop', None, lambda: pw.webkit.launch(headless=True), {'viewport': {'width': 1440, 'height': 900}}, False)
        flow(pw, 'webkit-iphone', None, lambda: pw.webkit.launch(headless=True), dict(pw.devices['iPhone 14']), True)
        try:
            ff = lambda: pw.firefox.launch(headless=True, firefox_user_prefs={'media.autoplay.default': 0, 'media.autoplay.block-webaudio': False})
            flow(pw, 'firefox', None, ff, {'viewport': {'width': 1440, 'height': 900}}, False)
        except Exception as e:
            check(False, 'firefox: launch', str(e)[:200])

    b = chromium(pw)
    def page_(q='', **kw):
        ctx = b.new_context(viewport={'width': 1440, 'height': 900}, **kw); p = ctx.new_page(); p._logs = watch(p); return p

    if 'watchdog' in SECTIONS:
        p = page_(); ready(p, '?engine=v12&quality=heavy'); start(p)
        trace = []
        for t in range(9):
            p.wait_for_timeout(1000); st = APP(p, STATE); trace.append(f"{t+1}s {st['level']} {st['load']:.2f}")
        check(st['level'] == 'light' and st['load'] < 0.9 and 'light quality' in (st['banner'] or 'light quality'), 'watchdog: V12 forced heavy steps down to light', ', '.join(trace))
        p.context.close()
        p = page_(); ready(p, '?engine=v12&quality=heavy&slow=2'); start(p)
        trace = []
        early = None
        for t in range(18):
            p.wait_for_timeout(1000); st = APP(p, STATE); trace.append(f"{t+1}s {st['level']} {st['load']:.2f}{' NOTICE' if st['notice'] else ''}")
            if st['notice'] and early is None: early = t + 1
            if st['notice']: break
        # the overload has to last: six seconds above the bar at the light level, not a burst
        check(st['notice'] and early >= 7, 'watchdog: still too heavy at light (slow=2) shows the notice, after a sustained overload only', ', '.join(trace))
        box = p.evaluate("() => { const r = (s) => document.querySelector(s).getBoundingClientRect(); const n = r('#heavy'), k = r('.k-deck'), c = r('#cluster'); return [n.bottom <= k.top, n.bottom <= c.top, Math.round(n.top), Math.round(n.bottom), Math.round(k.top)]; }")
        check(box[0] and box[1], 'watchdog: the notice is clear of the key deck and the tachometer', f'notice {box[2]}..{box[3]} px, deck from {box[4]} px')
        shot(p, 'too-heavy')
        txt = p.inner_text('#heavy'); sw = p.inner_text('#heavySwitch')
        p.click('#heavySwitch'); p.wait_for_timeout(4500); st = APP(p, STATE)
        check(st['name'] != 'v12' and not st['notice'] and st['load'] < 0.9, 'watchdog: the notice switches to an engine that keeps up', f"{sw!r} -> {st['name']} {st['level']} load {st['load']:.2f}")
        print('     notice text:', txt.replace('\n', ' '))
        p.context.close()
        # "Keep this one" is remembered: the notice does not come back for that engine
        p = page_(); ready(p, '?engine=v12&quality=heavy&slow=2'); start(p)
        p.wait_for_function("() => !document.getElementById('heavy').hidden", timeout=25000)
        p.click('#heavyKeep'); p.wait_for_timeout(11000); st = APP(p, STATE)
        check(not st['notice'] and st['name'] == 'v12' and st['load'] > 0.9, 'watchdog: an engine kept is not flagged again', f"11 s later: load {st['load']:.2f}, notice {st['notice']}")
        p.context.close()
        # and the notice goes away by itself when the load recovers. The test slowdown cannot be changed on a
        # running engine, so recovery is stood in for by making the engine's load reports read 0.5 from here on.
        p = page_(); ready(p, '?engine=flat6&slow=4'); start(p)
        p.wait_for_function("() => !document.getElementById('heavy').hidden", timeout=30000)
        l0 = APP(p, STATE)['load']
        p.evaluate("() => { const e = __app.engine; let real = e.stats; Object.defineProperty(e, 'stats', { configurable: true, get: () => Object.assign({}, real, { load: 0.5 }), set: (v) => { real = v; } }); }")
        t0 = time.time(); p.wait_for_function("() => document.getElementById('heavy').hidden", timeout=15000); dt = time.time() - t0
        check(2 <= dt <= 8 and APP(p, '() => __app.name') == 'flat6', 'watchdog: the notice clears itself after three reports of recovered load', f"notice at load {l0:.2f}; cleared {dt:.1f} s after the reports dropped to 0.50")
        p.context.close()

    if 'unsupported' in SECTIONS:
        for what, script in [('AudioWorklet', 'delete window.AudioWorkletNode;'),
                             ('WebGL2', "const g = HTMLCanvasElement.prototype.getContext; HTMLCanvasElement.prototype.getContext = function (t, o) { return t === 'webgl2' ? null : g.call(this, t, o); };")]:
            ctx = b.new_context(viewport={'width': 1440, 'height': 900}); ctx.add_init_script(script); p = ctx.new_page()
            p.goto(BASE); p.wait_for_selector('#startmsg:not([hidden])', timeout=15000)
            msg = p.inner_text('#startmsg'); hidden = p.is_hidden('#btnStart')
            check(what in msg and hidden, f'unsupported: message when {what} is missing', msg.replace('\n', ' ')[:200])
            if what == 'AudioWorklet': shot(p, 'unsupported')
            ctx.close()
        ctx = b.new_context(); p = ctx.new_page(); p.route('**/engsim.wasm', lambda r: r.fulfill(status=404, body='gone'))
        p.goto(BASE); p.wait_for_selector('#startmsg:not([hidden])', timeout=15000)
        check('could not be loaded' in p.inner_text('#startmsg') and p.inner_text('#btnStart') == 'RELOAD', 'unsupported: message when the simulator fails to load', p.inner_text('#startmsg').replace('\n', ' ')[:160])
        ctx.close()

    if 'links' in SECTIONS:
        p = page_()
        p.goto(BASE); p.wait_for_selector('#btnStart:not([disabled])')
        names = APP(p, '() => __app.presets.map((q) => [q.name, q.label])')
        bad = []
        for name, lab in names:
            ready(p, f'?engine={name}')
            got = APP(p, "() => [__app.name, document.getElementById('elabel').textContent, __app.preview.header.spec.label]")
            if got[0] != name or got[1] != lab: bad.append(f'{name}: {got}')
        check(not bad, f'links: ?engine= preview loads the right engine for all {len(names)} ids', '; '.join(bad) or 'all match')
        for name in ['kawasaki_h2_750', 'wankel4_pp', 'cadillac_v16_452']:
            ready(p, f'?engine={name}'); start(p); p.wait_for_timeout(800)
            st = APP(p, STATE)
            check(st['name'] == name and st['rpm'] > 100, f'links: ?engine={name} starts that engine', f"{st['label']} {round(st['rpm'])} rpm")
        ready(p, '?engine=not_an_engine')
        check(APP(p, '() => __app.name') == 'v8_cross' and 'not_an_engine' in p.inner_text('#starttext'), 'links: an unknown id falls back and says so', p.inner_text('#starttext')[:90])
        p.context.close()

    if 'share' in SECTIONS:
        p = page_(permissions=['clipboard-read', 'clipboard-write'])
        ready(p, '?engine=flat6&debug_unused=1'); start(p)
        p.evaluate("() => __app.pick('porsche_m97_21')"); p.wait_for_timeout(600)
        p.click('#btnShare'); p.wait_for_timeout(300)
        clip = p.evaluate('() => navigator.clipboard.readText()')
        check(clip == BASE + '?engine=porsche_m97_21' and 'copied' in p.inner_text('#banner'), 'share: copies a link to the running engine', f"{clip} | {p.inner_text('#banner')}")
        check(APP(p, '() => location.search').startswith('?engine=porsche_m97_21'), 'share: the address bar follows the running engine', APP(p, '() => location.search'))
        p2 = page_(); ready(p2, clip.replace(BASE, '')); start(p2)
        check(APP(p2, STATE)['name'] == 'porsche_m97_21', 'share: the copied link opens that engine', APP(p2, STATE)['label'])
        p.context.close(); p2.context.close()

    if 'throttle' in SECTIONS:
        p = page_(); ready(p); start(p); p.wait_for_timeout(500)
        thr = lambda: APP(p, '() => [__app.engine._throttle, __app.thr]')
        p.keyboard.down('5'); p.wait_for_timeout(30); a = thr(); p.wait_for_timeout(600); b5 = thr(); rpm5 = APP(p, STATE)['rpm']
        p.keyboard.down('Space'); p.wait_for_timeout(30); c = thr(); p.wait_for_timeout(300); d = thr()
        p.keyboard.up('Space'); p.wait_for_timeout(500); e = thr()
        p.keyboard.up('5'); p.wait_for_timeout(30); f = thr()
        p.keyboard.down('2'); p.wait_for_timeout(30); g = thr(); p.keyboard.up('2')
        ok = abs(a[0] - 0.5) < 1e-6 and abs(b5[1] - 0.5) < 0.01 and c[0] >= 0.9 and d[1] > 0.98 and abs(e[1] - 0.5) < 0.02 and f[0] <= 0.08 and abs(g[0] - 0.2) < 1e-6
        check(ok, 'throttle: number keys hold part throttle; Space still instant full; release closes', f'5 down {a[0]:.2f} at once, {b5[1]:.2f} held ({round(rpm5)} rpm); +Space {c[0]:.2f} at once, {d[1]:.2f}; Space up {e[1]:.2f}; 5 up {f[0]:.2f}; 2 down {g[0]:.2f}')
        p.context.close()

    if 'touch' in SECTIONS:
        ctx = b.new_context(viewport={'width': 390, 'height': 844}, device_scale_factor=3, is_mobile=True, has_touch=True)
        p = ctx.new_page(); logs = watch(p); ready(p); p.tap('#btnStart'); p.wait_for_function('() => __app.started && __app.engine.frame')
        cdp = ctx.new_cdp_session(p)
        # the part-throttle control is the row of number keys 1 to 9 and 0: a finger on key n holds n x 10% (0: 100%)
        r = p.evaluate("() => { const r = document.getElementById('pedal').getBoundingClientRect(); return [r.left, r.top + r.height / 2, r.width]; }")
        x = lambda v: r[0] + r[2] * (v * 10 - 0.5) / 10     # the middle of key v x 10 (ten keys: 1 to 9, then 0)
        touch = lambda typ, v: cdp.send('Input.dispatchTouchEvent', {'type': typ, 'touchPoints': [] if typ == 'touchEnd' else [{'x': x(v), 'y': r[1], 'id': 1}]})
        touch('touchStart', 0.3); p.wait_for_timeout(120); a = APP(p, '() => [__app.pedalDrag, __app.pedalVal, __app.engine._throttle]')
        for v in [0.4, 0.5, 0.7, 0.8]: touch('touchMove', v); p.wait_for_timeout(60)
        p.wait_for_timeout(300); bb = APP(p, '() => [__app.pedalDrag, __app.pedalVal, __app.engine._throttle, __app.engine.frame[2]]')
        touch('touchEnd', 0); p.wait_for_timeout(60); c = APP(p, '() => [__app.pedalDrag, __app.engine._throttle]')
        ok = a[0] and abs(a[1] - 0.3) < 0.03 and bb[0] and abs(bb[1] - 0.8) < 0.03 and abs(bb[2] - 0.8) < 0.05 and not c[0] and c[1] <= 0.08
        check(ok, 'touch: sliding a finger along the number keys sets the throttle (Chromium, CDP touch events)', f'down on 3: {a[1]:.2f}; slid to 8: pedal {bb[1]:.2f}, throttle {bb[2]:.2f}, {round(bb[3])} rpm; lifted: drag {c[0]}, throttle {c[1]:.2f}')
        check(not [l for l in logs if not l.startswith('warning')], 'touch: no console errors', '; '.join(logs) or 'clean')
        ctx.close()

    if 'memory' in SECTIONS:
        p = page_(); ready(p); start(p)
        res = p.evaluate("""async () => {
          const S = __app, wait = (ms) => new Promise((r) => setTimeout(r, ms));
          const names = ['i4', 'v12', 'wankel2', 'radial7', 'w16', 'twostroke_250', 'flat6', 'vw_vr6_aaa', 'ducati_1098_testastretta', 'v8_cross'];
          const out = [];
          for (let round = 0; round < 3; round++) {
            for (const n of names) { await S.pick(n); await wait(150); }
            await wait(400);
            out.push(S.scene.memory());
          }
          return out; }""")
        same = res[1] == res[2]
        check(same, 'memory: no WebGL object growth over 20 engine switches (rounds 2 and 3 of 10 switches each)', ' -> '.join(json.dumps(m) for m in res))
        p.context.close()

    if 'picture' in SECTIONS:
        p = page_(); ready(p); start(p)
        cdp = p.context.new_cdp_session(p); cdp.send('Emulation.setCPUThrottlingRate', {'rate': 20})
        p.wait_for_timeout(14000)
        log = APP(p, '() => __app.pic.log'); lvl = APP(p, '() => [__app.pic.level, __app.scene.pixelRatio, __app.scene.reflectionOn, __app.scene.bloomOn, __app.pic.half]')
        cdp.send('Emulation.setCPUThrottlingRate', {'rate': 1})
        check(len(log) > 0, 'picture: steps down when frames do not fit (main thread throttled 20x)', '; '.join(log) + f' | now level {lvl}')
        p.context.close()
        p = page_(); ready(p); start(p); p.wait_for_timeout(5000)
        check(APP(p, '() => __app.pic.log.length') == 0, 'picture: no step-down on this machine unthrottled', APP(p, '() => __app.pic.log'))
        p.context.close()

    if 'motion' in SECTIONS:
        p = page_(reduced_motion='reduce'); ready(p); start(p)
        p.keyboard.down('Space'); p.wait_for_timeout(800)
        sh = APP(p, '() => [__app.scene.reducedMotion, __app.scene.shakeAmp, __app.scene.shake.length()]'); p.keyboard.up('Space')
        p.wait_for_timeout(8000)
        orb = APP(p, '() => __app.scene.controls.autoRotate')
        check(sh[0] and sh[1] == 0 and sh[2] == 0 and not orb, 'motion: prefers-reduced-motion gives no camera shake and no auto-orbit', f'reduced {sh[0]}, shake {sh[1]} / {sh[2]}, auto-orbit after 8 s idle {orb}')
        p.context.close()
        p = page_(); ready(p); start(p); p.keyboard.down('Space'); p.wait_for_timeout(800)
        amp = APP(p, '() => __app.scene.shakeAmp'); p.keyboard.up('Space'); p.wait_for_timeout(8000)
        check(amp > 0 and APP(p, '() => __app.scene.controls.autoRotate'), 'motion: without the preference, shake and auto-orbit stay on', f'shake {amp:.5f}')
        p.context.close()

    if 'about' in SECTIONS:
        p = page_(); ready(p)
        p.click('#btnAbout'); p.wait_for_timeout(300)
        t = p.inner_text('#note')
        check(all(w in t for w in ['About', 'three.js', 'MIT', 'Chimera', 'estimated', 'What is simulated']), 'about: reachable from the start card; credits and estimates present', f'{len(t)} chars')
        shot(p, 'about'); p.keyboard.press('Escape'); p.wait_for_timeout(200)
        check(p.is_hidden('#note'), 'about: Escape closes it')
        start(p); p.click('#btnInfo'); p.wait_for_timeout(200)
        check(p.is_visible('#note') and p.evaluate("() => document.querySelector('#note .card').scrollTop") == 0, 'about: reachable from the top-bar button, opens at the top')
        p.context.close()

    if 'predict' in SECTIONS:
        p = page_(); ready(p)
        r = p.evaluate("""async () => {
          const { LiveEngine } = await import('./engine.js');
          const S = __app, out = [];
          for (const q of S.presets) { const m = await LiveEngine.measure(q.name, {}, 0); out.push([q.name, S.predict(q.name), m]); }
          return { factor: S.factor, bench: S.benchRef, rows: out }; }""")
        errs = sorted(((pr / m - 1, n, pr, m) for n, pr, m in r['rows']), key=lambda x: abs(x[0]))
        mean = sum(abs(e[0]) for e in errs) / len(errs)
        flips = [n for e, n, pr, m in errs if (pr > 0.85) != (m > 0.85)]
        print(f"     prediction vs main-thread measurement, all {len(errs)} engines: mean |error| {mean*100:.0f}%, worst {errs[-1][1]} {errs[-1][0]*100:+.0f}% (pred {errs[-1][2]:.2f}, measured {errs[-1][3]:.2f}); device factor {r['factor']:.3f} from i4 at {r['bench']:.3f}")
        check(mean < 0.25, 'predict: per-engine load prediction within 25% on average', f'{mean*100:.0f}%; threshold disagreements at this speed: {flips or "none"}')
        p.context.close()
    b.close()
  finally:
    srv.terminate()
print(f"\n{'ALL PASS' if not FAILS else str(len(FAILS)) + ' FAILED: ' + '; '.join(FAILS)}")
sys.exit(1 if FAILS else 0)
