#!/bin/sh
# Build the simulator for the browser and copy it next to the web app.
# Needs the rustup toolchain with the wasm target (rustup target add wasm32-unknown-unknown);
# a Rust installed without rustup (Homebrew's `rust`, for one) has no wasm target.
# RUST_TOOLCHAIN_BIN may name the folder holding that toolchain's cargo and rustc.
set -e
# --remap-path-prefix keeps the local home folder (and so the user name) out of the published file:
# panic messages in dependencies otherwise embed their full source paths.
cd "$(dirname "$0")/../sim"
TC="${RUST_TOOLCHAIN_BIN:-}"
if [ -z "$TC" ] && command -v rustup >/dev/null 2>&1; then TC="$(dirname "$(rustup which --toolchain stable rustc)")"; fi
[ -n "$TC" ] || TC="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin"
PATH="$TC:$PATH" RUSTC="$TC/rustc" RUSTFLAGS="-C target-feature=+simd128 --remap-path-prefix=$HOME=~ --remap-path-prefix=$(pwd)=sim" "$TC/cargo" build --release --lib \
  --target wasm32-unknown-unknown --target-dir target.nosync/wasm-simd
cp target.nosync/wasm-simd/wasm32-unknown-unknown/release/engsim.wasm ../live/engsim.wasm
ls -la ../live/engsim.wasm
if strings -n 6 ../live/engsim.wasm | grep -q -E "/(Users|home)/"; then echo "engsim.wasm still contains a local path" >&2; exit 1; fi
