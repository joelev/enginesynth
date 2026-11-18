//! Engine descriptions. Everything the simulation needs is geometry and timing:
//! bore, stroke, firing angles, valve events, pipe lengths and diameters.
//! SI units (metres, kg); angles in crank degrees.

use serde::{Deserialize, Serialize};

include!(concat!(env!("OUT_DIR"), "/defs.rs"));

fn nan() -> f64 {
    f64::NAN
}
fn kind_piston() -> String {
    "piston".into()
}
fn one() -> f64 {
    1.0
}
fn two() -> usize {
    2
}
fn merge_none() -> String {
    "none".into()
}
fn hyst_default() -> f64 {
    300.0
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct Source {
    pub url: String,
    #[serde(default)]
    pub what: String,
}

/// Published output, for checking the model (`engsim validate`).
#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct Published {
    #[serde(default)]
    pub power_kw: f64,
    #[serde(default)]
    pub power_rpm: f64,
    #[serde(default)]
    pub torque_nm: f64,
    #[serde(default)]
    pub torque_rpm: f64,
}

/// Valve events the way cam cards and service manuals give them: opening and closing angles
/// measured at a stated valve lift, plus peak lift. See `CamSpec::seat_events`.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct CamSpec {
    /// The valve lift at which the four angles below are quoted, metres (0 = seat to seat).
    #[serde(default)]
    pub at_lift: f64,
    /// degrees: intake opens before TDC, closes after BDC; exhaust opens before BDC, closes after TDC
    pub ivo: f64,
    pub ivc: f64,
    pub evo: f64,
    pub evc: f64,
    /// peak valve lift, metres
    pub in_lift: f64,
    pub ex_lift: f64,
}

/// Seat-to-seat valve events and peak lifts, as the simulator uses them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Events {
    pub ivo: f64,
    pub ivc: f64,
    pub evo: f64,
    pub evc: f64,
    pub in_lift: f64,
    pub ex_lift: f64,
}

impl CamSpec {
    /// The simulator's lift curve is L(x) = Lmax * sin(pi x)^1.4 over the seat-to-seat window,
    /// x from 0 to 1. A source that quotes events at lift h (often 1 mm, or 0.050 in) is giving
    /// the two points where that curve crosses h, i.e. x_h = asin((h / Lmax)^(1/1.4)) / pi from
    /// each end. So the quoted duration is (1 - 2 x_h) of the seat-to-seat duration, and each
    /// seat event lies x_h * seat-duration outside the quoted one. This keeps the quoted
    /// events and the peak lift exact; what it cannot keep is the real lobe's fullness (the area
    /// under a real lift curve is usually somewhat larger than under sin^1.4).
    pub fn seat_events(&self) -> Events {
        let widen = |open: f64, close: f64, lift: f64| -> (f64, f64) {
            if self.at_lift <= 0.0 || lift <= self.at_lift {
                return (open, close);
            }
            let xh = (self.at_lift / lift).powf(1.0 / 1.4).asin() / std::f64::consts::PI;
            let quoted = open + 180.0 + close;
            let seat = quoted / (1.0 - 2.0 * xh);
            (open + xh * seat, close + xh * seat)
        };
        let (ivo, ivc) = widen(self.ivo, self.ivc, self.in_lift);
        let (evo, evc) = widen(self.evo, self.evc, self.ex_lift);
        Events { ivo, ivc, evo, evc, in_lift: self.in_lift, ex_lift: self.ex_lift }
    }
}

fn three() -> usize {
    3
}

/// Bypass valve of a positive-displacement blower: outlet back to inlet, opened by manifold
/// vacuum (Eaton installations on throttled engines have one).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct BypassSpec {
    /// valve diameter, m (derived from the blower displacement)
    #[serde(default = "nan")]
    pub d: f64,
    /// manifold gauge pressure, kPa, at or below which the valve is fully open (derived -40)
    #[serde(default = "nan")]
    pub open_kpa: f64,
    /// manifold gauge pressure, kPa, at or above which it is shut (derived -10)
    #[serde(default = "nan")]
    pub shut_kpa: f64,
    /// actuator time constant, s (derived 0.05)
    #[serde(default = "nan")]
    pub tau: f64,
}

/// Charge-air cooler between the blower outlet and what follows it.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct IntercoolerSpec {
    /// volume of the core and its tanks, m^3
    #[serde(default = "nan")]
    pub vol: f64,
    /// equivalent orifice diameter for its pressure loss, m
    #[serde(default = "nan")]
    pub d: f64,
    /// share of the charge's temperature rise above the coolant that it removes
    #[serde(default = "nan")]
    pub effectiveness: f64,
    /// coolant (or ambient air) temperature, K
    #[serde(default = "nan")]
    pub t_cool: f64,
}

