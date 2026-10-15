"""Tessellate a STEP solid once. Called by the Rust importer, never by tracing."""
import json
import math
from pathlib import Path
import sys
import FreeCAD
import Part

shape = Part.Shape()
shape.read(str(Path(sys.argv[1]).resolve()))
deflection_mm = float(sys.argv[3]) if len(sys.argv) > 3 else 0.02
if not math.isfinite(deflection_mm) or deflection_mm <= 0:
    raise ValueError("CAD deflection must be finite and positive")
if not shape.isValid() or len(shape.Solids) != 1:
    raise ValueError("Exactly one valid closed STEP solid is required")
vertices, normals, triangles = [], [], []
for face in shape.Faces:
    points, cells = face.tessellate(deflection_mm)
    offset = len(vertices)
    face_normals = []
    for p in points:
        u, v = face.Surface.parameter(p)
        n = face.normalAt(u, v)
        face_normals.append([n.x, n.y, n.z])
        vertices.append([p.x, p.y, p.z])
        normals.append(face_normals[-1])
    for a, b, c in cells:
        cross = (points[b] - points[a]).cross(points[c] - points[a])
        n = face_normals[a]
        if cross.x*n[0]+cross.y*n[1]+cross.z*n[2] < 0:
            b, c = c, b
        triangles.append([offset+a, offset+b, offset+c])
result = {"type": "triangle_mesh", "vertices": vertices, "normals": normals, "triangles": triangles}
out = Path(sys.argv[2])
temporary = out.with_suffix(".tmp")
temporary.write_text(json.dumps(result, separators=(",", ":")), encoding="utf-8")
temporary.replace(out)
