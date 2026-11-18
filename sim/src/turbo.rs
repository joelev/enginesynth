//! Turbochargers: a compressor and a turbine on one shaft, moving gas between the simulator's
//! volumes like the throttle orifices do (see docs/physics.md, "Turbochargers").
//!
//! - Compressor: a speed line in non-dimensional form, isentropic head coefficient against flow
//!   coefficient, as the Moore-Greitzer cubic: rising from the shut-off head at zero flow to its
//!   peak at the surge line, falling past the design point to choke, and carried on through
//!   reverse flow. Pressure ratio follows from the head and the tip speed, so the map scales with
//!   speed as the Casey-Robinson relations do. The flow through it is a state (the Greitzer duct):
//!   d(mdot)/dt = A/L (p_in * PR(mdot) - p_out), so surge oscillation can emerge. Forward flow
//!   gains enthalpy (isentropic head / efficiency), which is the work taken from the shaft.
//! - Turbine: a quasi-steady nozzle (the existing orifice flow) from each turbine inlet volume to
//!   the turbine outlet volume; the gas gives up efficiency x isentropic enthalpy drop, with the
//!   efficiency a parabola in blade speed ratio peaking at 0.7; no reverse flow through the wheel.
//! - Shaft: one inertia; its kinetic energy takes turbine work, gives compressor work and bearing
//!   loss, so lag emerges.
//! - Wastegate: an orifice from turbine inlet to outlet, opened by a spring-and-diaphragm actuator
//!   fed with charge pressure. Blow-off valve: an orifice from the throttle inlet to a vent pipe
//!   open to the air (whose jet noise is the tailpipe's Lighthill term) or back to the compressor
//!   inlet, opened by charge pressure above manifold pressure.
//! - Whistle: a stand-in, not physics (the 1D model has no blades): sound radiated from the turbo,
//!   added to the intake stem at blade-pass frequencies of the simulated shaft speed, its size from
//!   the simulated pressure rise, tip speed and surge margin.

use crate::boost::{orifice_between, Intercooler};
use crate::engine::Volume;
use crate::gas::*;
use std::f64::consts::PI;

/// One turbine inlet (a scroll fed by one or more exhaust collectors).
pub struct Scroll {
    pub x: usize,
    pub area: f64,
    pub mdot: f64,
}

/// Electronic boost control: the ECU bleeds the wastegate actuator's signal (a frequency valve)
/// with a duty cycle, from a PI loop on manifold boost against its target.
pub struct BoostCtrl {
    pub target: f64,
    pub kp: f64,
    pub ki: f64,
    pub integ: f64,
    pub duty: f64,
    pub max_duty: f64,
    /// feed-forward duty: what holds the spring's crack pressure at the target
    pub ff: f64,
}

pub struct Wastegate {
    pub area: f64,
    pub p_crack: f64,
    pub p_span: f64,
    pub tau: f64,
    /// the volume whose pressure feeds the actuator (compressor outlet)
    pub sense: usize,
    pub x: f64,
    pub mdot: f64,
    pub ctrl: Option<BoostCtrl>,
    /// the manifold, for the boost controller
    pub plenum: usize,
}

pub struct Bov {
    /// throttle inlet volume; the vent volume (feeding a short pipe open to the air) and the
    /// compressor inlet (recirculating). `mode` picks one, and can change while running:
    /// 0 none (the valve stays shut), 1 recirculating, 2 venting
    pub from: usize,
    pub vent: usize,
    pub recirc: usize,
    pub mode: u8,
    pub area: f64,
    pub dp_open: f64,
    pub dp_full: f64,
    pub tau: f64,
    pub plenum: usize,
    pub x: f64,
    pub mdot: f64,
}

