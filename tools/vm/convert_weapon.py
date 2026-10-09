"""Bake a DOOM Eternal first-person weapon (arms + gun + animations) for doomslayer.dll.

Input: Vega's Cast export of the user's own Doom install (~/modtools/vega/exported_files).
Output: dist/natives/doom_vm/<name>/  ->  model.bin (meshes + baked clips) + textures (PNG).

Conventions:
  * Doom/Cast space: X forward, Y left, Z up, centimetres.
  * Every clip is evaluated per frame (30 fps), expressed relative to the animated `camera` bone
    (so Doom's camera kick is baked in) and converted to view space: x right, y up, z forward (m).
  * Per bone we store the 3x4 skinning matrix  V * inv(camera) * global * inv(bind_global).
  * The gun's root follows the arms' `righthandattach` bone.

    python tools/vm/convert_weapon.py super_shotgun
"""

import os
import json
import struct
import sys
from pathlib import Path

import numpy as np

# Inputs: "vega" = Vega's Cast exports (development), "doom" = read straight from the user's DOOM
# Eternal install with our own md6 readers (installer; tools/vm/doom_source.py). Both give the same
# model.bin (checked 2026-10-09).
SOURCE = os.environ.get("DOOMRING_VM_SOURCE", "vega")
sys.path.insert(0, str(Path(__file__).resolve().parent))
if SOURCE == "doom":
    cast = None
    import doom_source  # noqa: E402
    import md6tex  # noqa: E402
else:
    sys.path.insert(0, str(Path.home() / "doom-extract" / "py"))
    import cast  # noqa: E402

VEGA = Path.home() / "modtools" / "vega" / "exported_files"
OUT_ROOT = Path(os.environ.get("DOOMRING_VM_OUT", Path(__file__).resolve().parents[2] / "dist" / "natives" / "doom_vm"))
# marine.md6mesh is the Slayer's real first-person body (Praetor suit arms, torso, legs, wrist blade).
# arms.md6mesh is only the naked base body under the suit - its hands look like bare skin.
ARMS = VEGA / "models/md6/player/human/base/assets/mesh/marine.cast"
ARMS_ANIM = VEGA / "animations/md6/player/human/base/motion/core"
GUN_MODELS = VEGA / "models/md6/objects/weapons"
GUN_ANIM = VEGA / "animations/md6/objects/weapons"

# name -> (folder under core/ and objects/weapons/, gun model file, mesh material substrings to drop,
#          {standard clip: (arms clip, gun clip)})
STD = {"idle": ("idle", "idle_pose"), "bringup": ("bringup", "idle_pose"),
       "bringdown": ("bringdown", "idle_pose"), "dryfire": ("dryfire", "dryfire")}
