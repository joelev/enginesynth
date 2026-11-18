#!/usr/bin/env python3
"""Render a telemetry directory to an mp4 with the viz/ three.js scene in headless Chromium.

  tools/render_video.py <data_dir> <out.mp4> [--frames a:b] [--size WxH] [--fps 60]
                        [--audio file] [--no-audio] [--params "k=v&..."] [--png] [--still N out.png]
  tools/render_video.py <data_dir> --verify          # geometry consistency checks, prints JSON

Files are served to the page through Playwright request interception on a made-up origin,
so no network port is opened. Screenshots are piped straight into ffmpeg; nothing is written
except the output file.
"""
import argparse, glob, json, os, subprocess, sys, time
from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
VIZ = os.path.join(ROOT, "viz")
ORIGIN = "http://engine.local"


def find_chromium():
    cands = sorted(glob.glob(os.path.expanduser(
        "~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
    return cands[-1] if cands else None


def open_page(pw, data_dir, size, params, software=False):
    args = ["--ignore-gpu-blocklist", "--enable-webgl", "--hide-scrollbars", "--force-color-profile=srgb", "--autoplay-policy=no-user-gesture-required"]
    args += ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] if software else ["--use-angle=metal", "--enable-gpu"]
    exe = find_chromium()
    browser = pw.chromium.launch(headless=True, args=args, **({"executable_path": exe} if exe else {}))
    page = browser.new_page(viewport={"width": size[0], "height": size[1]}, device_scale_factor=1)

    def handle(route):
        path = route.request.url.split("?")[0][len(ORIGIN):]
        if path.startswith("/data/"):
            f = os.path.join(data_dir, path[len("/data/"):])
        elif path.startswith("/viz/"):
            f = os.path.join(VIZ, path[len("/viz/"):])
        else:
            return route.fulfill(status=404, body="")
        f = os.path.realpath(f)
        if not (f.startswith(os.path.realpath(data_dir)) or f.startswith(os.path.realpath(VIZ))) or not os.path.isfile(f):
            return route.fulfill(status=404, body="")
        route.fulfill(path=f)

    page.route(ORIGIN + "/**", handle)
    logs = []
    page.on("console", lambda m: logs.append(m.text) if m.type in ("error", "warning") else None)
    page.on("pageerror", lambda e: logs.append(str(e)))
    url = f"{ORIGIN}/viz/index.html?data=/data&capture=1" + (("&" + params) if params else "")
    page.goto(url)
    try:
        page.wait_for_function("window.sceneReady === true || !!window.sceneError", timeout=120000)
    except Exception:
        browser.close()
        raise SystemExit("page did not become ready:\n" + "\n".join(logs))
    err = page.evaluate("window.sceneError || null")
    if err:
        browser.close()
        raise SystemExit("scene error: " + err + "\n" + "\n".join(logs))
    return browser, page


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("data_dir")
    ap.add_argument("out", nargs="?")
    ap.add_argument("--frames", default="")
    ap.add_argument("--size", default="1920x1080")
    ap.add_argument("--fps", type=float, default=60)
    ap.add_argument("--audio")
    ap.add_argument("--no-audio", action="store_true")
    ap.add_argument("--params", default="")
    ap.add_argument("--png", action="store_true", help="lossless screenshots (slower)")
    ap.add_argument("--software", action="store_true", help="force software GL")
    ap.add_argument("--still", nargs=2, action="append", metavar=("FRAME", "PNG"), help="write single frames instead of a video")
    ap.add_argument("--verify", action="store_true")
    a = ap.parse_args()
    data_dir = os.path.abspath(a.data_dir)
    size = tuple(int(x) for x in a.size.lower().split("x"))
    if not os.path.isdir(os.path.join(VIZ, "vendor.nosync", "node_modules", "three")):
        raise SystemExit("three.js is not vendored. Run:  npm install --prefix viz/vendor.nosync three@0.170.0")

    with sync_playwright() as pw:
        try:
            browser, page = open_page(pw, data_dir, size, a.params, a.software)
        except SystemExit as e:
            if a.software:
                raise
            print(f"GPU launch failed ({e}); retrying with software GL", file=sys.stderr)
            browser, page = open_page(pw, data_dir, size, a.params, True)
        info = page.evaluate("window.sceneInfo")
        print(f"{info['label']}: {info['frames']} frames, t {info['t0']:.3f}..{info['t1']:.3f} s, GL: {info['gl']}", file=sys.stderr)

        if a.verify:
            print(json.dumps(page.evaluate("window.verifyScene(1)"), indent=1))
            browser.close()
            return
        if a.still:
            for frame, path in a.still:
                page.evaluate(f"window.renderFrame({int(frame)})")
                page.screenshot(path=path, type="png")
                print(path)
            browser.close()
            return
        if not a.out:
            raise SystemExit("need an output file")

        n = info["frames"]
        f0, f1 = 0, n
        if a.frames:
            s0, s1 = a.frames.split(":")
            f0 = int(s0) if s0 else 0
            f1 = min(n, int(s1)) if s1 else n
        times = page.evaluate("window.frameTimes()")
        vdur = (f1 - f0) / a.fps
        audio = None if a.no_audio else (a.audio or os.path.join(data_dir, "mix.wav"))
        cmd = ["ffmpeg", "-y", "-loglevel", "error", "-f", "image2pipe", "-framerate", str(a.fps), "-c:v", "png" if a.png else "mjpeg", "-i", "-"]
        if audio and os.path.isfile(audio):
            if a.audio:
                cmd += ["-i", audio]
            else:
                t0, t1 = times[f0], times[f1 - 1] + 1.0 / a.fps
                span = max(t1 - t0, 0.0)
                if abs(span - vdur) > 0.05 * vdur + 0.05:
                    print(f"note: frames cover {span:.3f} s of engine time but the video lasts {vdur:.3f} s; the audio cut will not be in sync (pass --audio or --no-audio)", file=sys.stderr)
                cmd += ["-ss", f"{max(t0, 0):.6f}", "-t", f"{max(span, 0.01):.6f}", "-i", audio]
            cmd += ["-af", f"apad=whole_dur={vdur:.6f}", "-c:a", "aac", "-b:a", "256k"]
        else:
            audio = None
        cmd += ["-c:v", "libx264", "-preset", "medium", "-crf", "15", "-pix_fmt", "yuv420p",
                "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709",
                "-movflags", "+faststart", "-t", f"{vdur:.6f}", a.out]
        proc = subprocess.Popen(cmd, stdin=subprocess.PIPE)
        start = time.time()
        for i in range(f0, f1):
            page.evaluate(f"window.renderFrame({i})")
            shot = page.screenshot(type="png") if a.png else page.screenshot(type="jpeg", quality=96)
            proc.stdin.write(shot)
            if (i - f0) % 120 == 119:
                el = time.time() - start
                print(f"  {i - f0 + 1}/{f1 - f0} frames, {el / (i - f0 + 1):.3f} s/frame", file=sys.stderr)
        proc.stdin.close()
        rc = proc.wait()
        browser.close()
        el = time.time() - start
        print(f"{a.out}: {f1 - f0} frames in {el:.1f} s ({el / max(1, f1 - f0):.3f} s/frame), audio: {'yes' if audio else 'no'}, ffmpeg exit {rc}", file=sys.stderr)
        sys.exit(rc)


if __name__ == "__main__":
    main()
