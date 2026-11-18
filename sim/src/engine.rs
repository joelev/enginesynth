//! The whole engine: cylinders, valves, volumes, pipes, crank dynamics and the microphones.

use crate::boost::*;
use crate::gas::*;
use crate::prof::{self, cat, cnt};
use crate::pipe::*;
use crate::spec::EngineSpec;
use crate::turbo::*;
use std::f64::consts::PI;

const FOUR_PI: f64 = 4.0 * PI;
const T_CYL_WALL: f64 = 450.0;
const VALVE_CD: f64 = 0.62;

// ---- mixture and afterfire (all estimates; see docs/physics.md, "Afterfire and fueling")
/// Lean and rich misfire limits (equivalence ratio): half the cycles misfire there, none
/// PHI_BAND inside the limit and all PHI_BAND beyond it.
const PHI_LEAN: f64 = 0.60;
const PHI_RICH: f64 = 1.85;
const PHI_BAND: f64 = 0.08;
/// Fueled overrun cycle: share of its normal heat released in the cylinder, spark after TDC
/// (degrees), and how much longer its burn is.
const OVERRUN_BURN: f64 = 0.2;
const OVERRUN_SPARK_ATDC: f64 = 15.0;
const OVERRUN_DUR: f64 = 1.8;
/// Exhaust reservoir: residence time in crank degrees of a four-stroke cycle (1.5 cycles).
const AF_RESIDENCE_DEG: f64 = 1080.0;
/// The gas arriving now (the latest exhaust slug, not yet mixed into the rest) is tracked with
/// this time constant in crank degrees of a four-stroke cycle.
const AF_ARRIVAL_DEG: f64 = 180.0;
/// Flammable window of the reservoir's fuel-to-air equivalence ratio.
const AF_PHI_MIN: f64 = 0.5;
const AF_PHI_MAX: f64 = 3.0;
/// Fuel needed in the reservoir, as a share of one cylinder's full-load stoichiometric charge.
const AF_FUEL_MIN: f64 = 0.03;
/// Flame window after a cylinder that burned starts exhausting, s.
const AF_FLAME_T: f64 = 2.0e-3;
/// Collector wall temperature that ignites a flammable mixture by itself, K. The wall is a
/// first-order lag of the collector gas temperature (the gas itself dips with every slug of
/// cold air on the overrun while the pipe stays hot).
const AF_T_IGNITE: f64 = 1050.0;
const AF_WALL_T: f64 = 0.5;
/// Mean ignition rate while eligible, per second (so the moment is random).
const AF_RATE: f64 = 1000.0;
/// Share of what can burn that does burn, and the time over which it releases its heat.
const AF_EFF: f64 = 0.5;
const AF_RELEASE: f64 = 1.5e-3;
/// Decay of the telemetry glow, s (a 1.5 ms flash would fall between 60 fps frames).
const AF_GLOW_T: f64 = 0.025;

// ---- lift-off pops (all estimates; see docs/physics.md, "Lift-off pops")
/// Share of the injected fuel that lands on the port walls at lift-off pops 1.0 and a full
/// charge of air (the share falls in proportion to the charge: at low manifold pressure the
/// spray evaporates before it reaches the wall).
pub const LIFT_WET: f64 = 0.14;
/// Time constant of the film evaporating into the intake charge, s.
pub const LIFT_TAU: f64 = 0.35;
/// Share of film-fed cycles (injector off) that light at all, when the film gives a mixture of
/// LIFT_PHI_FIRE or richer; in proportion below that. Such a cycle burns late, like a fueled
/// overrun cycle, and its flame follows it into the exhaust.
pub const LIFT_FIRE: f64 = 0.035;
pub const LIFT_PHI_FIRE: f64 = 0.5;
/// The gathered charge, all as shares of one full-load charge for every cylinder of the group:
/// the mass of gas in the pipe region where it collects (new exhaust displaces it), the least
/// burnable charge a flame can light, and the most that burns in one go (for that last one a
/// group counts as four cylinders at most, so a big group's bang is no bigger than a four's).
pub const LIFT_HOLD: f64 = 0.7;
pub const LIFT_GATHER: f64 = 0.04;
pub const LIFT_CAP: f64 = 0.12;
/// Leanest gathered charge a flame can light (fuel over air). It is well under the limit for an
/// even mixture (AF_PHI_MIN) because film fuel arrives in streaks, and above LIFT_WET, so the
/// thin mixture a fuel-cut limiter passes at full air never lights.
pub const LIFT_PHI_MIN: f64 = 0.2;
/// The gathered charge also thins out by itself (mixing and cooling in the pipe), s.
pub const LIFT_LINGER: f64 = 0.22;

/// Heat per kilogram of air relative to a stoichiometric charge. Lean, it follows the fuel;
/// rich, the air limits it: a little more up to phi 1.1 (lumped charge cooling and extra
/// product moles), then less as more of the oxygen ends as CO instead of CO2. Estimates.
pub fn mixture_heat(phi: f64) -> f64 {
    if phi <= 1.0 {
        phi
    } else if phi <= 1.1 {
        let x = 1.0 - (phi - 1.0) / 0.1;
        1.0 + 0.03 * (1.0 - x * x)
    } else {
        1.03 - 0.25 * (phi - 1.1)
    }
}

/// Burn duration relative to phi 1.0: turbulent burning scales roughly with laminar flame
/// speed to the 0.7 power, and gasoline's laminar flame speed is taken as peaking at phi
/// 1.05 and falling quadratically either side (the Metghalchi-Keck form, from memory).
pub fn mixture_burn_dur(phi: f64) -> f64 {
    let s = |p: f64| (1.0 - 2.1 * (p - 1.05) * (p - 1.05)).max(0.3);
    (s(1.0) / s(phi)).powf(0.7)
}

/// Share of cycles that fire at this mixture (1 inside the limits, falling to 0 across them).
pub fn mixture_fires(phi: f64) -> f64 {
    let ss = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    ss((phi - PHI_LEAN + PHI_BAND) / (2.0 * PHI_BAND)) * ss((PHI_RICH + PHI_BAND - phi) / (2.0 * PHI_BAND))
}