WEAPONS = {
    # Sticky Bombs gun: base + guts + the poprocket parts (MESH_MODES hides the base barrel, knob and
    # pump like Doom's mod decls do; Full Auto's tripleburst parts are dropped).
    "combat_shotgun": ("shotguns/shotgun", "shotgun.cast", ["mastery", "tripleburst"],
                       {**STD, "fire": ("shoot_delay", "shoot_delay"),
                        "sticky": ("shootpoprocket", "shootpoprocket"),
                        # Last bomb: the loader stays back (empty) until the magazine reloads;
                        # the reload is the end of Doom's switch_to_poprockets (loader back, new
                        # bombs in, loader forward), from frame 27.
                        "sticky_last": ("shootpoprocket", "shootpoprocket_last"),
                        "sticky_empty": ("idle", "shootpoprocket_last", {"gun_frame": "last"}),
                        # Doom's sticky reload (shotgun_secondary_pop_rockets.decl:
                        # dischargeOverheatedAdditiveAnim = additive_poprockets_reload, gun only,
                        # only in the level resources - exported from e1m1_intro): on top of the
                        # idle pose (as Doom layers it): the loader opens, the new bombs go in, it ends
                        # open with the tube in the chamber. 54 frames.
                        "sticky_reload": ("idle", "additive/poprockets_reload",
                                          {"gun_base": "idle_pose", "len": "gun"})}),
    # Precision Bolt (heavy_cannon_bolt_action.decl) has no showHideMeshInfo: Doom shows every part,
    # incl. the front pod + magazine (missle_mod / missiles_mod_inside) - user saw them missing.
    "heavy_cannon": ("assaultrifles/heavy_cannon", "heavy_cannon.cast", ["mastery"],
                     # User: the front piece (barrel_part02 + the pod riding on it) slides further back.
                     {**STD, "fire": ("shoot", "shoot", {"amp": {"barrel_part02_md": 1.8, "missle_part01": 1.8}}), "reload": ("reload", "reload"),
                      "zoom_idle": ("zoomidle", "idle_pose"), "zoom_fire": ("zoomshootstate_01", "shoot", {"amp": {"barrel_part02_md": 1.8, "missle_part01": 1.8}}),
                      "zoom_into": ("zoomshootstate_into", "idle_pose"), "zoom_out": ("zoomshootstate_recover", "idle_pose")}),
    "plasma_rifle": ("energy/plasmarifle", "plasmarifle.cast", ["mastery", "mod_00", "mod_01"],
                     {**STD, "fire": ("shootstate_01", "shootstate"), "recover": ("shootstate_recover", "shootstate_recover"),
                      "heat_blast": ("shoot_heatblast", "shoot_heatblast")}),
    "rocket_launcher": ("heavy/rocketlauncher", "rocketlauncher.cast", ["mastery", "rocket_lockon", "rocket_mod_2"],
                        {**STD, "idle": ("idle", "idle"), "fire": ("shoot_delay", "shoot_delay")}),
    "super_shotgun": ("shotguns/double_barrel", "double_barrel.cast", ["mastery", "supershotgun_base#4262"],
                      {**STD, "fire": ("shoot_reload", "shoot_reload"), "fire_single": ("shoot", "shoot"),
                       "reload": ("reloadfromempty", "reloadfromempty"),
                       # Meathook: Doom's additive layers (arms from the doom1 set, gun hook/chain) on
                       # top of the idle pose.
                       "hook_shoot": ("shoot_meathook", "additive/shoot_meathook",
                                      {"arms_path": "player/human/doom1/motion/core/shotguns/double_barrel/additive/shoot_meathook",
                                       "arms_base": "idle", "gun_base": "idle_pose_meathook", "frames": "max"}),
                       "hook_retract": ("retract_meathook", "additive/retract_meathook",
                                        {"arms_path": "player/human/doom1/motion/core/shotguns/double_barrel/additive/retract_meathook",
                                         "arms_base": "idle", "gun_base": "idle_pose_meathook", "frames": "max"}),
                       # No (valid) target: Doom's "discharge" - the hook's shoot layer played out and
                       # straight back (double_barrel_meat_hook.decl: dischargeAdditive*, reversed, 300 ms).
                       "hook_notarget": ("shoot_meathook", "additive/shoot_meathook",
                                         {"arms_path": "player/human/doom1/motion/core/shotguns/double_barrel/additive/shoot_meathook",
                                          # attack: the tips snap open in the first 15% and close slowly.
                                          "arms_base": "idle", "gun_base": "idle_pose_meathook", "frames": "max", "attack": 0.15,
                                          # User: only the hook tips opening and a slight gun movement,
                                          # eased in and back out.
                                          "envelope": True, "arms_w": 0.12,
                                          # (negative: the tips close in fast, then open slowly - user)
                                          "gun_bones": {"hooktip_hooktip": -1.0}}),
                       # The same sampling with no additive: the renderer layers
                       # (hook_notarget x inverse(hook_notarget_base)) on top of the live idle.
                       "hook_notarget_base": ("shoot_meathook", "additive/shoot_meathook",
                                              {"arms_path": "player/human/doom1/motion/core/shotguns/double_barrel/additive/shoot_meathook",
                                               "arms_base": "idle", "gun_base": "idle_pose_meathook", "frames": "max", "attack": 0.15,
                                               "envelope": True, "arms_w": 0.0, "gun_bones": {}})}),
    "ballista": ("energy/gauss_rifle", "gauss_rifle.cast", ["mastery", "destroyer", "base_mod", "glow"],
                 # Doom's normal shot is shoot_delay_ballista: the shot plus the reload cycle (string,
                 # limbs and rails moving); "shoot" alone was only the 5-frame kick.
                 {**STD, "fire": ("shoot_delay_ballista", "shoot_delay_ballista"),
                  "arb_to_idle": ("ballista_to_idle", "ballista_to_idle", {"trim": True}),
                  "arb_into": ("ballista_into", "ballista_into"), "arb_charge": ("ballista_charge", "ballista_charge"),
                  "arb_idle": ("ballista_charged_idle", "ballista_charged_idle"), "arb_fire": ("ballista_fire", "ballista_fire"),
                  "arb_out": ("ballista_out", "ballista_out")}),
    # Every part is kept (auger = energy shield mod, never shown); MESH_MODES picks per mode.
    "chaingun": ("heavy/chaingun", "chaingun.cast", ["mastery", "auger"],
                 {**STD, "fire": ("shootstate_01", "shootstate_01", {"gun_loop": True}), "recover": ("shootstate_recover", "shootstate_recover"),
                  "dryfire": ("dryfirestate", "dryfirestate"),
                  "turret_into": ("turretmode_into", "turretmode_into"), "turret_idle": ("turretmode_idle", "turretmode_idle"),
                  "turret_fire": ("turretmode_shootstate_01", "turretmode_shootstate_01", {"gun_loop": True}),
                  "turret_recover": ("turretmode_shootstate_recover", "turretmode_shootstate_recover"),
                  "turret_out": ("turretmode_out", "turretmode_out")}),
    "bfg": ("energy/bfg", "bfg.cast", ["mastery"],
            # (Doom's own gun bringup moves the whole gun: user found it over the top - arms only.)
            {**STD, "idle": ("idle", "idle"),
             # Doom's wind-up (fins, flaps, spinning core) before the shot, then the shot itself
             # (its gun clip runs longer than the arms one).
             "charge": ("charge", "charge_sphere_arc"),
             # charge_shoot_delay (user prefers it to the short charge_shoot), cut at the arms clip's
             # end (no frozen arms) and trimmed; the renderer then blends into idle over 0.3 s.
             "fire": ("charge_shoot_delay", "charge_shoot_sphere_arc", {"trim": True})}),
    # Arms only (no gun): Doom melee and Blood Punch ("knockback" is Doom's charged punch).
    "fists": ("melee/fists", None, [],
              {"idle": ("idle", None), "punch_l": ("meleeleft", None), "punch_r": ("meleeright", None),
               "punch_l2": ("meleeleft2", None), "punch_r2": ("meleeright2", None),
               "bloodpunch": ("knockback_1", None), "bringup": ("bringup", None)}),
    "chainsaw": ("melee/chainsaw", "chainsaw.cast", ["mastery"],
                 {"idle": ("idle", "idle"), "bringup": ("bringup", "idle"), "fire": ("melee1_1", "idle"),
                  "rev": ("rev_into", "idle")}),
    # The Crucible (user spec, MODLOG): draw = bringup_activate (hilt up, blade ignites - the same
    # draw every time), Doom's left / right swings, put away = deactivate. The blade's own clips:
    # bringup_activate / activate / deactivate / idle_pose.
    "crucible": ("melee/crucible", "crucible.cast", [],
                 {"idle": ("idle", "idle_pose"), "bringup": ("bringup_activate", "bringup_activate"),
                  "swing_into": ("swing_into", "idle_pose"), "swing_r1": ("swing_right_first", "idle_pose"),
                  "swing_l1": ("swing_left_01", "idle_pose"), "swing_l2": ("swing_left_02", "idle_pose"),
                  "swing_r2": ("swing_right_01", "idle_pose"), "swing_r3": ("swing_right_02", "idle_pose"),
                  "swing_l_out": ("swing_left_out", "idle_pose"), "swing_r_out": ("swing_right_out", "idle_pose"),
                  "bringdown": ("deactivate", "deactivate")}),
}

