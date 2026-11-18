# Engine Synthesizer

Engine Synthesizer makes engine sound by simulating the gas inside an engine. Its simulator, the Rust crate and command-line program in `sim/`, is called engsim. It plays no recordings. Each cylinder is a volume of gas with a moving piston; burning fuel releases heat; valves open and close; pressure waves travel, reflect and steepen in the intake and exhaust pipes; and the sound is computed from the changing flow out of the tailpipe and the intake, as it would be heard a metre away.

It runs as a command-line program, as a WebAssembly module in a browser (playable at <https://joeljohnson.com/enginesynth/>), and through an offline video renderer that draws the simulated engine with its pipes coloured by gas pressure.

The library holds 86 engines: 60 modelled on real engines from public data, and 26 generic engines of typical proportions. 77 are four-stroke piston engines, 5 two-strokes and 4 Wankel rotaries; 7 are supercharged or turbocharged; 59 carry the vehicle they came in, for Drive. `engsim list` prints them.

Contents: [How the sound is made](#how-the-sound-is-made) · [Code layout](#code-layout) · [Building and running](#building-and-running) · [Engine definitions](#engine-definitions) · [Accuracy against published figures](#accuracy-against-published-figures) · [Limitations](#limitations) · [Credits](#credits) · [License](#license)

## How the sound is made

This is an overview. [docs/physics.md](docs/physics.md) gives the equations and constants as the code implements them.

### Gas and pipes

- **One ideal gas** with γ = 4/3 and R = 287 J/(kg K). The value of γ sits between cold air and burned gas, and it turns every isentropic exponent into a whole number or a power of two, so the gas relations need no `pow` calls.
- **Pipes on a grid.** Each intake and exhaust pipe can be solved as quasi-one-dimensional compressible flow with varying cross-section: a finite-volume scheme with second-order MUSCL-Hancock reconstruction, HLLC fluxes, Darcy wall friction and wall heat transfer from the Reynolds analogy. Waves travel, reflect at area changes and open ends, steepen and interfere because the equations make them do so.
- **Pipes along characteristics** (the `+W` quality levels). A uniform pipe of gas at one entropy carries two Riemann variables unchanged along its characteristics, so it can be solved as two delay lines with no cells. Each sample's delay follows from its own wave speed, so waves still steepen; where samples would overtake, their arrival times are merged into a shock. Viscous and thermal boundary-layer loss is Kirchhoff's wide-tube law. These pipes are faster and sharper than the grid, but carry one entropy per pipe, so exhaust primaries (which carry hot slugs from the valve) stay on the grid.
- **Open ends** radiate through Flanagan's resistance-and-inductance radiation load. Each opening to the air is a monopole source: the pressure at the microphone is ρ₀ · dQ/dt / (4πr), where Q is the volume flow out of the opening and r is 1 m.

### Cylinders

- **Zero-dimensional cylinders.** Each cylinder is one zone of gas. Slider-crank kinematics set its volume; compression and expansion are isentropic within a step; the work goes to the crank.
- **Valves** are compressible orifices on the valve curtain area, with lift following a smooth curve placed to reproduce the published cam events where those exist. Each valve exchanges gas with a fixed 35 mm of port, so valve flow does not depend on the grid.
- **Combustion** follows a Wiebe curve. The burn duration depends on speed, load and residual gas, and varies from cycle to cycle; spark advance follows the burn duration as an engine map would. Mixture strength changes heat release, burn speed and misfire.
- **Heat loss** to the cylinder walls follows Woschni's correlation.
- **Friction** is a mean effective pressure that rises with engine speed.

### Engine kinds

- **Four-stroke piston engines** with any number of cylinders and banks: inline, V, flat, VR, W and radial layouts are one model with different geometry. Firing angles are given per cylinder, so cross-plane and flat-plane V8s differ only in those angles.
- **Two-strokes**, with crankcase compression, reed, piston-port or disc induction, piston-controlled exhaust and transfer ports, a scavenging model, and tapered expansion chambers whose returning wave produces the power band.
- **Wankel rotaries**, with three chambers per rotor on a 1080-degree cycle, sinusoidal chamber volume and ports that the rotor uncovers.

### Forced induction

- **Roots and twin-screw superchargers** are modelled as rotor pockets, each with its own gas, carried from inlet to outlet at a fixed ratio to the crank. When a Roots pocket opens to the outlet at inlet pressure, outlet gas rushes back into it. Those backflow pulses are the whine, at pockets × rotor speed, and the work done on the pocket gas is the drive torque, so the blower's inefficiency also comes out of the model. Leakage, a bypass valve and an intercooler are orifices.
- **Turbochargers.** The compressor is a non-dimensional speed line: the Moore-Greitzer cubic left of the surge line and through reverse flow, a parabola to choke on the right, with pressure ratio scaling with tip speed. The flow through it is a state of a Greitzer duct, so surge can occur when the throttle shuts. The turbine is a quasi-steady nozzle with an efficiency that depends on blade speed ratio. The shaft is one inertia, so turbo lag emerges rather than being scripted. A wastegate (spring and diaphragm, with optional electronic boost control) and a blow-off valve (venting to the air through a radiating pipe, or recirculating) complete it. Parallel twin turbos are supported.
- **Turbo whistle** cannot come out of a one-dimensional model, which has no blades. It is a stand-in: tones at the blade-pass frequencies of the simulated shaft speed, scaled by the simulated pressure rise and tip speed.

### Fueling, afterfire and Drive

- **Afterfire.** Unburned fuel and air leave the cylinders with the exhaust and collect in each exhaust collector. When they ignite, the heat goes into the collector volume and the pressure pulse travels and radiates like everything else. A rich mixture, fuel on the overrun, a spark-cut limiter, or fuel left on the port walls after a lift ("lift-off pops") can each make it happen.
- **Vehicles in Drive.** In Drive the engine pushes a vehicle through a slipping clutch, a gearbox with automatic shifts, a driveline loss, tyres whose force rises with slip to a peak and then falls, rolling resistance and aerodynamic drag. A real engine's definition may describe the car or motorcycle it came in (weight, gear ratios, final drive, tyre, driven wheels); other engines push a generic car scaled to their displacement.

### What is not first-principles

These parts are stand-ins rather than physics:

- Turbulence: band-limited noise on exhaust valve flow, and Lighthill's eighth-power jet-noise law at each tailpipe.
- Structure noise: the rate of cylinder pressure rise ringing three invented "block" resonances, mixed low.
- Turbo whistle, described above.
- A treble loss in the characteristic pipes, fitted to match the grid solver; without it, shocks that are real in the model reach the tailpipe and radiate as clicks.
- The idle control, rev limiter and overrun fuel cut imitate an engine computer.
- The mix: level riding and a soft clip.

## Code layout

```
sim/                 the simulator, in Rust (one crate: library, CLI, WebAssembly C ABI)
  src/gas.rs         gas constants, HLLC flux, pipe-end boundary states, orifice flow
  src/pipe.rs        grid pipes, open-end radiation load
  src/wave.rs        pipes along characteristics, boundary-layer loss, muffler terms
  src/engine.rs      cylinders, combustion, heat, valves, plumbing, sound sources, crank, Drive
  src/boost.rs       Roots and twin-screw superchargers
  src/turbo.rs       turbochargers
  src/spec.rs        the engine-definition format and its derived defaults
  src/live.rs        the real-time session used by the browser, and its WebAssembly C ABI
  src/lib.rs         the crate root and the quality levels
  src/audio.rs       decimation to 48 kHz, WAV output
  src/prof.rs        optional section timers (cargo build --features prof)
  src/main.rs        the engsim CLI; *check.rs hold the CLI's test rigs
  build.rs           embeds engines/defs into the binary
engines/
  defs/              one JSON definition per engine; ORDER lists them
  research/          the sourced data behind each real engine (format: SCHEMA.md)
  fixtures/          test-only definitions, never in the app
  VALIDATION.md      simulated against published torque and power
live/                the web app (HTML, JavaScript, the AudioWorklet, engsim.wasm, link-preview images)
viz/                 three.js engine model shared by the app and the video renderer
scenes/              scene files for offline renders
tools/               build, test and render scripts
docs/                physics.md, engine-definitions.md
```

## Building and running

Requirements: Rust (stable, through [rustup](https://rustup.rs/)), Python 3, Node.js and npm. The browser tests and the renderers also need `pip install playwright numpy pillow` and `playwright install chromium`; the renderers need FFmpeg on the path. A few analysis scripts use SciPy and Matplotlib.

Build output goes to folders ending in `.nosync` (`sim/target.nosync`, `out.nosync`, `dist.nosync`, `viz/vendor.nosync`), which git ignores.

### The simulator and its CLI

```sh
cd sim
cargo build --release
./target.nosync/release/engsim list                     # every engine in the library
./target.nosync/release/engsim selftest                 # tube resonance and mass conservation
./target.nosync/release/engsim libtest                  # definition-format checks
./target.nosync/release/engsim dyno v8_cross            # torque and power at full throttle across speeds
./target.nosync/release/engsim idle                     # every engine idles and recovers from a blip (several minutes)
./target.nosync/release/engsim validate                 # real engines against their published figures (a few minutes)
./target.nosync/release/engsim drive gm_ls3             # a standing start in the engine's own car
./target.nosync/release/engsim spec honda_b18c5         # a definition with every derived field
```

Running `engsim` with no arguments prints the main commands. The others are `idle`, `drive`, `turbo` (the turbocharger rigs), `trace` (one cycle of pressures), `bench`, `live` and `dump`; `sim/src/main.rs` documents their arguments.

Quality levels set the internal sample rate and the pipe solver:

| Level | Rate | Pipes |
|---|---|---|
| `final` | 192 kHz | grid, 8 mm exhaust and 5 mm intake cells |
| `live96`, `live48` | 96 or 48 kHz | grid, cells sized to each pipe's wave speed |
| `live96+W`, `live48+W` | 96 or 48 kHz | characteristics wherever possible, grid for exhaust primaries |

The browser uses `live48+W` (light) and `live96+W` (heavy), each with two arithmetic shortcuts (`+m`, `+r`) whose differences from the plain levels are at the level of rounding.

### Offline renders

A scene file sets the engine, the throttle over time, the mode (`free`, `dyno` or `drive`), the quality and any definition overrides:

```sh
sim/target.nosync/release/engsim render scenes/v8_cross_revs.json out.nosync/v8_cross_revs   # stems, mix.wav, telemetry
python3 tools/analyze.py out.nosync/v8_cross_revs 6.0 8                                    # levels, spectrum, spectrogram
```

### The web app

```sh
sh tools/build_wasm.sh                              # rebuilds live/engsim.wasm (only needed after changing sim/ or engines/)
npm install --prefix viz/vendor.nosync three@0.170.0
python3 tools/serve.py 47613                        # run from the repository root; serves it
```

Then open <http://127.0.0.1:47613/live/> in a current version of Chrome, Edge, Firefox or Safari (16.4 or later); it runs best on a desktop computer. The page needs WebAssembly SIMD, AudioWorklet and WebGL 2. `tools/build_wasm.sh` needs the `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`). Hold Space for throttle (1 to 9 and 0 for part and full throttle); D toggles Drive and Neutral; Q and E shift; the arrow keys step through the engines. The Workshop panel rebuilds the running engine with other pipe lengths, cam overlap and so on.

A built copy of `live/engsim.wasm` is committed, so the page runs without a Rust toolchain.

Tests:

```sh
node tools/live_test.mjs                 # the real-time session, run the way the audio thread runs it
python3 tools/live_browser_test.py       # headless Chromium: real AudioContext, worklet and WebAssembly
python3 tools/app_all_engines.py         # every engine loads and runs in the app
```

To build a self-contained static site and check it:

```sh
python3 tools/build_site.py                          # writes dist.nosync/enginesynth
python3 tools/site_test.py --local dist.nosync
```

`build_site.py --url <address> --public` sets absolute link-preview addresses and drops the `noindex` tag.

### The video renderer

Two renderers draw the simulated engine with three.js in headless Chromium and mux the simulator's own sound with FFmpeg:

```sh
# a scene render, frame by frame from its telemetry (options in viz/README.md)
python3 tools/render_video.py out.nosync/v8_cross_revs out.nosync/v8_cross_revs.mp4 --params "view=front"
# the live page itself, rendered offline: a launch from rest in the engine's own car
python3 tools/render_live_video.py bmw_s54b32 --size 1920x1080
```

Neither is a screen recording: the page's clock is replaced so every frame is drawn for its instant, and the sound is the samples computed for those frames. At speed the crank turns too fast for 60 frames per second, so scene renders are strobed and say so on screen.

## Engine definitions

An engine is a JSON file of geometry and timing: bore, stroke, rod length, compression ratio, firing angles, valve sizes and events, pipe lengths and diameters, and optional blocks for boost and a vehicle. Anything left out is derived from bore, stroke and cylinder count. [docs/engine-definitions.md](docs/engine-definitions.md) describes the format.

**Sourcing rule.** A definition models a real engine only if public sources give at least its bore, stroke, compression ratio, layout and firing pattern. Every number taken from a source is traced to it: the URL is in the definition's `sources` and in its research file in `engines/research/`, which records where each value was read and what could not be found. Every other number is an estimate and is listed as one in the definition's `notes`. To bring simulated output near the published figures, only estimated fields are tuned; a sourced number is never changed to make the output fit. Engines without enough public data are kept as generic engines and labelled as such.

## Accuracy against published figures

`engines/VALIDATION.md` is written by `engsim validate --write`: each real engine at full throttle, quality `live96`, at its published peak-torque and peak-power speeds. Of the 58 engines with published figures, 47 are within 5% of both published torque and power and 55 within 10%. Further off: `yamaha_r1_crossplane` (-12%, -16%), `ducati_desmosedici_stradale` (-3%, -16%), `kawasaki_h2_750` (-14%, -18%).

| Engine | Modelled on | Torque: simulated vs published | Power: simulated vs published |
|---|---|---|---|
| `ktm_620_lc4e` | KTM 620 LC4-E, 609 cc (620 Duke), c. 1998 | 55 vs 60 N·m at 5500 rpm (-9%) | 38 vs 40 kW at 7000 rpm (-6%) |
| `harley_twin_cam_88` | Harley-Davidson Twin Cam 88 (1450 cc), carburetted, 1999-2006 | 109 vs 117 N·m at 3500 rpm (-6%) | 50 vs 49 kW at 5200 rpm (+2%) |
| `ducati_1098_testastretta` | Ducati Testastretta Evoluzione 1099 cc L-twin, Ducati 1098 / 1098 S, 2007-2008 | 121 vs 123 N·m at 8000 rpm (-2%) | 113 vs 119 kW at 9750 rpm (-5%) |
| `yamaha_cp2_689` | Yamaha CP2 689 cc parallel twin, MT-07 / FZ-07, 2014 on | 68 vs 68 N·m at 6500 rpm (+0%) | 54 vs 55 kW at 9000 rpm (-1%) |
| `triumph_bonneville_865` | Triumph Bonneville T100 865 cc air-cooled parallel twin, carburetted, 2005-2007 | 68 vs 68 N·m at 6000 rpm (+1%) | 47 vs 47 kW at 7250 rpm (-0%) |
| `kawasaki_ex500` | Kawasaki EX500 (Ninja 500R / GPZ500S) 498 cc parallel twin, 1987-2009 | 45 vs 46 N·m at 8500 rpm (-3%) | 43 vs 44 kW at 9800 rpm (-3%) |
| `triumph_daytona_675` | Triumph Daytona 675 674.8 cc inline-three, 2006-2008 (up to VIN 381274) | 66 vs 72 N·m at 11750 rpm (-8%) | 84 vs 92 kW at 12500 rpm (-9%) |
| `toyota_4a_ge` | Toyota 4A-GE first generation (big port, T-VIS), European specification without catalyst (10.0:1), AE86 Corolla GT, 1983 on | 144 vs 142 N·m at 5200 rpm (+1%) | 93 vs 91 kW at 6600 rpm (+2%) |
| `honda_b18c5` | Honda B18C5, Acura Integra Type R (US), 1997-2001 | 180 vs 176 N·m at 7300 rpm (+2%) | 142 vs 145 kW at 8000 rpm (-3%) |
| `honda_f20c` | Honda F20C, Honda S2000 AP1, Japanese specification (11.7:1) as launched in 1999 | 212 vs 218 N·m at 7500 rpm (-3%) | 180 vs 184 kW at 8300 rpm (-2%) |
| `honda_k20a` | Honda K20A, Integra TYPE R (DC5), Japanese specification (11.5:1), 2001-2004 | 201 vs 206 N·m at 7000 rpm (-3%) | 160 vs 162 kW at 8000 rpm (-1%) |
| `honda_k20c1` | Honda K20C1 2.0 L DOHC VTEC turbocharged inline-four, Civic Type R (FK8), US specification, 2017-2021 | 402 vs 400 N·m at 3500 rpm (+0%) | 228 vs 228 kW at 6500 rpm (+0%) |
| `porsche_944s_m44_40` | Porsche M44/40 2.5 L 16-valve inline-four, 944 S, 1987-1988 | 233 vs 230 N·m at 4300 rpm (+1%) | 138 vs 140 kW at 6000 rpm (-2%) |
| `bmw_s14b23` | BMW S14B23 with catalytic converter, E30 M3, 1986-1989 (195 PS) | 226 vs 230 N·m at 4750 rpm (-2%) | 141 vs 143 kW at 6750 rpm (-1%) |
| `vw_07k_25_cbta` | Volkswagen 2.5 L inline-five, engine code CBTA/CBUA (07K), 2008-2014 | 245 vs 240 N·m at 4250 rpm (+2%) | 125 vs 125 kW at 5700 rpm (-0%) |
| `audi_rr_20v_turbo` | Audi RR 2.2 L DOHC 20-valve turbocharged inline-five, Audi quattro 20V (Ur-quattro), 1989-1991 | 307 vs 309 N·m at 1950 rpm (-1%) | 161 vs 162 kW at 5900 rpm (-0%) |
| `bmw_s54b32` | BMW S54B32, E46 M3, 2000-2006 | 365 vs 365 N·m at 4900 rpm (-0%) | 247 vs 252 kW at 7900 rpm (-2%) |
| `toyota_2jz_ge` | Toyota 2JZ-GE without VVT-i, 1991-1997 (Supra, Aristo/GS 300, SC 300) | 297 vs 279 N·m at 4800 rpm (+6%) | 155 vs 162 kW at 5800 rpm (-4%) |
| `buick_320_straight8` | Buick 320 cu in Fireball straight-eight, Series 70 Roadmaster, 1950 Dynaflow rating | 360 vs 380 N·m at 2000 rpm (-5%) | 116 vs 113 kW at 3600 rpm (+2%) |
| `vw_vr6_aaa` | Volkswagen VR6 2.8 L 12-valve, engine code AAA, 1991-1999 | 238 vs 235 N·m at 4200 rpm (+1%) | 121 vs 128 kW at 5800 rpm (-6%) |
| `nissan_vq35de` | Nissan VQ35DE, 350Z base engine, 2003-2005 | 365 vs 371 N·m at 4800 rpm (-2%) | 222 vs 214 kW at 6200 rpm (+4%) |
| `buick_3800_l36` | Buick 3800 Series II L36, 1995-2005 | n/a | n/a |
| `buick_3800_l67` | GM 3800 Series II L67 supercharged V6 with the Generation III Eaton M90, 1998-2003 rating (Regal GS, Grand Prix GTP, Park Avenue Ultra, Bonneville SSEi) | 366 vs 380 N·m at 3600 rpm (-4%) | 170 vs 179 kW at 5200 rpm (-5%) |
| `buick_231_oddfire` | Buick 231 cu in V6, odd-fire, 1975 to early 1977 | n/a | n/a |
| `subaru_ej253` | Subaru EJ253 2.5 L SOHC flat-four with i-AVLS, US 2006 Legacy / Outback 2.5i | 227 vs 229 N·m at 4400 rpm (-1%) | 134 vs 130 kW at 6000 rpm (+2%) |
| `vw_type1_1600dp` | Volkswagen Type 1 1600 dual-port (1584 cc, European 50 PS, engine code AD/AS), Beetle 1302 S / 1303 S, 1970-1980 | 100 vs 106 N·m at 2800 rpm (-5%) | 39 vs 37 kW at 4000 rpm (+4%) |
| `citroen_2cv_375` | Citroën 2CV flat-twin, type A, 375 cc (62 x 62 mm, 6.2:1, 9 ch at 3500 rpm), 2CV A and AU, 1949-1955 | 20 vs 20 N·m at 2000 rpm (+2%) | 7 vs 7 kW at 3500 rpm (+4%) |
| `citroen_2cv_425` | Citroën 2CV flat-twin, type A 53, 425 cc (66 x 62 mm, 7.5:1, 16 DIN-PS / 18 ch SAE at 5000 rpm), 2CV AZ / AZAM, February 1963 to February 1970 | 26 vs 25 N·m at 3500 rpm (+3%) | 12 vs 12 kW at 5000 rpm (+4%) |
| `citroen_2cv_435` | Citroën flat-twin type A 79/1 (plate AYA 2), 435 cc (68.5 x 59 mm, 8.5:1, 24 ch DIN / 26 ch SAE at 6750 rpm), 2CV 4 1970-1979 (Dyane from 1968) | 29 vs 28 N·m at 4000 rpm (+5%) | 18 vs 18 kW at 6750 rpm (+2%) |
| `citroen_2cv_602` | Citroën flat-twin type M 28/1, plate A 06/635, 602 cc (74 x 70 mm, 8.5:1, 29 ch DIN at 5750 rpm, two-stage Solex 26/35 CSIC), 2CV 6 September 1978 to July 1990 | 40 vs 39 N·m at 3500 rpm (+4%) | 22 vs 21 kW at 5750 rpm (+1%) |
| `porsche_m97_21` | Porsche M97.21 3.4 L flat-six, 987 Cayman S 2006-2008 and Boxster S 2007-2008 | 339 vs 340 N·m at 4400 rpm (-0%) | 217 vs 217 kW at 6250 rpm (-0%) |
| `porsche_m96_76` | Porsche M96/76 3.6 L flat-six, 996 GT3 Mk I, 1999-2001 | 386 vs 370 N·m at 5000 rpm (+4%) | 259 vs 265 kW at 7200 rpm (-2%) |
| `porsche_930_60` | Porsche 930/60 3.3 L turbocharged flat-six, 911 Turbo 3.3 (930), 1978 model year, rest of world | 409 vs 412 N·m at 4000 rpm (-1%) | 228 vs 221 kW at 5500 rpm (+3%) |
| `ford_voodoo_52` | Ford 5.2 L "Voodoo" V8, Shelby GT350 (2015-2020) | 578 vs 582 N·m at 4750 rpm (-1%) | 397 vs 392 kW at 7500 rpm (+1%) |
| `maserati_f136s_42` | Maserati F136 S 4.2 L V8, Quattroporte V DuoSelect, 2004-2008 | 453 vs 451 N·m at 4500 rpm (+1%) | 291 vs 294 kW at 7000 rpm (-1%) |
| `maserati_f136yg_47` | Maserati F136 YG 4.7 L V8, Quattroporte S, 2008-2010 | 504 vs 490 N·m at 4750 rpm (+3%) | 315 vs 317 kW at 7000 rpm (-1%) |
| `maserati_f154_38tt` | Maserati F154 AM 3.8 L twin-turbocharged V8, Quattroporte VI (M156) GTS, 2013-2016 (530 PS) | 658 vs 650 N·m at 2000 rpm (+1%) | 377 vs 390 kW at 6800 rpm (-3%) |
| `cosworth_dfv_1967` | Ford Cosworth DFV 3.0 L Formula One V8, 1967 specification (Lotus 49) | 322 vs 332 N·m at 8500 rpm (-3%) | 299 vs 302 kW at 9000 rpm (-1%) |
| `dodge_viper_84_v10` | Dodge / SRT Viper 8.4 L V10 (Gen V, 2013-2017) | 822 vs 813 N·m at 4950 rpm (+1%) | 470 vs 477 kW at 6150 rpm (-2%) |
| `lamborghini_52_v10` | Lamborghini 5.2 L V10, Gallardo LP560-4 (2008-2013) | 545 vs 540 N·m at 6500 rpm (+1%) | 405 vs 412 kW at 8000 rpm (-2%) |
| `bmw_s85b50` | BMW S85B50, E60 M5 / E63 M6, 2005-2010 | 531 vs 520 N·m at 6100 rpm (+2%) | 377 vs 373 kW at 7750 rpm (+1%) |
| `lexus_1lr_gue` | Lexus 1LR-GUE 4.8 L V10, LFA (2010-2012) | 495 vs 480 N·m at 7000 rpm (+3%) | 398 vs 412 kW at 8700 rpm (-3%) |
| `porsche_carrera_gt_v10` | Porsche Carrera GT 5.7 L V10, type 980/01 (2003-2006) | 593 vs 590 N·m at 5750 rpm (+0%) | 439 vs 450 kW at 8000 rpm (-3%) |
| `bmw_s70_2_v12` | BMW S70/2 6.1 L V12, McLaren F1 (1993-1998) | 654 vs 651 N·m at 5600 rpm (+0%) | 461 vs 461 kW at 7500 rpm (-0%) |
| `lamborghini_l539_v12` | Lamborghini L539 6.5 L V12, Aventador LP 700-4 (2011 specification) | 670 vs 690 N·m at 5500 rpm (-3%) | 525 vs 515 kW at 8250 rpm (+2%) |
| `vw_w12_6l` | Volkswagen Group 6.0 L W12, Audi A8 L W12 code BSB (2004 on) | 592 vs 580 N·m at 4000 rpm (+2%) | 332 vs 335 kW at 6200 rpm (-1%) |
| `cadillac_v16_452` | Cadillac Series 452 V-16, 7.4 L overhead-valve (1930-1937) | 428 vs 434 N·m at 1400 rpm (-1%) | 118 vs 123 kW at 3400 rpm (-4%) |
| `continental_w670` | Continental W670-6A / R-670-5 (Boeing-Stearman PT-17), 220 hp | n/a | 165 vs 164 kW at 2075 rpm (+1%) |
| `mazda_13b_fc_na` | Mazda 13B six-port, naturally aspirated (RX-7 FC Series 5, 1989-1991) | 188 vs 190 N·m at 4000 rpm (-1%) | 127 vs 119 kW at 7000 rpm (+6%) |
| `mazda_13b_msp_renesis` | Mazda 13B-MSP Renesis, high-power six-port (RX-8, 2003 launch specification) | 204 vs 216 N·m at 5500 rpm (-6%) | 183 vs 177 kW at 8500 rpm (+3%) |
| `gm_ls3` | GM LS3 6.2 L V8, Chevrolet Corvette C6 (2008-2013) | 572 vs 575 N·m at 4600 rpm (-1%) | 320 vs 321 kW at 5900 rpm (-0%) |
| `gm_ls9` | GM LS9 6.2 L supercharged V8, Chevrolet Corvette ZR1 (C6, 2009-2013) | 825 vs 819 N·m at 3800 rpm (+1%) | 460 vs 476 kW at 6500 rpm (-3%) |
| `dodge_hellcat_62` | Chrysler/SRT 6.2 L supercharged Hemi V8 "Hellcat", 2015 Dodge Challenger and Charger SRT Hellcat | 876 vs 881 N·m at 4800 rpm (-1%) | 510 vs 527 kW at 6000 rpm (-3%) |
| `chevrolet_lt1_350_1970` | Chevrolet 350 LT-1 (RPO LT1), 1970 Corvette, 370 hp SAE gross | 528 vs 515 N·m at 4000 rpm (+3%) | 275 vs 276 kW at 6000 rpm (-1%) |
| `ford_289_gt40` | Ford 289 small-block V8, GT40 Mk I competition specification (1965) | 426 vs 447 N·m at 5500 rpm (-5%) | 269 vs 283 kW at 6500 rpm (-5%) |
| `ford_289_gt40_180` | Ford 289 small-block V8, GT40 Mk I competition specification (1965) | 452 vs 447 N·m at 5500 rpm (+1%) | 278 vs 283 kW at 6500 rpm (-2%) |
| `yamaha_r1_crossplane` | Yamaha YZF-R1 cross-plane inline-four (model code 14B), 2009-2014; 2009 data | 102 vs 116 N·m at 10000 rpm (-12%) | 113 vs 134 kW at 12500 rpm (-16%) |
| `ducati_desmosedici_stradale` | Ducati Desmosedici Stradale 1103 cc V4, Panigale V4 / V4 S, 2018 | 120 vs 124 N·m at 10000 rpm (-3%) | 133 vs 158 kW at 13000 rpm (-16%) |
| `kawasaki_h2_750` | Kawasaki H2 Mach IV 750 (1972) | 67 vs 77 N·m at 6500 rpm (-14%) | 45 vs 55 kW at 6800 rpm (-18%) |
| `trabant_p601` | Trabant P 601, engine P 65/66 (594.5 cc, 26 DIN-PS, 7.5:1), 1974-1985 | 49 vs 54 N·m at 3000 rpm (-10%) | 20 vs 19 kW at 4200 rpm (+5%) |

Matching peak torque and power was reached by tuning estimated fields (cams, valve sizes, pipe lengths, friction and heat factors), and almost no engine has public cam data. A close fit says the engine breathes and burns at about the right rate; it does not show that the engine sounds like the real one.

## Limitations

What the model leaves out or gets wrong, as far as is known:

- **Not compared with recordings.** The output has not been compared numerically with recordings of the real engines. The checks are numerical: spectra, levels, torque, pressures, conservation and stability.
- **One gas.** γ is fixed at 4/3 for air and burned gas alike, and gas composition is not tracked in the pipes.
- **Plane waves only.** The pipes are one-dimensional: no bends, no cross-modes, no three-dimensional muffler acoustics. Mufflers are a pipe section (on the grid) or fitted loss terms (on characteristics).
- **Sound radiation** is from point sources in free air: no directivity, no ground or body reflections, no sound radiated by the engine block except the invented structure-noise stand-in.
- **Combustion** is a single-zone Wiebe burn with an empirical duration. There is no flame model, no knock model (boosted engines may use a simple knock-limited retard), and spark timing follows burn duration rather than a real engine map.
- **Friction** rises with engine speed regardless of stroke, which penalises short-stroke engines above about 12,000 rpm.
- **Two-strokes** trap too little of their fresh charge, so tuned two-strokes tend to make less than their published output. There is no power valve.
- **Rotaries** have no apex-seal leakage; port timings of the generic rotaries are typical values, not measurements.
- **Idle speed** settles above its target on many engines, by up to about 20%, most of them engines with one throttle per cylinder (`engsim idle` prints each engine's target and settled speed).
- **Not expressible in a definition yet:** variable cam phasing, variable or two-stage intakes, carburettor venturis, balance pipes, throttles behind a shared airbox, cylinder offset (VR6, W12), master-and-link rods, propeller load, direct injection, sequential twin turbos.
- **Forced induction.** Compressor maps, turbine areas, blower internals and boost-control gains are estimates for every engine, and the quasi-steady turbine is known to misstate mass flow and power under pulsating flow. The turbo whistle is a stand-in. The ECU's boost map by engine speed is not modelled (one target at all speeds).
- **Drive** has no wheelie model, no gearbox or wheel inertia, no downforce and no torque converter.
- **Real time.** The heaviest engines do not keep up at the heavy quality level on every computer, and phones may not keep up at all.

## Credits

- **[Chimera](https://github.com/jamesondh/chimera-vocal-synth)**, a voice synthesizer built by simulating the physics of the voice, was the inspiration for building an engine synthesizer the same way.
- **[engine-sim](https://github.com/ange-yaghi/engine-sim)** by Ange Yaghi is a related project that also simulates engines to produce their sound. Engine Synthesizer was written independently. It is not based on engine-sim and uses none of its code.
- The code and documentation were written by Claude, Anthropic's AI model, working with Joel Johnson, who directed the project and judged the sound by ear.
- Engine data comes from the manufacturers' documents, period road tests, service manuals, Wikipedia and enthusiast sources cited in each research file.

## License

MIT, Copyright (c) 2026 Joel Johnson. See [LICENSE](LICENSE). Third-party components are listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