/// Unburned fuel and unconsumed air in one exhaust group's collector region.
/// Fuel is counted as the mass of air it would need to burn completely (stoichiometric air),
/// so the equivalence ratio is simply fuel / air.
#[derive(Clone, Default)]
pub struct AfGroup {
    pub fuel: f64,
    pub air: f64,
    /// the same for the gas arriving now
    pub in_fuel: f64,
    pub in_air: f64,
    /// exhaust gas mass into the collector since the last ignition, kg: the burned pocket has
    /// to be pushed on by new gas (the collector's own mass of it) before the next can light
    pub flushed: f64,
    /// lift-off pops: unburned fuel and air from film-fed cycles gathered in the pipe, and the
    /// mass of gas that region holds, kg
    pub pool_fuel: f64,
    pub pool_air: f64,
    /// one full-load charge of air for every cylinder of the group, kg
    pub hold: f64,
    /// collector wall temperature, K
    pub t_wall: f64,
    /// seconds of flame window left
    pub flame: f64,
    /// heat of the burn now being released (J), and how far through it is
    q_ev: f64,
    t_ev: f64,
    xb_prev: f64,
    releasing: bool,
    /// heat release rate this step (W), and the telemetry glow 0..1
    pub rate: f64,
    pub glow: f64,
    /// counters for the checks
    pub events: u64,
    pub q_sum: f64,
    pub q_max: f64,
    pub last_t: f64,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Limiter {
    /// fuel is cut: cylinders pump air (the original behaviour)
    FuelCut,
    /// spark is cut: cylinders still get fuel but do not fire
    SparkCut,
}

pub struct Volume {
    pub name: String,
    pub m: f64,
    pub e: f64,
    pub v: f64,
    pub(crate) dm: f64,
    pub(crate) de: f64,
}

impl Volume {
    pub fn new(name: &str, v: f64, t: f64) -> Self {
        let m = P_ATM / (R_GAS * t) * v;
        Volume { name: name.into(), m, e: m * CV * t, v, dm: 0.0, de: 0.0 }
    }
    #[inline]
    pub fn p(&self) -> f64 {
        GM1 * self.e / self.v
    }
    #[inline]
    pub fn t(&self) -> f64 {
        self.e / (self.m * CV)
    }
    #[inline]
    pub fn rho(&self) -> f64 {
        self.m / self.v
    }
    /// Apply this step's accumulated inflow (test rigs; `System` does this itself).
    pub fn apply(&mut self) {
        self.m += self.dm;
        self.e += self.de;
        self.dm = 0.0;
        self.de = 0.0;
    }
    /// Set the gas to pressure p and temperature t (test rigs).
    pub fn set(&mut self, p: f64, t: f64) {
        self.m = p * self.v / (R_GAS * t);
        self.e = self.m * CV * t;
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum VRef {
    Ambient,
    Vol(usize),
}

pub struct Orifice {
    pub a: VRef,
    pub b: VRef,
    pub area: f64,
    pub throttle: bool,
    /// Mass flow a -> b, kg/s (last step).
    pub mdot: f64,
}

pub struct Cylinder {
    pub phase: f64,
    pub a_p: f64,
    pub crank_r: f64,
    pub rod: f64,
    pub v_c: f64,
    pub v_d: f64,
    pub bore: f64,
    pub m: f64,
    pub e: f64,
    pub v: f64,
    pub c: f64,
    pub m_fresh: f64,
    reverted: f64,
    pub in_pipe: usize,
    pub in_port: Option<usize>,
    pub ex_pipe: usize,
    // valve events in cycle angle (radians, 0 = firing TDC)
    ivo: f64,
    ivc: f64,
    evo: f64,
    evc: f64,
    in_lift_max: f64,
    ex_lift_max: f64,
    /// valves per cylinder (as a factor on flow area)
    in_n: f64,
    ex_n: f64,
    /// [ivo, ivc, evo, evc (cycle radians), intake lift, exhaust lift] for the low and high cams
    ev: [[f64; 6]; 2],
    /// which cam this cylinder is on; it changes only at firing TDC, when all valves are shut
    pub cam_high: bool,
    in_d: f64,
    ex_d: f64,
    // combustion
    planned: bool,
    burning: bool,
    q_tot: f64,
    burn_dur: f64,
    burn_ang: f64,
    spark: f64,
    xb_prev: f64,
    p_ivc: f64,
    v_ivc: f64,
    t_ivc: f64,
    // telemetry
    pub lift_in: f64,
    pub lift_ex: f64,
    pub hrr: f64,
    pub p_prev: f64,
    pub air_last: f64,
    pub p_peak: f64,
    pub q_dbg: f64,
    /// angle of one full cycle of this chamber (4 pi for a four-stroke piston, 6 pi for a Wankel chamber)
    pub cyc: f64,
    /// Wankel chamber: rotor radius, width (0 for a piston cylinder)
    rotor_r: f64,
    rotor_w: f64,
    in_area: f64,
    ex_area: f64,
    burn_scale: f64,
    // two-stroke: crankcase gas under the piston, and the ports the piston uncovers
    two: bool,
    pub cc_m: f64,
    pub cc_e: f64,
    cc_v: f64,
    cc_v_tdc: f64,
    cc_burned: f64,
    spilled: f64,
    tpo: f64,
    tr_area: f64,
    induction: u8,
    in_btdc: f64,
    in_atdc: f64,
    // afterfire: what this cycle leaves unburned (fuel as stoichiometric air, kg), whether it
    // burned at all (a flame follows its exhaust), and which exhaust group it feeds
    pub fuel_left: f64,
    pub air_left: f64,
    pub fired: bool,
    pub late: bool,
    pub af_group: Option<usize>,
    /// liquid fuel on this cylinder's port walls (as stoichiometric air, kg); stays 0 unless
    /// lift-off pops are on
    pub film: f64,
    /// this cycle ran on the film alone: what it leaves goes to the gathered charge
    pub pooled: bool,
    /// Woschni length scale to the power -0.2 (the length is fixed, so the power is taken once)
    clen_pow: f64,
}

impl Cylinder {
    #[inline]
    pub fn p(&self) -> f64 {
        GM1 * self.e / self.v
    }
    #[inline]
    pub fn t(&self) -> f64 {
        self.e / (self.m * CV)
    }
    pub fn cc_vol(&self) -> f64 {
        self.cc_v
    }
    /// Exchange the cylinder's gas with its crankcase's, so code written for the cylinder
    /// (flow to and from a pipe) can be applied to the crankcase.
    #[inline]
    fn swap_crankcase(&mut self) {
        std::mem::swap(&mut self.m, &mut self.cc_m);
        std::mem::swap(&mut self.e, &mut self.cc_e);
        std::mem::swap(&mut self.v, &mut self.cc_v);
    }
    /// Open fraction of a wall port that the piston crown starts to uncover `open` radians
    /// after TDC: it grows with piston travel and is full at BDC.
    #[inline]
    fn wall_port(&self, c: f64, open: f64) -> f64 {
        if c <= open || c >= self.cyc - open {
            return 0.0;
        }
        let x0 = self.piston_x(open);
        ((self.piston_x(c) - x0) / (self.piston_x(PI) - x0)).clamp(0.0, 1.0)
    }
    #[inline]
    fn piston_x(&self, c: f64) -> f64 {
        // distance of the piston below its TDC position
        let r = self.crank_r;
        let s = c.sin();
        r + self.rod - (r * c.cos() + (self.rod * self.rod - r * r * s * s).sqrt())
    }
    /// Chamber volume and (pistons) the piston's distance below TDC at cycle angle c, so callers
    /// that need both take the trigonometry once. Same arithmetic as `volume` and `piston_x`.
    #[inline]
    fn volume_x(&self, c: f64) -> (f64, f64) {
        if self.rotor_r > 0.0 {
            (self.volume(c), 0.0)
        } else {
            let x = self.piston_x(c);
            (self.v_c + self.a_p * x, x)
        }
    }
    /// Length scale for wall heat transfer: the bore, or a rotor's width.
    #[inline]
    fn wall_len(&self) -> f64 {
        if self.rotor_r > 0.0 {
            self.rotor_w
        } else {
            self.bore
        }
    }
    #[inline]
    fn volume(&self, c: f64) -> f64 {
        if self.rotor_r > 0.0 {
            // a rotor chamber's volume is sinusoidal in shaft angle, two swings per cycle
            self.v_c + 0.5 * self.v_d * (1.0 - (4.0 * PI * c / self.cyc).cos())
        } else {
            self.v_c + self.a_p * self.piston_x(c)
        }
    }
    /// Wetted wall area and the gas speed used for wall heat transfer, given the chamber volume
    /// `v` and piston position `x` at this angle (from `volume_x`).
    #[inline]
    fn wall(&self, v: f64, x: f64, omega: f64, stroke: f64) -> (f64, f64) {
        if self.rotor_r > 0.0 {
            // two side walls (volume / width each) plus rotor flank and housing arc; gas is swept
            // round with the rotor, so the speed scale is the flank's surface speed
            let area = 2.0 * v / self.rotor_w + 2.0 * 3.0f64.sqrt() * self.rotor_r * self.rotor_w;
            (area, 0.6 * self.rotor_r * omega / 3.0)
        } else {
            (2.0 * self.a_p + PI * self.bore * x, 2.0 * stroke * omega / (2.0 * PI))
        }
    }
}

/// Opening of a port that the rotor uncovers: a quick ramp, full area, a quick ramp shut.
#[inline]
fn port_shape(c: f64, open: f64, close: f64) -> f64 {
    if c <= open || c >= close {
        return 0.0;
    }
    let ramp = 0.14 * (close - open);
    let x = ((c - open).min(close - c) / ramp).min(1.0);
    x * x * (3.0 - 2.0 * x)
}

fn lift_shape(c: f64, open: f64, close: f64, fast: bool) -> f64 {
    if c <= open || c >= close {
        0.0
    } else {
        prof::count(cnt::POWF, 1);
        let s = (PI * (c - open) / (close - open)).sin();
        if fast {
            // s^1.4 = s * exp(0.4 ln s); s is in (0, 1]
            s * (0.4 * s.ln()).exp()
        } else {
            s.powf(1.4)
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum SrcKind {
    PipeLeft(usize),
    PipeRight(usize),
    Orifice(usize),
}

pub struct Source {
    pub kind: SrcKind,
    /// 0 = exhaust, 1 = intake
    pub stem: usize,
    pub pan: f64,
    pub dist: f64,
    /// an intake-side opening that also radiates jet noise (a blow-off valve's vent)
    pub jet: bool,
    q_prev: f64,
    ic1: f64,
    ic2: f64,
    /// Recent history of this opening's sound (pascals at 1 m), newest last, at SOUND_RATE.
    /// The picture draws the wave it sends out: pressure at distance r is hist(now - r/c) / r.
    pub hist: Vec<f32>,
    hist_pos: usize,
    hist_acc: f64,
    hist_n: usize,
}

/// Samples per second and length of each opening's sound history sent with every telemetry frame
/// (128 samples at 12 kHz is 10.7 ms, which is 3.7 m of travel at the speed of sound).
pub const SOUND_RATE: f64 = 12_000.0;
pub const SOUND_SAMPLES: usize = 128;

#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f64,
    ic2: f64,
}
impl Svf {
    #[inline]
    fn bp(&mut self, x: f64, g: f64, k: f64) -> f64 {
        let v1 = (self.ic1 + g * (x - self.ic2)) / (1.0 + g * (g + k));
        let v2 = self.ic2 + g * v1;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        k * v1
    }
}

pub struct Drive {
    pub mass: f64,
    pub wheel_r: f64,
    pub final_drive: f64,
    pub gears: Vec<f64>,
    pub shift_rpm: f64,
    pub shift_time: f64,
    pub gear: usize,
    pub v: f64,
    pub shifting: f64,
    pub cda: f64,
    /// shift up at shift_rpm and down at down_rpm without being asked
    pub auto: bool,
    /// 0 disables automatic downshifts
    pub down_rpm: f64,
    /// 0..1
    pub brake: f64,
    /// clutch fully engaged (engine speed tied to road speed)
    pub locked: bool,
    /// slipping clutch: most torque it passes at full throttle, N m per litre of displacement
    /// (scaled by the throttle), and how fast its torque rises with slip, N m per rad/s
    pub clutch_nm_per_l: f64,
    pub clutch_gain: f64,
    /// engine speed the slipping clutch holds the engine at while the car is slower than that,
    /// rad/s; the clutch locks once road speed reaches it. 0 (the generic car): the clutch pulls
    /// the engine towards road speed and locks within 2% of it
    pub clutch_launch: f64,
    /// share of the engine's torque that reaches the wheels (1 for the generic car)
    pub eta: f64,
    /// most drive force the tyres pass before they spin, N (infinite for the generic car, which
    /// keeps its original equations)
    pub grip: f64,
    /// crank torque averaged over about a cycle (vehicles only), N m
    pub t_avg: f64,
    /// driven wheels' grip from weight transfer: their load grows by this share of the net
    /// accelerating force (CG height over wheelbase; negative for front drive, 0 when not known)
    pub grip_hl: f64,
    /// tyre friction coefficient (vehicles only)
    pub mu: f64,
    /// traction control (or a driver feeding the throttle in): share of the throttle the engine
    /// is given, so the tyres are held at their grip instead of spinning (1 for the generic car)
    pub tc: f64,
}

/// Driver or rider added to a vehicle's curb weight in Drive, kg (estimate; docs/physics.md, "Vehicles in Drive").
pub const OCCUPANT_KG: f64 = 80.0;
/// A race car's driver, kg (estimate).
pub const RACE_DRIVER_KG: f64 = 75.0;
/// With a vehicle block the slipping clutch acts like a driver launching the car: it holds the
/// engine near a launch speed (launch_rpm) until road speed catches up, then locks; this share of
/// the redline is that speed when the engine has no published peak-torque speed (estimate).
pub const LAUNCH_SHARE: f64 = 0.4;

/// Engine speed the launching clutch holds, rpm (estimate; docs/physics.md, "Vehicles in Drive"): a driver
/// launching hard holds the engine near its torque peak, so the published peak-torque speed,
/// kept between 1.5 x idle and 0.7 x redline; without one, LAUNCH_SHARE x redline (at least 1.5 x idle).
pub fn launch_rpm(s: &EngineSpec) -> f64 {
    let lo = 1.5 * s.idle;
    match s.published.as_ref().map_or(0.0, |p| p.torque_rpm) {
        r if r > 0.0 => r.clamp(lo, (0.7 * s.redline).max(lo)),
        _ => (LAUNCH_SHARE * s.redline).max(lo),
    }
}
/// ... and it slips this share of the redline above its target when passing the published peak torque.
pub const LAUNCH_SLIP: f64 = 0.03;
/// Driveline efficiency, engine to wheels: cars and race cars, chain-drive motorcycles (estimates).
pub const DRIVELINE_CAR: f64 = 0.85;
pub const DRIVELINE_BIKE: f64 = 0.90;
/// Tyre friction coefficient by era (estimates): bias-ply and other tyres before 1975, radial
/// street tyres to the end of the 1990s, ZR-rated performance tyres from 2000.
pub const MU_EARLY: f64 = 0.85;
pub const MU_RADIAL: f64 = 1.0;
pub const MU_MODERN_ZR: f64 = 1.15;
/// A motorcycle's drive force is capped at this many g, standing in for the wheelie limit (estimate).
pub const WHEELIE_G: f64 = 1.0;
/// Share of a car's weight on its driven wheels: front drive, rear drive or not known (estimates);
/// all-wheel drive carries all of it.
pub const FRONT_SHARE_FWD: f64 = 0.6;
pub const DRIVEN_SHARE_CAR: f64 = 0.5;
/// Centre-of-gravity height over wheelbase, for weight transfer when the driven wheels are known (estimate).
pub const CG_HL: f64 = 0.2;
/// Tyre slip at peak grip (estimate): the force rises in proportion up to it and falls to
/// three-quarters of the peak by 40% more slip.
pub const TYRE_SLIP_PEAK: f64 = 0.09;
/// How fast traction control moves the throttle share, per second per unit of error.
pub const TC_RATE: f64 = 2.0;
/// On a boosted engine the traction control's trim is spark retard, not throttle: this many degrees
/// at no trim left (1 - trim of it in general). An estimate, standing in for an ECU's torque cut by
/// spark: closing the throttle on boost opened the blow-off valve and collapsed the boost.
pub const TC_RETARD_MAX: f64 = 60.0;

/// Tyre force as a share of its peak, for a slip ratio (positive when the wheel turns faster than the road).
pub fn tyre_force(slip: f64) -> f64 {
    let a = slip.abs();
    let f = if a <= TYRE_SLIP_PEAK { a / TYRE_SLIP_PEAK } else { 1.0 - 0.25 * ((a - TYRE_SLIP_PEAK) / 0.4).min(1.0) };
    if slip < 0.0 { -f } else { f }
}

/// Tyre friction coefficient for a vehicle: by the first year in its name and its tyre size
/// (docs/physics.md, "Vehicles in Drive").
pub fn tyre_mu(v: &crate::spec::VehicleSpec) -> f64 {
    let year = v.name.as_bytes().windows(4).filter_map(|w| std::str::from_utf8(w).ok()?.parse::<u32>().ok()).find(|y| (1880..2100).contains(y));
    // bias-ply sizes are written as section-rim in inches with a dash ("8.00-15", "5.20-13")
    let bias = v.tyre.as_deref().map_or(false, |t| t.as_bytes().windows(4).any(|w| w[0].is_ascii_digit() && w[1] == b'-' && w[2].is_ascii_digit() && w[3].is_ascii_digit()));
    match year {
        _ if bias => MU_EARLY,
        Some(y) if y < 1975 => MU_EARLY,
        Some(y) if y >= 2000 && v.tyre.as_deref().map_or(false, |t| t.contains("ZR")) => MU_MODERN_ZR,
        _ => MU_RADIAL,
    }
}

impl Drive {
    /// The car the engine pushes in Drive: the vehicle from its definition when it has one
    /// (docs/physics.md, "Vehicles in Drive"), otherwise the generic car scaled to the engine. In `gear`
    /// (0-based, clamped to the gearbox) with the clutch locked at engine speed `omega`, no
    /// automatic shifting; the live session switches those on (`live.rs`, `set_drive`).
    pub fn for_engine(s: &EngineSpec, gear: usize, omega: f64) -> Drive {
        let Some(veh) = &s.vehicle else {
            // generic car: about 2.6 kg per N m of typical torque (105 N m per litre), six gears
            let torque = 105.0 * s.disp() * 1000.0;
            let mass = (2.6 * torque).clamp(180.0, 2200.0);
            let final_drive = 3.7 * (s.redline / 7000.0);
            let gears = vec![3.2, 2.05, 1.45, 1.1, 0.88, 0.72];
            let gear = gear.min(gears.len() - 1);
            let v = omega * 0.32 / (gears[gear] * final_drive);
            return Drive { mass, wheel_r: 0.32, final_drive, gears, shift_rpm: 0.96 * s.redline, shift_time: 0.18, gear, v, shifting: 0.0, cda: 0.68, auto: false, down_rpm: 0.0, brake: 0.0, locked: true, clutch_nm_per_l: 130.0, clutch_gain: 1.5, clutch_launch: 0.0, eta: 1.0, grip: f64::INFINITY, t_avg: 0.0, grip_hl: 0.0, mu: 0.0, tc: 1.0 };
        };
        let kind = veh.kind.as_str();
        let mass = veh.curb_kg + if kind == "race car" { RACE_DRIVER_KG } else { OCCUPANT_KG };
        let n = veh.gear_count.max(1);
        let gears = if veh.gears.len() == n {
            veh.gears.clone()
        } else {
            // geometric series between a first and a top gear typical of the kind (estimates)
            let (first, top): (f64, f64) = match kind {
                "motorcycle" => (2.6, if n <= 4 { 1.0 } else { 0.9 }),
                "race car" => (2.6, 0.85),
                _ => (3.6, if n <= 4 { 1.0 } else if n == 5 { 0.8 } else { 0.7 }),
            };
            if n == 1 {
                vec![(first * top).sqrt()]
            } else {
                (0..n).map(|i| first * (top / first).powf(i as f64 / (n - 1) as f64)).collect()
            }
        };
        let wheel_r = veh.wheel_radius_m.unwrap_or(if kind == "motorcycle" { 0.31 } else { 0.32 });
        let cda = veh.cda_m2.unwrap_or(match kind {
            "motorcycle" => 0.55,
            "race car" => 0.9,
            _ => 0.68,
        });
        let w_red = s.redline * PI / 30.0;
        let top_gear = gears[n - 1];
        let final_drive = veh.final_drive.unwrap_or_else(|| {
            // top gear at the redline gives the published top speed, or failing that the speed at
            // which the published power meets the road and air resistance used below
            let pub_kw = s.published.as_ref().map_or(0.0, |p| p.power_kw);
            let v_top = match veh.top_speed_kmh {
                Some(k) => k / 3.6,
                None if pub_kw > 0.0 => {
                    let (mut lo, mut hi) = (1.0, 200.0);
                    for _ in 0..60 {
                        let v = 0.5 * (lo + hi);
                        if (0.5 * 1.2 * cda * v * v + mass * 9.81 * 0.012) * v > pub_kw * 1000.0 {
                            hi = v;
                        } else {
                            lo = v;
                        }
                    }
                    0.5 * (lo + hi)
                }
                None => return 3.7 * (s.redline / 7000.0),
            };
            w_red * wheel_r / (top_gear * v_top)
        });
        // the clutch follows the engine's published torque instead of the generic 105 N m per
        // litre, so a small engine is not dragged down to road speed as it pulls away
        let litres = s.disp() * 1000.0;
        let t_ref = s.published.as_ref().map_or(0.0, |p| p.torque_nm);
        let t_ref = if t_ref > 0.0 { t_ref } else { 105.0 * litres };
        // driven wheels: static share of the weight on them, and weight transfer under acceleration
        let mu = tyre_mu(veh);
        let (share, grip_hl, mu_eff) = match (kind, veh.driven.as_deref()) {
            ("motorcycle", _) => (1.0, 0.0, mu.min(WHEELIE_G)),
            (_, Some("all")) => (1.0, 0.0, mu),
            (_, Some("rear")) | ("race car", None) => (DRIVEN_SHARE_CAR, CG_HL, mu),
            (_, Some("front")) => (FRONT_SHARE_FWD, -CG_HL, mu),
            _ => (DRIVEN_SHARE_CAR, 0.0, mu),
        };
        let gear = gear.min(n - 1);
        let v = omega * wheel_r / (gears[gear] * final_drive);
        Drive {
            mass,
            wheel_r,
            final_drive,
            gears,
            shift_rpm: 0.96 * s.redline,
            shift_time: 0.18,
            gear,
            v,
            shifting: 0.0,
            cda,
            auto: false,
            down_rpm: 0.0,
            brake: 0.0,
            locked: true,
            clutch_nm_per_l: 130.0 * t_ref / (105.0 * litres),
            clutch_gain: t_ref / (LAUNCH_SLIP * w_red),
            clutch_launch: launch_rpm(s) * PI / 30.0,
            eta: if kind == "motorcycle" { DRIVELINE_BIKE } else { DRIVELINE_CAR },
            grip: mu_eff * mass * 9.81 * share,
            t_avg: 0.0,
            grip_hl,
            mu: mu_eff,
            tc: 1.0,
        }
    }

    /// Engine speed below which automatic shifting changes down a gear, rpm.
    pub fn auto_down_rpm(s: &EngineSpec) -> f64 {
        (0.28 * s.redline).max(1.6 * s.idle)
    }
}

pub enum Mode {
    Free,
    Dyno,
    Drive(Drive),
}

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub work: f64,
    pub fric_work: f64,
    pub angle: f64,
    pub air: f64,
    pub p_peak: f64,
    pub t_coll: f64,
    pub p_man: f64,
    pub p_coll: f64,
    pub p_last: f64,
    /// two-stroke bookkeeping: mass into the crankcase, fresh mass delivered to the cylinder,
    /// fresh mass lost out of the exhaust port (net of what the pipe pushed back)
    pub cc_in: f64,
    pub delivered: f64,
    pub spilled: f64,
    pub n: f64,
    /// work taken from the crank to drive a supercharger, J
    pub drive_work: f64,
}

pub struct System {
    pub spec: EngineSpec,
    pub pipes: Vec<Pipe>,
    pub vols: Vec<Volume>,
    pub orifs: Vec<Orifice>,
    pub cyls: Vec<Cylinder>,
    pub sources: Vec<Source>,
    pub theta: f64,
    pub omega: f64,
    pub t: f64,
    pub dt: f64,
    pub mode: Mode,
    pub throttle_cmd: f64,
    pub throttle_eff: f64,
    pub torque: f64,
    pub limiter: bool,
    pub fuel_cut: bool,
    idle_integ: f64,
    /// seconds spent off throttle, firing, above the idle band and not slowing down (see the idle loop)
    idle_hang: f64,
    pub idle_mode: bool,
    idle_spark: f64,
    rpm_prev: f64,
    rpm_rate: f64,
    rng: Rng,
    cfl: f64,
    valve_noise: f64,
    vn_state: f64,
    block: [Svf; 3],
    pub stats: Stats,
    pub plenum: Option<usize>,
    pub collectors: Vec<usize>,
    pub max_sub: usize,
    /// cam-switching engines: true above the switch speed (with hysteresis)
    pub cam_high: bool,
    /// profiling: nanoseconds in [pipes, orifices, cylinders, volumes+crank, mics]
    pub prof_on: bool,
    pub prof: [u64; 5],
    /// fuel-air equivalence ratio (1.0 = stoichiometric)
    pub phi: f64,
    /// share of overrun cycles that are fueled instead of cut, 0..1
    pub overrun_fuel: f64,
    pub limiter_kind: Limiter,
    /// lift-off pops, 0..1: how much of the injected fuel wets the port walls (0 = none, the
    /// original model)
    pub liftoff: f64,
    /// test hook: one row per film-fed cycle [t, cycle phi, air / full charge, residual share, 1 if it lit]
    pub lift_log: Option<Vec<[f64; 5]>>,
    /// one per exhaust group (collector); empty for engines with an open pipe per cylinder
    pub af: Vec<AfGroup>,
    af_rng: Rng,
    /// one cylinder's full-load charge of air, kg
    af_charge: f64,
    /// Values that depend only on the step or on a slowly changing input, kept with the input they
    /// were computed from and recomputed when it changes (so results are the same as computing
    /// them every step): turbulence filter (dt, coefficient, normaliser), block-mode filter gains
    /// (sample rate, gains), throttle shaping (command, command^1.6).
    vn_cache: (f64, f64, f64),
    /// experiment `+m`: cheaper cube root and Woschni powers (see `Quality::fast_math`)
    pub fast_math: bool,
    block_cache: (f64, [f64; 3]),
    thr_cache: (f64, f64),
    /// forced induction: blowers driven from the crank (empty for naturally aspirated engines)
    pub machines: Vec<Blower>,
    /// turbochargers (empty for every other engine)
    pub turbos: Vec<Turbo>,
    /// knock-limited spark retard, degrees per bar of pressure at intake closing above `knock_onset` bar (0 = none)
    pub knock_k: f64,
    pub knock_onset: f64,
    /// spark retard from a vehicle's traction control on a boosted engine, degrees (0 otherwise):
    /// on boosted engines the trim retards the spark instead of closing the throttle
    pub tc_retard: f64,
}

pub struct Quality {
    pub rate: f64,
    pub dx_ex: f64,
    pub dx_in: f64,
    /// 0 = fixed cell sizes above. Otherwise each kind of pipe gets cells sized so that its
    /// highest expected wave speed gives this CFL number, so no pipe carries finer cells than
    /// its gas needs. A pipe that still exceeds the limit takes smaller steps on its own.
    pub matched_cfl: f64,
    /// reconstruction order in the pipes: 0 second-order, 1 third-order
    pub recon: u8,
    /// solve valveless pipes (snorkel, secondaries, outlets) along characteristics
    pub wave: bool,
    /// also solve intake runners that way, each ending in a small port volume at its valve
    pub wave_runners: bool,
    /// experiment: grid pipes reconstruct and compute fluxes in f32 (`+f`)
    pub f32: bool,
    /// experiment: cheaper cube root and Woschni powers in the cylinders (`+m`)
    pub fast_math: bool,
    /// experiment: open-end radiation solve starts from the last solve's slope (`+r`)
    pub rad_fast: bool,
}

impl Quality {
    pub fn dx(&self, kind: &str) -> f64 {
        if self.matched_cfl <= 0.0 {
            return if kind == "runner" || kind == "snorkel" { self.dx_in } else { self.dx_ex };
        }
        let v = match kind {
            // highest wave speeds measured across the presets at full load (m/s)
            "primary" => 1250.0,
            "secondary" => 950.0,
            "outlet" => 1000.0,
            "runner" => 680.0,
            _ => 420.0,
        };
        v / self.rate / self.matched_cfl
    }
}

impl System {
    pub fn build(spec: &EngineSpec, q: &Quality, seed: u64) -> System {
        let n = spec.n();
        let mut pipes: Vec<Pipe> = Vec::new();
        let mut vols: Vec<Volume> = Vec::new();
        let mut orifs: Vec<Orifice> = Vec::new();
        let mut sources: Vec<Source> = Vec::new();
        let src = |kind, stem, pan, dist| Source { kind, stem, pan, dist, jet: false, q_prev: 0.0, ic1: 0.0, ic2: 0.0, hist: vec![0.0; SOUND_SAMPLES], hist_pos: 0, hist_acc: 0.0, hist_n: 0 };
        let sec = |len: f64, d0: f64, d1: f64, fric: f64| Sec { len, d0, d1, fric };
        let fi = 0.02;
        let fe = 0.03;

        // ---- intake
        let nu = spec.units(); // cylinders or rotors: one runner and one primary each
        let mut in_pipe = vec![0usize; nu];
        // With characteristic runners the valve end is a small port volume (the last PORT_LEN of
        // the runner) that the cylinder exchanges gas with.
        let mut in_port: Vec<Option<usize>> = vec![None; nu];
        // (not with individual throttle bodies: a runner between two tiny volumes at part throttle
        // is outside what the characteristic model handles; those engines keep grid runners)
        let wave_runners = q.wave_runners && !spec.itb;
        let runner_port_vol = PI / 4.0 * spec.runner_d * spec.runner_d * PORT_LEN;
        let runner_lens: Vec<f64> = (0..nu).map(|i| spec.runner_len_of(i)).collect();
        let runner_end = move |vols: &mut Vec<Volume>, in_port: &mut Vec<Option<usize>>, i: usize| -> (f64, End) {
            if wave_runners {
                vols.push(Volume::new(&format!("port{}", i + 1), runner_port_vol, T_ATM));
                in_port[i] = Some(vols.len() - 1);
                ((runner_lens[i] - PORT_LEN).max(0.02), End::Volume(vols.len() - 1))
            } else {
                (runner_lens[i], End::Closed)
            }
        };
        let mut plenum = None;
        let mut machines: Vec<Blower> = Vec::new();
        // turbo intake side, one entry per turbo, kept until the exhaust exists: (compressor inlet,
        // outlet, throttle inlet, blow-off target, intercooler from/to)
        let mut turbo_in: Vec<(usize, usize, usize, Option<(usize, usize, u8)>, Option<(usize, usize)>)> = Vec::new();
        if let Some((b, t)) = spec.boost.as_ref().filter(|b| b.kind == "turbo" && !spec.itb).and_then(|b| b.turbo.as_ref().map(|t| (b, t))) {
            // Turbocharged: snorkel -> airbox =compressor=> compressor outlet -> charge pipe
            //   [-> intercooler inlet -cooler-> intercooler -> charge pipe] -> throttle inlet -throttle-> plenum
            // with the blow-off valve on the throttle inlet (vented through a short pipe, or back to the airbox).
            // Parallel turbos (`count` 2): each has its own snorkel, airbox, compressor, charge pipes,
            // intercooler and blow-off valve; their charge pipes meet in the one throttle inlet.
            // (The second turbo's parts carry the tag "_t2"; a single turbo's are built exactly as before.)
            let nt = t.count.max(1);
            let tag = |k: usize| if k == 0 { String::new() } else { format!("_t{}", k + 1) };
            let mut sides = Vec::new(); // (airbox, compressor outlet, last volume before the second charge pipe, intercooler)
            for k in 0..nt {
                let tg = tag(k);
                let airbox = vols.len();
                vols.push(Volume::new(&format!("airbox{tg}"), spec.airbox_vol, T_ATM));
                pipes.push(Pipe::new(&format!("snorkel{tg}"), "snorkel", &[sec(spec.snorkel_len, spec.snorkel_d, spec.snorkel_d, fi)], q.dx("snorkel"), End::Ambient, End::Volume(airbox), T_ATM, T_ATM));
                sources.push(src(SrcKind::PipeLeft(pipes.len() - 1), 1, if nt == 1 { 0.5 } else { 0.3 + 0.4 * k as f64 / (nt - 1) as f64 }, 1.0));
                let cd = vols.len();
                vols.push(Volume::new(&format!("comp_out{tg}"), b.outlet_vol, T_ATM));
                let mut up = cd;
                let mut ic = None;
                let cp0 = t.charge_pipes[0];
                if let Some(c) = &b.intercooler {
                    let ii = vols.len();
                    vols.push(Volume::new(&format!("ic_in{tg}"), 0.4 * c.vol, T_ATM));
                    pipes.push(Pipe::new(&format!("charge1{tg}"), "charge", &[sec(cp0[0], cp0[1], cp0[1], fi)], q.dx("runner"), End::Volume(up), End::Volume(ii), T_ATM, 330.0));
                    let io = vols.len();
                    vols.push(Volume::new(&format!("intercooler{tg}"), 0.6 * c.vol, T_ATM));
                    ic = Some((ii, io));
                    up = io;
                }
                sides.push((airbox, cd, up, ic));
            }
            let cp1 = *t.charge_pipes.get(if b.intercooler.is_some() { 1 } else { 0 }).unwrap_or(&t.charge_pipes[0]);
            let ti = vols.len();
            vols.push(Volume::new("throttle_in", 1.5e-4, T_ATM));
            for (k, s) in sides.iter().enumerate() {
                let name = if b.intercooler.is_some() { "charge2" } else { "charge1" };
                pipes.push(Pipe::new(&format!("{name}{}", tag(k)), "charge", &[sec(cp1[0], cp1[1], cp1[1], fi)], q.dx("runner"), End::Volume(s.2), End::Volume(ti), T_ATM, 310.0));
            }
            let pl = vols.len();
            vols.push(Volume::new("plenum", spec.plenum_vol, T_ATM));
            plenum = Some(pl);
            orifs.push(Orifice { a: VRef::Vol(ti), b: VRef::Vol(pl), area: PI / 4.0 * spec.throttle_d * spec.throttle_d, throttle: true, mdot: 0.0 });
            // The vent pipe is always built, so the valve's routing can change while running
            // (`set_blowoff`); with recirculation or no valve it carries no flow.
            for (k, s) in sides.iter().enumerate() {
                let tg = tag(k);
                let bv = vols.len();
                vols.push(Volume::new(&format!("bov{tg}"), 2.0e-5, T_ATM));
                pipes.push(Pipe::new(&format!("bov_vent{tg}"), "bov", &[sec(0.06, t.bov_d, t.bov_d, fi)], q.dx("snorkel"), End::Volume(bv), End::Ambient, T_ATM, T_ATM));
                let mut sr = src(SrcKind::PipeRight(pipes.len() - 1), 1, if nt == 1 { 0.5 } else { 0.3 + 0.4 * k as f64 / (nt - 1) as f64 }, 1.0);
                sr.jet = true;
                sources.push(sr);
                let bov_to = Some((bv, s.0, match t.bov.as_str() {
                    "vent" => 2u8,
                    "recirc" => 1,
                    _ => 0,
                }));
                turbo_in.push((s.0, s.1, ti, bov_to, s.3));
            }
            for i in 0..nu {
                let (rl, rend) = runner_end(&mut vols, &mut in_port, i);
                let mut p = Pipe::new(&format!("runner{}", i + 1), "runner", &[sec(rl, spec.runner_d, spec.runner_d, fi)], q.dx("runner"), End::Volume(pl), rend, T_ATM, 310.0);
                p.cyl = i as i32;
                in_pipe[i] = pipes.len();
                pipes.push(p);
            }
        } else if let Some(b) = spec.boost.as_ref().filter(|_| !spec.itb) {
            // Supercharged plenum engine. Throttle before the blower:
            //   snorkel -> airbox -throttle-> blower inlet =rotors=> plenum
            //   (with an intercooler: =rotors=> blower outlet -cooler-> plenum)
            // throttle after it:
            //   snorkel -> airbox =rotors=> blower outlet [-cooler-> intercooler] [-> charge pipe -> throttle inlet] -throttle-> plenum
            // The bypass valve returns gas from the blower outlet to its inlet while the manifold is under vacuum.
            let airbox = vols.len();
            vols.push(Volume::new("airbox", spec.airbox_vol, T_ATM));
            pipes.push(Pipe::new("snorkel", "snorkel", &[sec(spec.snorkel_len, spec.snorkel_d, spec.snorkel_d, fi)], q.dx("snorkel"), End::Ambient, End::Volume(airbox), T_ATM, T_ATM));
            sources.push(src(SrcKind::PipeLeft(pipes.len() - 1), 1, 0.5, 1.0));
            let a_thr = PI / 4.0 * spec.throttle_d * spec.throttle_d;
            let ic_spec = b.intercooler.as_ref();
            let ic_vol = ic_spec.map_or(0.0, |c| c.vol);
            let (u, d, pl, ic) = if b.throttle == "before" {
                let u = vols.len();
                vols.push(Volume::new("blower_in", b.inlet_vol, T_ATM));
                orifs.push(Orifice { a: VRef::Vol(airbox), b: VRef::Vol(u), area: a_thr, throttle: true, mdot: 0.0 });
                if ic_spec.is_some() {
                    // the cooler's own volume is counted with the plenum it opens into
                    let d = vols.len();
                    vols.push(Volume::new("blower_out", b.outlet_vol, T_ATM));
                    let pl = vols.len();
                    vols.push(Volume::new("plenum", spec.plenum_vol + ic_vol, T_ATM));
                    (u, d, pl, Some((d, pl)))
                } else {
                    let pl = vols.len();
                    vols.push(Volume::new("plenum", spec.plenum_vol, T_ATM));
                    (u, pl, pl, None)
                }
            } else {
                let d = vols.len();
                vols.push(Volume::new("blower_out", b.outlet_vol, T_ATM));
                let mut up = d;
                let mut ic = None;
                if ic_spec.is_some() {
                    up = vols.len();
                    vols.push(Volume::new("intercooler", ic_vol, T_ATM));
                    ic = Some((d, up));
                }
                if b.charge_len > 0.0 && b.charge_d > 0.0 {
                    let tv = vols.len();
                    vols.push(Volume::new("throttle_in", PI / 4.0 * b.charge_d * b.charge_d * PORT_LEN, T_ATM));
                    pipes.push(Pipe::new("charge", "charge", &[sec(b.charge_len, b.charge_d, b.charge_d, fi)], q.dx("runner"), End::Volume(up), End::Volume(tv), T_ATM, 320.0));
                    up = tv;
                }
                let pl = vols.len();
                vols.push(Volume::new("plenum", spec.plenum_vol, T_ATM));
                orifs.push(Orifice { a: VRef::Vol(up), b: VRef::Vol(pl), area: a_thr, throttle: true, mdot: 0.0 });
                (airbox, d, pl, ic)
            };
            plenum = Some(pl);
            let mut bl = Blower::new(
                u,
                d,
                &BlowerGeom {
                    kind_screw: b.kind == "twin_screw",
                    displacement: b.displacement,
                    ratio: b.drive_ratio,
                    rotors: b.rotors,
                    lobes: b.lobes,
                    helix_deg: b.helix,
                    vi: b.vi,
                    carry: b.carry,
                    a_port: b.port_area,
                    a_leak: b.leak_area,
                    mech_eff: b.mech_eff,
                    drag: b.drag,
                },
            );
            if let Some(bp) = &b.bypass {
                bl.bypass = Some(Bypass { area: PI / 4.0 * bp.d * bp.d, p_open: P_ATM + 1e3 * bp.open_kpa, p_shut: P_ATM + 1e3 * bp.shut_kpa, tau: bp.tau, sense: pl, x: 1.0, mdot: 0.0 });
            }
            if let (Some((from, to)), Some(c)) = (ic, ic_spec) {
                bl.ic = Some(Intercooler { from, to, area: PI / 4.0 * c.d * c.d, eff: c.effectiveness, t_cool: c.t_cool, mdot: 0.0, q_out: 0.0 });
            }
            machines.push(bl);
            for i in 0..nu {
                let (rl, rend) = runner_end(&mut vols, &mut in_port, i);
                let mut p = Pipe::new(&format!("runner{}", i + 1), "runner", &[sec(rl, spec.runner_d, spec.runner_d, fi)], q.dx("runner"), End::Volume(pl), rend, T_ATM, 310.0);
                p.cyl = i as i32;
                in_pipe[i] = pipes.len();
                pipes.push(p);
            }
        } else if spec.itb {
            let a_run = PI / 4.0 * spec.runner_d * spec.runner_d;
            for i in 0..nu {
                let vi = vols.len();
                vols.push(Volume::new(&format!("tb{}", i + 1), 0.25 * spec.disp_cyl(), T_ATM));
                orifs.push(Orifice { a: VRef::Ambient, b: VRef::Vol(vi), area: a_run, throttle: true, mdot: 0.0 });
                sources.push(src(SrcKind::Orifice(orifs.len() - 1), 1, 0.5, 1.0));
                let (rl, rend) = runner_end(&mut vols, &mut in_port, i);
                let mut p = Pipe::new(&format!("runner{}", i + 1), "runner", &[sec(rl, spec.runner_d * 1.15, spec.runner_d, fi)], q.dx("runner"), End::Volume(vi), rend, T_ATM, 310.0);
                p.cyl = i as i32;
                in_pipe[i] = pipes.len();
                pipes.push(p);
            }
        } else {
            let airbox = vols.len();
            vols.push(Volume::new("airbox", spec.airbox_vol, T_ATM));
            let pl = vols.len();
            vols.push(Volume::new("plenum", spec.plenum_vol, T_ATM));
            plenum = Some(pl);
            pipes.push(Pipe::new("snorkel", "snorkel", &[sec(spec.snorkel_len, spec.snorkel_d, spec.snorkel_d, fi)], q.dx("snorkel"), End::Ambient, End::Volume(airbox), T_ATM, T_ATM));
            sources.push(src(SrcKind::PipeLeft(pipes.len() - 1), 1, 0.5, 1.0));
            orifs.push(Orifice { a: VRef::Vol(airbox), b: VRef::Vol(pl), area: PI / 4.0 * spec.throttle_d * spec.throttle_d, throttle: true, mdot: 0.0 });
            for i in 0..nu {
                let (rl, rend) = runner_end(&mut vols, &mut in_port, i);
                let mut p = Pipe::new(&format!("runner{}", i + 1), "runner", &[sec(rl, spec.runner_d, spec.runner_d, fi)], q.dx("runner"), End::Volume(pl), rend, T_ATM, 310.0);
                p.cyl = i as i32;
                in_pipe[i] = pipes.len();
                pipes.push(p);
            }
        }

        // ---- exhaust
        let mut ex_pipe = vec![0usize; nu];
        let mut collectors = Vec::new();
        let chambered = !spec.chamber.is_empty();
        if chambered {
            // each cylinder has its own tuned pipe, open to the air at its end
            for i in 0..nu {
                let secs: Vec<Sec> = spec.chamber.iter().map(|c| sec(c[0], c[1], c[2], fe)).collect();
                let mut p = Pipe::new(&format!("primary{}", i + 1), "primary", &secs, q.dx("primary"), End::Closed, End::Ambient, 700.0, 680.0);
                p.cyl = i as i32;
                p.group = i as i32;
                ex_pipe[i] = pipes.len();
                pipes.push(p);
                let pan = if nu == 1 { 0.5 } else { 0.15 + 0.7 * i as f64 / (nu - 1) as f64 };
                sources.push(src(SrcKind::PipeRight(pipes.len() - 1), 0, pan, 1.0));
            }
            vols.push(Volume::new("collector1", 1.0e-3, 600.0)); // unused; keeps the statistics code simple
            collectors.push(vols.len() - 1);
        }
        for (g, grp) in spec.groups.iter().enumerate().filter(|_| !chambered) {
            let cv = vols.len();
            vols.push(Volume::new(&format!("collector{}", g + 1), spec.collector_vol * (grp.len() as f64).max(1.0) / 2.0, 800.0));
            collectors.push(cv);
            for &i in grp {
                let mut p = Pipe::new(&format!("primary{}", i + 1), "primary", &[sec(spec.primary_len[i], spec.primary_d, spec.primary_d, fe)], q.dx("primary"), End::Closed, End::Volume(cv), 800.0, 800.0);
                p.cyl = i as i32;
                p.group = g as i32;
                ex_pipe[i] = pipes.len();
                pipes.push(p);
            }
        }
        let outlet_secs = |first_len: f64, first_d: f64| {
            let mut s = vec![sec(first_len, first_d, first_d, fe)];
            if spec.muffler_len > 0.0 {
                s.push(sec(spec.muffler_len, spec.muffler_d, spec.muffler_d, spec.muffler_fric));
            }
            let flare = if spec.tail_flare_d > spec.tail_d { spec.tail_flare_d } else { spec.tail_d };
            s.push(sec(spec.tail_len, spec.tail_d, flare, fe));
            s
        };
        let ng = spec.groups.len();
        let mut turbos: Vec<Turbo> = Vec::new();
        let turbo_spec = spec.boost.as_ref().and_then(|b| b.turbo.as_ref());
        if let (Some(t), false) = (turbo_spec, turbo_in.is_empty()) {
            // Each exhaust group's collector feeds the turbine: one scroll per group ("twin", or a
            // single group), or all groups merged through their secondaries into one turbine inlet.
            // The volute's volume is added to the turbine inlet; the turbine outlet feeds the downpipe.
            // Parallel turbos take equal shares of the groups in order (one bank each on a V8 with
            // one group per bank), each with its own downpipe, muffler and tailpipe.
            let nt = turbo_in.len();
            let gs = ng / nt;
            for (k, &(tu, td, ti, bov_to, ic)) in turbo_in.iter().enumerate() {
                let tg = if k == 0 { String::new() } else { format!("_t{}", k + 1) };
                let grs: Vec<usize> = (k * gs..(k + 1) * gs).collect();
                let mut xs = Vec::new();
                if t.scroll == "twin" || gs == 1 {
                    for &g in &grs {
                        let cv = collectors[g];
                        let v = vols[cv].v + t.volute_vol / gs as f64;
                        let name = vols[cv].name.clone();
                        vols[cv] = Volume::new(&name, v, 800.0);
                        xs.push(cv);
                    }
                } else {
                    let mv = vols.len();
                    vols.push(Volume::new(&format!("turbine_in{tg}"), spec.collector_vol + t.volute_vol, 800.0));
                    for &g in &grs {
                        let mut p = Pipe::new(&format!("secondary{}", g + 1), "secondary", &[sec(spec.secondary_len, spec.secondary_d, spec.secondary_d, fe)], q.dx("secondary"), End::Volume(collectors[g]), End::Volume(mv), 800.0, 750.0);
                        p.group = g as i32;
                        pipes.push(p);
                    }
                    xs.push(mv);
                }
                let y = vols.len();
                vols.push(Volume::new(&format!("turbine_out{tg}"), t.turbine_out_vol, 700.0));
                let mut p = Pipe::new(&format!("outlet{}", k + 1), "outlet", &outlet_secs(spec.mid_len, spec.mid_d), q.dx("outlet"), End::Volume(y), End::Ambient, 600.0, 560.0);
                p.group = k as i32;
                pipes.push(p);
                sources.push(src(SrcKind::PipeRight(pipes.len() - 1), 0, if nt == 1 { 0.5 } else { 0.15 + 0.7 * k as f64 / (nt - 1) as f64 }, 1.0));
                let scrolls: Vec<Scroll> = xs.iter().map(|&x| Scroll { x, area: t.turbine_area / xs.len() as f64, mdot: 0.0 }).collect();
                // each turbo's whistle noise has its own random stream (the first keeps the engine's)
                let mut tb = Turbo::new(tu, td, scrolls, y, t, seed ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
                tb.f_max = (0.45 * q.rate).min(20_000.0);
                let ctrl = if t.boost_control { spec.boost.as_ref().map(|b| BoostCtrl { target: 1e5 * b.boost_bar, kp: 0.15, ki: 0.6, integ: 0.0, duty: 0.0, max_duty: t.bc_max_duty, ff: (1.0 - t.wg_crack_bar / b.boost_bar).clamp(0.0, t.bc_max_duty) }) } else { None };
                tb.wg = Some(Wastegate { area: PI / 4.0 * t.wg_d * t.wg_d, p_crack: 1e5 * t.wg_crack_bar, p_span: 1e5 * t.wg_span_bar, tau: t.wg_tau, sense: td, x: 0.0, mdot: 0.0, ctrl, plenum: plenum.unwrap() });
                if let Some((vent, recirc, mode)) = bov_to {
                    tb.bov = Some(Bov { from: ti, vent, recirc, mode, area: PI / 4.0 * t.bov_d * t.bov_d, dp_open: 1e3 * t.bov_open_kpa, dp_full: 1e3 * t.bov_full_kpa, tau: t.bov_tau, plenum: plenum.unwrap(), x: 0.0, mdot: 0.0 });
                }
                if let (Some((from, to)), Some(c)) = (ic, spec.boost.as_ref().and_then(|b| b.intercooler.as_ref())) {
                    tb.ic = Some(Intercooler { from, to, area: PI / 4.0 * c.d * c.d, eff: c.effectiveness, t_cool: c.t_cool, mdot: 0.0, q_out: 0.0 });
                }
                turbos.push(tb);
            }
        } else if chambered {
            // no shared exhaust
        } else if spec.merge == "none" {
            for g in 0..ng {
                let mut p = Pipe::new(&format!("outlet{}", g + 1), "outlet", &outlet_secs(spec.secondary_len, spec.secondary_d), q.dx("outlet"), End::Volume(collectors[g]), End::Ambient, 600.0, 560.0);
                p.group = g as i32;
                pipes.push(p);
                let pan = if ng == 1 { 0.5 } else { 0.15 + 0.7 * g as f64 / (ng - 1) as f64 };
                sources.push(src(SrcKind::PipeRight(pipes.len() - 1), 0, pan, 1.0));
            }
        } else {
            let mv = vols.len();
            vols.push(Volume::new("merge", spec.collector_vol * 2.0, 700.0));
            for g in 0..ng {
                let mut p = Pipe::new(&format!("secondary{}", g + 1), "secondary", &[sec(spec.secondary_len, spec.secondary_d, spec.secondary_d, fe)], q.dx("secondary"), End::Volume(collectors[g]), End::Volume(mv), 700.0, 680.0);
                p.group = g as i32;
                pipes.push(p);
            }
            let nout = if spec.merge == "x" { 2 } else { 1 };
            for o in 0..nout {
                let mut p = Pipe::new(&format!("outlet{}", o + 1), "outlet", &outlet_secs(spec.mid_len, spec.mid_d), q.dx("outlet"), End::Volume(mv), End::Ambient, 600.0, 560.0);
                p.group = o as i32;
                pipes.push(p);
                let pan = if nout == 1 { 0.5 } else { 0.15 + 0.7 * o as f64 };
                sources.push(src(SrcKind::PipeRight(pipes.len() - 1), 0, pan, 1.0));
            }
        }

        // ---- cylinders
        let wankel = spec.is_wankel();
        let two = spec.is_two_stroke();
        let a_p = PI / 4.0 * spec.bore * spec.bore;
        let v_d = spec.disp_cyl();
        let v_c = v_d / (spec.cr - 1.0);
        let rad = PI / 180.0;
        let cyc = spec.cycle_deg() * rad;
        let cpu = spec.chambers_per_unit();
        // valve events for the low (or only) cam and the high cam, as cycle angles
        let ev_of = |high: bool| -> [f64; 6] {
            let e = spec.events(high);
            [2.0 * PI - e.ivo * rad, 3.0 * PI + e.ivc * rad, PI - e.evo * rad, 2.0 * PI + e.evc * rad, e.in_lift, e.ex_lift]
        };
        let ev_sets = [ev_of(false), ev_of(true)];
        let mut cyls = Vec::new();
        for i in 0..n {
            let mut cy = Cylinder {
                phase: spec.firing[i] * rad,
                a_p,
                crank_r: 0.5 * spec.stroke,
                rod: spec.rod,
                v_c,
                v_d,
                bore: spec.bore,
                m: 0.0,
                e: 0.0,
                v: 0.0,
                c: 0.0,
                m_fresh: 0.0,
                reverted: 0.0,
                in_pipe: in_pipe[i / cpu],
                in_port: in_port[i / cpu],
                ex_pipe: ex_pipe[i / cpu],
                ivo: if wankel { spec.port_in[0] * rad } else { ev_sets[0][0] },
                ivc: if wankel { spec.port_in[1] * rad } else { ev_sets[0][1] },
                evo: if wankel { spec.port_ex[0] * rad } else { ev_sets[0][2] },
                evc: if wankel { spec.port_ex[1] * rad } else { ev_sets[0][3] },
                in_lift_max: ev_sets[0][4],
                ex_lift_max: ev_sets[0][5],
                in_n: spec.in_valves as f64,
                ex_n: spec.ex_valves as f64,
                ev: ev_sets,
                cam_high: false,
                in_d: spec.in_valve_d,
                ex_d: spec.ex_valve_d,
                planned: false,
                burning: false,
                q_tot: 0.0,
                burn_dur: 1.0,
                burn_ang: 0.0,
                spark: cyc - 0.3,
                xb_prev: 0.0,
                p_ivc: P_ATM,
                v_ivc: v_d,
                t_ivc: T_ATM,
                lift_in: 0.0,
                lift_ex: 0.0,
                hrr: 0.0,
                p_prev: P_ATM,
                air_last: 0.0,
                p_peak: 0.0,
                q_dbg: 0.0,
                cyc,
                rotor_r: if wankel { spec.rotor_r } else { 0.0 },
                rotor_w: spec.rotor_w,
                in_area: spec.port_in_area,
                ex_area: spec.port_ex_area,
                burn_scale: spec.burn_scale,
                two,
                cc_m: 0.0,
                cc_e: 0.0,
                cc_v: 0.0,
                cc_v_tdc: if two { v_d * spec.crankcase_cr / (spec.crankcase_cr - 1.0) } else { 0.0 },
                cc_burned: 0.0,
                spilled: 0.0,
                tpo: spec.tr_port_open * rad,
                tr_area: spec.tr_port_area,
                induction: match spec.induction.as_str() {
                    "piston" => 1,
                    "disc" => 2,
                    _ => 0,
                },
                in_btdc: spec.in_window[0] * rad,
                in_atdc: spec.in_window[1] * rad,
                fuel_left: 0.0,
                air_left: 0.0,
                fired: false,
                late: false,
                af_group: if chambered { None } else { Some(pipes[ex_pipe[i / cpu]].group as usize) },
                film: 0.0,
                pooled: false,
                clen_pow: 0.0,
            };
            cy.clen_pow = cy.wall_len().powf(-0.2);
            if two {
                // exhaust port: `evo` after TDC to the same before it; the burn is planned when it closes
                cy.evo = spec.ex_port_open * rad;
                cy.evc = cyc - cy.evo;
                cy.ivc = cy.evc;
                cy.ivo = cy.tpo;
            }
            cy.c = (-cy.phase).rem_euclid(cyc);
            cy.v = cy.volume(cy.c);
            cy.m = RHO_ATM * cy.v;
            cy.e = cy.m * CV * T_ATM;
            cy.m_fresh = cy.m;
            if two {
                cy.cc_v = cy.cc_v_tdc - cy.a_p * cy.piston_x(cy.c);
                cy.cc_m = RHO_ATM * cy.cc_v;
                cy.cc_e = cy.cc_m * CV * T_ATM;
            }
            cyls.push(cy);
        }

        for p in pipes.iter_mut() {
            p.recon = q.recon;
            if p.kind == "outlet" {
                p.muffler_zeta = spec.muffler_zeta;
            }
            p.f32 = q.f32;
            p.rad_l.fast = q.rad_fast;
            p.rad_r.fast = q.rad_fast;
            let kinds = std::env::var("ENGSIM_WAVE_KINDS").unwrap_or_else(|_| "snorkel,secondary,outlet".into());
            if (q.wave && (p.kind == "charge" || kinds.split(',').any(|k| k == p.kind))) || (q.wave_runners && !spec.itb && p.kind == "runner") {
                p.make_wave();
            }
        }
        let mut af = if chambered { Vec::new() } else { vec![AfGroup { t_wall: 800.0, ..Default::default() }; collectors.len()] };
        for cy in &cyls {
            if let Some(g) = cy.af_group {
                af[g].hold += RHO_ATM * v_d;
            }
        }
        System {
            phi: 1.0,
            overrun_fuel: 0.0,
            limiter_kind: Limiter::FuelCut,
            liftoff: 0.0,
            lift_log: None,
            af,
            // its own random stream, so afterfire never shifts the draws of the rest of the model
            af_rng: Rng((seed ^ 0xA5F1_7E00_D15E_A5E5) | 1),
            af_charge: RHO_ATM * v_d,
            vn_cache: (f64::NAN, 0.0, 0.0),
            fast_math: q.fast_math,
            block_cache: (f64::NAN, [0.0; 3]),
            thr_cache: (f64::NAN, 0.0),
            machines,
            turbos,
            knock_k: turbo_spec.map_or(0.0, |t| t.knock_retard),
            knock_onset: turbo_spec.map_or(0.0, |t| t.knock_onset_bar),
            tc_retard: 0.0,
            spec: spec.clone(),
            pipes,
            vols,
            orifs,
            cyls,
            sources,
            theta: 0.0,
            omega: spec.idle * PI / 30.0,
            t: 0.0,
            dt: 1.0 / q.rate,
            mode: Mode::Free,
            throttle_cmd: 0.0,
            throttle_eff: 0.0,
            torque: 0.0,
            limiter: false,
            fuel_cut: false,
            idle_integ: 0.008,
            idle_hang: 0.0,
            idle_mode: false,
            idle_spark: 1.0,
            rpm_prev: spec.idle,
            rpm_rate: 0.0,
            rng: Rng(seed | 1),
            cfl: 0.5,
            valve_noise: 0.10,
            vn_state: 0.0,
            block: [Svf::default(); 3],
            stats: Stats::default(),
            plenum,
            collectors,
            max_sub: 1,
            cam_high: false,
            prof_on: false,
            prof: [0; 5],
        }
    }

    pub fn rpm(&self) -> f64 {
        self.omega * 30.0 / PI
    }

    /// Throttle opening from the pedal command, cmd^1.6 (cached: the pedal seldom moves).
    #[inline]
    fn throttle_shape(&mut self, cmd: f64) -> f64 {
        if cmd.to_bits() != self.thr_cache.0.to_bits() {
            self.thr_cache = (cmd, cmd.powf(1.6));
        }
        self.thr_cache.1
    }

    /// True when the fueling settings are the defaults (and the model is the original one).
    pub fn fueling_default(&self) -> bool {
        self.phi == 1.0 && self.overrun_fuel == 0.0 && self.limiter_kind == Limiter::FuelCut && self.liftoff == 0.0
    }

    /// Unburned fuel and spare air in each collector: carried on down the pipe, and lit by a
    /// flame or a hot collector when the mixture there is flammable. The heat goes into the
    /// collector volume; the pipes carry the pressure rise away as a wave.
    fn afterfire(&mut self, dt: f64) {
        // (nothing to do in the usual case: no unburned charge, no flame, no glow)
        if self.af.iter().all(|a| a.fuel == 0.0 && a.air == 0.0 && !a.releasing && a.glow == 0.0 && a.flame <= 0.0 && a.pool_fuel == 0.0 && a.pool_air == 0.0) {
            for g in 0..self.af.len() {
                let t = self.vols[self.collectors[g]].t();
                let a = &mut self.af[g];
                a.t_wall += (t - a.t_wall) * (dt / AF_WALL_T);
            }
            return;
        }
        let res_deg = if self.spec.is_two_stroke() { 0.5 * AF_RESIDENCE_DEG } else { AF_RESIDENCE_DEG };
        let tau = res_deg * PI / 180.0 / self.omega.max(1.0);
        let keep = (-dt / tau).exp();
        let keep_in = (-dt / (tau * AF_ARRIVAL_DEG / AF_RESIDENCE_DEG)).exp();
        let fuel_min = AF_FUEL_MIN * self.af_charge;
        let p_ign = AF_RATE * dt;
        // heat release rate shown as 1.0: a quarter of a full cylinder charge burning at its peak rate
        let rate_ref = 2.0 * self.spec.heat_q * AF_EFF * 0.25 * self.af_charge / AF_RELEASE;
        let glow_keep = (-dt / AF_GLOW_T).exp();
        for g in 0..self.af.len() {
            let cv = self.collectors[g];
            let a = &mut self.af[g];
            a.t_wall += (self.vols[cv].t() - a.t_wall) * (dt / AF_WALL_T);
            if a.fuel == 0.0 && a.air == 0.0 && !a.releasing && a.glow == 0.0 && a.flame <= 0.0 && a.pool_fuel == 0.0 && a.pool_air == 0.0 {
                continue;
            }
            a.fuel *= keep;
            a.air *= keep;
            a.in_fuel *= keep_in;
            a.in_air *= keep_in;
            if a.fuel < 1e-15 {
                a.fuel = 0.0;
                a.in_fuel = 0.0;
            }
            if a.air < 1e-15 {
                a.air = 0.0;
                a.in_air = 0.0;
            }
            if a.flame > 0.0 {
                a.flame -= dt;
            }
            a.rate = 0.0;
            if a.releasing {
                a.t_ev += dt;
                let x = a.t_ev / AF_RELEASE;
                let xb = if x >= 1.15 { 1.0 } else { 1.0 - (-5.0 * x * x * x).exp() };
                let dq = a.q_ev * (xb - a.xb_prev);
                a.xb_prev = xb;
                self.vols[cv].de += dq;
                a.rate = dq / dt;
                if x >= 1.15 {
                    a.releasing = false;
                }
            } else if a.fuel > fuel_min && a.air > 0.0 && a.flushed >= self.vols[cv].m {
                let flammable = |f: f64, x: f64| x > 0.0 && (AF_PHI_MIN * x..=AF_PHI_MAX * x).contains(&f);
                let lit = a.flame > 0.0 || a.t_wall > AF_T_IGNITE;
                if (flammable(a.fuel, a.air) || flammable(a.in_fuel, a.in_air)) && lit && self.af_rng.uniform() < p_ign {
                    // the flame takes the slug at the collector, not all 1.5 cycles of pipe: at most
                    // one cylinder's charge burns at once
                    let used = AF_EFF * a.fuel.min(a.air).min(self.af_charge);
                    a.fuel -= used;
                    a.air -= used;
                    a.in_fuel = (a.in_fuel - used).max(0.0);
                    a.in_air = (a.in_air - used).max(0.0);
                    // (the gathered lift-off charge is counted in the same air)
                    a.pool_air = (a.pool_air - used).max(0.0);
                    a.flushed = 0.0;
                    a.q_ev = self.spec.heat_q * used;
                    a.t_ev = 0.0;
                    a.xb_prev = 0.0;
                    a.releasing = true;
                    a.events += 1;
                    a.q_sum += a.q_ev;
                    a.q_max = a.q_max.max(a.q_ev);
                    a.last_t = self.t;
                }
            }
            // Lift-off pops: the charge gathered from film-fed cycles has no hot wall to light it
            // (it sits further down the pipe); a flame arriving lights all of it at once. What the
            // burn does not use is scattered by it.
            if a.pool_fuel > 0.0 || a.pool_air > 0.0 {
                let linger = 1.0 - dt / LIFT_LINGER;
                a.pool_fuel *= linger;
                a.pool_air *= linger;
                let b = a.pool_fuel.min(a.pool_air);
                if !a.releasing && b >= LIFT_GATHER * a.hold && a.pool_fuel >= LIFT_PHI_MIN * a.pool_air && a.flushed >= self.vols[cv].m && a.flame > 0.0 && self.af_rng.uniform() < p_ign {
                    a.pool_fuel -= b;
                    a.pool_air -= b;
                    a.air = (a.air - b).max(0.0);
                    a.in_air = (a.in_air - b).max(0.0);
                    a.flushed = 0.0;
                    a.q_ev = self.spec.heat_q * AF_EFF * b.min(LIFT_CAP * a.hold.min(4.0 * self.af_charge));
                    a.t_ev = 0.0;
                    a.xb_prev = 0.0;
                    a.releasing = true;
                    a.events += 1;
                    a.q_sum += a.q_ev;
                    a.q_max = a.q_max.max(a.q_ev);
                    a.last_t = self.t;
                }
                if a.pool_fuel < 1e-15 {
                    a.pool_fuel = 0.0;
                }
                if a.pool_air < 1e-15 {
                    a.pool_air = 0.0;
                }
            }
            a.glow = (a.rate / rate_ref).min(1.0).max(a.glow * glow_keep);
            if a.glow < 1e-4 {
                a.glow = 0.0;
            }
        }
    }

    fn vref_state(&self, r: VRef) -> (f64, f64, f64) {
        match r {
            VRef::Ambient => (P_ATM, T_ATM, f64::INFINITY),
            VRef::Vol(i) => (self.vols[i].p(), self.vols[i].t(), self.vols[i].v),
        }
    }

    fn substep(&mut self, dt: f64) -> f64 {
        let dth = self.omega * dt;
        let th1 = self.theta + dth;
        let rpm = self.rpm();

        let pt0 = if self.prof_on { Some(std::time::Instant::now()) } else { None };
        prof::count(cnt::SUBSTEPS, 1);
        let qt = prof::now();
        // --- pipes
        let mut cfl: f64 = 0.0;
        {
            let pipes = &mut self.pipes;
            let vols = &mut self.vols;
            for p in pipes.iter_mut() {
                let bl = match p.left {
                    End::Closed => Bc::Closed,
                    End::Volume(i) => Bc::Res { p0: vols[i].p(), rho0: vols[i].rho() },
                    End::Ambient => Bc::Open,
                };
                let br = match p.right {
                    End::Closed => Bc::Closed,
                    End::Volume(i) => Bc::Res { p0: vols[i].p(), rho0: vols[i].rho() },
                    End::Ambient => Bc::Open,
                };
                // a pipe whose waves outrun its grid this step takes smaller steps by itself
                let nsub_p = ((p.cfl_last * 1.05 * dt / p.dt_last.max(1e-12) / 0.92).ceil() as usize).clamp(1, 6);
                let dtp = dt / nsub_p as f64;
                let (mut ml, mut el, mut ql, mut mr, mut er, mut qr) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
                let mut smax: f64 = 0.0;
                for _ in 0..nsub_p {
                    smax = smax.max(p.step(dtp, bl, br));
                    ml += p.out_l.mdot;
                    el += p.out_l.edot;
                    ql += p.out_l.q;
                    mr += p.out_r.mdot;
                    er += p.out_r.edot;
                    qr += p.out_r.q;
                }
                let w = 1.0 / nsub_p as f64;
                p.out_l = EndFlux { mdot: ml * w, edot: el * w, q: ql * w };
                p.out_r = EndFlux { mdot: mr * w, edot: er * w, q: qr * w };
                p.cfl_last = smax * dtp / p.dx;
                p.dt_last = dtp;
                p.substeps += nsub_p as u64 - 1;
                cfl = cfl.max(p.cfl_last);
                if let End::Volume(i) = p.left {
                    vols[i].dm += p.out_l.mdot * dt;
                    vols[i].de += p.out_l.edot * dt;
                }
                if let End::Volume(i) = p.right {
                    vols[i].dm += p.out_r.mdot * dt;
                    vols[i].de += p.out_r.edot * dt;
                }
            }
        }

        let qt = prof::lap(cat::PIPE_OTHER, qt);
        let pt1 = pt0.map(|_| std::time::Instant::now());
        // --- orifices (throttles)
        for k in 0..self.orifs.len() {
            let (a, b, area, thr) = {
                let o = &self.orifs[k];
                (o.a, o.b, o.area, o.throttle)
            };
            let frac = if thr { self.throttle_eff } else { 1.0 };
            let (pa, ta, va) = self.vref_state(a);
            let (pb, tb, vb) = self.vref_state(b);
            let (pu, tu, pd, sign) = if pa >= pb { (pa, ta, pb, 1.0) } else { (pb, tb, pa, -1.0) };
            let mut dm = orifice_mdot(0.8 * area * frac, pu, tu, pd) * dt;
            let dm_eq = (pu - pd) / (GAMMA * R_GAS * tu * (1.0 / va + 1.0 / vb));
            dm = dm.min(0.5 * dm_eq);
            let h = CP * tu;
            let dm_ab = sign * dm;
            if let VRef::Vol(i) = a {
                self.vols[i].dm -= dm_ab;
                self.vols[i].de -= dm_ab * h;
            }
            if let VRef::Vol(i) = b {
                self.vols[i].dm += dm_ab;
                self.vols[i].de += dm_ab * h;
            }
            self.orifs[k].mdot = dm_ab / dt;
        }

        let mut qt = prof::lap(cat::ORIF, qt);
        // --- superchargers: rotor pockets between their inlet and outlet volumes, driven from the crank
        let mut w_drive = 0.0;
        if !self.machines.is_empty() {
            let theta = self.theta;
            for m in self.machines.iter_mut() {
                w_drive += m.step(&mut self.vols, theta, dth, dt);
            }
        }
        if !self.turbos.is_empty() {
            for t in self.turbos.iter_mut() {
                t.step(&mut self.vols, dt);
            }
        }

        let pt2 = pt0.map(|_| std::time::Instant::now());
        // --- cylinders
        // band-limited (about 4 kHz) turbulence in the valve flow, unit variance at any sample rate
        if dt != self.vn_cache.0 {
            let a_n = 1.0 - (-2.0 * PI * 4000.0 * dt).exp();
            self.vn_cache = (dt, a_n, (a_n / (2.0 - a_n)).sqrt());
        }
        let (a_n, vn_norm) = (self.vn_cache.1, self.vn_cache.2);
        self.vn_state += a_n * (self.rng.noise() - self.vn_state);
        let vn = self.vn_state / vn_norm;
        let mut work_total = 0.0;
        let stroke = self.spec.stroke;
        let omega_now = self.omega;
        let cam_want = self.cam_high;
        let idle_eff = if self.idle_mode { self.idle_spark } else { 1.0 };
        let fast = self.fast_math;
        for ci in 0..self.cyls.len() {
            let cy = &mut self.cyls[ci];
            let c0 = cy.c;
            let c1 = (th1 - cy.phase).rem_euclid(cy.cyc);
            let cm = c0 + 0.5 * dth;
            let crossed = |x: f64| c0 < x && (c1 >= x || c1 < c0);
            if c1 < c0 && cy.cam_high != cam_want && cy.rotor_r == 0.0 {
                // firing TDC: both valves are shut, so the profile can change with no step in flow
                cy.cam_high = cam_want;
                let e = cy.ev[cam_want as usize];
                cy.ivo = e[0];
                cy.ivc = e[1];
                cy.evo = e[2];
                cy.evc = e[3];
                cy.in_lift_max = e[4];
                cy.ex_lift_max = e[5];
            }

            // valves
            let rotor = cy.rotor_r > 0.0;
            let li = if rotor { port_shape(cm, cy.ivo, cy.ivc) } else { lift_shape(cm, cy.ivo, cy.ivc, fast) };
            let le = if rotor { port_shape(cm, cy.evo, cy.evc) } else { lift_shape(cm, cy.evo, cy.evc, fast) };
            // for a rotor the "lift" telemetry fields carry the open fraction of the port
            cy.lift_in = if rotor { li } else { li * cy.in_lift_max };
            cy.lift_ex = if rotor { le } else { le * cy.ex_lift_max };
            if cy.two {
                // gas exchange is handled below, through ports
            } else if li > 0.0 {
                let cda = if rotor { 0.7 * cy.in_area * li } else { cy.in_n * VALVE_CD * (PI * cy.in_d * cy.lift_in).min(0.9 * PI / 4.0 * cy.in_d * cy.in_d) };
                let m_before = cy.m;
                let dm = if let Some(pv) = cy.in_port {
                    // the stream arriving down the runner keeps its momentum into the valve throat:
                    // add its dynamic head to the port's pressure for flow toward the cylinder
                    let pipe = &self.pipes[cy.in_pipe];
                    let u = pipe.out_r.q / pipe.af[pipe.n];
                    let head = if u > 0.0 { 0.5 * self.vols[pv].rho() * u * u } else { 0.0 };
                    valve_exchange_vol(cy, &mut self.vols[pv], cda, dt, head)
                } else {
                    let pipe = &mut self.pipes[cy.in_pipe];
                    let cell = pipe.n - 1;
                    valve_exchange(cy, pipe, cell, cda, dt, 0.0)
                };
                if dm >= 0.0 {
                    let from_rev = dm.min(cy.reverted);
                    cy.reverted -= from_rev;
                    cy.m_fresh += dm - from_rev;
                } else {
                    let f = (cy.m_fresh / m_before).clamp(0.0, 1.0);
                    cy.m_fresh += f * dm;
                    cy.reverted += -(1.0 - f) * dm;
                }
            }
            if !cy.two && le > 0.0 {
                let cda = if rotor { 0.7 * cy.ex_area * le } else { cy.ex_n * VALVE_CD * (PI * cy.ex_d * cy.lift_ex).min(0.9 * PI / 4.0 * cy.ex_d * cy.ex_d) };
                let pipe = &mut self.pipes[cy.ex_pipe];
                let m_before = cy.m;
                let dm = valve_exchange(cy, pipe, 0, cda, dt, (self.valve_noise * vn).clamp(-0.5, 0.5));
                if dm < 0.0 {
                    let f = (cy.m_fresh / m_before).clamp(0.0, 1.0);
                    cy.m_fresh += f * dm;
                    // unburned fuel and spare air leave in proportion to the gas that leaves
                    if let Some(g) = cy.af_group {
                        let a = &mut self.af[g];
                        a.flushed -= dm;
                        if a.pool_fuel > 0.0 || a.pool_air > 0.0 {
                            // new exhaust pushes the gathered charge on down the pipe
                            let keep = (1.0 + dm / (LIFT_HOLD * a.hold)).max(0.0);
                            a.pool_fuel *= keep;
                            a.pool_air *= keep;
                        }
                        if cy.fuel_left > 0.0 || cy.air_left > 0.0 {
                            let s = (-dm / m_before).min(1.0);
                            let (df, da) = (s * cy.fuel_left, s * cy.air_left);
                            cy.fuel_left -= df;
                            cy.air_left -= da;
                            if cy.pooled {
                                // a film-fed cycle: its fuel gathers down the pipe; its air is
                                // also there for whatever a rich cycle left in the collector
                                a.pool_fuel += df;
                                a.pool_air += da;
                                a.air += da;
                                a.in_air += da;
                            } else {
                                a.fuel += df;
                                a.air += da;
                                a.in_fuel += df;
                                a.in_air += da;
                            }
                        }
                    }
                }
            }
            if let Some(g) = cy.af_group {
                // a cylinder that burned brings a flame into its group as it starts to exhaust,
                // and keeps it there while a late burn is still going with the valve open
                if (cy.fired && crossed(cy.evo)) || (cy.burning && cy.q_tot > 0.0 && le > 0.0) {
                    self.af[g].flame = AF_FLAME_T;
                }
            }
            if cy.m_fresh < 0.0 {
                cy.m_fresh = 0.0;
            }

            if cy.two {
                let fe = cy.wall_port(cm, cy.evo);
                let ft = cy.wall_port(cm, cy.tpo);
                cy.lift_ex = fe;
                cy.lift_in = ft;
                // 1. intake pipe -> crankcase
                let cc_p = GM1 * cy.cc_e / cy.cc_v;
                let pipe = &mut self.pipes[cy.in_pipe];
                let cell = pipe.n - 1;
                let open = match cy.induction {
                    // reed petals lift with the pressure difference and pass no reverse flow
                    0 => ((pipe.prim(cell).p - cc_p) / 6_000.0).clamp(0.0, 1.0),
                    // port under the piston skirt: opens as the piston rises past it
                    1 => {
                        let half = cy.in_btdc.max(1e-3);
                        let d = cm.min(cy.cyc - cm);
                        if d < half { ((cy.piston_x(half) - cy.piston_x(d)) / cy.piston_x(half)).clamp(0.0, 1.0) } else { 0.0 }
                    }
                    // rotary disc: a timed window around TDC
                    _ => {
                        let a = if cm > PI { cm - cy.cyc } else { cm };
                        port_shape(a, -cy.in_btdc, cy.in_atdc)
                    }
                };
                if open > 0.0 {
                    cy.swap_crankcase();
                    let dm = valve_exchange(cy, pipe, cell, 0.7 * cy.in_area * open, dt, 0.0);
                    cy.swap_crankcase();
                    self.stats.cc_in += dm;
                    if dm < 0.0 {
                        // blow-back of whatever the crankcase holds
                        cy.cc_burned = (cy.cc_burned * (1.0 + dm / cy.cc_m.max(1e-9))).max(0.0);
                    }
                }
                // 2. crankcase <-> cylinder through the transfer ports
                if ft > 0.0 {
                    let (p_c, t_c) = (GM1 * cy.cc_e / cy.cc_v, cy.cc_e / (cy.cc_m * CV));
                    let (p_y, t_y) = (cy.p(), cy.t());
                    let cda = 0.7 * cy.tr_area * ft;
                    if p_c > p_y {
                        let mut dm = orifice_mdot(cda, p_c, t_c, p_y) * dt;
                        let dm_eq = (p_c - p_y) / (GAMMA * R_GAS * t_c * (1.0 / cy.cc_v + 1.0 / cy.v));
                        dm = dm.min(0.5 * dm_eq).min(0.3 * cy.cc_m).max(0.0);
                        let h = CP * t_c;
                        // burned gas that blew down into the crankcase earlier comes back first
                        let burned = dm.min(cy.cc_burned);
                        cy.cc_burned -= burned;
                        cy.cc_m -= dm;
                        cy.cc_e -= dm * h;
                        cy.m += dm;
                        cy.e += dm * h;
                        cy.m_fresh += dm - burned;
                        self.stats.delivered += dm - burned;
                    } else {
                        let mut dm = orifice_mdot(cda, p_y, t_y, p_c) * dt;
                        let dm_eq = (p_y - p_c) / (GAMMA * R_GAS * t_y * (1.0 / cy.cc_v + 1.0 / cy.v));
                        dm = dm.min(0.5 * dm_eq).min(0.3 * cy.m).max(0.0);
                        let h = CP * t_y;
                        let f = (cy.m_fresh / cy.m).clamp(0.0, 1.0);
                        cy.m -= dm;
                        cy.e -= dm * h;
                        cy.m_fresh -= f * dm;
                        self.stats.delivered -= f * dm;
                        cy.cc_m += dm;
                        cy.cc_e += dm * h;
                        cy.cc_burned += (1.0 - f) * dm;
                    }
                }
                // 3. cylinder <-> exhaust pipe
                if fe > 0.0 {
                    let pipe = &mut self.pipes[cy.ex_pipe];
                    let m_before = cy.m;
                    let dm = valve_exchange(cy, pipe, 0, 0.7 * cy.ex_area * fe, dt, (self.valve_noise * vn).clamp(-0.5, 0.5));
                    if dm < 0.0 {
                        // Scavenging is neither perfect displacement nor perfect mixing: the fresh
                        // stream enters low and loops up the far wall, so early on mostly burned gas
                        // leaves. The share of fresh charge in the outflow is taken as the square of
                        // its share in the cylinder (an estimate between those two limits).
                        let f0 = (cy.m_fresh / m_before).clamp(0.0, 1.0);
                        let f = (f0 * f0).min(cy.m_fresh / (-dm).max(1e-12));
                        cy.m_fresh += f * dm;
                        cy.spilled -= f * dm; // fresh charge that short-circuited into the pipe
                        self.stats.spilled -= f * dm;
                    } else {
                        // the pipe's returning wave pushes some of that spilled charge back in
                        let back = 0.8 * dm.min(cy.spilled);
                        cy.spilled -= dm.min(cy.spilled);
                        cy.m_fresh += back;
                        self.stats.spilled -= back;
                    }
                }
                if cy.m_fresh < 0.0 {
                    cy.m_fresh = 0.0;
                }
            }

            qt = prof::lap(cat::CYL_VALVE, qt);
            // piston work (adiabatic volume change)
            let v0 = cy.v;
            let (v1, x1) = cy.volume_x(c1);
            let e0 = cy.e;
            cy.e *= if fast { cbrt_fast(v0 / v1) } else { pow_gm1(v0 / v1) };
            if cy.two {
                // the piston's underside works against the crankcase gas, not the atmosphere
                let cv0 = cy.cc_v;
                let cv1 = cy.cc_v_tdc - cy.a_p * x1;
                let ce0 = cy.cc_e;
                cy.cc_e *= if fast { cbrt_fast(cv0 / cv1) } else { pow_gm1(cv0 / cv1) };
                cy.cc_v = cv1;
                work_total += e0 - cy.e + ce0 - cy.cc_e;
            } else {
                work_total += e0 - cy.e - P_ATM * (v1 - v0);
            }
            cy.v = v1;

            // plan the burn when the intake valve closes
            if crossed(cy.ivc) {
                cy.p_ivc = cy.p();
                cy.v_ivc = cy.v;
                cy.t_ivc = cy.t();
                cy.reverted = 0.0;
                cy.spilled = 0.0;
                let load = (cy.m_fresh / (RHO_ATM * cy.v_d)).min(1.2);
                let x_res = (1.0 - cy.m_fresh / cy.m).clamp(0.0, 1.0);
                let sigma = 0.035 + 0.5 * (x_res - 0.18).max(0.0);
                let g1 = self.rng.noise();
                let g2 = self.rng.noise();
                let dur_deg = (40.0 + 14.0 * (rpm / 6000.0) + 20.0 * (1.0 - load.min(1.0)) + 60.0 * (x_res - 0.12).max(0.0)) * (1.0 + sigma * g1).clamp(0.6, 1.8);
                // the same burn takes more shaft degrees when the cycle is longer (and a rotor's long, thin chamber burns slower still)
                let kc = if cy.rotor_r > 0.0 { cy.cyc / FOUR_PI } else { 1.0 };
                let dur_deg = dur_deg * kc * cy.burn_scale;
                // mixture: slower burns away from phi 1.05 (the spark follows, as an ECU's map would)
                let dur_deg = if self.phi != 1.0 { dur_deg * mixture_burn_dur(self.phi) } else { dur_deg };
                cy.burn_dur = dur_deg * PI / 180.0;
                let mut adv = (0.517 * dur_deg - 8.0 * kc).clamp(2.0, 60.0 * kc * cy.burn_scale);
                if self.knock_k > 0.0 {
                    // knock-limited retard on boost (an ECU's knock map, as a rule; an estimate)
                    adv = (adv - self.knock_k * (cy.p_ivc / 1e5 - self.knock_onset).max(0.0)).max(1.0);
                }
                cy.spark = cy.cyc - adv * PI / 180.0;
                if self.tc_retard > 0.0 {
                    // traction control on a boosted engine: retarded, past TDC if need be
                    let a = adv - self.tc_retard;
                    cy.spark = if a >= 0.0 { cy.cyc - a * PI / 180.0 } else { -a * PI / 180.0 };
                }
                let mut eff = (1.0 - 0.5 * sigma * g2.abs()).clamp(0.0, 1.0);
                // very dilute charges burn partially or not at all
                let dilute = (x_res - 0.32).max(0.0);
                eff *= (1.0 - 3.0 * dilute - 2.0 * dilute * self.rng.uniform()).clamp(0.0, 1.0);
                // Fueling. With the defaults (phi 1, no overrun fueling, fuel-cut limiter) this
                // reduces to the original arithmetic exactly and draws no random numbers.
                let fueled = if self.fuel_cut {
                    self.overrun_fuel > 0.0 && self.af_rng.uniform() < self.overrun_fuel
                } else {
                    !self.limiter || self.limiter_kind == Limiter::SparkCut
                };
                // Lift-off pops: part of the injected fuel lands on the port walls and evaporates
                // from there, so a cycle gets what the injector adds now plus what the film gives
                // up. While fuel is injected the ECU trims the injector and the cylinder gets its
                // commanded mixture (nothing below changes) unless the film alone gives more.
                // With the injector off the cycle runs on the film.
                let mut phi = self.phi;
                let mut film_phi = 0.0;
                if self.liftoff > 0.0 {
                    let x = LIFT_WET * self.liftoff * load.min(1.0);
                    let evap = cy.film * (1.0 - (-(cy.cyc / self.omega.max(1.0)) / LIFT_TAU).exp());
                    let cmd = if fueled { self.phi * cy.m_fresh } else { 0.0 };
                    if evap > cmd {
                        cy.film -= evap;
                        if fueled {
                            phi = evap / cy.m_fresh.max(1e-12);
                        } else {
                            film_phi = evap / cy.m_fresh.max(1e-12);
                        }
                    } else {
                        cy.film += x * (cmd - evap) / (1.0 - x) - evap;
                    }
                }
                cy.pooled = film_phi > 0.0;
                if film_phi > 0.0 {
                    // A film-fed cycle: thin, diluted and with the spark pulled back, it seldom
                    // lights; when it does it burns late and badly, like a fueled overrun cycle.
                    // What is not burned goes to the exhaust as fuel and air.
                    let fire = self.af_rng.uniform() < LIFT_FIRE * (film_phi / LIFT_PHI_FIRE).min(1.0);
                    let burn = eff * OVERRUN_BURN;
                    let q = if fire { self.spec.heat_q * cy.m_fresh * idle_eff * mixture_heat(film_phi) * burn } else { 0.0 };
                    if fire {
                        cy.spark = OVERRUN_SPARK_ATDC * kc * PI / 180.0;
                        cy.burn_dur *= OVERRUN_DUR;
                    }
                    cy.q_tot = q;
                    let air = cy.m_fresh;
                    let fuel = film_phi * air;
                    let used = if fire { burn * air.min(fuel) } else { 0.0 };
                    cy.fuel_left = (fuel - used).max(0.0);
                    cy.air_left = (air - used).max(0.0);
                    // (its flame reaches the exhaust even when the diluted charge gives no heat)
                    cy.fired = fire;
                    cy.late = fire;
                    if let Some(log) = self.lift_log.as_mut() {
                        log.push([self.t, film_phi, air / (RHO_ATM * cy.v_d), x_res, if fire { 1.0 } else { 0.0 }]);
                    }
                } else {
                    let late = fueled && self.fuel_cut;
                    let mut fire = fueled && !(self.limiter && !self.fuel_cut);
                    let fires = if phi != 1.0 { mixture_fires(phi) } else { 1.0 };
                    if fire && fires < 1.0 {
                        fire = self.af_rng.uniform() < fires;
                    }
                    let mut q = if fire { self.spec.heat_q * cy.m_fresh * eff * idle_eff } else { 0.0 };
                    let burn = if late { OVERRUN_BURN } else { 1.0 };
                    if phi != 1.0 || late {
                        q *= mixture_heat(phi) * burn;
                    }
                    if late && fire {
                        // a fueled overrun cycle: lit late, burns slowly and badly
                        cy.spark = OVERRUN_SPARK_ATDC * kc * PI / 180.0;
                        cy.burn_dur *= OVERRUN_DUR;
                    }
                    cy.q_tot = q;
                    // what this cycle leaves: fuel (as the air it would need) and air, after the burn
                    let air = cy.m_fresh;
                    let fuel = if fueled { phi * air } else { 0.0 };
                    let used = if fire { burn * air.min(fuel) } else { 0.0 };
                    cy.fuel_left = (fuel - used).max(0.0);
                    cy.air_left = (air - used).max(0.0);
                    cy.fired = fire && q > 0.0;
                    cy.late = late && fire;
                }
                cy.planned = true;
                cy.q_dbg = cy.q_tot;
                cy.air_last = cy.m_fresh;
                self.stats.air += cy.m_fresh;
                self.stats.p_peak = self.stats.p_peak.max(cy.p_peak);
                cy.p_peak = 0.0;
            }
            if cy.planned && crossed(cy.spark) {
                cy.planned = false;
                cy.burning = true;
                cy.burn_ang = 0.0;
                cy.xb_prev = 0.0;
                if cy.q_tot > 0.0 {
                    cy.m_fresh = 0.0;
                }
            }
            cy.hrr = 0.0;
            if cy.burning {
                cy.burn_ang += dth;
                let x = cy.burn_ang / cy.burn_dur;
                let xb = if x >= 1.15 { 1.0 } else { 1.0 - (-5.0 * x * x * x).exp() };
                let dq = cy.q_tot * (xb - cy.xb_prev);
                cy.e += dq;
                cy.hrr = if cy.q_tot > 0.0 { (xb - cy.xb_prev) / dth * cy.burn_dur } else { 0.0 };
                cy.xb_prev = xb;
                if x >= 1.15 {
                    cy.burning = false;
                }
            }

            qt = prof::lap(cat::CYL_PISTON_BURN, qt);
            prof::count(cnt::POWF, 4);
            // heat loss to the walls (Woschni)
            let p = cy.p();
            let t = cy.t();
            let q4 = 0.25 * cy.cyc;
            let gas_exchange = c1 > q4 && c1 < 3.0 * q4;
            let (area, sp) = cy.wall(v1, x1, omega_now, stroke);
            let mut w = if gas_exchange { 6.18 * sp } else { 2.28 * sp };
            if !gas_exchange && c1 < q4 {
                let r = cy.v_ivc / cy.v;
                let p_mot = cy.p_ivc * if fast { r * cbrt_fast(r) } else { pow_g(r) };
                w += 3.24e-3 * cy.v_d * cy.t_ivc / (cy.p_ivc * cy.v_ivc) * (p - p_mot).max(0.0);
            }
            let h = if fast {
                // one log per factor and one exp instead of three pow calls; same correlation
                3.26 * cy.clen_pow * (0.8 * (p * 1e-3 * w.max(0.5)).ln() - 0.55 * t.ln()).exp()
            } else {
                3.26 * cy.clen_pow * (p * 1e-3).powf(0.8) * t.powf(-0.55) * w.max(0.5).powf(0.8)
            };
            cy.e -= h * area * (t - T_CYL_WALL) * dt;
            if cy.e < 1e-6 {
                cy.e = 1e-6;
            }
            cy.c = c1;
            if p > cy.p_peak {
                cy.p_peak = p;
            }
            qt = prof::lap(cat::CYL_HEAT, qt);
        }

        // --- afterfire in each exhaust group's collector
        if !self.af.is_empty() {
            self.afterfire(dt);
        }
        let qt = prof::lap(cat::AFTERFIRE, qt);

        let pt3 = pt0.map(|_| std::time::Instant::now());
        // --- volumes
        for v in self.vols.iter_mut() {
            v.m += v.dm;
            v.e += v.de;
            v.dm = 0.0;
            v.de = 0.0;
            if v.m < 1e-9 {
                v.m = 1e-9;
            }
            if v.e < 1e-6 {
                v.e = 1e-6;
            }
        }

        // --- crank
        let t_gas = if dth.abs() > 1e-12 { work_total / dth } else { 0.0 };
        let n1k = rpm / 1000.0;
        let fmep = self.spec.friction_scale * (0.70 + 0.11 * n1k + 0.03 * n1k * n1k) * 1e5;
        let t_fric = fmep * self.spec.disp() / FOUR_PI;
        let t_net = if self.machines.is_empty() {
            t_gas - t_fric
        } else {
            self.stats.drive_work += w_drive;
            t_gas - t_fric - if dth.abs() > 1e-12 { w_drive / dth } else { 0.0 }
        };
        self.stats.work += work_total;
        self.stats.fric_work += t_fric * dth;
        self.stats.angle += dth;
        match &mut self.mode {
            Mode::Dyno => {}
            Mode::Free => {
                self.omega += dt * t_net / self.spec.inertia;
            }
            Mode::Drive(d) => {
                let g = d.gears[d.gear] * d.final_drive;
                let resist = 0.5 * 1.2 * d.cda * d.v * d.v + d.mass * 9.81 * 0.012 + d.brake * 9.0 * d.mass;
                let idle_w = self.spec.idle * PI / 30.0;
                if d.shifting > 0.0 {
                    d.shifting -= dt;
                    self.omega += dt * t_net / self.spec.inertia;
                    d.v = (d.v - dt * resist / d.mass).max(0.0);
                    let target = d.v * g / d.wheel_r;
                    if d.locked {
                        if d.shifting < 0.45 * d.shift_time {
                            self.omega += (target - self.omega) * (dt / 0.025).min(1.0);
                        }
                        if d.shifting <= 0.0 {
                            self.omega = target;
                        }
                    }
                } else if d.grip.is_finite() {
                    // a vehicle from the definition (docs/physics.md, "Vehicles in Drive"): driveline loss,
                    // tyres that pass force through slip, and traction control
                    let k = g / d.wheel_r; // engine rad/s per m/s of road speed
                    // the driven tyres' peak force: mu x (static load + weight transfer from the net
                    // accelerating force), solved for the force; d.grip is mu x the static load
                    let grip = (d.grip - d.mu * d.grip_hl * resist) / (1.0 - d.mu * d.grip_hl);
                    d.t_avg += (t_net - d.t_avg) * (dt / 0.03).min(1.0);
                    if d.locked {
                        // clutch closed: the driven wheels turn with the engine and the tyres pass
                        // a force that grows with their slip up to the peak (tyre_force)
                        let u_w = self.omega / k;
                        let slip = (u_w - d.v) / u_w.max(d.v).max(0.5);
                        let f = grip * tyre_force(slip);
                        let t_load = if f >= 0.0 { f / (k * d.eta) } else { f * d.eta / k };
                        self.omega += dt * (t_net - t_load) / self.spec.inertia;
                        d.v = (d.v + dt * (f - resist) / d.mass).max(0.0);
                        // traction control (or the driver's foot): trims the throttle so the tyres
                        // sit at the slip of peak grip; it only ever takes throttle away
                        d.tc = (d.tc + dt * TC_RATE * (1.0 - slip / TYRE_SLIP_PEAK)).clamp(0.05, 1.0);
                        let rpm_now = self.omega * 30.0 / PI;
                        if d.auto && rpm_now >= d.shift_rpm && d.gear + 1 < d.gears.len() {
                            d.gear += 1;
                            d.shifting = d.shift_time;
                        } else if d.auto && d.down_rpm > 0.0 && rpm_now < d.down_rpm && d.gear > 0 && rpm_now * d.gears[d.gear - 1] / d.gears[d.gear] < 0.85 * d.shift_rpm {
                            d.gear -= 1;
                            d.shifting = d.shift_time;
                        } else if self.omega < 0.92 * idle_w {
                            d.locked = false;
                        }
                    } else {
                        // slipping clutch (pulling away), the engine held near its launch speed: the
                        // engine carries the clutch's torque and the car gets what the tyres pass,
                        // at most their peak force; any more spins them
                        let w_v = d.v * k;
                        let t_max = self.throttle_cmd * d.clutch_nm_per_l * self.spec.disp() * 1000.0;
                        let t_c = (d.clutch_gain * (self.omega - w_v.max(d.clutch_launch))).clamp(0.0, t_max);
                        self.omega += dt * (t_net - t_c) / self.spec.inertia;
                        d.v = (d.v + dt * ((d.eta * t_c * k).min(grip) - resist) / d.mass).max(0.0);
                        // traction control holds what the engine pushes with at the tyres' peak
                        d.tc = (d.tc + dt * TC_RATE * (1.0 - d.eta * d.t_avg.max(0.0) * k / grip.max(1.0))).clamp(0.05, 1.0);
                        if (w_v >= 0.98 * self.omega || w_v >= d.clutch_launch) && w_v > 1.02 * idle_w {
                            d.locked = true;
                            self.omega = w_v;
                        }
                    }
                } else if d.locked {
                    let i_tot = self.spec.inertia + d.mass * d.wheel_r * d.wheel_r / (g * g);
                    self.omega += dt * (t_net - resist * d.wheel_r / g) / i_tot;
                    d.v = self.omega * d.wheel_r / g;
                    let rpm_now = self.omega * 30.0 / PI;
                    if d.auto && rpm_now >= d.shift_rpm && d.gear + 1 < d.gears.len() {
                        d.gear += 1;
                        d.shifting = d.shift_time;
                    } else if d.auto && d.down_rpm > 0.0 && rpm_now < d.down_rpm && d.gear > 0 && rpm_now * d.gears[d.gear - 1] / d.gears[d.gear] < 0.85 * d.shift_rpm {
                        d.gear -= 1;
                        d.shifting = d.shift_time;
                    } else if self.omega < 0.92 * idle_w {
                        d.locked = false; // let the clutch slip rather than stall
                    }
                } else {
                    // slipping clutch: pulls away from rest and keeps the engine alive at low road speed
                    let w_v = d.v * g / d.wheel_r;
                    let thr = self.throttle_cmd;
                    let t_max = thr * d.clutch_nm_per_l * self.spec.disp() * 1000.0;
                    let t_c = (d.clutch_gain * (self.omega - w_v.max(d.clutch_launch))).clamp(0.0, t_max);
                    self.omega += dt * (t_net - t_c) / self.spec.inertia;
                    d.v = (d.v + dt * (t_c * g / d.wheel_r - resist) / d.mass).max(0.0);
                    if (w_v >= 0.98 * self.omega || (d.clutch_launch > 0.0 && w_v >= d.clutch_launch)) && w_v > 1.02 * idle_w {
                        d.locked = true;
                        self.omega = w_v;
                    }
                }
            }
        }
        if self.omega < 5.0 {
            self.omega = 5.0;
        }
        prof::lap(cat::VOL_CRANK, qt);
        if let (Some(a), Some(b), Some(c), Some(d)) = (pt0, pt1, pt2, pt3) {
            let e = std::time::Instant::now();
            self.prof[0] += (b - a).as_nanos() as u64;
            self.prof[1] += (c - b).as_nanos() as u64;
            self.prof[2] += (d - c).as_nanos() as u64;
            self.prof[3] += (e - d).as_nanos() as u64;
        }
        self.theta = th1;
        self.t += dt;
        self.cfl = cfl;
        t_net
    }

    /// Advance one sample at the inner rate. Returns [exhaust L, exhaust R, intake, block] in pascals at the mic.
    pub fn advance(&mut self) -> [f64; 4] {
        let qt = prof::now();
        prof::count(cnt::ADVANCES, 1);
        prof::count(cnt::POWF, 1);
        let dt = self.dt;
        let rpm = self.rpm();

        // controls
        let shifting = matches!(&self.mode, Mode::Drive(d) if d.shifting > 0.0);
        // a vehicle's traction control trims the throttle (always 1 otherwise, which leaves it exact)
        let tc = if let Mode::Drive(d) = &self.mode { d.tc } else { 1.0 };
        // on a boosted engine the trim is spark retard (TC_RETARD_MAX), and the throttle stays as asked
        let boosted = self.spec.boost.is_some();
        self.tc_retard = if boosted && tc < 1.0 { (1.0 - tc) * TC_RETARD_MAX } else { 0.0 };
        let cmd = if shifting { 0.0 } else if boosted { self.throttle_cmd } else { self.throttle_cmd * tc };
        // Idle: a slow air loop sets the mean, and a fast combustion-phasing reserve
        // (the stand-in for an ECU's spark control) holds the speed between firings.
        // anticipate falling revs so the catch starts before idle is reached
        let raw_rate = (rpm - self.rpm_prev) / dt;
        self.rpm_prev = rpm;
        // smooth over roughly one and a half firing intervals, or the torque ripple of a
        // single or twin reads as falling revs
        let tau = (180.0 / (rpm.max(100.0) * self.cyls.len() as f64) * 2.0).clamp(0.04, 0.3);
        self.rpm_rate += (raw_rate - self.rpm_rate) * (dt / tau);
        let rpm_pred = rpm + 0.15 * self.rpm_rate.min(0.0);
        let err = (self.spec.idle - rpm_pred) / self.spec.idle;
        self.idle_mode = cmd < 0.02 && rpm < self.spec.idle * 1.8 && !matches!(self.mode, Mode::Dyno);
        if !matches!(self.mode, Mode::Dyno) {
            // An engine can hang above the idle band and below the overrun fuel cut, firing on
            // the dashpot air with the air loop frozen (a low-idle engine started with more idle
            // air than it needs sat at twice its idle speed for good). If it stays there off
            // throttle without slowing, and the wheels are not driving it, let the loop work.
            let driven = matches!(&self.mode, Mode::Drive(d) if d.locked);
            if cmd < 0.02 && !self.idle_mode && !self.fuel_cut && !driven && self.rpm_rate > -0.25 * self.spec.idle {
                self.idle_hang += dt;
            } else {
                self.idle_hang = 0.0;
            }
            if self.idle_mode || self.idle_hang > 0.5 {
                // The idle air an engine needs, as a fraction of its throttle area, varies a
                // hundredfold between a big plenum V8 and a small engine with one large throttle
                // per cylinder, so the integrator works on the logarithm of the opening.
                self.idle_integ = (self.idle_integ * (2.0 * err * dt).exp()).clamp(2.0e-5, 0.05);
            }
            // on the overrun the idle air stays open (a dashpot), so the first firings after
            // fuel comes back have a normal charge to burn
            // (air scales with speed above idle so the charge per cycle stays at the idle level)
            let trim = (self.idle_integ * (rpm / self.spec.idle).clamp(1.0, 8.0) * (1.0 + 6.0 * err.clamp(-0.12, 1.0))).clamp(2.0e-5, 0.10);
            self.throttle_eff = self.throttle_shape(cmd).max(trim);
            self.idle_spark = (0.78 + 2.5 * err).clamp(0.4, 1.0);
        } else {
            self.throttle_eff = self.throttle_shape(cmd).max(0.002);
        }
        if self.spec.cam_high.is_some() {
            if rpm >= self.spec.cam_switch_rpm {
                self.cam_high = true;
            } else if rpm < self.spec.cam_switch_rpm - self.spec.cam_switch_hyst {
                self.cam_high = false;
            }
        }
        if rpm > self.spec.redline {
            self.limiter = true;
        } else if rpm < self.spec.redline - 120.0 {
            self.limiter = false;
        }
        let cut_above = (self.spec.idle * 2.2).max(self.spec.idle + 900.0);
        if shifting || (cmd < 0.01 && rpm > cut_above) {
            self.fuel_cut = true;
        } else if cmd >= 0.01 || rpm < cut_above - 350.0 {
            self.fuel_cut = false;
        }

        prof::lap(cat::CONTROLS, qt);
        let nsub = ((self.cfl / 1.0).ceil() as usize).clamp(1, 8);
        self.max_sub = self.max_sub.max(nsub);
        let mut tq = 0.0;
        for _ in 0..nsub {
            tq += self.substep(dt / nsub as f64);
        }
        self.torque = tq / nsub as f64;

        // statistics for the dyno table
        self.stats.n += 1.0;
        self.stats.t_coll += self.vols[self.collectors[0]].t();
        self.stats.p_coll += self.vols[self.collectors[0]].p();
        self.stats.p_last += self.vols[self.vols.len() - 1].p();
        if let Some(pl) = self.plenum {
            self.stats.p_man += self.vols[pl].p();
        }

        let pm0 = if self.prof_on { Some(std::time::Instant::now()) } else { None };
        let qt = prof::now();
        // microphones: each open end is a monopole, p = rho0 * dQ/dt / (4 pi r)
        let mut out = [0.0f64; 4];
        let fs = 1.0 / dt;
        for si in 0..self.sources.len() {
            let (q, jet) = match self.sources[si].kind {
                SrcKind::PipeRight(pi) => {
                    let p = &self.pipes[pi];
                    let w = p.prim(p.n - 1);
                    (p.out_r.q, Some((w, p.dia[p.n - 1], p.af[p.n])))
                }
                SrcKind::PipeLeft(pi) => (self.pipes[pi].out_l.q, None),
                SrcKind::Orifice(oi) => (-self.orifs[oi].mdot / RHO_ATM, None),
            };
            let s = &mut self.sources[si];
            let mut sig = RHO_ATM * (q - s.q_prev) / dt / (4.0 * PI * s.dist);
            s.q_prev = q;
            if s.stem == 0 {
                if let Some((w, d, area)) = jet {
                    sig += jet_noise(s, &mut self.rng, w, d, area, fs);
                }
                out[0] += sig * (1.0 - s.pan) * 2.0f64.sqrt();
                out[1] += sig * s.pan * 2.0f64.sqrt();
            } else {
                if s.jet {
                    if let Some((w, d, area)) = jet {
                        sig += jet_noise(s, &mut self.rng, w, d, area, fs);
                    }
                }
                out[2] += sig;
            }
            // keep a short, down-sampled history of what this opening radiated (for the picture only)
            s.hist_acc += sig;
            s.hist_n += 1;
            if s.hist_n as f64 * dt * SOUND_RATE >= 1.0 {
                s.hist[s.hist_pos] = (s.hist_acc / s.hist_n as f64) as f32;
                s.hist_pos = (s.hist_pos + 1) % SOUND_SAMPLES;
                s.hist_acc = 0.0;
                s.hist_n = 0;
            }
        }
        let qt = prof::lap(cat::MICS, qt);
        // turbo whistle stand-in, radiated from the turbo into the intake stem (turbo engines only)
        if !self.turbos.is_empty() {
            for t in &self.turbos {
                out[2] += t.wh_q;
            }
        }
        // structure-borne noise: cylinder pressure rise rate ringing a few block modes (a tuned stand-in, not first principles)
        let mut x = 0.0;
        for cy in self.cyls.iter_mut() {
            let p = cy.p();
            x += (p - cy.p_prev) / dt;
            cy.p_prev = p;
        }
        let modes = [(650.0, 5.0, 1.0), (1450.0, 6.0, 0.7), (2900.0, 7.0, 0.45)];
        if fs != self.block_cache.0 {
            self.block_cache = (fs, [0.0; 3]);
            for (m, (f, _, _)) in modes.iter().enumerate() {
                self.block_cache.1[m] = (PI * f / fs).tan();
            }
        }
        for (m, (_, qf, gain)) in modes.iter().enumerate() {
            let g = self.block_cache.1[m];
            out[3] += gain * self.block[m].bp(x, g, 1.0 / qf) * 2.0e-10;
        }
        if let Some(a) = pm0 {
            self.prof[4] += a.elapsed().as_nanos() as u64;
        }
        prof::lap(cat::BLOCK, qt);
        out
    }

    pub fn reset_stats(&mut self) {
        self.stats = Stats::default();
    }

    /// Fill a telemetry frame. Layout is described by `telemetry_layout`.
    pub fn snapshot(&mut self, samples: &[usize], out: &mut Vec<f32>) {
        let qt = prof::now();
        prof::count(cnt::SNAPSHOTS, 1);
        for p in self.pipes.iter_mut() {
            p.refresh_cells();
        }
        let (gear, speed) = match &self.mode {
            Mode::Drive(d) => (d.gear as f32 + 1.0, d.v as f32),
            _ => (0.0, 0.0),
        };
        out.extend_from_slice(&[self.t as f32, (self.theta % (3.0 * FOUR_PI * 1.0e4)) as f32, self.rpm() as f32, self.throttle_eff as f32, self.torque as f32, gear, speed, if self.limiter || self.fuel_cut { 1.0 } else { 0.0 }]);
        for cy in &self.cyls {
            out.extend_from_slice(&[cy.p() as f32, cy.t() as f32, cy.lift_in as f32, cy.lift_ex as f32, cy.hrr as f32, cy.c as f32]);
        }
        for (p, &s) in self.pipes.iter().zip(samples) {
            for k in 0..s {
                let i0 = k * p.n / s;
                let i1 = (((k + 1) * p.n) / s).max(i0 + 1).min(p.n);
                let (mut pp, mut tt, mut uu) = (0.0, 0.0, 0.0);
                for i in i0..i1 {
                    let w = p.prim(i);
                    pp += w.p;
                    tt += w.p / (w.rho * R_GAS);
                    uu += w.u;
                }
                let nn = (i1 - i0) as f64;
                out.extend_from_slice(&[(pp / nn) as f32, (tt / nn) as f32, (uu / nn) as f32]);
            }
        }
        for v in &self.vols {
            out.extend_from_slice(&[v.p() as f32, v.t() as f32]);
        }
        // afterfire glow per exhaust group, 0..1 (always 0 for engines without collectors)
        for g in 0..self.collectors.len() {
            out.push(self.af.get(g).map_or(0.0, |a| a.glow) as f32);
        }
        self.snapshot_sound(out);
        // superchargers, after everything else so no existing offset moves (none for other engines)
        for m in &self.machines {
            let pl = self.plenum.map_or(P_ATM, |i| self.vols[i].p());
            out.extend_from_slice(&[(pl - P_ATM) as f32, (self.vols[m.u].p() - P_ATM) as f32, (self.vols[m.d].p() - P_ATM) as f32, (self.omega * m.ratio * 30.0 / PI) as f32, m.bypass.as_ref().map_or(0.0, |b| b.x) as f32, (m.power * 1e-3) as f32]);
        }
        for t in &self.turbos {
            let pl = self.plenum.map_or(P_ATM, |i| self.vols[i].p());
            let ti = t.bov.as_ref().map_or(t.d, |b| b.from);
            out.extend_from_slice(&[(pl - P_ATM) as f32, (self.vols[ti].p() - P_ATM) as f32, (self.vols[t.d].p() - P_ATM) as f32, t.shaft_rpm() as f32, t.pr_c as f32, t.surge_margin as f32, t.wg.as_ref().map_or(0.0, |w| w.x) as f32, t.bov.as_ref().map_or(0.0, |b| b.x) as f32, (t.p_turb * 1e-3) as f32, (t.p_comp * 1e-3) as f32]);
        }
        prof::lap(cat::SNAPSHOT, qt);
    }

    /// Blow-off valve routing on every turbo, while running: "vent", "recirc" or "none".
    /// Returns false for an engine without a turbo or an unknown mode.
    pub fn set_blowoff(&mut self, mode: &str) -> bool {
        let m = match mode {
            "none" => 0u8,
            "recirc" => 1,
            "vent" => 2,
            _ => return false,
        };
        let mut any = false;
        for t in self.turbos.iter_mut() {
            if let Some(b) = t.bov.as_mut() {
                b.mode = m;
                any = true;
            }
        }
        any
    }

    /// Whistle stand-in gain on every turbo, while running (0 = none; the definition's default otherwise).
    pub fn set_whistle(&mut self, gain: f64) {
        for t in self.turbos.iter_mut() {
            t.whistle = gain.clamp(0.0, 16.0);
        }
    }

    /// Telemetry fields per turbocharger, after the supercharger block.
    pub const TURBO_FIELDS: [&'static str; 10] = ["boost_pa", "charge_pa", "comp_out_pa", "shaft_rpm", "pressure_ratio", "surge_margin", "wastegate", "bov", "turbine_kw", "compressor_kw"];

    /// Where the turbocharger values sit in a telemetry frame, and how many turbos there are.
    pub fn turbo_layout(&self, samples: &[usize]) -> (usize, usize) {
        let (off, n) = self.boost_layout(samples);
        (off + n * Self::BOOST_FIELDS.len(), self.turbos.len())
    }

    /// The telemetry header's description of the turbocharger block (None for other engines).
    pub fn turbo_header(&self, samples: &[usize]) -> Option<serde_json::Value> {
        let b = self.spec.boost.as_ref()?;
        let t = b.turbo.as_ref()?;
        let (off, n) = self.turbo_layout(samples);
        Some(serde_json::json!({"offset": off, "count": n, "fields": Self::TURBO_FIELDS, "units": "Pa gauge, rpm, ratio, share, 0..1, kW", "induction": b.induction, "d2": t.d2, "blades": t.blades, "splitters": t.splitters, "bov": t.bov, "scroll": t.scroll, "whistle": t.whistle}))
    }

    /// Telemetry fields per supercharger, at the end of each frame.
    pub const BOOST_FIELDS: [&'static str; 6] = ["boost_pa", "inlet_pa", "outlet_pa", "rotor_rpm", "bypass", "drive_kw"];

    /// Where the supercharger values sit in a telemetry frame, and how many machines there are.
    pub fn boost_layout(&self, samples: &[usize]) -> (usize, usize) {
        let (af_off, af_n) = self.afterfire_layout(samples);
        (af_off + af_n + SOUND_SAMPLES * self.sources.len(), self.machines.len())
    }

    /// The telemetry header's description of the supercharger block (None for other engines).
    pub fn boost_header(&self, samples: &[usize]) -> Option<serde_json::Value> {
        let b = self.spec.boost.as_ref()?;
        let (off, n) = self.boost_layout(samples);
        let m = self.machines.first()?;
        let _ = b.turbo.as_ref();
        Some(serde_json::json!({"offset": off, "count": n, "fields": Self::BOOST_FIELDS, "units": "Pa gauge, rpm, 0..1, kW", "kind": b.kind, "induction": b.induction, "throttle": b.throttle, "drive_ratio": b.drive_ratio, "displacement": b.displacement, "pockets_per_rev": m.pockets_per_rev()}))
    }

    /// Where the afterfire values sit in a telemetry frame, and how many there are.
    pub fn afterfire_layout(&self, samples: &[usize]) -> (usize, usize) {
        let off = 8 + 6 * self.cyls.len() + samples.iter().map(|s| 3 * s).sum::<usize>() + 2 * self.vols.len();
        (off, self.collectors.len())
    }

    /// Append each opening's sound history, newest sample first (index k is k / SOUND_RATE seconds old).
    fn snapshot_sound(&self, out: &mut Vec<f32>) {
        for s in &self.sources {
            for k in 0..SOUND_SAMPLES {
                out.push(s.hist[(s.hist_pos + SOUND_SAMPLES - 1 - k) % SOUND_SAMPLES]);
            }
        }
    }

    /// Where the sound block sits in a frame and what each opening is, for the telemetry header.
    /// `offset` is the index of the first float after the volumes.
    pub fn sound_header(&self, offset: usize) -> serde_json::Value {
        let sources: Vec<_> = self
            .sources
            .iter()
            .map(|s| match s.kind {
                SrcKind::PipeRight(pi) => serde_json::json!({"stem": if s.stem == 0 { "exhaust" } else { "intake" }, "pipe": pi, "end": "right", "name": self.pipes[pi].name}),
                SrcKind::PipeLeft(pi) => serde_json::json!({"stem": if s.stem == 0 { "exhaust" } else { "intake" }, "pipe": pi, "end": "left", "name": self.pipes[pi].name}),
                SrcKind::Orifice(oi) => {
                    // a throttle open to the air: it feeds a small volume at the mouth of one runner
                    let vol = match self.orifs[oi].b {
                        VRef::Vol(i) => i as i64,
                        VRef::Ambient => -1,
                    };
                    serde_json::json!({"stem": "intake", "pipe": -1, "volume": vol, "name": if vol >= 0 { self.vols[vol as usize].name.clone() } else { String::new() }})
                }
            })
            .collect();
        serde_json::json!({"offset": offset, "count": self.sources.len(), "samples": SOUND_SAMPLES, "rate": SOUND_RATE, "unit": "Pa at 1 m, newest first", "speed_of_sound": C_ATM, "sources": sources})
    }
}

/// Turbulent jet mixing noise at an open pipe end (Lighthill's eighth-power law), band-limited
/// around St ~ 0.25; pascals at the source's distance.
#[inline]
fn jet_noise(s: &mut Source, rng: &mut Rng, w: Prim, d: f64, area: f64, fs: f64) -> f64 {
    let c = w.a();
    let u = w.u.abs().min(c);
    let power = 3e-5 * w.rho * u.powi(8) * area / c.powi(5);
    let p_rms = (power * RHO_ATM * C_ATM / (4.0 * PI * s.dist * s.dist)).sqrt();
    let fc = (0.25 * u / d).clamp(150.0, 6000.0);
    let g = (PI * fc / fs).tan();
    let k = 1.0 / 0.8;
    let x = rng.noise();
    let v1 = (s.ic1 + g * (x - s.ic2)) / (1.0 + g * (g + k));
    let v2 = s.ic2 + g * v1;
    s.ic1 = 2.0 * v1 - s.ic1;
    s.ic2 = 2.0 * v2 - s.ic2;
    let norm = (PI * fc * k / fs).sqrt();
    p_rms * k * v1 / norm
}

/// Length of pipe next to a valve that takes part in the exchange with the cylinder.
/// A fixed physical length (not "one cell") keeps the valve flow independent of the grid.
const PORT_LEN: f64 = 0.035;

/// Flow through a poppet valve between a cylinder and the port end of a pipe.
/// `cell` is the pipe's end cell (0 or n-1). Returns the mass that entered the cylinder
/// this step (negative = left it).
#[inline]
fn valve_exchange(cy: &mut Cylinder, pipe: &mut Pipe, cell: usize, cda: f64, dt: f64, noise: f64) -> f64 {
    let m = ((PORT_LEN / pipe.dx).ceil() as usize).clamp(1, pipe.n / 2);
    let (j0, j1) = if cell == 0 { (0, m) } else { (pipe.n - m, pipe.n) };
    let mut vsum = 0.0;
    for j in j0..j1 {
        vsum += pipe.vol[j];
    }
    let w = pipe.prim(cell);
    let t_c = w.p / (w.rho * R_GAS);
    let p_y = cy.p();
    let t_y = cy.t();
    if w.p > p_y {
        let mut dm = orifice_mdot(cda, w.p, t_c, p_y) * dt * (1.0 + noise);
        let dm_eq = (w.p - p_y) / (GAMMA * R_GAS * t_c * (1.0 / vsum + 1.0 / cy.v));
        // each port cell gives up the same fraction of what it holds, so none can be emptied
        let mut msum = 0.0;
        for j in j0..j1 {
            msum += pipe.rho[j] * pipe.vol[j];
        }
        dm = dm.min(0.5 * dm_eq).min(0.3 * msum).max(0.0);
        let f = dm / msum;
        let mut de = 0.0;
        for j in j0..j1 {
            let wj = pipe.prim(j);
            let dej = f * (pipe.ene[j] + wj.p);
            de += dej * pipe.vol[j];
            pipe.rho[j] -= f * pipe.rho[j];
            pipe.mom[j] -= f * pipe.mom[j];
            pipe.ene[j] -= dej;
        }
        cy.m += dm;
        cy.e += de;
        dm
    } else {
        let mut dm = orifice_mdot(cda, p_y, t_y, w.p) * dt * (1.0 + noise);
        let dm_eq = (p_y - w.p) / (GAMMA * R_GAS * t_y * (1.0 / vsum + 1.0 / cy.v));
        dm = dm.min(0.5 * dm_eq).min(0.3 * cy.m).max(0.0);
        let h = CP * t_y;
        let per_vol = dm / vsum;
        for j in j0..j1 {
            pipe.rho[j] += per_vol;
            pipe.ene[j] += per_vol * h;
        }
        cy.m -= dm;
        cy.e -= dm * h;
        -dm
    }
}

/// Flow through a poppet valve between a cylinder and a port volume.
/// Returns the mass that entered the cylinder this step (negative = left it).
#[inline]
fn valve_exchange_vol(cy: &mut Cylinder, port: &mut Volume, cda: f64, dt: f64, head: f64) -> f64 {
    let (p_v, t_v) = (port.p() + head, port.t());
    let (p_y, t_y) = (cy.p(), cy.t());
    if p_v > p_y {
        let mut dm = orifice_mdot(cda, p_v, t_v, p_y) * dt;
        let dm_eq = (p_v - p_y) / (GAMMA * R_GAS * t_v * (1.0 / port.v + 1.0 / cy.v));
        dm = dm.min(0.5 * dm_eq).min(0.3 * port.m).max(0.0);
        let h = CP * t_v;
        port.m -= dm;
        port.e -= dm * h;
        cy.m += dm;
        cy.e += dm * h;
        dm
    } else {
        let mut dm = orifice_mdot(cda, p_y, t_y, p_v) * dt;
        let dm_eq = (p_y - p_v) / (GAMMA * R_GAS * t_y * (1.0 / port.v + 1.0 / cy.v));
        dm = dm.min(0.5 * dm_eq).min(0.3 * cy.m).max(0.0);
        let h = CP * t_y;
        port.m += dm;
        port.e += dm * h;
        cy.m -= dm;
        cy.e -= dm * h;
        -dm
    }
}
