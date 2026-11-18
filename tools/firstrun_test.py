"""Play the first run of the web app the way a new visitor would, with real key events, and check each stage.

usage: firstrun_test.py [base-url] [--out dir] [--video] [--only 1440x900]
  base-url defaults to http://127.0.0.1:47623/live/ (serve the repo folder first: python3 tools/serve.py 47623 .)
  --out    where stage screenshots (and the video) go; default out.nosync/first-run
  --video  also record the 1440x900 run (webm; converted to mp4 when ffmpeg is installed)

Runs, each in its own browser context (so nothing is remembered from the last):
  first run at 1440x900, 1280x720 and 390x844 (touch): load, the ready screen (ghost keys, the ripple
    from the key), Space (or a tap) to start, the swell and the lights, the engine in Drive at rest,
    hold to pull away through automatic upshifts, the shift keys wake, E (manual), A (auto), coast
    until the brake cue, S, the number row;
  returning visitor: a second load in the same context shows no cues and everything lit;
  space audit: Space with the library drawer open and scrolled, a chip focused, a Workshop slider and
    checkbox focused, About open: no scroll, no click, no panel change, and the throttle rises;
    the drawer leaves the tachometer, keys and spectrogram in view and shifts the picture; the
    left and right arrow keys step engines with Space held, drawer open or closed;
  start by holding Space: the hold that starts the engine is not read as throttle.
"""
import argparse
import glob
import os
import shutil
import subprocess
import sys

from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument('base', nargs='?', default='http://127.0.0.1:47623/live/')
ap.add_argument('--out', default=os.path.join(ROOT, 'out.nosync', 'first-run'))
ap.add_argument('--video', action='store_true')
ap.add_argument('--only')
ap.add_argument('--video-only', action='store_true')
A = ap.parse_args()
os.makedirs(A.out, exist_ok=True)

FR = "() => { const f = __app.firstRun; return { active: f.active, stage: f.stage, cue: f.cue, brakeCue: f.brakeCue, upshifts: f.upshifts, done: Object.assign({}, f.done) }; }"
ST = """() => { const S = __app, f = S.engine && S.engine.frame; return { started: S.started, thr: S.thr, sent: S.engine ? S.engine._throttle : null,
  mode: S.mode, auto: S.auto, rpm: f ? f[2] : 0, gear: f ? f[5] : 0, speed: f ? f[6] : 0, cut: f ? f[7] : 0, name: S.name, redline: S.header ? S.header.spec.redline : 0,
  body: document.body.className, lit: S.scene.lit, ctx: S.ctx && S.ctx.state }; }"""
# opacity an element is actually shown at (product up the tree) and whether it is on screen
SHOWN = """(sel) => { const e = document.querySelector(sel); if (!e) return null; let o = 1; for (let n = e; n && n.nodeType === 1; n = n.parentElement) o *= +getComputedStyle(n).opacity;
  const r = e.getBoundingClientRect(); return { o: +o.toFixed(2), x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height), inView: r.left >= -1 && r.top >= -1 && r.right <= innerWidth + 1 && r.bottom <= innerHeight + 1 }; }"""

# a ghost is a key drawn as its bare outline in the bed (not woken yet): no slab under it
GHOST = "(sel) => getComputedStyle(document.querySelector(sel)).boxShadow === 'none'"
LITROW = "() => [...document.querySelectorAll('#pedal .num')].filter((c) => c.classList.contains('lit')).length"
INTRO = "() => { const I = __app.scene.intro; return I ? { phase: I.phase, reveal: I.reveal, r: I.ripple[0], h: I.ripple[2], dEng: I.dEng, lift: I.lift } : null; }"

results = []


def check(ok, what, detail=''):
    results.append(bool(ok))
    print(('  ok   ' if ok else '  FAIL ') + what + (f'  [{detail}]' if detail else ''))


def watch(page):
    logs = []
    page.on('console', lambda m: logs.append(f'{m.type}: {m.text}') if m.type in ('error', 'warning') and 'favicon' not in m.text else None)
    page.on('pageerror', lambda e: logs.append(f'pageerror: {e}'))
    return logs


def wait_stage(page, stage, timeout=15000):
    page.wait_for_function(f"() => __app.firstRun.stage === '{stage}'", timeout=timeout)


