"""Mean band-level error of engsim dumps against the final-quality reference, over a set of cases.
usage: accuracy.py <golden-dir> name=prefix=rate ...   (files <prefix>_<case>.bin, reference ref_<case>.bin at 192 kHz)"""
import sys, numpy as np
G=sys.argv[1]
bands=[(20,100),(100,300),(300,1000),(1000,2000),(2000,4000),(4000,8000)]
cases=["v8_cross_4000","v8_cross_6500","i4_8500","flat6_3000","i4_3000","flat6_7000","v12_5000"]
def lv(path,fs,ch):
    a=np.fromfile(path,dtype='<f8').reshape(-1,4)
    if not np.isfinite(a).all(): raise SystemExit('not finite: '+path)
    x=a[:,0]+a[:,1] if ch=='ex' else a[:,2]
    sp=np.abs(np.fft.rfft(x*np.hanning(len(x))))**2; f=np.fft.rfftfreq(len(x),1/fs); k=(x**2).mean()/sp.sum()
    return np.array([10*np.log10(max(sp[(f>=lo)&(f<hi)].sum()*k,1e-30)/4e-10) for lo,hi in bands])
for ch,title in [('ex','EXHAUST'),('in','INTAKE')]:
    ref={c:lv(f'{G}/ref_{c}.bin',192000,ch) for c in cases}
    print(f'{title} vs final   mean|err| per band (dB): '+' '.join(f'{lo}-{hi}' for lo,hi in bands)+'   overall   mean signed')
    for arg in sys.argv[2:]:
        name,pre,fs=arg.split('=')
        d=np.array([lv(f'{G}/{pre}_{c}.bin',float(fs),ch)-ref[c] for c in cases])
        print(f'{name:14s}  '+'  '.join(f'{v:5.1f}' for v in np.abs(d).mean(axis=0))+f'   {np.abs(d).mean():5.2f}    '+'  '.join(f'{v:+5.1f}' for v in d.mean(axis=0)))
