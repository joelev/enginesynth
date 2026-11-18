# viz: video renderer for engsim telemetry

Draws one telemetry directory (as written by `engsim render`) with three.js and captures it
frame by frame in headless Chromium. One telemetry frame is one video frame.

`viz/engine-model.js` holds the layout, meshes and per-frame update for one engine; both this
renderer (`viz/main.js`) and the live app (`live/scene.js`) use it.

## Setup (once per machine)

three.js is installed locally into a folder that git ignores:

    npm install --prefix viz/vendor.nosync three@0.170.0

Python needs `playwright` (the script uses the newest Chromium in `~/Library/Caches/ms-playwright`).

## Render

    python3 tools/render_video.py <data_dir> <out.mp4> [--frames a:b] [--size WxH] [--fps 60]
            [--audio file] [--no-audio] [--params "k=v&..."] [--png] [--software]
    python3 tools/render_video.py <data_dir> --still 120 frame.png        # single frames
    python3 tools/render_video.py <data_dir> --verify                     # geometry checks (JSON)

Default audio is `mix.wav` cut to the time span of the chosen frames. If the frames are slow
motion or stroboscopic, that span will not match the video length; the script says so, and
`--audio` or `--no-audio` is the answer.

## Preview with sound

    python3 -m http.server 47613 --bind 127.0.0.1      # from the engine-synth folder
    open "http://127.0.0.1:47613/viz/index.html?data=../out.nosync/<dir>"

## Page parameters (`--params` or the URL)

| name | meaning | default |
|---|---|---|
| `view` | `engine` (engine about a third of the frame height, exhaust trailing off), `wide` (engine plus the whole exhaust), `exhaust` (from the tailpipe end, looking back), `front` (end-on along the crank) | `engine` |
| `az0`, `az1` | camera azimuth at the first and last frame, degrees (override the view) | per view |
| `el` | camera elevation, degrees | per view |
| `dist` | distance multiplier on the automatic fit | 1 |
| `tx`, `ty`, `tz` | target offset, metres | 0 |
| `fov` | field of view, degrees | 30 |
| `pfull` | pressure difference (bar) at the ends of the colour scale | 0.7 |
| `pexp`, `gain` | how fast gas brightness rises with pressure difference, and its peak | 1.6, 4 |
| `pmode` | `ac` colours pressure minus its running average over `acwin` frames (default 61) instead of pressure relative to ambient; the legend text changes to say so | off |
| `bloom`, `exposure`, `env` | glow strength, tone-mapping exposure, reflection strength | 0.32, 1, 0.42 |
| `caption` | text under the title (empty by default) | |
| `tag` | small text under the colour legend, e.g. a note about the frame mode (empty by default) | |
| `hud`, `grid`, `audio` | set to `0` to hide the overlay, the floor grid, or skip the scope | |
| `waves` | `0` hides the sound waves on the floor (drawn only when the telemetry has `header.sound`) | on |
| `wavemap` | JSON overriding fields of `waveMapping` in `sound-waves.js` for this render, e.g. `{"gain":1.2}` | |

A `viz.json` in the data directory may hold `{"caption": "..."}`, `{"tag": "..."}` or
`{"captions": [{"from": 0, "to": 120, "text": "..."}]}` (frame ranges).

## What is drawn from data, and what is a drawing choice

From telemetry: crank angle, piston position, valve lift (true scale), cylinder temperature and
pressure (glow), pressure in every pipe sample and volume (colour), rpm, throttle, gear, speed,
fuel cut, and the audio waveform. Pipe diameters and lengths are the simulated ones; each path
is bent until its drawn length equals its physical length.

Gas colour: hue shows the sign and size of the pressure difference, brightness rises with its
size, so gas at ambient pressure is dark and a pulse is a bright slug. Cylinder glow brightness
follows cylinder pressure and burn rate; temperature only sets its colour.

Drawing choices the simulation does not fix: where pipes run in space (primaries fan into a
collector in cylinder order and are checked not to touch; secondaries may be drawn longer than
their physical length where the layout needs it; `--verify` lists every drawn length), crank web and rod shapes,
piston height, the shape of the combustion chamber (a flat disc holding the clearance volume;
if true-scale valve lift would meet the piston in a flat chamber, part of that volume is drawn
as valve pockets instead), and volumes shown as spheres, a box, or a capsule of the right volume.

## Sound waves (`sound-waves.js`)

