"""Build the custom infected models (assets/models/Strain_*.glb) with Blender.

Run with a Python that can `import bpy` (Blender's Python, or `pip install bpy`):

    python -I tools/blender/build_enemies.py [names...] [--preview DIR]

Each model starts from a rigged, animated Quaternius zombie (CC0). Blender builds the
Lovecraftian growths (bone spikes, tentacles, tumours, chitin plates, maws, crystals, sacs)
procedurally, adds displacement detail, and bakes ambient occlusion into vertex colours with
Cycles. The new parts take their skin weights from the nearest body vertices. The body itself is
reshaped and repainted (veins, bruising, blood, embers).

The result is spliced into the original GLB: the skeleton, inverse bind matrices and animation
clips stay byte-for-byte identical, so clips can still be shared between models by bone name.
New parts use materials named `T:<Tex>` that the game maps to its texture layers; vertex colour
alpha below 1 marks emission.
"""
import math
import os
import random
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import bpy  # noqa: E402  (must come first: it provides bmesh and mathutils)
import bmesh  # noqa: E402
import numpy as np  # noqa: E402
from mathutils import Matrix, Vector, bvhtree, kdtree, noise  # noqa: E402

from glb import Glb  # noqa: E402

ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
MODELS = os.path.join(ROOT, "assets", "models")
UP = Vector((0, 1, 0))
FWD = Vector((0, 0, 1))
LEFT = Vector((1, 0, 0))

# Base colour multipliers per texture, applied over the game's photo textures.
PALETTE = {
    "Flesh": (1.0, 0.82, 0.8),
    "Vein": (0.9, 0.75, 0.85),
    "Bone": (1.0, 0.95, 0.85),
    "Chitin": (0.55, 0.5, 0.48),
    "Growth": (0.95, 0.9, 0.8),
    "GrowthGlow": (1.0, 1.0, 1.0),
    "Char": (0.5, 0.45, 0.42),
    "Ember": (1.0, 1.0, 1.0),
    "Lattice": (1.0, 1.0, 1.0),
    "RiftGlow": (1.0, 1.0, 1.0),
    "RiftStone": (0.8, 0.8, 0.85),
    "Skin": (1.0, 1.0, 1.0),
}


# ------------------------------------------------------------------------------------------------
# Small helpers

def fbm(p, octaves=3):
    return noise.fractal(Vector(p), 0.5, 2.0, octaves, noise_basis="PERLIN_ORIGINAL")


def vein(p, scale=6.0, sharp=10):
    """Thin dark lines: ridges of turbulence."""
    v = 1.0 - abs(noise.noise(Vector(p) * scale, noise_basis="PERLIN_ORIGINAL"))
    return max(0.0, v) ** sharp


def frame(d):
    d = d.normalized()
    a = UP if abs(d.dot(UP)) < 0.9 else LEFT
    x = d.cross(a).normalized()
    y = x.cross(d).normalized()
    return x, y


def bezier(a, b, c, n):
    return [(1 - t) ** 2 * a + 2 * (1 - t) * t * b + t * t * c for t in (i / (n - 1) for i in range(n))]


def catmull(pts, per=4):
    out = []
    p = [pts[0]] + list(pts) + [pts[-1]]
    for i in range(1, len(p) - 2):
        for k in range(per):
            t = k / per
            t2, t3 = t * t, t * t * t
            out.append(0.5 * ((2 * p[i]) + (-p[i - 1] + p[i + 1]) * t + (2 * p[i - 1] - 5 * p[i] + 4 * p[i + 1] - p[i + 2]) * t2 + (-p[i - 1] + 3 * p[i] - 3 * p[i + 1] + p[i + 2]) * t3))
    out.append(pts[-1])
    return out


class Part:
    def __init__(self, obj, tex, mode="anchor", anchor=None, glow=0.0, color=None, veins=0.0, ao=True, rough=0.0):
        self.obj = obj
        self.tex = tex
        self.mode = mode
        self.anchor = anchor
        self.glow = glow
        self.color = color or PALETTE.get(tex, (1, 1, 1))
        self.veins = veins
        self.ao = ao
        self.rough = rough


# ------------------------------------------------------------------------------------------------
# Builder: one model

