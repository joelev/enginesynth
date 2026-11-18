# Research file format

One JSON file per engine, `engines/research/<id>.json`, written by a researcher from public sources.
Every number carries the URL it was read from. A value that could not be found is `null` with a note; nothing is guessed.

```json
{
  "id": "honda_b18c5",
  "name": "Honda B18C5 (Integra Type R, 1997-2001, US)",
  "induction": "naturally aspirated",
  "summary": "one or two plain sentences on what makes this engine's sound distinctive",
  "fields": {
    "<field>": {"value": 81.0, "unit": "mm", "source": "https://...", "note": "optional: measuring convention, variant, doubt"}
  },
  "missing": ["fields looked for and not found"],
  "sources": [{"url": "https://...", "what": "what this page is and why it is credible"}]
}
```

Fields to look for (use these names; add others that matter):
cylinders, layout (inline / V / flat / VR / W / radial / rotary), bank_angle_deg, bore_mm, stroke_mm, rod_length_mm,
compression_ratio, displacement_cc, firing_order (state the cylinder numbering convention and which cylinders are on which bank),
crank (plane layout, pin offsets, anything unusual), firing_intervals_deg (if uneven),
valves_per_cylinder, intake_valve_dia_mm, exhaust_valve_dia_mm, intake_lift_mm, exhaust_lift_mm,
intake_duration_deg, exhaust_duration_deg, ivo_btdc, ivc_abdc, evo_bbdc, evc_atdc (state the lift the timing is measured at),
second cam profile and switch rpm for variable systems (same fields with a `_high` suffix, and `cam_switch_rpm`),
redline_rpm, idle_rpm, peak_power_kw, peak_power_rpm, peak_torque_nm, peak_torque_rpm,
intake_type (plenum / individual throttles), runner_length_mm, runner_dia_mm, throttle_dia_mm,
header_type (4-1, 4-2-1, log manifold, ...), primary_dia_mm, primary_length_mm, exhaust_pipe_dia_mm, exhaust_notes,
for rotaries: eccentricity_mm, rotor_radius_mm, rotor_width_mm, port type and timings;
for two-strokes: port timings (exhaust, transfer, intake durations or heights), reed or piston port, expansion chamber dimensions.

## The vehicle (optional)

The car or motorcycle the engine came in, for Drive (README "Vehicles in Drive"; the definition's `vehicle` block is described with the rest of the definition format). Its sources go in the research file under a top-level `vehicle` key: one entry per field (value, unit, source URL, note), either directly under `vehicle` or under `vehicle.fields` (the data pass used both), using the definition's field names: `name`, `kind`, `curb_kg`, `gear_count`, `gears`, `final_drive`, `wheel_radius_m`, `tyre`, `driven`, `cda_m2`, `top_speed_kmh`. A value not found is `null` with a note. Motorcycle final drive is primary times secondary reduction: record both published ratios in the note and say the multiplication is ours. Weight: state whether it is curb, wet, dry or unladen. The simulator does not read research files.

```json
"vehicle": {
  "curb_kg": {"value": 1442, "unit": "kg", "source": "https://...", "note": "3,179 lb coupe, manual"},
  "gears": {"value": [2.66, 1.78, 1.30, 1.00, 0.74, 0.50], "unit": "", "source": "https://..."},
  "final_drive": {"value": 3.42, "unit": "", "source": "https://..."}
}
```
