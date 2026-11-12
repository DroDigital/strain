"""Realistic humans: player, soldiers and civilians on Quaternius' Universal Base Characters, animated
with the Universal Animation Library (both CC0), plus clip libraries for the infected.

Run with a Python that can `import bpy` (Blender's Python, or `pip install bpy`):

    python -I tools/blender/build_humans.py

It downloads both packs into target/asset-cache (or uses STRAIN_UBC_DIR / STRAIN_UAL_DIR) and
writes, under assets/:

* models/Human_*.glb      every human variant: the realistic base body with clothing modelled
                          onto it in Blender terms (garments are shells grown from body regions,
                          with circumferential folds; the skin they cover is removed so it never
                          pokes through), rigged hair and eyebrows, its own skeleton;
* models/Clips_Male.glb,  skeleton-only clip libraries: the animation library's clips and the kit's
  Clips_Female.glb        zombie clips, retargeted onto each body's skeleton (bone directions
                          aligned in world space, hips scaled by leg length);
* characters/*.png        per-character textures (skin, eyes, hair) at 1024 px, with a packed
                          material map (`*_n.png`: normal, roughness, metalness).

Materials named `C:<name>` select a per-character texture; `T:<Tex>` a tiling photo layer.
"""
import glob
import io
import json
import math
import os
import re
import sys
import urllib.request
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import bpy  # noqa: E402,F401  (provides mathutils)
import numpy as np  # noqa: E402
from mathutils import Matrix, Quaternion, Vector, noise  # noqa: E402

from glb import Glb  # noqa: E402
from realism import box_uv, layer_means, srgb_to_lin  # noqa: E402

ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
MODELS = os.path.join(ROOT, "assets", "models")
CHARS = os.path.join(ROOT, "assets", "characters")
CACHE = os.environ.get("STRAIN_ASSET_CACHE", os.path.join(ROOT, "target", "asset-cache"))
CHAR_TEX = 1024
FPS = 30.0


# ------------------------------------------------------------------------------------------------
# Sources

def itch_download(game, dest):
    """Fetch a free (pay-what-you-want, $0) itch.io download of a Quaternius pack."""
    import http.cookiejar

    cj = http.cookiejar.CookieJar()
    op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cj))
    op.addheaders = [("User-Agent", "strain-asset-fetch/1.0")]
    page = op.open(f"https://quaternius.itch.io/{game}", timeout=60).read().decode("utf8", "replace")
    tok = re.search(r'name="csrf_token" value="([^"]+)"', page).group(1)
    data = urllib.parse.urlencode({"csrf_token": tok}).encode()
    url = json.loads(op.open(f"https://quaternius.itch.io/{game}/download_url", data, timeout=60).read())["url"]
    page = op.open(url, timeout=60).read().decode("utf8", "replace")
    upload = re.search(r'data-upload_id="(\d+)"', page).group(1)
    furl = json.loads(op.open(f"https://quaternius.itch.io/{game}/file/{upload}?source=game_download", data, timeout=60).read())["url"]
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    with op.open(furl, timeout=600) as r, open(dest + ".part", "wb") as f:
        f.write(r.read())
    os.replace(dest + ".part", dest)


def source_dir(env, game):
    if os.environ.get(env):
        return os.environ[env]
    out = os.path.join(CACHE, game)
    if not os.path.isdir(out):
        z = os.path.join(CACHE, game + ".zip")
        if not os.path.exists(z):
            print("downloading", game)
            itch_download(game, z)
        zipfile.ZipFile(z).extractall(out)
    return out


class Gltf(Glb):
    """A .gltf with an external .bin, behaving like a Glb (and saved as one)."""

    def __init__(self, path):
        self.j = json.load(open(path))
        uri = self.j["buffers"][0]["uri"]
        self.bin = bytearray(open(os.path.join(os.path.dirname(path), uri), "rb").read())
        self.j["buffers"][0].pop("uri", None)


def find(base, pattern):
    hits = sorted(glob.glob(os.path.join(glob.escape(base), "**", pattern), recursive=True))
    if not hits:
        raise FileNotFoundError(pattern)
    return hits[0]


# ------------------------------------------------------------------------------------------------
# Skeletons and clips