class Builder:
    def __init__(self, name, base, seed=1):
        self.name = name
        self.base = base
        self.g = Glb(base if base.endswith(".glb") else os.path.join(MODELS, base + ".glb"))
        self.rng = random.Random(seed)
        self.parts = []
        self.joint_names = self.g.joint_names()
        self.jpos = {k: Vector(v) for k, v in self.g.bind_positions().items()}
        nodes = self.g.j["nodes"]
        joints = self.g.skin()["joints"]
        jset = set(joints)
        self.children = {nodes[n].get("name", ""): [nodes[c].get("name", "") for c in nodes[n].get("children", []) if c in jset] for n in joints}
        # body primitives
        self.prims = []
        for mi, p in self.g.skinned_prims():
            a = p["attributes"]
            pos = self.g.acc(a["POSITION"]).astype(np.float64)
            nrm = self.g.acc(a["NORMAL"]).astype(np.float64)
            jn = self.g.acc(a["JOINTS_0"]).astype(np.int64)
            wt = self.g.acc(a["WEIGHTS_0"]).astype(np.float64)
            idx = self.g.acc(p["indices"]).astype(np.int64) if "indices" in p else np.arange(len(pos))
            col = self.g.acc(a["COLOR_0"]).astype(np.float64) if "COLOR_0" in a else np.ones((len(pos), 4))
            if col.shape[1] == 3:
                col = np.concatenate([col, np.ones((len(col), 1))], axis=1)
            self.prims.append({"mesh": mi, "prim": p, "pos": pos, "nrm": nrm, "j": jn, "w": wt, "idx": idx, "col": col})
        self.main = max(self.prims, key=lambda q: len(q["pos"]))
        self.skin_tex = "Skin"
        self._sample_atlas()
        self.height = max(q["pos"][:, 1].max() for q in self.prims)
        self._bvh = None

    def _sample_atlas(self, skin_test=None):
        """Colour of every body vertex in the shared atlas, and whether it is bare skin."""
        img = bpy.data.images.get("atlas.png") or bpy.data.images.load(os.path.join(MODELS, "atlas.png"))
        w, h = img.size
        px = np.zeros(w * h * 4, dtype=np.float32)
        img.pixels.foreach_get(px)
        px = px.reshape(h, w, 4)
        test = skin_test or (lambda c: c[1] > c[0] * 1.08 and c[1] > c[2] * 1.25 and c.max() - c.min() > 0.12)
        for q in self.prims:
            uv = self.g.acc(q["prim"]["attributes"]["TEXCOORD_0"])
            x = np.clip((uv[:, 0] % 1.0) * w, 0, w - 1).astype(int)
            y = np.clip((1.0 - uv[:, 1] % 1.0) * h, 0, h - 1).astype(int)
            q["atlas"] = px[y, x, :3].astype(np.float64)
            q["skin"] = np.array([bool(test(c)) for c in q["atlas"]])
            # the cartoon's black outlines, brows and pupils on the head become dark, sunken skin
            dark = q["atlas"].max(axis=1) < 0.16
            head = self.bone_w(q, self.subtree("Head")) > 0.5 if "Head" in self.jpos else np.zeros(len(dark), dtype=bool)
            q["socket"] = dark & head
            q["skin"] = q["skin"] | q["socket"]

    def reskin(self, skin_test):
        self._sample_atlas(skin_test)

    def drop(self, colors, tol=0.07, where=None):
        """Remove the faces of body parts painted in one of the atlas `colors` (backpacks,
        bandanas, the parts a growth replaces). `where(pos)` can restrict it further."""
        for q in self.prims:
            m = np.zeros(len(q["pos"]), dtype=bool)
            for c in colors:
                m |= np.abs(q["atlas"] - np.array(c)).max(axis=1) < tol
            if where is not None:
                m &= np.array([bool(where(p)) for p in q["pos"]])
            q["drop"] = q.get("drop", np.zeros(len(m), dtype=bool)) | m
        self._bvh = None

    # ---- skeleton / body queries
    def J(self, name):
        return self.jpos[name].copy()

    def subtree(self, name):
        out = [name]
        for c in self.children.get(name, []):
            out += self.subtree(c)
        return out

    def bone_w(self, prim, bones):
        ids = [self.joint_names.index(b) for b in bones if b in self.joint_names]
        m = np.isin(prim["j"], ids)
        return (prim["w"] * m).sum(axis=1)

    def region(self, bones, thresh=0.5):
        """Centre, min and max of the body vertices mostly weighted to `bones`."""
        pts = np.concatenate([q["pos"][self.bone_w(q, bones) > thresh] for q in self.prims])
        if len(pts) == 0:
            c = self.J(bones[0])
            return c, c, c
        return Vector(pts.mean(axis=0)), Vector(pts.min(axis=0)), Vector(pts.max(axis=0))

    def bvh(self):
        if self._bvh is None:
            verts, tris = self.body_arrays()
            self._bvh = bvhtree.BVHTree.FromPolygons([Vector(v) for v in verts], [tuple(t) for t in tris])
        return self._bvh

    def body_arrays(self):
        verts, tris, off = [], [], 0
        for q in self.prims:
            verts.append(q["pos"])
            t = q["idx"].reshape(-1, 3)
            if "drop" in q:
                t = t[~q["drop"][t].any(axis=1)]
            tris.append(t + off)
            off += len(q["pos"])
        return np.concatenate(verts), np.concatenate(tris)

    def surf(self, origin, direction, fallback=0.15):
        """Point on the body surface hit by a ray cast from outside towards `origin` along -direction."""
        o = Vector(origin)
        d = Vector(direction).normalized()
        hit = self.bvh().ray_cast(o + d * 2.0, -d, 4.0)
        if hit[0] is None:
            return o + d * fallback, d
        return Vector(hit[0]), Vector(hit[1])

    # ---- body edits
    def inflate(self, bones, amount, lumpy=0.0, seed=0):
        for q in self.prims:
            w = self.bone_w(q, bones)
            for i in np.nonzero(w > 0.01)[0]:
                p = q["pos"][i]
                n = 1.0 + lumpy * fbm(p * 5.0 + seed)
                q["pos"][i] = p + q["nrm"][i] * amount * w[i] * n

    def scale_about(self, bones, center, s):
        c = np.array(center)
        for q in self.prims:
            w = self.bone_w(q, bones)[:, None]
            q["pos"] = q["pos"] + (q["pos"] - c) * (np.array(s) - 1.0) * w

    def collapse(self, bone, to=None, amount=0.97):
        tgt = np.array(to if to is not None else self.J(bone))
        for q in self.prims:
            w = self.bone_w(q, self.subtree(bone))[:, None] * amount
            q["pos"] = q["pos"] * (1 - w) + tgt * w

    def paint(self, skin=(0.7, 0.8, 0.95), cloth_tint=(1, 1, 1), tint=(1, 1, 1), veins=0.5, bruise=0.3, blood=(), ember=0.0, char=0.0, glow_veins=0.0, glow_col=(1, 1, 1), seed=0, mottle=0.35, grime=0.35, skin_tex="Skin", gore=0.0):
        """Repaint the body. Bare skin moves onto a photo texture (`skin_tex`) coloured `skin` with
        mottling; clothes keep the atlas colours under dirt. Then dark veins, bruising, blood
        around bones, charred skin with glowing cracks, all scaled by `tint`."""
        self.skin_tex = skin_tex
        bl = [(self.J(b), r) for b, r in blood if b in self.jpos]
        for q in self.prims:
            col = q["col"]
            for i, p in enumerate(q["pos"]):
                v = Vector(p) + Vector((seed, seed * 0.7, 0))
                if q["socket"][i]:
                    c = np.array(skin, dtype=np.float64) * np.array([0.32, 0.16, 0.15])
                elif q["skin"][i]:
                    m = fbm(v * 4.0 + Vector((3, 1, 2)))
                    c = np.array(skin, dtype=np.float64) * (1.0 - mottle * 0.5 + mottle * m) * np.array(tint)
                else:
                    # clothing moves onto the fabric photo texture: the atlas colour, faded and
                    # dirtied (the game's cloth layer averages about 0.6 grey)
                    a = q["atlas"][i]
                    lum = float(a @ np.array([0.3, 0.55, 0.15]))
                    faded = a * 0.6 + lum * 0.4
                    c = np.clip(faded / 0.6, 0.0, 1.0) * np.array(tint, dtype=np.float64) * np.array(cloth_tint)
                    c *= 1.0 - grime * max(0.0, fbm(v * 3.0 + Vector((9, 9, 9))) + 0.3)
                    c *= np.array([0.95, 0.92, 0.86])
                vn = vein(v, 5.0, 5)
                c *= 1.0 - veins * vn * np.array([0.55, 0.75, 0.6])
                b = max(0.0, fbm(v * 2.5))
                c *= 1.0 - bruise * b * np.array([0.2, 0.55, 0.35])
                if gore > 0:
                    gk = max(0.0, fbm(v * 2.2 + Vector((5, 1, 7))) - (0.55 - gore * 0.35)) * 4.0
                    if gk > 0:
                        gk = min(1.0, gk)
                        c = c * (1 - gk) + np.array([0.55, 0.1, 0.1]) * (0.7 + 0.3 * vein(v, 11.0, 3)) * gk
                for jp, r in bl:
                    dv = Vector(p) - jp
                    if dv.y < 0:
                        dv.y *= 0.35 + 0.3 * abs(noise.noise(Vector((p[0] * 30, 0, p[2] * 30))))
                    d = dv.length
                    if d < r:
                        k = (1 - d / r) * (0.6 + 0.4 * fbm(v * 9.0))
                        c = c * (1 - k) + np.array([0.45, 0.04, 0.05]) * k
                e = 0.0
                if char > 0:
                    cr = abs(noise.noise(v * 4.0, noise_basis="VORONOI_F2F1"))
                    crack = max(0.0, 1.0 - cr * 6.0)
                    c *= 1.0 - char * 0.75
                    if ember > 0 and crack > 0:
                        c = c * (1 - crack) + np.array([3.0, 1.1, 0.25]) * crack
                        e = max(e, ember * crack)
                gv = vein(v + Vector((11, 5, 3)), 6.0, 24) if glow_veins > 0 else 0.0
                if gv > 0.35:
                    k = (gv - 0.35) / 0.65
                    c = c * (1 - k) + np.array(glow_col) * k
                    e = max(e, glow_veins * k)
                col[i, :3] = col[i, :3] * c
                col[i, 3] = min(col[i, 3], 1.0 - e)

    # ---- part generators (all coordinates in glTF model space: +Y up, +Z forward, +X left)
    def _obj(self, bm, name):
        me = bpy.data.meshes.new(name)
        bm.to_mesh(me)
        bm.free()
        ob = bpy.data.objects.new(name, me)
        bpy.context.scene.collection.objects.link(ob)
        return ob

    def tube(self, path, radii, sides=8, cap=True, twist=0.0, bumps=0.0, name="tube"):
        bm = bmesh.new()
        rings = []
        x, y = frame(path[1] - path[0])
        for i, p in enumerate(path):
            d = (path[min(i + 1, len(path) - 1)] - path[max(i - 1, 0)]).normalized()
            # parallel transport
            x = (x - d * x.dot(d)).normalized()
            y = d.cross(x).normalized()
            r = radii[i]
            ring = []
            for k in range(sides):
                a = k / sides * math.tau + twist * i
                rr = r * (1.0 + bumps * math.sin(a * 3 + i * 1.7) * math.sin(i * 2.1))
                ring.append(bm.verts.new(p + (x * math.cos(a) + y * math.sin(a)) * rr))
            rings.append(ring)
        for a, b in zip(rings, rings[1:]):
            for k in range(sides):
                bm.faces.new((a[k], a[(k + 1) % sides], b[(k + 1) % sides], b[k]))
        if cap:
            tip = bm.verts.new(path[-1] + (path[-1] - path[-2]).normalized() * radii[-1] * 0.8)
            for k in range(sides):
                bm.faces.new((rings[-1][k], rings[-1][(k + 1) % sides], tip))
            base = bm.verts.new(path[0] - (path[1] - path[0]).normalized() * radii[0] * 0.3)
            for k in range(sides):
                bm.faces.new((rings[0][(k + 1) % sides], rings[0][k], base))
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        return self._obj(bm, name)

    def spike(self, base, direction, length, radius, bend=None, bend_amt=0.25, sides=7, tex="Bone", glow=0.0, color=None, anchor=None):
        d = Vector(direction).normalized()
        bd = Vector(bend).normalized() if bend is not None else frame(d)[0]
        a = Vector(base)
        c = a + d * length
        mid = a + d * length * 0.5 + bd * length * bend_amt
        path = bezier(a, mid, c, 7)
        radii = [radius * (1 - t) ** 0.9 + 0.002 for t in (i / 6 for i in range(7))]
        ob = self.tube(path, radii, sides=sides, cap=True, name="spike")
        self.parts.append(Part(ob, tex, anchor=anchor if anchor is not None else a, glow=glow, color=color))
        return ob

    def tentacle(self, pts, r0, r1, tex="Flesh", sides=8, bumps=0.12, color=None, glow=0.0, anchor=None, veins=0.6):
        path = catmull([Vector(p) for p in pts], per=5)
        n = len(path)
        radii = [r0 + (r1 - r0) * (i / (n - 1)) ** 0.8 for i in range(n)]
        ob = self.tube(path, radii, sides=sides, bumps=bumps, twist=0.25, name="tentacle")
        self._detail(ob, 0.006 + r0 * 0.08, subdiv=1)
        self.parts.append(Part(ob, tex, anchor=anchor if anchor is not None else Vector(pts[0]), color=color, glow=glow, veins=veins))
        return ob

    def blob(self, center, radius, lobes=5, squash=(1, 1, 1), tex="Flesh", seed=0, mode="surface", color=None, glow=0.0, veins=0.7, rough=0.25, anchor=None):
        bm = bmesh.new()
        bmesh.ops.create_icosphere(bm, subdivisions=4 if radius > 0.11 else 3, radius=1.0)
        rng = random.Random(seed)
        dirs = [Vector((rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1))).normalized() for _ in range(lobes)]
        amps = [rng.uniform(0.15, 0.45) for _ in range(lobes)]
        for v in bm.verts:
            n = v.co.normalized()
            r = 1.0 + sum(a * max(0.0, n.dot(d)) ** 4 for d, a in zip(dirs, amps)) + rough * fbm(n * 2.2 + Vector((seed, 0, 0)))
            v.co = Vector((n.x * squash[0], n.y * squash[1], n.z * squash[2])) * r * radius + Vector(center)
        ob = self._obj(bm, "blob")
        self._detail(ob, radius * 0.06, subdiv=0)
        self.parts.append(Part(ob, tex, mode=mode, anchor=anchor if anchor is not None else Vector(center), color=color, glow=glow, veins=veins))
        return ob

    def pustules(self, center, radius, n, size, seed=0, glow=0.8, color=(0.8, 2.2, 0.6), tex="GrowthGlow", normal=None):
        """Small glowing sacs scattered over a spherical patch."""
        rng = random.Random(seed)
        for _ in range(n):
            d = Vector((rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1))).normalized()
            if normal is not None and d.dot(Vector(normal)) < 0:
                d = -d
            s = size * rng.uniform(0.6, 1.3)
            self.blob(Vector(center) + d * radius, s, lobes=2, tex=tex, seed=rng.randint(0, 999), mode="anchor", color=color, glow=glow, veins=0.0, rough=0.1)

    def plate(self, center, normal, up, w, h, curve=0.25, thick=0.03, ridges=5, tex="Chitin", color=None, mode="surface", anchor=None, taper=0.0, segs=(8, 8)):
        n = Vector(normal).normalized()
        u = Vector(up)
        u = (u - n * u.dot(n)).normalized()
        r = u.cross(n).normalized()
        bm = bmesh.new()
        nu, nv = segs
        grid = []
        for j in range(nv + 1):
            row = []
            for i in range(nu + 1):
                s = i / nu * 2 - 1
                t = j / nv * 2 - 1
                ww = w * (1 - taper * (t * 0.5 + 0.5))
                ridge = 0.25 * thick * math.sin((t * 0.5 + 0.5) * ridges * math.pi) ** 2
                p = Vector(center) + r * s * ww + u * t * h - n * (curve * (s * s * w * w + t * t * h * h)) + n * ridge
                row.append(bm.verts.new(p))
            grid.append(row)
        for j in range(nv):
            for i in range(nu):
                bm.faces.new((grid[j][i], grid[j][i + 1], grid[j + 1][i + 1], grid[j + 1][i]))
        ob = self._obj(bm, "plate")
        m = ob.modifiers.new("solid", "SOLIDIFY")
        m.thickness = thick
        m.offset = -1.0
        self._apply(ob)
        self._detail(ob, thick * 0.3, subdiv=0)
        self.parts.append(Part(ob, tex, mode=mode, anchor=anchor if anchor is not None else Vector(center), color=color))
        return ob

    def maw(self, center, normal, radius, teeth=10, tooth_len=None, tex_lip="Flesh", color=None, anchor=None, inner=(0.18, 0.02, 0.03)):
        n = Vector(normal).normalized()
        x, y = frame(n)
        tl = tooth_len or radius * 0.7
        ring = [Vector(center) + (x * math.cos(a) + y * math.sin(a)) * radius for a in (k / 16 * math.tau for k in range(17))]
        lip = self.tube(ring, [radius * 0.28] * 17, sides=8, cap=False, bumps=0.15, name="lip")
        self._detail(lip, radius * 0.04)
        self.parts.append(Part(lip, tex_lip, anchor=anchor if anchor is not None else Vector(center), color=color, veins=0.8))
        for k in range(teeth):
            a = k / teeth * math.tau + 0.2
            rd = x * math.cos(a) + y * math.sin(a)
            base = Vector(center) + rd * radius * 0.92
            self.spike(base, (-rd * 0.75 + n * 0.55), tl * self.rng.uniform(0.7, 1.15), radius * 0.13, bend=n, bend_amt=0.15, sides=5, anchor=anchor if anchor is not None else Vector(center), color=(1.0, 0.92, 0.75))
        # dark gullet
        self.blob(Vector(center) - n * radius * 0.25, radius * 0.85, lobes=2, squash=(1, 1, 1), tex="Flesh", seed=3, mode="anchor", color=inner, veins=0.0, rough=0.1, anchor=anchor if anchor is not None else Vector(center))

    def crystal(self, base, direction, length, radius, tex="Lattice", glow=0.7, color=(1.2, 0.9, 2.2), anchor=None):
        d = Vector(direction).normalized()
        a = Vector(base)
        path = [a, a + d * length * 0.15, a + d * length * 0.75, a + d * length]
        radii = [radius * 0.7, radius, radius, 0.0005]
        ob = self.tube(path, radii, sides=6, cap=True, name="crystal")
        self.parts.append(Part(ob, tex, anchor=anchor if anchor is not None else a, glow=glow, color=color, ao=False))
        ob["flat"] = True
        return ob

    def petal(self, base, direction, length, width, curl=0.6, tex="Flesh", color=None, anchor=None, glow=0.0):
        d = Vector(direction).normalized()
        x, y = frame(d)
        bm = bmesh.new()
        rows = []
        nv, nu = 8, 4
        for j in range(nv + 1):
            t = j / nv
            wid = width * math.sin(min(1.0, t * 1.15) * math.pi) ** 0.7
            ang = curl * t * t
            dd = (d * math.cos(ang) + y * math.sin(ang)).normalized()
            centre = Vector(base) + d * length * t * 0.6 + dd * length * t * 0.4
            row = []
            for i in range(nu + 1):
                s = i / nu * 2 - 1
                row.append(bm.verts.new(centre + x * s * wid + y * (s * s) * wid * 0.35))
            rows.append(row)
        for j in range(nv):
            for i in range(nu):
                bm.faces.new((rows[j][i], rows[j][i + 1], rows[j + 1][i + 1], rows[j + 1][i]))
        ob = self._obj(bm, "petal")
        m = ob.modifiers.new("solid", "SOLIDIFY")
        m.thickness = width * 0.12
        self._apply(ob)
        self.parts.append(Part(ob, tex, anchor=anchor if anchor is not None else Vector(base), color=color, veins=1.0, glow=glow))
        return ob

    def pipe(self, base, direction, length, radius, tex="Bone", color=None, anchor=None, bend=None):
        d = Vector(direction).normalized()
        a = Vector(base)
        bd = Vector(bend) if bend is not None else frame(d)[0]
        path = bezier(a, a + d * length * 0.5 + bd * length * 0.1, a + d * length, 6)
        outer = self.tube(path, [radius * (1.0 + 0.25 * (i == 5)) for i in range(6)], sides=10, cap=False, name="pipe")
        m = outer.modifiers.new("solid", "SOLIDIFY")
        m.thickness = radius * 0.3
        self._apply(outer)
        self.parts.append(Part(outer, tex, anchor=anchor if anchor is not None else a, color=color))
        return outer

    def spine_row(self, bones, n, length, radius, out=-1.0, tex="Bone", color=None, lean=0.4):
        """A ridge of spikes down the back, from the first bone to the last."""
        pts = [self.J(b) for b in bones]
        for k in range(n):
            t = k / max(1, n - 1)
            f = t * (len(pts) - 1)
            i = min(int(f), len(pts) - 2)
            p = pts[i].lerp(pts[i + 1], f - i)
            s, nn = self.surf(p, FWD * out + UP * 0.15)
            ln = length * (0.6 + 0.4 * math.sin(t * math.pi)) * self.rng.uniform(0.85, 1.1)
            self.spike(s - nn * radius * 0.4, nn + UP * lean, ln, radius, bend=UP, bend_amt=0.25, tex=tex, color=color)

    # ---- mesh utilities
    def _apply(self, ob):
        dg = bpy.context.evaluated_depsgraph_get()
        me = bpy.data.meshes.new_from_object(ob.evaluated_get(dg))
        ob.modifiers.clear()
        ob.data = me

    def _detail(self, ob, strength, subdiv=1):
        if subdiv:
            m = ob.modifiers.new("sub", "SUBSURF")
            m.levels = subdiv
            m.render_levels = subdiv
        tex = bpy.data.textures.get("strain_clouds") or bpy.data.textures.new("strain_clouds", "CLOUDS")
        tex.noise_scale = 0.08
        tex.noise_depth = 2
        m = ob.modifiers.new("disp", "DISPLACE")
        m.texture = tex
        m.texture_coords = "GLOBAL"
        m.strength = strength
        m.mid_level = 0.5
        self._apply(ob)


