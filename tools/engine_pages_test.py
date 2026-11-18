"""Check the per-engine pages of a built site (tools/build_site.py): e/<id>/ for every engine.

usage: engine_pages_test.py [build folder] [--url https://joeljohnson.com/enginesynth/] [--public] [--port N] [--only id,id]
  build folder defaults to dist.nosync/enginesynth; --url and --public are what the build was given.

1. What a link-preview crawler sees (the HTML, no JavaScript), for every engine: title, og:title,
   twitter:title, og:description, twitter:description, og:image and twitter:image (absolute with
   --url, and the file is in the build), og:image:alt, og:url and canonical (with --url), the base
   tag, the engine meta tag, and the robots tag (present unless --public). The root page too.
2. In headless Chromium, serving the folder: every e/<id>/ page enables Start, runs that engine,
   makes no failed request, asks for nothing outside the folder and logs no error or warning.
3. Share copies <root>/e/<id>/; the Right arrow key changes engine and the address bar to the next
   engine's page, and reloading that address brings the same engine; a picked engine before Start
   moves the address too; the root page with ?engine= still opens that engine and moves to its page.
"""
import argparse
import glob
import html.parser
import os
import socket
import sys
import time
import urllib.parse

from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument('build', nargs='?', default=os.path.join(ROOT, 'dist.nosync', 'enginesynth'))
ap.add_argument('--url')
ap.add_argument('--public', action='store_true')
ap.add_argument('--port', type=int, default=47657)
ap.add_argument('--only')
A = ap.parse_args()
B = os.path.abspath(A.build)
IDS = [l.strip() for l in open(os.path.join(ROOT, 'engines', 'defs', 'ORDER')) if l.strip()]
if A.only: IDS = [i for i in IDS if i in A.only.split(',')]
fails = []
def check(ok, what, detail=''):
    print(('ok    ' if ok else 'FAIL  ') + what + (f'  [{detail}]' if detail and not ok else ''), flush=True)
    if not ok: fails.append(what)


class Head(html.parser.HTMLParser):
    def __init__(self):
        super().__init__(); self.meta = {}; self.links = {}; self.base = None; self.title = ''; self._t = False; self.counts = {}
    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == 'meta':
            k = a.get('property') or a.get('name')
            if k: self.meta[k] = a.get('content'); self.counts[k] = self.counts.get(k, 0) + 1
        elif tag == 'link' and a.get('rel'): self.links[a['rel']] = a.get('href')
        elif tag == 'base': self.base = a.get('href')
        elif tag == 'title': self._t = True
    def handle_endtag(self, tag):
        if tag == 'title': self._t = False
    def handle_data(self, d):
        if self._t: self.title += d


def parse(path):
    h = Head(); h.feed(open(path).read()); return h


def local_file(img_url, page_dir):
    """Where an og:image address lands in the build folder."""
    if A.url:
        if not img_url.startswith(A.url): return None
        return os.path.join(B, img_url[len(A.url):])
    return os.path.normpath(os.path.join(page_dir, img_url))


# ---------------------------------------------------------------- 1. crawler view
import json
bad = []
for i in IDS:
    p = os.path.join(B, 'e', i, 'index.html')
    if not os.path.isfile(p): bad.append(f'{i}: no page'); continue
    h = parse(p); m = h.meta
    label = json.load(open(os.path.join(ROOT, 'engines', 'defs', i + '.json')))['label']
    want_t = f'{label} · Engine Synthesizer'
    probs = []
    if h.title.strip() != want_t: probs.append(f'title {h.title!r}')
    for k in ('og:title', 'twitter:title'):
        if m.get(k) != want_t: probs.append(f'{k} {m.get(k)!r}')
    d = m.get('og:description') or ''
    if not d.endswith('Sound computed from the pressure waves in a simulated engine. No recordings.') or 'rpm redline.' not in d: probs.append(f'og:description {d!r}')
    if m.get('twitter:description') != d: probs.append('twitter:description differs')
    img = m.get('og:image') or ''
    if m.get('twitter:image') != img: probs.append('twitter:image differs')
    want_img = (A.url or '') + f'og/{i}.jpg'
    if img != want_img: probs.append(f'og:image {img!r}')
    f = local_file(img, B)       # relative addresses resolve against the base, which is the root
    if not f or not os.path.isfile(f): probs.append(f'og:image file missing {f}')
    if not m.get('og:image:alt') or label not in m['og:image:alt']: probs.append('og:image:alt')
    if h.base != '../../': probs.append(f'base {h.base!r}')
    if m.get('engine') != i: probs.append(f'engine meta {m.get("engine")!r}')
    if A.url:
        u = f'{A.url}e/{i}/'
        if m.get('og:url') != u: probs.append(f'og:url {m.get("og:url")!r}')
        if h.links.get('canonical') != u: probs.append(f'canonical {h.links.get("canonical")!r}')
    if ('robots' in m) == A.public: probs.append(f'robots {m.get("robots")!r}')
    dup = [k for k, n in h.counts.items() if n > 1]
    if dup: probs.append(f'duplicate meta {dup}')
    if probs: bad.append(f'{i}: ' + '; '.join(probs))