class Skel:
    def __init__(self, g):
        self.g = g
        j = g.j
        self.nodes = j["nodes"]
        self.joints = g.skin()["joints"]
        self.names = [self.nodes[n].get("name", "") for n in self.joints]
        self.jidx = {n: i for i, n in enumerate(self.joints)}
        parent = {}
        for i, nd in enumerate(self.nodes):
            for c in nd.get("children", []):
                parent[c] = i
        self.node_parent = parent
        self.parent = [self.jidx.get(parent.get(n)) for n in self.joints]
        # rest local transforms
        self.rest_t, self.rest_r, self.rest_s = [], [], []
        for n in self.joints:
            nd = self.nodes[n]
            self.rest_t.append(Vector(nd.get("translation", [0, 0, 0])))
            x, y, z, w = nd.get("rotation", [0, 0, 0, 1])
            self.rest_r.append(Quaternion((w, x, y, z)))
            self.rest_s.append(Vector(nd.get("scale", [1, 1, 1])))
        # the transform above the skeleton (armature node chain)
        root = next(i for i, p in enumerate(self.parent) if p is None)
        m = Matrix.Identity(4)
        n = parent.get(self.joints[root])
        chain = []
        while n is not None:
            chain.append(n)
            n = parent.get(n)
        for n in reversed(chain):
            m = m @ self.node_local(n)
        self.pre = m
        self.order = sorted(range(len(self.joints)), key=self.depth)
        self.rest_g = self.globals(self.rest_t, self.rest_r, self.rest_s)

    def node_local(self, n):
        nd = self.nodes[n]
        if "matrix" in nd:
            return Matrix([nd["matrix"][i::4] for i in range(4)])
        t = Vector(nd.get("translation", [0, 0, 0]))
        x, y, z, w = nd.get("rotation", [0, 0, 0, 1])
        s = Vector(nd.get("scale", [1, 1, 1]))
        return Matrix.LocRotScale(t, Quaternion((w, x, y, z)), s)

    def depth(self, i):
        d = 0
        while self.parent[i] is not None:
            i = self.parent[i]
            d += 1
        return d

    def globals(self, ts, rs, ss):
        out = [None] * len(self.joints)
        for i in self.order:
            loc = Matrix.LocRotScale(ts[i], rs[i], ss[i])
            p = self.parent[i]
            out[i] = (out[p] if p is not None else self.pre) @ loc
        return out

    def find(self, name):
        return self.names.index(name) if name in self.names else None

    def pos(self, g, name):
        return g[self.find(name)].to_translation()


_ACC = {}


def acc(g, i):
    key = (id(g), i)
    if key not in _ACC:
        _ACC[key] = g.acc(i)
    return _ACC[key]


def sample(g, anim, t):
    """Local TRS overrides {joint node: (t, r, s)} of a glTF animation at time t."""
    out = {}
    for ch in anim["channels"]:
        smp = anim["samplers"][ch["sampler"]]
        times = acc(g, smp["input"]).ravel()
        vals = acc(g, smp["output"])
        cubic = smp.get("interpolation") == "CUBICSPLINE"
        if cubic:
            vals = vals.reshape(len(times), 3, -1)[:, 1]
        k = int(np.searchsorted(times, t, side="right")) - 1
        k = max(0, min(k, len(times) - 1))
        k2 = min(k + 1, len(times) - 1)
        f = 0.0 if k2 == k or smp.get("interpolation") == "STEP" else (t - times[k]) / max(times[k2] - times[k], 1e-9)
        f = min(max(f, 0.0), 1.0)
        node = ch["target"]["node"]
        path = ch["target"]["path"]
        cur = out.setdefault(node, {})
        if path == "rotation":
            a, b = vals[k], vals[k2]
            qa, qb = Quaternion((a[3], a[0], a[1], a[2])), Quaternion((b[3], b[0], b[1], b[2]))
            cur["r"] = qa.slerp(qb, f)
        elif path in ("translation", "scale"):
            cur[path[0]] = Vector(vals[k] * (1 - f) + vals[k2] * f)
    return out


def anim_duration(g, anim):
    return max(float(acc(g, anim["samplers"][c["sampler"]]["input"]).max()) for c in anim["channels"])


def bone_dir(sk, g, name, child):
    a = sk.pos(g, name)
    if child is None or sk.find(child) is None:
        return None
    d = sk.pos(g, child) - a
    return d.normalized() if d.length > 1e-6 else None


UE_CHILD = {"pelvis": "spine_01", "spine_01": "spine_02", "spine_02": "spine_03", "spine_03": "neck_01", "neck_01": "Head",
            "clavicle_l": "upperarm_l", "upperarm_l": "lowerarm_l", "lowerarm_l": "hand_l", "hand_l": "middle_01_l",
            "clavicle_r": "upperarm_r", "upperarm_r": "lowerarm_r", "lowerarm_r": "hand_r", "hand_r": "middle_01_r",
            "thigh_l": "calf_l", "calf_l": "foot_l", "foot_l": "ball_l", "thigh_r": "calf_r", "calf_r": "foot_r", "foot_r": "ball_r"}
KIT_CHILD = {"Hips": "Abdomen", "Abdomen": "Torso", "Torso": "Neck", "Neck": "Head", "Shoulder.L": "UpperArm.L", "UpperArm.L": "LowerArm.L",
             "LowerArm.L": "Middle1.L", "Shoulder.R": "UpperArm.R", "UpperArm.R": "LowerArm.R", "LowerArm.R": "Middle1.R",
             "UpperLeg.L": "LowerLeg.L", "LowerLeg.L": "Foot.L", "UpperLeg.R": "LowerLeg.R", "LowerLeg.R": "Foot.R"}
# realistic bone <- kit bone (both rigs face +Z with anatomical left at +X)
FROM_KIT = {"pelvis": "Hips", "spine_01": "Abdomen", "spine_03": "Torso", "neck_01": "Neck", "Head": "Head"}
for s, k in (("l", "L"), ("r", "R")):
    FROM_KIT.update({f"clavicle_{s}": f"Shoulder.{k}", f"upperarm_{s}": f"UpperArm.{k}", f"lowerarm_{s}": f"LowerArm.{k}",
                     f"thigh_{s}": f"UpperLeg.{k}", f"calf_{s}": f"LowerLeg.{k}", f"foot_{s}": f"Foot.{k}"})


