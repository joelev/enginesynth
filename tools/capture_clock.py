"""The capture clock shared by the offline renderers (render_live_video.py, render_og_all.py).

CLOCK is a script to add to a page before any of its own run (Playwright add_init_script). It
replaces performance.now and requestAnimationFrame, so the page (opened with ?capture=1, see
live/capture-engine.js) advances only when __cap.step(samples) is called: the simulator by that
many samples, the page's clock by 1/60 s, and CSS transitions and animations with it."""
# the page's clock, replaced before any of its scripts run
CLOCK = """(() => {
  const C = window.__cap = { t: 0, q: [], id: 0, frames: 0 };
  performance.now = () => C.t;
  window.requestAnimationFrame = (cb) => { C.q.push(cb); return ++C.id; };
  window.cancelAnimationFrame = () => {};
  // one video frame: the simulator first, then everything the page draws for that instant
  C.step = (samples) => {
    const S = window.__app, e = S && S.engine;
    if (e && e.step && S.started) e.step(samples);
    C.t += 1000 / 60; C.frames++;
    const q = C.q; C.q = [];
    for (const cb of q) cb(C.t);
    // CSS transitions and animations (key travel, the gear numeral's tick, the tachometer's kick) are held
    // and moved by the same clock, so they too are drawn for this instant and not for however long the screenshot took
    for (const a of document.getAnimations()) {
      if (a.__t0 === undefined) { a.__t0 = C.t; a.pause(); }
      const end = a.effect ? a.effect.getComputedTiming().endTime : 0;
      if (Number.isFinite(end) && C.t - a.__t0 >= end) a.finish(); else a.currentTime = C.t - a.__t0;
    }
    if (!e || !e.frame) return null;
    const f = e.frame, r = new Float32Array(samples); e.last(r);
    let s = 0, pk = 0; for (const v of r) { s += v * v; if (Math.abs(v) > pk) pk = Math.abs(v); }
    return [f[2], f[5], f[6], f[7], f[3], Math.sqrt(s / samples), pk, Array.prototype.every.call(f, Number.isFinite) ? 1 : 0, S.thr, f[0]];
  };
})();"""
