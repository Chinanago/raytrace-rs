# Running raytrace-rs

## Locate the application and documentation

1. Use the executable or installation explicitly selected by the user.
2. Otherwise check `raytrace-rs` on PATH, an extracted Windows package, or the current raytrace-rs source checkout. The release CLI is `raytrace-rs.exe`; a Windows source build places it in `target/release`. The GUI is `raytrace-gui.exe`.
3. Check `--version` and `--help` before using version-specific arguments. Record the selected executable/version with results. Do not assume the skill is installed inside the application checkout.

Upstream: [source](https://github.com/Chinanago/raytrace-rs), [Windows releases](https://github.com/Chinanago/raytrace-rs/releases).

The baseline below is **v0.11.0**. Prefer matching local documentation if the installed version differs:

| Need | Local documentation | v0.11.0 upstream copy |
|---|---|---|
| Physics, supported inputs and restrictions | `docs/FEATURES.md` | [Features](https://github.com/Chinanago/raytrace-rs/blob/v0.11.0/docs/FEATURES.md) |
| Import, placement, sources, screens, scans, convergence and replay | `docs/GUIDE.md` | [Guide](https://github.com/Chinanago/raytrace-rs/blob/v0.11.0/docs/GUIDE.md) |
| Build, test or modify the application | `docs/DEVELOPMENT.md` | [Development](https://github.com/Chinanago/raytrace-rs/blob/v0.11.0/docs/DEVELOPMENT.md) |
| Model examples and exact schemas | `examples/*.json`, `src/project.rs`, `src/assembly.rs`, `src/model.rs` | [Examples](https://github.com/Chinanago/raytrace-rs/tree/v0.11.0/examples) |

The Windows ZIP runs without Rust or OpticStudio installed. If a source build is needed, run `cargo build --release --locked` from the checkout; add `--features gui --bins` for CLI and GUI together. Follow applicable project instructions when modifying code. Validate other-platform builds before promising support.

## First run from an extracted Windows ZIP

Run these PowerShell commands from the extracted application folder. The examples include dispersive materials. For a real task, set the executable, model and output paths to that task's locations.

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

Read the printed report and saved project. Import entries distinguish definitions used for calculation, preserved-only information, conditions requiring explicit settings and unsupported definitions. Do not use `--ignore-coatings` merely to bypass a blocker. Parsing/saving can succeed while tracing remains blocked.

AGF catalogs are searched beside the input ZMX and, on Windows, under `%USERPROFILE%\Documents\Zemax\Glasscat`. The GUI can add catalog folders. Preserve resolvable STEP references; placing referenced STEP files beside the ZMX is supported. Check declared catalogs and resolved materials.

For GUI assembly: new optical system → add source ZMX → add lens ZMX → add rod → add screen → place components → calculate → inspect screens. Save a project JSON. v0.11.0 supports assembly JSON and individual-model ZMX editing/export; assembly-to-ZMX export is unavailable.

CLI lengths are **mm**, wavelengths **μm**; GUI wavelengths are **nm**. `--wavelength-um 0.465` explicitly replaces the source spectrum with 465 nm. A spectral study should retain the full intended spectrum.

## Fixed rays, replay and parameter work

`--rays-csv` reads:
`id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium`.
Check medium indices against the effective scene. `--terminals` saves terminal rays; `--paths N` saves paths for launch IDs below N. Large path collections increase output size.

Replay freezes optical conditions. Output, threads, repeat and path/terminal recording settings can change; source, geometry, wavelength and mode overrides require a new calculation.

Read the guide's plan schema before creating scan/convergence plans. With the actual model and plan paths assigned:

```powershell
& $rtCli --scan $rtModel --plan $rtScanPlan --output (Join-Path $rtWork 'scan')
& $rtCli --resume-scan (Join-Path $rtWork 'scan')
& $rtCli --convergence $rtModel --plan $rtConvergencePlan --output (Join-Path $rtWork 'convergence')
```

Initial STEP parsing/tessellation in v0.11.0 uses Rust. Supported inputs are single connected closed solids with supported surfaces and boundaries; see Features for exact restrictions. The default deflection is 0.02 mm. CAD convergence needs the original STEP-referencing ZMX because saved projects contain an already generated mesh. Check normal fallbacks and sensitivity near grazing or critical angles. Geometry tessellation and wavelength-dependent material physics are separate calculation steps.

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
| `benchmark.timed_scope` | Measured scope. `--repeat N` reports median tracing/report-accumulation time after a warmup, excluding import, ray generation, I/O and drawing. |

Use the GUI or existing screen/export functions for peak normalization, profiles, FWHM and ROI. Wave optics, full diffraction PSF/MTF and general automatic optimization are outside the v0.11.0 feature set; check newer documentation before repeating versioned limitations.