def overlaps(page, sels):
    """Pairs of the given elements whose boxes intersect (both visible)."""
    boxes = {s: page.evaluate(SHOWN, s) for s in sels}
    bad = []
    keys = [s for s in sels if boxes[s] and boxes[s]['o'] > 0.05 and boxes[s]['w'] > 0]
    for i, a in enumerate(keys):
        for b in keys[i + 1:]:
            A_, B_ = boxes[a], boxes[b]
            if A_['x'] < B_['x'] + B_['w'] and B_['x'] < A_['x'] + A_['w'] and A_['y'] < B_['y'] + B_['h'] and B_['y'] < A_['y'] + A_['h']:
                bad.append(f'{a} x {b}')
    off = [s for s in keys if not boxes[s]['inView']]
    return bad, off


def first_run(browser, w, h, touch, video=False):
    tag = f'{w}x{h}'
    print(f'\nfirst run at {tag}{" (touch)" if touch else ""}')
    opts = dict(viewport={'width': w, 'height': h}, has_touch=touch, is_mobile=touch, device_scale_factor=2 if touch else 1)
    if video:
        opts.update(record_video_dir=os.path.join(A.out, 'video'), record_video_size={'width': w, 'height': h})
    ctx = browser.new_context(**opts)
    page = ctx.new_page()
    logs = watch(page)
    shot = lambda name: page.screenshot(path=os.path.join(A.out, f'{tag}-{name}.png'))
    page.goto(A.base + '?firstrun=1&engine=v8_cross')
    page.wait_for_timeout(250)
    shot('0-loading')
    page.wait_for_selector('#btnStart:not([disabled])', timeout=30000)
    page.wait_for_timeout(1300 if video else 700)
    fr = page.evaluate(FR)
    sp, tach, top = page.evaluate(SHOWN, '#btnStart'), page.evaluate(SHOWN, '#cluster'), page.evaluate(SHOWN, '#topright')
    ghosts = [page.evaluate(GHOST, s_) for s_ in ['#pedal .num', '#btnDown', '#btnUp', '#btnAuto', '#btnBrake', '#btnMode']]
    check(fr['stage'] == 'ready' and fr['cue'] == 'space' and sp['o'] == 1 and tach['o'] == 0 and top['o'] == 0 and page.evaluate('() => __app.scene.lit') == 0 and all(ghosts) and not page.evaluate(GHOST, '#btnStart'),
          'ready screen: only the space bar is lit in a bed of ghost keys, the engine is dark', f"{fr['stage']}, cue {fr['cue']}, space {sp['o']}, tach {tach['o']}, ghosts {ghosts}")
    # the heartbeat: one ripple leaves the point under the key and passes under the engine
    page.wait_for_function("() => { const I = __app.scene.intro; return I && I.phase === 'ready' && I.ripple[2] > 0.02 && I.ripple[0] > 0.45 * I.dEng && I.ripple[0] < 0.75 * I.dEng; }", timeout=15000)
    it = page.evaluate(INTRO)
    key = page.evaluate("() => { const S = __app, I = S.scene.intro, v = I.key.clone().project(S.scene.camera), r = document.getElementById('btnStart').getBoundingClientRect(); return [(v.x + 1) / 2 * innerWidth - (r.left + r.width / 2), (1 - v.y) / 2 * innerHeight - (r.top + r.height / 2)]; }")
    check(it['reveal'] >= 5 and abs(key[0]) < 6 and abs(key[1]) < 6, 'the floor has spread and its ripple starts under the key', f"ripple at {it['r']:.2f} m of {it['dEng']:.2f} to the engine, height {it['h']:.3f}; origin off the key centre by {key[0]:.1f}, {key[1]:.1f} px")
    check(abs((sp['x'] + sp['w'] / 2) - w / 2) < 2 and sp['y'] > h * 0.75, 'the space bar is low and centred', f"centre x {sp['x'] + sp['w'] / 2}, top {sp['y']} of {h}")
    pt = page.evaluate(SHOWN, '#pitch')
    words = page.inner_text('#pitch')
    # on a phone the words' box (with its veil) spans the top; their right padding keeps the text clear of About
    bad, off = overlaps(page, ['#pitch', '#btnStart', '#kLetters', '#pedal'] + ([] if touch else ['#btnAbout']))
    check(pt['o'] == 1 and pt['inView'] and not bad and 'Engine Synthesizer' in words and 'No recordings.' in words and 'locally on your CPU' in words and 'Time Stolen by Joel Johnson' in words,
          'the loading screen says what this is (title, claim, note), clear of the keys', f"{pt['w']}x{pt['h']} at {pt['x']},{pt['y']}; overlaps {bad}")
    shot('1-ready')
    # start: Space on a keyboard, a tap on a touch screen
    if touch:
        page.tap('#btnStart')
    else:
        page.keyboard.down('Space')
        page.wait_for_timeout(120)
        page.keyboard.up('Space')
    page.wait_for_function('() => __app.started && __app.engine.frame', timeout=20000)
    page.wait_for_function("() => __app.scene.intro && __app.scene.intro.phase === 'bloom' && __app.scene.intro.ripple[0] > 0.6 * __app.scene.intro.dEng", timeout=5000)
    it, lit = page.evaluate(INTRO), page.evaluate('() => __app.scene.lit')
    shot('2a-bloom')
    check(it['phase'] == 'bloom' and lit < 0.9, 'the press sends one swell from the key; the lights are still coming up behind it', f"front at {it['r']:.2f} m, engine at {it['dEng']:.2f} m, lit {lit:.2f}")
    page.wait_for_timeout(250)
    sp2 = page.evaluate(SHOWN, '#btnStart')
    check(sp2['x'] == sp['x'] and sp2['y'] == sp['y'] and sp2['w'] == sp['w'], 'the key that started the engine has not moved', f"{sp['x']},{sp['y']} -> {sp2['x']},{sp2['y']}")
    page.wait_for_timeout(1500)
    st, fr = page.evaluate(ST), page.evaluate(FR)
    q = {s: page.evaluate(SHOWN, s)['o'] for s in ['#tach', '#btnStart', '#topright', '#center']}
    ghosts = [page.evaluate(GHOST, s_) for s_ in ['#pedal .num', '#btnDown', '#btnUp', '#btnAuto', '#btnBrake', '#btnMode']]
    check(fr['stage'] == 'started' and st['lit'] == 1 and all(v == 1 for v in q.values()) and not any(ghosts) and not page.query_selector('.fr-quiet, .fr-noshift'),
          'started: engine lit and every key drawn in full: no ghosts, nothing dimmed, once the engine runs', f'{q}, ghosts {ghosts}')
    page.wait_for_function('() => __app.scene.intro === null', timeout=8000)
    check(True, 'the opening swell has gone and the floor is on its own pulses')
    st = page.evaluate(ST)
    check(st['thr'] == 0 and st['rpm'] > 300 and st['mode'] == 'drive' and st['auto'] and st['gear'] == 1 and st['speed'] < 0.3 and page.inner_text('#gear') == '1',
          'the engine starts in Drive, first gear, automatic, idling at rest with the throttle shut', f"throttle {st['thr']}, {round(st['rpm'])} rpm, gear {st['gear']}, {st['speed']:.2f} m/s")
    check(page.evaluate(SHOWN, '#pitch')['o'] == 0 and page.evaluate(SHOWN, '#title')['o'] == 1, 'on start the words fade and the name is the small label top left')
    shot('2-started')
    # first throttle: hold to the limiter
    if touch:
        # a real finger, through the browser's touch input (not a synthetic pointer event)
        cdp = ctx.new_cdp_session(page)
        fx, fy = sp['x'] + sp['w'] * 0.5, sp['y'] + sp['h'] * 0.5
        hold = lambda: cdp.send('Input.dispatchTouchEvent', {'type': 'touchStart', 'touchPoints': [{'x': fx, 'y': fy, 'id': 1}]})
        lift = lambda: cdp.send('Input.dispatchTouchEvent', {'type': 'touchEnd', 'touchPoints': []})
    else:
        hold = lambda: page.keyboard.down('Space')
        lift = lambda: page.keyboard.up('Space')
    hold()
    page.wait_for_timeout(350)
    st, fr = page.evaluate(ST), page.evaluate(FR)
    down = page.evaluate("() => document.getElementById('btnStart').classList.contains('down')")
    row = page.evaluate(LITROW)
    woke = not any(page.evaluate(GHOST, s_) for s_ in ['#pedal .num', '#btnBrake', '#btnMode', '#btnDown', '#btnAuto'])
    check(st['thr'] > 0.8 and fr['stage'] == 'driving' and fr['cue'] is None and down and row == 10 and woke and 'fr-quiet' not in st['body'],
          'first throttle: it pulls away, the space-bar cue stops, the key is down, the row is lit to 0, no key is a ghost', f"throttle {st['thr']:.2f}, {round(st['rpm'])} rpm, {st['speed']:.1f} m/s, stage {fr['stage']}")
    shot('3-throttle')
    wait_stage(page, 'cue-shift', timeout=40000)
    page.wait_for_timeout(700)
    st, fr = page.evaluate(ST), page.evaluate(FR)
    q = {s_: (page.evaluate(SHOWN, s_)['o'], not page.evaluate(GHOST, s_)) for s_ in ['#btnDown', '#btnUp', '#btnAuto']}
    alabel = page.inner_text('#btnAuto i').lower()
    check(fr['upshifts'] >= 2 and fr['cue'] == 'shift' and not fr['brakeCue'] and all(v == (1, True) for v in q.values()) and alabel == 'auto', 'after two automatic upshifts Q and E take the accent, A reading "auto"', f"gear {st['gear']}, upshifts {fr['upshifts']}, {q}, A says {alabel!r}")
    shot('6-shift-cue')
    page.wait_for_function('() => __app.firstRun.upshifts >= 3', timeout=40000)
    check(True, 'held Space through three automatic upshifts', f"gear {page.evaluate(ST)['gear']}")
    page.wait_for_timeout(700)   # the gearbox ignores a shift asked for while one is still going through
    g0 = page.evaluate(ST)['gear']
    lift()       # near the limiter a shift either way can be refused; shift up with the throttle closed
    page.wait_for_timeout(150)
    if touch:
        page.tap('#btnUp')
    else:
        page.keyboard.press('KeyE')
    page.wait_for_timeout(400)
    st, fr = page.evaluate(ST), page.evaluate(FR)
    alabel = page.inner_text('#btnAuto i').lower()
    check(fr['stage'] == 'done' and fr['cue'] is None and not st['auto'] and st['gear'] == g0 + 1 and alabel == 'manual', 'a shift key shifts, ends the cue, takes over from auto, and A now reads "manual"', f"gear {g0} -> {st['gear']}, auto {st['auto']}, done {fr['done']}")
    shot('7-manual')
    if touch:
        page.tap('#btnAuto')
    else:
        page.keyboard.press('KeyA')
    page.wait_for_timeout(300)
    check(page.evaluate(ST)['auto'] and page.inner_text('#btnAuto i').lower() == 'auto', 'A gives the shifting back to auto')
    # the brake is taught when it is wanted: coasting at speed for a couple of seconds
    page.wait_for_function('() => __app.firstRun.brakeCue', timeout=8000)
    st = page.evaluate(ST)
    page.wait_for_timeout(300)
    shot('7a-brake-cue')
    check(st['speed'] > 20 and page.evaluate("() => document.getElementById('btnBrake').classList.contains('teach')"), 'coasting at speed: the S key gives its nudge', f"{st['speed'] * 3.6:.0f} km/h")
    v0 = st['speed']
    if touch:
        bx = page.evaluate(SHOWN, '#btnBrake')
        cdp.send('Input.dispatchTouchEvent', {'type': 'touchStart', 'touchPoints': [{'x': bx['x'] + bx['w'] / 2, 'y': bx['y'] + bx['h'] / 2, 'id': 5}]})
        page.wait_for_timeout(900)
        cdp.send('Input.dispatchTouchEvent', {'type': 'touchEnd', 'touchPoints': []})
    else:
        page.keyboard.down('KeyS'); page.wait_for_timeout(900); page.keyboard.up('KeyS')
    st, fr = page.evaluate(ST), page.evaluate(FR)
    check(not fr['brakeCue'] and fr['done'].get('brake') == 1 and st['speed'] < v0 - 3 and not page.query_selector('.teach'), 'braking slows the car and ends the cue for good', f"{v0 * 3.6:.0f} -> {st['speed'] * 3.6:.0f} km/h")
    page.wait_for_timeout(600)
    shot('8-end')
    # part throttle on the number row: 5 lights the row to 5, 0 is full throttle
    hold5 = (lambda: page.keyboard.down('Digit5')) if not touch else (lambda: cdp.send('Input.dispatchTouchEvent', {'type': 'touchStart', 'touchPoints': [{'x': nums['x'] + nums['w'] * 0.45, 'y': nums['y'] + nums['h'] / 2, 'id': 2}]}))
    nums = page.evaluate(SHOWN, '#pedal')
    hold5()
    page.wait_for_timeout(700)
    t5, row5 = page.evaluate(ST)['thr'], page.evaluate(LITROW)
    shot('8a-row-half')
    (lambda: page.keyboard.up('Digit5'))() if not touch else lift()
    page.wait_for_timeout(500)
    row0 = page.evaluate(LITROW)
    if not touch:
        page.keyboard.down('Digit0'); page.wait_for_timeout(500)
        t0, rowf = page.evaluate(ST)['thr'], page.evaluate(LITROW)
        page.keyboard.up('Digit0'); page.wait_for_timeout(400)
    else:
        cdp.send('Input.dispatchTouchEvent', {'type': 'touchStart', 'touchPoints': [{'x': nums['x'] + nums['w'] * 0.95, 'y': nums['y'] + nums['h'] / 2, 'id': 3}]}); page.wait_for_timeout(500)
        t0, rowf = page.evaluate(ST)['thr'], page.evaluate(LITROW)
        lift(); page.wait_for_timeout(400)
    check(abs(t5 - 0.5) < 0.03 and row5 == 5 and row0 == 0 and t0 > 0.95 and rowf == 10, 'number row: 5 holds half throttle and lights five keys; 0 is full throttle and lights all ten', f'5: throttle {t5:.2f}, {row5} lit; released {row0} lit; 0: throttle {t0:.2f}, {rowf} lit')
    # one grid: the three wide keys of each row, the number row and the space bar share their left edge, and the legends their two lines
    if not touch:
        g = page.evaluate("() => { const r = (s) => document.querySelector(s).getBoundingClientRect(); const b = (s) => Math.round(r(s + ' i').bottom - r(s).bottom); return { left: ['#btnStart', '#pedal .num', '#btnDown', '#btnAuto'].map((s) => Math.round(r(s).left)), right: [Math.round(r('#btnStart').right), Math.round(r('#lever').right)], base: ['#btnDown', '#btnUp', '#btnAuto', '#btnBrake', '#btnMode'].map(b), w: ['#btnDown', '#gear', '#btnUp', '#btnAuto', '#btnBrake', '#btnMode'].map((s) => Math.round(r(s).width)) }; }")
        check(len(set(g['left'])) == 1 and abs(g['right'][0] - g['right'][1]) <= 1 and len(set(g['base'])) == 1 and max(g['w']) - min(g['w']) <= 1, 'deck: one grid (shared edges, equal key widths, legends on one baseline)', str(g))
    else:
        page.tap('#btnEngines'); page.wait_for_timeout(700); shot('8b-drawer'); page.tap('#brClose'); page.wait_for_timeout(300)
    bad, off = overlaps(page, ['#cluster', '#kLetters', '#pedal', '#btnStart', '#center', '#brand', '#topright', '#btnWorkshop'])
    check(not bad and not off, 'nothing overlaps or leaves the screen', f'overlap {bad}, off screen {off}')
    check(not logs, 'console clean', '; '.join(logs[:3]))

    if not video:
        # returning visitor: same context, plain address
        page.goto(A.base + '?engine=v8_cross')
        page.wait_for_selector('#btnStart:not([disabled])', timeout=30000)
        fr = page.evaluate(FR)
        pulse = page.evaluate("() => getComputedStyle(document.getElementById('btnStart')).animationName")
        (page.tap if touch else page.click)('#btnStart')
        page.wait_for_function('() => __app.started && __app.engine.frame', timeout=20000)
        page.wait_for_timeout(1300)
        q = {s: page.evaluate(SHOWN, s)['o'] for s in ['#tach', '#btnStart', '#pedal', '#btnMode', '#btnBrake', '#topright', '#center']}
        st = page.evaluate(ST)
        q['no ghosts'] = 1 if not any(page.evaluate(GHOST, s_) for s_ in ['#pedal .num', '#btnDown', '#btnAuto', '#btnBrake', '#btnMode']) else 0
        check(not fr['active'] and fr['stage'] == 'off' and pulse == 'none' and all(v == 1 for v in q.values()) and 'fr-' not in st['body'] and not page.query_selector('.teach'),
              'returning visitor: no cues, everything lit', f"{fr}, {q}")
        shot('9-returning')
    page.close()
    ctx.close()
    if video:
        vids = sorted(glob.glob(os.path.join(A.out, 'video', '*.webm')), key=os.path.getmtime)
        if vids and shutil.which('ffmpeg'):
            mp4 = os.path.join(A.out, 'first-run.mp4')
            subprocess.run(['ffmpeg', '-y', '-loglevel', 'error', '-i', vids[-1], '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-crf', '20', '-movflags', '+faststart', mp4], check=False)
            print('  video:', mp4 if os.path.isfile(mp4) else vids[-1], '(picture only: the headless recording has no sound)')
        elif vids:
            print('  video:', vids[-1])


