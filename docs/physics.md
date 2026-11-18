# The physics in Engine Synthesizer

This document describes the model as the code implements it. Every equation and constant below was checked against `sim/src` (file names are given with each part). Where a model is a standard one (HLLC, Wiebe, Woschni, Flanagan's radiation load, Lighthill's jet-noise law), it was implemented from its usual textbook form; the implementation has been checked against the code, not against the original papers.

Units are SI throughout: metres, kilograms, seconds, kelvin, pascals. Crank angles in definitions are in degrees; inside the simulator they are radians.

## 1. The gas (`gas.rs`)

There is one gas: an ideal gas with R = 287 J/(kg K) and a fixed ratio of specific heats γ = 4/3, so c_v = R/(γ − 1) = 861 J/(kg K) and c_p = γ c_v = 1148 J/(kg K). Ambient is 101 325 Pa and 293.15 K.

γ = 4/3 lies between cold air (1.40) and hot burned gas (about 1.30). It was chosen because it makes every isentropic exponent a whole number or a power of two: x^(1/γ) = x^(3/4), x^((γ−1)/γ) = x^(1/4), x^(γ/(γ−1)) = x^4, x^(2/(γ−1)) = x^6 and so on. Those are computed with multiplications and square roots, which matters for speed in WebAssembly. The cost of the choice: compressed charge air comes out about 13% too cool before an intercooler, and engine torque is a few per cent lower than with γ = 1.35. Gas composition (fresh charge against burned gas) is not tracked in the pipes; the cylinders track the mass of fresh charge separately for the combustion model.

### Orifice flow

Every lumped restriction (throttles, valves, ports, wastegates, blow-off valves, intercoolers, leakage paths) uses the same compressible orifice law:

    mdot = CdA · p_up / sqrt(R T_up) · ψ(p_dn / p_up)
    ψ(x) = sqrt( 2γ/(γ−1) · (x^(2/γ) − x^((γ+1)/γ)) )

With γ = 4/3 the exponents are 1.5 and 1.75. Below the critical pressure ratio (2/(γ+1))^(γ/(γ−1)) = (6/7)^4 ≈ 0.540 the flow is choked and ψ is held at its critical value. Above a pressure ratio of 0.99 the curve is replaced by a straight line to zero, so the flow is smooth through reversal. Each step, the mass moved through an orifice between two finite volumes is limited to half of the mass that would equalise their pressures:

    dm_eq = (p_1 − p_2) / (γ R T_up (1/V_1 + 1/V_2))

This limiter is what keeps small volumes stable at audio-rate time steps.

### Volumes

A volume (plenum, airbox, collector, turbine inlet, crankcase, blower pocket) holds a mass m and internal energy E in a fixed volume V. Pressure is (γ − 1) E / V. Flows into and out of it carry total enthalpy c_p T of the gas they come from.

## 2. Pipes on a grid (`pipe.rs`)

A pipe is a series of straight or tapered sections. On the grid it is solved as quasi-one-dimensional compressible flow with area A(x):

    ∂(ρA)/∂t  + ∂(ρuA)/∂x            = 0
    ∂(ρuA)/∂t + ∂((ρu² + p)A)/∂x     = p dA/dx − friction
    ∂(EA)/∂t  + ∂(u(E + p)A)/∂x      = − wall heat

with E = p/(γ − 1) + ρu²/2.