/// Forced induction. Absent for naturally aspirated engines. Only positive-displacement
/// superchargers driven from the crank exist so far (`sim/src/boost.rs`, docs/physics.md, "Superchargers").
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct BoostSpec {
    /// "roots" or "twin_screw"
    pub kind: String,
    /// how the library describes it, e.g. "supercharged (Eaton M90 Roots blower)"
    #[serde(default)]
    pub induction: String,
    /// gas swept per rotor revolution, m^3 (derived: 0.4 of the engine's displacement)
    #[serde(default = "nan")]
    pub displacement: f64,
    /// rotor speed over crank speed (derived from `boost_bar` at `boost_rpm`)
    #[serde(default = "nan")]
    pub drive_ratio: f64,
    /// Roots: rotors and lobes per rotor; twin-screw: lobes of the male rotor (which `drive_ratio` refers to)
    #[serde(default = "two")]
    pub rotors: usize,
    #[serde(default = "three")]
    pub lobes: usize,
    /// twist of the lobes along the rotor, degrees (sets how gradually a pocket opens to the outlet)
    #[serde(default = "nan")]
    pub helix: f64,
    /// built-in volume ratio (twin-screw; 1 for Roots)
    #[serde(default = "nan")]
    pub vi: f64,
    /// "before": the throttle feeds the blower inlet; "after": the blower feeds the throttle
    #[serde(default)]
    pub throttle: String,
    /// volume between the throttle and the rotors (throttle before), m^3
    #[serde(default = "nan")]
    pub inlet_vol: f64,
    /// blower outlet volume when it is not the plenum (intercooler or throttle after), m^3
    #[serde(default = "nan")]
    pub outlet_vol: f64,
    /// full open area of one pocket's inlet or outlet port, m^2
    #[serde(default = "nan")]
    pub port_area: f64,
    /// effective clearance area for leakage from outlet to inlet, m^2
    #[serde(default = "nan")]
    pub leak_area: f64,
    /// gas carried back through the mesh, as a share of one pocket
    #[serde(default = "nan")]
    pub carry: f64,
    /// mechanical efficiency of the drive (belt, gears, bearings)
    #[serde(default = "nan")]
    pub mech_eff: f64,
    /// drag at the rotor shaft, N m
    #[serde(default = "nan")]
    pub drag: f64,
    /// gauge boost, bar, and the engine speed it is quoted at (used only to derive `drive_ratio`)
    #[serde(default = "nan")]
    pub boost_bar: f64,
    #[serde(default = "nan")]
    pub boost_rpm: f64,
    #[serde(default)]
    pub bypass: Option<BypassSpec>,
    #[serde(default)]
    pub intercooler: Option<IntercoolerSpec>,
    /// throttle after the blower: a pipe from the blower outlet (or intercooler) to the throttle,
    /// solved like the other pipes. Length and diameter, m (0 = none)
    #[serde(default)]
    pub charge_len: f64,
    #[serde(default)]
    pub charge_d: f64,
    /// turbocharger (`kind: "turbo"`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turbo: Option<TurboSpec>,
}

/// A turbocharger: compressor, turbine and shaft (`sim/src/turbo.rs`). Every field is derived
/// when left out, from the engine's air flow at `match_rpm` and the boost target (all estimates).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct TurboSpec {
    /// number of parallel turbochargers (1, or 2 for one per bank). Each takes an equal share of the
    /// exhaust groups and has its own air filter, compressor, charge pipes, intercooler (when the
    /// engine has one), wastegate and blow-off valve; the charge paths join at the throttle and each
    /// turbine has its own downpipe and tailpipe. Every per-turbo size (stated or derived) is for one
    /// turbo, and derived sizes are for its share of the engine.
    #[serde(default = "one_usize", skip_serializing_if = "is_one_usize")]
    pub count: usize,
    /// engine speed at which the compressor sits at its design point (a smaller turbo: lower)
    #[serde(default = "nan")]
    pub match_rpm: f64,
    /// compressor wheel tip diameter, m; main blades and splitter blades
    #[serde(default = "nan")]
    pub d2: f64,
    #[serde(default = "six")]
    pub blades: usize,
    #[serde(default = "six")]
    pub splitters: usize,
    /// compressor peak isentropic efficiency, and its design flow coefficient (mass flow / (inlet density x tip speed x d2^2))
    #[serde(default = "nan")]
    pub eta_c: f64,
    #[serde(default = "nan")]
    pub phi_d: f64,
    /// isentropic head coefficient at the surge line (the peak of the speed line), and at zero flow
    #[serde(default = "nan")]
    pub psi_peak: f64,
    #[serde(default = "nan")]
    pub psi_shut: f64,
    /// design flow coefficient over the surge flow coefficient, and choke over design (map width)
    #[serde(default = "nan")]
    pub surge_ratio: f64,
    #[serde(default = "nan")]
    pub choke_ratio: f64,
    /// compressor tip speed at the design point, m/s. Below it the surge line moves to lower flow
    /// coefficients (in proportion 0.55 + 0.45 x tip speed / this), as real maps' surge lines lean
    #[serde(default = "nan")]
    pub tip_d: f64,
    /// inlet duct of the compressor (the Greitzer duct): length and diameter, m
    #[serde(default = "nan")]
    pub duct_len: f64,
    #[serde(default = "nan")]
    pub duct_d: f64,
    /// turbine equivalent nozzle area (all scrolls together), m^2; wheel diameter, m; peak efficiency
    #[serde(default = "nan")]
    pub turbine_area: f64,
    #[serde(default = "nan")]
    pub turbine_d: f64,
    #[serde(default = "nan")]
    pub eta_t: f64,
    /// half-width of the turbine efficiency parabola in blade speed ratio (0.7: the Euler-work form, zero at standstill and at 1.4, so a stalled wheel still gives torque)
    #[serde(default = "nan")]
    pub bsr_width: f64,
    /// volute volume added to the turbine inlet, and the turbine outlet volume, m^3
    #[serde(default = "nan")]
    pub volute_vol: f64,
    #[serde(default = "nan")]
    pub turbine_out_vol: f64,
    /// "single" (all exhaust groups merge before the turbine) or "twin" (each group its own scroll)
    #[serde(default)]
    pub scroll: String,
    /// rotating inertia of the shaft, kg m^2; bearing loss at the design speed, W
    #[serde(default = "nan")]
    pub inertia: f64,
    #[serde(default = "nan")]
    pub bearing_w: f64,
    /// internal wastegate: diameter, m; actuator spring: starts to open at `wg_crack_bar` gauge
    /// boost, full open `wg_span_bar` above that; time constant, s
    #[serde(default = "nan")]
    pub wg_d: f64,
    #[serde(default = "nan")]
    pub wg_crack_bar: f64,
    #[serde(default = "nan")]
    pub wg_span_bar: f64,
    #[serde(default = "nan")]
    pub wg_tau: f64,
    /// electronic boost control: a PI loop holds manifold boost at `boost_bar` by bleeding the
    /// wastegate actuator's signal (duty 0 to `bc_max_duty`); off unless set
    #[serde(default)]
    pub boost_control: bool,
    #[serde(default = "nan")]
    pub bc_max_duty: f64,
    /// blow-off valve: "vent" (to the air), "recirc" (back to the compressor inlet) or "none";
    /// diameter, m; opens when charge pressure exceeds manifold pressure by `bov_open_kpa`,
    /// fully at `bov_full_kpa`; time constant, s
    #[serde(default)]
    pub bov: String,
    #[serde(default = "nan")]
    pub bov_d: f64,
    #[serde(default = "nan")]
    pub bov_open_kpa: f64,
    #[serde(default = "nan")]
    pub bov_full_kpa: f64,
    #[serde(default = "nan")]
    pub bov_tau: f64,
    /// charge pipes, [length, diameter] in m: compressor to intercooler (or throttle), intercooler to throttle
    #[serde(default)]
    pub charge_pipes: Vec<[f64; 2]>,
    /// knock-limited spark retard: degrees per bar of intake pressure at valve closing above `knock_onset_bar` (absolute)
    #[serde(default = "nan")]
    pub knock_retard: f64,
    #[serde(default = "nan")]
    pub knock_onset_bar: f64,
    /// compressor whistle stand-in gain (0 = none; 1, the default, is the reference level)
    #[serde(default = "nan")]
    pub whistle: f64,
}