pub struct Turbo {
    // compressor
    pub u: usize,
    pub d: usize,
    pub d2: f64,
    pub a_duct: f64,
    pub l_duct: f64,
    pub phi_d: f64,
    pub eta_c: f64,
    /// Moore-Greitzer cubic: shut-off head, semi-height H, semi-width W (in flow coefficient)
    pub psi0: f64,
    pub h: f64,
    pub w: f64,
    /// flow coefficient where the speed line's head reaches zero (choke), and the inducer area
    pub phi_c: f64,
    pub a_ind: f64,
    /// design tip speed: the map's flow coefficients shrink below it (see `scale`)
    pub u2_d: f64,
    pub mdot_c: f64,
    // turbine
    pub scrolls: Vec<Scroll>,
    pub y: usize,
    pub d_t: f64,
    pub eta_t: f64,
    pub bsr_w: f64,
    // shaft
    pub inertia: f64,
    pub omega: f64,
    pub c_bear: f64,
    pub wg: Option<Wastegate>,
    pub bov: Option<Bov>,
    pub ic: Option<Intercooler>,
    // whistle stand-in
    pub whistle: f64,
    /// highest whistle partial allowed: 0.45 of the engine's sample rate (not of a substep's) and 20 kHz
    pub f_max: f64,
    pub blades: f64,
    pub splitters: f64,
    wh_phase: [f64; 3],
    wh_rng: Rng,
    wh_lp: f64,
    // last step, for telemetry and checks
    pub pr_c: f64,
    pub phi: f64,
    pub surge_margin: f64,
    pub p_comp: f64,
    pub p_turb: f64,
    pub wh_q: f64,
    // cumulative
    pub w_t: f64,
    pub w_c: f64,
    pub w_b: f64,
    pub m_c: f64,
    pub m_t: f64,
    pub m_wg: f64,
    pub m_bov: f64,
    pub e_wh: f64,
}

impl Turbo {
    /// Flow coefficient at the surge line at the design tip speed (multiply by `scale(u2)` elsewhere).
    pub fn surge_phi(&self) -> f64 {
        2.0 * self.w
    }
    /// How the speed line's flow coefficients shrink at lower tip speed (an estimate).
    #[inline]
    pub fn scale(&self, u2: f64) -> f64 {
        (0.55 + 0.45 * u2 / self.u2_d).clamp(0.55, 1.1)
    }
    /// Isentropic head coefficient on the speed line at flow coefficient phi.
    #[inline]
    pub fn psi(&self, phi: f64) -> f64 {
        if phi <= 2.0 * self.w {
            // left of the surge line, and reverse flow: the Moore-Greitzer cubic
            let x = phi / self.w - 1.0;
            self.psi0 + self.h * (1.0 + 1.5 * x - 0.5 * x * x * x)
        } else {
            // the stable branch: falls from the peak at the surge line to zero head at choke
            let r = (phi - 2.0 * self.w) / (self.phi_c - 2.0 * self.w);
            (self.psi0 + 2.0 * self.h) * (1.0 - r * r)
        }
    }
    /// Compressor efficiency at flow coefficient phi (peak at the design flow, an estimate).
    #[inline]
    pub fn eta(&self, phi: f64) -> f64 {
        let r = (phi - self.phi_d) / self.phi_d;
        (self.eta_c * (1.0 - 0.8 * r * r)).clamp(0.3, self.eta_c)
    }
    /// Tip speed used by the map: the real one, but not below a floor so the map does not
    /// become singular as the shaft stops (the map then reads as a mild flow resistance).
    #[inline]
    fn u2_map(&self) -> f64 {
        (0.5 * self.omega * self.d2).max(40.0)
    }
    /// Pressure ratio on the speed line for mass flow m at inlet density rho and temperature t.
    #[inline]
    pub fn pr_at(&self, m: f64, rho: f64, t: f64) -> f64 {
        let u2 = self.u2_map();
        let phi = m / (rho * u2 * self.d2 * self.d2) / self.scale(u2);
        if phi > self.phi_c {
            // past choke the wheel adds no head; the passage is a loss of about one dynamic head
            // of the flow above the choke flow (a windmilling wheel is a mild restriction, not a wall)
            let v = m / (rho * self.a_ind);
            let vc = self.phi_c * self.scale(u2) * u2 * self.d2 * self.d2 / self.a_ind;
            return (1.0 - 0.5 * rho * (v * v - vc * vc) / (rho * R_GAS * t)).max(0.3);
        }
        let b = 1.0 + self.psi(phi) * u2 * u2 / (CP * t);
        let b2 = b * b;
        b2 * b2
    }

