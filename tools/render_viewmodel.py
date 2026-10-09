"""Render a DOOM Eternal weapon (OBJ from the user's own install via samuel-cli) into the
first-person sprite doomslayer.dll draws in the HUD.

    blender -b -P tools/render_viewmodel.py -- <model.obj> <out.png> [yaw pitch roll dx dy dz scale] [--skip a,b]

The gun sits bottom-right like Doom Eternal's viewmodel; the DLL animates recoil, bob and swaps.
"""

import math
import sys
from pathlib import Path

import bpy
from mathutils import Euler, Vector

argv = sys.argv[sys.argv.index("--") + 1:]
skip = []
if "--skip" in argv:
    i = argv.index("--skip")
    skip = argv[i + 1].split(",")
    argv = argv[:i] + argv[i + 2:]
obj_path, out_path = Path(argv[0]), Path(argv[1])
nums = [float(v) for v in argv[2:9]] if len(argv) >= 9 else [0, 0, 0, 0, 0, 0, 1]
yaw, pitch, roll, dx, dy, dz, scale = nums

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene

bpy.ops.wm.obj_import(filepath=str(obj_path), forward_axis="NEGATIVE_Z", up_axis="Y")
meshes = [o for o in scene.objects if o.type == "MESH"]
for o in meshes:
    if any(s and s in o.name for s in skip):
        bpy.data.objects.remove(o)
meshes = [o for o in scene.objects if o.type == "MESH"]

# Materials: Doom's _n (normal) / _s (specular) next to the base colour.
img_dir = obj_path.parent / "images"
for o in meshes:
    for slot in o.material_slots:
        m = slot.material
        if not m:
            continue
        m.use_nodes = True
        nt = m.node_tree
        nt.nodes.clear()
        out = nt.nodes.new("ShaderNodeOutputMaterial")
        bsdf = nt.nodes.new("ShaderNodeBsdfPrincipled")
        nt.links.new(bsdf.outputs[0], out.inputs[0])
        base = img_dir / f"{m.name}.png"
        if base.exists():
            t = nt.nodes.new("ShaderNodeTexImage")
            t.image = bpy.data.images.load(str(base))
            nt.links.new(t.outputs["Color"], bsdf.inputs["Base Color"])
            nt.links.new(t.outputs["Alpha"], bsdf.inputs["Alpha"])
        n = img_dir / f"{m.name}_n.png"
        if n.exists():
            t = nt.nodes.new("ShaderNodeTexImage")
            t.image = bpy.data.images.load(str(n))
            t.image.colorspace_settings.name = "Non-Color"
            nm = nt.nodes.new("ShaderNodeNormalMap")
            nt.links.new(t.outputs["Color"], nm.inputs["Color"])
            nt.links.new(nm.outputs["Normal"], bsdf.inputs["Normal"])
        s = img_dir / f"{m.name}_s.png"
        e = img_dir / f"{m.name}_e.png"
        if e.exists():
            t = nt.nodes.new("ShaderNodeTexImage")
            t.image = bpy.data.images.load(str(e))
            nt.links.new(t.outputs["Color"], bsdf.inputs["Emission Color"])
            bsdf.inputs["Emission Strength"].default_value = 4.0
        if s.exists():
            t = nt.nodes.new("ShaderNodeTexImage")
            t.image = bpy.data.images.load(str(s))
            t.image.colorspace_settings.name = "Non-Color"
            inv = nt.nodes.new("ShaderNodeInvert")
            nt.links.new(t.outputs["Color"], inv.inputs["Color"])
            nt.links.new(inv.outputs["Color"], bsdf.inputs["Roughness"])
            bsdf.inputs["Metallic"].default_value = 0.6
        else:
            bsdf.inputs["Roughness"].default_value = 0.45

