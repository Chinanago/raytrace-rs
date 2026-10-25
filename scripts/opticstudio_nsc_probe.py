"""Native NSC fixture comparison. Run with Python 3.12 and pythonnet.

Object parameter indices follow the installed 2026 R1 User Manual:
Standard Lens p558, Rectangular Volume p553, Source Diode p582-584,
Detector Rectangle p615. All vendor systems/ray databases stay under results/.
"""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import time
import traceback


def write(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False), encoding="utf-8")


def parameter(api, obj, index, value):
    cell = obj.GetObjectCell(getattr(api.Editors.NCE.ObjectColumn, f"Par{index}"))
    # Integer cells reject DoubleValue; use the declared cell type.
    if str(cell.DataType) == "Integer":
        cell.IntegerValue = int(value)
        actual = cell.IntegerValue
    else:
        cell.DoubleValue = float(value)
        actual = cell.DoubleValue
    if abs(actual - value) > 1e-10:
        raise ValueError(f"Parameter {index}: wrote {value}, read {actual}")


def set_pose(obj, pose):
    for key, value in zip(("XPosition", "YPosition", "ZPosition"), pose["translation_mm"]):
        setattr(obj, key, float(value))
    for key, value in zip(("TiltAboutX", "TiltAboutY", "TiltAboutZ"), pose["rotation_deg"]):
        setattr(obj, key, float(value))


def build(api, system, fixture, run, rays):
    system.New(False)
    system.MakeNonSequential()
    env = system.SystemData.Environment
    # OpticStudio uses n_air=1 in its relative glass-index convention with
    # environmental index adjustment disabled (catalog reference 20 C, 1 atm).
    env.Temperature = 20.0
    env.Pressure = 1.0
    env.AdjustIndexToEnvironment = False
    system.SystemData.Wavelengths.GetWavelength(1).Wavelength = run["wavelength_um"]
    system.SystemData.MaterialCatalogs.AddCatalog("SCHOTT")
    ns = system.SystemData.NonSequentialData
    ns.MaximumIntersectionsPerRay = fixture["settings"]["max_depth"]
    ns.MaximumSegmentsPerRay = fixture["settings"]["max_events_per_launch"]
    ns.MinimumRelativeRayIntensity = fixture["settings"]["min_relative_branch_weight"]
    ns.MinimumAbsoluteRayIntensity = 0.0
    ns.SimpleRaySplitting = False
    nce = system.NCE
    objects = []

    def add(kind, name):
        i = len(objects) + 1
        if i > nce.NumberOfObjects:
            nce.InsertNewObjectAt(i)
        obj = nce.GetObjectAt(i)
        obj.ChangeType(obj.GetObjectTypeSettings(getattr(api.Editors.NCE.ObjectType, kind)))
        obj.Comment = name
        obj.RefObject = 0
        obj.TypeData.UseGlobalXYZRotationOrder = False
        obj.TypeData.UseFastApproximateRayTrace = False
        objects.append(obj)
        return obj

    src = fixture["source"]
    d = src["diode"]
    source = add("SourceDiode", "diode")
    set_pose(source, {"translation_mm": src["origin_mm"], "rotation_deg": d["rotation_deg"]})
    values = {1: 0, 2: rays, 3: src["total_weight"], 4: 1, 5: 0,
              6: d["astigmatism_mm"], 7: d["divergence_deg"][0], 8: d["angular_supergaussian"][0],
              9: d["divergence_deg"][1], 10: d["angular_supergaussian"][1],
              11: d["array"][0], 12: d["array"][1], 13: d["spacing_mm"][0], 14: d["spacing_mm"][1],
              15: d["half_width_mm"][0], 16: d["gaussian_width_mm"][0], 17: d["spatial_supergaussian"][0],
              18: d["half_width_mm"][1], 19: d["gaussian_width_mm"][1], 20: d["spatial_supergaussian"][1]}
    for i, value in values.items():
        parameter(api, source, i, value)
    source.SourcesData.RandomPolarization = True
    optic = fixture["optic"]
    if optic:
        obj = add("StandardLens" if optic["type"] == "lens" else "RectangularVolume", "N-BK7 optic")
        set_pose(obj, optic["pose"])
        obj.Material = "N-BK7"
        if optic["type"] == "lens":
            r1, r2 = optic["radii_mm"]
            semi = optic["semi_mm"]
            values = [r1, 0, semi, semi, optic["thickness_mm"], r2, 0, semi, semi]
        else:
            x, y = optic["half_width_mm"]
            values = [x, y, optic["length_mm"], x, y, 0, 0, 0, 0]
        for i, value in enumerate(values, 1):
            parameter(api, obj, i, value)
    detector = fixture["detectors"][run["detector"]]
    obj = add("DetectorRectangle", detector["name"])
    set_pose(obj, detector["pose"])
    obj.Material = "ABSORB"
    obj.TypeData.UsePixelInterpolation = False
    for i, value in enumerate(detector["half_extent_mm"] + detector["pixels"], 1):
        parameter(api, obj, i, value)
    readback = []
    for obj in objects:
        readback.append({"object": obj.ObjectNumber, "type": obj.TypeName, "material": obj.Material,
                         "position_mm": [obj.XPosition, obj.YPosition, obj.ZPosition],
                         "rotation_deg": [obj.TiltAboutX, obj.TiltAboutY, obj.TiltAboutZ],
                         "global_xyz_rotation_order": obj.TypeData.UseGlobalXYZRotationOrder})
    return source, len(objects), detector, readback


