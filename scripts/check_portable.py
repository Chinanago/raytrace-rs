"""Verify a Windows ZIP and exercise owned models with only Windows on PATH."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import zipfile

from check_publish import embedded_document
from package_windows import FILES, ROOT, dependency_notices


def owned_step_box():
    """A closed 2 x 4 x 3 mm test solid, authored independently of a CAD app."""
    records = []
    def add(value):
        records.append(value)
        return len(records)
    def vector(values):
        return "(" + ",".join(format(v, ".17E") for v in values) + ")"
    def refs(values):
        return "(" + ",".join(f"#{v}" for v in values) + ")"
    def direction(values):
        length = math.sqrt(sum(x*x for x in values))
        return add(f"DIRECTION('',{vector([x/length for x in values])})")
    points = [(-1,-2,0),(1,-2,0),(1,2,0),(-1,2,0),(-1,-2,3),(1,-2,3),(1,2,3),(-1,2,3)]
    locations = [add(f"CARTESIAN_POINT('',{vector(p)})") for p in points]
    vertices = [add(f"VERTEX_POINT('',#{p})") for p in locations]
    edges, faces = {}, []
    for corners in [(0,3,2,1),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7),(4,5,6,7)]:
        oriented = []
        for i, a in enumerate(corners):
            b = corners[(i+1) % 4]
            key = tuple(sorted((a,b)))
            if key not in edges:
                delta = [y-x for x,y in zip(points[a],points[b])]
                axis = direction(delta)
                length = math.sqrt(sum(x*x for x in delta))
                v = add(f"VECTOR('',#{axis},{length})")
                line = add(f"LINE('',#{locations[a]},#{v})")
                edge = add(f"EDGE_CURVE('',#{vertices[a]},#{vertices[b]},#{line},.T.)")
                edges[key] = edge, a
            edge, start = edges[key]
            oriented.append(add(f"ORIENTED_EDGE('',*,*,#{edge},.{('T' if a == start else 'F')}.)"))
        loop = add(f"EDGE_LOOP('',{refs(oriented)})")
        bound = add(f"FACE_OUTER_BOUND('',#{loop},.T.)")
        a,b,c = [points[i] for i in corners[:3]]
        x = [v-u for u,v in zip(a,b)]
        y = [v-u for u,v in zip(a,c)]
        normal = [x[1]*y[2]-x[2]*y[1],x[2]*y[0]-x[0]*y[2],x[0]*y[1]-x[1]*y[0]]
        zaxis, xaxis = direction(normal), direction(x)
        frame = add(f"AXIS2_PLACEMENT_3D('',#{locations[corners[0]]},#{zaxis},#{xaxis})")
        plane = add(f"PLANE('',#{frame})")
        faces.append(add(f"ADVANCED_FACE('',(#{bound}),#{plane},.T.)"))
    shell = add(f"CLOSED_SHELL('',{refs(faces)})")
    solid = add(f"MANIFOLD_SOLID_BREP('Owned box',#{shell})")
    unit = add("(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))")
    angle = add("(NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.))")
    steradian = add("(NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT())")
    context = add(f"(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNIT_ASSIGNED_CONTEXT((#{unit},#{angle},#{steradian})) REPRESENTATION_CONTEXT('',''))")
    add(f"ADVANCED_BREP_SHAPE_REPRESENTATION('',(#{solid}),#{context})")
    return "ISO-10303-21;\nHEADER;FILE_DESCRIPTION(('Owned test'),'2;1');FILE_NAME('owned.step','',(''),(''),'raytrace','','');FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));ENDSEC;\nDATA;\n" + "".join(f"#{i}={r};\n" for i,r in enumerate(records,1)) + "ENDSEC;END-ISO-10303-21;\n"


def check(archive, output):
    if os.name != "nt":
        raise ValueError("This acceptance check targets Windows")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    prefix = f"raytrace-rs-{version}-windows-x64/"
    allowed = set(FILES.values()) | {"package.json", "START_HERE.md", "THIRD_PARTY_LICENSES.txt"}
    output.parent.mkdir(parents=True, exist_ok=True)
    checks = []
    with tempfile.TemporaryDirectory(prefix="portable_", dir=output.parent) as temporary:
        destination = Path(temporary)
        with zipfile.ZipFile(archive) as package:
            if len(package.namelist()) != len(allowed) or set(package.namelist()) != {prefix + n for n in allowed}:
                raise ValueError("Unexpected package members")
            if package.testzip() is not None:
                raise ValueError("ZIP integrity check failed")
            manifest = json.loads(package.read(prefix + "package.json"))
            if manifest["version"] != version or manifest["platform"] != "windows-x64":
                raise ValueError("Package version/platform mismatch")
            if set(manifest["sha256"]) != set(FILES.values()):
                raise ValueError("Incomplete package checksums")
            for name, digest in manifest["sha256"].items():
                if hashlib.sha256(package.read(prefix + name)).hexdigest() != digest:
                    raise ValueError(f"Package checksum mismatch: {name}")
                if name.startswith("examples/") and embedded_document(json.loads(package.read(prefix + name))):
                    raise ValueError("Example contains an original optical document")
            if package.read(prefix + "THIRD_PARTY_LICENSES.txt") != dependency_notices():
                raise ValueError("Third-party notices do not match the resolved Windows dependencies")
            checks.append("License texts for the complete resolved Windows dependency graph")
            package.extractall(destination)
        root = destination / prefix.rstrip("/")
        environment = os.environ.copy()
        system = Path(environment.get("SystemRoot", "C:/Windows"))
        environment["PATH"] = os.pathsep.join([str(system / "System32"), str(system)])
        environment["RAYTRACE_DATA_DIR"] = str(destination / "runtime")
        environment["LOCALAPPDATA"] = str(destination / "empty_appdata")
        environment["RAYTRACE_FREECAD_PYTHON"] = str(destination / "absent_python.exe")
        environment.pop("RAYTRACE_ZEMAX_ROOT", None)

        def run(binary, *args):
            result = subprocess.run([str(root / (binary + ".exe")), *map(str, args)],
                                    cwd=root, env=environment, capture_output=True,
                                    encoding="utf-8", errors="replace", timeout=90)
            if result.returncode:
                raise ValueError(f"{binary} failed: {result.stderr}")
            return result.stdout

        def record(name):
            return json.loads((root / name).read_text(encoding="utf-8"))

        for binary in ["raytrace-rs", "raytrace-gui"]:
            if run(binary, "--version").strip() != f"{binary} {version}":
                raise ValueError("Executable version mismatch")
        checks.append("GUI/CLI version without development tools on PATH")
        for sample in (root / "examples" / name for name in ("sequential_lens.json", "nonsequential_slab.json", "integrator_rod.json", "folded_mirror.json")):
            name = sample.stem + ".run.json"
            run("raytrace-rs", sample, "--rays-per-source", 256, "--threads", 2, "--output", name)
            data = record(name)
            if abs(data["relative_budget_closure_error"]) > 1e-10:
                raise ValueError("Sample power budget failed")
            checks.append("Trace owned sample: " + sample.stem)

        schema=json.loads(run("raytrace-rs","commands"))
        if schema.get("type")!="object" or not schema.get("$defs"):
            raise ValueError("Command schema is unavailable")
        run("raytrace-rs","run","examples/automation_assembly.json","--output","automation.script_report.json")
        automation=record("automation.script_report.json")
        if not automation["success"]:
            raise ValueError("JSON assembly script failed")
        generated=record("results/automation/run.json")
        run("raytrace-rs","--replay","results/automation/run.json","--threads",4,"--output","automation.replay.json")
        if record("automation.replay.json")["report"]!=generated["report"]:
            raise ValueError("JSON assembly replay differs")
        checks.append("JSON command schema, assembly script, saved project and exact replay without development tools")

        # A fresh native file verifies parsing and source-file-independent replay.
        native = root / "owned.zmx"
        native.write_text("MODE SEQ\nUNIT MM\nENPD 2\nWAVM 1 .55 1\nSURF 0\nTYPE STANDARD\nDISZ INFINITY\nSURF 1\nTYPE STANDARD\nSTOP\nDIAM 1\nDISZ 10\nSURF 2\nTYPE STANDARD\nDIAM 2\nDISZ 0\n", encoding="utf-8")
        run("raytrace-rs", native, "--inspect", "--output", "owned.project.json")
        project = record("owned.project.json")
        project["scene"]["surfaces"][-1]["pose"]["translation_mm"][2] = 12.0
        (root / "moved.project.json").write_text(json.dumps(project), encoding="utf-8")
        native.unlink()
        run("raytrace-rs", "moved.project.json", "--export-zmx", "moved.zmx", "--threads", 1, "--output", "moved.run.json")
        run("raytrace-rs", "moved.zmx", "--threads", 1, "--output", "native.run.json")
        if abs(record("native.run.json")["report"]["budget"]["detected"] - 1.0) > 1e-10:
            raise ValueError("Native roundtrip lost power")
        (root / "moved.project.json").unlink()
        (root / "moved.zmx").unlink()
        run("raytrace-rs", "--replay", "moved.run.json", "--threads", 2, "--output", "replay.run.json")
        if record("moved.run.json")["report"] != record("replay.run.json")["report"]:
            raise ValueError("Saved calculation replay differs")
        checks.extend(["Native ZMX import, placement edit, JSON save, ZMX roundtrip",
                       "Frozen calculation replay after original inputs are removed"])

        step = root / "owned.STEP"
        step.write_text(owned_step_box(), encoding="ascii")
        catalog = root / "OWNED.AGF"
        catalog.write_text("NM OWNED 1 0 1.5 50 0 0\nCD 2.25 0 0 0 0 0\nLD 0.4 0.7\n", encoding="ascii")
        cad = root / "owned_cad.zmx"
        cad.write_text("MODE NSC\nUNIT MM\nGCAT OWNED\nWAVM 1 .55 1\nPWAV 1\nSURF 1\n TYPE NONSEQCO\n NSOH NSC_IMPT 0 0 owned.STEP\n NSOP 0 0 0 0 0 0 OWNED\n NSOH NSC_DETC 0 0 Screen\n NSOP 0 0 15 0 0 0\n NSOD 1 10\n NSOD 2 10\n NSOD 3 4\n NSOD 4 4\n", encoding="ascii")
        cache_dir = destination / "runtime" / "cad_cache"
        if cache_dir.exists() and list(cache_dir.iterdir()):
            raise ValueError("STEP acceptance needs an empty conversion cache")
        run("raytrace-rs", cad, "--inspect", "--output", "cad.project.json")
        project = record("cad.project.json")
        if project["scene"]["surfaces"][0]["shape"]["type"] != "triangle_mesh":
            raise ValueError("Native STEP import produced no mesh")
        caches = list(cache_dir.glob("*_native_v1.json"))
        if len(caches) != 1:
            raise ValueError("Native STEP cache was not created")
        caches[0].write_text('{"format":"corrupt"}', encoding="ascii")
        run("raytrace-rs", cad, "--inspect", "--output", "cad.recovered.json")
        if record("cad.recovered.json")["scene"] != project["scene"]:
            raise ValueError("Corrupt STEP cache recovery changed geometry")
        project["scene"]["surfaces"][0]["pose"]["translation_mm"][0] = 0.25
        (root / "cad.moved.json").write_text(json.dumps(project), encoding="utf-8")
        csv = root / "cad.rays.csv"
        csv.write_text("id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium\n0,0,0,-5,0,0,1,1,.55,0\n", encoding="ascii")
        for original in [step,catalog,cad,*caches]:
            original.unlink()
        run("raytrace-rs", "cad.moved.json", "--rays-csv", csv, "--threads", 1, "--output", "cad.run.json")
        result = record("cad.run.json")
        if abs(result["report"]["budget"]["detected"] - 12/13) > 1e-6:
            raise ValueError("Native CAD slab optical power differs from the analytic value")
        (root / "cad.moved.json").unlink()
        csv.unlink()
        run("raytrace-rs", "--replay", "cad.run.json", "--threads", 2, "--output", "cad.replay.json")
        if result["report"] != record("cad.replay.json")["report"]:
            raise ValueError("Native CAD frozen replay differs")
        checks.extend(["Fresh STEP through ZMX without FreeCAD/Python lookup or a conversion cache",
                       "Corrupt native STEP cache recovery",
                       "CAD placement, analytic optical power, save and replay without original files/cache"])
    result = {"version": version, "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
              "checks": checks, "passed": True,
              "scope": "Windows CLI workflows, fresh native STEP via ZMX, and GUI version entry point with only Windows directories on PATH. Interactive GUI operation on another PC remains a separate check."}
    output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(f"Portable acceptance: {len(checks)} checks passed; {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "results/portable_summary.json")
    options = parser.parse_args()
    check(options.archive.resolve(), options.output.resolve())
