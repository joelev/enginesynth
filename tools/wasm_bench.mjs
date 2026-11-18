// Time the simulator compiled to WebAssembly (node uses V8, the same engine as Chrome).
// usage: node wasm_bench.mjs <engsim.wasm> [engine ...]
import fs from 'node:fs';
const names = ["single","vtwin45","ltwin90","i3","i4","i5","i6","boxer4_uel","boxer4_el","flat6","v8_cross","v8_flat","v8_cam","v10_f1","v12"];
const levels = [["live48+W", 4, 48000], ["live48", 2, 48000], ["live96+W", 5, 96000], ["live96", 3, 96000], ["final", 1, 192000]];
const wasm = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
const bench = wasm.instance.exports.wasm_bench;
const which = process.argv.length > 3 ? process.argv.slice(3) : ["single","vtwin45","i4","flat6","v8_cross","v10_f1","v12"];
console.log("engine      " + levels.map(l => l[0].padStart(10)).join(""));
for (const name of which) {
  const idx = names.indexOf(name);
  let row = name.padEnd(12);
  for (const [, lvl, rate] of levels) {
    bench(idx, lvl, 4000, 2000); // warm up the JIT
    let best = 1e9;
    for (let k = 0; k < 3; k++) { const t0 = performance.now(); bench(idx, lvl, 4000, rate / 2); best = Math.min(best, (performance.now() - t0) / 500); }
    row += best.toFixed(2).padStart(10);
  }
  console.log(row);
}
