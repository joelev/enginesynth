"""Assemble the web app into one self-contained folder for static hosting.
usage: build_site.py [out_dir] [--url https://example.com/enginesynth/] [--public]
       (default out_dir dist.nosync/enginesynth)
Copies live/, the shared scene module and only the three.js files the page imports,
and rewrites the import paths so nothing reaches outside the folder.

By default the page carries <meta name="robots" content="noindex"> (unlisted).
--url   the address the folder will be served at: the link-preview image (og:image,
        twitter:image) becomes an absolute URL, and og:url and a canonical link are added.
        Link previews on most sites need this.
--public  leave out the noindex tag, so search engines may list the page.

Engine pages: e/<id>/index.html for every id in engines/defs/ORDER, so a shared link to one engine
unfurls with that engine's own title, description and picture (link-preview crawlers run no
JavaScript, so ?engine= cannot change them). Each is the full app page with <base href="../../">,
so every relative address resolves to the shared files at the folder root, and with
<meta name="engine" content="<id>"> to preselect the engine. The root page gets <base href="./">,
so its relative addresses stay put when the address bar moves to e/<id>/. Its preview image is og/<id>.jpg
(live/og/, rendered by tools/render_og_all.py). window.__enginePages tells the app these pages
exist, so Share and the address bar use e/<id>/."""
import argparse, html, json, math, os, re, shutil, sys
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser(description='Assemble the web app for static hosting.')
ap.add_argument('out', nargs='?', default=os.path.join(ROOT, 'dist.nosync', 'enginesynth'))
ap.add_argument('--url', help='public address of the folder, ending in /')
ap.add_argument('--public', action='store_true', help='omit the noindex robots tag')
A = ap.parse_args()
if A.url and not re.match(r'^https?://[^\s"<>]+/$', A.url): sys.exit('--url must be an http(s) address ending in /')
OUT = os.path.abspath(A.out)
THREE = os.path.join(ROOT, 'viz', 'vendor.nosync', 'node_modules', 'three')
if os.path.isdir(OUT): shutil.rmtree(OUT)
os.makedirs(OUT)
LIVE = ['index.html', 'style.css', 'controls.css', 'app.js', 'firstrun.js', 'spectro.js', 'scene.js', 'engine.js', 'worklet.js', 'engsim.wasm', 'favicon.svg', 'og.jpg']
VIZ = ['engine-model.js', 'wave-studies.js']     # shared with the video renderer; they sit beside the page's own files here
for f in LIVE:
    shutil.copy(os.path.join(ROOT, 'live', f), os.path.join(OUT, f))
for f in VIZ:
    shutil.copy(os.path.join(ROOT, 'viz', f), os.path.join(OUT, f))
# link-preview images, one per engine
IDS = [l.strip() for l in open(os.path.join(ROOT, 'engines', 'defs', 'ORDER')) if l.strip()]
nopic = [i for i in IDS if not os.path.isfile(os.path.join(ROOT, 'live', 'og', i + '.jpg'))]
if nopic: sys.exit(f'no preview image in live/og/ for {nopic}: run tools/render_og_all.py')
os.makedirs(os.path.join(OUT, 'og'))
for i in IDS: shutil.copy(os.path.join(ROOT, 'live', 'og', i + '.jpg'), os.path.join(OUT, 'og', i + '.jpg'))
# three.js: the module build plus the add-ons actually imported, followed through their own relative imports
os.makedirs(os.path.join(OUT, 'vendor', 'three', 'build'))
build = 'three.module.min.js' if os.path.isfile(os.path.join(THREE, 'build', 'three.module.min.js')) else 'three.module.js'
for f in os.listdir(os.path.join(THREE, 'build')):
    if f == build or (f.startswith('three.core') and f.endswith('.js') and ('min' in f) == ('min' in build)):
        shutil.copy(os.path.join(THREE, 'build', f), os.path.join(OUT, 'vendor', 'three', 'build', f))
