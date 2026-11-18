# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy", "matplotlib", "scipy"]
# ///
"""Level against time and a spectrogram for lift-off renders.

usage: uv run tools/liftoff_plot.py <name> [<name> ...]
Each <name> is a path without extension: <name>.wav and <name>.csv as written by
`engsim liftoff <engine> <quality> <level> --wav <name>.wav --csv <name>.csv`; writes <name>.png.
The wav starts 0.5 s before the lift; the csv's times count from the lift.
"""
import csv
import sys

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
from scipy.io import wavfile
from scipy.signal import spectrogram

PRE = 0.5

for name in sys.argv[1:]:
    rate, data = wavfile.read(name + ".wav")
    x = data.astype(np.float64).mean(axis=1) / 32768.0
    level, pops, rpm = [], [], []
    with open(name + ".csv") as f:
        for row in csv.DictReader(f):
            {"level": level, "pop": pops, "rpm": rpm}[row["kind"]].append((float(row["t"]), float(row["value"])))
    level, pops, rpm = np.array(level), np.array(pops).reshape(-1, 2), np.array(rpm)
    fig, ax = plt.subplots(3, 1, figsize=(11, 8.5), sharex=True, gridspec_kw={"height_ratios": [2, 2, 1]})
    ax[0].plot(level[:, 0], level[:, 1], lw=0.8, color="#2a6f97")
    if len(pops):
        ax[0].vlines(pops[:, 0], -45, pops[:, 1], color="#c1121f", lw=0.8, alpha=0.6)
        ax[0].plot(pops[:, 0], pops[:, 1], "v", color="#c1121f", ms=5, label=f"{len(pops)} ignitions in the model (loudest 10 ms in the 30 ms after each)")
        ax[0].legend(loc="upper right", fontsize=8, frameon=False)
    ax[0].axhline(0, color="#888", lw=0.6, ls="--")
    ax[0].set_ylim(-45, 25)
    ax[0].set_ylabel("exhaust level, 5 ms windows\ndB against loudest 10 ms at full throttle")
    ax[0].set_title(name.split("/")[-1] + ": throttle shut at 0 s (exhaust microphone; lower panels from the mixed wav)", fontsize=10)
    f_, t_, s_ = spectrogram(x, fs=rate, nperseg=1024, noverlap=896)
    ax[1].pcolormesh(t_ - PRE, f_, 10 * np.log10(s_ + 1e-14), shading="auto", cmap="magma", vmin=-110, vmax=-40)
    ax[1].set_ylim(0, 6000)
    ax[1].set_ylabel("frequency, Hz")
    ax[2].plot(rpm[:, 0], rpm[:, 1], color="#333", lw=1)
    ax[2].set_ylabel("rpm")
    ax[2].set_xlabel("seconds after the throttle shuts")
    ax[2].set_xlim(-PRE, level[-1, 0])
    for a in ax:
        a.grid(alpha=0.25, lw=0.5)
    fig.tight_layout()
    fig.savefig(name + ".png", dpi=110)
    plt.close(fig)
    print("wrote", name + ".png")
