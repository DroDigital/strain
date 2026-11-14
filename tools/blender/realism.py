"""Realism pass for the skinned characters (assets/models/Characters_*.glb and Strain_*.glb).

Run with a Python that can `import bpy` (Blender's Python, or `pip install bpy`):

    python -I tools/blender/realism.py [names...]

The Quaternius kit characters are stylised: big heads and hands, flat colour swatches from one
shared atlas, low-poly faceting. This pass, in Blender:

* re-proportions them toward adult anatomy: heads, hands and feet shrink about their own joints
  (the skeleton is untouched, so every clip still fits);
* replaces the flat atlas swatches with the game's photo material layers (skin, fabric, denim,
  leather, hair), carrying the original colour in the vertex colours, with grime that gathers
  toward the feet;
* subdivides once (Catmull-Clark) with creases on hard edges, so silhouettes are smooth while
  spikes, plates and teeth stay sharp, and interpolates the skin weights onto the new vertices;
* bakes ambient occlusion with Cycles into the vertex colours of the converted surfaces.

Like the enemy builder, it splices new vertex data into the original GLB, so the skeleton,
inverse bind matrices and animation clips stay byte-for-byte identical. A marker in
`asset.extras` makes it idempotent.
"""
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import bpy  # noqa: E402
import bmesh  # noqa: E402
import numpy as np  # noqa: E402
from mathutils import Vector, noise  # noqa: E402

from glb import Glb  # noqa: E402

ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
MODELS = os.path.join(ROOT, "assets", "models")
TEXTURES = os.path.join(ROOT, "assets", "textures")

HUMANS = ["Characters_Matt_SingleWeapon"]
INFECTED = ["Strain_Carrier", "Strain_Shambler", "Strain_Bulk", "Strain_Skitter", "Strain_Retch", "Strain_Larva", "Strain_Bulwark", "Strain_Lurker",
            "Strain_Matron", "Strain_Digestor", "Strain_Geometer", "Strain_MotherCinder", "Strain_Cantor", "Strain_Seed"]
# the humans and infected are now built on realistic bodies (build_humans.py, build_infected.py);
# nothing in the game still comes through this pass, which remains for kit models
TARGETS = []

# head, hands, feet scale about their joints; arms and legs slim toward their bone axes
PROPORTIONS = {"human": (0.72, 0.72, 0.86), "infected": (0.82, 0.86, 0.9), "none": (1.0, 1.0, 1.0)}
SLIM = {"human": (0.8, 0.88), "infected": (0.92, 0.95), "none": (1.0, 1.0)}
# texture repeats per metre for each converted surface
UV_SCALE = {"Skin": 2.5, "Cloth": 8.0, "Denim": 6.5, "Leather": 4.0, "Hair": 9.0}
CREASE_ANGLE = math.radians(50)
SMOOTH_ANGLE = math.radians(55)


def srgb_to_lin(c):
    c = np.asarray(c, dtype=np.float64)
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def load_rgb(name):
    from PIL import Image

    return np.asarray(Image.open(os.path.join(TEXTURES, name)).convert("RGBA")).astype(np.float64) / 255.0


def layer_means():
    """Linear mean albedo of each texture layer, mirroring src/render/textures.rs."""
    leather = load_rgb("leather.png")
    lum = leather[..., 0] * 0.3 + leather[..., 1] * 0.55 + leather[..., 2] * 0.15
    ao = load_rgb("leather_n.png")[..., 3]
    k = 0.75 + 0.4 * (0.45 + (lum - 0.45) * 0.4)
    skin = np.stack([0.85 * k, 0.66 * k, 0.55 * k], -1) * (0.35 + 0.65 * ao)[..., None]
    fab = load_rgb("fabric.png")
    flum = np.clip((fab[..., 0] * 0.3 + fab[..., 1] * 0.55 + fab[..., 2] * 0.15) * 1.35, 0, 1) * (0.35 + 0.65 * load_rgb("fabric_n.png")[..., 3])
    detail = 0.8  # the neutral detail layers are normalised to this sRGB grey
    return {
        "Skin": srgb_to_lin(np.clip(skin, 0, 1)).reshape(-1, 3).mean(0),
        "Cloth": np.full(3, srgb_to_lin(flum).mean()),
        "Denim": np.full(3, float(srgb_to_lin(detail))),
        "Leather": np.full(3, float(srgb_to_lin(detail))),
        "Hair": np.full(3, float(srgb_to_lin(detail * 0.8))),
    }