# ------------------------------------------------------------------------------------------------
# Baking, weights and export

def setup_cycles():
    sc = bpy.context.scene
    sc.render.engine = "CYCLES"
    sc.cycles.device = "CPU"
    sc.cycles.samples = 24
    sc.render.bake.target = "VERTEX_COLORS"
    if sc.world is None:
        sc.world = bpy.data.worlds.new("w")
    sc.world.color = (1, 1, 1)
    sc.world.light_settings.distance = 0.35


def bake_ao(b, body_ob):
    setup_cycles()
    mat = bpy.data.materials.get("bake") or bpy.data.materials.new("bake")
    objs = [p.obj for p in b.parts] + [body_ob]
    for ob in objs:
        if not ob.data.materials:
            ob.data.materials.append(mat)
        ca = ob.data.color_attributes.new("AO", "FLOAT_COLOR", "POINT")
        ob.data.color_attributes.active_color = ca
    bpy.ops.object.select_all(action="DESELECT")
    for ob in objs:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.bake(type="AO", target="VERTEX_COLORS")


def read_ao(ob):
    ca = ob.data.color_attributes.get("AO")
    if ca is None:
        return np.ones(len(ob.data.vertices))
    a = np.zeros(len(ca.data) * 4)
    ca.data.foreach_get("color", a)
    return a.reshape(-1, 4)[:, 0]