def retarget(src_g, src_sk, anim, dst_sk, bone_map, src_child, dst_child, name):
    """World-space retarget of one clip; returns (times, {joint: [quat]}, pelvis translations)."""
    dur = anim_duration(src_g, anim)
    n = max(2, int(round(dur * FPS)) + 1)
    times = np.linspace(0.0, dur, n)
    # rest alignment: rotate each target bone's rest direction onto its source's
    align = {}
    for d, s in bone_map.items():
        di, si = dst_sk.find(d), src_sk.find(s)
        if di is None or si is None:
            continue
        a = bone_dir(dst_sk, dst_sk.rest_g, d, dst_child.get(d))
        b = bone_dir(src_sk, src_sk.rest_g, s, src_child.get(s))
        align[d] = a.rotation_difference(b) if a is not None and b is not None else Quaternion()
    src_pel = bone_map.get("pelvis")
    s_h = src_sk.pos(src_sk.rest_g, src_pel).y if src_pel else 1.0
    d_h = dst_sk.pos(dst_sk.rest_g, "pelvis").y
    k_h = d_h / max(abs(s_h), 1e-4)
    rots = {i: [] for i in range(len(dst_sk.joints))}
    pel_t = []
    for t in times:
        ov = sample(src_g, anim, float(t))
        ts, rs, ss = list(src_sk.rest_t), list(src_sk.rest_r), list(src_sk.rest_s)
        for node, trs in ov.items():
            i = src_sk.jidx.get(node)
            if i is None:
                continue
            if "t" in trs:
                ts[i] = trs["t"]
            if "r" in trs:
                rs[i] = trs["r"]
            if "s" in trs:
                ss[i] = trs["s"]
        sg = src_sk.globals(ts, rs, ss)
        # target world rotations
        world = {}
        for d, s in bone_map.items():
            di, si = dst_sk.find(d), src_sk.find(s)
            if di is None or si is None:
                continue
            ds = sg[si].to_quaternion() @ src_sk.rest_g[si].to_quaternion().inverted()
            world[di] = ds @ align[d] @ dst_sk.rest_g[di].to_quaternion()
        glob_r = [None] * len(dst_sk.joints)
        local = [None] * len(dst_sk.joints)
        for i in dst_sk.order:
            p = dst_sk.parent[i]
            pr = glob_r[p] if p is not None else dst_sk.pre.to_quaternion()
            if i in world:
                local[i] = (pr.inverted() @ world[i]).normalized()
            else:
                local[i] = dst_sk.rest_r[i]
            glob_r[i] = pr @ local[i]
            rots[i].append(local[i])
        # hips: source displacement from rest, scaled by leg length
        if src_pel:
            si = src_sk.find(src_pel)
            delta = (sg[si].to_translation() - src_sk.rest_g[si].to_translation()) * k_h
            pi = dst_sk.find("pelvis")
            wp = dst_sk.rest_g[pi].to_translation() + delta
            pp = dst_sk.parent[pi]
            pm = dst_sk.globals(dst_sk.rest_t, [rots[j][-1] if rots[j] else dst_sk.rest_r[j] for j in range(len(dst_sk.joints))], dst_sk.rest_s)[pp] if pp is not None else dst_sk.pre
            pel_t.append(pm.inverted() @ wp)
    return name, times, rots, pel_t


def write_library(dst_path_gltf, out, clips):
    """A skeleton-only GLB carrying the retargeted clips."""
    g = Gltf(dst_path_gltf)
    for nd in g.j["nodes"]:
        nd.pop("mesh", None)
    g.j["meshes"] = []
    for k in ("materials", "textures", "images", "samplers"):
        g.j.pop(k, None)
    sk = Skel(g)
    anims = []
    for name, times, rots, pel_t in clips:
        tin = g.add(times.astype(np.float32).reshape(-1, 1), "time", 5126, minmax=True)
        samplers, channels = [], []
        for i, qs in rots.items():
            # skip joints that never leave their rest pose
            arr = np.array([[q.x, q.y, q.z, q.w] for q in qs], dtype=np.float32)
            rq = sk.rest_r[i]
            if np.abs(arr - np.array([rq.x, rq.y, rq.z, rq.w])).max() < 1e-4:
                continue
            samplers.append({"input": tin, "output": g.add(arr, "rot", 5126), "interpolation": "LINEAR"})
            channels.append({"sampler": len(samplers) - 1, "target": {"node": sk.joints[i], "path": "rotation"}})
        if pel_t:
            arr = np.array([[v.x, v.y, v.z] for v in pel_t], dtype=np.float32)
            samplers.append({"input": tin, "output": g.add(arr, "pos", 5126), "interpolation": "LINEAR"})
            channels.append({"sampler": len(samplers) - 1, "target": {"node": sk.joints[sk.find("pelvis")], "path": "translation"}})
        anims.append({"name": name, "samplers": samplers, "channels": channels})
    g.j["animations"] = anims
    g.save(out)
    print(f"{os.path.relpath(out, ROOT)}: {len(anims)} clips")


# ------------------------------------------------------------------------------------------------
# Textures