    pub fn step(&mut self, vols: &mut [Volume], dt: f64) {
        // ---- compressor: Greitzer duct, semi-implicit in its own flow and the two volumes' answer to it
        let (pu, tu, rho_u, vu) = (vols[self.u].p(), vols[self.u].t(), vols[self.u].rho(), vols[self.u].v);
        let (pd, td, vd) = (vols[self.d].p(), vols[self.d].t(), vols[self.d].v);
        let k = self.a_duct / self.l_duct;
        let m0 = self.mdot_c;
        let stiff = GAMMA * R_GAS * (tu / vu + td / vd) * dt;
        let f = |m: f64| k * (pu * self.pr_at(m, rho_u, tu) - pd - stiff * (m - m0));
        let dm_fd = 1e-4 + 1e-3 * m0.abs();
        let f0 = f(m0);
        let df = (f(m0 + dm_fd) - f0) / dm_fd;
        let mut m1 = m0 + dt * f0 / (1.0 - dt * df).max(0.2);
        // never more than a share of what the donor holds in one step
        let cap = 0.3 * if m1 >= 0.0 { vols[self.u].m } else { vols[self.d].m } / dt;
        m1 = m1.clamp(-cap, cap);
        self.mdot_c = m1;
        let dm = m1 * dt;
        let u2 = 0.5 * self.omega * self.d2;
        let u2m = self.u2_map();
        let phi = m1 / (rho_u * u2m * self.d2 * self.d2) / self.scale(u2m);
        self.phi = phi;
        self.pr_c = self.pr_at(m1, rho_u, tu);
        self.surge_margin = phi / self.surge_phi() - 1.0;
        let w_c;
        if dm >= 0.0 {
            // forward: the head the wheel puts in at this flow, over its efficiency
            let dh = self.psi(phi).max(0.02) * u2 * u2 / self.eta(phi);
            let h = CP * tu;
            vols[self.u].dm -= dm;
            vols[self.u].de -= dm * h;
            vols[self.d].dm += dm;
            vols[self.d].de += dm * (h + dh);
            w_c = dm * dh;
        } else {
            // reverse flow (deep surge): the outlet's gas goes back, churned by the wheel
            let dh = 0.5 * u2 * u2;
            let h = CP * td;
            vols[self.d].dm += dm;
            vols[self.d].de += dm * h;
            vols[self.u].dm -= dm;
            vols[self.u].de -= dm * (h + dh);
            w_c = -dm * dh;
        }
        self.m_c += dm;

        // ---- turbine scrolls: quasi-steady nozzles, no reverse flow through the wheel
        let mut w_t = 0.0;
        let (py, vy) = (vols[self.y].p(), vols[self.y].v);
        let u_t = 0.5 * self.omega * self.d_t;
        for s in self.scrolls.iter_mut() {
            let (px, tx, vx, mx) = (vols[s.x].p(), vols[s.x].t(), vols[s.x].v, vols[s.x].m);
            s.mdot = 0.0;
            if px <= py {
                continue;
            }
            let mut m = orifice_mdot(s.area, px, tx, py) * dt;
            let m_eq = (px - py) / (GAMMA * R_GAS * tx * (1.0 / vx + 1.0 / vy));
            m = m.min(0.5 * m_eq).min(0.3 * mx).max(0.0);
            let dhs = CP * tx * (1.0 - pow_gm1_g(py / px));
            let eta = if dhs > 1.0 {
                let bsr = u_t / (2.0 * dhs).sqrt();
                let r = (bsr - 0.7) / self.bsr_w;
                (self.eta_t * (1.0 - r * r)).max(0.0)
            } else {
                0.0
            };
            let w = m * eta * dhs;
            vols[s.x].dm -= m;
            vols[s.x].de -= m * CP * tx;
            vols[self.y].dm += m;
            vols[self.y].de += m * CP * tx - w;
            w_t += w;
            s.mdot = m / dt;
            self.m_t += m;
        }

        // ---- wastegate: spring and diaphragm fed with charge pressure
        if let Some(wg) = self.wg.as_mut() {
            let mut boost = vols[wg.sense].p() - P_ATM;
            if let Some(c) = wg.ctrl.as_mut() {
                // the ECU holds the gate shut below target by bleeding the actuator's signal
                // feed-forward plus PI; the integral only works near the target (no wind-up while spooling)
                let b = vols[wg.plenum].p() - P_ATM;
                let err = (c.target - b) / 1e5;
                if b > 0.6 * c.target {
                    c.integ = (c.integ + c.ki * err * dt).clamp(-0.2, 0.2);
                }
                c.duty = (c.ff + c.kp * err.clamp(-0.3, 0.3) + c.integ).clamp(0.0, c.max_duty);
                boost *= 1.0 - c.duty;
            }
            let target = ((boost - wg.p_crack) / wg.p_span).clamp(0.0, 1.0);
            wg.x += (target - wg.x) * (dt / wg.tau).min(1.0);
            wg.mdot = 0.0;
            if wg.x > 1e-4 {
                for s in &self.scrolls {
                    let (m, _) = orifice_between(vols, s.x, self.y, wg.area * wg.x / self.scrolls.len() as f64, dt, None);
                    wg.mdot += m;
                }
                self.m_wg += wg.mdot * dt;
            }
        }
        // ---- blow-off valve: charge pressure above manifold pressure lifts it
        if let Some(b) = self.bov.as_mut() {
            let dp = vols[b.from].p() - vols[b.plenum].p();
            let target = if b.mode == 0 { 0.0 } else { ((dp - b.dp_open) / (b.dp_full - b.dp_open)).clamp(0.0, 1.0) };
            b.x += (target - b.x) * (dt / b.tau).min(1.0);
            b.mdot = 0.0;
            if b.x > 1e-4 {
                let to = if b.mode == 2 { b.vent } else { b.recirc };
                b.mdot = orifice_between(vols, b.from, to, 0.8 * b.area * b.x, dt, None).0;
                self.m_bov += b.mdot * dt;
            }
        }
        if let Some(ic) = self.ic.as_mut() {
            let (m, q) = orifice_between(vols, ic.from, ic.to, ic.area, dt, Some((ic.eff, ic.t_cool)));
            ic.mdot = m;
            ic.q_out += q;
        }

        // ---- whistle stand-in (not physics): blade-pass partials of the simulated shaft speed,
        // radiated as sound from the turbo (pascals at 1 m, added to the intake stem). An earlier
        // version injected it as flow into the compressor inlet volume; the airbox's compliance
        // took it 80 to 90 dB under the exhaust at 10 kHz, inaudible at any sane gain.
        self.wh_q = 0.0;
        if self.whistle > 0.0 && self.omega > 100.0 && m1 > 0.0 {
            let f_rot = self.omega / (2.0 * PI);
            // loudness grows with pressure rise and tip speed, and near surge (tip-clearance flow)
            let near_surge = (1.0 - self.surge_margin / 0.3).clamp(0.0, 1.0);
            // 1 Pa at 1 m for gain 1 at a pressure ratio of 2 and 400 m/s tip speed (a reference
            // level set by ear)
            let tip = u2 / 400.0;
            let amp = self.whistle * 1.0 * (self.pr_c - 1.0).max(0.0) * tip * tip * (1.0 + near_surge);
            let freqs = [self.blades * f_rot, (self.blades + self.splitters) * f_rot, 2.0 * (self.blades + self.splitters) * f_rot];
            let gains = [1.0, 0.6, 0.25];
            let mut q = 0.0;
            for i in 0..3 {
                self.wh_phase[i] = (self.wh_phase[i] + 2.0 * PI * freqs[i] * dt) % (2.0 * PI);
                if freqs[i] < self.f_max {
                    q += gains[i] * self.wh_phase[i].sin();
                }
            }
            // a breath of band noise near surge (tip-clearance "whoosh")
            self.wh_lp += 0.2 * (self.wh_rng.noise() - self.wh_lp);
            q += 0.5 * near_surge * self.wh_lp;
            self.wh_q = amp * q;
        }

        // ---- shaft: kinetic energy in, out and lost in the bearings
        let w_b = self.c_bear * self.omega * self.omega * dt;
        let e = (0.5 * self.inertia * self.omega * self.omega + w_t - w_c - w_b).max(0.0);
        self.omega = (2.0 * e / self.inertia).sqrt();
        self.p_comp = w_c / dt;
        self.p_turb = w_t / dt;
        self.w_t += w_t;
        self.w_c += w_c;
        self.w_b += w_b;
    }