The rings on the floor are the sound each radiating opening sends out (telemetry `header.sound`:
each opening's last 128 samples, pascals at 1 m). Each opening is the model's point source in
free air, so the pressure at distance r is hist(r / c) / r; the floor shows the sum over all
openings at each point's true 3D distance. Warm is compression, blue rarefaction; brightness is
a power law of pressure, weighted toward the crests and troughs so the field reads as rings.

Where each opening is drawn (`soundSources` in `engine-model.js`): a pipe's open end is the end
of its drawn path, an open throttle is the lip of its bellmouth. `--verify` reports `sound`:
each source's distance from the centre of the drawn opening (the tube's last ring of vertices,
the bell's lip) and the angle between its direction and the drawn pipe's axis.

Everything that turns data into picture is the `waveMapping` object at the top of
`sound-waves.js`, read every frame: the age-to-radius curve (`radiusOfAge`, default the physics,
radius = age x c), the display smoothing (`smoothHz`, follows the firing frequency), the 1/r
floor (`rMin`), the brightness law (`pFull`, `exponent`, `gain`, `negGain`), the ring weighting
(`crestLen`, `fill`, `maxWave`), the outer fade (`fadeFrom`) and the two tints. The age-to-radius
curve, smoothing and 1/r are applied in JavaScript when each opening's radial profile is built,
so a new curve needs no shader change. `tools/waves_check.py spacing <dyno render>` measures the
drawn pulse spacing from the profiles the shader is fed.

Left out, as in the model: reflections from the ground and the engine, directivity, and any
sound radiated by the block. The waves are strobed with the rest of the picture.

## The sound picture on the live app's floor (`wave-studies.js`, live app only)

The live app draws this instead of the rings; the video renderer still draws the rings above. The "Sound waves"
button (or `?waves=0` / `?waves=off`) turns it off and leaves the plain grid floor.

- **Pulse clock** (`PulseClock`, `pulseConfig`): a slow pulse train whose rate glides with engine
  speed, 1 a second at idle to 3.2 at the redline on a geometric curve, so it only rises as the
  revs rise. It also follows the loudness of each opening (rms of its 11 ms history, 30 ms up,
  0.3 s down, through a power law) and forms `drive` = loudness x a throttle term. A sudden rise in
  drive starts a pulse at once. The pulses are not individual firings: at 60 telemetry frames a
  second those cannot be resolved.
- **Membrane** (`membraneConfig`): a hexagonal lattice of dots on the floor. Each pulse starts one
  soft swell at every opening; swells travel faster with drive, sit closer together as the rate
  rises, and the height of every swell follows drive live, so opening the throttle lifts the whole
  floor within a frame or two and a lift-off lets it sink. Dots are lit by the slope (warm facing
  the light, blue facing away), blurred away from the focus distance and faded at the edge. They
  are drawn at half brightness (`dim: 0.5`).
- **Spectrogram disc** (`SpectroFloor`, `spectroFloorConfig`): a spectrogram of the sound the page
  plays, as a disc on the floor around the engine. Frequency runs outward (30 Hz to 9 kHz, log),
  time runs round, and a write line sweeps round once in 9 s; sound fades as it ages and is gone
  just before the line returns. A steady note is a circle, a change of revs a spiral, a throttle
  blip a spoke. It reads `live/spectro.js` `SpectroSource` (an 8192-point transform below 300 Hz,
  a 2048-point one above 1.2 kHz, blended between), so it is empty until sound starts.
- **Together** (`SoundFloor`): the disc is bent by the membrane's swell (the two shaders share
  it), the dots inside the disc take the disc's light, brighter where they are lifted, and the
  disc itself is dimmed to a wash under them (`membraneConfig.withSpectro`).
- **Spectrogram widget** (`live/spectro.js` `LiveSpectro`): the dashboard's spectrogram, from the
  same two-transform spectrum. It scrolls by time, glows on loud ridges and behind the newest
  column, shows the spectrum of the moment as a curve in a gutter at the right with a falling
  peak-hold line, and marks 0.5, 1, 2, 3 and 4 times the firing frequency.
- **Warm-up** (`warmupConfig` in `live/scene.js`): one factor on every glow the engine model draws
  (cylinders, pipe gas, hot headers, volumes), from 0.5 for an engine at idle to 1 for one worked
  hard. It is a smoothed state heading for 0.6 x engine speed + 0.6 x throttle (capped at 1),
  about 3 s up and 9 s down. It is not a simulation of heat. `EngineModel.glowScale` carries it;
  the video renderer leaves it at 1.