check(not bad, f'crawler view: title, og/twitter tags, image file, base, canonical and robots right on all {len(IDS)} engine pages', ' | '.join(bad[:4]))
h = parse(os.path.join(B, 'index.html'))
check(h.base == './' and 'engine' not in h.meta and h.meta.get('og:image') == (A.url or './') + 'og.jpg' and (not A.url or h.links.get('canonical') == A.url),
      'crawler view: the root page keeps its own tags, base ./', f'{h.base} {h.meta.get("og:image")} {h.links.get("canonical")}')
sample = parse(os.path.join(B, 'e', IDS[0], 'index.html'))
print('      e.g.', IDS[0], '|', sample.title, '|', sample.meta.get('og:description'), '|', sample.meta.get('og:image'))

# ---------------------------------------------------------------- 2 and 3. in the browser
s = socket.socket()
if s.connect_ex(('127.0.0.1', A.port)) == 0: sys.exit(f'port {A.port} is in use')
s.close()
# served here rather than by tools/nocache_server.py, so the server's own log shows requests the browser
# does not report to Playwright (the AudioWorklet's module is fetched without a page event)
import functools, http.server, threading
SEEN = []
class H(http.server.SimpleHTTPRequestHandler):
    def end_headers(self): self.send_header('Cache-Control', 'no-store, max-age=0'); super().end_headers()
    def log_message(self, *a): pass
    def log_request(self, code='-', size='-'): SEEN.append((self.path, int(code) if isinstance(code, int) or str(code).isdigit() else 0))
class Srv(http.server.ThreadingHTTPServer): request_queue_size = 128; daemon_threads = True
httpd = Srv(('127.0.0.1', A.port), functools.partial(H, directory=os.path.dirname(B)))
threading.Thread(target=httpd.serve_forever, daemon=True).start()
class _S:
    def terminate(self): httpd.shutdown()