    pub fn new_whistle_rng(seed: u64) -> Rng {
        Rng((seed ^ 0x7B0_5EED_0F_F1CE) | 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(u: usize, d: usize, scrolls: Vec<Scroll>, y: usize, t: &crate::spec::TurboSpec, seed: u64) -> Turbo {
        // Moore-Greitzer cubic through the shut-off head and the peak at the surge line; the design
        // flow sits at 1.25 times the surge flow
        // surge line at phi_d / surge_ratio: the cubic's peak sits there (2W)
        let w = 0.5 * t.phi_d / t.surge_ratio;
        let h = 0.5 * (t.psi_peak - t.psi_shut);
        let omega_ref = {
            // bearing loss is given at the design speed: tip speed for the design head at the boost target is not known here,
            // so take a typical design tip speed of 350 m/s (estimate)
            2.0 * 350.0 / t.d2
        };
        Turbo {
            u,
            d,
            d2: t.d2,
            a_duct: PI / 4.0 * t.duct_d * t.duct_d,
            l_duct: t.duct_len,
            phi_d: t.phi_d,
            eta_c: t.eta_c,
            psi0: t.psi_shut,
            h,
            w,
            phi_c: t.phi_d * t.choke_ratio,
            a_ind: PI / 4.0 * (0.7 * t.d2) * (0.7 * t.d2),
            u2_d: t.tip_d,
            mdot_c: 0.0,
            scrolls,
            y,
            d_t: t.turbine_d,
            eta_t: t.eta_t,
            bsr_w: t.bsr_width,
            inertia: t.inertia,
            omega: 0.15 * omega_ref,
            c_bear: t.bearing_w / (omega_ref * omega_ref),
            wg: None,
            bov: None,
            ic: None,
            whistle: t.whistle,
            f_max: 20_000.0,
            blades: t.blades as f64,
            splitters: t.splitters as f64,
            wh_phase: [0.0; 3],
            wh_rng: Self::new_whistle_rng(seed),
            wh_lp: 0.0,
            pr_c: 1.0,
            phi: 0.0,
            surge_margin: 0.0,
            p_comp: 0.0,
            p_turb: 0.0,
            wh_q: 0.0,
            w_t: 0.0,
            w_c: 0.0,
            w_b: 0.0,
            m_c: 0.0,
            m_t: 0.0,
            m_wg: 0.0,
            m_bov: 0.0,
            e_wh: 0.0,
        }
    }

    pub fn shaft_rpm(&self) -> f64 {
        self.omega * 30.0 / PI
    }
}
