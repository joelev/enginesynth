"""Band levels of engsim dumps. usage: bands.py name=rate=file ..."""
import sys, numpy as np
bands=[(20,100),(100,300),(300,1000),(1000,2000),(2000,4000),(4000,8000),(8000,16000)]
print('quality     total  '+'  '.join(f'{lo}-{hi}' for lo,hi in bands))
for arg in sys.argv[1:]:
    q,fs,path=arg.split('='); fs=float(fs)
    a=np.fromfile(path,dtype='<f8').reshape(-1,4); x=a[:,0]+a[:,1]
    sp=np.abs(np.fft.rfft(x*np.hanning(len(x))))**2; f=np.fft.rfftfreq(len(x),1/fs)
    tot=(x**2).mean(); k=tot/sp.sum()
    print(f'{q:11s} {10*np.log10(tot/4e-10):6.1f}  '+'  '.join(f'{10*np.log10(max(sp[(f>=lo)&(f<hi)].sum()*k,1e-30)/4e-10):6.1f}' for lo,hi in bands), '' if np.isfinite(a).all() else ' NOT FINITE')
