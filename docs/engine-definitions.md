# Engine definitions

Every engine is one JSON file in `engines/defs/`. The build embeds them in the binary and in the WebAssembly module in the order given by `engines/defs/ORDER`, one id per line; the build fails if the folder and the list disagree. To add an engine, write a file, add its id to `ORDER` next to its relatives, and rebuild.

`engsim spec <id | file.json>` prints a definition with every derived field filled in, which also shows each field's name and unit. `engsim dyno`, `idle` and `validate` also accept a path to a file, so a definition can be tried before it is added. Good starting points to read: `engines/defs/v8_cross.json` (generic) and `engines/defs/honda_b18c5.json` (a real engine).

## Format

Units are metres, square metres, cubic metres and degrees.

- **Required:** `name` (the id), `label`, `layout`, `bore`, `stroke`, `rod`, `cr` (compression ratio), and `firing`: for each cylinder, the crank angle (0 to 720) at which it is at firing top dead centre, cylinder 1 at 0.
- **Derived when left out:** valve sizes and lifts, valve events, intake and exhaust plumbing, flywheel inertia, redline, idle speed, `heat_q`. They are filled from bore, stroke and cylinder count by the rules in `EngineSpec::finish` (`sim/src/spec.rs`).
- **Valves:** `in_valves`, `ex_valves` per cylinder (default 2). `in_valve_d`, `ex_valve_d`, `in_lift`, `ex_lift`.
- **Cam timing as published:** `cam: {at_lift, ivo, ivc, evo, evc, in_lift, ex_lift}`. The simulator's lift curve is L_max · sin(π x)^1.4 across the seat-to-seat window, so events quoted at a checking lift h are converted exactly: the curve crosses h at x_h = asin((h/L_max)^(1/1.4)) / π in from each end. This reproduces the quoted events and peak lift. It cannot reproduce how full a real lobe is. `at_lift: 0` means the angles are already seat to seat.
- **Cam switching:** `cam_high` (same shape as `cam`), `cam_switch_rpm`, `cam_switch_hyst` (default 300 rpm).
- **Banks:** `bank` (each cylinder's bank) and `bank_dirs` (each bank's cylinder-axis angle from vertical), or `bank_angle` for a simple V. A W engine is four banks; a radial is one bank per cylinder on one crank position. `slot` gives each cylinder's crank position.
- **Plumbing:** `runner_len`, `runner_len_cyl` (per cylinder), `runner_d`, `plenum_vol`, `throttle_d`, `itb` (one throttle per runner), `snorkel_len`, `snorkel_d`, `airbox_vol`, `primary_len` (per cylinder), `primary_d`, `groups` (which cylinders share an exhaust collector; groups may mix banks), `collector_vol`, `secondary_len`, `secondary_d`, `mid_len`, `mid_d`, `muffler_len`, `muffler_d`, `muffler_zeta`, `tail_len`, `tail_d`.
- **Fit factors** (only for estimates, see the rules): `friction_scale`, `heat_q`, `burn_scale`.
- **Two-strokes:** `kind: "two_stroke"`, `itb: true`, firing angles within 0 to 360, `ex_port_open` and `tr_port_open` in degrees after top dead centre (a port quoted as a duration D opens at 180 − D/2), `port_ex_area`, `tr_port_area`, `port_in_area`, `induction` ("reed", "piston" with `in_window: [half-duration, half-duration]`, or "disc" with `in_window: [opens before TDC, closes after TDC]`), `crankcase_cr`, and `chamber` (the expansion chamber as [length, start diameter, end diameter] sections). See `engines/defs/twostroke_250.json`.
- **Rotaries:** `kind: "wankel"`, `ecc`, `rotor_r`, `rotor_w`, `rotor_phase`, `port_in`, `port_ex`. See `engines/defs/wankel2.json`.
- **Forced induction:** a `boost` object. Superchargers: `kind` ("roots" or "twin_screw"), `displacement` (m³ per rotor revolution), `drive_ratio`, `rotors`, `lobes`, `helix`, `vi`, `throttle` ("before" or "after" the blower), volumes, port and leakage areas, `bypass`, `intercooler`, charge pipe. Turbochargers: `kind: "turbo"`, `boost_bar`, `intercooler`, and a `turbo` object (wheel sizes, map width, turbine area, inertia, wastegate, blow-off valve, `count: 2` for parallel twins, `scroll`). Anything left out is derived (`BoostSpec::finish`, `finish_turbo`; all estimates). Boost is refused on engines with individual throttles and on two-strokes. See `engines/defs/buick_3800_l67.json` and `engines/defs/audi_rr_20v_turbo.json`.
- **Library information** (not used by the physics): `generic` (true for invented engines), `family`, `based_on` (the real engine's name and years), `sources` (a list of `{url, what}`), `published` (peak `power_kw` at `power_rpm`, `torque_nm` at `torque_rpm`), `notes`.
- **Vehicle** (optional): a `vehicle` object for the car or motorcycle the engine came in, used by Drive. Required: `name` (with the year), `kind` ("car", "motorcycle" or "race car"), `curb_kg` (wet weight for motorcycles), `gear_count`. Optional: `gears` (falling, exactly `gear_count` of them), `final_drive` (for a motorcycle, primary times secondary reduction), `wheel_radius_m`, `tyre`, `driven` ("front", "rear" or "all"), `cda_m2`, `top_speed_kmh` (used only to set a missing final drive), `notes`.

## The sourcing rule for real engines

An engine is modelled as a real engine only if public sources give its bore, stroke, compression ratio, cylinder layout and firing pattern. Each real engine has a research file, `engines/research/<id>.json` (format in `engines/research/SCHEMA.md`), in which every number carries the URL it was read from, and a value that could not be found is `null` with a note.

1. Put every sourced value into the definition, converted to its units, and copy the source URLs into `sources`. Set `generic: false`, `based_on`, `family`, `published` (when sourced) and a `label` people will recognise.
2. Anything the research does not give is an estimate. Estimate it from the engine's known character, or leave it to the derived defaults, and list every estimated or defaulted field in `notes` in plain words. If the firing order is an assumption, say so in `notes`.
3. Firing angles come from the sourced firing order and crank layout. `engsim pins <file>` prints which cylinders share a crank pin and the firing intervals; both must match the research.
4. Check it runs: `engsim dyno <file>`, `engsim idle <file>` (it must settle near its idle target without stalling) and, when `published` exists, `engsim validate <file>`.
5. Tuning is allowed only on fields that are estimates: cam duration and lift, unsourced valve sizes, runner and primary lengths and diameters, `friction_scale` (0.6 to 1.2), `heat_q` (2.2e6 to 2.7e6), `burn_scale`. The aim is simulated peak torque and power within about 10% of published. A sourced number is never changed to make the output fit. If an engine will not come within 20%, tuning stops and the gap is reported; a known gap is acceptable, a fudge is not.
6. If an engine needs something the format cannot express, write the definition as far as it goes and record what is missing.
7. Forced induction only on engines that came boosted from the factory, with the `boost` object stating what is sourced and the rest listed in `notes`.
8. A vehicle block should describe a vehicle the engine was actually sold in (the earliest or best documented one), with its sources in the research file under `vehicle`. Check it with `engsim drive <id>` against published acceleration where one exists.

Two-strokes: model an engine as itself only if at least its exhaust and transfer port timings are sourced. Expansion-chamber dimensions are almost never published; estimate them so the torque peak lands at the published speed (lengthening the pipe lowers it), use `engsim trace` to see the returning wave, and say in `notes` that the pipe is fitted.

A close fit to published torque and power does not show that an engine sounds like the real one. Almost no engine has public cam data, and most fits were reached by tuning only the estimated fields listed in each definition's `notes`.

## Validation

`engsim validate --write` runs every real engine at full throttle at its published peak-torque and peak-power speeds (quality `live96`) and writes `engines/VALIDATION.md`.