`tools/wave_studies.py time` measures frame time with the floor on against off; `shots` takes
stills at idle, mid revs and the limiter.

## Rotary (Wankel) engines

A header with `kind: "wankel"` is drawn as rotors in trochoid housings. Exact: the housing bore
(two-lobe epitrochoid from `ecc` and `rotor_r`), the eccentric shaft angle, the rotor centre and
apex positions (the apexes lie on the bore; `--verify` checks this every frame), which chamber
is where (a chamber fires at the plug side), and the port positions on the bore from
`port_in`/`port_ex`. Stylised: the rotor flank is a circular arc held 1 mm short of the bore's
waist (no combustion pocket), so the drawn chamber volume is not the simulated one; the housing
is glass with its outside line 20 mm out; a side intake port is drawn as a window on the rear
face of each rotor housing; spark plugs, side seals, shaft and flywheel are decoration. The
engine is drawn with the housing's long axis upright and the ports facing +Z. `view=front` is
the natural view.

## W and radial engines

Cylinder positions follow the header: `bank_dir` gives each cylinder's axis angle from vertical
(any number of banks) and `spec.slot` its place along the crank. A W engine is simply four rows;
rods, pistons and crank pins are placed by the same rules as a V and pass the same `--verify`
checks.

A radial is any crank position with three or more cylinders on it. They are drawn in one plane
around a single crank throw with a counterweight. Every rod is drawn straight from the shared
crank pin to its piston, which is the kinematics the simulator uses; a real radial has a master
rod with link rods, whose pistons move slightly differently, and that is not drawn. Induction is
drawn as a hub on the crank axis behind the cylinders with the air entering from the rear, and
each exhaust stack sweeps back from its head. `view=front` is the natural view.

`in_valves` / `ex_valves` of 1 draw one larger valve on the centre line of that port side.

## Two-stroke engines

A header with `kind: "two_stroke"` has no poppet valves. Each liner shows an exhaust port facing
the exhaust side and two transfer ports either side of it; a port's top edge sits where the
piston crown is at the crank angle the port opens (`ex_port_open`, `tr_port_open`, degrees after
TDC), so the drawn piston uncovers it at the simulated moment, and `--verify` checks those edges.
The ports brighten with the open fractions the telemetry carries in `lift_ex` / `lift_in`.
Intake enters the crankcase behind the cylinder. Each primary that ends in open air (an expansion
chamber, shape from `spec.chamber`, so the tube shows its diffuser, belly and baffle cone) drops
from its port to its own lane below the crank and runs back; the front cylinder's pipe takes the
outermost lane, and `--verify` reports the smallest gap between pipe surfaces (`minOpenPipeGap`).
A collector volume that no pipe enters is not drawn. Not drawn: crankcase pressure, reed petals,
the boost port.

## Superchargers and turbochargers

Drawn for engines whose header has `turbo` or `boost` (the supercharged and
turbocharged engines). Every part corresponds to a simulated element; where it sits and
its shape are drawing choices, listed below with what is sourced. Nothing here changes the
picture of an engine without boost.

**What each part is.**