def sample_atlas(uv):
    from PIL import Image

    at = np.asarray(Image.open(os.path.join(MODELS, "atlas.png")).convert("RGB")).astype(np.float64) / 255.0
    h, w = at.shape[:2]
    x = np.clip(((uv[:, 0] % 1.0) * w).astype(int), 0, w - 1)
    y = np.clip(((uv[:, 1] % 1.0) * h).astype(int), 0, h - 1)
    return at[y, x]


def classify(c, skin_ref, head_w, dog):
    """Which photo layer a flat atlas swatch becomes."""
    lum = c[0] * 0.3 + c[1] * 0.55 + c[2] * 0.15
    sat = c.max() - c.min()
    if dog:
        return "Leather" if lum < 0.12 else "Hair"
    if skin_ref is not None and np.linalg.norm(c - skin_ref) < 0.1:
        return "Skin"
    if head_w > 0.6:
        if lum > 0.55 and sat < 0.18:
            return "Bone"  # teeth, eyeballs
        if c[0] > c[1] + 0.15 and c[0] > c[2] + 0.1:
            return "Flesh"  # lips, gums, the inside of a mouth
        return "Hair"
    if c[2] > c[0] + 0.08 and c[2] > c[1] - 0.02:
        return "Denim"
    if lum < 0.13 or (lum < 0.3 and c[0] > c[2] + 0.08 and sat < 0.3):
        return "Leather"
    return "Cloth"


def subtree(names, parents, root):
    out = {root}
    changed = True
    while changed:
        changed = False
        for j, p in enumerate(parents):
            if p in out and j not in out:
                out.add(j)
                changed = True
    return out


class Model:
    def __init__(self, name):
        self.name = name
        self.path = os.path.join(MODELS, name + ".glb")
        self.g = Glb(self.path)
        self.names = self.g.joint_names()
        nodes = self.g.j["nodes"]
        joints = self.g.skin()["joints"]
        node_to_j = {n: i for i, n in enumerate(joints)}
        self.parents = [None] * len(joints)
        for i, n in enumerate(joints):
            for c in nodes[n].get("children", []):
                if c in node_to_j:
                    self.parents[node_to_j[c]] = i
        self.bind = self.g.bind_positions()
        self.kind = "human" if name in HUMANS else ("none" if name in ("Characters_GermanShepherd", "Strain_Skitter", "Strain_Larva") else "infected")
        self.dog = name in ("Characters_GermanShepherd",)

    def j(self, name):
        return self.names.index(name) if name in self.names else None

    def weight_of(self, jn, wt, joints):
        m = np.isin(jn, list(joints))
        return (wt * m).sum(axis=1)

    def reproportion(self, pos, jn, wt):
        head_s, hand_s, foot_s = PROPORTIONS[self.kind]
        out = pos.copy()
        h = self.j("Head")
        if h is not None and head_s != 1.0:
            w = self.weight_of(jn, wt, subtree(self.names, self.parents, h))
            piv = np.array(self.bind["Head"])
            out = out + (piv + (out - piv) * head_s - out) * w[:, None]
        for side in "LR":
            fingers = [self.j(f"{f}1.{side}") for f in ("Pinky", "Middle", "Index", "Thumb")]
            fingers = [f for f in fingers if f is not None]
            elbow = self.j(f"LowerArm.{side}")
            if fingers and elbow is not None and hand_s != 1.0:
                palm = np.mean([self.bind[self.names[f]] for f in fingers], axis=0)
                el = np.array(self.bind[self.names[elbow]])
                axis = palm - el
                axis /= max(np.linalg.norm(axis), 1e-6)
                wrist = el + (palm - el) * 0.78
                hand_j = set()
                for f in fingers:
                    hand_j |= subtree(self.names, self.parents, f)
                w = self.weight_of(jn, wt, hand_j | {elbow})
                t = np.clip(((out - wrist) @ axis) / 0.03, 0.0, 1.0)
                k = (w * t)[:, None]
                out = out + (wrist + (out - wrist) * hand_s - out) * k
            foot = self.j(f"Foot.{side}")
            if foot is not None and foot_s != 1.0:
                w = self.weight_of(jn, wt, subtree(self.names, self.parents, foot))
                piv = np.array(self.bind[self.names[foot]])
                out = out + (piv + (out - piv) * foot_s - out) * w[:, None]
        # slimmer limbs: pull each limb's vertices toward its bone segment
        arm_s, leg_s = SLIM[self.kind]
        for side in "LR":
            fingers = [self.j(f"{f}1.{side}") for f in ("Middle", "Index")]
            palm = np.mean([self.bind[self.names[f]] for f in fingers if f is not None], axis=0) if any(f is not None for f in fingers) else None
            segs = [(f"UpperArm.{side}", f"LowerArm.{side}", arm_s), (f"UpperLeg.{side}", f"LowerLeg.{side}", leg_s), (f"LowerLeg.{side}", f"Foot.{side}", leg_s)]
            if palm is not None:
                segs.append((f"LowerArm.{side}", None, arm_s))
            for a_name, b_name, s in segs:
                ja = self.j(a_name)
                if ja is None or s == 1.0:
                    continue
                a = np.array(self.bind[a_name])
                b = np.array(self.bind[b_name]) if b_name and self.j(b_name) is not None else palm
                if b is None:
                    continue
                ab = b - a
                t = np.clip(((out - a) @ ab) / max(ab @ ab, 1e-9), 0.0, 1.0)
                closest = a + t[:, None] * ab
                w = self.weight_of(jn, wt, {ja})
                out = out + (closest + (out - closest) * s - out) * w[:, None]
        return out


