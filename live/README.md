# live/

The playable engine synthesizer. Serve the repository root with any static server and open `/live/`:

```
python3 -m http.server 47613 --bind 127.0.0.1
```

- `engsim.wasm`: the simulator, built by `tools/build_wasm.sh` from `sim/`.
- `worklet.js`: runs it on the audio thread.
- `engine.js`: `LiveEngine`, the page-side handle (throttle, gears, engine swaps, telemetry frames).
- Fueling on `LiveEngine` (remembered across engine swaps, like throttle and drive mode): `setMixture(phi)` sets the fuel-air equivalence ratio (1 stoichiometric, clamped to 0.7..1.7); `setOverrunFuel(x)` the share of overrun cycles that are fueled instead of cut (0..1; fueled ones burn late and pop); `setLimiter('fuel' | 'spark')` the rev limiter type. Getters `mixture`, `overrunFuel`, `limiter`. The mock engine accepts and ignores them.
- Afterfire telemetry: the header's `afterfire: {offset, count, glow_ms}` gives, for each exhaust group (collector), one value per frame at `frame[offset + g]`: the afterfire heat-release rate there, 0..1, held with a `glow_ms` decay so a 1.5 ms flash shows at 60 fps. It is updated at the moment a strobe frame is shown, not at its crank phase. Group `g` is the `group` field of that group's primaries in `pipes`, and its collector is the volume `collector<g+1>`. Engines with a pipe per cylinder (the two-strokes) have one value, always 0.
- `engine.js`: `LiveEngine`, the page-side handle (throttle, gears, engine swaps, telemetry frames). `LiveEngine.preview(engine, overrides)` returns `{header, frame}` without an audio context; `setQuality('auto' | 'light' | 'heavy')` rebuilds at another level; `level` is the running level; a `stats` event follows each load report; `LiveEngine.slow = k` (test hook, set before `create`) makes the simulator k times as costly.
- `og.jpg` and `og/<engine id>.jpg`: the link-preview images of the root page and of each engine's page (`e/<id>/`, made by `tools/build_site.py`), rendered by `tools/render_og_all.py`.
- `controls.css`: the key-cap controls; part 1 is the skin (custom properties on one root class), part 2 layout and motion.
- `firstrun.js`: the first-run teaching sequence (`?firstrun=1` forces it, `?firstrun=0` skips it).
- The rest is the interface.

The audio context must run at 48 kHz: `new AudioContext({sampleRate: 48000})`.