# Which gun parts each mode shows (Doom's weapon decls: showHideMeshInfo), by md6mesh part name
# (the name before "$"). 1 = normal mode only, 2 = Chaingun turret mode only; unlisted = always.
MESH_MODES = {
    # The sticky ammo tube is mode 1 only: hidden (mode 2) while the magazine is empty.
    "combat_shotgun": {"combatshotgun_base_barrel_knob": 9, "combatshotgun_base_barrel": 9, "combatshotgun_pump": 9,
                       "combatshotgun_base_guts": 9, "combatshotgun_poprocket_ammo_loader": 0,
                       "combatshotgun_poprocket_ammo": 1},
    # 9 = kept in the model but never shown (user: hide the Heavy Cannon's missile parts for now).
    "heavy_cannon": {"missle_mod": 9, "missiles_mod_inside": 9},
    "chaingun": {"default": 1, "nonturret": 1, "chaingun_base_handle_rear": 1, "chaingun_base_barrel_ammo": 1,
                 "turret": 2, "chaingun_turret_ammo": 2},
}
DOOM_BASE = Path("C:/Program Files (x86)/Steam/steamapps/common/DOOMEternal/base")
SAMUEL = Path.home() / "modtools/samuel-src/build/samuel-cli.exe"
RAW = Path.home() / "doom-extract/meshnames"


def md6_part_names(mesh_res):
    """Part names of an md6mesh in mesh order (its header lists name + material per mesh; Vega's
    Cast export drops the names but keeps the order)."""
    if SOURCE == "doom":
        return doom_model(mesh_res).part_names
    import re
    import subprocess
    fn = mesh_res.replace("/", "_") + ".baseModel.bin"
    f = RAW / fn
    if not f.exists():
        RAW.mkdir(parents=True, exist_ok=True)
        subprocess.run([str(SAMUEL), str(DOOM_BASE / "gameresources.resources"), "raw", str(RAW), mesh_res], check=True,
                       capture_output=True)
    b = f.read_bytes()
    names = []
    strs = [(m.start(), m.group()) for m in re.finditer(rb"[ -~]{3,}", b[:65536])]
    for (o, a), (_, nxt) in zip(strs, strs[1:]):
        if a.startswith(b"art/"):
            if names and names[-1] is None:
                break
            continue
        # "<name><len byte of the material path>" then the material path
        if nxt.startswith(b"art/") and len(a) > 1 and a[-1] == len(nxt):
            names.append(a[:-1].decode())
    return names


# Barrel spin groups: (group, subtree roots, pivot bone). Chaingun group 0 = the whole rotary
# cluster (main barrel + the four turret barrels around it) in normal mode, group 1 = each turret
# barrel about its own axis once the turret has unfolded.
SPIN = {
    # (Turret-mod chaingun: the four turret barrels are the rotary cluster in normal mode.)
    "chaingun": [(0, ["barrel_part01_md", "tbarrel_part01_md"], "barrel_part01_md")]
    + [(1, [b], b) for b in ("tblower_part02_lf", "tblower_part02_rt", "tbupper_part02_lf", "tbupper_part02_rt")],
}

