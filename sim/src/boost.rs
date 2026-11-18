//! Forced induction: positive-displacement superchargers driven from the crank.
//!
//! A blower is a set of rotor pockets that carry gas from an inlet volume U to an outlet volume D.
//! Each pocket has its own gas state and a volume that follows its rotor angle:
//!
//! - filling: open to U while its volume grows from the small carry-back volume to the full
//!   pocket (the lobes unmesh at the inlet);
//! - sealed: carried round the housing; a Roots pocket keeps its volume (no internal
//!   compression), a twin-screw pocket is squeezed to its built-in volume ratio;
//! - opening: the leading lobe crosses the outlet port edge, so the area to D ramps open over
//!   the helix twist; D's gas rushes back into the pocket, which is still at inlet pressure
//!   (the backflow that Eaton's patents describe as the source of the whine);
//! - discharge: the lobes mesh and squeeze the pocket into D, down to the carry-back volume,
//!   whose gas then expands back into U when the pocket starts again.
//!
//! Nothing here is a tone: the outlet and inlet pulses come from the pocket gas exchange, at
//! pockets x rotor speed, with a size set by the pressure ratio. The drive work is the work the
//! pockets' volume changes do on their gas, so the isochoric (Delta p V) loss of a Roots blower
//! appears without being put in. Clearance leakage is one orifice from D to U. A bypass valve
//! (an orifice from D to U opened by manifold vacuum) and an intercooler (an orifice with a pressure
//! loss and a set effectiveness) complete the installation. See docs/physics.md, "Superchargers".

use crate::engine::Volume;
use crate::gas::*;
use std::f64::consts::PI;

const TAU: f64 = 2.0 * PI;
/// Discharge coefficient of the pocket ports and the bypass valve.
const PORT_CD: f64 = 0.8;

#[inline]
fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[derive(Clone, Copy, Default)]
pub struct Pocket {
    pub m: f64,
    pub e: f64,
    pub v: f64,
}

impl Pocket {
    #[inline]
    pub fn p(&self) -> f64 {
        GM1 * self.e / self.v
    }
    #[inline]
    pub fn t(&self) -> f64 {
        self.e / (self.m * CV)
    }
}

/// Valve from the blower outlet back to its inlet, opened by manifold vacuum through a diaphragm.
pub struct Bypass {
    pub area: f64,
    /// manifold pressure (Pa, absolute) at or below which it is fully open, and at or above which it is shut
    pub p_open: f64,
    pub p_shut: f64,
    pub tau: f64,
    /// the volume whose pressure the actuator senses
    pub sense: usize,
    /// opening 0..1 and last flow D -> U, kg/s
    pub x: f64,
    pub mdot: f64,
}

/// Charge-air cooler: an orifice-like pressure loss from `from` to `to`, and cooling of the gas
/// passing forward by `eff` of its temperature above `t_cool`.
pub struct Intercooler {
    pub from: usize,
    pub to: usize,
    pub area: f64,
    pub eff: f64,
    pub t_cool: f64,
    pub mdot: f64,
    /// heat taken out, J (cumulative)
    pub q_out: f64,
}

pub struct Blower {
    /// inlet and outlet volumes
    pub u: usize,
    pub d: usize,
    /// rotor speed / crank speed
    pub ratio: f64,
    /// rotor-angle offset of each pocket
    pub offs: Vec<f64>,
    pub pockets: Vec<Pocket>,
    /// swept volume per pocket, the carry-back volume, and the volume a sealed pocket is
    /// squeezed to before it opens (v_p / vi; equal to v_p for a Roots blower)
    pub v_p: f64,
    pub v_min: f64,
    pub v_top: f64,
    /// pocket cycle, rotor radians: filling ends, outlet starts to open, discharge starts
    f_end: f64,
    i_close: f64,
    o_start: f64,
    ramp: f64,
    d_start: f64,
    in_ramp: f64,
    pub a_port: f64,
    pub a_leak: f64,
    pub mech_eff: f64,
    /// bearing, seal and gear drag at the rotor shaft, N m
    pub drag: f64,
    pub bypass: Option<Bypass>,
    pub ic: Option<Intercooler>,
    // last step
    pub power: f64,
    pub mdot_leak: f64,
    // cumulative, for the checks
    pub m_in: f64,
    pub m_out: f64,
    pub m_leak: f64,
    pub w_gas: f64,
    pub w_crank: f64,
    pub revs: f64,
}