srv = _S()
SITE = f'http://127.0.0.1:{A.port}/{os.path.basename(B)}/'
try:
    exe = sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
    with sync_playwright() as pw:
        b = pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required', '--use-angle=metal'], **({"executable_path": exe[-1]} if exe else {}))

        def page_(**kw):
            ctx = b.new_context(viewport={'width': 1280, 'height': 720}, **kw)
            pg = ctx.new_page(); pg.logs, pg.bad, pg.reqs = [], [], []
            pg.on('console', lambda m: pg.logs.append(f'{m.type}: {m.text[:160]}') if m.type in ('error', 'warning') else None)
            pg.on('pageerror', lambda e: pg.logs.append(f'pageerror: {str(e)[:160]}'))
            pg.on('requestfailed', lambda r: pg.bad.append(f'failed {r.url}'))
            pg.on('response', lambda r: (pg.reqs.append(r.url), pg.bad.append(f'{r.status} {r.url}') if r.status >= 400 else None))
            return pg

        def ready(pg, url):
            pg.goto(url); pg.wait_for_selector('#btnStart:not([disabled])', timeout=30000)

        def start(pg):
            pg.click('#btnStart'); pg.wait_for_function('() => __app.started && __app.engine && __app.engine.frame', timeout=30000)

        name = lambda pg: pg.evaluate('() => [__app.name, __app.engine ? __app.engine.engine : null, document.getElementById("elabel").textContent]')

        bad = []
        for i in IDS:
            for attempt in range(2):
                pg = page_()
                try:
                    ready(pg, f'{SITE}e/{i}/?orbit=0&firstrun=0'); start(pg); pg.wait_for_timeout(600)
                    n = name(pg)
                    outside = [u for u in pg.reqs if not u.startswith(SITE)]
                    probs = ([f'running {n}'] if n[0] != i or n[1] != i else []) + pg.bad[:3] + [f'outside {outside[:2]}'] * bool(outside) + pg.logs[:3]
                    if not pg.evaluate('() => !!(__app.engine.frame && Array.prototype.every.call(__app.engine.frame, Number.isFinite))'): probs.append('frame not finite')
                except Exception as x:
                    probs = [type(x).__name__ + ' ' + str(x)[:120]] + pg.logs[:2] + pg.bad[:2]
                pg.context.close()
                if not probs or not any('ERR_CONNECTION' in p or 'ERR_EMPTY' in p for p in probs): break
            if probs: bad.append(f'{i}: ' + '; '.join(probs))
        check(not bad, f'browser: all {len(IDS)} engine pages start their own engine, no failed or outside requests, console clean', ' | '.join(bad[:4]))

        # where the page's own files came from (the base tag): the module scripts, the worklet and the wasm
        pg = page_(permissions=['clipboard-read', 'clipboard-write'])
        a = 'flat6'
        ready(pg, f'{SITE}e/{a}/?orbit=0&firstrun=0'); start(pg); pg.wait_for_timeout(500)
        keys = ('engsim.wasm', 'worklet.js', 'app.js', 'three.module')
        del SEEN[:]
        pg.reload(); pg.wait_for_selector('#btnStart:not([disabled])', timeout=30000); start(pg); pg.wait_for_timeout(500)
        got = [(q, c) for q, c in SEEN if any(k in q for k in keys)]
        root = '/' + os.path.basename(B) + '/'
        check(all(any(k in q for q, _ in got) for k in keys) and all(q.startswith(root) and '/e/' not in q and c == 200 for q, c in got),
              'base tag (server log): wasm, worklet, app.js and three.js each load from the folder root', str(got))
        print('      ' + ', '.join(q.split('?')[0] for q, _ in got))
        pg.click('#btnShare'); pg.wait_for_timeout(300)
        clip = pg.evaluate('() => navigator.clipboard.readText()')
        check(clip == f'{SITE}e/{a}/', 'share: copies the engine page address', clip)
        pg.keyboard.press('ArrowRight')
        pg.wait_for_function(f'() => __app.engine.engine !== "{a}" && __app.shownEngine === __app.engine.engine', timeout=15000); pg.wait_for_timeout(300)
        nxt = name(pg)[1]
        check(pg.url == f'{SITE}e/{nxt}/?orbit=0&firstrun=0', 'arrow key: the address bar moves to the next engine page', pg.url)
        pg.click('#btnShare'); pg.wait_for_timeout(300)
        clip = pg.evaluate('() => navigator.clipboard.readText()')
        check(clip == f'{SITE}e/{nxt}/', 'share after the switch: the new engine, no other parameters', clip)
        pg.reload(); pg.wait_for_selector('#btnStart:not([disabled])', timeout=30000); start(pg)
        check(name(pg)[1] == nxt, 'reload of that address: the same engine', str(name(pg)))
        check(not pg.bad and not pg.logs, 'engine page after a switch and a reload: no failed request, console clean', str(pg.bad[:3] + pg.logs[:3]))
        pg.context.close()

        pg = page_()
        ready(pg, f'{SITE}?engine=porsche_m97_21&firstrun=0')
        check(name(pg)[0] == 'porsche_m97_21', 'root ?engine= (old links): opens that engine', str(name(pg)))
        start(pg); pg.wait_for_timeout(400)
        check(pg.url == f'{SITE}e/porsche_m97_21/?firstrun=0', 'root ?engine=: after Start the address is the engine page', pg.url)
        pg.wait_for_timeout(1000)
        check(not pg.bad and not pg.logs, 'root page after the address moved: no failed request (favicon and the rest), console clean', str(pg.bad[:3] + pg.logs[:3]))
        pg.context.close()

        pg = page_()
        ready(pg, f'{SITE}e/i6/?firstrun=0')
        pg.evaluate("() => __app.pick('v12')"); pg.wait_for_timeout(1200)
        check(pg.url == f'{SITE}e/v12/?firstrun=0' and name(pg)[0] == 'v12', 'pick before Start: the address bar moves too', pg.url)
        pg.context.close()

        pg = page_(permissions=['clipboard-read', 'clipboard-write'])
        ready(pg, SITE); start(pg); pg.wait_for_timeout(400)
        n0 = name(pg)[1]
        pg.click('#btnShare'); pg.wait_for_timeout(300)
        clip = pg.evaluate('() => navigator.clipboard.readText()')
        check(clip == f'{SITE}e/{n0}/' and pg.url == f'{SITE}e/{n0}/', 'root page, default engine: Share and address bar give its page', f'{clip} {pg.url}')
        pg.context.close()
        b.close()
finally:
    srv.terminate()
print(f'{len(fails)} failed' if fails else 'all passed')
sys.exit(1 if fails else 0)