fn six() -> usize {
    6
}
fn one_usize() -> usize {
    1
}
fn is_one_usize(n: &usize) -> bool {
    *n == 1
}

impl BoostSpec {
    /// Fill derived fields from the engine's displacement (m^3, four-stroke convention) and speeds.
    fn finish(&mut self, disp: f64, redline: f64) -> Result<(), String> {
        if self.kind != "roots" && self.kind != "twin_screw" && self.kind != "turbo" {
            return Err(format!("boost kind '{}' is not modelled (roots, twin_screw, turbo)", self.kind));
        }
        let fill = |x: &mut f64, v: f64| {
            if x.is_nan() {
                *x = v;
            }
        };
        if self.kind == "turbo" {
            return self.finish_turbo(disp, redline);
        }
        let screw = self.kind == "twin_screw";
        if self.throttle.is_empty() {
            self.throttle = "before".into();
        }
        if self.throttle != "before" && self.throttle != "after" {
            return Err("boost throttle must be \"before\" or \"after\" the blower".into());
        }
        fill(&mut self.displacement, 0.4 * disp);
        let vd = self.displacement;
        fill(&mut self.helix, if screw { 300.0 } else { 60.0 });
        fill(&mut self.vi, if screw { 1.8 } else { 1.0 });
        fill(&mut self.inlet_vol, 0.35 * vd);
        fill(&mut self.outlet_vol, 0.6 * vd);
        let n = if screw { self.lobes } else { self.rotors * self.lobes } as f64;
        // a pocket's port: about as wide as the pocket is long, so its area scales as a pocket's
        // volume to the 2/3 (estimate)
        fill(&mut self.port_area, 0.6 * (vd / n).powf(2.0 / 3.0));
        fill(&mut self.leak_area, 0.005 * vd.powf(2.0 / 3.0));
        fill(&mut self.carry, 0.03);
        fill(&mut self.mech_eff, 0.92);
        fill(&mut self.drag, 0.3);
        fill(&mut self.boost_bar, 0.5);
        fill(&mut self.boost_rpm, 0.7 * redline);
        // Drive ratio: the blower must deliver the engine's air at the boost pressure ratio, at
        // inlet density, with some leakage (an estimate, used only when no ratio is given).
        let pr = 1.0 + self.boost_bar * 1e5 / 101_325.0;
        fill(&mut self.drive_ratio, pr * 0.5 * disp * 0.9 / (vd * 0.85));
        if let Some(b) = self.bypass.as_mut() {
            fill(&mut b.d, 1.1 * vd.cbrt() * 0.3);
            fill(&mut b.open_kpa, -40.0);
            fill(&mut b.shut_kpa, -10.0);
            fill(&mut b.tau, 0.05);
            if !(b.shut_kpa > b.open_kpa) {
                return Err("bypass shut_kpa must be above open_kpa".into());
            }
        }
        if let Some(ic) = self.intercooler.as_mut() {
            fill(&mut ic.vol, 2.0 * vd);
            fill(&mut ic.d, 0.06);
            fill(&mut ic.effectiveness, 0.7);
            fill(&mut ic.t_cool, 293.15);
        }
        if !(self.displacement > 0.0 && self.drive_ratio > 0.0 && self.lobes >= 2) {
            return Err("boost needs a positive displacement, drive ratio and at least two lobes".into());
        }
        Ok(())
    }