def save_char_texture(name, albedo_path, normal_path=None, rough_path=None, rough=0.6, alpha=False, sss=0.0):
    """Albedo plus a material map: R/G normal (OpenGL), B roughness, A subsurface scattering."""
    from PIL import Image

    os.makedirs(CHARS, exist_ok=True)
    im = Image.open(albedo_path).convert("RGBA").resize((CHAR_TEX, CHAR_TEX), Image.LANCZOS)
    if not alpha:
        im.putalpha(255)
    im.save(os.path.join(CHARS, f"{name}.png"), optimize=True)
    n = Image.open(normal_path).convert("RGB").resize((CHAR_TEX, CHAR_TEX), Image.LANCZOS) if normal_path else Image.new("RGB", (CHAR_TEX, CHAR_TEX), (128, 128, 255))
    r = Image.open(rough_path).convert("L").resize((CHAR_TEX, CHAR_TEX), Image.LANCZOS) if rough_path else Image.new("L", (CHAR_TEX, CHAR_TEX), int(rough * 255))
    nr, ng, _ = n.split()
    Image.merge("RGBA", (nr, ng, r, Image.new("L", (CHAR_TEX, CHAR_TEX), int(sss * 255)))).save(os.path.join(CHARS, f"{name}_n.png"), optimize=True)


# ------------------------------------------------------------------------------------------------
# Bodies and garments

class Body:
    """The base body of one character, in bind space, with per-vertex skin weights."""

    def __init__(self, g, sk):
        self.g, self.sk = g, sk
        mats = [m["name"] for m in g.j["materials"]]
        self.parts = {}
        for _, p in g.skinned_prims():
            a = p["attributes"]
            d = {
                "pos": g.acc(a["POSITION"]).astype(np.float64),
                "nrm": g.acc(a["NORMAL"]).astype(np.float64),
                "uv": g.acc(a["TEXCOORD_0"]).astype(np.float64),
                "j": g.acc(a["JOINTS_0"]).astype(np.int64),
                "w": g.acc(a["WEIGHTS_0"]).astype(np.float64),
                "tri": g.acc(p["indices"]).astype(np.int64).reshape(-1, 3),
            }
            m = mats[p["material"]]
            key = "eyes" if "Eye" in m else ("brows" if "Hair" in m else "skin")
            self.parts[key] = d
        b = self.parts["skin"]
        self.pos, self.nrm, self.j, self.w, self.tri = b["pos"], b["nrm"], b["j"], b["w"], b["tri"]

    def bw(self, patterns, part=None):
        """Per-vertex weight on bones whose names match any of the regex patterns."""
        d = self.parts[part] if part else self.parts["skin"]
        ids = [i for i, n in enumerate(self.sk.names) if any(re.fullmatch(p, n) for p in patterns)]
        return (d["w"] * np.isin(d["j"], ids)).sum(1)

    def jpos(self, name):
        return np.array(self.sk.pos(self.sk.rest_g, name))


def vertex_normals(pos, tri):
    fn = np.cross(pos[tri[:, 1]] - pos[tri[:, 0]], pos[tri[:, 2]] - pos[tri[:, 0]])
    vn = np.zeros_like(pos)
    for k in range(3):
        np.add.at(vn, tri[:, k], fn)
    return vn / np.maximum(np.linalg.norm(vn, axis=1, keepdims=True), 1e-12)


def boundary_verts(tri):
    e = np.sort(np.concatenate([tri[:, [0, 1]], tri[:, [1, 2]], tri[:, [2, 0]]]), axis=1)
    u, c = np.unique(e, axis=0, return_counts=True)
    return np.unique(u[c == 1].ravel())


class Garment:
    """A piece of clothing grown from a body region. `drape` is how many smoothing passes the cloth
    gets before it is held off the skin: more drape bridges more of the anatomy underneath."""

    def __init__(self, tex, color, offset, sel, folds=1.0, plates=0.0, drape=12, sole=None, free=None):
        self.tex, self.color, self.offset, self.sel, self.folds, self.plates, self.drape, self.sole = tex, color, offset, sel, folds, plates, drape, sole
        # vertices that may sink into the (hidden) skin while relaxing, e.g. toes inside a boot
        self.free = free


def smooth(pos, tri, iters, pin=None):
    """Taubin smoothing (no shrinkage) over the mesh's edge graph."""
    e = np.concatenate([tri[:, [0, 1]], tri[:, [1, 2]], tri[:, [2, 0]]])
    e = np.unique(np.sort(e, axis=1), axis=0)
    deg = np.zeros(len(pos))
    np.add.at(deg, e[:, 0], 1)
    np.add.at(deg, e[:, 1], 1)
    deg = np.maximum(deg, 1)[:, None]
    p = pos.copy()
    for _ in range(iters):
        for k in (0.5, -0.53):
            acc = np.zeros_like(p)
            np.add.at(acc, e[:, 0], p[e[:, 1]])
            np.add.at(acc, e[:, 1], p[e[:, 0]])
            d = acc / deg - p
            if pin is not None:
                d[pin] = 0
            p = p + k * d
    return p