# View transform: Doom (X fwd, Y left, Z up, cm) -> view (x right, y up, z fwd, m).
V = np.array([[0, -1, 0, 0], [0, 0, 1, 0], [1, 0, 0, 0], [0, 0, 0, 1]], dtype=np.float64)
V[:3, :3] *= 0.01


def quat_to_mat(q):
    x, y, z, w = q
    return np.array([
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ])


def trs(t, q):
    m = np.eye(4)
    m[:3, :3] = quat_to_mat(q)
    m[:3, 3] = t
    return m


def slerp(a, b, t):
    a = np.asarray(a, float)
    b = np.asarray(b, float)
    d = np.dot(a, b)
    if d < 0:
        b, d = -b, -d
    if d > 0.9995:
        r = a + (b - a) * t
        return r / np.linalg.norm(r)
    th = np.arccos(d)
    return (np.sin((1 - t) * th) * a + np.sin(t * th) * b) / np.sin(th)


def doom_name(vega_path, ext):
    """A Vega export path (<VEGA>/models|animations/md6/...cast) -> the Doom resource name."""
    rel = Path(vega_path).relative_to(VEGA).as_posix()
    return rel.split("/", 1)[1].rsplit(".", 1)[0] + ext


_doom_models = {}


def doom_model(res_name):
    if res_name not in _doom_models:
        _doom_models[res_name] = doom_source.Model(res_name)
    return _doom_models[res_name]


def clip_exists(path):
    if SOURCE == "doom":
        return doom_source.anim(doom_name(path, ".md6anim")) is not None
    return path.exists()


def load_model(path):
    if SOURCE == "doom":
        return doom_model(doom_name(path, ".md6mesh"))
    c = cast.Cast.load(str(path))
    for root in c.Roots():
        for m in root.ChildrenOfType(cast.Model):
            return m
    raise RuntimeError(f"no model in {path}")


def skeleton(model):
    bones = model.Skeleton().Bones()
    names = [b.Name() for b in bones]
    parents = [b.ParentIndex() for b in bones]
    local = [(np.array(b.LocalPosition(), float), np.array(b.LocalRotation(), float)) for b in bones]
    return names, parents, local


def globals_from_local(parents, local_mats):
    g = [None] * len(parents)
    for i, p in enumerate(parents):
        g[i] = local_mats[i] if p < 0 else g[p] @ local_mats[i]
    return g


class Clip:
    def __init__(self, path):
        self.curves = {}
        self.frames = 1
        self.fps = 30.0
        if SOURCE == "doom":
            a = doom_source.anim(doom_name(path, ".md6anim")) if path is not None else None
            if a is not None:
                # (a copy: the bake edits curves in place - "amp", "drop_bones" - and the parsed
                # clip is cached; shared, the zoomed shot got the slide boost twice)
                self.curves = {bone: dict(props) for bone, props in a.curves.items()}
                self.frames, self.fps = a.frames, a.fps
            return
        if path is None or not path.exists():
            return
        c = cast.Cast.load(str(path))
        for root in c.Roots():
            for a in root.ChildrenOfType(cast.Animation):
                self.fps = a.Framerate() or 30.0
                for cv in a.Curves():
                    kb = list(cv.KeyFrameBuffer())
                    kv = list(cv.KeyValueBuffer())
                    if cv.KeyPropertyName() == "rq":  # flat x,y,z,w stream
                        kv = [tuple(kv[i:i + 4]) for i in range(0, len(kv), 4)]
                    self.curves.setdefault(cv.NodeName(), {})[cv.KeyPropertyName()] = (kb, kv, cv.Mode())
                    if kb:
                        self.frames = max(self.frames, int(max(kb)) + 1)

    def sample(self, bone, prop, frame):
        c = self.curves.get(bone, {}).get(prop)
        if not c:
            return None
        kb, kv, _ = c
        if len(kb) == 1 or frame <= kb[0]:
            return kv[0]
        if frame >= kb[-1]:
            return kv[-1]
        for i in range(len(kb) - 1):
            if kb[i] <= frame <= kb[i + 1]:
                t = (frame - kb[i]) / max(kb[i + 1] - kb[i], 1e-6)
                a, b = kv[i], kv[i + 1]
                if prop == "rq":
                    return tuple(slerp(a, b, t))
                return a + (b - a) * t
        return kv[-1]

    def local(self, names, bind_local, frame, base=None, weight=1.0, bone_w=None):
        """Local bone transforms at `frame`. Absolute curves replace the pose; additive curves
        (Doom's meathook layers) are added on top of `base` (another Clip, sampled at the same
        frame) or of the bind pose."""
        out = []
        for i, n in enumerate(names):
            t, q = bind_local[i]
            t = t.copy()
            q = np.array(q, float)
            if base is not None:
                bf = min(frame, base.frames - 1)
                rq = base.sample(n, "rq", bf)
                if rq is not None:
                    q = np.array(rq, float)
                for k, axis in (("tx", 0), ("ty", 1), ("tz", 2)):
                    v = base.sample(n, k, bf)
                    if v is not None:
                        t[axis] = v
            props = self.curves.get(n, {})
            # Additive layers can be faded (weight) and limited to some bones (bone_w: substring ->
            # scale; bones not listed are left out).
            bw = weight
            if bone_w is not None:
                bw *= next((v for k, v in bone_w.items() if k in n), 0.0)
            rq = self.sample(n, "rq", frame)
            if rq is not None:
                if props["rq"][2] == "additive":
                    if bw > 0:
                        q = quat_mul(q, slerp([0.0, 0.0, 0.0, 1.0], rq, bw))
                else:
                    q = np.array(rq, float)
            for k, axis in (("tx", 0), ("ty", 1), ("tz", 2)):
                v = self.sample(n, k, frame)
                if v is not None:
                    t[axis] = t[axis] + v * bw if props[k][2] == "additive" else v
            out.append(trs(t, q))
        return out