    /// Size a turbocharger for the engine (every rule here is an estimate; see docs/physics.md, "Turbochargers").
    fn finish_turbo(&mut self, disp: f64, redline: f64) -> Result<(), String> {
        let fill = |x: &mut f64, v: f64| {
            if x.is_nan() {
                *x = v;
            }
        };
        // parallel turbos: each one (and its intercooler) is sized for its share of the engine
        let count = self.turbo.as_ref().map_or(1, |t| t.count);
        if !(1..=2).contains(&count) {
            return Err("turbo count must be 1 or 2".into());
        }
        let disp = disp / count as f64;
        self.throttle = "after".into();
        fill(&mut self.boost_bar, 0.7);
        // the blower fields mean nothing here; zero them so the spec round-trips through JSON (NaN does not)
        for x in [&mut self.displacement, &mut self.drive_ratio, &mut self.helix, &mut self.vi, &mut self.inlet_vol, &mut self.port_area, &mut self.leak_area, &mut self.carry, &mut self.mech_eff, &mut self.drag, &mut self.boost_rpm] {
            fill(x, 0.0);
        }
        fill(&mut self.outlet_vol, 0.3e-3);
        if let Some(ic) = self.intercooler.as_mut() {
            fill(&mut ic.vol, 1.2 * disp + 1.0e-3);
            fill(&mut ic.d, 0.03 * (disp * 1000.0).sqrt().max(1.0));
            fill(&mut ic.effectiveness, 0.7);
            fill(&mut ic.t_cool, 293.15);
        }
        let t = self.turbo.get_or_insert_with(|| serde_json::from_str("{}").unwrap());
        let (cp, r, t_atm, rho_atm) = (crate::gas::CP, crate::gas::R_GAS, crate::gas::T_ATM, crate::gas::RHO_ATM);
        fill(&mut t.match_rpm, 0.6 * redline);
        fill(&mut t.eta_c, 0.74);
        fill(&mut t.phi_d, 0.11);
        fill(&mut t.psi_peak, 0.58);
        fill(&mut t.psi_shut, 0.30);
        let pr = 1.0 + self.boost_bar * 1e5 / 101_325.0;
        // air at the match speed: the charge is denser by the pressure ratio, less the heating
        // an intercooler does not take out (volumetric efficiency 0.9)
        let m_d = rho_atm * 0.5 * disp * t.match_rpm / 60.0 * 0.9 * pr.powf(if self.intercooler.is_some() { 0.85 } else { 0.7 });
        fill(&mut t.surge_ratio, 1.25);
        fill(&mut t.choke_ratio, 1.5);
        // head at the design flow on the stable branch (falls quadratically from the surge peak to choke)
        let rr = (1.0 - 1.0 / t.surge_ratio) / (t.choke_ratio - 1.0 / t.surge_ratio);
        let psi_d = t.psi_peak * (1.0 - rr * rr);
        let dh_is = cp * t_atm * (pr.sqrt().sqrt() - 1.0);
        let u2 = (dh_is / psi_d).sqrt();
        fill(&mut t.tip_d, u2);
        fill(&mut t.d2, (m_d / (rho_atm * t.phi_d * u2)).sqrt());
        let omega_d = 2.0 * u2 / t.d2;
        fill(&mut t.duct_d, 0.75 * t.d2);
        fill(&mut t.duct_len, 0.15);
        // turbine: enough isentropic power at the match point to drive the compressor with 30%
        // to spare (which the wastegate bleeds off), from exhaust at about 1050 K
        fill(&mut t.eta_t, 0.70);
        fill(&mut t.bsr_width, 0.7);
        let p_c = m_d * dh_is / t.eta_c;
        let m_t = m_d * 1.07;
        // (a road exhaust behind the turbine holds its outlet near 1.5 bar at full power in this model)
        let (t_x, p_y) = (1050.0, 1.5e5);
        let dhs = 1.3 * p_c / (m_t * t.eta_t);
        let er = 1.0 / (1.0 - dhs / (cp * t_x)).max(0.3).powi(4);
        let p_x = p_y * er;
        let x = 1.0 / er;
        let psi = (8.0 * (x.powf(1.5) - x.powf(1.75))).max(1e-4).sqrt();
        fill(&mut t.turbine_area, m_t * (r * t_x).sqrt() / (p_x * psi));
        let c_s = (2.0 * dhs).sqrt();
        fill(&mut t.turbine_d, 2.0 * 0.7 * c_s / omega_d);
        fill(&mut t.volute_vol, 0.25 * disp);
        fill(&mut t.turbine_out_vol, 0.2 * disp);
        if t.scroll.is_empty() {
            t.scroll = "single".into();
        }
        // a 2.0 L engine's turbo: 2.7e-5 kg m^2 (Chung and Lee, Transactions of KSAE 24(1), 2016, Table 4; the wheel size is not given there,
        // taken as 52 mm, an estimate), scaled with diameter^5 (geometric similarity)
        fill(&mut t.inertia, 2.7e-5 * (t.d2 / 0.052).powi(5));
        fill(&mut t.bearing_w, 400.0 * (t.d2 / 0.046).powi(2));
        fill(&mut t.wg_d, 0.7 * (4.0 * t.turbine_area / std::f64::consts::PI).sqrt());
        fill(&mut t.wg_crack_bar, self.boost_bar - 0.08);
        fill(&mut t.wg_span_bar, 0.15);
        fill(&mut t.wg_tau, 0.05);
        fill(&mut t.bc_max_duty, 0.6);
        if t.bov.is_empty() {
            t.bov = "vent".into();
        }
        if !["vent", "recirc", "none"].contains(&t.bov.as_str()) {
            return Err("turbo bov must be vent, recirc or none".into());
        }
        fill(&mut t.bov_d, 0.022 * (disp * 1000.0).sqrt().max(1.0));
        fill(&mut t.bov_open_kpa, 25.0);
        fill(&mut t.bov_full_kpa, 50.0);
        fill(&mut t.bov_tau, 0.005);
        if t.charge_pipes.is_empty() {
            let d = (1.1 * t.d2).max(0.045);
            t.charge_pipes = if self.intercooler.is_some() { vec![[0.6, d], [0.8, d]] } else { vec![[0.8, d]] };
        }
        fill(&mut t.knock_retard, 0.0);
        fill(&mut t.knock_onset_bar, 1.4);
        fill(&mut t.whistle, 1.0);
        if !(t.d2 > 0.0 && t.turbine_area > 0.0 && t.inertia > 0.0) {
            return Err("turbo sizing failed".into());
        }
        Ok(())
    }
}

