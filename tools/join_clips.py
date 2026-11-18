"""Join clips into one mp4, levelling each clip's loudness first.
usage: join_clips.py out.mp4 clip1.mp4 clip2.mp4 ..."""
import subprocess, sys
out, clips = sys.argv[1], sys.argv[2:]
cmd = ['ffmpeg', '-y', '-loglevel', 'error']
for c in clips: cmd += ['-i', c]
f = ''.join(f'[{i}:a]loudnorm=I=-16:TP=-1.5:LRA=11,aresample=48000[a{i}];' for i in range(len(clips)))
f += ''.join(f'[{i}:v][a{i}]' for i in range(len(clips))) + f'concat=n={len(clips)}:v=1:a=1[v][a]'
cmd += ['-filter_complex', f, '-map', '[v]', '-map', '[a]', '-c:v', 'libx264', '-crf', '18', '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-b:a', '192k', '-movflags', '+faststart', out]
sys.exit(subprocess.run(cmd).returncode)