shutil.copy(os.path.join(THREE, 'LICENSE'), os.path.join(OUT, 'vendor', 'three', 'LICENSE'))
todo, seen = [], set()
for f in ['app.js', 'scene.js'] + VIZ:
    todo += re.findall(r"from\s+'three/addons/([^']+)'", open(os.path.join(OUT, f)).read())
while todo:
    rel = os.path.normpath(todo.pop())
    if rel in seen: continue
    seen.add(rel)
    src = os.path.join(THREE, 'examples', 'jsm', rel)
    dst = os.path.join(OUT, 'vendor', 'three', 'jsm', rel)
    os.makedirs(os.path.dirname(dst), exist_ok=True); shutil.copy(src, dst)
    for imp in re.findall(r"from\s+'(\.[^']+)'", open(src).read()):
        todo.append(os.path.join(os.path.dirname(rel), imp))
# rewrite paths
def sub(name, pairs):
    p = os.path.join(OUT, name); s = open(p).read()
    for a, b in pairs:
        if a not in s: raise SystemExit(f'{name}: expected to find {a!r}')
        s = s.replace(a, b)
    open(p, 'w').write(s)
head = [('../viz/vendor.nosync/node_modules/three/build/three.module.js', './vendor/three/build/' + build),
        ('../viz/vendor.nosync/node_modules/three/examples/jsm/', './vendor/three/jsm/')]
if not A.public: head.append(('<head>', '<head>\n<meta name="robots" content="noindex">'))
if A.url:
    head += [('<meta property="og:image" content="./og.jpg">', f'<meta property="og:image" content="{A.url}og.jpg">'),
             ('<meta name="twitter:image" content="./og.jpg">', f'<meta name="twitter:image" content="{A.url}og.jpg">'),
             ('<meta property="og:type" content="website">', f'<meta property="og:type" content="website">\n<meta property="og:url" content="{A.url}">\n<link rel="canonical" href="{A.url}">')]
elif A.public:
    print('warning: --public without --url: link previews need an absolute image address')
sub('index.html', head)
# every import of a file in viz/ becomes an import of the copy beside the page
for f in ['app.js', 'scene.js']:
    fp = os.path.join(OUT, f); s = open(fp).read()
    open(fp, 'w').write(re.sub(r"'\.\./viz/([\w.-]+\.js)'", r"'./\1'", s))
# Version-stamp our own files: every reference gets ?v=<hash of all contents>, so a browser that
# cached an older copy cannot mix it with a new deploy (GitHub Pages serves max-age=600 and
# phones hold module scripts longer than that).
import hashlib
own = ['app.js', 'firstrun.js', 'spectro.js', 'scene.js', 'engine.js', 'worklet.js', 'engsim.wasm', 'style.css', 'controls.css', 'favicon.svg', 'mock-engine.js'] + VIZ
h = hashlib.sha256()
for f in sorted(own):
    fp = os.path.join(OUT, f)
    if os.path.isfile(fp): h.update(open(fp, 'rb').read())
ver = h.hexdigest()[:10]
for f in ['index.html', 'app.js', 'scene.js', 'engine.js'] + VIZ:
    fp = os.path.join(OUT, f); s = open(fp).read()
    for name in own:
        s = re.sub(r"(['\"])\./" + re.escape(name) + r"\1", lambda m: f"{m.group(1)}./{name}?v={ver}{m.group(1)}", s)
    if f == 'index.html':
        # <base href="./">: the address bar moves to e/<id>/ (history.replaceState), and without a base
        # element relative addresses (the favicon, for one) would then resolve against that address.
        # A base element's URL is fixed when it is parsed, so it stays the folder root.
        s = s.replace('<head>', f'<head>\n<base href="./">\n<script>window.__build = "{ver}"; window.__enginePages = true;</script>', 1)
    open(fp, 'w').write(s)