/// Exchange gas between a pocket and a volume through an opening of effective area `cda`.
/// Returns the mass that entered the pocket (negative: it left).
#[inline]
fn exchange(pk: &mut Pocket, vol: &mut Volume, cda: f64, dt: f64) -> f64 {
    let (pp, tp) = (pk.p(), pk.t());
    let (pv, tv) = (vol.p(), vol.t());
    if pv > pp {
        let mut dm = orifice_mdot(cda, pv, tv, pp) * dt;
        let dm_eq = (pv - pp) / (GAMMA * R_GAS * tv * (1.0 / vol.v + 1.0 / pk.v));
        dm = dm.min(0.5 * dm_eq).min(0.3 * vol.m).max(0.0);
        let h = CP * tv;
        pk.m += dm;
        pk.e += dm * h;
        vol.dm -= dm;
        vol.de -= dm * h;
        dm
    } else {
        let mut dm = orifice_mdot(cda, pp, tp, pv) * dt;
        let dm_eq = (pp - pv) / (GAMMA * R_GAS * tp * (1.0 / vol.v + 1.0 / pk.v));
        dm = dm.min(0.5 * dm_eq).min(0.3 * pk.m).max(0.0);
        let h = CP * tp;
        pk.m -= dm;
        pk.e -= dm * h;
        vol.dm += dm;
        vol.de += dm * h;
        -dm
    }
}

/// Orifice flow between two volumes (as the throttle orifices do it). `cool` takes `eff` of the
/// forward-flowing gas's temperature above `t_cool` out on the way (an intercooler). Returns the
/// mass flow a -> b (kg/s) and the heat taken out this step (J).
#[inline]
pub fn orifice_between(vols: &mut [Volume], a: usize, b: usize, cda: f64, dt: f64, cool: Option<(f64, f64)>) -> (f64, f64) {
    let (pa, ta, va) = (vols[a].p(), vols[a].t(), vols[a].v);
    let (pb, tb, vb) = (vols[b].p(), vols[b].t(), vols[b].v);
    let (pu, tu, pd, sign) = if pa >= pb { (pa, ta, pb, 1.0) } else { (pb, tb, pa, -1.0) };
    let mut dm = orifice_mdot(cda, pu, tu, pd) * dt;
    let dm_eq = (pu - pd) / (GAMMA * R_GAS * tu * (1.0 / va + 1.0 / vb));
    dm = dm.min(0.5 * dm_eq);
    let h = CP * tu;
    let dm_ab = sign * dm;
    let mut q = 0.0;
    let h_in = match cool {
        Some((eff, t_cool)) if sign > 0.0 => {
            let h_c = CP * (tu - eff * (tu - t_cool).max(0.0));
            q = dm * (h - h_c);
            h_c
        }
        _ => h,
    };
    vols[a].dm -= dm_ab;
    vols[a].de -= if sign > 0.0 { dm * h } else { -dm * h_in };
    vols[b].dm += dm_ab;
    vols[b].de += if sign > 0.0 { dm * h_in } else { -dm * h };
    (dm_ab / dt, q)
}

pub struct BlowerGeom {
    pub kind_screw: bool,
    pub displacement: f64,
    pub ratio: f64,
    pub rotors: usize,
    pub lobes: usize,
    pub helix_deg: f64,
    pub vi: f64,
    pub carry: f64,
    pub a_port: f64,
    pub a_leak: f64,
    pub mech_eff: f64,
    pub drag: f64,
}

impl Blower {
    pub fn new(u: usize, d: usize, g: &BlowerGeom) -> Blower {
        let lobes = g.lobes.max(2);
        // A Roots blower's pockets come from both rotors, half a lobe pitch apart, so the outlet
        // sees rotors x lobes pockets per revolution. A twin-screw's pockets are counted on the
        // male rotor (its lobes), which `ratio` refers to.
        let rotors = if g.kind_screw { 1 } else { g.rotors.max(1) };
        let n = rotors * lobes;
        let pitch = TAU / lobes as f64;
        let mut offs = Vec::with_capacity(n);
        for r in 0..rotors {
            for k in 0..lobes {
                offs.push(k as f64 * pitch + r as f64 * pitch / rotors as f64);
            }
        }
        let v_p = g.displacement / n as f64;
        let v_min = g.carry * v_p;
        let vi = if g.kind_screw { g.vi.max(1.0) } else { 1.0 };
        // Filling over one lobe pitch, with the inlet port shutting over the next quarter pitch
        // (after the pocket has stopped growing, so it is not drawn below inlet pressure);
        // discharge over the last lobe pitch; the outlet opening over the helix twist (at least
        // 15 degrees, and for a twin-screw leaving at least half the sealed stretch to compress in).
        let f_end = pitch;
        let i_close = f_end + 0.25 * pitch;
        let d_start = TAU - pitch;
        let room = if g.kind_screw { 0.5 } else { 1.0 } * (d_start - i_close);
        let ramp = (g.helix_deg.max(15.0) * PI / 180.0).min(room);
        let o_start = d_start - ramp;
        let mut b = Blower {
            u,
            d,
            ratio: g.ratio,
            offs,
            pockets: vec![Pocket::default(); n],
            v_p,
            v_min,
            v_top: v_p / vi,
            f_end,
            i_close,
            o_start,
            ramp,
            d_start,
            in_ramp: 0.1 * pitch,
            a_port: g.a_port,
            a_leak: g.a_leak,
            mech_eff: g.mech_eff,
            drag: g.drag,
            bypass: None,
            ic: None,
            power: 0.0,
            mdot_leak: 0.0,
            m_in: 0.0,
            m_out: 0.0,
            m_leak: 0.0,
            w_gas: 0.0,
            w_crank: 0.0,
            revs: 0.0,
        };
        for j in 0..n {
            let v = b.volume_at(b.offs[j]);
            let m = RHO_ATM * v;
            b.pockets[j] = Pocket { m, e: m * CV * T_ATM, v };
        }
        b
    }

