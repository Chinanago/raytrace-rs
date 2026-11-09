# raytrace-rs reference (0.12+)

Commands, fields and result meanings. The schemas are the source of truth: `describe_command` over MCP, `raytrace-rs COMMAND --help` or `raytrace-rs command.describe id=COMMAND` on the CLI, `raytrace-rs schema.get name=NAME` (`request`, `response`, `scene`, `design`, `run`, `scan_plan`, `scan`, `convergence_plan`, `convergence`) for file formats. v0.11 used a different CLI (options such as `--inspect`); upgrade instead of mapping its options.

## Locate the application

1. Use the executable the user names.
2. Otherwise `raytrace-rs` on PATH, then the installer's location: `~/.local/bin/raytrace-rs` (macOS, Linux) or `%LOCALAPPDATA%\Programs\raytrace-rs\raytrace-rs.exe` (Windows). Examples, schemas and docs sit in `~/.local/share/raytrace-rs` (macOS, Linux) or beside the Windows executable.
3. If it is missing, point the user to `docs/AI.md` (installer one-liners and Releases at `https://github.com/Chinanago/raytrace-rs/releases/latest`). Linux CLI: `raytrace-rs-linux-x64.tar.gz`; anywhere with Rust: `cargo install --git https://github.com/Chinanago/raytrace-rs --locked raytrace-cli`.
4. Check `raytrace-rs --version` and `raytrace-rs --help` (lists all commands). MCP clients start the server as `raytrace-rs mcp` with the absolute path configured in the client.

Documentation next to the application (or upstream under `https://github.com/Chinanago/raytrace-rs/tree/main`): `docs/AI.md` (setup), `docs/GUIDE.md` (operations, plans, error codes), `docs/FEATURES.md` (supported physics and inputs), `examples/*.json` (four sample models), `schemas/*.schema.json`.

## Calling commands

- MCP: `execute {"command": "analysis.trace", "params": {...}}`; `batch {"requests": [{"command": ..., "params": ...}], "stop_on_error": true}`; `list_commands` returns `{commands: [{id, kind, summary}]}` (`kind` is `query`, `mutation` or `job`); `describe_command {command}` returns `input_schema` and `output_schema`. A failed command comes back with `isError` and `error.code`/`error.message`. Long jobs (`job`) can be cancelled by the client (`error.code` `cancelled`) and report progress.
- CLI: `raytrace-rs COMMAND field=text field:=JSON field:=@file.json`; dotted names reach nested fields (`overrides.rays_per_source:=1024`); unknown fields fail with `invalid_request`. Only dotted ids work (the pre-0.12 short names `trace`, `inspect`, `open`, ... fail with a hint). A single command line is a new session, so pass `model.path=...`. Stdin: `raytrace-rs run -` or `run FILE.json` executes `{"command": "<id>", ...fields}` lines in one session; the same MCP calls apply (`doc.open` first, later commands omit `model`).
- Responses: `{"ok": true, "command", "revision": N, "result": {...}}` or `{"ok": false, ..., "error": {"code", "message"}}` (exit code 1 on the CLI). Human text and progress go to stderr.
- `error.code` values: `io`, `invalid_json`, `invalid_request`, `usage`, `invalid_scene`, `invalid_ray`, `design`, `zmx`, `agf`, `step`, `plan`, `limit`, `cases_failed`, `cancelled`, `revision`, `no_document`, `no_run`, `history`. Messages are Japanese.

## Commands

| Area | Commands |
|---|---|
| Document | `doc.new`, `doc.open {path}`, `doc.save`, `doc.get {path?}`, `doc.inspect`, `doc.export_zmx`, `doc.set {path, value}`, `doc.replace` |
| Components | `component.add` (`path` + `kind` source / optic / system, or `kind` rod / detector), `component.remove {occurrence}`, `component.duplicate` |
| Calculation | `analysis.trace`, `run.replay`, `run.open`, `run.save`, `run.export_screen` |
| Analyses | `analysis.budget`, `analysis.screen` |
| Studies | `study.scan`, `study.resume`, `study.convergence`, `study.open` |
| Session | `session.undo`, `session.redo`, `session.status`, `session.log` |
| Meta | `app.version`, `command.list`, `command.describe`, `schema.get` |

## Session and editing

- `doc.open` makes a file the session document; commands that take `model` (`analysis.trace`, `doc.inspect`, `doc.export_zmx`, `study.scan`, ...) use it when `model` is omitted. `model` can also be `{"path"}`, `{"design": {...}}` or `{"scene": {...}}`.
- A design (`raytrace-design-v1`) holds `parts` (what a thing is: a `system` with a `scene`, a rod, a detector, ...) and `occurrences` (placements, with a `pose`). Ids are stable (`rod-4`, `system-1`). A scene JSON opens as a design with one part. Read the structure with `doc.get {"path": "/occurrences"}`.
- `doc.set {"path": "/occurrences/@rod-4/pose/translation_mm/2", "value": 12.5}` writes one value by JSON Pointer; array elements by index or `@<id>`. Typical paths: `/parts/@system-1/scene/surfaces/2/pose/translation_mm/2`, `/parts/0/scene/sources/0/count`. `component.remove {"occurrence": "rod-4"}` deletes a placement.
- Changes take an optional `group`: consecutive changes with the same `group` become one undo step. `expected_revision` rejects a change if the document moved (`error.code` `revision`). A failed change leaves the document untouched. `session.undo` / `session.redo` step through the history (64 steps); `session.log` returns `{base, entries}` of committed `{command, params}`; `session.status` shows the revision and the latest run.
- ZMX import: AGF catalogs are searched beside the ZMX, in the extra `catalogs` folders and the standard Zemax Glasscat folder; STEP files referenced by the ZMX are found beside it. Missing dependencies and unsupported definitions are reported by `doc.inspect`.

