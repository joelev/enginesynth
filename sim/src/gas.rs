//! Ideal-gas constants, the HLLC Riemann solver, boundary states and orifice flow.

/// Ratio of specific heats. 4/3 sits between cold air (1.40) and hot burned gas (about 1.30),
/// and it makes every isentropic exponent a whole number or a power of two, so the relations
/// below need only multiplications and square roots (no pow calls, which are slow in WebAssembly).
pub const GAMMA: f64 = 4.0 / 3.0;
pub const GM1: f64 = GAMMA - 1.0;
pub const R_GAS: f64 = 287.0;
pub const CV: f64 = R_GAS / GM1;
pub const CP: f64 = GAMMA * CV;
pub const P_ATM: f64 = 101_325.0;
pub const T_ATM: f64 = 293.15;
pub const RHO_ATM: f64 = P_ATM / (R_GAS * T_ATM);
pub const C_ATM: f64 = 343.0;

// Isentropic powers for GAMMA = 4/3. The names give the exponent in terms of gamma.
/// x^(1/gamma) = x^(3/4)
#[inline(always)]
pub fn pow_1_g(x: f64) -> f64 {
    let s = x.sqrt();
    s * s.sqrt()
}
/// x^(2/(gamma-1)) = x^6
#[inline(always)]
pub fn pow_2_gm1(x: f64) -> f64 {
    let x2 = x * x;
    x2 * x2 * x2
}
/// x^(2 gamma/(gamma-1)) = x^8
#[inline(always)]
pub fn pow_2g_gm1(x: f64) -> f64 {
    let x2 = x * x;
    let x4 = x2 * x2;
    x4 * x4
}
/// x^((gamma-1)/(2 gamma)) = x^(1/8)
#[inline(always)]
pub fn pow_gm1_2g(x: f64) -> f64 {
    x.sqrt().sqrt().sqrt()
}
/// x^(1/(gamma-1)) = x^3
#[inline(always)]
pub fn pow_1_gm1(x: f64) -> f64 {
    x * x * x
}
/// x^(gamma/(gamma-1)) = x^4
#[inline(always)]
pub fn pow_g_gm1(x: f64) -> f64 {
    let x2 = x * x;
    x2 * x2
}
/// x^((gamma-1)/gamma) = x^(1/4)
#[inline(always)]
pub fn pow_gm1_g(x: f64) -> f64 {
    x.sqrt().sqrt()
}
/// x^(gamma-1) = x^(1/3)
#[inline(always)]
pub fn pow_gm1(x: f64) -> f64 {
    x.cbrt()
}
/// x^gamma = x^(4/3)
#[inline(always)]
pub fn pow_g(x: f64) -> f64 {
    x * x.cbrt()
}