def quat_mul(a, b):
    """Hamilton product of (x, y, z, w) quaternions: rotation b applied in a's frame."""
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return np.array([aw * bx + ax * bw + ay * bz - az * by,
                     aw * by - ax * bz + ay * bw + az * bx,
                     aw * bz + ax * by - ay * bx + az * bw,
                     aw * bw - ax * bx - ay * by - az * bz])


def mesh_arrays(m):
    pos = np.array(m.VertexPositionBuffer(), np.float32).reshape(-1, 3)
    nrm = np.array(m.VertexNormalBuffer(), np.float32).reshape(-1, 3)
    uv = np.array(m.VertexUVLayerBuffer(0), np.float32).reshape(-1, 2)
    inf = m.MaximumWeightInfluence()
    n = len(pos)
    bi = np.zeros((n, 4), np.uint16)
    bw = np.zeros((n, 4), np.float32)
    if inf > 0:
        ib = np.array(m.VertexWeightBoneBuffer(), np.int64).reshape(n, inf)
        wb = np.array(m.VertexWeightValueBuffer(), np.float32).reshape(n, inf)
        k = min(inf, 4)
        bi[:, :k] = ib[:, :k]
        bw[:, :k] = wb[:, :k]
        s = bw.sum(1, keepdims=True)
        s[s == 0] = 1
        bw /= s
    else:
        bw[:, 0] = 1
    faces = np.array(m.FaceBuffer(), np.uint32).reshape(-1, 3)
    return pos, nrm, uv, bi, bw, faces


def material_name(m):
    mat = m.Material()
    return mat.Name().split("/")[-1] if mat else "none"


MD6DEF = Path.home() / "doom-extract/wdecl/gameresources/generated/decls/md6def/md6def/objects/weapons"
DECL_NEWER = Path.home() / "doom-extract/wdecl2"
if SOURCE == "doom":
    MD6DEF = doom_source.WORK / "decls" / "gameresources"
    DECL_NEWER = doom_source.WORK / "decls"


def all_tags(stem, g_names, g_bind_g):
    import re
    newer = DECL_NEWER
    f = next(iter(sorted(newer.rglob(f"{stem}.md6.decl"), reverse=True)), None) or next(MD6DEF.rglob(f"{stem}.md6.decl"), None)
    if not f:
        return {}
    txt = f.read_text(errors="ignore")
    out = {}
    for name, x, y, z, parent in re.findall(r'tag "([^"]+)" \{\s*trans \( ([-\d.e]+) ([-\d.e]+) ([-\d.e]+) \)\s*rot \([^)]*\)\s*parent "([^"]+)"', txt):
        if parent in g_names:
            bi = g_names.index(parent)
            pos = (g_bind_g[bi] @ np.array([float(x) * 100, float(y) * 100, float(z) * 100, 1.0]))[:3]
            out[name] = {"bone": bi, "pos": pos.tolist()}
    return out


def muzzle_tag(stem):
    """(parent joint, offset m, tag name) of the weapon's muzzle tag: fx_muzzle if present, else
    the last plain "muzzle" (the first one is a shared default)."""
    import re
    f = next(MD6DEF.rglob(f"{stem}.md6.decl"), None)
    if not f:
        return None
    txt = f.read_text(errors="ignore")
    tags = re.findall(r'tag "([^"]+)" \{\s*trans \( ([-\d.e]+) ([-\d.e]+) ([-\d.e]+) \)\s*rot \([^)]*\)\s*parent "([^"]+)"', txt)
    pick = [t for t in tags if t[0] == "fx_muzzle"] or [t for t in tags if t[0] == "muzzle"]
    if not pick:
        return None
    t = pick[-1]
    return (t[4], (float(t[1]), float(t[2]), float(t[3])), t[0])