fn kind_car() -> String {
    "car".into()
}

/// The vehicle an engine came in (`vehicle` in a definition; docs/physics.md, "Vehicles in Drive"). Only
/// `name`, `curb_kg` and `gear_count` are required; whatever else is left out is derived by
/// `engine::Drive::for_engine` (all such values are estimates).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct VehicleSpec {
    /// the car or motorcycle, as the app shows it, e.g. "Citroën 2CV6 Club (1980)"
    pub name: String,
    /// "car", "motorcycle" or "race car"
    #[serde(default = "kind_car")]
    pub kind: String,
    /// curb weight, kg (the driver or rider is added in Drive)
    pub curb_kg: f64,
    pub gear_count: usize,
    /// gearbox ratios, first gear first (absent: a geometric series by kind)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gears: Vec<f64>,
    /// final drive; for motorcycles primary times secondary reduction
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_drive: Option<f64>,
    /// rolling radius of the driven wheel, m
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheel_radius_m: Option<f64>,
    /// tyre size as published (informational)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tyre: Option<String>,
    /// drag area Cd x A, m^2
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cda_m2: Option<f64>,
    /// driven wheels: "front", "rear" or "all" (sets the grip; absent: half the weight, no weight transfer)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driven: Option<String>,
    /// published top speed, km/h (sets a missing final drive)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_speed_kmh: Option<f64>,
    /// what is estimated
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

impl VehicleSpec {
    fn check(&self) -> Result<(), String> {
        let pos = |x: Option<f64>| x.map_or(true, |v| v.is_finite() && v > 0.0);
        if self.name.trim().is_empty() {
            return Err("vehicle: name is empty".into());
        }
        if !matches!(self.kind.as_str(), "car" | "motorcycle" | "race car") {
            return Err(format!("vehicle: kind must be \"car\", \"motorcycle\" or \"race car\", not {:?}", self.kind));
        }
        if !(self.curb_kg.is_finite() && self.curb_kg > 0.0) {
            return Err("vehicle: curb_kg must be positive".into());
        }
        if !(1..=10).contains(&self.gear_count) {
            return Err("vehicle: gear_count must be 1 to 10".into());
        }
        if !self.gears.is_empty() && self.gears.len() != self.gear_count {
            return Err(format!("vehicle: {} gear ratios for gear_count {}", self.gears.len(), self.gear_count));
        }
        if !self.gears.iter().all(|&g| g.is_finite() && g > 0.0) || !self.gears.windows(2).all(|w| w[1] < w[0]) {
            return Err("vehicle: gears must be positive and fall from first to top".into());
        }
        if !matches!(self.driven.as_deref(), None | Some("front") | Some("rear") | Some("all")) {
            return Err(format!("vehicle: driven must be \"front\", \"rear\" or \"all\", not {:?}", self.driven));
        }
        if !(pos(self.final_drive) && pos(self.wheel_radius_m) && pos(self.cda_m2) && pos(self.top_speed_kmh)) {
            return Err("vehicle: final_drive, wheel_radius_m, cda_m2 and top_speed_kmh must be positive when given".into());
        }
        Ok(())
    }
}

