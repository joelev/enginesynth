"""Run the live engine in headless Chromium: real AudioContext, worklet, WebAssembly.
Checks that sound comes out, frames arrive, the throttle and drive mode respond, and reports audio-thread load."""
import glob, os, socket, subprocess, sys, time
from playwright.sync_api import sync_playwright
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PORT = 47633
s = socket.socket()
if s.connect_ex(('127.0.0.1', PORT)) == 0: sys.exit(f'port {PORT} is in use')
s.close()
srv = subprocess.Popen([sys.executable, '-m', 'http.server', str(PORT), '--bind', '127.0.0.1'], cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    time.sleep(0.8)
    exe = sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
    with sync_playwright() as pw:
        b = pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required'], **({"executable_path": exe[-1]} if exe else {}))
        page = b.new_page()
        logs = []
        page.on('console', lambda m: logs.append(f'{m.type}: {m.text}'))
        page.on('pageerror', lambda e: logs.append(f'pageerror: {e}'))
        page.goto(f'http://127.0.0.1:{PORT}/live/worklet.js')
        res = page.evaluate("""async (engines) => {
          const { LiveEngine } = await import('/live/engine.js');
          const ctx = new AudioContext({ sampleRate: 48000 });
          await ctx.resume();
          const presets = await LiveEngine.presets();
          const out = { presets: presets.length, state: ctx.state, rate: ctx.sampleRate, runs: [] };
          const eng = await LiveEngine.create(ctx, { engine: engines[0] });
          const an = ctx.createAnalyser(); an.fftSize = 2048; eng.output.connect(an); eng.output.connect(ctx.destination);
          const buf = new Float32Array(2048);
          const rms = () => { an.getFloatTimeDomainData(buf); let s = 0; for (const v of buf) s += v * v; return Math.sqrt(s / buf.length); };
          const wait = ms => new Promise(r => setTimeout(r, ms));
          let frames = 0; eng.addEventListener('frame', () => frames++);
          let rebuilds = 0; eng.addEventListener('rebuild', () => rebuilds++);
          for (const name of engines) {
            if (name !== eng.engine) await eng.setEngine(name);
            frames = 0; eng.setMode('free'); eng.setThrottle(0); await wait(1500);
            const idle = { rpm: eng.frame && eng.frame[2], rms: rms() };
            eng.setThrottle(0.5); await wait(700);
            const rev = { rpm: eng.frame && eng.frame[2], rms: rms() };
            eng.setThrottle(0); await wait(1200);
            out.runs.push({ name, quality: eng.stats.quality, load: eng.stats.load, dropped: eng.stats.dropped, fps: frames / 3.4, idle, rev, floats: eng.frame && eng.frame.length, expect: eng.header.frame_floats });
          }
          eng.setMode('drive'); eng.setThrottle(1); await wait(4000);
          out.drive = { rpm: eng.frame[2], gear: eng.frame[5], kmh: eng.frame[6] * 3.6 };
          eng.setThrottle(0);
          await eng.setEngine(eng.engine, { muffler_len: 0 }); await wait(500);
          out.rebuilds = rebuilds; out.afterOverride = { rpm: eng.frame && eng.frame[2], pipes: eng.header.pipes.length };
          await ctx.close();
          return out;
        }""", sys.argv[1:] or ['i4', 'flat6', 'v8_cross', 'v10_f1'])
        b.close()
    print(f"presets {res['presets']}, context {res['state']} at {res['rate']} Hz, rebuild events {res['rebuilds']}")
    for r in res['runs']:
        print(f"  {r['name']:9s} {r['quality']:9s} load {r['load']:.2f} dropped {r['dropped']} frames/s {r['fps']:.0f} | idle {r['idle']['rpm']:.0f} rpm rms {r['idle']['rms']:.3f} | rev {r['rev']['rpm']:.0f} rpm rms {r['rev']['rms']:.3f} | frame {r['floats']}={r['expect']}")
    print('  drive after 4 s full throttle:', {k: round(v) for k, v in res['drive'].items()}, '| after muffler-off rebuild:', res['afterOverride'])
    print('console:', logs if logs else 'clean')
finally:
    srv.terminate()