def make_body_object(b):
    verts, tris = b.body_arrays()
    me = bpy.data.meshes.new("body")
    me.from_pydata([tuple(v) for v in verts], [], [tuple(int(i) for i in t) for t in tris])
    ob = bpy.data.objects.new("body", me)
    bpy.context.scene.collection.objects.link(ob)
    return ob


def transfer_weights(b, positions, anchor=None):
    """Skin weights for new vertices from the nearest body vertices."""
    keep = [~q.get("drop", np.zeros(len(q["pos"]), dtype=bool)) for q in b.prims]
    allpos = np.concatenate([q["pos"][k] for q, k in zip(b.prims, keep)])
    allj = np.concatenate([q["j"][k] for q, k in zip(b.prims, keep)])
    allw = np.concatenate([q["w"][k] for q, k in zip(b.prims, keep)])
    kd = kdtree.KDTree(len(allpos))
    for i, p in enumerate(allpos):
        kd.insert(Vector(p), i)
    kd.balance()

    def at(p):
        acc = {}
        for (_, i, d) in kd.find_n(Vector(p), 8):
            k = 1.0 / (d * d + 1e-4)
            for jj, ww in zip(allj[i], allw[i]):
                if ww > 0:
                    acc[int(jj)] = acc.get(int(jj), 0.0) + ww * k
        top = sorted(acc.items(), key=lambda x: -x[1])[:4]
        s = sum(w for _, w in top) or 1.0
        j = [t[0] for t in top] + [0] * (4 - len(top))
        w = [t[1] / s for t in top] + [0.0] * (4 - len(top))
        return j, w

    if anchor is not None:
        j, w = at(anchor)
        return np.tile(j, (len(positions), 1)), np.tile(w, (len(positions), 1))
    js, ws = zip(*(at(p) for p in positions))
    return np.array(js), np.array(ws)