/// One engine. Only the first block is required in a definition file; every field marked
/// "derived" may be left out and is then filled from bore, stroke and cylinder count by
/// `finish()` using the same rules of thumb the original presets used.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct EngineSpec {
    pub name: String,
    pub label: String,
    /// "inline", "v", "flat", "rotary", or anything else when `bank_dirs` is given ("w", "vr", "radial")
    pub layout: String,
    /// Included angle between the two banks (0 inline, 180 flat). Ignored when `bank_dirs` is given.
    #[serde(default)]
    pub bank_angle: f64,
    #[serde(default = "nan")]
    pub bore: f64,
    #[serde(default = "nan")]
    pub stroke: f64,
    #[serde(default = "nan")]
    pub rod: f64,
    pub cr: f64,
    /// Crank angle (0..720) at which each cylinder is at firing TDC.
    pub firing: Vec<f64>,
    /// Which bank each cylinder is on (derived: all 0).
    #[serde(default)]
    pub bank: Vec<usize>,
    /// Position along the crankshaft (derived: 0, 1, 2, ...). Cylinders at the same position share a slot.
    #[serde(default)]
    pub slot: Vec<usize>,
    /// Cylinder-axis direction of each bank, degrees from vertical. When given it replaces
    /// `bank_angle`, so any number of banks can be described: four rows for a W engine, or one
    /// "bank" per cylinder for a radial.
    #[serde(default)]
    pub bank_dirs: Vec<f64>,
    // valve events seat to seat, degrees: IVO before TDC, IVC after BDC, EVO before BDC, EVC after TDC (derived)
    #[serde(default = "nan")]
    pub ivo: f64,
    #[serde(default = "nan")]
    pub ivc: f64,
    #[serde(default = "nan")]
    pub evo: f64,
    #[serde(default = "nan")]
    pub evc: f64,
    /// valve head diameters and peak lifts, metres (derived)
    #[serde(default = "nan")]
    pub in_valve_d: f64,
    #[serde(default = "nan")]
    pub ex_valve_d: f64,
    #[serde(default = "nan")]
    pub in_lift: f64,
    #[serde(default = "nan")]
    pub ex_lift: f64,
    /// valves per cylinder
    #[serde(default = "two")]
    pub in_valves: usize,
    #[serde(default = "two")]
    pub ex_valves: usize,
    /// Cam events as published (at a stated lift). When present it replaces ivo/ivc/evo/evc and the lifts.
    #[serde(default)]
    pub cam: Option<CamSpec>,
    /// Second cam profile for cam-switching engines, used above `cam_switch_rpm`.
    #[serde(default)]
    pub cam_high: Option<CamSpec>,
    #[serde(default)]
    pub cam_switch_rpm: f64,
    /// Revs must fall this far below the switch point before the low cam returns.
    #[serde(default = "hyst_default")]
    pub cam_switch_hyst: f64,
    // intake (all derived)
    #[serde(default)]
    pub itb: bool,
    #[serde(default = "nan")]
    pub runner_len: f64,
    /// Per-cylinder runner lengths where they differ (empty = `runner_len` for all).
    #[serde(default)]
    pub runner_len_cyl: Vec<f64>,
    #[serde(default = "nan")]
    pub runner_d: f64,
    #[serde(default = "nan")]
    pub plenum_vol: f64,
    #[serde(default = "nan")]
    pub throttle_d: f64,
    #[serde(default = "nan")]
    pub snorkel_len: f64,
    #[serde(default = "nan")]
    pub snorkel_d: f64,
    #[serde(default = "nan")]
    pub airbox_vol: f64,
    // exhaust (all derived)
    #[serde(default)]
    pub primary_len: Vec<f64>,
    #[serde(default = "nan")]
    pub primary_d: f64,
    /// Which cylinders feed each collector. A group may mix banks (180-degree headers).
    #[serde(default)]
    pub groups: Vec<Vec<usize>>,
    #[serde(default = "nan")]
    pub collector_vol: f64,
    #[serde(default = "nan")]
    pub secondary_len: f64,
    #[serde(default = "nan")]
    pub secondary_d: f64,
    /// "none": each collector has its own outlet. "y": all merge into one. "x": merge, two outlets.
    #[serde(default = "merge_none")]
    pub merge: String,
    #[serde(default = "nan")]
    pub mid_len: f64,
    #[serde(default = "nan")]
    pub mid_d: f64,
    #[serde(default = "nan")]
    pub muffler_len: f64,
    #[serde(default = "nan")]
    pub muffler_d: f64,
    #[serde(default = "nan")]
    pub muffler_fric: f64,
    /// muffler exit loss coefficient on the tailpipe's dynamic head for pipes solved along
    /// characteristics (absent: 7, fitted to the grid solver's muffler; see docs/physics.md, section 3)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muffler_zeta: Option<f64>,
    #[serde(default = "nan")]
    pub tail_len: f64,
    #[serde(default = "nan")]
    pub tail_d: f64,
    /// Megaphone: tailpipe exit diameter if larger than tail_d (0 = straight).
    #[serde(default)]
    pub tail_flare_d: f64,
    // running gear (all derived)
    #[serde(default = "nan")]
    pub inertia: f64,
    #[serde(default = "nan")]
    pub redline: f64,
    #[serde(default = "nan")]
    pub idle: f64,
    #[serde(default = "one")]
    pub friction_scale: f64,
    /// Heat released per kg of fresh air burned (J/kg).
    #[serde(default = "nan")]
    pub heat_q: f64,
    /// "piston" or "wankel"
    #[serde(default = "kind_piston")]
    pub kind: String,
    // --- Wankel only. Each rotor has three working chambers; `firing` lists every chamber
    // (three per rotor, 360 shaft degrees apart) and one chamber's cycle is 1080 shaft degrees.
    /// eccentricity, rotor (generating) radius, rotor width, metres
    #[serde(default)]
    pub ecc: f64,
    #[serde(default)]
    pub rotor_r: f64,
    #[serde(default)]
    pub rotor_w: f64,
    /// shaft angle at which each rotor's first chamber is at firing TDC
    #[serde(default)]
    pub rotor_phase: Vec<f64>,
    /// port open and close, shaft degrees after a chamber's firing TDC (0..1080)
    #[serde(default)]
    pub port_in: [f64; 2],
    #[serde(default)]
    pub port_ex: [f64; 2],
    /// port flow areas, m^2
    #[serde(default)]
    pub port_in_area: f64,
    #[serde(default)]
    pub port_ex_area: f64,
    /// burn duration relative to a piston engine at the same point in its cycle
    #[serde(default = "one")]
    pub burn_scale: f64,
    // --- library information (not used by the physics)
    /// true = an invented, typical engine; false = modelled on a real one from public data
    #[serde(default)]
    pub generic: bool,
    /// grouping for an engine picker: "single", "twin", "triple", "inline-four", "five", "six",
    /// "V6", "V8", "V10", "V12", "W", "flat", "rotary", "radial", "two-stroke"
    #[serde(default)]
    pub family: String,
    /// the real engine this is modelled on (empty for generic engines)
    #[serde(default)]
    pub based_on: String,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub published: Option<Published>,
    /// what in this definition is estimated rather than sourced
    #[serde(default)]
    pub notes: String,
    // --- Two-stroke only (`kind: "two_stroke"`). The piston uncovers ports in the cylinder wall;
    // the crankcase under each piston pumps the fresh charge. One cycle is 360 degrees.
    /// exhaust and transfer ports open this many degrees after TDC (and close symmetrically before it)
    #[serde(default)]
    pub ex_port_open: f64,
    #[serde(default)]
    pub tr_port_open: f64,
    /// total transfer port area, m^2 (exhaust and intake use `port_ex_area`, `port_in_area`)
    #[serde(default)]
    pub tr_port_area: f64,
    /// how the crankcase is filled: "reed", "piston" (port in the cylinder wall under the piston
    /// skirt) or "disc" (rotary disc valve)
    #[serde(default)]
    pub induction: String,
    /// piston or disc induction: opens this many degrees before TDC and closes this many after
    #[serde(default)]
    pub in_window: [f64; 2],
    /// crankcase volume at TDC divided by its volume at BDC (about 1.3 to 1.6)
    #[serde(default)]
    pub crankcase_cr: f64,
    /// each cylinder's own exhaust pipe as [length, start diameter, end diameter] sections,
    /// open to air at the end (an expansion chamber: header, diffuser, belly, baffle cone,
    /// stinger). When present it replaces the primaries, collectors and outlet.
    #[serde(default)]
    pub chamber: Vec<[f64; 3]>,
    /// Forced induction (absent: naturally aspirated). Plenum engines only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost: Option<BoostSpec>,
    /// The car or motorcycle this engine came in, for Drive (absent: the generic car). Library
    /// information plus gearing and weight; never used outside Drive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vehicle: Option<VehicleSpec>,
}