def build_object(name, pos, tris, corner_uv, corner_col, face_mat, face_conv, jn, wt):
    me = bpy.data.meshes.new(name)
    me.from_pydata([tuple(p) for p in pos], [], [tuple(int(i) for i in t) for t in tris])
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    uvl = me.uv_layers.new(name="uv")
    uvl.data.foreach_set("uv", corner_uv.astype(np.float32).ravel())
    ca = me.color_attributes.new("col", "FLOAT_COLOR", "CORNER")
    ca.data.foreach_set("color", corner_col.astype(np.float32).ravel())
    fa = me.attributes.new("mat", "INT", "FACE")
    fa.data.foreach_set("value", face_mat.astype(np.int32))
    fc = me.attributes.new("conv", "INT", "FACE")
    fc.data.foreach_set("value", face_conv.astype(np.int32))
    groups = {}
    for v in range(len(pos)):
        for k in range(4):
            if wt[v, k] > 1e-4:
                jj = int(jn[v, k])
                if jj not in groups:
                    groups[jj] = ob.vertex_groups.new(name=f"j{jj}")
                groups[jj].add([v], float(wt[v, k]), "ADD")
    # weld the seams so subdivision does not open cracks; crease hard edges
    bm = bmesh.new()
    bm.from_mesh(me)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    cl = bm.edges.layers.float.get("crease_edge") or bm.edges.layers.float.new("crease_edge")
    for e in bm.edges:
        if len(e.link_faces) == 2 and e.calc_face_angle(0.0) > CREASE_ANGLE:
            e[cl] = 1.0
        elif len(e.link_faces) != 2:
            e[cl] = 1.0
    bm.to_mesh(me)
    bm.free()
    return ob


def subdivide(ob):
    m = ob.modifiers.new("sub", "SUBSURF")
    m.levels = 1
    m.render_levels = 1
    m.uv_smooth = "PRESERVE_BOUNDARIES"
    m.use_creases = True
    dg = bpy.context.evaluated_depsgraph_get()
    me = bpy.data.meshes.new_from_object(ob.evaluated_get(dg), preserve_all_data_layers=True, depsgraph=dg)
    ob.modifiers.clear()
    ob.data = me


def bake_ao(ob):
    sc = bpy.context.scene
    sc.render.engine = "CYCLES"
    sc.cycles.device = "CPU"
    sc.cycles.samples = 32
    sc.render.bake.target = "VERTEX_COLORS"
    if sc.world is None:
        sc.world = bpy.data.worlds.new("w")
    sc.world.color = (1, 1, 1)
    sc.world.light_settings.distance = 0.3
    mat = bpy.data.materials.get("bake") or bpy.data.materials.new("bake")
    ob.data.materials.clear()
    ob.data.materials.append(mat)
    ca = ob.data.color_attributes.new("AO", "FLOAT_COLOR", "POINT")
    ob.data.color_attributes.active_color = ca
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    bpy.ops.object.bake(type="AO", target="VERTEX_COLORS")
    a = np.zeros(len(ca.data) * 4)
    ca.data.foreach_get("color", a)
    return a.reshape(-1, 4)[:, 0]