def space_audit(browser):
    print('\nspace audit (1440x900)')
    ctx = browser.new_context(viewport={'width': 1440, 'height': 900})
    page = ctx.new_page()
    logs = watch(page)
    page.goto(A.base + '?firstrun=0&engine=i4&orbit=0')
    page.wait_for_selector('#btnStart:not([disabled])', timeout=30000)
    # the hold that starts the engine is not throttle until released and pressed again
    page.keyboard.down('Space')
    page.wait_for_function('() => __app.started && __app.engine.frame', timeout=20000)
    page.wait_for_timeout(900)
    st = page.evaluate(ST)
    check(st['thr'] == 0 and st['sent'] == 0, 'holding Space to start: still idling while it stays held', f"throttle {st['thr']}, {round(st['rpm'])} rpm")
    page.keyboard.up('Space')
    page.wait_for_timeout(150)
    page.keyboard.down('Space')
    page.wait_for_timeout(300)
    check(page.evaluate(ST)['thr'] > 0.8, 'released and pressed again: throttle')
    page.keyboard.up('Space')
    page.wait_for_timeout(500)

    clicks = "() => { window.__clicks = []; document.addEventListener('click', (e) => window.__clicks.push(e.target.id || e.target.className || e.target.tagName)); }"
    page.evaluate(clicks)
    snap = "() => ({ lib: !document.getElementById('browser').hidden, ws: document.getElementById('workshop').classList.contains('open'), note: !document.getElementById('note').hidden, list: document.getElementById('brList').scrollTop, card: document.querySelector('#note .card').scrollTop, win: scrollY, name: __app.name, view: __app.scene.viewName, muffler: document.getElementById('wsMuffler').checked, header: document.getElementById('wsHeader').value, waves: __app.wavesOn, clicks: window.__clicks.length, search: document.getElementById('brSearch').value, focus: document.activeElement.id || document.activeElement.className || document.activeElement.tagName })"

    def hold_space(what, expect_throttle=True):
        page.evaluate('() => { window.__clicks.length = 0; }')
        a = page.evaluate(snap)
        page.keyboard.down('Space')
        page.wait_for_timeout(450)
        thr = page.evaluate(ST)['thr']
        page.keyboard.up('Space')
        page.wait_for_timeout(250)
        b = page.evaluate(snap)
        same = a == b
        ok = same and ((thr > 0.8) if expect_throttle else thr == 0)
        check(ok, what, f"throttle {thr:.2f}; focus {a['focus']}" + ('' if same else f'; changed: { {k: (a[k], b[k]) for k in a if a[k] != b[k]} }'))

    # library open and scrolled
    page.click('#btnEngines')
    page.wait_for_timeout(200)
    page.evaluate("() => { document.getElementById('brList').scrollTop = 300; }")
    hold_space('library open and scrolled: no scroll, throttle rises')
    # an engine card focused
    page.evaluate("() => { const c = document.querySelectorAll('#brList .eng')[9]; c.focus({ preventScroll: true }); }")
    hold_space('an engine card focused: not loaded, throttle rises')
    page.evaluate("() => document.querySelector('#brFilter button[data-f=real]').focus()")
    hold_space('a filter chip focused: not pressed, throttle rises')
    page.keyboard.down('Digit5'); page.wait_for_timeout(350); t5 = page.evaluate(ST)['thr']; page.keyboard.up('Digit5')
    page.keyboard.press('KeyD'); page.wait_for_timeout(100); m = page.evaluate(ST)['mode']; page.keyboard.press('KeyD')
    check(abs(t5 - 0.5) < 0.03 and m == 'free', 'library open: 5 holds half throttle and D moves the lever (to neutral, then back)', f'throttle {t5:.2f}, mode {m}')
    # the drawer: beside the picture, not over the dashboard
    page.wait_for_timeout(500)
    box = {k: page.evaluate(SHOWN, k) for k in ['#browser', '#cluster', '#kb', '#center']}
    bad, off = overlaps(page, ['#browser', '#cluster', '#kb', '#center', '#topright'])
    shift = page.evaluate('() => [__app.scene.shift, __app.scene.shiftTarget]')
    check(not bad and box['#browser']['x'] == 0 and box['#browser']['w'] <= 460 and abs(shift[0] - box['#browser']['w'] / 2) < 1 and all(box[k]['o'] == 1 for k in box),
          'drawer: down the left side, tachometer, keys and spectrogram in view, picture shifted by half its width', f"drawer {box['#browser']['w']}x{box['#browser']['h']}, shift {shift[0]:.0f} px, overlaps {bad}")
    page.screenshot(path=os.path.join(A.out, 'drawer-1440x900.png'))
    # previous / next engine with the throttle held
    order = page.evaluate("() => { const o = []; for (const c of document.querySelectorAll('#brList .eng')) o.push(c.dataset.name); return o; }")
    i0 = order.index('i4')
    page.keyboard.down('Space'); page.wait_for_timeout(300)
    page.keyboard.press('ArrowRight')
    page.wait_for_function(f"() => __app.engine.engine === '{order[i0 + 1]}' && __app.engine.header === __app.header && __app.engine.frame && __app.engine.frame[6] > 1", timeout=15000)
    st = page.evaluate(ST)
    act = page.evaluate("() => { const c = document.querySelector('#brList .eng.active'), r = c.getBoundingClientRect(), l = document.getElementById('brList').getBoundingClientRect(); return [c.dataset.name, c.classList.contains('on'), r.top >= l.top - 1 && r.bottom <= l.bottom + 1]; }")
    check(st['name'] == order[i0 + 1] and st['thr'] > 0.8 and st['mode'] == 'drive' and act == [order[i0 + 1], True, True],
          'Right arrow with Space held: the next engine in library order, already on throttle in Drive and pulling away; highlighted and in view in the drawer', f"{st['name']}, throttle {st['thr']:.2f}, {st['speed']:.1f} m/s, drawer {act}")
    page.keyboard.press('ArrowLeft'); page.keyboard.press('ArrowLeft')     # two quick steps: neither is dropped
    page.wait_for_function(f"() => __app.engine.engine === '{order[i0 - 1]}' && __app.engine.header === __app.header", timeout=15000)
    check(page.evaluate(ST)['thr'] > 0.8, 'Left arrow twice in quick succession lands two engines back, throttle still held', order[i0 - 1])
    page.keyboard.up('Space')
    page.focus('#brClose')
    hold_space('drawer Done button focused: stays open, throttle rises')
    page.click('#brClose'); page.wait_for_timeout(900)
    sh = page.evaluate('() => __app.scene.shift')
    check(page.evaluate(snap)['lib'] is False and sh == 0, 'Done closes the drawer and the picture moves back', f'shift {sh}')
    page.keyboard.press('ArrowRight')
    page.wait_for_function("() => __app.engine.engine === 'i4' && __app.engine.header === __app.header", timeout=15000)
    check(True, 'Right arrow with the drawer closed steps too (back on i4)')
    page.click('#btnNextEng'); page.wait_for_function(f"() => __app.engine.engine === '{order[i0 + 1]}' && __app.engine.header === __app.header", timeout=15000)
    page.click('#btnPrevEng'); page.wait_for_function("() => __app.engine.engine === 'i4' && __app.engine.header === __app.header", timeout=15000)
    check(True, 'the two arrow key caps beside the engine name step as well')
    page.click('#btnEngines'); page.wait_for_timeout(300)
    # the search box is a real text field: Space types there
    page.click('#brSearch')
    page.keyboard.type('flat 6')
    v, thr = page.input_value('#brSearch'), page.evaluate(ST)['thr']
    check(v == 'flat 6' and thr == 0, 'search box: Space and digits type, no throttle', f'value {v!r}, throttle {thr}')
    page.fill('#brSearch', '')
    page.keyboard.press('Escape')
    # a view chip focused
    page.focus('#views button[data-view=wide]')
    hold_space('a view chip focused: view unchanged, throttle rises')
    page.focus('#btnWaves')
    hold_space('the sound-waves chip focused: not toggled, throttle rises')
    page.focus('#btnShare')
    hold_space('Share focused: not pressed, throttle rises')
    # workshop: slider and checkbox
    page.click('#btnWorkshop')
    page.wait_for_timeout(400)
    page.focus('#wsHeader')
    hold_space('Workshop slider focused: value unchanged, throttle rises')
    page.focus('#wsMuffler')
    hold_space('Workshop checkbox focused: not toggled, throttle rises')
    # the lift-off pops slider reaches the simulator
    pops = page.evaluate("() => { const s = document.getElementById('fuPops'); const shown = !document.getElementById('fuel').hidden && !document.getElementById('rowPops').hidden; const d = __app.engine.liftoffPops; s.value = 0.6; s.dispatchEvent(new Event('input', { bubbles: true })); const v = __app.engine.liftoffPops, label = document.getElementById('oPops').textContent; s.value = 0; s.dispatchEvent(new Event('input', { bubbles: true })); return [shown, d, v, label, __app.engine.liftoffPops]; }")
    check(pops == [True, 0, 0.6, '60%', 0], 'Workshop: the Lift-off pops slider is there, off by default, and sets the simulator', str(pops))
    page.focus('#fuPops')
    hold_space('Lift-off pops slider focused: value unchanged, throttle rises')
    page.focus('#wsReset')
    hold_space('Workshop reset button focused: not pressed, throttle rises')
    page.click('#btnWorkshop')
    # About open (a scrolling card)
    page.click('#btnInfo')
    page.wait_for_timeout(200)
    hold_space('About open: card does not scroll or close, throttle rises')
    page.focus('#noteClose')
    hold_space('About close button focused: stays open, throttle rises')
    page.keyboard.press('Escape')
    # the controls' own caps focused
    page.focus('#btnMode')
    hold_space('D key cap focused: mode unchanged, throttle rises')
    page.focus('#credit a')
    hold_space('the credit link focused: not followed, throttle rises')
    check(len(ctx.pages) == 1 and page.url.startswith(A.base), 'no new tab or navigation came of it', f'{len(ctx.pages)} page(s)')
    cr = page.evaluate("() => { const a = document.querySelector('#credit a'); return [document.getElementById('credit').innerText, a.href, a.target, a.rel]; }")
    check(cr == ['Time Stolen by Joel Johnson', 'https://joeljohnson.com/', '_blank', 'noopener'] and page.evaluate(SHOWN, '#credit')['o'] == 1 and page.evaluate(SHOWN, '#credit')['inView'], 'the credit is on screen while running and links to joeljohnson.com in a new tab', str(cr))
    check(page.evaluate(ST)['mode'] == 'drive', 'still in Drive after all that')
    check(not logs, 'console clean', '; '.join(logs[:3]))
    ctx.close()
    # the drawer at the smaller desktop size, for the eye
    ctx = browser.new_context(viewport={'width': 1280, 'height': 720})
    page = ctx.new_page()
    page.goto(A.base + '?firstrun=0&engine=v8_cross&orbit=0')
    page.wait_for_selector('#btnStart:not([disabled])', timeout=30000)
    page.click('#btnStart')
    page.wait_for_function('() => __app.started && __app.engine.frame', timeout=20000)
    page.wait_for_timeout(3500)
    page.click('#btnEngines'); page.keyboard.down('Space'); page.wait_for_timeout(1500)
    bad, off = overlaps(page, ['#browser', '#cluster', '#kb', '#center', '#views', '#btnWaves', '#load', '#btnShare', '#btnInfo'])   # the top bar's own box is wider than its chips
    print('     drawer', page.evaluate(SHOWN, '#browser'), 'views', page.evaluate(SHOWN, '#views'))
    page.screenshot(path=os.path.join(A.out, 'drawer-1280x720.png'))
    check(not bad, 'drawer at 1280x720: nothing under it but the picture', str(bad))
    page.keyboard.up('Space')
    ctx.close()


exe = sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
with sync_playwright() as pw:
    b = pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required', '--use-angle=metal'], **({"executable_path": exe[-1]} if exe else {}))
    sizes = [(1440, 900, False), (1280, 720, False), (390, 844, True)]
    if A.only:
        sizes = [s for s in sizes if f'{s[0]}x{s[1]}' == A.only]
    for w, h, touch in ([] if A.video_only else sizes):
        first_run(b, w, h, touch)
    if not A.only and not A.video_only:
        space_audit(b)
    if A.video or A.video_only:
        first_run(b, 1440, 900, False, video=True)
    b.close()
print(f'\n{sum(results)} of {len(results)} checks passed')
sys.exit(0 if all(results) else 1)