## Calculation

- `analysis.trace` fields: `model`, `output` (save the record), `overrides.mode|sampling|rays_per_source|wavelength_um|ignore_coatings`, `rays.csv` (explicit rays: `id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium`), `terminals:=true`, `paths:=N` (path segments of the first N rays), `threads`, `include_record`. Without `output` nothing is saved and only the summary returns. Omitted overrides keep the model's settings; default sampling is Sobol (best for power-of-two counts); `pseudorandom` is the alternative.
- Summary: `launched_rays`, `detected_percent_of_reference`, `budget_percent_of_reference`, `detector_percent_of_reference` (by screen), `relative_budget_closure_error`, `warnings`.
- Sequential mode follows the surface order (a ray that fails total internal reflection ends); non-sequential mode follows the nearest hit and splits Fresnel branches. Assemblies use non-sequential tracing.
- `run.replay record.path=... output=...` repeats a saved record with its frozen optical conditions (threads and recording can change; source, geometry, wavelength and mode need a new `analysis.trace`). `run.export_screen record.path=... output=DIR` writes CSV and PNG of a screen.

## Analyses

`analysis.budget` and `analysis.screen` post-process a record, so trace first. Without `run` they use the latest record in the session (from `analysis.trace`, `run.replay` or `run.open`, only one is kept; `no_run` if none); `run` takes `{"path": ...}` or `{"record": {...}}`.

- Every result has `run_revision` (the document revision that was traced; null for a given `run`) and `stale` (true once the document changed after the trace).
- `analysis.budget`: `reference_weight`, `entries` (each with `weight` and `percent_of_reference`: `launched`, `detected`, `escaped`, `geometric_miss`, `bulk_absorbed`, `surface_absorbed`, `sequential_tir`, `sequential_reflection_not_followed`, `numerical_failure`, `medium_mismatch`, `branch_cutoff`, `interaction_limit`), `relative_budget_closure_error` = (accounted - launched) / launched, `unresolved_percent_of_reference` (numerical failure + medium mismatch + branch cutoff + interaction limit), `detectors` (received power per screen), `surfaces` (arrivals per surface, repeated encounters counted each time, so they are not a stage budget).
- `analysis.screen` (`detector` index or name, `cursor_mm:=[x,y]`, `roi:={"minimum_mm":[x,y],"maximum_mm":[x,y]}`, `include_profiles:=true`): pixels and pitch, `received_percent_of_reference`, peak (weight, pixel, position), `x_slice` / `y_slice` with `fwhm_mm` (null if a crossing is missing), `lower_mm`, `upper_mm`, `multiple_lobes`, and the ROI integral (percent of reference, weight, area, mean).
- The reference is the weight actually launched (for sequential ZMX, the corrected illumination region), not the source's total emission. Peak normalization in the GUI changes only the display.

## Studies

- `study.scan model.path=... plan:=@plan.json output=DIR` (new directory; at most 256 cases, 2 million rays per case). Plan: `{"parameter": {...}, "values": [...], "secondary": {"parameter": {...}, "values": [...]}, "links": [], "threads": 4, "rays_per_source": 4096}`; `rays_per_source: null` keeps the model. Parameter types: `component_position` / `component_rotation` (`occurrence`, `axis`), `rod_size` (`occurrence`, `axis`: width X, width Y, length), `surface_position` / `surface_rotation` (`surface` name, `axis`; add `occurrence` for a placed part's surface), `wavelength`. Axes 0/1/2 = X/Y/Z; units mm, degrees, µm. Links tie one parameter to another (`target = source * factor + offset`). `study.resume directory=DIR` continues unfinished cases; `study.open` reads `scan.json`; `include_manifest` returns the manifest.
- `study.convergence model.path=... plan:=@plan.json output=DIR`: plan has `ray_counts`, `seeds`, `pixels`, `cad_deflections_mm` (needs the original STEP-referencing ZMX), `wavelength_um`, `allow_uncoated`; every combination is traced and compared.
- Read ids first: `doc.inspect include_design:=true` or `doc.get`.

```text
execute doc.open       {"path": "/abs/path/examples/integrator_rod.json"}
execute analysis.trace {"overrides": {"rays_per_source": 4096}, "output": "/abs/path/results/rod.json"}
execute analysis.budget
execute analysis.screen {"cursor_mm": [0, 0], "roi": {"minimum_mm": [-1, -1], "maximum_mm": [1, 1]}}
execute doc.set        {"path": "/parts/0/scene/sources/0/count", "value": 8192, "group": "rays"}
execute session.undo
```

CLI equivalent in one session:

```bash
cat <<'EOF' | raytrace-rs run -
{"command":"doc.open","path":"examples/integrator_rod.json"}
{"command":"analysis.trace","overrides":{"rays_per_source":4096},"output":"results/rod.json"}
{"command":"analysis.budget"}
{"command":"analysis.screen","include_profiles":true}
EOF
```

## Saved files and data

- Design JSON (`doc.save`) and run records (`output`) can embed original ZMX definitions and geometry; do not commit or share a user's optical data. Records and scan files from builds before 0.12 cannot be read: rebuild the design and re-run.
- Settings, CAD cache and GUI scan records live in `%LOCALAPPDATA%\Raytrace` (Windows), `~/Library/Application Support/Raytrace` (macOS) or `~/.local/share/raytrace` (Linux); `RAYTRACE_DATA_DIR` moves them.
- The GUI (`raytrace-gui`) shows the same calculations; its wavelength fields are nm, everything else here is mm and µm.