# ------------------------------------------------------------------ one page per engine
def short_spec(d):
    """One line from the definition: cylinders (or rotors), bore x stroke, redline. Layout and displacement
    are left to the title (the label), whose nominal figures a computed displacement would contradict."""
    rot = d.get('layout') == 'rotary' or d.get('kind') == 'wankel'
    n = len(d.get('rotor_phase') or []) if rot else len(d['firing'])
    mm = lambda x: f'{x * 1000:.1f}'.rstrip('0').rstrip('.')
    parts = ['Two-stroke'] if d.get('kind') == 'two_stroke' else []
    parts.append(f"{n} rotor{'s' * (n > 1)}" if rot else f"{n} cylinder{'s' * (n > 1)}")
    if not rot: parts.append(f"{mm(d['bore'])} x {mm(d['stroke'])} mm bore and stroke")
    parts.append(f"{int(round(d['redline'])):,} rpm redline")
    s = ', '.join(parts) + '.'
    return s[0].upper() + s[1:]
root_html = open(os.path.join(OUT, 'index.html')).read()
def meta(s, attr, key, value):
    pat = re.compile(r'<meta ' + attr + '="' + re.escape(key) + r'" content="[^"]*">')
    if len(pat.findall(s)) != 1: sys.exit(f'index.html: expected one <meta {attr}="{key}">')
    return pat.sub(lambda m: f'<meta {attr}="{key}" content="{html.escape(value)}">', s)
for i in IDS:
    d = json.load(open(os.path.join(ROOT, 'engines', 'defs', i + '.json')))
    title = f"{d['label']} · Engine Synthesizer"
    desc = f"{short_spec(d)} Sound computed from the pressure waves in a simulated engine. No recordings."
    img = f'{A.url or ""}og/{i}.jpg'
    if root_html.count('<base href="./">') != 1: sys.exit('index.html: expected one <base href="./">')
    s = root_html.replace('<base href="./">', '<base href="../../">', 1)
    s = s.replace('<title>Engine Synthesizer</title>', f'<title>{html.escape(title)}</title>\n<meta name="engine" content="{i}">', 1)
    for attr, key, v in [('name', 'description', desc), ('property', 'og:title', title), ('name', 'twitter:title', title),
                         ('property', 'og:description', desc), ('name', 'twitter:description', desc),
                         ('property', 'og:image', img), ('name', 'twitter:image', img),
                         ('property', 'og:image:alt', f"The {d['label']} drawn in 3D, its pipes glowing with the gas pressure inside them")]:
        s = meta(s, attr, key, v)
    if A.url:
        u = f'{A.url}e/{i}/'
        s = meta(s, 'property', 'og:url', u)
        s, k = re.subn(r'<link rel="canonical" href="[^"]*">', f'<link rel="canonical" href="{u}">', s)
        if k != 1: sys.exit('index.html: expected one canonical link')
    if '<meta name="engine"' not in s: sys.exit('engine page: title not found')
    os.makedirs(os.path.join(OUT, 'e', i))
    open(os.path.join(OUT, 'e', i, 'index.html'), 'w').write(s)
print('build', ver, '| public (indexable)' if A.public else '| noindex', '|', A.url or 'relative preview URLs')
left = [f for f in ['index.html', 'app.js', 'firstrun.js', 'spectro.js', 'scene.js', 'engine.js'] + VIZ if re.search(r"\.\./|nosync", open(os.path.join(OUT, f)).read())]
total = sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(OUT) for f in fs)
print(f'{OUT}: {sum(len(fs) for _, _, fs in os.walk(OUT))} files, {total/1e6:.2f} MB; files still pointing outside: {left or "none"}')
missing = []
for f in [x for x in os.listdir(OUT) if x.endswith('.js')]:
    for imp in re.findall(r"(?:from|import)\s*\(?\s*['\"](\.[^'\"?]+)", open(os.path.join(OUT, f)).read()):
        if not os.path.isfile(os.path.normpath(os.path.join(OUT, imp))): missing.append(f'{f} -> {imp}')
if missing or left: sys.exit(f'build is not self-contained: missing {missing}, pointing outside {left}')