impl EngineSpec {
    /// Fill every derived field that the definition left out, and check the rest.
    pub fn finish(mut self) -> Result<EngineSpec, String> {
        let n = self.firing.len();
        if n == 0 {
            return Err("no cylinders".into());
        }
        let wankel = self.kind == "wankel";
        if wankel {
            if !(self.ecc > 0.0 && self.rotor_r > 0.0 && self.rotor_w > 0.0) || n % 3 != 0 {
                return Err("a wankel needs ecc, rotor_r, rotor_w and three chambers per rotor".into());
            }
            // stand-in piston dimensions of about the same four-stroke-equivalent size, used only
            // by the plumbing rules of thumb below
            if self.bore.is_nan() {
                self.bore = 0.103;
            }
            if self.stroke.is_nan() {
                self.stroke = 0.0785;
            }
            if self.rod.is_nan() {
                self.rod = 0.15;
            }
        } else if !(self.bore > 0.0 && self.stroke > 0.0 && self.rod > 0.0) {
            return Err("a piston engine needs bore, stroke and rod".into());
        }
        if self.bank.is_empty() {
            self.bank = vec![0; n];
        }
        if self.slot.is_empty() {
            self.slot = (0..n).collect();
        }
        if self.bank.len() != n || self.slot.len() != n {
            return Err("bank and slot need one entry per cylinder".into());
        }
        if !self.bank_dirs.is_empty() && self.bank.iter().any(|b| *b >= self.bank_dirs.len()) {
            return Err("a cylinder refers to a bank that bank_dirs does not list".into());
        }
        let units = n / if wankel { 3 } else { 1 };
        let bore = self.bore;
        let vd_l = std::f64::consts::FRAC_PI_4 * bore * bore * self.stroke * n as f64 * 1000.0;
        let fill = |x: &mut f64, v: f64| {
            if x.is_nan() {
                *x = v;
            }
        };
        fill(&mut self.ivo, 16.0);
        fill(&mut self.ivc, 56.0);
        fill(&mut self.evo, 54.0);
        fill(&mut self.evc, 14.0);
        // one big valve where a head has a single intake or exhaust valve
        fill(&mut self.in_valve_d, if self.in_valves == 1 { 0.48 } else { 0.38 } * bore);
        fill(&mut self.ex_valve_d, if self.ex_valves == 1 { 0.40 } else { 0.32 } * bore);
        let (ivd, evd) = (self.in_valve_d, self.ex_valve_d);
        fill(&mut self.in_lift, if self.in_valves == 1 { 0.27 } else { 0.32 } * ivd);
        fill(&mut self.ex_lift, if self.ex_valves == 1 { 0.28 } else { 0.33 } * evd);
        fill(&mut self.runner_len, 0.30);
        fill(&mut self.runner_d, 0.45 * bore);
        fill(&mut self.plenum_vol, 1.2e-3 * vd_l);
        fill(&mut self.throttle_d, 0.040 * vd_l.sqrt().max(0.8));
        let thr = self.throttle_d;
        fill(&mut self.snorkel_len, 0.45);
        fill(&mut self.snorkel_d, thr * 1.05);
        fill(&mut self.airbox_vol, 2.0e-3 + 1.2e-3 * vd_l);
        if self.primary_len.is_empty() {
            self.primary_len = vec![0.45; units];
        }
        fill(&mut self.primary_d, 0.43 * bore);
        let pd = self.primary_d;
        if self.groups.is_empty() {
            self.groups = vec![(0..units).collect()];
        }
        fill(&mut self.collector_vol, 0.6e-3 * vd_l / n as f64 + 0.15e-3);
        fill(&mut self.secondary_len, 1.2);
        fill(&mut self.secondary_d, pd * (n as f64).sqrt() * 0.82);
        fill(&mut self.mid_len, 0.8);
        fill(&mut self.mid_d, pd * (n as f64).sqrt() * 0.82);
        fill(&mut self.muffler_len, 0.5);
        fill(&mut self.muffler_d, 0.17);
        fill(&mut self.muffler_fric, 0.3);
        fill(&mut self.tail_len, 0.5);
        fill(&mut self.tail_d, pd * (n as f64).sqrt() * 0.75);
        fill(&mut self.inertia, 0.08 + 0.03 * vd_l);
        fill(&mut self.redline, 7000.0);
        fill(&mut self.idle, 850.0);
        fill(&mut self.heat_q, 2.5e6);
        if self.primary_len.len() != units {
            return Err(format!("primary_len needs {units} entries"));
        }
        if !self.runner_len_cyl.is_empty() && self.runner_len_cyl.len() != units {
            return Err(format!("runner_len_cyl needs {units} entries"));
        }
        let mut seen = vec![false; units];
        for g in &self.groups {
            for &i in g {
                if i >= units || seen[i] {
                    return Err("exhaust groups must list every cylinder exactly once".into());
                }
                seen[i] = true;
            }
        }
        if seen.iter().any(|s| !s) {
            return Err("exhaust groups must list every cylinder exactly once".into());
        }
        if self.cam_high.is_some() && !(self.cam_switch_rpm > 0.0) {
            return Err("cam_high needs cam_switch_rpm".into());
        }
        if self.boost.is_some() {
            if self.itb || self.kind == "two_stroke" || !self.chamber.is_empty() {
                return Err("boost needs an engine with one throttle and a plenum (not individual throttles, not a two-stroke)".into());
            }
            let (disp, redline) = (self.disp(), self.redline);
            self.boost.as_mut().unwrap().finish(disp, redline)?;
            let count = self.boost.as_ref().and_then(|b| b.turbo.as_ref()).map_or(1, |t| t.count);
            if count > 1 && self.groups.len() % count != 0 {
                return Err(format!("{count} parallel turbos need the exhaust groups to split evenly between them ({} groups)", self.groups.len()));
            }
        }
        if let Some(v) = &self.vehicle {
            v.check()?;
        }
        Ok(self)
    }

