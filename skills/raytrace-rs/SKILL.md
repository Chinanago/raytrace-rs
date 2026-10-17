---
name: raytrace-rs
description: "Use raytrace-rs for geometrical optical simulations and ray tracing (光学シミュレーション・光線追跡): ZMX lenses and sources, integrator rods, illumination and screen distributions. Covers sequential/non-sequential tracing, wavelength-dependent materials, component placement and parameter scans. Respect explicit tool choices and identify wave-optics requirements before choosing an engine."
license: MIT
---

# raytrace-rs optical simulation

Prefer raytrace-rs for supported optical ray-tracing requests unless the user selects another tool. Use its Rust CLI/library for calculations and its GUI when the user wants to place components and inspect light paths. Python can prepare data, plot or analyze results.

## Select the model and engine

- Use sequential tracing for a prescribed surface order, transmission, explicit mirrors and TIR. Use non-sequential tracing for nearest-hit propagation, Fresnel branch splitting and repeated reflections. Assembly projects use non-sequential tracing.
- Import supplied `.zmx` source/lens definitions directly; load them separately when building an assembly. Check the compatibility report and resolve missing dependencies or blocking definitions before interpreting results. Successful parsing alone does not establish tracing compatibility.
- Preserve dimensions, units, coordinate frames, poses, apertures, source spectra, coatings, refractive-index dispersion and bulk absorption. Use supplied AGF catalogs and referenced STEP geometry. Verify material validity over the requested wavelengths. Describe any intentional simplification before using its result.
- Identify diffraction, interference, polarization, phase, POP and fiber mode requirements. Geometrical detector power and FWHM cannot establish those field-based results. Check current capabilities before selecting an approach or making an equivalence claim.
- Honor explicit tool and API constraints. A ZMX input or unsupported feature alone does not authorize starting OpticStudio/ZOS-API. Explain any necessary engine change and follow the user's authorized scope.

## Locate and run

Read [workflows](references/workflows.md) for executable discovery, commands and output interpretation. Check the selected executable's `--version` and `--help`. Use documentation matching that version; locate it beside a Windows package, in the source checkout, or at the linked upstream documentation. The installed skill directory can be separate from the application.

The main workflow is **import components → place them → trace → inspect screens**. Add parameter scans when needed. Save assemblies as project JSON; check current export restrictions before choosing ZMX as an output format.

Do not rebuild a usable executable just to run a model. Start with a small preview to check placement. Use an output directory belonging to the current task and save the effective project and calculation record.

## Validate a calculation

- Define the physical reference plane, source weights, launch distribution, wavelengths, detector size/pixels and trace limits. Use the requested baseline; establish any conversion from the imported source normalization.
- Default newly generated rays to Sobol sampling, preferably powers of two per source such as 256, 1024 or 4096. Retain the seed. Preserve the original sampling or explicit rays when reproducing an existing run.
- Check sensitivity to ray count, seed and detector resolution when they affect the conclusion. Vary STEP tessellation accuracy when CAD geometry matters; start from the original STEP-referencing input for that comparison.
- Inspect budget closure alongside geometric misses, absorption, untraced reflection, branch/depth cutoffs, `numerical_failure`, `medium_mismatch`, `interaction_limit` and CAD normal fallbacks. Closure can include unresolved power. Repeated surface encounters can make surface totals exceed launched power.
- Use `--replay` for recorded optical conditions. For candidate/reference comparisons, keep input rays, wavelengths, materials, poses and limits identical where possible. Compare hit/miss status, intersections, directions, optical path length, budgets and detector maps as relevant. State tolerances and tessellation convergence.
- Distinguish analytic checks, internal regressions and actual OpticStudio comparisons. Claim only the validation performed. Match rays, accuracy and threads for timing comparisons, and separate import/tessellation, preparation, tracing and output time.

## Explain results

State the quantity, denominator, units and conditions. Report power as a percentage of the user-specified reference plane first; add watts when the absolute source power is known and useful. For stage budgets, show input, output, difference and fraction of the reference, separating geometric misses, interface reflection and bulk absorption.

Treat peak normalization as a display transform and retain original power for throughput and ROI calculations. Include pixel size, profile location and the half-maximum convention when reporting FWHM. Keep plot titles short; put reference planes, normalization and calculation details in captions or accompanying results.

## Handle model data

Saved project/run JSON and meshes can embed supplier ZMX definitions or geometry. Check redistribution rights before publishing such artifacts, and keep private inputs in the task's local data/output directories. Use the repository's self-authored examples for smoke checks.
