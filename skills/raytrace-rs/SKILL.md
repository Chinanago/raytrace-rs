---
name: raytrace-rs
description: "Run geometric-optics ray tracing with raytrace-rs 0.12+: open ZMX or JSON optical designs, check ZMX compatibility, edit components, trace sequential and non-sequential systems, then read screen distributions, FWHM and power budgets, or scan parameters. Use when the user mentions raytrace-rs, wants a ZMX or lens/rod/screen layout traced, or asks about screen irradiance, FWHM, coupling or power budget."
license: MIT
---

# raytrace-rs

Geometric-optics ray tracer: sources, lenses, integrator rods and screens go in, the light distribution on the screens and a power budget come out. Lengths are mm and wavelengths are µm. Needs raytrace-rs **0.12 or newer** (`raytrace-rs --version`); v0.11 had a different CLI, so upgrade it. Install and AI-client setup (Windows, macOS, Linux): `docs/AI.md`, or run the one-line installer from the README.

## Connect

1. **MCP first.** If the tools `list_commands`, `describe_command`, `execute` and `batch` exist, use them (the server is `raytrace-rs mcp`). The server keeps one session: the open design, its edit history and the latest calculation survive between calls. Call `list_commands` once, `describe_command` before any command you have not used (it returns the exact input and output schemas), then `execute {command, params}` or `batch {requests: [{command, params}]}`.
2. **CLI fallback.** `raytrace-rs COMMAND a.b=text a.b:=JSON` (one session per call). For several steps in one session pipe JSON lines: `echo '{"command":"doc.open","path":"x.json"}' | raytrace-rs run -`. `raytrace-rs COMMAND --help` lists the fields. Stdout is one JSON line per request (`ok`, `command`, `revision`, `result` or `error.code`/`error.message`); the message is Japanese.
3. **An open GUI:** start it with `raytrace-gui --automation-session /abs/private/session.json`, then use MCP `raytrace-rs mcp --connect /abs/private/session.json` or CLI `raytrace-rs --connect /abs/private/session.json COMMAND ...`. This shares the GUI's document, undo history and latest run. Read `session.status` first and give its `revision` as `expected_revision` on every mutation; after a `revision` error, read the state and decide again. Keep the token-bearing file private. GUI and client versions must match.
4. Relative paths follow the server's or shell's working directory; connected calls use the GUI's working directory. Prefer absolute paths for files you write.

## Workflow

1. `doc.open {path}`: a `.zmx`, a design JSON (`raytrace-design-v1`) or a scene JSON like `examples/*.json`.
2. **ZMX: `doc.inspect` first.** Read `traceable` and `compatibility.components[].entries[]`. `computed` is used; `preserved_only` is kept but not calculated; `requires_override` and `unsupported` stop tracing, so tell the user what is needed (for example an uncoated Fresnel calculation changes the physics: ask before using `overrides.ignore_coatings`).
3. `analysis.trace` (omit `model` to trace the session document). Start with `overrides.rays_per_source` around 1024 to 4096; Sobol sampling is best at powers of two. Pass `output` to save the record; leave it out for a quick look.
4. Read results with the analyses, not the large record: `analysis.budget` (power budget) and `analysis.screen` (`detector`, `cursor_mm`, `roi`, `include_profiles`). They use the latest trace unless `run` is given.
5. To change the design: `doc.set {path, value}` (JSON Pointer; array elements by index or `@<id>`, for example `/occurrences/@rod-4/pose/translation_mm/2`; look at the structure with `doc.get`), `component.add|remove|duplicate`. Give related edits the same `group` so one `session.undo` reverts them together; `session.log` lists the changes. Re-trace after every edit.
6. Comparisons: `study.scan` (plan: `parameter`, `values`, optional `secondary`, `links`; parameters such as `component_position`, `rod_size`, `surface_position`, `wavelength`, addressed by occurrence id or surface name, axes 0/1/2 = X/Y/Z), `study.convergence` (ray counts, seeds, pixels), `run.replay` (same optical conditions from a saved record). Describe the command for the plan schema and read the ids with `doc.inspect include_design:=true`.

## Read the results

- Percentages are of the **reference**: the launched weight, which is the 100 % line (`reference_plane` says where it is measured). A screen's `received_percent_of_reference` is its share of that, not of everything the source emits.
- Trust a calculation only after the budget closes: `relative_budget_closure_error` should be near zero (about 1e-9 or smaller) and `unresolved_percent_of_reference` (numerical failures, medium mismatches, branch cutoff, interaction limit) small. Large values mean the model or limits need attention; say so instead of reporting the screen numbers alone.
- `stale: true` means the design changed after the trace. Trace again before quoting numbers.
- `fwhm_mm` is `null` when the half-maximum crossing is missing; `multiple_lobes` marks a profile with several peaks. Distributions from few rays are noisy: check with more rays or `study.convergence` before comparing small differences.
- Report the settings you used (rays per source, sampling, wavelength, mode) so the result can be reproduced.

## Limits

Geometric optics only: no polarization or phase, diffraction or interference, MTF/PSF, automatic optimization or tolerancing. Supported ZMX surfaces, sources, coatings and STEP shapes are listed in `docs/FEATURES.md`; do not promise others. Details of every command, error code and output field are in [workflows](references/workflows.md); the full manual is `docs/GUIDE.md`.