def corner_normals(me):
    """Smooth normals that stay hard across creased (sharp) edges."""
    me.calc_loop_triangles()
    nf = len(me.polygons)
    fn = np.zeros(nf * 3)
    me.polygons.foreach_get("normal", fn)
    fn = fn.reshape(-1, 3)
    loops_v = np.zeros(len(me.loops), dtype=np.int64)
    me.loops.foreach_get("vertex_index", loops_v)
    loop_f = np.zeros(len(me.loops), dtype=np.int64)
    for p in me.polygons:
        loop_f[p.loop_start:p.loop_start + p.loop_total] = p.index
    # faces around each vertex
    around = [[] for _ in range(len(me.vertices))]
    for li, v in enumerate(loops_v):
        around[v].append(loop_f[li])
    cos_lim = math.cos(SMOOTH_ANGLE)
    out = np.zeros((len(me.loops), 3))
    for li, v in enumerate(loops_v):
        f = loop_f[li]
        n0 = fn[f]
        acc = np.zeros(3)
        for g in set(around[v]):
            if fn[g] @ n0 >= cos_lim:
                acc += fn[g]
        nl = np.linalg.norm(acc)
        out[li] = acc / nl if nl > 1e-9 else n0
    return out


def box_uv(pos, nrm, scale):
    ax = np.abs(nrm).argmax(axis=1)
    uv = np.where(ax[:, None] == 0, pos[:, [2, 1]], np.where(ax[:, None] == 1, pos[:, [0, 2]], pos[:, [0, 1]]))
    return uv * scale[:, None]