def box_uv(pos, nrm, scale=2.0):
    """Box-projected UVs, `scale` texture repeats per metre."""
    ax = np.abs(nrm).argmax(axis=1)
    return np.where(ax[:, None] == 0, pos[:, [2, 1]], np.where(ax[:, None] == 1, pos[:, [0, 2]], pos[:, [0, 1]])) * scale


def part_arrays(b, part, ao):
    ob = part.obj
    me = ob.data
    if ob.get("flat"):
        m = ob.modifiers.new("split", "EDGE_SPLIT")
        m.split_angle = math.radians(30)
        b._apply(ob)
        me = ob.data
        ao = np.ones(len(me.vertices))
    me.calc_loop_triangles()
    n = len(me.vertices)
    pos = np.zeros(n * 3)
    me.vertices.foreach_get("co", pos)
    pos = pos.reshape(-1, 3)
    nrm = np.zeros(n * 3)
    me.vertices.foreach_get("normal", nrm)
    nrm = nrm.reshape(-1, 3)
    tri = np.zeros(len(me.loop_triangles) * 3, dtype=np.int64)
    me.loop_triangles.foreach_get("vertices", tri)
    uv = box_uv(pos, nrm)
    # colours: palette x variation x veins x ambient occlusion; alpha < 1 = emission
    col = np.ones((n, 4))
    base = np.array(part.color, dtype=np.float64)
    for i, p in enumerate(pos):
        v = Vector(p)
        c = base * (0.85 + 0.3 * fbm(v * 3.0 + Vector((7, 3, 1))))
        if part.veins > 0:
            c = c * (1.0 - part.veins * vein(v, 9.0) * np.array([0.5, 0.75, 0.6]))
        if part.ao:
            c = c * (0.3 + 0.7 * min(1.0, ao[i] ** 1.3))
        col[i, :3] = c
        col[i, 3] = 1.0 - part.glow
    if part.mode == "surface":
        j, w = transfer_weights(b, pos)
    else:
        j, w = transfer_weights(b, pos, anchor=part.anchor)
    return pos, nrm, uv, col, j, w, tri


