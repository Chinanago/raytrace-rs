"""OpticStudio reference for supported coaxial sequential fixtures.

Direct unpolarized batches compare intersections and directions. Returned
intensity is logged; Fresnel power, optical path and NSC checks are separate.
"""
import argparse
import json
import math
import os
from pathlib import Path


def wave_number(wavelength, waves):
    matches = [i + 1 for i, w in enumerate(waves) if abs(w - wavelength) < 1e-12]
    if not matches:
        raise ValueError(f"Wavelength {wavelength} um is absent from the model")
    return matches[0]


def expected_rows(reference, surface, final):
    terminals = {t["ray"]["id"]: t["ray"] for t in reference["report"].get("terminals", []) if t["reason"] == "detected"}
    paths = {(p["id"], p["surface"]): p for p in reference["report"].get("paths", [])}
    rows = {}
    for ray in reference["rays"]:
        ray_id = ray["id"]
        if final:
            if ray_id not in terminals:
                raise ValueError(f"Rust ray {ray_id} did not reach the detector")
            rows[ray_id] = terminals[ray_id]
        elif (ray_id, surface) in paths and (ray_id, surface + 1) in paths:
            hit, following = paths[ray_id, surface], paths[ray_id, surface + 1]
            delta = [b - a for a, b in zip(following["start_mm"], following["end_mm"])]
            norm = math.sqrt(sum(v * v for v in delta))
            if norm == 0:
                raise ValueError("Zero-length reference path")
            rows[ray_id] = {"position_mm": hit["end_mm"], "direction": [v / norm for v in delta]}
    return rows


