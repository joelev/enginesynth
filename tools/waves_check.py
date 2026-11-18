#!/usr/bin/env python3
"""Checks for the sound-wave picture (viz/sound-waves.js).

  tools/waves_check.py spacing <data_dir> [firings_per_rev]
      Ring spacing against the physics, on a steady-speed (dyno) render: for every frame, the
      radial pressure profile the shader is fed (viz page, window.waves.radialProfile) is
      searched for compression peaks, and the gaps between successive peaks are compared with
      c x firing interval. firings_per_rev defaults to cylinders / 2 (four-stroke).

  tools/waves_check.py live [--shots DIR] [--port N]
      The live page in headless Chromium (real audio thread): source positions for every engine
      in the library, WebGL objects and console across 20 engine switches, frame time at
      1920x1080 with the waves on and off, and screenshots at idle, mid revs and near the limiter.
"""
import argparse, glob, json, os, socket, statistics, subprocess, sys, time
from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools"))


def chromium():
    c = sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
    return {"executable_path": c[-1]} if c else {}


def spacing(data_dir, per_rev):
    from render_video import open_page
    with sync_playwright() as pw:
        browser, page = open_page(pw, os.path.abspath(data_dir), (960, 540), "hud=0")
        # the radial pressure profile each opening's history is drawn with, frame by frame
        res = page.evaluate("""() => {
          const w = window.waves, n = window.sceneInfo.frames, rows = [];
          for (const s of w.sources) rows.push({ name: s.name, prof: [] });
          for (let i = 0; i < n; i++) {
            window.renderFrame(i);
            w.sources.forEach((s, j) => rows[j].prof.push(w.radialProfile(s.index, 0.004).map((q) => q[1] * Math.max(q[0], w.mapping.rMin))));
          }
          return { frames: n, step: 0.004, rows };
        }""")
        browser.close()
    H = json.load(open(os.path.join(data_dir, "telemetry.json")))
    import numpy as np
    b = np.fromfile(os.path.join(data_dir, "telemetry.bin"), dtype=np.float32)
    FF = H["frame_floats"]
    b = b[: len(b) // FF * FF].reshape(-1, FF)
    rpm = float(np.median(b[:, 2]))
    n = len(H["spec"]["firing"])
    per_rev = per_rev or n / 2
    c = H["sound"]["speed_of_sound"]
    want = c * 60.0 / (rpm * per_rev)
    print(f"{H['spec']['label']}: {rpm:.0f} rpm, {per_rev:g} firings per rev -> firing interval {1000 * 60 / (rpm * per_rev):.3f} ms, c x interval = {want:.3f} m")
    # the same opening's history in time (raw samples), for the period actually in the data
    S = H["sound"]
    hist_rows = {j: b[:, S["offset"] + j * S["samples"]: S["offset"] + (j + 1) * S["samples"]] for j in range(S["count"])}
    def time_period(j, want_s):
        X = hist_rows[j] - hist_rows[j].mean(axis=1, keepdims=True)
        n = X.shape[1]; M = int(0.8 * n); ac = np.zeros(M + 1)
        for row in X:
            ac += np.correlate(row, row, "full")[n - 1:][: M + 1] / np.arange(n, n - M - 1, -1)
        lo, hi = int(0.5 * want_s * S["rate"]), min(M - 1, int(1.5 * want_s * S["rate"]))
        if hi <= lo: return None
        k = lo + int(np.argmax(ac[lo:hi + 1])); y0, y1, y2 = ac[k - 1], ac[k], ac[k + 1]
        d = y0 - 2 * y1 + y2
        return (k + (0.5 * (y0 - y2) / d if d else 0)) / S["rate"]
    names = {s["name"]: j for j, s in enumerate(S["sources"])}
    # autocorrelation along the radius (the 1/r spreading undone), summed over frames; its first
    # peak past zero lag is the distance between successive pulses as drawn
    step = res["step"]
    for r in res["rows"]:
        P = np.array(r["prof"])[:, int(0.15 / step):]
        P = P - P.mean(axis=1, keepdims=True)
        L = P.shape[1]
        M = int(0.8 * L)
        ac = np.zeros(M + 1)
        for row in P:
            full = np.correlate(row, row, "full")[L - 1:]
            ac += full[: M + 1] / np.arange(L, L - M - 1, -1)
        ac /= ac[0]
        lo, hi = int(0.5 * want / step), min(len(ac) - 2, int(1.5 * want / step))
        if hi <= lo:
            print(f"  {r['name']}: the drawn radius ({L * step:.2f} m) is too short to see two pulses")
            continue
        k = lo + int(np.argmax(ac[lo:hi + 1]))
        y0, y1, y2 = ac[k - 1], ac[k], ac[k + 1]
        frac = 0.5 * (y0 - y2) / (y0 - 2 * y1 + y2) if (y0 - 2 * y1 + y2) != 0 else 0
        got = (k + frac) * step
        tp = time_period(names.get(r["name"], 0), want / c)
        tps = f"; period in the raw history {1000 * tp:.3f} ms x c = {tp * c:.3f} m, drawn/that {100 * (got / (tp * c) - 1):+.2f}%" if tp else ""
        print(f"  {r['name']}: pulse spacing drawn {got:.3f} m (autocorrelation {y1:.2f}), c x firing interval {want:.3f} m, error {100 * (got / want - 1):+.2f}%{tps}")

def live(shots, port):
    s = socket.socket()
    if s.connect_ex(("127.0.0.1", port)) == 0:
        sys.exit(f"port {port} is in use")
    s.close()
    srv = subprocess.Popen([sys.executable, "-m", "http.server", str(port), "--bind", "127.0.0.1"], cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        time.sleep(0.8)
        with sync_playwright() as pw:
            b = pw.chromium.launch(headless=True, args=["--autoplay-policy=no-user-gesture-required", "--use-angle=metal", "--enable-gpu", "--ignore-gpu-blocklist"], **chromium())
            page = b.new_page(viewport={"width": 1920, "height": 1080}, device_scale_factor=1)
            logs = []
            page.on("console", lambda m: logs.append(f"{m.type}: {m.text}") if m.type in ("error", "warning") else None)
            page.on("pageerror", lambda e: logs.append(f"pageerror: {e}"))
            page.goto(f"http://127.0.0.1:{port}/live/?orbit=0&dpr=1&engine=v8_cross")
            page.wait_for_function("document.getElementById('btnStart') && !document.getElementById('btnStart').disabled", timeout=60000)
            page.click("#btnStart")
            page.wait_for_function("window.__app.engine && window.__app.engine.frame", timeout=30000)
            page.evaluate("""() => {
              const S = window.__app;
              window.__wait = (ms) => new Promise((r) => setTimeout(r, ms));
              window.__switch = async (name) => { await S.engine.setEngine(name); S.name = name; await window.__wait(150); while (!S.engine.frame) await window.__wait(50); };
              // hold engine speed near a target with the throttle (free mode)
              window.__hold = (rpm) => {
                clearInterval(window.__holdT);
                if (!rpm) { S.engine.setThrottle(0); return; }
                let thr = 0.15;
                window.__holdT = setInterval(() => { const f = S.engine.frame; if (!f) return; thr = Math.min(1, Math.max(0, thr + 0.00003 * (rpm - f[2]) - (f[7] > 0.5 ? 0.01 : 0))); S.engine.setThrottle(thr); }, 20);
              };
            }""")
            out = {}
            # ---- source positions, every engine in the library
            names = page.evaluate("window.__app.presets.map((p) => p.name)")
            pos = []
            for nm in names:
                n0 = len(logs)
                r = page.evaluate("""async (nm) => { await window.__switch(nm); const m = window.__app.scene.model, c = m.checkSoundSources(), sp = m.sp;
                  return { name: nm, layout: m.kind === 'wankel' ? 'rotary' : sp.layout, itb: !!sp.itb, n: c.count, maxPosErr: c.maxPosErr, maxDirErrDeg: c.maxDirErrDeg, missing: c.missing,
                           kinds: c.sources.map((s) => s.kind + ':' + s.stem), drawn: window.__app.scene.waves.enabled }; }""", nm)
                r["errors"] = logs[n0:]
                pos.append(r)
            out["positions"] = pos
            # ---- 20 switches: WebGL objects and console
            seq = ["i4", "v8_cross", "wankel4_pp", "bmw_s70_2_v12", "flat6", "v12", "wankel2", "single", "radial7", "vw_w12_6l"] * 2
            seq = [x for x in seq if x in names]
            mem = []
            for nm in seq:
                mem.append(page.evaluate("async (nm) => { await window.__switch(nm); await window.__wait(250); return [nm, window.__app.scene.memory()]; }", nm))
            out["memory"] = mem
            # ---- frame time and screenshots
            def frame_ms(on):
                return page.evaluate("""async (on) => {
                  const S = window.__app, sc = S.scene, gl = sc.renderer.getContext(), px = new Uint8Array(4);
                  sc.setWaves(on);
                  const ts = [];
                  for (let k = 0; k < 150; k++) {
                    const t0 = performance.now();
                    sc.update(S.engine.frame); sc.render(); gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
                    ts.push(performance.now() - t0);
                    if (k % 10 === 9) await new Promise((r) => setTimeout(r, 0));
                  }
                  ts.sort((a, b) => a - b);
                  sc.setWaves(S.wavesOn);
                  return { median: ts[75], p90: ts[135] };
                }""", on)
            os.makedirs(shots, exist_ok=True)
            ft = []
            for nm in ["v8_cross", "wankel4_pp", "bmw_s70_2_v12"]:
                page.evaluate("async (nm) => { await window.__switch(nm); }", nm)
                red = page.evaluate("window.__app.header.spec.redline")
                idle = page.evaluate("window.__app.header.spec.idle")
                for label, rpm in [("idle", 0), ("mid", 0.5 * red), ("high", 0.96 * red)]:
                    page.evaluate("(r) => window.__hold(r)", rpm)
                    page.wait_for_timeout(6000 if rpm else 3000)
                    for view in (["engine", "exhaust"] if label != "idle" or nm == "v8_cross" else ["engine"]):
                        page.evaluate("(v) => { window.__app.scene.setView(v, true); }", view)
                        page.wait_for_timeout(250)
                        rp = page.evaluate("window.__app.engine.frame[2]")
                        f = os.path.join(shots, f"{nm}_{label}_{view}.png")
                        page.screenshot(path=f)
                        print(f"{f}  ({rp:.0f} rpm)")
                    if nm == "v8_cross":
                        ft.append({"engine": nm, "state": label, "rpm": page.evaluate("window.__app.engine.frame[2]"), "on": frame_ms(True), "off": frame_ms(False)})
                page.evaluate("() => window.__hold(0)")
            page.evaluate("(v) => window.__app.scene.setView('engine', true)", "engine")
            out["frame_ms"] = ft
            # waves=0 parameter and the toggle
            page.click("#btnWaves")
            out["toggle_off_visible"] = page.evaluate("window.__app.scene.waves.root.visible")
            page.click("#btnWaves")
            out["toggle_on_visible"] = page.evaluate("window.__app.scene.waves.root.visible")
            page.goto(f"http://127.0.0.1:{port}/live/?orbit=0&dpr=1&engine=v8_cross&waves=0")
            page.wait_for_function("document.getElementById('btnStart') && !document.getElementById('btnStart').disabled", timeout=60000)
            page.click("#btnStart")
            page.wait_for_function("window.__app.engine && window.__app.engine.frame", timeout=30000)
            out["param_waves0_visible"] = page.evaluate("window.__app.scene.waves.root.visible")
            out["param_waves0_button_on"] = page.evaluate("document.getElementById('btnWaves').classList.contains('on')")
            b.close()
        thrown = [p for p in out["positions"] if p["errors"]]
        for p in thrown:
            print(f"  engine {p['name']} raised while loading (not checked): {p['errors'][0][:120]}")
        out["positions"] = [p for p in out["positions"] if not p["errors"]]
        bad = [p for p in out["positions"] if p["maxPosErr"] > 1e-4 or p["maxDirErrDeg"] > 1.0 or p["missing"]]
        print(f"positions: {len(out['positions'])} engines, worst position error {max(p['maxPosErr'] for p in out['positions']):.2e} m, worst direction error {max(p['maxDirErrDeg'] for p in out['positions']):.3f} deg, failures {len(bad)}")
        for p in bad:
            print("  FAIL", p)
        by = {}
        for p in out["positions"]:
            k = p["layout"] + (" itb" if p["itb"] else "")
            by.setdefault(k, []).append(f"{p['name']}({p['n']})")
        for k, v in sorted(by.items()):
            print(f"  {k}: {', '.join(v)}")
        print("memory across switches:", " ".join(f"{m[0]}:{m[1]['geometries']}g/{m[1]['textures']}t/{m[1]['programs']}p" for m in out["memory"]))
        for f in out["frame_ms"]:
            print(f"frame time v8_cross {f['state']} ({f['rpm']:.0f} rpm): waves on median {f['on']['median']:.2f} ms p90 {f['on']['p90']:.2f}; off median {f['off']['median']:.2f} ms p90 {f['off']['p90']:.2f}")
        print("toggle:", out["toggle_off_visible"], out["toggle_on_visible"], "| waves=0:", out["param_waves0_visible"], out["param_waves0_button_on"])
        print("console:", logs if logs else "clean")
        json.dump(out, open(os.path.join(shots, "waves_check.json"), "w"), indent=1)
    finally:
        srv.terminate()


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["spacing", "live"])
    ap.add_argument("data_dir", nargs="?")
    ap.add_argument("per_rev", nargs="?", type=float, default=0)
    ap.add_argument("--shots", default=os.path.join(ROOT, "out.nosync", "waves"))
    ap.add_argument("--port", type=int, default=47719)
    a = ap.parse_args()
    if a.cmd == "spacing":
        spacing(a.data_dir, a.per_rev)
    else:
        live(a.shots, a.port)