- **Scheme.** Finite volume. Second-order MUSCL-Hancock: limited slopes of the primitive variables (ρ, u, p), a half-step predictor, then fluxes at each face. The limiter is the harmonic (van Leer) form, slope = 2ab/(a + b) when a and b have the same sign, else 0. The two end cells stay first order. A third-order variant (κ = 1/3 with Koren's limiter) exists but is not used: it changed spectra by under 1 dB.
- **Fluxes.** HLLC (Toro), with wave speeds estimated from the primitive-variable (PVRS) pressure p* = (p_L + p_R)/2 − (u_R − u_L)(ρ_L + ρ_R)(a_L + a_R)/8.
- **Area changes.** A face's area is the smaller of the two cells' areas, and the cell gains the source term p (A_right − A_left) in momentum.
- **Friction.** Each step, after the conservative update, the velocity is relaxed implicitly by the Darcy friction law du/dt = −(f / 2D) u|u|: u_new = u / (1 + dt (f/2D) |u|). The kinetic energy removed stays in the gas as heat.
- **Wall heat.** A heat-transfer coefficient from the Reynolds analogy, h = (f/8) ρ |u| c_p + 8 W/(m² K), drives the gas towards the wall temperature over the wetted perimeter (4/D per unit volume): T_new = T_wall + (T − T_wall) · e^(−x), with x = h (4/D) dt / (ρ c_v) and e^(−x) taken to second order.
- **Cell size.** At `final` quality the cells are 8 mm (exhaust) and 5 mm (intake) at 192 kHz; `draft` uses 16 and 10 mm at 96 kHz. The `live` levels size each pipe's cells to its own peak wave speed, so every pipe runs near a CFL number of 0.88 at the chosen rate. Each sample at the internal rate is normally one step; more steps are taken only if a pipe's CFL limit requires it.

### Pipe ends

- **Closed:** a mirror state.
- **Into a volume** (`reservoir_face`): if gas leaves the pipe, the face pressure is the volume's pressure, the outgoing Riemann invariant u + 2a/(γ − 1) is carried to the face, and the flow chokes at the face if it would be supersonic. If gas enters the pipe, it enters isentropically from the volume at rest, matched to the pipe's outgoing wave. The two branches meet smoothly at zero flow.
- **Open to the air:** the same boundary against atmospheric pressure plus a radiation pressure. The radiation load is Flanagan's parallel resistance and inductance for a piston in a baffle:

      R = 128 ρ c / (9 π² A),   L = 8 ρ / (3 π sqrt(π A))
      p_rad = R (Q − U_L),       dU_L/dt = p_rad / L

  where Q is the volume flow out of the end. p_rad and the face flow Q(p_rad) are solved together each step by a secant search (with a bracketed fallback).

## 3. Pipes along characteristics (`wave.rs`)

The `+W` quality levels replace most pipes with a cheaper and sharper solver. In a uniform pipe of gas at one entropy the two Riemann variables

    λ = A + kU   (travels right at u + a)
    β = A − kU   (travels left at a − u)

with A = a/a_ref = (p/p_atm)^((γ−1)/(2γ)) = (p/p_atm)^(1/8), U = u/a_ref and k = (γ − 1)/2 = 1/6, are carried unchanged along their characteristics. Each direction of a pipe is then a delay line, and each sample's delay follows from its own speed. Faster, higher-pressure samples catch slower ones, which is how a wave steepens; where samples would overtake, their arrival times are pooled, which turns the front into a shock one sample thick and dissipates it. Area changes are steps at constant pressure; friction and wall heat are lumped per pipe.

Losses and filtering in a characteristic pipe:

- **Band-limited reading** (`BAND_LIMIT = 2`). Each delay line reads the mean of what arrives over a sample period, averaged over two periods (a triangle kernel, as in polyBLEP). A step's position between two samples survives as the split of a two-sample ramp, so fronts do not alias.
- **Boundary-layer loss** (`KIRCHHOFF`). Kirchhoff's wide-tube loss for viscous and thermal boundary layers, with the same amount of phase lag:

      α = sqrt(ω ν / 2) / (r c) · (1 + (γ − 1)/sqrt(Pr))   nepers per metre

  for each pipe's radius r and gas temperature (ν from Sutherland's law for air at atmospheric pressure, Pr = 0.71). In the time domain this is exp(−a sqrt(s)) per crossing, with a = len · (1 + (γ − 1)/sqrt(Pr)) · sqrt(ν) / (r c). The sqrt(s) is approximated by seven first-order high-pass sections with poles at 20 Hz × 4^k; magnitude and phase are within 10% and 10 degrees of the exact form from 100 Hz to 20 kHz. The per-metre form was derived for this project from the wavenumber quoted in [arXiv 1812.11063](https://arxiv.org/pdf/1812.11063); see also [Weston 1953](https://www.ece.uvic.ca/~bctill/papers/numacoust/Weston_1953.pdf) for the wide-tube regime. Bulk (classical) absorption is under 2% of this at 10 kHz over these lengths and is left out.
- **Treble loss, fitted, not physical** (`TREBLE_HZ = 5000`). At high load, exhaust pulses steepen into true shocks in the model, and the open end radiates each one as a click. The grid solver does not produce them only because it damps waves shorter than about eight cells, which is a numerical effect. To match it, each crossing also passes a second-order Butterworth low-pass with cutoff 5 kHz × (1 m / length)^(1/4), read ahead by its own low-frequency delay so pipe lengths and tuning are unchanged. 5 kHz was fitted to the grid's spectra above 4 kHz. This term copies a numerical artifact and is labelled as fitted in the app.
- **Mufflers.** On characteristics a muffler is an exit loss coefficient on the tailpipe's dynamic head (`MUFFLER_ZETA = 7`, which reproduces the grid muffler's back-pressure of about 0.34 bar at full power on a large V8; a definition may set `muffler_zeta`) and an absorptive shelf that removes 20% of the wave above 600 Hz per crossing. All three numbers are fitted to the grid solver, not derived.