def trace(system, mode, saved=None, timeout=300):
    import ZOSAPI
    tool = system.Tools.OpenNSCRayTrace()
    started = time.monotonic()
    try:
        tool.SplitNSCRays = mode == "fresnel"
        tool.UsePolarization = mode == "fresnel"
        tool.ScatterNSCRays = False
        # Continue after rare edge/roundoff errors. With IgnoreErrors=False,
        # OpticStudio can return a successful tool with only a partial detector
        # map after the first error. Preserve lost power in the recorded budget.
        tool.IgnoreErrors = True
        tool.SetRandomSeed(24681357)
        tool.SaveRays = saved is not None
        if saved:
            tool.SaveRaysFile = saved.name
            tool.ZRDFormat = ZOSAPI.Tools.RayTrace.ZRDFormatType.UncompressedFullData
        tool.ClearDetectors(0)
        tool.Run()
        while tool.IsRunning:
            if time.monotonic() - started > timeout:
                tool.Cancel()
                tool.WaitForCompletion()
                raise TimeoutError(f"NSC trace exceeded {timeout}s")
            time.sleep(0.1)
        tool.WaitForCompletion()
        if hasattr(tool, "Succeeded") and not tool.Succeeded:
            raise RuntimeError(f"NSC trace failed: {tool.ErrorMessage}")
        return {"seconds": time.monotonic() - started,
                "dead_ray_errors": float(tool.DeadRayErrors),
                "dead_ray_threshold": float(tool.DeadRayThreshold),
                "total_ray_energy": float(tool.GetTotalRayEnergy())}
    finally:
        tool.Close()


def detector_map(system, index, detector):
    nx, ny = detector["pixels"]
    weights = []
    # Pixel 0 is total power; positive bins start at lower-left, X fastest.
    for pixel in range(1, nx * ny + 1):
        ok, value = system.NCE.GetDetectorData(index, pixel, 0, 0.0)
        if not ok:
            raise RuntimeError(f"Detector pixel {pixel} unavailable")
        weights.append(float(value))
    ok, total = system.NCE.GetDetectorData(index, 0, 0, 0.0)
    if not ok or abs(sum(weights) - total) > 1e-8:
        raise RuntimeError(f"Detector bin sum {sum(weights)} differs from total {total}")
    return {"name": detector["name"], "pixels": detector["pixels"],
            "half_extent_mm": detector["half_extent_mm"], "weights": weights}