def bake(name):
    folder, gun_file, drop, clip_map = WEAPONS[name]
    arms_dir = gun_model_dir = gun_anim_dir = folder
    arms = load_model(ARMS)
    a_names, a_par, a_bind = skeleton(arms)
    a_bind_g = globals_from_local(a_par, [trs(t, q) for t, q in a_bind])
    a_inv_bind = [np.linalg.inv(g) for g in a_bind_g]
    cam_i = a_names.index("camera")
    attach_i = a_names.index("righthandattach")

    if gun_file:
        gun_path = GUN_MODELS / gun_model_dir / "assets/mesh" / gun_file
        gun = load_model(gun_path)
        g_names, g_par, g_bind = skeleton(gun)
    else:
        gun_path = ARMS  # textures dir fallback
        gun = None
        g_names, g_par, g_bind = ["origin"], [-1], [(np.zeros(3), np.array([0.0, 0.0, 0.0, 1.0]))]
    g_bind_g = globals_from_local(g_par, [trs(t, q) for t, q in g_bind])
    g_inv_bind = [np.linalg.inv(g) for g in g_bind_g]

    out = OUT_ROOT / name
    out.mkdir(parents=True, exist_ok=True)

    # ---- meshes: arms (left/right material picked by side) then gun
    meshes = []
    # Arms only: the left/right Praetor sleeves + gloves (torso, legs and the wrist blade stay hidden).
    for m in arms.Meshes():
        mat = material_name(m)
        if mat not in ("doomslayer_1p_left", "doomslayer_1p_right"):
            continue
        p, n, uv, bi, bw, f = mesh_arrays(m)
        meshes.append(("arms", mat, p, n, uv, bi, bw, f))
    part_names = []
    if gun_file and name in MESH_MODES:
        stem = Path(gun_file).stem
        res = str(gun_path.relative_to(GUN_MODELS.parent.parent.parent)).replace("\\", "/").replace(".cast", ".md6mesh")
        part_names = md6_part_names(res)
        print(f"  {len(part_names)} md6 parts: {sorted({p.split('$')[0] for p in part_names})}")
    mesh_modes = []
    for mi, m in enumerate(gun.Meshes() if gun else []):
        mat = material_name(m)
        part = part_names[mi].split("$")[0] if mi < len(part_names) else ""
        if part == "auger":
            continue
        if any(d in mat for d in drop if "#" not in d):
            continue
        p, n, uv, bi, bw, f = mesh_arrays(m)
        # "material#verts": one exact mesh (alternative mod parts that share a material).
        if any(d == f"{mat}#{len(p)}" for d in drop):
            continue
        meshes.append(("gun", mat, p, n, uv, bi, bw, f))
        modes = MESH_MODES.get(name, {})
        mesh_modes.append(next((v for k, v in modes.items() if part == k or part.startswith(k + "_")), 0))

    # ---- clips
    clips = []
    gun_core = GUN_ANIM / gun_anim_dir / "motion/player/human/base/core"
    for clip, spec in clip_map.items():
        arms_clip, gun_clip = spec[0], spec[1]
        opt = spec[2] if len(spec) > 2 else {}
        ap = VEGA / "animations/md6" / f"{opt['arms_path']}.cast" if "arms_path" in opt else ARMS_ANIM / arms_dir / f"{arms_clip}.cast"
        ac = Clip(ap)
        if not ac.curves:
            print(f"  (no arms clip {arms_clip})")
            continue
        a_base = Clip(ARMS_ANIM / arms_dir / f"{opt['arms_base']}.cast") if "arms_base" in opt else None
        g_base = Clip(gun_core / f"{opt['gun_base']}.cast") if "gun_base" in opt else None
        gp = gun_core / f"{gun_clip}.cast" if gun_clip else None
        if gp is None or not clip_exists(gp):
            gp = next((p for p in [gun_core / "idle_pose.cast", gun_core / "idle.cast"] if clip_exists(p)), None) if gun else None
        gc = Clip(gp)
        for sub in opt.get("drop_bones", []):
            for bone in [b for b in gc.curves if sub in b]:
                del gc.curves[bone]
        # Exaggerate some gun parts' slide: translation offsets from the clip's first frame x k.
        for sub, k in opt.get("amp", {}).items():
            for bone, props in gc.curves.items():
                if sub not in bone:
                    continue
                for prop in ("tx", "ty", "tz"):
                    if prop in props:
                        kb, kv, mode = props[prop]
                        v0 = kv[0]
                        props[prop] = (kb, [v0 + (v - v0) * k for v in kv], mode)
        frames = max(ac.frames, 1)
        if opt.get("frames") == "max":
            frames = max(frames, gc.frames)
        if opt.get("len") == "gun":
            frames = gc.frames
        start = opt.get("start", 0)
        if start:
            frames = max(frames - start, 1)
        gun_start = opt.get("gun_start", 0)
        if gun_start:
            frames = max(gc.frames - gun_start, 1)
        src_frames = frames
        if opt.get("attack"):
            frames = frames * 2 - 1
        pal_a = np.zeros((frames, len(a_names), 3, 4), np.float32)
        pal_g = np.zeros((frames, len(g_names), 3, 4), np.float32)
        for fr in range(frames):
            env = (fr / max(frames - 1, 1)) if opt.get("envelope") else 1.0
            if opt.get("attack"):
                a = opt["attack"] * (frames - 1)
                env = fr / a if fr < a else 1.0 - (fr - a) / max(frames - 1 - a, 1)
                env = min(max(env, 0.0), 1.0)
            env = env * env * (3.0 - 2.0 * env)
            src = min(fr, src_frames - 1) + start
            ag = globals_from_local(a_par, ac.local(a_names, a_bind, min(src, ac.frames - 1), a_base, env * opt.get("arms_w", 1.0)))
            view = V @ np.linalg.inv(ag[cam_i])
            for i in range(len(a_names)):
                pal_a[fr, i] = (view @ ag[i] @ a_inv_bind[i])[:3]
            # Looping shoot states: the gun's own (shorter) cycle repeats under the arms loop.
            if opt.get("gun_frame") == "last":
                gf = max(gc.frames - 1, 0)
            else:
                gf = fr % max(gc.frames, 1) if opt.get("gun_loop") else min(src + gun_start, max(gc.frames - 1, 0))
            gl = gc.local(g_names, g_bind, gf, g_base, env, opt.get("gun_bones"))
            gg = globals_from_local(g_par, gl)
            root = ag[attach_i]
            for i in range(len(g_names)):
                pal_g[fr, i] = (view @ root @ gg[i] @ g_inv_bind[i])[:3]
        if opt.get("trim"):
            # Cut a still tail (the clip holding its last pose) so the next clip follows at once.
            n = len(pal_a)
            while n > 2 and max(np.abs(pal_a[n - 1] - pal_a[n - 2])[:, :, 3].max(), np.abs(pal_g[n - 1] - pal_g[n - 2])[:, :, 3].max()) < 0.01:
                n -= 1
            pal_a, pal_g, frames = pal_a[:n], pal_g[:n], n
        if opt.get("pingpong"):
            pal_a = np.concatenate([pal_a, pal_a[-2::-1]])
            pal_g = np.concatenate([pal_g, pal_g[-2::-1]])
            frames = len(pal_a)
        clips.append((clip, ac.fps, pal_a, pal_g))
        print(f"  clip {clip}: {frames} frames")

    # ---- write model.bin
    with open(out / "model.bin", "wb") as fh:
        fh.write(b"DVM1")
        fh.write(struct.pack("<III", len(a_names), len(g_names), len(meshes)))
        for part, mat, p, n, uv, bi, bw, f in meshes:
            mb = mat.encode()
            fh.write(struct.pack("<BH", 0 if part == "arms" else 1, len(mb)) + mb)
            fh.write(struct.pack("<II", len(p), len(f)))
            verts = np.zeros(len(p), dtype=[("p", "<f4", 3), ("n", "<f4", 3), ("uv", "<f4", 2),
                                            ("bi", "<u2", 4), ("bw", "<f4", 4)])
            verts["p"], verts["n"], verts["uv"], verts["bi"], verts["bw"] = p, n, uv, bi, bw
            fh.write(verts.tobytes())
            fh.write(f.astype("<u4").tobytes())
        fh.write(struct.pack("<I", len(clips)))
        for clip, fps, pa, pg in clips:
            cb = clip.encode()
            fh.write(struct.pack("<H", len(cb)) + cb)
            fh.write(struct.pack("<fI", fps, pa.shape[0]))
            fh.write(pa.astype("<f4").tobytes())
            fh.write(pg.astype("<f4").tobytes())

    # ---- textures: base colour / normal / specular / emissive per material
    import shutil
    mats = sorted({m[1] for m in meshes})
    img_dirs = [gun_path.parent / "_images", ARMS.parent / "_images"]
    if SOURCE == "doom":
        # SAMUEL's image exports, made to match Vega's (md6tex: colour scale / bias, greyscale)
        img_dirs = [load_model(gun_path).dir if gun_file else load_model(ARMS).dir, load_model(ARMS).dir]
    copied = []
    for mat in mats:
        for suffix in ("", "_n", "_s", "_e", "_g", "_pm"):
            for d in img_dirs:
                src = d / f"{mat}{suffix}.png"
                if SOURCE == "doom":
                    src = d / "images" / f"{mat}{suffix}.png"
                if src.exists():
                    if SOURCE == "doom":
                        art = md6tex.decl_images(d).get(src.stem)
                        hdr = md6tex.image_header(doom_source.RES, art) if art else None
                        md6tex.convert(src, hdr).save(out / src.name)
                    else:
                        shutil.copy(src, out / src.name)
                    copied.append(src.name)
                    break
    info = {
        "materials": mats, "textures": copied, "clips": [c[0] for c in clips],
        "arms_bones": len(a_names), "gun_bones": len(g_names),
    }
    # Doom's own muzzle attachment (md6def tag: parent joint + offset in metres) in gun bind space
    # (cm), so the renderer can skin it like a vertex: exact bore position + barrel axis.
    if gun_file:
        tag = muzzle_tag(Path(gun_file).stem)
        if tag and tag[0] in g_names:
            bi = g_names.index(tag[0])
            g = g_bind_g[bi]
            off = np.array(tag[1], float) * 100.0
            pos = (g @ np.append(off, 1.0))[:3]
            # Barrel axis = the gun's forward (+X in bind space); the tag's parent joint may be rotated.
            fwd = pos + np.array([10.0, 0.0, 0.0])
            info["muzzle"] = {"bone": bi, "pos": pos.tolist(), "dir": fwd.tolist(), "tag": tag[2]}
            gx = max(float(np.max(m[2][:, 0])) for m in meshes if m[0] == "gun")
            print(f"  muzzle tag {tag[2]} on {tag[0]}: bind {pos.round(1)} (gun max x {gx:.1f})")
        else:
            print(f"  !! no muzzle tag for {gun_file}: {tag}")
    # Spinning barrels (Doom turns them in code, not in the clips): each group is a bone subtree
    # turned about the gun's forward axis (+X in bind space) through its pivot bone, in cm.
    spins = []
    for group, roots, pivot in SPIN.get(name, []):
        if pivot not in g_names:
            continue
        sub = {g_names.index(r) for r in roots if r in g_names}
        grew = True
        while grew:
            grew = False
            for i, par in enumerate(g_par):
                if par in sub and i not in sub:
                    sub.add(i)
                    grew = True
        spins.append({"group": group, "bones": sorted(sub), "pivot": g_bind_g[g_names.index(pivot)][:3, 3].tolist(),
                      "axis": [1.0, 0.0, 0.0]})
    if spins:
        info["spin"] = spins
    if any(mesh_modes):
        # Per mesh in model.bin order (arms first): 0 always, 1 normal mode, 2 turret mode.
        info["mesh_modes"] = [0] * sum(1 for m in meshes if m[0] == "arms") + mesh_modes
    # Every md6def tag (bind space, cm) - the renderer anchors effects to them (rocket in chamber).
    if gun_file:
        info["tags"] = all_tags(Path(gun_file).stem, g_names, g_bind_g)
    (out / "info.json").write_text(json.dumps(info, indent=1))
    print(f"{name}: {len(meshes)} meshes, {len(clips)} clips, {len(copied)} textures -> {out}")