    pub fn pockets_per_rev(&self) -> usize {
        self.offs.len()
    }

    /// Pocket volume at pocket angle phi (0..2 pi).
    #[inline]
    pub fn volume_at(&self, phi: f64) -> f64 {
        if phi < self.f_end {
            self.v_min + self.v_p * smooth(phi / self.f_end)
        } else if phi < self.i_close {
            self.v_min + self.v_p
        } else if phi < self.o_start {
            // sealed: a twin-screw squeezes the pocket to v_p / vi on the way round
            self.v_min + self.v_p - (self.v_p - self.v_top) * smooth((phi - self.i_close) / (self.o_start - self.i_close))
        } else if phi < self.d_start {
            self.v_min + self.v_top
        } else {
            self.v_min + self.v_top * (1.0 - smooth((phi - self.d_start) / (TAU - self.d_start)))
        }
    }

    /// Open fraction of the inlet and outlet ports at pocket angle phi.
    #[inline]
    pub fn ports_at(&self, phi: f64) -> (f64, f64) {
        let a_in = if phi < self.i_close { smooth(phi / self.in_ramp) * (1.0 - smooth((phi - self.f_end) / (self.i_close - self.f_end))) } else { 0.0 };
        let a_out = if phi > self.o_start { smooth((phi - self.o_start) / self.ramp) } else { 0.0 };
        (a_in, a_out)
    }

    /// One step. `theta` and `dth` are the crank angle at the start of the step and its change.
    /// Gas exchange uses the volumes' state at the start of the step and goes into their
    /// accumulators, like the orifices. Returns the work taken from the crank (J).
    pub fn step(&mut self, vols: &mut [Volume], theta: f64, dth: f64, dt: f64) -> f64 {
        let psi0 = (self.ratio * theta).rem_euclid(TAU);
        let dpsi = self.ratio * dth;
        let (u, d) = (self.u, self.d);
        let mut w_gas = 0.0;
        for j in 0..self.pockets.len() {
            let phi0 = (psi0 + self.offs[j]).rem_euclid(TAU);
            let phim = (phi0 + 0.5 * dpsi).rem_euclid(TAU);
            let phi1 = (phi0 + dpsi).rem_euclid(TAU);
            let (fi, fo) = self.ports_at(phim);
            let v1 = self.volume_at(phi1);
            let pk = &mut self.pockets[j];
            if fi > 0.0 {
                let dm = exchange(pk, &mut vols[u], PORT_CD * self.a_port * fi, dt);
                self.m_in += dm;
            }
            if fo > 0.0 {
                let dm = exchange(pk, &mut vols[d], PORT_CD * self.a_port * fo, dt);
                self.m_out -= dm;
            }
            // the rotors change the pocket's volume: adiabatic work on its gas
            let e0 = pk.e;
            pk.e *= pow_gm1(pk.v / v1);
            pk.v = v1;
            w_gas += pk.e - e0;
            if pk.e < 1e-9 {
                pk.e = 1e-9;
            }
            if pk.m < 1e-12 {
                pk.m = 1e-12;
            }
        }
        // clearance leakage, outlet back to inlet
        let (ml, _) = orifice_between(vols, d, u, self.a_leak, dt, None);
        self.mdot_leak = ml;
        self.m_leak += ml * dt;
        if let Some(bp) = self.bypass.as_mut() {
            let ps = vols[bp.sense].p();
            let target = ((bp.p_shut - ps) / (bp.p_shut - bp.p_open)).clamp(0.0, 1.0);
            bp.x += (target - bp.x) * (dt / bp.tau).min(1.0);
            bp.mdot = if bp.x > 1e-4 { orifice_between(vols, d, u, PORT_CD * bp.area * bp.x, dt, None).0 } else { 0.0 };
        }
        if let Some(ic) = self.ic.as_mut() {
            let (m, q) = orifice_between(vols, ic.from, ic.to, ic.area, dt, Some((ic.eff, ic.t_cool)));
            ic.mdot = m;
            ic.q_out += q;
        }
        let w_crank = if w_gas > 0.0 { w_gas / self.mech_eff } else { w_gas * self.mech_eff } + self.drag * dpsi.abs();
        self.w_gas += w_gas;
        self.w_crank += w_crank;
        self.revs += dpsi / TAU;
        self.power = w_crank / dt;
        w_crank
    }

    /// Total mass and internal energy held in the pockets.
    pub fn held(&self) -> (f64, f64) {
        self.pockets.iter().fold((0.0, 0.0), |(m, e), p| (m + p.m, e + p.e))
    }
}
