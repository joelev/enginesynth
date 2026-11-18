//! engsim: engine sound from one-dimensional gas dynamics.
pub mod audio;
pub mod boost;
pub mod engine;
pub mod gas;
pub mod live;
pub mod pipe;
pub mod prof;
pub mod spec;
pub mod turbo;
pub mod wave;

use engine::{Mode, Quality, System};

pub fn quality(name: &str) -> Quality {
    // Experiments, off unless asked for:
    // "<name>+f" = grid pipes reconstruct and compute fluxes in 32-bit floats (state stays 64-bit)
    if let Some(base) = name.strip_suffix("+f") {
        let mut q = quality(base);
        q.f32 = true;
        return q;
    }
    // "<name>+m" = cheaper transcendental functions in the cylinder model (cube root, Woschni powers)
    if let Some(base) = name.strip_suffix("+m") {
        let mut q = quality(base);
        q.fast_math = true;
        return q;
    }
    // "<name>+r" = open-end radiation solve warm-started with the last solve's slope
    if let Some(base) = name.strip_suffix("+r") {
        let mut q = quality(base);
        q.rad_fast = true;
        return q;
    }
    // "<name>+k" = same grid with third-order reconstruction
    if let Some(base) = name.strip_suffix("+k") {
        let mut q = quality(base);
        q.recon = 1;
        return q;
    }
    // "<name>+w" = pipes without a valve on them are solved along characteristics (wave.rs)
    if let Some(base) = name.strip_suffix("+w") {
        let mut q = quality(base);
        q.wave = true;
        return q;
    }
    // "<name>+W" = as +w, and intake runners too
    if let Some(base) = name.strip_suffix("+W") {
        let mut q = quality(base);
        q.wave = true;
        q.wave_runners = true;
        return q;
    }
    // experiment hook: "m<rate-kHz>c<cfl>" e.g. m96c0.6 = matched grids at 96 kHz, CFL 0.6
    if let Some(rest) = name.strip_prefix('m') {
        if let Some((r, c)) = rest.split_once('c') {
            if let (Ok(r), Ok(c)) = (r.parse::<f64>(), c.parse::<f64>()) {
                return Quality { rate: r * 1000.0, dx_ex: 0.0, dx_in: 0.0, matched_cfl: c, recon: 0, wave: false, wave_runners: false, f32: false, fast_math: false, rad_fast: false };
            }
        }
    }
    match name {
        "final" => Quality { rate: 192_000.0, dx_ex: 0.008, dx_in: 0.005, matched_cfl: 0.0, recon: 0, wave: false, wave_runners: false, f32: false, fast_math: false, rad_fast: false },
        // grids matched to each pipe's gas, for real-time use
        "live96" => Quality { rate: 96_000.0, dx_ex: 0.0, dx_in: 0.0, matched_cfl: 0.88, recon: 0, wave: false, wave_runners: false, f32: false, fast_math: false, rad_fast: false },
        "live48" => Quality { rate: 48_000.0, dx_ex: 0.0, dx_in: 0.0, matched_cfl: 0.88, recon: 0, wave: false, wave_runners: false, f32: false, fast_math: false, rad_fast: false },
        _ => Quality { rate: 96_000.0, dx_ex: 0.016, dx_in: 0.010, matched_cfl: 0.0, recon: 0, wave: false, wave_runners: false, f32: false, fast_math: false, rad_fast: false },
    }
}

/// Speed test callable from WebAssembly: run `samples` inner-rate steps of preset number
/// `engine` (index into spec::names()) at steady `rpm`, full throttle. Quality level:
/// 0 draft, 1 final, 2 live48, 3 live96, 4 live48+W, 5 live96+W. Returns a checksum.
#[no_mangle]
pub extern "C" fn wasm_bench(engine: u32, quality_level: u32, rpm: f64, samples: u32) -> f64 {
    let names = spec::names();
    let s = spec::preset(names[engine as usize % names.len()]).unwrap();
    let q = quality(["draft", "final", "live48", "live96", "live48+W", "live96+W"][quality_level as usize % 6]);
    let mut sys = System::build(&s, &q, 3);
    sys.mode = Mode::Dyno;
    sys.omega = rpm * std::f64::consts::PI / 30.0;
    sys.throttle_cmd = 1.0;
    let mut acc = 0.0;
    for _ in 0..samples {
        let o = sys.advance();
        acc += o[0].abs() + o[2].abs();
    }
    acc
}