# The Crucible's energy blade has no textures in the Cast export: Doom draws it from mask
# textures (art/weapons/crucible/crucible_blade_*). They come from gameresources (base game) via
# samuel-cli and are saved under the blade materials' names: <material>.png = the overall mask
# (vm.hlsl GlowMain reads its red), <material>_e.png = the red flow texture that breaks it up.
BLADE_TEXTURES = {
    "crucible_blade.png": "art/weapons/crucible/crucible_blade_overall_mask.tga$bc7srgb$streamed",
    "crucible_blade_e.png": "art/weapons/crucible/crucible_blade_mask2.tga$bc7srgb$streamed",
    "crucible_blade_card.png": "art/weapons/crucible/crucible_blade_card_overall_mask.tga$bc7srgb$streamed",
    "crucible_blade_card_e.png": "art/weapons/crucible/crucible_blade_card_flow.tga$bc7srgb$streamed",
}


def crucible_blade_textures():
    import shutil
    import subprocess
    import doom_res
    samuel = doom_res.SAMUEL
    base = doom_res.DOOM / "base"
    tmp = Path(os.environ.get("DOOMRING_WORK", Path.home() / "doom-extract")) / "crucible_tex"
    for out_name, res_name in BLADE_TEXTURES.items():
        for r in ("gameresources", "gameresources_patch1", "gameresources_patch2", "gameresources_patch3"):
            subprocess.run([str(samuel), str(base / f"{r}.resources"), "export", str(tmp), res_name],
                           capture_output=True, stdin=subprocess.DEVNULL)
        src = next(tmp.rglob(Path(res_name).name + ".png"), None)
        if src is None:
            print("!! blade texture missing:", res_name)
            continue
        shutil.copy(src, OUT_ROOT / "crucible" / out_name)
        print("  blade texture", out_name)


def export_decls():
    """Doom mode: the weapons' md6def decls (muzzle + attachment tags)."""
    # (guns only: the chainsaw / Crucible were baked without their decls - no muzzle there)
    names = sorted({f"generated/decls/md6def/md6def/objects/weapons/{f}.md6.decl"
                    for f, gf, _, _ in WEAPONS.values() if gf and not f.startswith("melee/")})
    doom_source.export_decls(names)


def prepare():
    """Doom mode: export every model and decl once (the bakes can then run in parallel)."""
    export_decls()
    load_model(ARMS)
    for folder, gun_file, _, _ in WEAPONS.values():
        if gun_file:
            load_model(GUN_MODELS / folder / "assets/mesh" / gun_file)
    print("models exported")


if __name__ == "__main__":
    if "--prepare" in sys.argv:
        prepare()
        sys.exit(0)
    if SOURCE == "doom" and not any((doom_source.WORK / "decls").rglob("*.md6.decl")):
        export_decls()
    for n in sys.argv[1:] or list(WEAPONS):
        bake(n)
        if n == "crucible":
            crucible_blade_textures()