    /// Seat-to-seat valve events for the low (or only) cam, or the high cam.
    pub fn events(&self, high: bool) -> Events {
        let base = Events { ivo: self.ivo, ivc: self.ivc, evo: self.evo, evc: self.evc, in_lift: self.in_lift, ex_lift: self.ex_lift };
        match (high, &self.cam_high, &self.cam) {
            (true, Some(c), _) => c.seat_events(),
            (_, _, Some(c)) => c.seat_events(),
            _ => base,
        }
    }
    /// Intake runner length for a cylinder (or rotor).
    pub fn runner_len_of(&self, unit: usize) -> f64 {
        self.runner_len_cyl.get(unit).copied().unwrap_or(self.runner_len)
    }

    pub fn n(&self) -> usize {
        self.firing.len()
    }
    pub fn is_wankel(&self) -> bool {
        self.kind == "wankel"
    }
    /// Working chambers that share one intake runner and one exhaust primary (3 on a rotor).
    pub fn chambers_per_unit(&self) -> usize {
        if self.is_wankel() {
            3
        } else {
            1
        }
    }
    /// Cylinders, or rotors.
    pub fn units(&self) -> usize {
        self.n() / self.chambers_per_unit()
    }
    /// Crank or eccentric-shaft angle for one chamber's full cycle, degrees.
    pub fn is_two_stroke(&self) -> bool {
        self.kind == "two_stroke"
    }
    pub fn cycle_deg(&self) -> f64 {
        if self.is_wankel() {
            1080.0
        } else if self.is_two_stroke() {
            360.0
        } else {
            720.0
        }
    }
    /// Swept volume of one chamber.
    pub fn disp_cyl(&self) -> f64 {
        if self.is_wankel() {
            3.0 * 3.0f64.sqrt() * self.ecc * self.rotor_r * self.rotor_w
        } else {
            std::f64::consts::FRAC_PI_4 * self.bore * self.bore * self.stroke
        }
    }
    /// Displacement on the four-stroke convention: the volume swept by the chambers that fire
    /// in two shaft revolutions (so a two-rotor with 654 cc chambers counts as 2.6 L here, though
    /// it is sold as a 1.3).
    pub fn disp(&self) -> f64 {
        if self.is_wankel() {
            2.0 * self.disp_cyl() * self.units() as f64
        } else if self.is_two_stroke() {
            // every cylinder fires every revolution
            2.0 * self.disp_cyl() * self.n() as f64
        } else {
            self.disp_cyl() * self.n() as f64
        }
    }
    /// Direction of a bank's cylinder axis, degrees from vertical.
    pub fn bank_dir(&self, bank: usize) -> f64 {
        if !self.bank_dirs.is_empty() {
            return self.bank_dirs[bank];
        }
        if self.layout == "inline" || self.is_wankel() {
            0.0
        } else if bank == 0 {
            -0.5 * self.bank_angle
        } else {
            0.5 * self.bank_angle
        }
    }
    /// Crank-pin angle for a cylinder, in the crank's own frame: the pin points along the
    /// cylinder's axis when that cylinder is at top dead centre.
    pub fn pin_angle(&self, i: usize) -> f64 {
        (self.firing[i] + self.bank_dir(self.bank[i])).rem_euclid(360.0)
    }
    /// Firing intervals in crank degrees, in firing order, over one cycle.
    pub fn intervals(&self) -> Vec<f64> {
        let cyc = self.cycle_deg();
        let per = if self.is_wankel() { 360.0 } else { cyc };
        let mut f: Vec<f64> = self.firing.iter().map(|a| a.rem_euclid(per)).collect();
        f.sort_by(|a, b| a.partial_cmp(b).unwrap());
        f.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        (0..f.len()).map(|i| if i + 1 < f.len() { f[i + 1] - f[i] } else { f[0] + per - f[i] }).collect()
    }
}

/// Ids of every built-in engine, in library order.
pub fn names() -> Vec<&'static str> {
    DEFS.iter().map(|(id, _)| *id).collect()
}

/// Parse a definition (the JSON text of an `engines/defs` file) into a complete spec.
pub fn parse(json: &str) -> Result<EngineSpec, String> {
    serde_json::from_str::<EngineSpec>(json).map_err(|e| e.to_string())?.finish()
}

pub fn preset(name: &str) -> Option<EngineSpec> {
    let (id, json) = DEFS.iter().find(|(id, _)| *id == name)?;
    match parse(json) {
        Ok(s) => Some(s),
        Err(e) => panic!("engines/defs/{id}.json: {e}"),
    }
}