def garment_mesh(body, gm, seed):
    vsel = gm.sel(body)
    faces = body.tri[vsel[body.tri].all(axis=1)]
    used = np.unique(faces)
    remap = -np.ones(len(body.pos), dtype=np.int64)
    remap[used] = np.arange(len(used))
    tri = remap[faces]
    pos = body.pos[used].copy()
    nrm = body.nrm[used]
    j, w = body.j[used], body.w[used]
    # folds wrap around the limb each vertex follows: bands along its bone's axis, broken by noise
    names = body.sk.names
    dom = j[np.arange(len(j)), w.argmax(1)]
    axis = np.zeros_like(pos)
    for jj in np.unique(dom):
        nm = names[jj]
        child = UE_CHILD.get(nm)
        a = body.jpos(nm)
        b = body.jpos(child) if child and body.sk.find(child) is not None else a + np.array([0, 1.0, 0])
        d = b - a
        axis[dom == jj] = d / max(np.linalg.norm(d), 1e-6)
    # weld UV-seam duplicates so the cloth moves as one surface
    key = np.round(pos * 1e5).astype(np.int64)
    _, canon, inv = np.unique(key, axis=0, return_index=True, return_inverse=True)
    inv = inv.ravel()
    wtri = inv[tri]
    wpos = pos[canon]
    wn = np.zeros_like(wpos)
    np.add.at(wn, inv, nrm)
    wn /= np.maximum(np.linalg.norm(wn, axis=1, keepdims=True), 1e-9)
    # drape as shrink-wrap: relax the cloth (it may shrink, like fabric pulled taut), then hold it
    # at least `offset` outside the skin; repeated, it bridges every valley and rests on the
    # convex parts
    e = np.concatenate([wtri[:, [0, 1]], wtri[:, [1, 2]], wtri[:, [2, 0]]])
    e = np.unique(np.sort(e, axis=1), axis=0)
    deg = np.zeros(len(wpos))
    np.add.at(deg, e[:, 0], 1)
    np.add.at(deg, e[:, 1], 1)
    deg = np.maximum(deg, 1)[:, None]
    bnd = np.zeros(len(wpos), dtype=bool)
    bnd[boundary_verts(wtri)] = True
    sm = wpos + wn * gm.offset
    for _ in range(gm.drape):
        acc = np.zeros_like(sm)
        np.add.at(acc, e[:, 0], sm[e[:, 1]])
        np.add.at(acc, e[:, 1], sm[e[:, 0]])
        d = acc / deg - sm
        d[bnd] = 0.0  # hems and cuffs hold their line
        sm = sm + 0.5 * d
        out = ((sm - wpos) * wn).sum(1)
        push = np.maximum(gm.offset - out, 0.0)
        if gm.free is not None:
            push[gm.free(wpos, j[canon], w[canon], body)] = 0.0
        sm = sm + wn * push[:, None]
    if gm.sole is not None:
        sm[:, 1] = np.maximum(sm[:, 1], gm.sole)
    pos = sm[inv]
    nrm = wn[inv]
    t = (pos * axis).sum(1)
    disp = np.zeros(len(pos))
    for i, p in enumerate(pos):
        v = Vector(p) * 9.0 + Vector((seed, seed * 0.7, seed * 1.3))
        n1 = noise.noise(v)
        fold = math.sin(t[i] * 55.0 + n1 * 4.0) * (0.5 + 0.5 * noise.noise(v * 0.5))
        disp[i] = gm.folds * (0.0025 * fold + 0.002 * noise.fractal(v * 2.0, 0.6, 2.0, 3))
    # displacement is a function of position, so welded duplicates stay together
    pos = pos + nrm * disp[:, None]
    if gm.plates > 0:
        # stiff panels (vest plates): flatten toward the mean normal in blocks
        pos = pos + nrm * gm.plates * (np.abs(np.sin(pos[:, 1] * 18.0)) > 0.25)[:, None]
    wn2 = vertex_normals(pos[canon], wtri)
    nrm2 = wn2[inv]
    return {"pos": pos, "nrm": nrm2, "j": j, "w": w, "tri": tri, "used": used, "tex": gm.tex, "color": gm.color}


def region(patterns, exclude=(), exclude_w=0.3, keep=None):
    def sel(body):
        m = body.bw(patterns) > 0.5
        for ex in exclude:
            m &= body.bw(ex) < exclude_w
        if keep is not None:
            m &= keep(body)
        return m
    return sel


ARMS = [r"clavicle_[lr]", r"upperarm_[lr]", r"lowerarm_[lr]"]
TORSO = [r"spine_0[123]"]
HANDS = [r"hand_[lr]", r"(index|middle|pinky|ring|thumb)_0\d.*"]
NECK = [r"neck_01", r"Head"]
FEET = [r"foot_[lr]", r"ball_[lr]", r"ball_leaf_[lr]"]


def waist_keep(drop):
    """A level hem at (waist - drop), never down the legs."""
    def k(body):
        y0 = body.jpos("pelvis")[1] - drop
        return (body.pos[:, 1] > y0) & (body.bw([r"thigh_[lr]"]) < 0.25)
    return k