What this solver gives up: entropy is one value per pipe (it cannot carry hot slugs from an exhaust valve), so exhaust primaries always stay on the grid; mass is not exactly conserved inside a pipe during transients; and characteristic intake runners make engines with one throttle per cylinder unstable at idle, so those engines keep grid runners.

## 4. Cylinders (`engine.rs`)

Each cylinder (or rotor face) is a single zone of gas with mass m, internal energy E and volume V(θ). It also tracks how much of its mass is fresh charge.

- **Kinematics.** Slider-crank: the piston's distance below top dead centre is x(θ) = r + l − (r cos θ + sqrt(l² − r² sin² θ)), with crank radius r = stroke/2 and rod length l. V = V_c + A_p x, with the clearance volume from the compression ratio.
- **Piston work.** Over each step the gas is compressed or expanded isentropically: E ← E · (V_0/V_1)^(γ−1), a cube root for γ = 4/3. The work E_0 − E_1 − p_atm (V_1 − V_0) goes to the crank (the piston's underside sees atmospheric pressure in a four-stroke and crankcase pressure in a two-stroke).
- **Valves.** A poppet valve is an orifice with discharge coefficient 0.62 on the curtain area n π d L(θ), capped at 0.9 of the valve's own area. The lift curve is L(θ) = L_max · sin(π x)^1.4, with x running from 0 to 1 across the seat-to-seat window. On the pipe side the valve exchanges gas with a fixed 35 mm of port (all grid cells within it share the exchange in proportion to their mass), so valve flow does not depend on cell size. Exhaust valve flow carries low-pass-filtered random noise of about 10% rms, a stand-in for turbulence.
- **Cam timing.** Events published at a checking lift h are converted to seat-to-seat events through the lift curve: the curve crosses h at x_h = asin((h/L_max)^(1/1.4)) / π from each end, so the seat-to-seat duration is the quoted duration divided by (1 − 2 x_h). A second cam profile (VTEC-style switching) changes per cylinder at its next firing top dead centre, where both valves are shut.

### Combustion

The burn is planned at intake closing from the trapped fresh charge m_fresh and the residual-gas fraction x_res = 1 − m_fresh/m:

    load      = min(m_fresh / (ρ_atm V_d), 1.2)
    duration  = (40 + 14 N/6000 + 20 (1 − load) + 60 max(0, x_res − 0.12)) degrees
                × (1 + σ g),  clamped to 0.6..1.8 of that,  σ = 0.035 + 0.5 max(0, x_res − 0.18)
    advance   = 0.517 · duration − 8 degrees, clamped to 2..60
    heat      = heat_q · m_fresh · efficiency

N is engine speed in rpm and g is unit-variance random noise, so cycles vary. heat_q is the heat released per kilogram of trapped air (default 2.5 MJ/kg; fuel's heating value over a stoichiometric air-fuel ratio, about 3 MJ/kg, less incomplete combustion). The efficiency also carries cycle-to-cycle variation, and very dilute charges (x_res above 0.32) burn partly or not at all. Definitions may scale the duration (`burn_scale`).

The heat is released along a Wiebe curve, x_b = 1 − exp(−5 (θ/Δθ)³) (efficiency parameter 5, form factor 2), ended at 1.15 of the duration.

Boosted engines may add a knock-limited spark retard: the advance falls by `knock_retard` degrees per bar of cylinder pressure at intake closing above `knock_onset_bar`, never below 1 degree.

**Mixture and fueling.** The equivalence ratio φ changes the burn: lean, heat scales with φ; rich, air limits it (a few per cent more heat up to φ 1.1, then less) and the excess fuel leaves unburned. Burn duration lengthens away from φ 1.05 (laminar flame speed taken as 1 − 2.1 (φ − 1.05)², floored at 0.3, with burn time proportional to it to the power −0.7). Very lean or very rich cycles misfire. All of these curves are estimates. At the default φ = 1 the code takes the original path and draws no random numbers.

### Wall heat: Woschni

    h = 3.26 · B^(−0.2) · (p/1 kPa)^0.8 · T^(−0.55) · w^0.8      W/(m² K)
    w = 6.18 S_p                                                   during gas exchange
    w = 2.28 S_p + 3.24e−3 · V_d T_ivc / (p_ivc V_ivc) · (p − p_mot)   otherwise

S_p is the mean piston speed (2 × stroke × revolutions per second), p_mot the motored pressure (isentropic from intake closing), B the bore. The wetted area is both end faces plus the exposed liner, the wall is at 450 K, and the heat is removed from the cylinder's energy each step. These are Woschni's standard constants.

### Engine kinds

- **Four-stroke piston engines**, any number of cylinders and banks. Firing angles are given per cylinder (0 to 720 degrees). Banks are given as cylinder-axis directions, so V, flat, W, VR and radial layouts are all the same model with different geometry.
- **Two-strokes** (`kind: "two_stroke"`). A 360-degree cycle. Each cylinder has its own crankcase gas, filled through reeds (open with the pressure difference, no reverse flow), a piston port, or a rotary disc, and compressed by the descending piston. The piston crown uncovers the exhaust and transfer ports, with open area following piston travel. The share of fresh charge in what leaves the exhaust port is taken as the square of its share in the cylinder (between perfect displacement and perfect mixing, an estimate); 80% of spilled fresh charge returns if the pipe's reflected wave pushes gas back before the port closes. Each cylinder can have a tapered expansion chamber (header, diffuser, belly, baffle cone, stinger) on the grid solver.
- **Wankel rotaries** (`kind: "wankel"`). Each rotor face is a chamber with a 1080-degree cycle, three per rotor, 360 shaft degrees apart. Volume is sinusoidal in shaft angle, V = V_c + (V_d/2)(1 − cos(4π θ/1080°)), with displacement per chamber 3·sqrt(3)·e·R·w. Ports replace valves: a flow area that ramps open and shut as the rotor uncovers it. Wall heat uses the chamber's own wetted area and the flank's surface speed. Burn duration is scaled to the longer cycle, so a burn takes the same share of the cycle as in a piston engine; the rotary definitions also set `burn_scale` to 1.25 (an estimate) for the long, thin chamber. Apex-seal leakage is not modelled.

## 5. Intake and exhaust plumbing (`engine.rs`, `spec.rs`)

A definition gives geometry; `EngineSpec::finish` builds the system and fills anything left out by rules of thumb from bore, stroke and cylinder count.

- **Intake:** snorkel pipe (open end, radiates), airbox volume, throttle orifice (opening follows the command to the power 1.6), plenum volume, one runner pipe per cylinder. Engines with individual throttles (`itb`) have a throttle per runner, and each open throttle radiates.
- **Exhaust:** one primary pipe per cylinder into a collector volume per exhaust group; then either secondary pipes into a merge volume feeding one or more outlets, or an outlet per group. An outlet is a mid pipe, a muffler section and a tailpipe whose open end radiates. Groups may mix banks, which is how 180-degree headers and twin-scroll turbines are expressed.

## 6. Sound (`engine.rs`, `audio.rs`, `live.rs`)

Each opening to the air (every tailpipe, the intake snorkel, each open individual throttle, a blow-off vent) is a monopole source. The pressure at the microphone, 1 m away, is

    p = ρ_0 · dQ/dt / (4 π r)

where Q is the volume flow out of the opening. This is summed into an exhaust pair (panned left and right by position) and an intake channel.

Added terms that are not resolved by the 1D model:

- **Jet noise** at each tailpipe (and a blow-off vent): Lighthill's eighth-power law, acoustic power W = 3e−5 · ρ u⁸ A / c⁵, radiated as p_rms = sqrt(W ρ c / (4π r²)) of band-pass noise centred on a Strouhal number of 0.25 (f = 0.25 u/D, held between 150 Hz and 6 kHz).
- **Structure noise**, a stand-in: the summed rate of cylinder pressure rise drives three band-pass "block modes" at 650, 1450 and 2900 Hz (Q 5, 6, 7). The frequencies and gains are invented and mixed low.
- **Turbo whistle**, a stand-in (section 8).

The simulation rate (48 to 192 kHz) is brought to 48 kHz by a windowed-sinc decimator (Blackman window, 48k + 1 taps for a ratio k, cutoff 0.45/k). The live mix puts the intake and block channels in the centre, rides the level (halving the dynamic range in dB against a slowly adapting reference) and soft-clips with tanh. The offline renderer writes the stems as 32-bit float pascals and a mixed WAV with an optional level compressor.

## 7. Crank, controls and drive (`engine.rs`)

- **Crank:** J dω/dt = T_gas − T_friction − T_drive − T_load. Friction is a mean effective pressure, FMEP = friction_scale × (0.70 + 0.11 N + 0.03 N²) bar with N in thousands of rpm, applied as torque FMEP · V_d / (4π). T_drive is a supercharger's drive torque; T_load is the dyno, the car, or nothing.
- **Controls** imitate an ECU: an idle air loop that integrates on the logarithm of the opening plus a fast spark-efficiency reserve, a rev limiter (fuel cut or spark cut) and an overrun fuel cut. These are control stand-ins, not physics.
- **Modes:** free (no load), dyno (speed held), and Drive (section 9).

### Afterfire and fueling

Unburned fuel and spare air leave each cylinder with its exhaust gas into a per-group reservoir that stands for the collector and the pipe just after it. A reservoir ignites when it holds enough flammable mixture and there is a flame (from a cylinder that has just burned, or a late burn with the valve open) or the collector wall is above 1050 K; the heat goes into the collector volume over a 1.5 ms Wiebe curve, and the pressure rise travels and radiates through the same model as everything else. Every threshold, efficiency and time constant here is an estimate chosen to give plausible behaviour.

### Lift-off pops

A separate control models fuel on the port walls. A share of the injected fuel lands on a film that evaporates with a time constant of 0.35 s; while the injectors work the cylinder still gets its commanded mixture, so the film only records how hard the engine has been working. When the injectors shut (overrun, gear change, fuel-cut limiter) the film keeps evaporating into the small charge of a shut throttle. A few such cycles light late and carry a flame into the exhaust; the rest pass their fuel and air on to gather in the exhaust, where a flame lights the gathered charge all at once. So a few large pops follow a lift and then die away as the film is used up. These numbers are estimates too.

## 8. Forced induction (`boost.rs`, `turbo.rs`)

Boost devices are lumped elements that move gas between the simulator's volumes, like the throttle does. They are allowed only on engines with one throttle and a plenum.

### Superchargers

Positive-displacement blowers, Roots and twin-screw.

A blower is a set of rotor pockets, each with its own gas. The rotors turn at a fixed ratio to the crank. Over one rotor revolution a pocket:

1. fills from the inlet volume while its volume grows from a small carry-back volume to the full pocket;
2. is carried round sealed (a twin-screw pocket is squeezed to its built-in volume ratio; a Roots pocket keeps its volume);
3. opens to the outlet over the helix twist while still at inlet pressure, so outlet gas rushes back into it;
4. is squeezed into the outlet as the lobes mesh, down to the carry-back volume, whose gas then expands back into the inlet.

A Roots blower with two rotors of n lobes has 2n pockets per revolution, alternating between the rotors. The whine is the pulses of that backflow and refill, at pockets × rotor speed, with a size set by the pressure ratio; nothing is a programmed tone. The drive torque is the work the pockets' volume changes do on their gas, divided by a mechanical efficiency, plus shaft drag; so the Roots blower's isochoric (Δp·V) loss appears without being put in. Clearance leakage is one orifice from outlet to inlet. A bypass valve opens from outlet to inlet as manifold vacuum rises, through a lagged diaphragm. An intercooler is an orifice with a pressure loss that removes a share (its effectiveness) of the charge temperature above the coolant's. The pocket ports' discharge coefficient is 0.8.

### Turbochargers

- **Compressor.** One speed line in non-dimensional form: isentropic head coefficient ψ against flow coefficient φ = ṁ / (ρ_in u_2 D_2²). Left of the surge line, and through reverse flow, it is the Moore-Greitzer cubic, ψ = ψ_0 + H (1 + 1.5 x − 0.5 x³) with x = φ/W − 1, which rises from the shut-off head to its peak at the surge line φ = 2W. Right of it a parabola falls from that peak to zero head at choke. Pressure ratio is (1 + ψ u_2² / (c_p T_in))^4 (the exponent γ/(γ − 1) = 4), so speed lines scale with tip speed as Casey and Robinson's relations do. Below the design tip speed the line's flow coefficients shrink by 0.55 + 0.45 u_2/u_2,design (an estimate), so the surge line leans the way real maps do. Efficiency peaks at the design flow and falls as 1 − 0.8 ((φ − φ_d)/φ_d)². Past choke the wheel adds no head and the passage costs about one dynamic head of the excess flow.
- **Compressor duct.** The flow through the compressor is a state (Greitzer's compression-system model): dṁ/dt = (A/L)(p_in · PR(ṁ) − p_out), updated semi-implicitly. Surge oscillation can therefore emerge when the throttle shuts with no blow-off valve.
- **Turbine.** A quasi-steady nozzle per scroll (the orifice law above) from the turbine inlet volume to the turbine outlet volume. The gas gives up η Δh_s, with Δh_s = c_p T_in (1 − (p_out/p_in)^(1/4)) and η = η_peak (1 − ((BSR − 0.7)/w)²) in blade speed ratio BSR = u_t / sqrt(2 Δh_s), never below zero. No reverse flow through the wheel.
- **Shaft.** One inertia whose kinetic energy takes turbine work and gives up compressor work and a bearing loss proportional to speed squared. Turbo lag is not scripted; it emerges.
- **Wastegate:** an orifice from turbine inlet to outlet, opened by a lagged spring-and-diaphragm actuator fed with compressor-outlet pressure. Optional electronic boost control bleeds the actuator signal with a duty cycle (feed-forward plus a PI loop on manifold boost).
- **Blow-off valve:** an orifice from the throttle inlet, opened by charge pressure above manifold pressure, either to a short vent pipe open to the air (which radiates, including its jet noise) or back to the compressor inlet.
- **Plumbing:** airbox, compressor, compressor outlet volume, charge pipe, intercooler, second charge pipe, throttle, plenum. Charge pipes are wave pipes like the rest. On the exhaust side each group's collector becomes a turbine scroll (twin-scroll is two groups into one turbine). Parallel twin turbos split the exhaust groups between them, each with its own intake and exhaust path.
- **Whistle, a stand-in.** The 1D model has no blades, so compressor blade-pass whistle cannot emerge. A source added to the intake channel carries partials at blades × shaft speed, (blades + splitters) × shaft speed and twice that, from the simulated shaft speed. Its size is 1 Pa × gain × (pressure ratio − 1) × (tip speed / 400 m/s)², doubled at the surge line, where band noise is added for the tip-clearance "whoosh". Partials above 0.45 of the sample rate or 20 kHz are left out.

When a definition leaves turbo sizes out, `finish_turbo` in `spec.rs` derives them (all estimates): the compressor is sized so the engine's air flow at a match speed (default 0.6 of the redline) sits at a design flow coefficient of 0.11 and peak head coefficient 0.58; the turbine area gives 1.3 times the compressor's power there from 1050 K exhaust; inertia is 2.7e−5 kg m² scaled with wheel diameter to the fifth power from a 52 mm reference.

## 9. Vehicles in Drive (`engine.rs`, `Drive::for_engine`)

In Drive the engine pushes a vehicle through a clutch and a gearbox.

- **The vehicle.** A definition may carry a `vehicle` block for the car or motorcycle the engine came in: curb mass, gear count and ratios, final drive, rolling radius, tyre size, which wheels are driven, drag area. Missing values are derived (geometric gear series by vehicle kind, a final drive from the top speed or from the speed where published power meets rolling resistance and drag, typical radius and drag area). Without a block the engine pushes a generic car scaled to its displacement.
- **Road load:** rolling resistance 0.012 × weight plus aerodynamic drag ½ ρ CdA v² with ρ = 1.2 kg/m³. 85% of engine torque reaches the wheels in cars, 90% in chain-drive motorcycles.
- **Clutch:** when pulling away it slips, holding the engine near its published peak-torque speed (within limits) until road speed catches up, then locks.
- **Tyres:** the driven wheels turn with the engine, and the tyre force rises with slip to a peak at 9% and falls to three-quarters of the peak by 49%. Peak grip is μ times the load on the driven wheels, with μ set by tyre era (0.85 for bias-ply tyres and anything before 1975, 1.15 for ZR-rated tyres from 2000, 1.0 otherwise) and load transfer from a centre of gravity at 0.2 of the wheelbase. A motorcycle's force is capped at 1 g as a stand-in for the wheelie limit.
- **Traction control** trims the throttle so the tyres stay near the slip of peak grip; it only ever takes throttle away.
- **Gearbox:** automatic upshifts at 0.96 of the redline with a 0.18 s throttle cut, downshifts when revs fall low.

Not modelled: wheelies, gearbox and wheel inertia, downforce, torque converters.

## 10. Checks built into the CLI

- `engsim selftest`: a closed-open tube rings at the frequency theory gives (with end correction), and a sealed shock tube conserves mass to rounding.
- `engsim libtest`: definition parsing, derived defaults, crank-pin sharing, cam conversion and the other library rules.
- `engsim validate`: each real engine at its published peak-torque and peak-power speeds; `--write` regenerates `engines/VALIDATION.md`.
- `engsim boost ...` and `engsim turbo ...`: rig tests of the blower and the turbocharger elements (delivered volume, work against Δp·V, map reproduction, turbine curve, energy and mass balances, transmission loss, spool, surge on lift-off).
- `engsim afterfire ...` and `engsim liftoff ...`: the fueling models across every engine.