def read_budget(system, path, detected, detector_index, rays, trace_stats):
    categories = {k: {"status": "not_compared", "reason": "ZRD terminal/absorption mapping not yet established"}
                  for k in ("detected", "escaped", "bulk_absorbed", "surface_absorbed", "branch_cutoff", "interaction_limit", "errors")}
    categories["detected"] = {"status": "compared", "fraction": detected,
                               "mapping": "absorbing Detector Rectangle integrated flux from separate saved-ray run"}
    result = {"categories": categories, "reader": "not_run"}
    reader = None
    try:
        reader = system.Tools.OpenRayDatabaseReader()
        reader.ZRDFile = str(path)
        reader.RunAndWaitForCompletion()
        if not reader.Succeeded:
            raise RuntimeError(str(reader.ErrorMessage))
        data = reader.GetResults()
        statuses = Counter()
        samples = []
        launches = 0
        escaped = 0.0
        detected_zrd = 0.0
        error_segments = 0
        unknown_terminal = 0
        threshold_terminal = 0
        threshold = float(system.SystemData.NonSequentialData.MinimumRelativeRayIntensity) / rays
        source_sums = [0.0] * 6
        started = time.monotonic()
        while True:
            if not data.ReadNextRay():
                break
            launches += 1
            if time.monotonic() - started > 300:
                raise TimeoutError("ZRD read exceeded 300s")
            for _ in range(data.NumSegments):
                if not data.ReadNextRaySegment():
                    raise RuntimeError("Truncated ZRD segment list")
                status = ("terminated" if data.SegTerminated else "active") + (" error" if data.RayError else "")
                statuses[status] += 1
                if data.SegmentLevel == 0:
                    for j, v in enumerate((data.X, data.Y, data.Z)):
                        source_sums[j] += v
                        source_sums[j + 3] += v * v
                if data.RayError:
                    error_segments += 1
                elif data.SegTerminated:
                    if data.HitObject == 0:
                        escaped += data.Intensity
                    elif data.HitObject == detector_index:
                        detected_zrd += data.Intensity
                    elif data.Intensity <= 2.0 * threshold * (1.0 + 1e-8):
                        # An uncoated interface has at most two children. If
                        # both are dropped, the incoming segment can be marked
                        # terminated even though its intensity exceeds one
                        # child's threshold. DeadRayThreshold accounts for it.
                        threshold_terminal += 1
                    else:
                        unknown_terminal += 1
                if launches <= 3:
                    samples.append({"level": data.SegmentLevel, "parent": data.SegmentParent, "hit_object": data.HitObject, "inside_of": data.InsideOf, "status": status, "xyz": [data.X, data.Y, data.Z], "intensity": data.Intensity, "path_length": data.PathLength, "xybin": data.XYBin, "refractive_index": data.Index})
        if launches != rays:
            raise RuntimeError(f"ZRD launch count {launches} differs from requested {rays}")
        result.update(reader="succeeded", launches=launches, segment_status_counts=dict(statuses), first_three_launches=samples)
        result["source_position_moments"] = {"mean_mm": [v / launches for v in source_sums[:3]],
            "rms_mm": [(max(0.0, source_sums[j + 3] / launches - (source_sums[j] / launches) ** 2)) ** 0.5 for j in range(3)]}
        result["read_seconds"] = time.monotonic() - started
        result["unknown_terminal_segments"] = unknown_terminal
        result["threshold_terminal_segments"] = threshold_terminal
        result["zrd_detected_fraction"] = detected_zrd
        if abs(detected_zrd - detected) > 1e-7:
            raise RuntimeError(f"ZRD detector sum {detected_zrd} differs from detector {detected}")
        categories["escaped"] = {"status": "compared", "fraction": escaped,
            "mapping": "ZRD SegTerminated && !RayError && HitObject==0; excludes source level 0"}
        categories["branch_cutoff"] = {"status": "compared", "fraction": trace_stats["dead_ray_threshold"],
            "mapping": "INSCRayTrace.DeadRayThreshold (source W), includes relative threshold only; absolute threshold=0"}
        # RayError also includes intersection/segment limit failures. A nonzero
        # value cannot be split into Rust's two categories with this API.
        for key in ("errors", "interaction_limit"):
            if error_segments == 0 and trace_stats["dead_ray_errors"] == 0:
                categories[key] = {"status": "compared", "fraction": 0.0,
                    "mapping": "zero ZRD RayError segments AND zero INSCRayTrace.DeadRayErrors; limits are error conditions"}
            else:
                categories[key]["reason"] = "RayError/DeadRayErrors do not distinguish numeric errors from segment/intersection limits"
        # These owned fixtures contain uncoated dielectrics and an ABSORB
        # detector, whose absorption belongs exclusively to 'detected'.
        categories["surface_absorbed"] = {"status": "compared", "fraction": 0.0,
            "mapping": "uncoated dielectric interfaces have R+T=1; ABSORB detector assigned to detected"}
        if not error_segments and not unknown_terminal and trace_stats["dead_ray_errors"] == 0:
            bulk = trace_stats["total_ray_energy"] - detected_zrd - escaped - trace_stats["dead_ray_threshold"]
            if bulk < -1e-7:
                raise RuntimeError(f"Negative residual bulk absorption {bulk}")
            categories["bulk_absorbed"] = {"status": "compared", "fraction": max(0.0, bulk),
                "mapping": "launched - ZRD detected - ZRD escaped - DeadRayThreshold; uncoated, no scattering, no errors or unclassified terminations"}
        else:
            categories["bulk_absorbed"]["reason"] = f"Cannot isolate bulk absorption: {unknown_terminal} unclassified interface terminations, {error_segments} error segments; DeadRayErrors={trace_stats['dead_ray_errors']}"
    except Exception as error:
        result.update(reader="unusable", reason=repr(error))
        for key in categories:
            if key != "detected":
                categories[key] = {"status": "not_compared", "reason": f"ZRD reader unusable: {error!r}"}
    finally:
        if reader is not None:
            reader.Close()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--fixture")
    parser.add_argument("--rays", type=int)
    parser.add_argument("--budget-rays", type=int)
    args = parser.parse_args()
    if any(v is not None and v <= 0 for v in (args.rays, args.budget_rays)):
        parser.error("Ray counts must be positive")
    paths = sorted(args.input.resolve().glob("*/fixture.json"))
    if args.fixture:
        paths = [p for p in paths if p.parent.name == args.fixture]
    if not paths:
        parser.error("No matching fixture.json")
    app = None
    statuses = []
    failed = False
    try:
        import clr
        root = Path(os.environ.get("RAYTRACE_ZEMAX_ROOT", r"C:\Program Files\Ansys Zemax OpticStudio 2026 R1.02"))
        clr.AddReference(str(root / "ZOSAPI_NetHelper.dll"))
        import ZOSAPI_NetHelper
        ZOSAPI_NetHelper.ZOSAPI_Initializer.Initialize()
        clr.AddReference(str(root / "ZOSAPI_Interfaces.dll"))
        clr.AddReference(str(root / "ZOSAPI.dll"))
        import ZOSAPI
        app = ZOSAPI.ZOSAPI_Connection().CreateNewApplication()
        if app is None or not app.IsValidLicenseForAPI:
            raise RuntimeError("api_license_unavailable")
        system = app.PrimarySystem
        for path in paths:
            directory = path.parent
            status = {"stage": "building", "success": False}
            statuses.append((directory, status))
            write(directory / "status.json", status)
            try:
                fixture = json.loads(path.read_text(encoding="utf-8-sig"))
                rays = args.rays or fixture["source"]["count"]
                budget_rays = args.budget_rays or fixture["budget_rays"]
                output = {"fixture": fixture["name"], "opticstudio_version": f"{app.ZOSMajorVersion}.{app.ZOSMinorVersion}.{app.ZOSSPVersion}", "runs": []}
                write(directory / "opticstudio.json", output)
                for run in fixture["runs"]:
                    print(f'{fixture["name"]}/{run["name"]}: {rays} rays', flush=True)
                    source, index, detector, readback = build(ZOSAPI, system, fixture, run, rays)
                    model = directory / f'opticstudio_{run["name"]}.zmx'
                    system.SaveAs(str(model))
                    system.SaveAs(str(directory / "opticstudio.zmx"))
                    status["stage"] = f'tracing {run["name"]}'
                    write(directory / "status.json", status)
                    elapsed = trace(system, run["mode"])
                    map_data = detector_map(system, index, detector)
                    parameter(ZOSAPI, source, 2, budget_rays)
                    system.SaveAs(str(model))
                    zrd = model.with_suffix(".ZRD")
                    budget_elapsed = trace(system, run["mode"], zrd)
                    small_map = detector_map(system, index, detector)
                    budget = read_budget(system, zrd, sum(small_map["weights"]), index, budget_rays, budget_elapsed)
                    parameter(ZOSAPI, source, 2, rays)
                    system.SaveAs(str(model))
                    ns = system.SystemData.NonSequentialData
                    result = {"name": run["name"], "rays": rays, "budget_rays": budget_rays, "launched_weight": 1.0,
                              "totals": {"launched_W": 1.0, "detected_W": sum(map_data["weights"])},
                              "detectors": [map_data], "budget": budget, "objects": readback,
                              "settings": {"use_polarization": run["mode"] == "fresnel", "split_nsc_rays": run["mode"] == "fresnel", "random_polarization": source.SourcesData.RandomPolarization,
                                           "scatter": False, "ignore_errors": True, "minimum_relative_intensity": ns.MinimumRelativeRayIntensity,
                                           "minimum_absolute_intensity": ns.MinimumAbsoluteRayIntensity, "max_intersections": ns.MaximumIntersectionsPerRay,
                                           "max_segments": ns.MaximumSegmentsPerRay, "temperature_C": 20.0, "pressure_atm": 1.0,
                                           "adjust_index_to_environment": False, "air_convention": "n_air=1 relative catalog index; Rust uses same relative N-BK7 index", "detector_material": "ABSORB",
                                           "use_pixel_interpolation": False, "random_seed": 24681357, "zrd_format": "UncompressedFullData",
                                           "source_sampling_method": str(source.SourcesData.SamplingMethod),
                                           "known_model_difference": "geometric: polarization OFF suppresses OpticStudio bulk absorption, so Rust also drops AGF absorption in this mode. fresnel: Rust averages S/P independently per interface; OpticStudio preserves polarization state.",
                                           "pixel_order": "row-major +Y, X fastest; integrated W per pixel; pixel 1 lower left"},
                              "timings_s": {"map_trace": elapsed, "budget_trace": budget_elapsed}}
                    output["runs"].append(result)
                    write(directory / "opticstudio.json", output)
                status.update(stage="complete", success=True)
            except Exception as error:
                failed = True
                status.update(stage="failed", error=repr(error), traceback=traceback.format_exc(), failure_kind="fixture_or_trace_failure")
                print(status["traceback"], flush=True)
            finally:
                write(directory / "status.json", status)
    except Exception as error:
        failed = True
        for path in paths:
            write(path.parent / "status.json", {"stage": "connecting", "success": False,
                  "failure_kind": "api_license_unavailable" if "license" in str(error).lower() else "connection_failure",
                  "error": repr(error), "traceback": traceback.format_exc()})
        print(traceback.format_exc(), flush=True)
    finally:
        if app is not None:
            app.CloseApplication()
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