def shirt(tex, color, offset=0.014, sleeves="long", drop=0.05):
    def keep(body):
        k = waist_keep(drop)(body)
        if sleeves == "short":
            for s in "lr":
                a, b = body.jpos(f"upperarm_{s}"), body.jpos(f"lowerarm_{s}")
                ab = b - a
                tt = ((body.pos - a) @ ab) / (ab @ ab)
                k &= ~((body.bw([f"upperarm_{s}", f"lowerarm_{s}"]) > 0.5) & (tt > 0.55))
        return k
    return Garment(tex, color, offset, region(TORSO + ARMS + [r"pelvis"], exclude=[HANDS, NECK], exclude_w=0.25, keep=keep), drape=40)


def pants(tex, color, offset=0.017, cuff=0.07):
    def keep(body):
        k = np.ones(len(body.pos), dtype=bool)
        top = body.jpos("pelvis")[1] + 0.11
        k &= body.pos[:, 1] < top
        for s in "lr":
            ank = body.jpos(f"foot_{s}")[1] + cuff
            k &= ~((body.bw([f"calf_{s}"]) > 0.3) & (body.pos[:, 1] < ank))
        return k
    return Garment(tex, color, offset, region([r"pelvis", r"thigh_[lr]", r"calf_[lr]", r"spine_01"], exclude=[FEET], exclude_w=0.2, keep=keep), drape=70)


def boots(tex, color, height=0.16):
    def keep(body):
        k = np.zeros(len(body.pos), dtype=bool)
        for s in "lr":
            k |= body.pos[:, 1] < body.jpos(f"foot_{s}")[1] + height
        return k
    def toes(pos, jn, wt, body):
        ids = [i for i, n in enumerate(body.sk.names) if n.startswith("ball")]
        return (wt * np.isin(jn, ids)).sum(1) > 0.05
    return Garment(tex, color, 0.014, region(FEET + [r"calf_[lr]"], keep=keep), folds=0.3, drape=80, sole=0.012, free=toes)


def gloves(color):
    return Garment("Leather", color, 0.004, region(HANDS), folds=0.3, drape=2)


def helmet(color):
    def keep(body):
        h = body.jpos("Head")
        return body.pos[:, 1] > h[1] + 0.085
    return Garment("Tarp", color, 0.03, region([r"Head"], keep=keep), folds=0.15, drape=30)


def vest(color):
    def keep(body):
        return body.pos[:, 1] > body.jpos("pelvis")[1] + 0.02
    return Garment("Camo", color, 0.032, region(TORSO + [r"clavicle_[lr]"], exclude=[ARMS[1:], NECK], exclude_w=0.35, keep=keep), folds=0.5, plates=0.006, drape=60)


# variants: (model name, body, skin tone, hairstyles, garments)
HAIR = {"Human_Player": (0.06, 0.045, 0.035), "Human_Soldier_M": (0.22, 0.16, 0.1), "Human_Soldier_F": (0.07, 0.05, 0.04),
        "Human_Civilian_M1": (0.32, 0.22, 0.13), "Human_Civilian_M2": (0.05, 0.04, 0.035), "Human_Civilian_F1": (0.5, 0.38, 0.22), "Human_Civilian_F2": (0.12, 0.07, 0.045)}

VARIANTS = [
    ("Human_Player", "Male", "Dark", ["Hair_Buzzed", "Hair_Beard"], lambda: [shirt("Cloth", (0.32, 0.3, 0.27)), shirt("Leather", (0.24, 0.13, 0.07), offset=0.024, drop=0.12), pants("Denim", (0.14, 0.18, 0.26)), boots("Leather", (0.12, 0.09, 0.07))]),
    ("Human_Soldier_M", "Male", "Light", [], lambda: [shirt("Camo", (0.62, 0.66, 0.5)), pants("Camo", (0.62, 0.66, 0.5)), vest((0.42, 0.46, 0.32)), helmet((0.23, 0.26, 0.17)), gloves((0.1, 0.1, 0.09)), boots("Leather", (0.09, 0.08, 0.07), 0.22)]),
    ("Human_Soldier_F", "Female", "Dark", ["Hair_Buns"], lambda: [shirt("Camo", (0.62, 0.66, 0.5)), pants("Camo", (0.62, 0.66, 0.5)), vest((0.42, 0.46, 0.32)), gloves((0.1, 0.1, 0.09)), boots("Leather", (0.09, 0.08, 0.07), 0.22)]),
    ("Human_Civilian_M1", "Male", "Light", ["Hair_SimpleParted"], lambda: [shirt("Cloth", (0.55, 0.12, 0.1), sleeves="short"), pants("Denim", (0.2, 0.26, 0.38)), boots("Leather", (0.75, 0.73, 0.7), 0.1)]),
    ("Human_Civilian_M2", "Male", "Dark", ["Hair_Buzzed"], lambda: [shirt("Cloth", (0.6, 0.62, 0.64)), pants("Cloth", (0.18, 0.17, 0.16)), boots("Leather", (0.2, 0.12, 0.07))]),
    ("Human_Civilian_F1", "Female", "Light", ["Hair_Long"], lambda: [shirt("Cloth", (0.2, 0.32, 0.36), sleeves="short"), pants("Denim", (0.12, 0.13, 0.18)), boots("Leather", (0.1, 0.09, 0.09), 0.12)]),
    ("Human_Civilian_F2", "Female", "Dark", ["Hair_Buns"], lambda: [shirt("Cloth", (0.62, 0.55, 0.42)), shirt("Leather", (0.14, 0.12, 0.11), offset=0.022, drop=0.1), pants("Cloth", (0.3, 0.27, 0.22)), boots("Leather", (0.16, 0.1, 0.06))]),
]