| Drawn | Simulated element | Colour |
|---|---|---|
| Turbine housing: a snail-shell volute fed tangentially by its bank's collector, with the collector's fan of primaries | the collector volume (the simulator adds the volute's volume to it) | collector pressure; the shell glows with gas temperature as the exhaust pipes do |
| Can behind the turbine, along the shaft; the downpipe leaves its end | turbine outlet volume | its pressure and heat |
| Centre section, V-band clamps, compressor wheel (still: it turns at up to some 200,000 rpm) | the shaft and the wheels | metal |
| Wastegate actuator can and rod | the wastegate and its actuator | metal |
| Compressor housing (volute) on the same shaft, ahead of the turbine; its tangential outlet | compressor outlet volume | charge scale (below) |
| Air filter box (flat, of the simulated volume) and its duct to the compressor inlet | the airbox volume (box and duct are one volume) | ghost |
| Intercooler: two tanks either end of a finned core | inlet volume and intercooler volume; the core is the intercooler element (pressure loss and cooling) | tanks ghosted on the charge scale; core metal |
| Charge pipes, bent to their physical length | the two charge pipes per turbo | charge scale |
| Throttle inlet volume, throttle body (its plate turns with the throttle), blow-off valves on top with their vent pipes | throttle inlet volume, throttle, blow-off valves and vent pipes | charge scale; vents as pipes |
| Roots blower on top of the plenum: case, two three-lobe rotors with a 60 degree twist, mirror images, meshing; outlet throat into the plenum | the blower's rotor pockets (rotors, lobes, twist from the definition) | rotors metal in a glass case |
| Blower inlet box at the rear of the case, throttle body behind it, air filter box and snorkel | blower inlet volume, throttle, airbox | its pressure; airbox ghost |
| Drive snout, blower pulley, crank pulley and belt | the drive (the simulator takes the drive torque from the crank) | metal; the rotors and blower pulley turn with the crank at the drive ratio |
| Bypass actuator on the inlet box | bypass valve | metal |

**Charge scale.** The charge side (compressor or blower outlet to the intake valves: compressor
volute, charge pipes, intercooler tanks, throttle inlet, plenum, runners) sits at boost pressure
for as long as the throttle is open. On the exhaust's scale it would be a steady white glow
blooming over the engine, so it is drawn on its own scale (`chargeColor`, `chargeConfig` in
`engine-model.js`): above ambient a teal ramp, full at 1.25 times the engine's boost target, with
a peak brightness below the bloom threshold; below ambient exactly as everywhere else (blue). The
live app's legend shows this scale in a second row ("charge air") on boosted engines only. The
turbine housings and turbine outlets glow with gas temperature like the exhaust pipes.

**Rotors.** The blower's rotor angle is the drive ratio times the crank angle, the crank angle
unwrapped from the cycle angle with the engine speed between frames, so the rotors stay locked to
the crank as the simulator has them. Rotor profiles are a cosine lobe (not the real involute), set
3.5% apart so they never touch (checked numerically: at least 3% of the tip radius clear).

**Pipe routes** (sound-neutral: the simulator's pipes have length and area only, no bend loss). Every boost pipe is drawn as tubing, straight runs joined by bends of about 5 cm radius (`bendPath`), at its simulated length; spare length moves a corner along the engine or the bay instead of bowing the pipe into open air (`routePipe`). Compressor to intercooler: out of the compressor outboard and down, forward low along the bay, into the inlet tank from behind. Intercooler to throttle: back from the outlet tank and up to the throttle inlet, entering it from the front (a twin's two pipes side by side); the spare length moves the corner toward running low to the engine's front face and rising up it. Turbo downpipes drop under the sump and run straight back. Snorkels leave their air box and fold outboard (turbos) or down (the blower) before running on. No length or diameter was changed for the picture.

**Sizes.** Compressor wheel diameter is the header's `d2`; turbine wheel the definition's derived
`turbine_d`; housing proportions are drawn (spiral section 0.25 to 0.7 of the wheel radius round a
hub of 1.2 to 1.25 wheel radii, ports flaring to the pipe they meet). Roots rotors: length 0.19 m
(estimate), tip radius from the displacement per revolution (D = 0.9 pi R^2 L, an estimate of the
swept share), root radius half the tip. Pulleys: blower 3.8 in (3800wiki, Wikipedia), crank pulley
the blower pulley times the drive ratio (7.2 in, the forum measurement the drive ratio came from).
Intercooler cores: 0.17 x 0.11 m per side on a twin, 0.34 x 0.14 m on a single (estimates).

**Positions: sourced and estimated.**

| Engine | Part | Position | Basis |
|---|---|---|---|
| Maserati F154 | turbos | one per bank, at that bank's exhaust collector, outboard of the bank | sourced: "integrating the turbine housing into the exhaust manifold", one turbo per bank of four (launch text, `engines/research/maserati_f154_38tt.json`); outboard because the intake manifolds sit in the vee (Maserati diagnostic sheet MAS004832: RH and LH intake manifolds and a central compensator); "two parallel twin-scroll turbochargers supplied by IHI sit outboard of the block" ([Silodrome, Ferrari F154 cutaway](https://silodrome.com/ferrari-f154-v8-cutaway/), secondary source), and the California T's F154 has its intake "topside" ([AutoGuide, 2014](https://Autoguide.com/auto-news/2014/03/everything-you-need-to-know-about-the-first-turbocharged-ferrari-engine-in-decades.html)) |
| | turbo height, shaft along the crank, compressor facing forward, outlet up toward the intake side | tight under its bank's ports at mid-bank (the manifold runners are the definition's cast-manifold lengths, 0.115 and 0.20 m), housings 0.075 m clear of the block | estimate |
| | intercoolers | one per turbo, 0.5 m ahead of the engine's front face, low and 0.42 m out to each side (the definition's charge pipes, 0.80 and 0.90 m, come from the same packaging estimate and are drawn at those lengths; the second pipes climb the front of the engine and enter the throttle inlet side by side from the front) | sourced: "two intercoolers, each mounted low and wide of the main radiators" (launch text); distance from the engine an estimate |
| | air filter boxes | two, ahead of each compressor, raised | two boxes sourced (MAS002310: "the air filter boxes"); position estimate |
| | throttle | one, at the front of the plenum, both charge pipes meeting at it | single throttle an inference in the research file; front position estimate |
| Audi RR | turbo | on the exhaust side, at mid-engine on the cast manifold's outlet (the definition's runners: 0.24, 0.15, 0.10, 0.15, 0.24 m), hanging below the header | the K24 is fed by the exhaust manifold (sourced: K24, research file); where along the engine, height and orientation estimates |
| | intercooler | one, across the front, low | intercooler sourced ("charge air intercooler"); front mounting inferred from an owner's mailing-list remark about "the stock position of the IC on ur quattros" and "the intake in the valance" ([audifans, 2005](https://audifans.com/archives-2k/urq/2005-December/007927.html), seen as a search excerpt); core drawn 0.34 x 0.14 m, smaller than the related RS2's stock 500 x 185 x 65 mm (a retailer, search excerpt), to keep it from dominating |
| | throttle, air filter box | throttle at the front of the plenum; box ahead of the compressor | estimates |
| GM 3800 L67 | blower | on top of the intake in the vee, rotors along the crank; the definition's runners are 0.21 m (blower outlet plenum just above the vee to the ports), so the plenum and blower sit low on the engine | sourced: M90 on the lower intake (research file); the plenum and runners here are drawn by the general rule, so the blower sits higher than on the real engine |
| | throttle before the blower, then the blower inlet | throttle, inlet volume, rotors, plenum in that order | sourced: GM TechLink, "through the throttle body, and into the supercharger" |
| | pulley at the front end on one rotor's shaft, inlet and throttle at the rear | | estimate (no source found for which end carries which); the M90 is "17-inches" long ([therangerstation](https://www.therangerstation.com/tech/eaton-m62-m90-m112-superchargers/)); drawn case, snout and inlet box come to about 0.38 m (15 in) |
| Porsche 930/60 | layout rule | a turbocharged flat engine whose vehicle drives the rear wheels is drawn rear-engined (`rearTurbo` in `routePipes`): the crank pulley end is the car's tail, so the exhaust collectors sweep toward it, the merged turbine inlet and the turbo sit low beyond that end, the turbine outlet can points down and the downpipe runs out past the pulley end to the silencer | the 911 Turbo is rear-engined (sourced, research file); the rule is an inference that holds for every flat engine in the library that drives the rear wheels (911s, the Cayman, the Beetle); a front-engined turbo boxer would need another rule |
| | turbo | one, low behind the engine at the pulley end, fed by both banks' collectors merged; shaft along the crank, compressor outermost, its outlet pointing up | single turbo fed by both banks through a crossover sourced (parts listings, research file); the real crossover makes one bank's pipe longer, which the definition cannot say (one secondary length); orientation and height estimates |
| | intercooler | lying flat on top of the engine at its pulley end, 0.16 m above the plenum, core 0.36 x 0.20 m, tanks along the crank either side; entered and left through the bottoms of its tanks | sourced: "positioniert einen Ladeluftkühler unterhalb des Heckspoilers" (Porsche newsroom, research file); size and height estimates |
| | throttle, air filter box | throttle at the pulley end of the plenum, under the intercooler; box beyond the compressor | estimates |
| Honda K20C1 | turbo, intercooler | the general single-turbo layout (turbo below the cast-in manifold's single outlet, intercooler across the front, low) | intercooler "positioned low in the front of the car" sourced (Honda, research file); the real engine is transverse with its exhaust facing the bulkhead, which the picture (a longitudinal engine) does not show |

Checks (in-page, `EngineModel` of each boosted engine): every sound source on its opening (position
error under 1e-7 m, direction 0 degrees); every charge pipe, snorkel, vent and downpipe drawn at its
physical length; no pipe passes through another except where a blow-off valve and a charge pipe
both meet the throttle inlet volume; no NaN. Turbo housings sit 0.11 to 0.12 m clear of a box round
the crankcase and cylinders.
