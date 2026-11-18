//! Section timers and counters for profiling, compiled in only with `--features prof`.
//! Without the feature every call here is an empty inline function, so the default build and the
//! WebAssembly build carry no trace of it. With it, timers read the CPU's virtual counter
//! directly (one instruction on aarch64), cheap enough to wrap per-cylinder sections.
//! Single-threaded use only.

/// Timer and counter slots. Nested timers (RAD inside GRID_BC or WAVE, SNAPSHOT inside STROBE)
/// are "of which" figures and must not be added to their parent.
pub mod cat {
    pub const GRID_PRIM: usize = 0;
    pub const GRID_RECON: usize = 1;
    pub const GRID_FLUX: usize = 2;
    pub const GRID_BC: usize = 3;
    pub const GRID_UPDATE: usize = 4;
    pub const GRID_SMAX: usize = 5;
    pub const WAVE: usize = 6;
    pub const PIPE_OTHER: usize = 7;
    pub const ORIF: usize = 8;
    pub const CYL_VALVE: usize = 9;
    pub const CYL_PISTON_BURN: usize = 10;
    pub const CYL_HEAT: usize = 11;
    pub const AFTERFIRE: usize = 12;
    pub const VOL_CRANK: usize = 13;
    pub const CONTROLS: usize = 14;
    pub const MICS: usize = 15;
    pub const BLOCK: usize = 16;
    pub const STROBE: usize = 17;
    pub const DECIM: usize = 18;
    pub const OUTPUT: usize = 19;
    /// nested
    pub const RAD: usize = 20;
    pub const SNAPSHOT: usize = 21;
    pub const WAVE_JUNCTION: usize = 22;
    pub const N: usize = 24;
    pub const NAMES: [&str; N] = [
        "grid: primitives",
        "grid: reconstruction",
        "grid: HLLC fluxes",
        "grid: boundary faces",
        "grid: update+friction+heat",
        "grid: max wave speed",
        "wave pipes",
        "pipe loop bookkeeping",
        "orifices (throttles)",
        "cyl: valves/ports",
        "cyl: piston+burn",
        "cyl: Woschni heat",
        "afterfire",
        "volumes+crank",
        "controls (advance)",
        "mics+jet noise",
        "block modes",
        "live: strobe/telemetry",
        "live: decimation",
        "live: compressor+output",
        "(of which) radiation solve",
        "(of which) snapshot",
        "(of which) wave junctions+launch",
        "",
    ];
}

/// Counters.
pub mod cnt {
    pub const RAD_CALLS: usize = 0;
    pub const RAD_EVALS: usize = 1;
    pub const RAD_FALLBACK: usize = 2;
    pub const GRID_CELL_STEPS: usize = 3;
    pub const WAVE_SEG_STEPS: usize = 4;
    pub const SUBSTEPS: usize = 5;
    pub const ADVANCES: usize = 6;
    pub const SNAPSHOTS: usize = 7;
    pub const POWF: usize = 8;
    pub const RAD_ZERO_ITER: usize = 9;
    pub const N: usize = 12;
    pub const NAMES: [&str; N] = [
        "radiation solves",
        "radiation residual evaluations",
        "radiation bracketed fallbacks",
        "grid cell-steps",
        "wave segment-steps",
        "substeps",
        "inner steps",
        "telemetry snapshots",
        "powf calls",
        "radiation solves done on the warm start",
        "",
        "",
    ];
}

#[cfg(feature = "prof")]
mod imp {
    use super::{cat, cnt};
    use std::ptr::addr_of_mut;
    static mut ACC: [u64; cat::N] = [0; cat::N];
    static mut CNT: [u64; cnt::N] = [0; cnt::N];

    #[inline(always)]
    pub fn now() -> u64 {
        #[cfg(target_arch = "aarch64")]
        {
            let v: u64;
            unsafe { std::arch::asm!("mrs {}, cntvct_el0", out(reg) v, options(nomem, nostack)) };
            v
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            use std::sync::OnceLock;
            static T0: OnceLock<std::time::Instant> = OnceLock::new();
            T0.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64
        }
    }
    /// Counter ticks per second.
    pub fn freq() -> f64 {
        #[cfg(target_arch = "aarch64")]
        {
            let v: u64;
            unsafe { std::arch::asm!("mrs {}, cntfrq_el0", out(reg) v, options(nomem, nostack)) };
            v as f64
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            1e9
        }
    }
    #[inline(always)]
    pub fn add(c: usize, t0: u64) -> u64 {
        let t = now();
        unsafe { *addr_of_mut!(ACC[c]) += t.wrapping_sub(t0) };
        t
    }
    #[inline(always)]
    pub fn count(c: usize, n: u64) {
        unsafe { *addr_of_mut!(CNT[c]) += n };
    }
    pub fn take() -> ([f64; cat::N], [u64; cnt::N]) {
        let f = freq();
        let mut a = [0.0; cat::N];
        let mut c = [0; cnt::N];
        unsafe {
            for i in 0..cat::N {
                a[i] = *addr_of_mut!(ACC[i]) as f64 / f;
                *addr_of_mut!(ACC[i]) = 0;
            }
            for i in 0..cnt::N {
                c[i] = *addr_of_mut!(CNT[i]);
                *addr_of_mut!(CNT[i]) = 0;
            }
        }
        (a, c)
    }
}

#[cfg(not(feature = "prof"))]
mod imp {
    use super::{cat, cnt};
    #[inline(always)]
    pub fn now() -> u64 {
        0
    }
    #[inline(always)]
    pub fn add(_c: usize, _t0: u64) -> u64 {
        0
    }
    #[inline(always)]
    pub fn count(_c: usize, _n: u64) {}
    pub fn take() -> ([f64; cat::N], [u64; cnt::N]) {
        ([0.0; cat::N], [0; cnt::N])
    }
}

/// True when the profiler is compiled in.
pub const ON: bool = cfg!(feature = "prof");
/// Start a timed section: returns the current tick.
#[inline(always)]
pub fn now() -> u64 {
    imp::now()
}
/// Close a section begun at `t0`, charging it to `c`; returns the current tick so sections can chain.
#[inline(always)]
pub fn lap(c: usize, t0: u64) -> u64 {
    imp::add(c, t0)
}
#[inline(always)]
pub fn count(c: usize, n: u64) {
    imp::count(c, n)
}
/// Seconds per timer slot and the counters since the last call; resets both.
pub fn take() -> ([f64; cat::N], [u64; cnt::N]) {
    imp::take()
}
