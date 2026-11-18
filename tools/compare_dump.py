"""Compare two engsim dumps: how far apart are they, per channel, in dB relative to the signal."""
import sys, numpy as np
a = np.fromfile(sys.argv[1], dtype='<f8').reshape(-1, 4); b = np.fromfile(sys.argv[2], dtype='<f8').reshape(-1, 4)
n = min(len(a), len(b)); a, b = a[:n], b[:n]
names = ['exhaust L', 'exhaust R', 'intake', 'block']
out = []
for c in range(4):
    s = np.sqrt((a[:, c]**2).mean()); d = np.sqrt(((a[:, c]-b[:, c])**2).mean())
    q = n // 4
    early = np.sqrt(((a[:q, c]-b[:q, c])**2).mean())
    out.append(f"{names[c]}: diff {20*np.log10(max(d,1e-300)/s):.0f} dB (first quarter {20*np.log10(max(early,1e-300)/s):.0f} dB), level change {20*np.log10(np.sqrt((b[:,c]**2).mean())/s):+.2f} dB")
print(' | '.join(out))
