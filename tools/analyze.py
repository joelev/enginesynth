"""Quick numeric look at a render: level over time, spectral peaks vs firing frequency, and a spectrogram PNG."""
import sys, struct, subprocess, numpy as np, csv, os
d = sys.argv[1]
def wav(path):
    b = open(path, 'rb').read()
    fmt, ch, rate = struct.unpack('<HHI', b[20:28]); bits = struct.unpack('<H', b[34:36])[0]
    raw = b[44:]
    a = np.frombuffer(raw, dtype='<f4' if fmt == 3 else '<i2').astype(np.float64)
    if fmt != 3: a /= 32768
    return a.reshape(-1, ch), rate
log = list(csv.DictReader(open(os.path.join(d, 'log.csv'))))
t = np.array([float(r['t']) for r in log]); rpm = np.array([float(r['rpm']) for r in log]); tq = np.array([float(r['torque']) for r in log])
ex, sr = wav(os.path.join(d, 'stem_exhaust.wav')); it, _ = wav(os.path.join(d, 'stem_intake.wav')); bl, _ = wav(os.path.join(d, 'stem_block.wav'))
db = lambda x: 20*np.log10(max(np.sqrt(np.mean(x**2)), 1e-12)/2e-5)
print(f"finite: ex {np.isfinite(ex).all()} intake {np.isfinite(it).all()} block {np.isfinite(bl).all()}")
step = max(1, len(t)//16)
print("   t    rpm  torque  ex dB  in dB  blk dB")
for i in range(0, len(t), step):
    a, b = int(t[i]*sr), int(min(t[i]+0.25, t[-1])*sr)
    if b <= a: continue
    print(f"{t[i]:5.2f} {rpm[i]:6.0f} {tq[i]:7.1f} {db(ex[a:b]):6.1f} {db(it[a:b]):6.1f} {db(bl[a:b]):6.1f}")
if len(sys.argv) > 3:
    t0 = float(sys.argv[2]); n_cyl = int(sys.argv[3])
    a = int(t0*sr); seg = ex[a:a+sr//2].mean(axis=1) * np.hanning(sr//2)
    sp = np.abs(np.fft.rfft(seg)); f = np.fft.rfftfreq(len(seg), 1/sr)
    r = np.interp(t0+0.25, t, rpm); f_cycle = r/120
    print(f"\nspectrum at t={t0}s, {r:.0f} rpm: cycle {f_cycle:.1f} Hz, firing {f_cycle*n_cyl:.1f} Hz")
    idx = np.argsort(sp)[::-1]; seen = []
    for i in idx:
        if all(abs(f[i]-s) > 6 for s in seen):
            seen.append(f[i]); print(f"  {f[i]:7.1f} Hz  order {f[i]/f_cycle:5.2f} x cycle   {20*np.log10(sp[i]/sp.max()):6.1f} dB")
        if len(seen) >= 10: break
    bands = [(20,100),(100,300),(300,1000),(1000,3000),(3000,8000),(8000,20000)]
    tot = (sp**2).sum()
    print("  band energy: " + "  ".join(f"{lo}-{hi}: {10*np.log10((sp[(f>=lo)&(f<hi)]**2).sum()/tot):.0f} dB" for lo, hi in bands))
subprocess.run(['ffmpeg','-y','-loglevel','error','-i',os.path.join(d,'mix.wav'),'-lavfi','showspectrumpic=s=1400x500:legend=1:scale=log:fscale=log:start=20:stop=16000','-frames:v','1','-update','1',os.path.join(d,'spectrogram.png')])
