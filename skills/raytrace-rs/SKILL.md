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

## Load and calculate

- Load `.zmx` or project JSON. `--inspect` imports and saves without tracing, and prints a compatibility report distinguishing used definitions, preserved-only data, required conditions and unsupported entries. Trace compatibility depends on those entries, even if parsing succeeds.
- ZMX imports resolve AGF materials and referenced STEP geometry. The workflow reference describes their search locations. Import diagnostics identify missing dependencies and unsupported definitions.
- Sequential mode follows the prescribed surface order, transmission, explicit mirrors and TIR. Non-sequential mode follows nearest hits and splits Fresnel reflection/transmission branches. Assembly projects use non-sequential tracing.
- CLI lengths are mm and wavelengths are μm; GUI wavelength fields use nm. Options such as `--wavelength-um`, `--rays-per-source` and `--sampling` override the corresponding model settings. `--rays-csv` supplies explicit input rays.
- The source generator's default sampling is Sobol; `--sampling pseudorandom` selects the alternative. Sobol's balance properties apply to power-of-two sample counts. Preserve the selected sampling settings or explicit rays for reproduction.

## Save, compare and inspect

- Save assemblies as project JSON. For individual-model ZMX export, check the installed version's export restrictions.
- `--replay` reruns recorded optical conditions. `--scan` and `--convergence` use the plan schemas documented in the guide; `--resume-scan` resumes a saved scan directory.
- CLI percentage fields use the launch-weight sum as their denominator. `reference_weight` and `scene.reference_plane` describe that normalization. GUI peak normalization changes the display while retaining original detector powers.
- Budget fields distinguish detected power, misses, absorption, untraced reflection and unresolved/cutoff weight. Closure includes accounted unresolved power, and surface totals include repeated encounters. See the workflow reference for exact field meanings and timing scope.
- Saved project/run JSON can embed original ZMX definitions, geometry and explicit input rays. Screen export provides CSV/PNG, profiles, FWHM and ROI data.

## Supported physics

raytrace-rs v0.11.0 implements geometrical ray tracing. Wave optics, polarization and general automatic optimization are outside that version's feature set. STEP support is limited to the documented solids, surfaces and boundaries. Consult version-matched Features documentation for exact restrictions.
