"""How far each lift-off pop stands above the same lift without pops.

usage: python3 tools/liftoff_prominence.py <run-with-pops.csv> <same-run-at-level-0.csv>
Both are csv files from `engsim liftoff <engine> <quality> <level> --csv ...`. For each ignition:
the loudest 5 ms window in the 30 ms after it, minus the median level of the level-0 run in the
100 ms around the same moment (both in dB against full throttle). A numeric stand-in for "would
it be heard over what the exhaust is doing anyway"; nobody has listened.
"""
import csv
import statistics
import sys


def load(path):
    level, pops = [], []
    with open(path) as f:
        for row in csv.DictReader(f):
            if row["kind"] == "level":
                level.append((float(row["t"]), float(row["value"])))
            elif row["kind"] == "pop":
                pops.append(float(row["t"]))
    return level, pops


run, pops = load(sys.argv[1])
base, _ = load(sys.argv[2])
out = []
for t in pops:
    peak = max(v for (x, v) in run if t <= x <= t + 0.030)
    floor = statistics.median(v for (x, v) in base if t - 0.050 <= x <= t + 0.050)
    out.append((t, peak, floor, peak - floor))
print(sys.argv[1])
for t, peak, floor, prom in out:
    print(f"  {1000 * t:5.0f} ms: peak {peak:+5.1f} dB, level-0 run there {floor:+5.1f} dB, stands {prom:+5.1f} dB above it")
if out:
    print(f"  {sum(1 for o in out if o[3] >= 6)} of {len(out)} stand 6 dB or more above the level-0 run")