def process(name, means):
    md = Model(name)
    g = md.g
    extras = g.j.setdefault("asset", {}).setdefault("extras", {})
    if extras.get("realism"):
        print(f"{name}: already processed")
        return
    mats = g.j.setdefault("materials", [])
    mat_names = [m.get("name", "") for m in mats]
    # every skinned mesh is rebuilt as one object
    meshes = {}
    for mi, p in g.skinned_prims():
        meshes.setdefault(mi, []).append(p)
    height = None
    for mi, prims in meshes.items():
        P, N, U, C, J, W, T, F, FC = [], [], [], [], [], [], [], [], []
        layers = []  # material name per face group
        base = 0
        for p in prims:
            a = p["attributes"]
            pos = g.acc(a["POSITION"]).astype(np.float64)
            uv = g.acc(a["TEXCOORD_0"]).astype(np.float64) if "TEXCOORD_0" in a else np.zeros((len(pos), 2))
            col = g.acc(a["COLOR_0"]).astype(np.float64) if "COLOR_0" in a else np.ones((len(pos), 4))
            if col.shape[1] == 3:
                col = np.concatenate([col, np.ones((len(col), 1))], 1)
            jn = g.acc(a["JOINTS_0"]).astype(np.int64)
            wt = g.acc(a["WEIGHTS_0"]).astype(np.float64)
            idx = g.acc(p["indices"]).astype(np.int64) if "indices" in p else np.arange(len(pos))
            tri = idx.reshape(-1, 3)
            mname = mat_names[p["material"]] if "material" in p and p["material"] < len(mat_names) else ""
            fmat = []
            if mname == "Atlas":
                atl = sample_atlas(uv)
                # the bare-skin colour: what the fingers are painted with
                fingers = set()
                for f in ("Middle1.L", "Middle1.R", "Index1.L", "Index1.R"):
                    if md.j(f) is not None:
                        fingers |= subtree(md.names, md.parents, md.j(f))
                fw = md.weight_of(jn, wt, fingers)
                skin_ref = None
                if (fw > 0.5).any():
                    q = np.round(atl[fw > 0.5] * 32) / 32
                    vals, counts = np.unique(q, axis=0, return_counts=True)
                    skin_ref = vals[counts.argmax()]
                head = md.j("Head")
                hw = md.weight_of(jn, wt, subtree(md.names, md.parents, head)) if head is not None else np.zeros(len(pos))
                vcls = [classify(atl[i], skin_ref, hw[i], md.dog) for i in range(len(pos))]
                # infected bodies keep the skin tone their builder painted
                for t in tri:
                    cls = vcls[t[0]]
                    fmat.append("T:" + cls)
                lin = srgb_to_lin(atl)
                newc = np.ones((len(pos), 4))
                for i in range(len(pos)):
                    # bone and flesh keep their own colour; the rest carry the swatch's hue
                    newc[i, :3] = np.clip(lin[i] / means[vcls[i]], 0, 1.6) if vcls[i] in means else 1.0
                col = newc
                col[:, 3] = 1.0
                uv = np.full((len(pos), 2), -1.0)  # recomputed after subdivision
            else:
                fmat = [mname] * len(tri)
            FC.extend([1 if mname == "Atlas" else 0] * len(tri))
            P.append(pos)
            U.append(uv)
            C.append(col)
            J.append(jn)
            W.append(wt)
            T.append(tri + base)
            F.extend(fmat)
            base += len(pos)
        pos = np.concatenate(P)
        jn = np.concatenate(J)
        wt = np.concatenate(W)
        uv = np.concatenate(U)
        col = np.concatenate(C)
        tri = np.concatenate(T)
        layers = sorted(set(F))
        face_mat = np.array([layers.index(f) for f in F])
        pos = md.reproportion(pos, jn, wt)
        lo, hi = pos[:, 1].min(), pos[:, 1].max()
        height = (lo, hi)
        # per-corner attributes
        corner_uv = uv[tri.ravel()]
        corner_col = col[tri.ravel()]
        ob = build_object(name, pos, tri, corner_uv, corner_col, face_mat, np.array(FC), jn, wt)
        subdivide(ob)
        me = ob.data
        try:
            ao = bake_ao(ob) if name in HUMANS + ["Characters_GermanShepherd"] else np.ones(len(me.vertices))
        except Exception as ex:  # noqa: BLE001
            print("  AO bake failed:", ex)
            ao = np.ones(len(me.vertices))
        # gather the subdivided data
        nv = len(me.vertices)
        vpos = np.zeros(nv * 3)
        me.vertices.foreach_get("co", vpos)
        vpos = vpos.reshape(-1, 3)
        gnames = {vg.index: int(vg.name[1:]) for vg in ob.vertex_groups}
        vj = np.zeros((nv, 4), dtype=np.int64)
        vw = np.zeros((nv, 4))
        for v in me.vertices:
            gs = sorted(((gnames[x.group], x.weight) for x in v.groups if x.weight > 1e-4), key=lambda x: -x[1])[:4]
            s = sum(w for _, w in gs) or 1.0
            for k, (jj, ww) in enumerate(gs):
                vj[v.index, k] = jj
                vw[v.index, k] = ww / s
        cn = corner_normals(me)
        loops_v = np.zeros(len(me.loops), dtype=np.int64)
        me.loops.foreach_get("vertex_index", loops_v)
        cuv = np.zeros(len(me.loops) * 2)
        me.uv_layers["uv"].data.foreach_get("uv", cuv)
        cuv = cuv.reshape(-1, 2)
        ccol = np.zeros(len(me.loops) * 4)
        me.color_attributes["col"].data.foreach_get("color", ccol)
        ccol = ccol.reshape(-1, 4)
        fmat_s = np.zeros(len(me.polygons), dtype=np.int64)
        me.attributes["mat"].data.foreach_get("value", fmat_s)
        me.calc_loop_triangles()
        lt = np.zeros(len(me.loop_triangles) * 3, dtype=np.int64)
        me.loop_triangles.foreach_get("loops", lt)
        lt_poly = np.zeros(len(me.loop_triangles), dtype=np.int64)
        me.loop_triangles.foreach_get("polygon_index", lt_poly)
        # converted surfaces: box UVs at their layer's scale, grime toward the feet, AO
        lp = vpos[loops_v]
        span = max(hi - lo, 1e-3)
        fconv = np.zeros(len(me.polygons), dtype=np.int64)
        me.attributes["conv"].data.foreach_get("value", fconv)
        loop_mat = np.zeros(len(me.loops), dtype=np.int64)
        conv_loop = np.zeros(len(me.loops), dtype=bool)
        for p in me.polygons:
            loop_mat[p.loop_start:p.loop_start + p.loop_total] = fmat_s[p.index]
            conv_loop[p.loop_start:p.loop_start + p.loop_total] = fconv[p.index] == 1
        scale = np.array([UV_SCALE.get(layers[m][2:], 3.0) for m in loop_mat])
        buv = box_uv(lp, cn, scale)
        cuv = np.where(conv_loop[:, None], buv, cuv)
        hgt = (lp[:, 1] - lo) / span
        grime = np.array([0.82 + 0.3 * noise.fractal(Vector(p) * 6.0, 0.5, 2.0, 3) for p in lp])
        dirt = 1.0 - 0.28 * np.clip((0.35 - hgt) / 0.35, 0, 1)
        aof = 0.35 + 0.65 * np.clip(ao[loops_v], 0, 1) ** 1.4
        shade = np.clip(grime * dirt * aof, 0.2, 1.2)
        ccol[:, :3] = np.where(conv_loop[:, None], ccol[:, :3] * shade[:, None], ccol[:, :3])
        # to GLB convention: linear colour that the loader gamma-encodes (see model.rs)
        out_col = np.where(conv_loop[:, None], np.concatenate([np.clip(ccol[:, :3], 0, 1) ** 2.2, ccol[:, 3:]], 1), ccol)
        # one output vertex per distinct (vertex, material, uv, colour, normal)
        key = np.concatenate([loops_v[:, None], loop_mat[:, None], np.round(cuv * 2048), np.round(out_col * 255), np.round(cn * 512)], 1)
        uniq, inv = np.unique(key, axis=0, return_inverse=True)
        inv = inv.ravel()
        first = np.zeros(len(uniq), dtype=np.int64)
        first[inv[::-1]] = np.arange(len(inv))[::-1]
        o_pos = vpos[loops_v[first]].astype(np.float32)
        o_nrm = cn[first].astype(np.float32)
        o_uv = cuv[first].astype(np.float32)
        o_col = np.round(np.clip(out_col[first], 0, 1) * 255)
        o_j = vj[loops_v[first]].astype(np.uint16)
        o_w = vw[loops_v[first]].astype(np.float32)
        o_mat = loop_mat[first]
        tri_v = inv[lt].reshape(-1, 3)
        tri_m = o_mat[tri_v[:, 0]]
        new_prims = []
        for li, lname in enumerate(layers):
            sel = tri_v[tri_m == li]
            if len(sel) == 0:
                continue
            # each primitive carries only its own vertices (the game loads a primitive's whole
            # vertex set, so shared accessors would be duplicated per material)
            used, local = np.unique(sel.ravel(), return_inverse=True)
            attrs = {
                "POSITION": g.add(o_pos[used], "pos", 5126, target=34962, minmax=True),
                "NORMAL": g.add(o_nrm[used], "nrm", 5126, target=34962),
                "TEXCOORD_0": g.add(o_uv[used], "uv", 5126, target=34962),
                "COLOR_0": g.add(o_col[used], "color", 5121, target=34962),
                "JOINTS_0": g.add(o_j[used], "joints", 5123, target=34962),
                "WEIGHTS_0": g.add(o_w[used], "weights", 5126, target=34962),
            }
            mi_ = next((i for i, m in enumerate(mats) if m.get("name") == lname), None)
            if mi_ is None:
                mats.append({"name": lname, "pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 0.8}})
                mi_ = len(mats) - 1
            new_prims.append({"attributes": attrs, "indices": g.add(local.ravel().astype(np.uint32), "idx", 5125, target=34963), "material": mi_})
        g.j["meshes"][mi]["primitives"] = new_prims
        print(f"  mesh {mi}: {base} -> {len(o_pos)} vertices, {len(tri_v)} triangles, layers {layers}")
        bpy.data.objects.remove(ob)
    extras["realism"] = 1
    g.save(md.path)
    print(f"{name}: done")


def main():
    names = [a for a in sys.argv[1:] if not a.startswith("--")] or TARGETS
    means = layer_means()
    print("layer means (linear):", {k: np.round(v, 3).tolist() for k, v in means.items()})
    for n in names:
        bpy.ops.wm.read_factory_settings(use_empty=True)
        process(n, means)


if __name__ == "__main__":
    main()