/// Cube root for x > 0 without libm (experiment `+m`): a bit-level first guess and three
/// Halley steps. Within a few ulp of `cbrt`; in WebAssembly the libm cube root uses a software
/// fused multiply-add and costs several times more.
#[inline(always)]
pub fn cbrt_fast(x: f64) -> f64 {
    let mut y = f64::from_bits(x.to_bits() / 3 + 0x2A9F_7893_782D_A1CE);
    for _ in 0..3 {
        let y3 = y * y * y;
        y *= (y3 + 2.0 * x) / (2.0 * y3 + x);
    }
    y
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Prim {
    pub rho: f64,
    pub u: f64,
    pub p: f64,
}

impl Prim {
    #[inline]
    pub fn a(&self) -> f64 {
        (GAMMA * self.p / self.rho).sqrt()
    }
    #[inline]
    pub fn energy(&self) -> f64 {
        self.p / GM1 + 0.5 * self.rho * self.u * self.u
    }
    #[inline]
    pub fn flux(&self) -> [f64; 3] {
        let e = self.energy();
        [
            self.rho * self.u,
            self.rho * self.u * self.u + self.p,
            self.u * (e + self.p),
        ]
    }
}

/// HLLC approximate Riemann flux (Toro), pressure-based wave speed estimates.
#[inline]
pub fn hllc(l: Prim, r: Prim) -> [f64; 3] {
    let al = l.a();
    let ar = r.a();
    let p_pvrs = 0.5 * (l.p + r.p) - 0.5 * (r.u - l.u) * 0.25 * (l.rho + r.rho) * (al + ar);
    let ps = p_pvrs.max(0.0);
    let k = (GAMMA + 1.0) / (2.0 * GAMMA);
    let ql = if ps <= l.p { 1.0 } else { (1.0 + k * (ps / l.p - 1.0)).sqrt() };
    let qr = if ps <= r.p { 1.0 } else { (1.0 + k * (ps / r.p - 1.0)).sqrt() };
    let sl = l.u - al * ql;
    let sr = r.u + ar * qr;
    if sl >= 0.0 {
        return l.flux();
    }
    if sr <= 0.0 {
        return r.flux();
    }
    let ss = (r.p - l.p + l.rho * l.u * (sl - l.u) - r.rho * r.u * (sr - r.u))
        / (l.rho * (sl - l.u) - r.rho * (sr - r.u));
    if ss >= 0.0 {
        let f = l.flux();
        let e = l.energy();
        let c = l.rho * (sl - l.u) / (sl - ss);
        let us = [c, c * ss, c * (e / l.rho + (ss - l.u) * (ss + l.p / (l.rho * (sl - l.u))))];
        [
            f[0] + sl * (us[0] - l.rho),
            f[1] + sl * (us[1] - l.rho * l.u),
            f[2] + sl * (us[2] - e),
        ]
    } else {
        let f = r.flux();
        let e = r.energy();
        let c = r.rho * (sr - r.u) / (sr - ss);
        let us = [c, c * ss, c * (e / r.rho + (ss - r.u) * (ss + r.p / (r.rho * (sr - r.u))))];
        [
            f[0] + sr * (us[0] - r.rho),
            f[1] + sr * (us[1] - r.rho * r.u),
            f[2] + sr * (us[2] - e),
        ]
    }
}

/// State on the end face of a pipe that opens into a reservoir at rest (p0, rho0).
/// `i` is the interior cell with `u` measured along the outward normal; the returned
/// state uses the same convention. Outflow: face pressure equals the reservoir pressure
/// (pressure release, so waves reflect inverted). Inflow: isentropic from the reservoir.
#[inline]
pub fn reservoir_face(i: Prim, p0: f64, rho0: f64) -> Prim {
    let ai = i.a();
    if i.u >= ai {
        return i; // supersonic outflow
    }
    let jp = i.u + 2.0 * ai / GM1; // outgoing Riemann invariant
    // try outflow
    let rho_f = i.rho * pow_1_g(p0 / i.p);
    let a_f = (GAMMA * p0 / rho_f).sqrt();
    let u_f = jp - 2.0 * a_f / GM1;
    if u_f >= 0.0 {
        if u_f > a_f {
            // choked: sonic at the exit
            let a_s = jp * GM1 / (GAMMA + 1.0);
            let r = a_s / ai;
            return Prim {
                rho: i.rho * pow_2_gm1(r),
                u: a_s,
                p: i.p * pow_2g_gm1(r),
            };
        }
        return Prim { rho: rho_f, u: u_f, p: p0 };
    }
    // Inflow. The outgoing wave ties face velocity to face pressure through the interior gas;
    // the entering gas obeys the reservoir's energy equation. With s = sqrt(1 - (p*/p0)^(2 beta))
    // the matching condition is smooth and monotone in s, and s = 0 is exactly the zero-flow
    // limit of the outflow branch, so the boundary has no dead band around flow reversal.
    let a0 = (GAMMA * p0 / rho0).sqrt();
    let kk = (2.0 / GM1).sqrt();
    let c1 = 2.0 * ai / GM1;
    let c2 = c1 * pow_gm1_2g(p0 / i.p);
    let s_max = (GM1 / (GAMMA + 1.0)).sqrt();
    let mut s = (-u_f / (kk * a0)).min(s_max);
    for _ in 0..6 {
        let root = (1.0 - s * s).sqrt();
        let h = i.u + c1 - c2 * root + kk * a0 * s;
        let dh = c2 * s / root + kk * a0;
        let ds = h / dh;
        s = (s - ds).clamp(0.0, s_max);
        if ds.abs() < 1e-12 {
            break;
        }
    }
    let w = 1.0 - s * s;
    Prim {
        rho: rho0 * pow_1_gm1(w),
        u: -kk * a0 * s,
        p: p0 * pow_g_gm1(w),
    }
}

/// Compressible orifice mass flow (kg/s) from upstream (p_up, t_up) to p_dn, with choking.
#[inline]
pub fn orifice_mdot(cda: f64, p_up: f64, t_up: f64, p_dn: f64) -> f64 {
    const PR_LIN: f64 = 0.99;
    let pr = (p_dn / p_up).clamp(0.0, 1.0);
    let pc = pow_g_gm1(2.0 / (GAMMA + 1.0));
    // x^(2/gamma) = x^1.5 and x^((gamma+1)/gamma) = x^1.75
    let psi = |x: f64| {
        let s = x.sqrt();
        (2.0 * GAMMA / GM1 * (x * s - x * s * s.sqrt())).max(0.0).sqrt()
    };
    let f = if pr <= pc {
        psi(pc)
    } else if pr > PR_LIN {
        psi(PR_LIN) * (1.0 - pr) / (1.0 - PR_LIN)
    } else {
        psi(pr)
    };
    cda * p_up / (R_GAS * t_up).sqrt() * f
}

/// Small deterministic RNG (xorshift64*).
pub struct Rng(pub u64);
impl Rng {
    #[inline]
    pub fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        ((self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64) / ((1u64 << 53) as f64)
    }
    /// Roughly unit-variance, zero-mean.
    #[inline]
    pub fn noise(&mut self) -> f64 {
        (self.uniform() + self.uniform() + self.uniform() + self.uniform() - 2.0) * 1.732
    }
}