def seam_normals(pos, tri, old):
    """Smooth vertex normals of a deformed mesh, welded by position."""
    key = np.round(pos * 1e5).astype(np.int64)
    _, inv = np.unique(key, axis=0, return_inverse=True)
    inv = inv.ravel()
    fn = np.cross(pos[tri[:, 1]] - pos[tri[:, 0]], pos[tri[:, 2]] - pos[tri[:, 0]])
    acc = np.zeros((inv.max() + 1, 3))
    for k in range(3):
        np.add.at(acc, inv[tri[:, k]], fn)
    n = acc[inv]
    ln = np.linalg.norm(n, axis=1, keepdims=True)
    # vertices no triangle uses keep their old normal
    return np.where(ln > 1e-12, n / np.maximum(ln, 1e-12), old)


def write_model(b, out_path):
    body = make_body_object(b)
    try:
        bake_ao(b, body)
        body_ao = read_ao(body)
    except Exception as ex:  # baking needs Cycles; fall back to flat shading
        print("  AO bake failed:", ex)
        body_ao = np.ones(sum(len(q["pos"]) for q in b.prims))
    g = b.g
    # body: deformed positions and painted colours (with contact shadows from the new parts)
    off = 0
    for q in b.prims:
        n = len(q["pos"])
        ao = body_ao[off:off + n]
        off += n
        col = q["col"].copy()
        col[:, :3] *= (0.55 + 0.45 * np.clip(ao, 0, 1) ** 1.5)[:, None]
        a = q["prim"]["attributes"]
        a["POSITION"] = g.add(q["pos"].astype(np.float32), "pos", 5126, target=34962, minmax=True)
        # the body was reshaped (inflated, torn, twisted): its normals follow the new surface,
        # averaged across UV-seam duplicates so no creases appear
        a["NORMAL"] = g.add(seam_normals(q["pos"], q["idx"].reshape(-1, 3), q["nrm"]).astype(np.float32), "nrm", 5126, target=34962)
        a["COLOR_0"] = g.add(np.round(np.clip(col, 0, 1) * 255), "color", 5121, target=34962)
        tri = q["idx"].reshape(-1, 3)
        if "drop" in q:
            tri = tri[~q["drop"][tri].any(axis=1)]
            q["prim"]["indices"] = g.add(tri.ravel().astype(np.uint32), "idx", 5125, target=34963)
        sk = q["skin"][tri].all(axis=1)
        if sk.any():
            # `C:` skins are per-character textures with their own UVs; `T:` layers tile
            name = b.skin_tex if b.skin_tex.startswith("C:") else "T:" + b.skin_tex
            mats = g.j.setdefault("materials", [])
            mi = next((i for i, m in enumerate(mats) if m.get("name") == name), None)
            if mi is None:
                mats.append({"name": name, "pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 0.8}})
                mi = len(mats) - 1
            sa = dict(a)
            if not name.startswith("C:"):
                sa["TEXCOORD_0"] = g.add(box_uv(q["pos"], q["nrm"]).astype(np.float32), "uv", 5126, target=34962)
            q["prim"]["indices"] = g.add(tri[~sk].ravel().astype(np.uint32), "idx", 5125, target=34963)
            if (~sk).sum() == 0:
                # nothing left on the original material: drop it rather than ship its vertices
                g.j["meshes"][q["mesh"]]["primitives"].remove(q["prim"])
            g.j["meshes"][q["mesh"]]["primitives"].append({"attributes": sa, "indices": g.add(tri[sk].ravel().astype(np.uint32), "idx", 5125, target=34963), "material": mi})
    # new parts, grouped per texture into primitives of the main body mesh
    groups = {}
    for p in b.parts:
        ao = read_ao(p.obj) if p.ao else np.ones(len(p.obj.data.vertices))
        groups.setdefault(p.tex, []).append(part_arrays(b, p, ao))
    mesh = g.j["meshes"][b.main["mesh"]]
    mats = g.j.setdefault("materials", [])
    total = 0
    for tex, arrs in groups.items():
        pos, nrm, uv, col, jn, wt, tri = [], [], [], [], [], [], []
        base = 0
        for (P, N, U, C, Jn, W, T) in arrs:
            pos.append(P)
            nrm.append(N)
            uv.append(U)
            col.append(C)
            jn.append(Jn)
            wt.append(W)
            tri.append(T + base)
            base += len(P)
        total += base
        name = "T:" + tex
        mi = next((i for i, m in enumerate(mats) if m.get("name") == name), None)
        if mi is None:
            mats.append({"name": name, "pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 0.8}})
            mi = len(mats) - 1
        attrs = {
            "POSITION": g.add(np.concatenate(pos).astype(np.float32), "pos", 5126, target=34962, minmax=True),
            "NORMAL": g.add(np.concatenate(nrm).astype(np.float32), "nrm", 5126, target=34962),
            "TEXCOORD_0": g.add(np.concatenate(uv).astype(np.float32), "uv", 5126, target=34962),
            "COLOR_0": g.add(np.round(np.clip(np.concatenate(col), 0, 1) * 255), "color", 5121, target=34962),
            "JOINTS_0": g.add(np.concatenate(jn).astype(np.uint16), "joints", 5123, target=34962),
            "WEIGHTS_0": g.add(np.concatenate(wt).astype(np.float32), "weights", 5126, target=34962),
        }
        idx = g.add(np.concatenate(tri).astype(np.uint32), "idx", 5125, target=34963)
        mesh["primitives"].append({"attributes": attrs, "indices": idx, "material": mi})
    g.save(out_path)
    return total


def write_static(parts, out_path):
    """A plain (unskinned) GLB from parts built without a base model: gibs and props."""
    setup_cycles()
    mat = bpy.data.materials.get("bake") or bpy.data.materials.new("bake")
    objs = [p.obj for p in parts]
    for ob in objs:
        if not ob.data.materials:
            ob.data.materials.append(mat)
        ca = ob.data.color_attributes.new("AO", "FLOAT_COLOR", "POINT")
        ob.data.color_attributes.active_color = ca
    bpy.ops.object.select_all(action="DESELECT")
    for ob in objs:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    try:
        bpy.ops.object.bake(type="AO", target="VERTEX_COLORS")
    except Exception as ex:
        print("  AO bake failed:", ex)
    g = Glb.__new__(Glb)
    g.j = {"asset": {"version": "2.0", "generator": "strain build_enemies.py"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"name": "gib", "mesh": 0}], "meshes": [{"name": "gib", "primitives": []}], "materials": [], "accessors": [], "bufferViews": [], "buffers": [{"byteLength": 0}]}
    g.bin = bytearray()
    groups = {}
    for p in parts:
        ob = p.obj
        if ob.get("flat"):
            m = ob.modifiers.new("split", "EDGE_SPLIT")
            m.split_angle = math.radians(30)
            Builder._apply(None, ob)
        me = ob.data
        me.calc_loop_triangles()
        n = len(me.vertices)
        pos = np.zeros(n * 3)
        me.vertices.foreach_get("co", pos)
        pos = pos.reshape(-1, 3)
        nrm = np.zeros(n * 3)
        me.vertices.foreach_get("normal", nrm)
        nrm = nrm.reshape(-1, 3)
        tri = np.zeros(len(me.loop_triangles) * 3, dtype=np.int64)
        me.loop_triangles.foreach_get("vertices", tri)
        ao = read_ao(ob) if p.ao else np.ones(n)
        if len(ao) != n:
            ao = np.ones(n)
        col = np.ones((n, 4))
        base = np.array(p.color, dtype=np.float64)
        for i, q in enumerate(pos):
            v = Vector(q)
            c = base * (0.85 + 0.3 * fbm(v * 9.0 + Vector((7, 3, 1))))
            if p.veins > 0:
                c = c * (1.0 - p.veins * vein(v, 25.0) * np.array([0.5, 0.75, 0.6]))
            col[i, :3] = c * (0.3 + 0.7 * min(1.0, ao[i] ** 1.3))
            col[i, 3] = 1.0 - p.glow
        groups.setdefault(p.tex, []).append((pos, nrm, box_uv(pos, nrm, 6.0), col, tri))
    for tex, arrs in groups.items():
        base, P, N, U, C, T = 0, [], [], [], [], []
        for (pos, nrm, uv, col, tri) in arrs:
            P.append(pos)
            N.append(nrm)
            U.append(uv)
            C.append(col)
            T.append(tri + base)
            base += len(pos)
        g.j["materials"].append({"name": "T:" + tex, "pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 0.6}})
        attrs = {
            "POSITION": g.add(np.concatenate(P).astype(np.float32), "pos", 5126, target=34962, minmax=True),
            "NORMAL": g.add(np.concatenate(N).astype(np.float32), "nrm", 5126, target=34962),
            "TEXCOORD_0": g.add(np.concatenate(U).astype(np.float32), "uv", 5126, target=34962),
            "COLOR_0": g.add(np.round(np.clip(np.concatenate(C), 0, 1) * 255), "color", 5121, target=34962),
        }
        g.j["meshes"][0]["primitives"].append({"attributes": attrs, "indices": g.add(np.concatenate(T).astype(np.uint32), "idx", 5125, target=34963), "material": len(g.j["materials"]) - 1})
    g.save(out_path)
    return sum(len(a[0]) for arrs in groups.values() for a in arrs)


def clear_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)


# ------------------------------------------------------------------------------------------------
# Preview renders (Cycles) for checking the models without the game

def preview(b, path):
    sc = bpy.context.scene
    setup_cycles()
    sc.cycles.samples = 16
    sc.render.resolution_x, sc.render.resolution_y = 520, 640
    sc.render.film_transparent = False
    sc.world.color = (0.5, 0.5, 0.55)
    atlas = bpy.data.images.load(os.path.join(MODELS, "atlas.png"))

    def mat_with(img, name):
        m = bpy.data.materials.new(name)
        m.use_nodes = True
        nt = m.node_tree
        bsdf = nt.nodes["Principled BSDF"]
        vc = nt.nodes.new("ShaderNodeVertexColor")
        vc.layer_name = "Col"
        mul = nt.nodes.new("ShaderNodeMix")
        mul.data_type = "RGBA"
        mul.blend_type = "MULTIPLY"
        mul.inputs[0].default_value = 1.0
        if img is not None:
            t = nt.nodes.new("ShaderNodeTexImage")
            t.image = img
            t.interpolation = "Closest"
            nt.links.new(t.outputs[0], mul.inputs[6])
        nt.links.new(vc.outputs[0], mul.inputs[7])
        nt.links.new(mul.outputs[2], bsdf.inputs["Base Color"])
        nt.links.new(mul.outputs[2], bsdf.inputs["Emission Color"])
        nt.links.new(vc.outputs[1], bsdf.inputs["Emission Strength"])
        return m

    g = Glb(path)
    for mi, p in g.skinned_prims():
        a = p["attributes"]
        P = g.acc(a["POSITION"])
        T = g.acc(p["indices"]).reshape(-1, 3)
        U = g.acc(a["TEXCOORD_0"])
        C = g.acc(a["COLOR_0"]) if "COLOR_0" in a else np.ones((len(P), 4))
        me = bpy.data.meshes.new("m")
        me.from_pydata([(v[0], -v[2], v[1]) for v in P], [], [tuple(int(i) for i in t) for t in T])
        uvl = me.uv_layers.new(name="uv")
        loops = np.zeros(len(me.loops), dtype=np.int64)
        me.loops.foreach_get("vertex_index", loops)
        uvs = U[loops].copy()
        uvs[:, 1] = 1 - uvs[:, 1]
        uvl.data.foreach_set("uv", uvs.ravel())
        ca = me.color_attributes.new("Col", "FLOAT_COLOR", "POINT")
        mname = g.j["materials"][p["material"]]["name"]
        c = C.copy()
        c[:, :3] *= np.array(PREVIEW_TINT.get(mname[2:], (1, 1, 1)))
        c[:, 3] = (1 - c[:, 3]) * 6.0
        ca.data.foreach_set("color", c.ravel())
        me.shade_smooth()
        ob = bpy.data.objects.new("m", me)
        sc.collection.objects.link(ob)
        mname = g.j["materials"][p["material"]]["name"]
        if mname == "Atlas":
            ob.data.materials.append(mat_with(atlas, "atlas"))
        else:
            tname = mname[2:].lower()
            fn = {"flesh": "leather", "vein": "leather", "growth": "moss_rock", "growthglow": None, "char": "rock", "ember": "lava", "lattice": None, "riftglow": None, "riftstone": "rock", "skin": None, "cloth": "fabric"}.get(tname, tname)
            img = None
            if fn and os.path.exists(os.path.join(ROOT, "assets", "textures", fn + ".png")):
                img = bpy.data.images.load(os.path.join(ROOT, "assets", "textures", fn + ".png"))
            ob.data.materials.append(mat_with(img, mname))
    h = max(g.acc(p["attributes"]["POSITION"])[:, 1].max() for _, p in g.skinned_prims())
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    sc.collection.objects.link(cam)
    sc.camera = cam
    light = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
    light.data.energy = 3.5
    light.rotation_euler = (0.9, 0.2, 0.6)
    sc.collection.objects.link(light)
    back = bpy.data.objects.new("back", bpy.data.lights.new("back", "SUN"))
    back.data.energy = 2.0
    back.rotation_euler = (-0.9, 0.2, 0.4)
    sc.collection.objects.link(back)
    out = []
    for k, ang in enumerate((0.55, math.pi + 0.6)):
        dist = 1.7 * h + 1.0
        cam.location = (math.sin(ang) * dist, -math.cos(ang) * dist, h * 0.55)
        cam.rotation_euler = (math.radians(88), 0, ang)
        cam.data.lens = 50
        f = path.replace(".glb", f"_{k}.png").replace(MODELS, PREVIEW_DIR)
        sc.render.filepath = f
        bpy.ops.render.render(write_still=True)
        out.append(f)
    return out


PREVIEW_DIR = None
# approximate colours the game's texture generator gives each layer (preview only)
PREVIEW_TINT = {"Cloth": (0.8, 0.8, 0.8), "Skin": (0.85, 0.66, 0.55), "Flesh": (1.3, 0.5, 0.5), "Vein": (0.5, 0.22, 0.3), "Bone": (1.0, 0.95, 0.82), "Chitin": (0.55, 0.45, 0.75), "Growth": (0.6, 0.35, 0.55), "GrowthGlow": (0.25, 0.2, 0.3), "Char": (0.35, 0.35, 0.35), "RiftStone": (0.6, 0.5, 0.9)}

if __name__ == "__main__":
    from recipes import GIBS, RECIPES

    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if "--preview" in sys.argv:
        PREVIEW_DIR = sys.argv[sys.argv.index("--preview") + 1]
        args = [a for a in args if a != PREVIEW_DIR]
        os.makedirs(PREVIEW_DIR, exist_ok=True)
    names = args or list(RECIPES) + list(GIBS)
    for name in [n for n in names if n in GIBS]:
        clear_scene()
        b = Builder.__new__(Builder)
        b.name, b.parts, b.rng = name, [], random.Random(GIBS[name][0])
        GIBS[name][1](b)
        out = os.path.join(MODELS, name + ".glb")
        print(f"{name}: {len(b.parts)} parts, {write_static(b.parts, out)} vertices -> {os.path.relpath(out, ROOT)}")
    for name in [n for n in names if n in RECIPES]:
        base, seed, fn = RECIPES[name]
        clear_scene()
        b = Builder(name, base, seed)
        fn(b)
        out = os.path.join(MODELS, name + ".glb")
        tris = write_model(b, out)
        print(f"{name}: {len(b.parts)} parts, {tris} new vertices -> {os.path.relpath(out, ROOT)}")
        if PREVIEW_DIR:
            clear_scene()
            print("  preview", preview(b, out))