def compare_rows(rows, expected, position_tolerance=1e-7, direction_tolerance=1e-9):
    ids = [r["id"] for r in rows]
    if len(set(ids)) != len(ids) or set(ids) != set(expected):
        raise ValueError("Ray identity/count mismatch")
    if any(r["error"] or r["vignette"] for r in rows):
        raise ValueError("OpticStudio error or vignetted ray")
    position = max(math.dist(r["position_mm"], expected[r["id"]]["position_mm"]) for r in rows)
    direction = max(math.dist(r["direction"], expected[r["id"]]["direction"]) for r in rows)
    if not math.isfinite(position) or not math.isfinite(direction):
        raise ValueError("Nonfinite reference difference")
    return {"maximum_position_error_mm": position, "maximum_direction_error": direction,
            "position_tolerance_mm": position_tolerance, "direction_tolerance": direction_tolerance,
            "passed": position <= position_tolerance and direction <= direction_tolerance}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rays", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    status = {"stage": "initializing", "success": False, "scope": "coaxial sequential geometry",
              "metrics_not_compared": ["Fresnel power", "optical path", "non-sequential branches"]}

    def save():
        (args.output / "status.json").write_text(json.dumps(status, indent=2), encoding="utf-8")

    save()
    app = None
    try:
        import clr
        from System import Double, Int32
        root = Path(os.environ.get("RAYTRACE_ZEMAX_ROOT", r"C:\Program Files\Ansys Zemax OpticStudio 2026 R1.02"))
        clr.AddReference(str(root / "ZOSAPI_NetHelper.dll"))
        import ZOSAPI_NetHelper
        ZOSAPI_NetHelper.ZOSAPI_Initializer.Initialize()
        clr.AddReference(str(root / "ZOSAPI_Interfaces.dll"))
        clr.AddReference(str(root / "ZOSAPI.dll"))
        import ZOSAPI
        status["stage"] = "connecting"
        save()
        app = ZOSAPI.ZOSAPI_Connection().CreateNewApplication()
        if app is None or not app.IsValidLicenseForAPI:
            status["failure_kind"] = "api_license_unavailable"
            raise RuntimeError("OpticStudio API license unavailable")
        system = app.PrimarySystem
        if args.input:
            system.LoadFile(str(args.input.resolve()), False)
        else:
            system.New(False)
            surface = system.LDE.GetSurfaceAt(1)
            aperture = surface.ApertureData.CreateApertureTypeSettings(ZOSAPI.Editors.LDE.SurfaceApertureTypes.RectangularAperture)
            aperture._S_RectangularAperture.XHalfWidth = 2.0
            aperture._S_RectangularAperture.YHalfWidth = 3.0
            surface.ApertureData.ChangeApertureTypeSettings(aperture)
            system.SaveAs(str((args.output / "rectangular_probe.zmx").resolve()))
        status["stage"] = "readback"
        status["surfaces"] = [{"index": i, "type": system.LDE.GetSurfaceAt(i).TypeName,
                               "radius_mm": str(system.LDE.GetSurfaceAt(i).Radius),
                               "thickness_mm": str(system.LDE.GetSurfaceAt(i).Thickness),
                               "material": system.LDE.GetSurfaceAt(i).Material,
                               "semi_mm": system.LDE.GetSurfaceAt(i).SemiDiameter}
                              for i in range(system.LDE.NumberOfSurfaces)]
        if args.rays:
            reference = json.loads(args.rays.read_text(encoding="utf-8"))
            rays = reference["rays"]
            if not rays or len(rays) > 20000:
                raise ValueError("Reference batches must contain 1..20000 rays")
            waves = [system.SystemData.Wavelengths.GetWavelength(i + 1).Wavelength
                     for i in range(system.SystemData.Wavelengths.NumberOfWavelengths)]
            start, final = reference.get("start_surface", 1), system.LDE.NumberOfSurfaces - 1
            if start != 1:
                raise ValueError("This probe requires input rays in surface 1 coordinates")
            if any(s["type"].replace(" ", "").lower() not in {"standard", "evenasphere"} for s in status["surfaces"][start:]):
                raise ValueError("Only coaxial Standard/Even Asphere surfaces are supported by this probe")
            checks = []
            for end in range(start + 1, final + 1):
                expected = expected_rows(reference, end - start - 1, end == final)
                if not expected:
                    continue
                tool = system.Tools.OpenBatchRayTrace()
                try:
                    data = tool.CreateDirectUnpol(len(rays), ZOSAPI.Tools.RayTrace.RaysType.Real, start, end)
                    for ray in rays:
                        if not data.AddRay(Int32(wave_number(ray["wavelength_um"], waves)),
                                           *(Double(v) for v in ray["position_mm"] + ray["direction"])):
                            raise RuntimeError("OpticStudio rejected an input ray")
                    tool.RunAndWaitForCompletion()
                    if not data.StartReadingResults():
                        raise RuntimeError("No batch results")
                    zero_i, zero_d = Int32(0), Double(0.0)
                    z_origin = sum(system.LDE.GetSurfaceAt(i).Thickness for i in range(1, end))
                    rows = []
                    while True:
                        row = data.ReadNextResult(zero_i, zero_i, zero_i, *([zero_d] * 10))
                        if not row[0]:
                            break
                        ordinal = int(row[1]) - 1
                        if not 0 <= ordinal < len(rays):
                            raise ValueError("Unexpected batch ray number")
                        rows.append({"id": rays[ordinal]["id"], "error": int(row[2]), "vignette": int(row[3]),
                                     "position_mm": [float(row[4]), float(row[5]), float(row[6]) + z_origin],
                                     "direction": [float(row[7]), float(row[8]), float(row[9])],
                                     "unpol_api_intensity": float(row[13])})
                    difference = compare_rows(rows, expected)
                    checks.append({"surface": end, "rays": rows, **difference})
                    status["surface_checks"] = checks
                    save()
                    if not difference["passed"]:
                        raise RuntimeError(f"Geometric tolerance exceeded at surface {end}")
                finally:
                    tool.Close()
            if not checks:
                raise ValueError("No comparable surfaces")
        status["success"] = True
    except Exception as error:
        status["error"] = repr(error)
    finally:
        if app is not None:
            try:
                app.CloseApplication()
            except Exception as error:
                status["cleanup_error"] = repr(error)
                status["success"] = False
    status["stage"] = "finished"
    save()
    return 0 if status["success"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
