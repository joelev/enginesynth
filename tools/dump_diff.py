#!/usr/bin/env python3
"""Compare two `engsim dump` files (raw little-endian f64 [exL, exR, intake, block] per inner sample).

usage: python3 tools/dump_diff.py <reference.bin> <test.bin> <inner-rate-Hz>

Prints, for the exhaust (L+R) and intake signals:
  - waveform error: peak and RMS of (test - reference), in dB relative to the reference RMS
    (a chaotic model can drift in phase, so this is a strict measure; spectra are the fair one)
  - band levels: third-octave bands 50 Hz to 16 kHz, each in dB, and the test minus the
    reference per band; mean absolute and worst difference over bands that are within 60 dB
    of the loudest band (quieter bands are below anything audible in the mix)
  - the level of the difference signal per band, relative to the reference band (the "noise
    floor" a change adds), worst band
"""
import sys
import numpy as np


def load(path):
    a = np.fromfile(path, dtype="<f8")
    return a.reshape(-1, 4)


def bands(x, fs):
    n = len(x)
    w = np.hanning(n)
    spec = np.abs(np.fft.rfft(x * w)) ** 2
    f = np.fft.rfftfreq(n, 1 / fs)
    centres = [1000 * 2 ** (k / 3) for k in range(-13, 13)]  # 50 Hz .. 16 kHz
    out = []
    for c in centres:
        lo, hi = c / 2 ** (1 / 6), c * 2 ** (1 / 6)
        if hi > fs / 2:
            break
        m = (f >= lo) & (f < hi)
        out.append((c, 10 * np.log10(spec[m].sum() + 1e-30)))
    return out


def main():
    ref, tst, fs = load(sys.argv[1]), load(sys.argv[2]), float(sys.argv[3])
    n = min(len(ref), len(tst))
    ref, tst = ref[:n], tst[:n]
    for name, sel in (("exhaust", lambda a: a[:, 0] + a[:, 1]), ("intake", lambda a: a[:, 2])):
        r, t = sel(ref), sel(tst)
        rms = np.sqrt(np.mean(r * r))
        d = t - r
        peak_db = 20 * np.log10(np.max(np.abs(d)) / rms + 1e-30)
        rms_db = 20 * np.log10(np.sqrt(np.mean(d * d)) / rms + 1e-30)
        br, bt, bd = bands(r, fs), bands(t, fs), bands(d, fs)
        top = max(v for _, v in br)
        keep = [i for i, (_, v) in enumerate(br) if v > top - 60]
        diffs = [bt[i][1] - br[i][1] for i in keep]
        floor = [bd[i][1] - br[i][1] for i in keep]
        worst = max(keep, key=lambda i: abs(bt[i][1] - br[i][1]))
        print(f"{name:8} waveform error peak {peak_db:7.1f} dB, rms {rms_db:7.1f} dB re signal rms | "
              f"bands: mean |diff| {np.mean(np.abs(diffs)):.2f} dB, worst {bt[worst][1] - br[worst][1]:+.2f} dB at {br[worst][0]:.0f} Hz | "
              f"difference signal, worst band {max(floor):6.1f} dB re that band")


if __name__ == "__main__":
    main()
