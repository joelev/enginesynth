"""Render a video of the live page offline, frame by frame, with the simulator's own sound.

usage: render_live_video.py <engine> [--out dir] [--name file-stem] [--size 1920x1080] [--scale 1|2]
                            [--idle 0.5] [--hold 7.5] [--overrun 3] [--az degrees] [--orbit speed]
                            [--view engine] [--quality heavy|light] [--keep-frames]
                            [--overrides JSON]

Not a screen recording. The page is opened with ?capture=1 (live/capture-engine.js): the simulator
runs on the main thread and is advanced exactly 800 samples (1/60 s at 48 kHz) per video frame;
the page's clock (performance.now, requestAnimationFrame) is replaced by one that moves 1/60 s per
frame, so the camera, the floor's pulses, the warm-up and every dial are drawn for that instant;
then the page is screenshotted. The samples computed for those frames are the sound track, so
picture and sound are in step by construction. Input is real key events (Space held), so the deck
shows what a visitor's would.

Timeline: the engine is started and left to idle in Drive for 6 s (not recorded), then recorded:
--idle seconds at rest, --hold seconds of Space held (launch, automatic upshifts), --overrun seconds
after the lift. The camera is the page's own view (--view) turned --az degrees from its usual
angle and orbiting slowly (--orbit is OrbitControls' autoRotateSpeed; 1 is 6 degrees a second).

Output in --out (default out.nosync/launch-videos): <stem>.mp4 (H.264 high, yuv420p, 60 fps, CRF 16,
AAC 256k, faststart), <stem>.jpg (a poster: a frame late in the pull) and <stem>-first-frame.jpg, <stem>.json (per-frame rpm, gear, speed, fuel
cut, throttle and sound level, and the checks). The sound is peak-normalised to -1 dBFS with a
150 ms fade in and a 300 ms fade out; nothing else is done to it.
"""
import argparse
import array
import base64
import glob
import io
import json
import math
import os
import shutil
import socket
import subprocess
import sys
import time
import wave

from PIL import Image
from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument('engine')
ap.add_argument('--out', default=os.path.join(ROOT, 'out.nosync', 'launch-videos'))
ap.add_argument('--name')
ap.add_argument('--size', default='1920x1080')
ap.add_argument('--scale', type=int, default=1)
ap.add_argument('--idle', type=float, default=0.5)
ap.add_argument('--hold', type=float, default=7.5)
ap.add_argument('--overrun', type=float, default=3.0)
ap.add_argument('--az', type=float, default=0.0)
ap.add_argument('--orbit', type=float, default=0.8)
ap.add_argument('--view', default='engine')
ap.add_argument('--quality', default='heavy')
ap.add_argument('--keep-frames', action='store_true')
ap.add_argument('--blowoff', choices=['vent', 'recirc', 'none'], help="turbo engines: blow-off valve routing for this render (default: the engine's own)")
ap.add_argument('--whistle', type=float, help="turbo engines: whistle stand-in gain for this render (default: the engine's own, 0.25)")
ap.add_argument('--lifts', type=int, default=0, help='extra lifts after the first: each is --gap seconds off the throttle, then --rehold seconds back on')
ap.add_argument('--gap', type=float, default=1.0)
ap.add_argument('--rehold', type=float, default=2.5)
ap.add_argument('--rev', type=float, default=0.0, help='before the pull: shift to Neutral (the D key), hold Space this many seconds, release, wait --rev-rest seconds, back to Drive (D), --rev-settle seconds at rest, then the pull as usual')
ap.add_argument('--rev-rest', type=float, default=2.0)
ap.add_argument('--rev-settle', type=float, default=0.7)
ap.add_argument('--rev-fall', dest='rev_rest', type=float, default=argparse.SUPPRESS, help='the same as --rev-rest')
ap.add_argument('--overrides', help='definition overrides as JSON, applied by rebuilding the engine after the start, e.g. \'{"vehicle": null}\' for the generic car in Drive')
ap.add_argument('--stills-only', action='store_true', help='only the first frame (to choose --az)')
A = ap.parse_args()
W, H = (int(v) for v in A.size.split('x'))
FPS, RATE, SPF = 60, 48000, 800
stem = A.name or f'enginesynth-{A.engine}'
os.makedirs(A.out, exist_ok=True)
frames_dir = os.path.join(A.out, stem + '-frames')
shutil.rmtree(frames_dir, ignore_errors=True)
os.makedirs(frames_dir)

from capture_clock import CLOCK   # the page's clock, replaced before any of its scripts run
HIDE = '#load { display: none !important; } * { cursor: none !important; }'