def tint_to_vertex(color, tex, means):
    """Vertex colour (GLB convention) that makes layer `tex` average to sRGB `color`."""
    lin = srgb_to_lin(np.array(color))
    m = means.get(tex)
    if m is None:
        m = np.full(3, 0.35)
    c = np.clip(lin / m, 0, 1) ** 2.2
    return np.append(c, 1.0)


def build_character(name, gpath, tone, hairs, garments, ubc, means, seed):
    g = Gltf(gpath)
    sk = Skel(g)
    body = Body(g, sk)
    sex = "male" if "Male" in os.path.basename(gpath) else "female"
    prims = []  # (material, pos, nrm, uv, col, j, w, tri)
    covered = np.zeros(len(body.pos), dtype=bool)
    for k, gm in enumerate(garments):
        m = garment_mesh(body, gm, seed + k * 3.1)
        # hide skin that is fully inside a garment (keep a ring at its edges)
        inner = np.setdiff1d(m["used"], m["used"][boundary_verts(m["tri"])])
        covered[inner] = True
        uv = box_uv(m["pos"], m["nrm"], np.full(len(m["pos"]), {"Cloth": 8.0, "Denim": 6.5, "Leather": 4.0, "Camo": 3.0, "Tarp": 5.0}.get(m["tex"], 4.0)))
        col = np.tile(tint_to_vertex(m["color"], m["tex"], means), (len(m["pos"]), 1))
        # grime toward the hem and the ground
        col[:, :3] *= (0.8 + 0.2 * np.clip(m["pos"][:, 1] / 1.0, 0, 1))[:, None]
        prims.append(("T:" + m["tex"], m["pos"], m["nrm"], uv, col, m["j"], m["w"], m["tri"]))
    keep = ~covered[body.tri].all(axis=1)
    skin_tex = f"C:skin_{sex}_{tone.lower()}"
    white = np.ones((len(body.pos), 4))
    prims.append((skin_tex, body.pos, body.nrm, body.parts["skin"]["uv"], white, body.j, body.w, body.tri[keep]))
    e = body.parts["eyes"]
    prims.append(("C:eyes", e["pos"], e["nrm"], e["uv"], np.ones((len(e["pos"]), 4)), e["j"], e["w"], e["tri"]))
    # hair textures are neutral grey, made to be tinted
    hc = np.append(np.clip(srgb_to_lin(np.array(HAIR.get(name, (0.1, 0.08, 0.06)))) / 0.27, 0, 1) ** 2.2, 1.0)
    b = body.parts.get("brows")
    if b is not None:
        prims.append((f"C:hair{'1' if sex == 'male' else '2'}", b["pos"], b["nrm"], b["uv"], np.tile(hc, (len(b["pos"]), 1)), b["j"], b["w"], b["tri"]))
    for h in hairs:
        hg = Gltf(next(p for p in glob.glob(os.path.join(glob.escape(ubc), "**", f"{h}.gltf"), recursive=True) if "Rigged to Head Bone" in p))
        hn = hg.joint_names()
        # move the hair from its own head bind pose into this body's
        hb = hg.bind_matrices()["Head"]
        bb = g.bind_matrices()["Head"]
        xf = bb @ np.linalg.inv(hb)
        for _, p in hg.skinned_prims():
            a = p["attributes"]
            mat = hg.j["materials"][p["material"]]["name"]
            jn = hg.acc(a["JOINTS_0"]).astype(np.int64)
            jn = np.vectorize(lambda x: sk.find(hn[x]) or 0)(jn)
            pos = hg.acc(a["POSITION"]).astype(np.float64)
            pos = pos @ xf[:3, :3].T + xf[:3, 3]
            hnrm = hg.acc(a["NORMAL"]).astype(np.float64) @ xf[:3, :3].T
            prims.append((f"C:hair{'1' if mat.endswith('1') else '2'}", pos, hnrm, hg.acc(a["TEXCOORD_0"]).astype(np.float64),
                          np.tile(hc, (len(pos), 1)), jn, hg.acc(a["WEIGHTS_0"]).astype(np.float64), hg.acc(p["indices"]).astype(np.int64).reshape(-1, 3)))
    write_character(g, name, prims)


