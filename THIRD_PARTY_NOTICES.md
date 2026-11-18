# Third-party notices

No third-party source code is included in this repository. The components below are fetched by the build or used as tools, under their own licenses.

## Compiled into the simulator (`engsim` and `live/engsim.wasm`)

Rust crates, fetched by Cargo at the versions pinned in `sim/Cargo.lock`:

| Crate | License |
|---|---|
| serde, serde_core, serde_derive | MIT OR Apache-2.0 |
| serde_json | MIT OR Apache-2.0 |
| itoa | MIT OR Apache-2.0 |
| zmij | MIT |
| memchr | Unlicense OR MIT |
| proc-macro2, quote, syn (build-time only) | MIT OR Apache-2.0 |
| unicode-ident (build-time only) | (MIT OR Apache-2.0) AND Unicode-3.0 |

`live/engsim.wasm` is a build of this repository's simulator and contains compiled code from the first group of crates.

## Shipped with the web app

[three.js](https://threejs.org/) 0.170.0, MIT License, Copyright © 2010-2024 three.js authors. It is installed with `npm install --prefix viz/vendor.nosync three@0.170.0` and is not in this repository. `tools/build_site.py` copies the three.js module build, the add-ons the page imports (OrbitControls, RoomEnvironment, Reflector, EffectComposer, RenderPass, OutputPass, UnrealBloomPass) and three.js's `LICENSE` file into the built site.

The page uses the visitor's system fonts. No font files are included or fetched.

## Tools used by the scripts in `tools/` (not distributed)

- Python packages: NumPy, SciPy, Matplotlib, Pillow, Playwright (which downloads its own Chromium, WebKit and Firefox builds).
- [FFmpeg](https://ffmpeg.org/), run as an external program by the video renderers to encode MP4 files.
- Node.js, to run the JavaScript tests against the WebAssembly build.

## Data

The research files in `engines/research/` and the `sources` and `notes` of the definitions in `engines/defs/` cite public web pages, manuals, brochures and articles by URL, with short quotations. Those quotations remain the work of their authors.
