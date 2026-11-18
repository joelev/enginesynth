//! Embed every engine definition in ../engines/defs into the binary, in the order given by
//! ../engines/defs/ORDER (one id per line). A definition missing from ORDER, or listed but
//! absent, fails the build, so the list and the folder cannot drift apart.
use std::{env, fs, path::PathBuf};

fn main() {
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../engines/defs");
    let dir = dir.canonicalize().expect("engines/defs not found");
    println!("cargo:rerun-if-changed={}", dir.display());
    let order = fs::read_to_string(dir.join("ORDER")).expect("engines/defs/ORDER missing");
    let ids: Vec<&str> = order.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    let mut on_disk: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".json")).map(|s| s.to_string()))
        .collect();
    on_disk.sort();
    for f in &on_disk {
        println!("cargo:rerun-if-changed={}", dir.join(format!("{f}.json")).display());
        if !ids.contains(&f.as_str()) {
            panic!("engines/defs/{f}.json is not listed in engines/defs/ORDER");
        }
    }
    let mut out = String::from("pub const DEFS: &[(&str, &str)] = &[\n");
    for id in &ids {
        let p = dir.join(format!("{id}.json"));
        if !p.is_file() {
            panic!("engines/defs/ORDER lists '{id}' but engines/defs/{id}.json does not exist");
        }
        out.push_str(&format!("    ({:?}, include_str!({:?})),\n", id, p.display().to_string()));
    }
    out.push_str("];\n");
    fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("defs.rs"), out).unwrap();
}
