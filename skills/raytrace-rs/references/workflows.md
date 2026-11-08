# Running raytrace-rs

## Locate the application and documentation

1. Use the executable or installation explicitly selected by the user.
2. Otherwise check `raytrace-rs` on PATH, an extracted Windows or Linux package, or the current raytrace-rs source checkout. On Linux without a package, newer releases provide `https://github.com/Chinanago/raytrace-rs/releases/latest/download/raytrace-rs-linux-x64.tar.gz` (static CLI), and `cargo install --git https://github.com/Chinanago/raytrace-rs --locked raytrace-cli` builds the CLI anywhere Rust is available. The release CLI is `raytrace-rs.exe`; a Windows source build places it in `target/release`. The GUI is `raytrace-gui.exe`.
3. Check `--version` and `--help` before using version-specific arguments. The skill directory and application checkout can be separate.

Upstream: [source](https://github.com/Chinanago/raytrace-rs), [Windows releases](https://github.com/Chinanago/raytrace-rs/releases).

The baseline below is **v0.11.0**. For another installed version, use its matching documentation:

| Need | Local documentation | v0.11.0 upstream copy |
|---|---|---|
| Physics, supported inputs and restrictions | `docs/FEATURES.md` | [Features](https://github.com/Chinanago/raytrace-rs/blob/v0.11.0/docs/FEATURES.md) |
| Import, placement, sources, screens, scans, convergence and replay | `docs/GUIDE.md` | [Guide](https://github.com/Chinanago/raytrace-rs/blob/v0.11.0/docs/GUIDE.md) |
| Build, test or modify the application | `docs/DEVELOPMENT.md` | [Development](https://github.com/Chinanago/raytrace-rs/blob/v0.11.0/docs/DEVELOPMENT.md) |
| Model examples and exact schemas | `schemas/*.schema.json` (newer builds), `examples/*.json`, `crates/raytrace-doc/src/design.rs` (newer builds; older: `crates/raytrace-io/src/project.rs`, `crates/raytrace-io/src/assembly.rs`), `crates/raytrace-core/src/model.rs` | [Examples](https://github.com/Chinanago/raytrace-rs/tree/v0.11.0/examples) |

The Windows ZIP runs without Rust or OpticStudio installed. If a source build is needed, run `cargo build --release --locked -p raytrace-cli` from the checkout for the CLI, or `cargo build --release --locked --workspace --bins` for CLI and GUI together. Windows is the verified distribution platform for v0.11.0.

## Command forms by version

Check `--help` first. v0.11.0 uses the options shown in the PowerShell examples below. Newer builds (`--help` lists commands such as `trace` and `run`) have no options of their own: every argument sets a field of a command's parameters, and the GUI calls the same commands. Builds with the session (`--help` lists ids such as `analysis.trace`) name commands `<area>.<action>`; the short names `trace`, `inspect`, `replay`, `scan`, `resume_scan`, `convergence`, `open`, `save_design`, `save_run`, `export_zmx`, `export_screen`, `schema` and `version` remain as deprecated aliases (`open` opens designs only: records are `run.open`, scan directories `study.open`). Prefer the dotted ids.

- `raytrace-rs COMMAND field=text field:=JSON field:=@file.json`; dotted names reach nested fields (`overrides.rays_per_source:=1024`). `raytrace-rs COMMAND --help` lists that command's fields, types, defaults and descriptions, generated from the schema. Unknown fields fail with `invalid_request`.
- `raytrace-rs run REQUEST.json` or `raytrace-rs run -` (stdin, several requests in a row) executes the same requests as JSON, `{"command": "<id>", ...parameters}`. `model` is `{"path": ...}`, `{"design": {...}}` or `{"scene": {...}}` (builds before the design document used `{"project": {...}}`); `include_record` returns the full calculation record in `result.record`.
- A `run` stream is one session. `doc.open` opens a file as the session document; commands then run without `model`: `analysis.trace`, `doc.inspect`, `doc.export_zmx`, `study.scan`. Edit with `doc.set {"path": "/occurrences/@rod-4/pose/translation_mm/2", "value": 12.5}` (JSON Pointer into the design JSON; array elements by index or `@<id>`; read the ids and the structure with `doc.get {"path": "/occurrences"}`), `component.add` (`path`+`kind` source|optic|system, or `kind` rod|detector), `component.remove {"occurrence": "rod-4"}`, `component.duplicate`, `doc.replace`; `session.undo`/`session.redo` step through the history and `session.status` shows the revision. Every response has `revision`; pass `expected_revision` to a change to reject it if the document moved meanwhile (`error.code` `revision`). A failed change leaves the document alone. A single command line is a new session, so pass `model` there. `command.list` and `command.describe id=...` return the commands and their input/output schemas.
- stdout carries exactly one JSON line per request: `{"ok": true, "command": ..., "revision": N, "result": {...}}` or `{"ok": false, "command": ..., "revision": N, "error": {"code": ..., "message": ...}}`. Exit code 1 on failure. Human text, warnings and scan progress go to stderr. Without `output`, `trace` and `replay` save nothing and return only the summary.
- `raytrace-rs schema.get name=scene` returns a JSON Schema in `result.schema` (names: request, response, scene, design, run, scan_plan, scan, convergence_plan, convergence; builds before the design document call `design` `project`); `schema.get output=DIR` writes all of them. `request` and `response` list every command. The package's `schemas/` folder holds the same files. Prefer the schemas over source code when writing models, plans and requests.
- Scan plans in builds with the design document name components by occurrence id and surfaces by name: `{"type": "component_position", "occurrence": "rod-4", "axis": 2}`, `{"type": "rod_size", "occurrence": "rod-4", "axis": 2}`, `{"type": "surface_position", "surface": "screen", "axis": 2}` (add `occurrence` to address a surface of a placed part). Read the ids from `doc.inspect ... include_design:=true` or the saved design JSON. Earlier builds used `component`/`surface` indices, and their scan records cannot be resumed.
- `error.code` values are stable identifiers such as `io`, `invalid_json`, `invalid_request`, `usage`, `invalid_scene`, `invalid_ray`, `design` (newer builds), `zmx`, `agf`, `step`, `plan`, `limit`, `cases_failed`, `cancelled`, `revision`, `no_document` and `history`; `message` is Japanese.

| v0.11.0 | Newer builds |
|---|---|
| `MODEL --output RUN.json` | `analysis.trace model.path=MODEL output=RUN.json` |
| `--rays-per-source N`, `--sampling S`, `--mode M`, `--wavelength-um W`, `--ignore-coatings` | `overrides.rays_per_source:=N`, `overrides.sampling=S`, `overrides.mode=M`, `overrides.wavelength_um:=W`, `overrides.ignore_coatings:=true` |
| `--rays-csv FILE`, `--terminals`, `--paths N`, `--threads N`, `--repeat N` | `rays.csv=FILE`, `terminals:=true`, `paths:=N`, `threads:=N`, `repeat:=N` (add `warmup:=true` to match the old benchmark timing) |
| `MODEL --inspect --output P.json`, `--export-zmx Z` | `doc.inspect model.path=MODEL output=P.json`, `export_zmx=Z` |
| `--replay RUN.json --output NEW.json` | `run.replay record.path=RUN.json output=NEW.json` |
| `--scan MODEL --plan PLAN --output DIR`, `--resume-scan DIR` | `study.scan model.path=MODEL plan:=@PLAN output=DIR`, `study.resume directory=DIR` |
| `--convergence MODEL --plan PLAN --output DIR` | `study.convergence model.path=MODEL plan:=@PLAN output=DIR` |

```bash
raytrace-rs doc.inspect model.path=lens.zmx
raytrace-rs analysis.trace model.path=examples/sequential_lens.json overrides.rays_per_source:=1024 overrides.sampling=sobol threads:=4 output=results/lens.json
echo '{"command":"run.replay","record":{"path":"results/lens.json"},"output":"results/lens_replay.json"}' | raytrace-rs run -
# One session: open, edit, calculate, undo
cat <<'EOF' | raytrace-rs run -
{"command":"doc.open","path":"examples/sequential_lens.json"}
{"command":"doc.set","path":"/parts/@system-1/scene/surfaces/2/pose/translation_mm/2","value":70.0}
{"command":"analysis.trace","overrides":{"rays_per_source":1024},"output":"results/moved.json"}
{"command":"session.undo"}
EOF
```

## First run from an extracted Windows ZIP (v0.11.0 options)

Run these PowerShell commands from the extracted application folder; for newer builds convert the options with the table above. The examples include dispersive materials and explicitly set 1024 rays per source, Sobol sampling and four threads. Those are example parameters; omitted overrides retain model settings, and an omitted thread count uses the runtime's default. Set paths and options for the current task.

```powershell
$rtRoot = (Get-Location).Path
$rtCli = (Resolve-Path -LiteralPath '.\raytrace-rs.exe').Path
$rtWork = Join-Path $rtRoot 'results\optics'
New-Item -ItemType Directory -Path $rtWork -Force | Out-Null
& $rtCli --version
& $rtCli --help
& $rtCli (Join-Path $rtRoot 'examples\sequential_lens.json') --rays-per-source 1024 --sampling sobol --threads 4 --output (Join-Path $rtWork 'lens_run.json')
& $rtCli --replay (Join-Path $rtWork 'lens_run.json') --threads 4 --output (Join-Path $rtWork 'lens_replay.json')
& $rtCli (Join-Path $rtRoot 'examples\integrator_rod.json') --rays-per-source 1024 --sampling sobol --threads 4 --output (Join-Path $rtWork 'rod_run.json')
```

For a source checkout, resolve `target/release/raytrace-rs.exe` instead and keep `$rtRoot` at the checkout root. A CLI already on PATH accepts the same arguments using the shell's normal command syntax.

## Import and assemble

Inspect a supplied ZMX before tracing. Set `$rtInputZmx` to the actual input path:

```powershell
& $rtCli $rtInputZmx --inspect --output (Join-Path $rtWork 'imported_project.json')
```

Read the printed report and saved project. Import entries distinguish definitions used for calculation, preserved-only information, conditions requiring explicit settings and unsupported definitions. `--ignore-coatings` opts into an uncoated calculation and changes the physical model. Parsing/saving can succeed while tracing remains blocked.

AGF catalogs are searched beside the input ZMX and, on Windows, under `%USERPROFILE%\Documents\Zemax\Glasscat`. The GUI can add catalog folders. Preserve resolvable STEP references; placing referenced STEP files beside the ZMX is supported. Check declared catalogs and resolved materials.

For GUI assembly: new optical system → add source ZMX → add lens ZMX → add rod → add screen → place components → calculate → inspect screens. Save the design JSON (`save_design`; the format `raytrace-design-v1` holds parts and occurrences with stable ids such as `rod-4`, and builds before the design document saved `raytrace-project-v1` to `v3`, which newer builds still open). v0.11.0 supports assembly JSON and individual-model ZMX editing/export; assembly-to-ZMX export is unavailable.

CLI lengths are **mm**, wavelengths **μm**; GUI wavelengths are **nm**. `--wavelength-um 0.465` explicitly replaces the source spectrum with 465 nm. Omitting that override retains the model's spectrum.

## Fixed rays, replay and parameter work

Explicit rays (`--rays-csv`, or `rays.csv=` in newer builds) read:
`id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium`.
Check medium indices against the effective scene. `--terminals` saves terminal rays; `--paths N` saves paths for the first N launch rays in input order. Large path collections increase output size.

Replay freezes optical conditions. Output, threads, repeat and path/terminal recording settings can change; source, geometry, wavelength and mode overrides require a new calculation.

Read the guide's plan schema before creating scan/convergence plans. With the actual model and plan paths assigned:

```powershell
& $rtCli --scan $rtModel --plan $rtScanPlan --output (Join-Path $rtWork 'scan')
& $rtCli --resume-scan (Join-Path $rtWork 'scan')
& $rtCli --convergence $rtModel --plan $rtConvergencePlan --output (Join-Path $rtWork 'convergence')
```

Initial STEP parsing/tessellation in v0.11.0 uses Rust. Supported inputs are single connected closed solids with supported surfaces and boundaries; see Features for exact restrictions. The default deflection is 0.02 mm. CAD convergence needs the original STEP-referencing ZMX because saved projects contain an already generated mesh. CAD normal fallbacks are recorded in trace diagnostics. Geometry tessellation and wavelength-dependent material physics are separate calculation steps.

Windows settings/cache location is `%LOCALAPPDATA%\Raytrace`. Set process-local `RAYTRACE_DATA_DIR` to a task-owned folder when isolated runtime/cache storage is useful.

## Interpret a saved run

v0.11.0 CLI output uses `format: raytrace-run-v1`, `schema_version: 2`:

| Field | Meaning |
|---|---|
| `reference_weight`, `scene.reference_plane` | Launch-weight sum and reference description. Establish a conversion if the requested physical baseline differs. |
| `detector_percent_of_reference`, `budget_percent_of_reference` | Already percentages of that launch sum. |
| `report.detectors[].weights` | Integrated power weights per pixel; divide by pixel area for density. |
| `report.budget`, `relative_budget_closure_error` | Accounted powers and `(accounted - launched) / launched`. Inspect unresolved categories as well as closure. |
| `surface_encounter_percent_of_reference` | Includes repeated encounters; derive physical stage budgets with that distinction in mind. |
| `calculation` | Frozen effective model, source settings, software version and limits. CSV input rays are embedded for replay. |
| `benchmark.timed_scope` | Actual measurement scope. Generated-source runs include ray generation, tracing and accumulation; explicit-ray runs include tracing and accumulation. `--repeat N` reports their median after a warmup, excluding scene preparation, file I/O and drawing. Older builds can exclude generation; use the saved field when comparing runs. |

Use the GUI or existing screen/export functions for peak normalization, profiles, FWHM and ROI. Wave optics, full diffraction PSF/MTF and general automatic optimization are outside the v0.11.0 feature set; check newer documentation before repeating versioned limitations.
