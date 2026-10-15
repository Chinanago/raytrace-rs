"""Independent OpticStudio load/readback probe. Does not overwrite input models."""
from pathlib import Path
import argparse
import json
import os
parser=argparse.ArgumentParser()
parser.add_argument("--input",type=Path)
parser.add_argument("--output",type=Path,required=True)
parser.add_argument("--rays",type=Path)
args=parser.parse_args()
args.output.mkdir(parents=True,exist_ok=True)
status={"stage":"initializing","success":False}
def save():
    (args.output/"status.json").write_text(json.dumps(status,indent=2),encoding="utf-8")
save()
try:
    import clr
    root=Path(os.environ.get("RAYTRACE_ZEMAX_ROOT", r"C:\Program Files\Ansys Zemax OpticStudio 2026 R1.02"))
    clr.AddReference(str(root/"ZOSAPI_NetHelper.dll"))
    import ZOSAPI_NetHelper
    ZOSAPI_NetHelper.ZOSAPI_Initializer.Initialize()
    clr.AddReference(str(root/"ZOSAPI_Interfaces.dll"));clr.AddReference(str(root/"ZOSAPI.dll"))
    import ZOSAPI
    status["stage"]="connecting";save()
    app=ZOSAPI.ZOSAPI_Connection().CreateNewApplication()
    try:
        if app is None or not app.IsValidLicenseForAPI:
            raise RuntimeError("OpticStudio API license unavailable")
        system=app.PrimarySystem
        if args.input:
            system.LoadFile(str(args.input.resolve()),False)
        else:
            system.New(False)
            surface=system.LDE.GetSurfaceAt(1)
            aperture=surface.ApertureData.CreateApertureTypeSettings(ZOSAPI.Editors.LDE.SurfaceApertureTypes.RectangularAperture)
            aperture._S_RectangularAperture.XHalfWidth=2.0
            aperture._S_RectangularAperture.YHalfWidth=3.0
            surface.ApertureData.ChangeApertureTypeSettings(aperture)
            system.SaveAs(str((args.output/"rectangular_probe.zmx").resolve()))
        status["stage"]="readback"
        status["surfaces"]=[{"index":i,"type":system.LDE.GetSurfaceAt(i).TypeName,"radius_mm":str(system.LDE.GetSurfaceAt(i).Radius),"thickness_mm":str(system.LDE.GetSurfaceAt(i).Thickness),"material":system.LDE.GetSurfaceAt(i).Material,"semi_mm":system.LDE.GetSurfaceAt(i).SemiDiameter} for i in range(system.LDE.NumberOfSurfaces)]
        if args.rays:
            from System import Int32,Double
            reference=json.loads(args.rays.read_text(encoding="utf-8"))
            rays=reference["rays"]
            tool=system.Tools.OpenBatchRayTrace()
            try:
                data=tool.CreateDirectUnpol(len(rays),ZOSAPI.Tools.RayTrace.RaysType.Real,1,system.LDE.NumberOfSurfaces-1)
                for ray in rays:
                    data.AddRay(Int32(1),*(Double(v) for v in ray["position_mm"]+ray["direction"]))
                tool.RunAndWaitForCompletion();data.StartReadingResults()
                zero_i=Int32(0);zero_d=Double(0.0)
                rows=[]
                while True:
                    row=data.ReadNextResult(zero_i,zero_i,zero_i,zero_d,zero_d,zero_d,zero_d,zero_d,zero_d,zero_d,zero_d,zero_d,zero_d)
                    if not row[0]:break
                    rows.append({"id":int(row[1])-1,"error":int(row[2]),"vignette":int(row[3]),"position_mm":[float(row[4]),float(row[5]),float(row[6])],"direction":[float(row[7]),float(row[8]),float(row[9])]})
                status["rays"]=rows
                rust={t["ray"]["id"]:t["ray"] for t in reference["report"]["terminals"] if t["reason"]=="detected"}
                if len(rust)!=len(rows) or any(r["error"] or r["vignette"] for r in rows):
                    raise RuntimeError("Ray-count/error/vignetting mismatch")
                position=max(sum((r["position_mm"][i]-rust[r["id"]]["position_mm"][i])**2 for i in (0,1))**0.5 for r in rows)
                direction=max(sum((r["direction"][i]-rust[r["id"]]["direction"][i])**2 for i in range(3))**0.5 for r in rows)
                status["maximum_xy_error_mm"]=position;status["maximum_direction_error"]=direction
                if position>1e-7 or direction>1e-9:raise RuntimeError("Geometric ray agreement outside tolerance")
            finally:
                tool.Close()
        status["success"]=True
    finally:
        app.CloseApplication()
except Exception as error:
    status["error"]=repr(error)
status["stage"]="finished";save()
