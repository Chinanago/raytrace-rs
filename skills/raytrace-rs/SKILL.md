---
name: raytrace-rs
description: "Operate raytrace-rs when the user requests this tool or is working with a raytrace-rs project. Documents ZMX/JSON import, CLI and GUI operations, sequential/non-sequential tracing, materials, screens, parameter scans, convergence and replay."
license: MIT
---

# raytrace-rs usage

This skill describes raytrace-rs commands, inputs, outputs and limitations. Tool selection, physical assumptions, calculation settings and presentation requirements come from the current task.

## Locate the application

Read [workflows](references/workflows.md) for executable discovery, commands and output fields. Check the selected executable's `--version` and `--help`, then use matching documentation from the application package, source checkout or linked upstream copy. The skill can be installed separately from the application.

The CLI runs models and writes calculation records. The GUI provides component placement, light-path viewing, screen displays and project editing. The Rust library exposes the calculation and export functions.

## Choose the interface

When the MCP tools of raytrace-rs are available (`list_commands`, `describe_command`, `execute`, `batch`; the server is started by `raytrace-rs mcp`, and the Claude Code plugin registers it), prefer them to the CLI: the server keeps one session, so `execute doc.open`, edits with `doc.set`/`component.*`, `analysis.trace` without `model` and `session.undo` work on the same open design without reloading it. Call `list_commands` once, call `describe_command` before using a command you do not know, and send long calculations with `execute` (they can be cancelled and report progress). Otherwise use the CLI (`raytrace-rs COMMAND ...`, or `raytrace-rs run -` for a session). Both run the same commands, so the field names below apply to either.

## Load and calculate

- Check `--help`. v0.11.0 takes options such as `--inspect` and `--replay`. Newer builds take command parameters instead (`raytrace-rs analysis.trace model.path=lens.zmx overrides.rays_per_source:=1024`; current builds accept only dotted ids, not the old short names such as `trace`), print one JSON response per request on stdout, accept JSON with `raytrace-rs run -` (one session per stream: `doc.open`, `doc.set`, `analysis.trace`, `session.undo` without `model`), and describe every command and field with `raytrace-rs --help`, `raytrace-rs COMMAND --help` and `raytrace-rs schema.get name=NAME`. The workflow reference maps the old options to fields.
- Load `.zmx` or project JSON. `--inspect` imports and saves without tracing, and prints a compatibility report distinguishing used definitions, preserved-only data, required conditions and unsupported entries. Trace compatibility depends on those entries, even if parsing succeeds.
- ZMX imports resolve AGF materials and referenced STEP geometry. The workflow reference describes their search locations. Import diagnostics identify missing dependencies and unsupported definitions.
- Sequential mode follows the prescribed surface order, transmission and explicit mirrors; TIR terminates the ray. Non-sequential mode follows nearest hits and splits Fresnel reflection/transmission branches. Assembly projects use non-sequential tracing.
- CLI lengths are mm and wavelengths are μm; GUI wavelength fields use nm. Options such as `--wavelength-um`, `--rays-per-source` and `--sampling` override the corresponding model settings. `--rays-csv` supplies explicit input rays.
- The source generator's default sampling is Sobol; `--sampling pseudorandom` selects the alternative. Sobol's balance properties apply to power-of-two sample counts. Preserve the selected sampling settings or explicit rays for reproduction.

## Save, compare and inspect

- Save assemblies as design JSON (project JSON in builds before the design document; newer builds open both). For individual-model ZMX export, check the installed version's export restrictions.
- `--replay` reruns recorded optical conditions. `--scan` and `--convergence` use the plan schemas documented in the guide; `--resume-scan` resumes a saved scan directory.
- CLI percentage fields use the launch-weight sum as their denominator. `reference_weight` and `scene.reference_plane` describe that normalization. GUI peak normalization changes the display while retaining original detector powers.
- Budget fields distinguish detected power, misses, absorption, untraced reflection and unresolved/cutoff weight. Closure includes accounted unresolved power, and surface totals include repeated encounters. See the workflow reference for exact field meanings and timing scope.
- Saved project/run JSON can embed original ZMX definitions, geometry and explicit input rays. Screen export provides CSV/PNG, profiles, FWHM and ROI data.

## Supported physics

raytrace-rs v0.11.0 implements geometrical ray tracing. Wave optics, polarization and general automatic optimization are outside that version's feature set. STEP support is limited to the documented solids, surfaces and boundaries. Consult version-matched Features documentation for exact restrictions.