def free_port():
    s = socket.socket(); s.bind(('127.0.0.1', 0)); p = s.getsockname()[1]; s.close(); return p


port = free_port()
srv = subprocess.Popen([sys.executable, os.path.join(ROOT, 'tools', 'serve.py'), str(port), ROOT], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(0.8)
log, t_start = [], time.time()
try:
    exe = sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")))
    with sync_playwright() as pw:
        b = pw.chromium.launch(headless=True, args=['--autoplay-policy=no-user-gesture-required', '--use-angle=metal'], **({"executable_path": exe[-1]} if exe else {}))
        ctx = b.new_context(viewport={'width': W, 'height': H}, device_scale_factor=A.scale)
        page = ctx.new_page()
        errs = []
        page.on('pageerror', lambda e: errs.append(str(e)[:300]))
        page.on('console', lambda m: errs.append(m.text[:300]) if m.type == 'error' and 'favicon' not in m.text else None)
        page.add_init_script(CLOCK)
        page.goto(f'http://127.0.0.1:{port}/live/?capture=1&engine={A.engine}&firstrun=0&preserve=1&quality={A.quality}&view={A.view}')
        page.add_style_tag(content=HIDE)
        page.wait_for_selector('#btnStart:not([disabled])', timeout=60000)
        page.click('#btnStart')
        page.wait_for_function('() => __app.started && __app.engine && __app.engine.step', timeout=30000, polling=100)   # not on animation frames: the page's are held
        if A.overrides:
            page.evaluate('async ([n, o]) => { await __app.pick(n, o); }', [A.engine, json.loads(A.overrides)])
        # settle: the opening swell, the camera's move to its view, the engine at idle in Drive
        for _ in range(6 * FPS):
            page.evaluate('(n) => { __cap.step(n); }', SPF)
        # camera: the view's own position turned by --az about the vertical through what it looks at, then a slow orbit
        page.evaluate("""([view, az, speed]) => {
          const sc = __app.scene, c = sc.controls;
          sc.setView(view, true);
          const p = sc.camera.position, t = c.target, a = az * Math.PI / 180, dx = p.x - t.x, dz = p.z - t.z;
          p.set(t.x + dx * Math.cos(a) + dz * Math.sin(a), p.y, t.z - dx * Math.sin(a) + dz * Math.cos(a));
          c.update();
          sc.lastInput = -1e12; c.autoRotateSpeed = speed; c.autoRotate = speed !== 0; sc.idleOrbit = speed !== 0;
        }""", [A.view, A.az, A.orbit])
        if A.blowoff:
            page.evaluate('(m) => __app.engine.setBlowoff(m)', A.blowoff)
        if A.whistle is not None:
            page.evaluate('(v) => __app.engine.setWhistle(v)', A.whistle)
        for _ in range(20):
            page.evaluate('(n) => { __cap.step(n); }', SPF)
        n_idle, n_hold, n_over = round(A.idle * FPS), round(A.hold * FPS), round(A.overrun * FPS)
        n_gap, n_rehold = round(A.gap * FPS), round(A.rehold * FPS)
        # Space down and up frames: the first pull, then each extra lift and pull
        # with --rev, a free rev in Neutral comes first and the pull starts after it
        keys, rev_downs, rev_ups = {}, [], []
        if A.rev > 0:
            n_rev, n_rest, n_settle = round(A.rev * FPS), round(A.rev_rest * FPS), round(A.rev_settle * FPS)
            keys[n_idle] = keys[n_idle + n_rev + n_rest] = 'KeyD'
            rev_downs, rev_ups = [n_idle + 1], [n_idle + 1 + n_rev]
            n_idle += n_rev + n_rest + n_settle
        downs, ups = [n_idle], [n_idle + n_hold]
        for _ in range(A.lifts):
            downs.append(ups[-1] + n_gap)
            ups.append(downs[-1] + n_rehold)
        total = 1 if A.stills_only else ups[-1] + n_over
        downs, ups = rev_downs + downs, rev_ups + ups
        page.evaluate('() => __app.engine.record()')
        shot_ms = 0
        for i in range(total):
            if i in keys and not A.stills_only:
                page.keyboard.press(keys[i])
            if i in downs and not A.stills_only:
                page.keyboard.down('Space')
            if i in ups:
                page.keyboard.up('Space')
            r = page.evaluate('(n) => __cap.step(n)', SPF)
            log.append(r)
            t0 = time.time()
            page.screenshot(path=os.path.join(frames_dir, f'{i:05d}.jpg'), type='jpeg', quality=97)
            shot_ms += (time.time() - t0) * 1000
            if i % 120 == 0:
                print(f'  frame {i}/{total}: {r[0]:.0f} rpm, gear {r[1]:.0f}, {r[2] * 3.6:.0f} km/h', flush=True)
        label = page.inner_text('#elabel')
        gear_box = page.evaluate("() => { const r = document.getElementById('gear').getBoundingClientRect(); return [Math.round(r.left) + 6, Math.round(r.top) + 6, Math.round(r.width) - 12, Math.round(r.height) - 12]; }")
        pcm = base64.b64decode(page.evaluate("""() => { const a = new Uint8Array(__app.engine.recorded().buffer); let s = ''; for (let i = 0; i < a.length; i += 32768) s += String.fromCharCode.apply(null, a.subarray(i, i + 32768)); return btoa(s); }"""))
        b.close()
finally:
    srv.terminate()

# poster: for a full render, a frame late in the pull (the engine worked and glowing); for --stills-only, the first frame
poster_frame = 0 if A.stills_only else round(n_idle + 0.72 * A.hold * FPS)
poster = os.path.join(A.out, stem + '.jpg')
Image.open(os.path.join(frames_dir, f'{poster_frame:05d}.jpg')).convert('RGB').resize((W, H), Image.LANCZOS).save(poster, quality=93)
if not A.stills_only:
    Image.open(os.path.join(frames_dir, '00000.jpg')).convert('RGB').resize((W, H), Image.LANCZOS).save(os.path.join(A.out, stem + '-first-frame.jpg'), quality=93)
if A.stills_only:
    print('still:', poster, '| errors:', errs[:3] or 'none')
    shutil.rmtree(frames_dir, ignore_errors=True)
    sys.exit(0)

# ---- sound: as computed, peak to -1 dBFS, short fades
snd = array.array('f'); snd.frombytes(pcm)
n = len(snd) // 2
peak = max(abs(v) for v in snd) or 1.0
g = 10 ** (-1 / 20) / peak
fi, fo = int(0.150 * RATE), int(0.300 * RATE)
out = array.array('h', bytes(4 * n))
clipped = 0
for i in range(n):
    k = g * (min(1.0, i / fi) if i < fi else 1.0) * (min(1.0, (n - 1 - i) / fo) if i >= n - fo else 1.0)
    for ch in (0, 1):
        v = snd[2 * i + ch] * k
        if abs(v) >= 1.0: clipped += 1
        out[2 * i + ch] = int(max(-1.0, min(1.0, v)) * 32767)
wav = os.path.join(A.out, stem + '.wav')
with wave.open(wav, 'wb') as w:
    w.setnchannels(2); w.setsampwidth(2); w.setframerate(RATE); w.writeframes(out.tobytes())

# ---- checks on the frames: every one drawn (not black, not blank), every telemetry frame finite
def frame_ok(path):
    im = Image.open(path).convert('L').resize((96, 54))
    px = list(im.getdata())
    return sum(px) / len(px) > 6 and max(px) > 120 and len(set(px)) > 40
bad_frames = [i for i in range(total) if not frame_ok(os.path.join(frames_dir, f'{i:05d}.jpg'))]
not_finite = [i for i, r in enumerate(log) if not r or not r[7] or not all(math.isfinite(v) for v in r)]

gear = [r[1] for r in log]
shifts = [i for i in range(1, total) if gear[i] > gear[i - 1] >= 1]

mp4 = os.path.join(A.out, stem + '.mp4')
# JPEG frames are full range; the file is written limited range, BT.709, plain yuv420p, as players and Twitter expect
vf = ['-vf', f'scale={W}:{H}:flags=lanczos:in_range=pc:out_range=tv:out_color_matrix=bt709,format=yuv420p', '-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']
subprocess.run(['ffmpeg', '-y', '-loglevel', 'error', '-framerate', str(FPS), '-i', os.path.join(frames_dir, '%05d.jpg'), '-i', wav, *vf,
                '-c:v', 'libx264', '-profile:v', 'high', '-pix_fmt', 'yuv420p', '-crf', '16', '-preset', 'slow', '-r', str(FPS),
                '-c:a', 'aac', '-b:a', '256k', '-movflags', '+faststart', '-shortest', mp4], check=True)
probe = subprocess.run(['ffprobe', '-v', 'error', '-show_entries', 'stream=codec_name,pix_fmt,r_frame_rate,width,height,sample_rate,channels:format=duration,size', '-of', 'json', mp4], capture_output=True, text=True).stdout
# ---- sync, proved on the finished file from three independent sides
import numpy as np
#  (a) the file's sound against the samples the simulator computed: the lag that best lines them up
dec = np.frombuffer(subprocess.run(['ffmpeg', '-loglevel', 'error', '-i', mp4, '-map', '0:a', '-ac', '1', '-ar', str(RATE), '-f', 's16le', '-'], capture_output=True).stdout, dtype=np.int16).astype(np.float64)
ref = (np.frombuffer(out.tobytes(), dtype=np.int16).astype(np.float64).reshape(-1, 2)).mean(axis=1)
a0, seg = 2 * RATE, 4 * RATE                      # four seconds from the middle of the clip
lags = range(-2400, 2401)
best_lag = max(lags, key=lambda L: float(np.dot(ref[a0:a0 + seg], dec[a0 + L:a0 + L + seg])))
#  (b) the file's picture against the telemetry: the frames at which the gear numeral's window changes most
gx, gy, gw, gh = gear_box
raw = subprocess.run(['ffmpeg', '-loglevel', 'error', '-i', mp4, '-map', '0:v', '-vf', f'crop={gw}:{gh}:{gx}:{gy},format=gray', '-f', 'rawvideo', '-'], capture_output=True).stdout
cells = np.frombuffer(raw, dtype=np.uint8).reshape(-1, gh, gw).astype(np.int16)
diff = np.abs(cells[1:] - cells[:-1]).mean(axis=(1, 2))
# around each telemetry gear change, the first frame whose window differs clearly from the frame before
def first_change(s_):
    lo, hi = max(0, s_ - 4), min(len(diff), s_ + 4)
    top = diff[lo:hi].max()
    return next((k + 1 for k in range(lo, hi) if diff[k] > 0.25 * top), None)
picture_shift_frames = [first_change(s_) for s_ in shifts]
#  (c) the telemetry against the sample clock: engine time shown in frame i, less the time of the samples computed up to it
et = [r[9] for r in log]
lag_ms = [1000 * ((et[i] - et[0]) - i / FPS) for i in range(total)]
sync = {
    'file_frames': int(cells.shape[0]), 'file_audio_samples': int(dec.shape[0]),
    'audio_lag_samples_file_vs_computed': int(best_lag),
    'gear_change_frames_telemetry': shifts, 'gear_numeral_change_frames_in_file_picture': picture_shift_frames,
    'telemetry_time_minus_sample_clock_ms': {'min': round(min(lag_ms), 2), 'max': round(max(lag_ms), 2)},
}
info = {
    'engine': A.engine, 'overrides': json.loads(A.overrides) if A.overrides else None, 'label': label, 'file': mp4, 'poster': poster, 'poster_frame': poster_frame, 'frames': total, 'seconds': total / FPS,
    'timeline': {'space_down_frame': n_idle, 'space_up_frame': n_idle + n_hold, 'space_down_frames': downs, 'space_up_frames': ups, 'key_frames': keys},
    'blowoff': A.blowoff, 'whistle': A.whistle,
    'top_gear_reached': max(gear), 'peak_rpm': round(max(r[0] for r in log)), 'top_speed_kmh': round(max(r[2] for r in log) * 3.6),
    'upshift_frames': shifts, 'sync': sync,
    'sound': {'samples': n, 'seconds': n / RATE, 'peak_before_gain': round(peak, 4), 'gain': round(g, 3), 'samples_at_full_scale': clipped},
    'checks': {'frames_not_drawn': bad_frames, 'frames_not_finite': not_finite, 'page_errors': errs[:5]},
    'ffprobe': json.loads(probe), 'render_minutes': round((time.time() - t_start) / 60, 1), 'screenshot_ms_per_frame': round(shot_ms / total),
    'per_frame': [[round(r[0]), r[1], round(r[2], 2), r[3], round(r[8], 2), round(r[5], 4)] for r in log],
}
json.dump(info, open(os.path.join(A.out, stem + '.json'), 'w'))
if not A.keep_frames:
    shutil.rmtree(frames_dir, ignore_errors=True)
os.remove(wav)
print(json.dumps({k: v for k, v in info.items() if k not in ('per_frame', 'ffprobe')}, indent=1))
ok = not bad_frames and not not_finite and not errs and clipped == 0 and abs(n / RATE - total / FPS) < 1e-6
ok = ok and abs(best_lag) < SPF and picture_shift_frames == shifts and max(abs(v) for v in lag_ms) < 1000 / FPS + 0.01
print('OK' if ok else 'CHECK FAILED')
sys.exit(0 if ok else 1)