# Normalise: centre the gun on its bounding box and scale it to the Super Shotgun's length, so
# one pose (tuned on the SSG) frames every weapon the same way.
corners = [o.matrix_world @ Vector(c) for o in meshes for c in o.bound_box]
lo = Vector([min(c[i] for c in corners) for i in range(3)])
hi = Vector([max(c[i] for c in corners) for i in range(3)])
centre = (lo + hi) / 2
length = hi.x - lo.x
norm = 0.82 / max(length, 1e-3)
for o in meshes:
    o.location -= centre
pivot = bpy.data.objects.new("pivot", None)
scene.collection.objects.link(pivot)
for o in meshes:
    o.parent = pivot
print(f"normalise: length {length:.2f} m -> scale {norm:.2f}, height {(hi.z - lo.z):.2f}")
scale *= norm
# Taller guns (BFG, chaingun) sit a little lower so they don't cover the crosshair.
dz -= max(0.0, (hi.z - lo.z) * norm - 0.22) * 0.5
pivot.rotation_euler = Euler((math.radians(roll), math.radians(pitch), math.radians(yaw)))
pivot.location = Vector((dx, dy, dz))
pivot.scale = (scale, scale, scale)

# Camera at the origin looking down +X (gun barrel axis), Z up.
cam_data = bpy.data.cameras.new("cam")
cam_data.lens_unit = "FOV"
cam_data.angle = math.radians(62)
cam_data.clip_start = 0.01
cam = bpy.data.objects.new("cam", cam_data)
# Look down +X with Z up (screen right = -Y).
cam.rotation_euler = Vector((1, 0, 0)).to_track_quat("-Z", "Y").to_euler()
scene.collection.objects.link(cam)
scene.camera = cam

# Lighting tuned toward Elden Ring's warm overcast daylight.
def light(kind, energy, rot, color, size=1.0):
    ld = bpy.data.lights.new(kind, kind)
    ld.energy = energy
    ld.color = color
    if kind == "AREA":
        ld.size = size
    lo = bpy.data.objects.new(kind, ld)
    lo.rotation_euler = Euler([math.radians(a) for a in rot])
    scene.collection.objects.link(lo)
    return lo

light("SUN", 3.2, (40, -20, 30), (1.0, 0.93, 0.82))
fill = light("AREA", 18.0, (0, 70, 180), (0.7, 0.8, 1.0), 2.0)
fill.location = (0.4, 0.8, 0.3)
world = bpy.data.worlds.new("w")
world.use_nodes = True
world.node_tree.nodes["Background"].inputs[0].default_value = (0.20, 0.21, 0.22, 1)
world.node_tree.nodes["Background"].inputs[1].default_value = 0.8
scene.world = world

for engine in ("BLENDER_EEVEE", "BLENDER_EEVEE_NEXT"):
    try:
        scene.render.engine = engine
        break
    except TypeError:
        continue
scene.render.film_transparent = True
scene.render.resolution_x = 1600
scene.render.resolution_y = 900
scene.render.image_settings.file_format = "PNG"
scene.render.image_settings.color_mode = "RGBA"
scene.view_settings.view_transform = "AgX" if "AgX" in [v.identifier for v in bpy.types.ColorManagedViewSettings.bl_rna.properties["view_transform"].enum_items] else "Filmic"
scene.render.filepath = str(out_path)
bpy.ops.render.render(write_still=True)
print("rendered", out_path)

# Muzzle = centre of the gun's +X face, projected to normalised screen coords for the flash.
import json
from bpy_extras.object_utils import world_to_camera_view
bpy.context.view_layer.update()
muzzle_local = Vector((hi.x - centre.x, 0.0, (hi.z + lo.z) / 2 - centre.z))
muzzle_world = pivot.matrix_world @ muzzle_local
ndc = world_to_camera_view(scene, cam, muzzle_world)
meta = {"muzzle": [ndc.x, 1.0 - ndc.y]}
out_path.with_suffix(".json").write_text(json.dumps(meta))
print("muzzle", meta)