def write_character(g, name, prims, extras=None):
    mats = []
    out = []
    for mat, pos, nrm, uv, col, jn, wt, tri in prims:
        if len(tri) == 0:
            continue
        used, local = np.unique(tri.ravel(), return_inverse=True)
        if mat not in mats:
            mats.append(mat)
        attrs = {
            "POSITION": g.add(pos[used].astype(np.float32), "pos", 5126, target=34962, minmax=True),
            "NORMAL": g.add(nrm[used].astype(np.float32), "nrm", 5126, target=34962),
            "TEXCOORD_0": g.add(uv[used].astype(np.float32), "uv", 5126, target=34962),
            "COLOR_0": g.add(np.round(np.clip(col[used], 0, 1) * 255), "color", 5121, target=34962),
            "JOINTS_0": g.add(jn[used].astype(np.uint16), "joints", 5123, target=34962),
            "WEIGHTS_0": g.add(wt[used].astype(np.float32), "weights", 5126, target=34962),
        }
        out.append({"attributes": attrs, "indices": g.add(local.ravel().astype(np.uint32), "idx", 5125, target=34963), "material": mats.index(mat)})
    g.j["materials"] = [{"name": m, "pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 0.8}} for m in mats]
    for k in ("textures", "images", "samplers", "animations"):
        g.j.pop(k, None)
    g.j["meshes"] = [{"name": name, "primitives": out}]
    first = True
    for nd in g.j["nodes"]:
        if "mesh" in nd:
            if first:
                nd["mesh"] = 0
                first = False
            else:
                nd.pop("mesh")
    if extras:
        g.j.setdefault("asset", {})["extras"] = extras
    path = os.path.join(MODELS, name + ".glb")
    g.save(path)
    print(f"{os.path.relpath(path, ROOT)}: {sum(len(p[7]) for p in prims)} triangles, materials {mats}")


def main():
    ubc = source_dir("STRAIN_UBC_DIR", "universal-base-characters")
    ual = source_dir("STRAIN_UAL_DIR", "universal-animation-library")
    tex = os.path.join(ubc, "Universal Base Characters[Standard]", "Base Characters", "Textures")
    if not os.path.isdir(tex):
        tex = os.path.dirname(find(ubc, "T_Superhero_Male_Dark.png"))
    nrm_dir = os.path.dirname(find(ubc, "T_Superhero_Male_Normal.png")) if False else os.path.join(tex, "Normals Unity - Godot")
    # per-character textures
    for sex, short in (("Male", "male"), ("Female", "female")):
        for tone in ("Dark", "Light"):
            alb = [f for f in glob.glob(os.path.join(glob.escape(tex), f"T_Superhero_{sex}_{tone[:4]}*.png"))][0]
            save_char_texture(f"skin_{short}_{tone.lower()}", alb, os.path.join(nrm_dir, f"T_Superhero_{sex}_Normal.png"), os.path.join(tex, f"T_Superhero_{sex}_Roughness.png"), sss=1.0)
    save_char_texture("eyes", os.path.join(tex, "T_Eye_Brown.png"), os.path.join(nrm_dir, "T_Eye_Normal.png"), rough=0.08)
    for h in ("1", "2"):
        save_char_texture(f"hair{h}", os.path.join(tex, f"T_Hair_{h}_BaseColor.png"), os.path.join(nrm_dir, f"T_Hair_{h}_Normal.png"), rough=0.45, alpha=True)
    bodies = {s: find(ubc, f"Superhero_{s}_FullBody.gltf") for s in ("Male", "Female")}
    bodies = {s: next(p for p in glob.glob(os.path.join(glob.escape(ubc), "**", f"Superhero_{s}_FullBody.gltf"), recursive=True) if "Godot" in p) for s in bodies}
    means = layer_means()
    means["Camo"] = np.full(3, 0.05)
    means["Tarp"] = np.full(3, 0.12)
    # clip libraries
    ual_g = Glb(find(ual, "UAL1_Standard.glb"))
    ual_sk = Skel(ual_g)
    kit_g = Glb(os.path.join(MODELS, "Zombie_Basic.glb"))
    kit_sk = Skel(kit_g)
    keep = {"Idle_Loop", "Walk_Loop", "Jog_Fwd_Loop", "Sprint_Loop", "Crouch_Idle_Loop", "Crouch_Fwd_Loop", "Roll", "Sword_Attack", "Sword_Idle",
            "Punch_Cross", "Punch_Jab", "Hit_Chest", "Hit_Head", "Death01", "Pistol_Idle_Loop", "Pistol_Aim_Neutral", "Pistol_Shoot",
            "Spell_Simple_Shoot", "Spell_Simple_Idle_Loop", "Interact", "Jump_Start", "Jump_Loop", "Jump_Land", "Idle_Torch_Loop"}
    zombie = {"Walk": "Z_Walk", "Run": "Z_Run", "Idle": "Z_Idle", "Punch": "Z_Attack", "Run_Attack": "Z_Lunge", "Idle_Attack": "Z_Scream", "Crawl": "Z_Crawl", "HitReact": "Z_HitReact"}
    ue_map = {n: n for n in UE_CHILD} | {"Head": "Head"}
    for s, path in bodies.items():
        dst = Skel(Gltf(path))
        clips = []
        for a in ual_g.j["animations"]:
            if a["name"] in keep:
                clips.append(retarget(ual_g, ual_sk, a, dst, ue_map, UE_CHILD, UE_CHILD, a["name"]))
        for a in kit_g.j["animations"]:
            if a["name"] in zombie:
                clips.append(retarget(kit_g, kit_sk, a, dst, FROM_KIT, KIT_CHILD, UE_CHILD, zombie[a["name"]]))
        write_library(path, os.path.join(MODELS, f"Clips_{s}.glb"), clips)
    for i, (name, sex, tone, hairs, gar) in enumerate(VARIANTS):
        build_character(name, bodies[sex], tone, hairs, gar(), ubc, means, seed=i * 7.3)


if __name__ == "__main__":
    import urllib.parse  # noqa: F401

    main()
