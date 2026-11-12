"""The infected on realistic bodies: body horror built in Blender on Universal Base Characters.

Run after tools/blender/build_humans.py, with a Python that can `import bpy`:

    python -I tools/blender/build_infected.py [names...] [--preview DIR]

Every archetype and boss starts as a realistic human (the same skeleton and retargeted zombie
clips as the living, so they move like people who are no longer people). The body is then
deformed, torn open and grown over with the enemy builder's parts (bone, chitin, tumours, maws,
tentacles, crystals), and its skin is replaced by a diseased texture generated here in UV space
from the real skin: pallid and veined with sores and bruising, flayed to the muscle, charred with
burning cracks, rotting, or drained white. Clothing is torn, holed and blood-soaked.

Writes assets/models/Strain_*.glb (kept by name, so the game's archetype tables are unchanged)
and the diseased skins under assets/characters/.
"""
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import bpy  # noqa: E402,F401
import numpy as np  # noqa: E402
from mathutils import Vector, noise  # noqa: E402

import build_enemies as be  # noqa: E402
import build_humans as bh  # noqa: E402
from build_enemies import FWD, LEFT, UP, Builder, clear_scene, write_model  # noqa: E402

ROOT = bh.ROOT
BASES = os.path.join(bh.CACHE, "bases")
CHARS = bh.CHARS
T = bh.CHAR_TEX

# kit bone name -> realistic bone name (recipes and the old helpers speak kit)
KIT = {"Root": "root", "Body": "pelvis", "Hips": "pelvis", "Abdomen": "spine_01", "Torso": "spine_03", "Neck": "neck_01", "Head": "Head"}
for s, k in (("l", "L"), ("r", "R")):
    KIT.update({f"Shoulder.{k}": f"clavicle_{s}", f"UpperArm.{k}": f"upperarm_{s}", f"LowerArm.{k}": f"lowerarm_{s}", f"Middle1.{k}": f"middle_01_{s}",
                f"Index1.{k}": f"index_01_{s}", f"Pinky1.{k}": f"pinky_01_{s}", f"Thumb1.{k}": f"thumb_01_{s}", f"UpperLeg.{k}": f"thigh_{s}",
                f"LowerLeg.{k}": f"calf_{s}", f"Foot.{k}": f"foot_{s}", f"Hand.{k}": f"hand_{s}"})


# ------------------------------------------------------------------------------------------------
# Diseased skins, generated in UV space from the real ones

def _noise_img(n, scale, seed, octaves=4):
    """Tileable-enough fBm image (n x n) from Blender's noise."""
    ys, xs = np.mgrid[0:n, 0:n] / n
    out = np.zeros((n, n))
    amp, tot, f = 1.0, 0.0, scale
    rng = np.random.default_rng(seed)
    for _ in range(octaves):
        ox, oy = rng.uniform(0, 100, 2)
        # vectorised value noise through a coarse random lattice, bilinear
        g = rng.random((int(f) + 2, int(f) + 2))
        u, v = xs * f, ys * f
        i, j = np.floor(u).astype(int), np.floor(v).astype(int)
        fu, fv = u - i, v - j
        fu, fv = fu * fu * (3 - 2 * fu), fv * fv * (3 - 2 * fv)
        a = g[j, i] * (1 - fu) + g[j, i + 1] * fu
        b = g[j + 1, i] * (1 - fu) + g[j + 1, i + 1] * fu
        out += amp * (a * (1 - fv) + b * fv)
        tot += amp
        amp *= 0.5
        f *= 2.0
        _ = (ox, oy)
    return out / tot


def _ridges(n, scale, seed, width=0.06):
    """Branching vein network: thin ridges where an fBm field crosses its mid value."""
    f = _noise_img(n, scale, seed, 5)
    return np.clip(1.0 - np.abs(f - 0.5) / width, 0, 1) ** 2


def _cells(n, count, seed, rmin, rmax):
    """Round sores: (mask, rim) images."""
    rng = np.random.default_rng(seed)
    ys, xs = np.mgrid[0:n, 0:n] / n
    m = np.zeros((n, n))
    rim = np.zeros((n, n))
    for _ in range(count):
        cx, cy, r = rng.random(), rng.random(), rng.uniform(rmin, rmax)
        d = np.sqrt((xs - cx) ** 2 + (ys - cy) ** 2) / r
        m = np.maximum(m, np.clip(1 - d, 0, 1) ** 0.6)
        rim = np.maximum(rim, np.clip(1 - np.abs(d - 1.0) * 4, 0, 1))
    return m, rim


def _height_to_normal(h, strength):
    gy, gx = np.gradient(h)
    n = np.stack([-gx * strength, gy * strength, np.ones_like(h)], -1)
    return n / np.linalg.norm(n, axis=-1, keepdims=True)


def diseased_skin(name, source, kind, seed):
    from PIL import Image

    alb = np.asarray(Image.open(os.path.join(CHARS, f"{source}.png")).convert("RGBA")).astype(np.float64) / 255
    mat = np.asarray(Image.open(os.path.join(CHARS, f"{source}_n.png")).convert("RGBA")).astype(np.float64) / 255
    c = alb[..., :3]
    lum = (c * [0.3, 0.55, 0.15]).sum(-1, keepdims=True)
    n0 = np.stack([mat[..., 0] * 2 - 1, mat[..., 1] * 2 - 1], -1)
    rough = mat[..., 2].copy()
    height = np.zeros(lum.shape[:2])
    veins = _ridges(T, 13, seed, 0.016)
    fine = _ridges(T, 31, seed + 1, 0.025) * 0.4
    blotch = _noise_img(T, 5, seed + 2)
    sores, rim = _cells(T, 70, seed + 3, 0.008, 0.03)
    if kind == "pale":
        # bloodless grey-yellow skin, dark veins standing up, bruising, weeping sores
        base = lum * np.array([0.92, 0.95, 0.82]) * 1.05 + 0.04
        bruise = np.clip((blotch - 0.55) * 4, 0, 1)[..., None]
        base = base * (1 - bruise * np.array([0.35, 0.55, 0.25]))
        v = np.clip(veins + fine, 0, 1)[..., None]
        base = base * (1 - v * 0.38) + v * np.array([0.2, 0.17, 0.3]) * 0.38
        s = sores[..., None]
        base = base * (1 - s) + s * np.array([0.32, 0.06, 0.05]) * (0.6 + 0.4 * blotch[..., None])
        base = base * (1 - rim[..., None] * 0.6) + rim[..., None] * np.array([0.55, 0.45, 0.2]) * 0.6
        height = veins * 0.6 + fine * 0.3 - sores * 0.8 + rim * 0.4
        rough = np.clip(rough * 1.05 - sores * 0.6, 0.12, 1)
    elif kind == "flayed":
        # skin torn away over most of the body: wet muscle with fibre streaks, fat, tendons
        torn = np.clip((_noise_img(T, 4, seed + 4, 3) - 0.45) * 5, 0, 1)
        ys, xs = np.mgrid[0:T, 0:T] / T
        # fibres: fine, running along a slowly turning direction, bundled into muscle bellies
        w1, w2 = _noise_img(T, 3, seed + 5, 2), _noise_img(T, 3, seed + 15, 2)
        ang = (w1 - 0.5) * 3.0
        coord = xs * np.sin(ang) + ys * np.cos(ang) + w2 * 0.05
        fib = 0.5 + 0.5 * np.sin(coord * 2 * np.pi * 180)
        fib = fib * (0.6 + 0.4 * _noise_img(T, 40, seed + 16, 2))
        grooves = _ridges(T, 6, seed + 17, 0.012)
        muscle = np.array([0.27, 0.035, 0.035]) * (0.82 + 0.3 * fib[..., None])
        muscle = muscle * (1 - grooves[..., None] * 0.6)
        tendon = np.clip((_noise_img(T, 9, seed + 18) - 0.7) * 6, 0, 1)[..., None] * (0.5 + 0.5 * fib[..., None])
        muscle = muscle * (1 - tendon) + np.array([0.7, 0.62, 0.55]) * tendon
        fat = np.clip((_noise_img(T, 14, seed + 6) - 0.66) * 5, 0, 1)[..., None]
        muscle = muscle * (1 - fat) + np.array([0.62, 0.5, 0.3]) * fat
        skin = lum * np.array([0.85, 0.78, 0.72])
        edge = np.clip(1 - np.abs(torn - 0.5) * 6, 0, 1)[..., None]
        t = torn[..., None]
        base = skin * (1 - t) + muscle * t
        base = base * (1 - edge * 0.5) + edge * np.array([0.25, 0.04, 0.03]) * 0.5
        height = torn * (fib * 0.15 - grooves * 0.6 - 0.5) + edge[..., 0] * 0.6
        rough = rough * (1 - torn) + 0.18 * torn
    elif kind == "burnt":
        # charred to black and leather, split by cracks still glowing beneath
        cr = _ridges(T, 14, seed + 7, 0.03)
        crust = np.clip(_noise_img(T, 10, seed + 8) * 1.3, 0, 1)[..., None]
        base = np.array([0.05, 0.035, 0.03]) * (0.6 + 0.8 * crust) + lum * 0.08
        k = cr[..., None]
        base = base * (1 - k) + np.array([1.0, 0.42, 0.08]) * k
        height = crust[..., 0] * 0.8 - cr * 0.9
        rough = 0.85 - cr * 0.4
    elif kind == "rot":
        # green-black decomposition, slick, marbled
        marb = _ridges(T, 6, seed + 9, 0.08)
        base = lum * np.array([0.55, 0.62, 0.42]) * (0.7 + 0.5 * blotch[..., None])
        base = base * (1 - marb[..., None] * 0.7) + marb[..., None] * np.array([0.08, 0.12, 0.06]) * 0.7
        s = sores[..., None]
        base = base * (1 - s) + s * np.array([0.18, 0.15, 0.05])
        height = -sores * 0.7 + marb * 0.3
        rough = np.clip(0.35 + blotch * 0.3 - sores * 0.2, 0.1, 1)
    else:  # "white": drained, translucent, every vessel visible
        base = lum * 0.15 + np.array([0.78, 0.76, 0.74])
        v = np.clip(veins * 1.2 + fine, 0, 1)[..., None]
        base = base * (1 - v * 0.5) + v * np.array([0.25, 0.22, 0.4]) * 0.5
        height = veins * 0.5 + fine * 0.3
        rough = rough * 0.8
    nb = _height_to_normal(height, 6.0)
    nn = np.concatenate([n0 * 0.7, np.ones(n0.shape[:2] + (1,))], -1) + np.concatenate([nb[..., :2], np.zeros(nb.shape[:2] + (1,))], -1)
    nn /= np.linalg.norm(nn, axis=-1, keepdims=True)
    out_a = np.concatenate([np.clip(base, 0, 1), np.ones(lum.shape)], -1)
    Image.fromarray((out_a * 255).astype(np.uint8), "RGBA").save(os.path.join(CHARS, f"{name}.png"), optimize=True)
    sss = 0.0 if kind == "burnt" else 1.0
    out_m = np.stack([nn[..., 0] * 0.5 + 0.5, nn[..., 1] * 0.5 + 0.5, np.clip(rough, 0.04, 1), np.full(rough.shape, sss)], -1)
    Image.fromarray((out_m * 255).astype(np.uint8), "RGBA").save(os.path.join(CHARS, f"{name}_n.png"), optimize=True)
    print("skin", name)


# ------------------------------------------------------------------------------------------------
# Ragged base bodies

def torn(gm, seed, holes=0.45):
    """Wrap a garment's selector so noise tears holes and ragged hems into it."""
    inner = gm.sel

    def sel(body):
        m = inner(body)
        rng = Vector((seed, seed * 1.7, seed * 0.3))
        keep = np.array([noise.noise(Vector(p) * 7.0 + rng) + 0.35 * noise.noise(Vector(p) * 23.0 + rng) > -holes for p in body.pos])
        return m & keep
    gm.sel = sel
    return gm


def build_base(sex, seed, garments):
    g = bh.Gltf(BODIES[sex])
    sk = bh.Skel(g)
    body = bh.Body(g, sk)
    prims = []
    covered = np.zeros(len(body.pos), dtype=bool)
    means = bh.layer_means()
    for k, gm in enumerate(garments):
        m = bh.garment_mesh(body, gm, seed + k * 2.3)
        inner = np.setdiff1d(m["used"], m["used"][bh.boundary_verts(m["tri"])])
        covered[inner] = True
        uv = bh.box_uv(m["pos"], m["nrm"], np.full(len(m["pos"]), 6.0))
        col = np.tile(bh.tint_to_vertex(m["color"], m["tex"], means), (len(m["pos"]), 1))
        # filth and old blood soaked into the cloth
        for i, p in enumerate(m["pos"]):
            v = Vector(p) * 4.0 + Vector((seed, 0, 0))
            dirt = 0.65 + 0.35 * noise.noise(v)
            blood = max(0.0, noise.noise(v * 0.8 + Vector((9, 3, 1))) - 0.15) * 2.0
            col[i, :3] = col[i, :3] * dirt * (1 - min(blood, 1) * np.array([0.3, 0.85, 0.85]))
        prims.append(("T:" + m["tex"], m["pos"], m["nrm"], uv, col, m["j"], m["w"], m["tri"]))
    keep = ~covered[body.tri].all(axis=1)
    prims.append((f"C:skin_{sex.lower()}_light", body.pos, body.nrm, body.parts["skin"]["uv"], np.ones((len(body.pos), 4)), body.j, body.w, body.tri[keep]))
    e = body.parts["eyes"]
    prims.append(("C:eyes", e["pos"], e["nrm"], e["uv"], np.ones((len(e["pos"]), 4)), e["j"], e["w"], e["tri"]))
    os.makedirs(BASES, exist_ok=True)
    path = os.path.join(BASES, f"Base_{sex}_{seed}.glb")
    _write(g, path, prims)
    return path


def _write(g, path, prims):
    saved = bh.MODELS
    bh.MODELS = os.path.dirname(path)
    try:
        bh.write_character(g, os.path.splitext(os.path.basename(path))[0], prims)
    finally:
        bh.MODELS = saved


# ------------------------------------------------------------------------------------------------
# Builder on a realistic body

class RealBuilder(Builder):
    def __init__(self, name, base_path, skin, seed=1):
        self.skin_name = skin
        super().__init__(name, base_path, seed)
        for k, v in KIT.items():
            if v in self.jpos and k not in self.jpos:
                self.jpos[k] = self.jpos[v]
                self.children[k] = self.children.get(v, [])
        self.skin_tex = "C:" + skin
        self.height = max(q["pos"][:, 1].max() for q in self.prims)

    def _sample_atlas(self, skin_test=None):
        mats = [m.get("name", "") for m in self.g.j.get("materials", [])]
        for q in self.prims:
            n = len(q["pos"])
            q["atlas"] = np.ones((n, 3))
            q["skin"] = np.full(n, mats[q["prim"].get("material", 0)].startswith("C:skin"))
            q["eye"] = mats[q["prim"].get("material", 0)] == "C:eyes"

    def bone_w(self, prim, bones):
        bones = [KIT.get(b, b) for b in bones]
        return super().bone_w(prim, bones)

    def subtree(self, name):
        name = KIT.get(name, name)
        out = [name]
        for c in self.children.get(name, []):
            out += self.subtree(c)
        return out

    def paint(self, **kw):
        kw.setdefault("mottle", 0.15)
        kw.setdefault("grime", 0.15)
        kw.setdefault("skin", (1.0, 1.0, 1.0))
        super().paint(**kw)
        self.skin_tex = "C:" + self.skin_name

    def eyes(self, color=(1.0, 0.9, 0.5), glow=0.0, milky=True):
        """Cataract-white or glowing eyes."""
        for q in self.prims:
            if q.get("eye") is not None and q["eye"]:
                q["col"][:, :3] = np.array(color) * (0.9 if milky else 1.0)
                q["col"][:, 3] = 1.0 - glow

    def remove(self, bones, thresh=0.5, where=None):
        """Delete body (and clothing) faces weighted to `bones` — limbs lost, flesh eaten away."""
        for q in self.prims:
            m = self.bone_w(q, self.subtree(bones[0]) if len(bones) == 1 else bones) > thresh
            if where is not None:
                m &= np.array([bool(where(p)) for p in q["pos"]])
            q["drop"] = q.get("drop", np.zeros(len(m), dtype=bool)) | m
        self._bvh = None

    def stretch(self, bone, axis, amount, start=0.0):
        """Pull the vertices of a bone's subtree along `axis` (elongated jaws, distended bellies)."""
        a = np.array(axis, dtype=np.float64)
        c = np.array(self.J(bone))
        for q in self.prims:
            w = self.bone_w(q, self.subtree(bone))
            d = np.clip((q["pos"] - c) @ a - start, 0, None)
            q["pos"] = q["pos"] + a[None] * (d * amount * w)[:, None]


# ------------------------------------------------------------------------------------------------
# Shared horror

BONE = (0.95, 0.88, 0.74)
DARK_BLOOD = (0.3, 0.04, 0.04)
GREEN_GLOW = (0.6, 2.2, 0.5)
PINK_GLOW = (2.4, 0.6, 1.2)
VIOLET_GLOW = (1.3, 0.8, 2.8)
EMBER = (3.0, 1.2, 0.3)


def J(b, n):
    return b.J(KIT.get(n, n))


def hanging_jaw(b, drop=0.09, teeth=14):
    """The mouth torn open down to the throat: everything below the lips drops, tearing deepest
    at the centre; the skin stretched across the gap is the inside of the mouth; broken teeth
    line both lips."""
    h = J(b, "Head")
    mouth = h.y + 0.026
    for q in b.prims:
        w = b.bone_w(q, ["Head", "Neck"])
        p = q["pos"]
        front = np.clip((p[:, 2] - h.z - 0.03) / 0.04, 0, 1)
        centre = np.clip(1 - np.abs(p[:, 0] - h.x) / 0.075, 0, 1) ** 0.6
        below = np.clip((mouth - p[:, 1]) / 0.006, 0, 1)
        k = w * front * centre
        # the lip line and the skin that will stretch across the gap become the mouth's interior
        band = k * np.clip(1 - np.abs(p[:, 1] - (mouth - 0.004)) / 0.012, 0, 1)
        q["col"][:, :3] = q["col"][:, :3] * (1 - band[:, None]) + np.array([0.16, 0.012, 0.015]) * band[:, None]
        p[:, 1] -= drop * k * below
        p[:, 2] += drop * 0.25 * k * below
    b._bvh = None
    n = max(teeth // 2, 3)
    for row, y, d in ((0, mouth + 0.002, -UP), (1, mouth - drop - 0.004, UP)):
        for k in range(n):
            x = (k / (n - 1) - 0.5) * 0.06
            s, nn = b.surf(Vector((h.x + x, y, h.z + 0.08)), FWD)
            ln = 0.012 + 0.012 * b.rng.random() * (1.0 - abs(x) * 10)
            b.spike(s - nn * 0.004, d + FWD * 0.15 + LEFT * (b.rng.random() - 0.5) * 0.4, ln, 0.0035, color=(0.85, 0.78, 0.55), anchor=h)


def ribs_out(b, pairs=4, length=0.16, spread=0.16, at=None):
    """The chest split down the sternum, ribs levered outward through the skin: a ragged wound of
    torn tissue, some ribs snapped short, some bent back, all of them uneven."""
    c = at or J(b, "Torso") + Vector((0, -0.06, 0.0))
    s, n = b.surf(c, FWD)
    r = b.rng
    for k in range(5):
        o = UP * (r.random() - 0.5) * spread * 1.2 + LEFT * (r.random() - 0.5) * 0.05
        b.blob(s + o - n * 0.012, spread * r.uniform(0.18, 0.32), lobes=4, squash=(r.uniform(0.5, 0.9), r.uniform(0.8, 1.6), 0.35), tex="Flesh", seed=11 + k,
               color=(r.uniform(0.2, 0.42), 0.03, 0.035), veins=0.5, rough=0.08)
    for k in range(pairs):
        y = (k - (pairs - 1) / 2) * spread * 0.42 + (r.random() - 0.5) * 0.02
        for sx in (-1, 1):
            if r.random() < 0.15:
                continue
            base = s + UP * y + LEFT * sx * r.uniform(0.015, 0.04)
            snapped = r.random() < 0.3
            ln = length * r.uniform(0.55, 1.15) * (0.45 if snapped else 1.0) * (1 - abs(k - pairs / 2) * 0.1)
            d = n * r.uniform(0.35, 0.8) + LEFT * sx * r.uniform(0.7, 1.2) - UP * r.uniform(0.0, 0.35)
            b.spike(base, d, ln, r.uniform(0.008, 0.013), bend=-n, bend_amt=r.uniform(0.15, 0.55), color=(0.9, 0.82, 0.66) if not snapped else (0.75, 0.55, 0.45))


def spine_ridge(b, n=9, length=0.12, radius=0.016):
    pel, tor, nk = J(b, "Hips"), J(b, "Torso"), J(b, "Neck")
    for k in range(n):
        t = k / (n - 1)
        p = pel.lerp(tor, min(1, t * 1.6)) if t < 0.62 else tor.lerp(nk, (t - 0.62) / 0.38)
        s, nn = b.surf(p, -FWD)
        b.spike(s, nn + UP * 0.35, length * (0.6 + 0.6 * math.sin(t * math.pi)), radius, bend=UP, bend_amt=0.2, color=BONE)


def long_claws(b, side, length=0.28, radius=0.008):
    for f in ("Index1", "Middle1", "Pinky1", "Thumb1"):
        p = J(b, f"{f}.{side}")
        d = (p - J(b, f"LowerArm.{side}")).normalized()
        b.spike(p + d * 0.02, d - UP * 0.1, length * b.rng.uniform(0.85, 1.15), radius, bend=-UP, bend_amt=0.25, anchor=p, color=(0.2, 0.17, 0.14))


def gut_trail(b, n=4, length=0.6):
    """Intestines spilling from the abdomen and dragging behind."""
    c = J(b, "Abdomen") + Vector((0, -0.06, 0))
    s, nn = b.surf(c, FWD)
    for k in range(n):
        a = (k / max(n - 1, 1) - 0.5) * 1.6
        pts = [s, s + nn * 0.05 + LEFT * a * 0.04, s + nn * 0.1 - UP * length * 0.3 + LEFT * a * 0.08, s + nn * 0.08 - UP * length * 0.6, s - FWD * 0.1 - UP * length * 0.8]
        b.tentacle(pts, 0.022, 0.018, tex="Flesh", sides=7, bumps=0.25, color=(0.75, 0.42, 0.42), anchor=c, veins=0.3)


# ------------------------------------------------------------------------------------------------
# The infected

def carrier(b):
    """Rusher: starved to the frame, jaw torn loose and hanging, ribs prised out of the chest."""
    b.inflate(["Torso", "Abdomen", "UpperArm.L", "UpperArm.R", "UpperLeg.L", "UpperLeg.R"], -0.012, lumpy=0.4)
    b.paint(veins=0.4, bruise=0.4, blood=[("Head", 0.16), ("Torso", 0.12)], gore=0.25, seed=1)
    b.eyes(milky=True)
    hanging_jaw(b)
    ribs_out(b)
    long_claws(b, "L", 0.12)
    long_claws(b, "R", 0.12)


def shambler(b):
    """Rusher variant: the head split down the middle and peeled open like a seed pod."""
    h = J(b, "Head")
    for q in b.prims:
        w = b.bone_w(q, ["Head"])
        up = np.clip((q["pos"][:, 1] - h.y - 0.04) / 0.1, 0, 1)
        side = np.sign(q["pos"][:, 0] - h.x)
        q["pos"][:, 0] += side * 0.05 * up * w
    b.paint(veins=0.3, bruise=0.3, blood=[("Head", 0.2)], gore=0.3, seed=2)
    b.eyes(color=(0.9, 0.2, 0.15))
    top = h + Vector((0, 0.13, 0.01))
    b.blob(top - UP * 0.02, 0.05, lobes=4, squash=(0.6, 1.0, 1.2), tex="Flesh", seed=3, color=DARK_BLOOD, veins=0.6, rough=0.1, anchor=h)
    for k in range(10):
        a = (k / 9 - 0.5) * 2.4
        base = top + Vector((math.sin(a) * 0.035, -0.01, math.cos(a) * 0.04 - 0.01))
        b.spike(base, UP + Vector((math.sin(a) * 0.3, 0, 0.2)), 0.035, 0.005, color=BONE, anchor=h)
    for side in ("L", "R"):
        r = J(b, f"UpperArm.{side}") + Vector((0, -0.18, -0.06))
        s, n = b.surf(r, -FWD + (LEFT if side == "L" else -LEFT) * 0.6)
        pts = [s, s + n * 0.06, s + n * 0.12 - UP * 0.1 + FWD * 0.1, s + n * 0.1 - UP * 0.2 + FWD * 0.25]
        b.tentacle(pts, 0.022, 0.006, tex="Flesh", color=(0.7, 0.55, 0.5), anchor=J(b, "Torso"))
    spine_ridge(b, 7, 0.08, 0.012)


def bulk(b):
    """Brute: bloated to bursting, the skin stretched over tumours, one arm a swollen bone club."""
    b.inflate(["Torso", "Abdomen", "Hips"], 0.11, lumpy=0.7, seed=3)
    b.inflate(["UpperArm.R", "LowerArm.R"], 0.06, lumpy=0.8, seed=4)
    b.inflate(["Neck", "Shoulder.L", "Shoulder.R"], 0.04, lumpy=0.5, seed=5)
    b.paint(veins=0.6, bruise=0.55, blood=[("Abdomen", 0.2)], gore=0.15, seed=3)
    b.eyes(color=(0.9, 0.8, 0.4))
    for k, (at, out) in enumerate([(J(b, "Abdomen") + Vector((0.08, 0, 0.1)), FWD + LEFT * 0.5), (J(b, "Torso") + Vector((-0.12, 0.05, 0.05)), FWD - LEFT * 0.6),
                                    (J(b, "Abdomen") + Vector((-0.06, -0.08, 0.06)), FWD), (J(b, "Torso") + Vector((0.1, 0.1, -0.08)), -FWD + LEFT)]):
        s, n = b.surf(at, out)
        b.blob(s, 0.07 + 0.02 * k % 3, lobes=5, tex="Flesh", seed=20 + k, color=(0.85, 0.6, 0.55), veins=0.8, rough=0.3)
        b.pustules(s + n * 0.04, 0.06, 6, 0.014, seed=30 + k, color=(1.6, 1.3, 0.3), glow=0.4)
    h = J(b, "Middle1.R")
    d = (h - J(b, "LowerArm.R")).normalized()
    for k in range(6):
        a = k / 6 * math.tau
        o = Vector((math.cos(a), math.sin(a), 0)) * 0.05
        b.spike(h + d * 0.02 + o, d + o * 4, 0.12, 0.016, color=BONE, anchor=h)


def retch(b):
    """Spitter: the throat swollen into a glowing bile sac, the lower jaw split into mandibles."""
    b.inflate(["Neck"], 0.06, lumpy=0.4, seed=6)
    b.paint(veins=0.4, bruise=0.3, blood=[("Head", 0.12)], glow_veins=0.6, glow_col=GREEN_GLOW, seed=6)
    b.eyes(color=(0.6, 1.0, 0.3), glow=0.4, milky=False)
    nk = J(b, "Neck")
    s, n = b.surf(nk + Vector((0, -0.02, 0.06)), FWD)
    b.blob(s + n * 0.04, 0.09, lobes=4, squash=(1.0, 0.9, 1.1), tex="GrowthGlow", seed=7, color=(0.6, 1.6, 0.4), glow=0.6, veins=0.9, rough=0.15, anchor=nk)
    h = J(b, "Head")
    for sx in (-1, 1):
        base = h + Vector((0.03 * sx, -0.06, 0.07))
        b.spike(base, FWD + LEFT * sx * 0.5 - UP * 0.4, 0.13, 0.012, bend=-LEFT * sx, bend_amt=-0.5, color=BONE, anchor=h)
    for k in range(5):
        p = J(b, "Torso") + Vector(((k - 2) * 0.05, 0.03, -0.1))
        s, n = b.surf(p, -FWD)
        b.pipe(s, n + UP * 0.6, 0.18, 0.016, tex="Bone", color=(0.6, 0.65, 0.4), anchor=J(b, "Torso"), bend=UP)


def larva(b):
    """Crawler: legs gone below the hips, guts dragging, arms long and clawed for hauling."""
    b.remove(["UpperLeg.L"])
    b.remove(["UpperLeg.R"])
    b.paint(veins=0.4, bruise=0.5, blood=[("Hips", 0.25), ("Abdomen", 0.15)], gore=0.4, seed=8)
    b.eyes()
    hp = J(b, "Hips")
    s, n = b.surf(hp - UP * 0.08, -UP)
    b.blob(s, 0.11, lobes=4, squash=(1.2, 0.5, 1.0), tex="Flesh", seed=8, color=DARK_BLOOD, veins=0.5, rough=0.1)
    gut_trail(b, 5, 0.7)
    long_claws(b, "L", 0.16)
    long_claws(b, "R", 0.16)
    hanging_jaw(b, 0.05, 10)


def bulwark(b):
    """Shielded: the ribcage turned inside out into a breastplate of bone, forearms plated."""
    b.inflate(["Torso"], 0.03, lumpy=0.3, seed=9)
    b.paint(veins=0.3, bruise=0.3, blood=[("Torso", 0.18)], seed=9)
    b.eyes(color=(0.85, 0.85, 0.8))
    t = J(b, "Torso")
    r = b.rng
    # the ribs, grown thick and fused into a cage that wraps the chest
    for k in range(7):
        y = t.y + 0.1 - k * 0.055 + (r.random() - 0.5) * 0.01
        path, radii = [], []
        for i in range(9):
            a = (i / 8 - 0.5) * 2.6
            dirv = Vector((math.sin(a), 0, math.cos(a)))
            s, n = b.surf(Vector((t.x, y, t.z)) + dirv * 0.05, dirv)
            path.append(s + n * (0.025 + 0.01 * math.sin(i * 1.7 + k)))
            radii.append(0.013 + 0.008 * r.random() + 0.006 * math.cos(a))
        b.parts.append(be.Part(b.tube(path, radii, sides=7, bumps=0.25, name=f"rib{k}"), "Bone", mode="surface", color=(0.9 - 0.1 * r.random(), 0.82, 0.68), ao=True))
    for side in ("L", "R"):
        e, h = J(b, f"LowerArm.{side}"), J(b, f"Middle1.{side}")
        mid = e.lerp(h, 0.45)
        s, n = b.surf(mid, -FWD + (LEFT if side == "L" else -LEFT))
        b.plate(s, n, (h - e).normalized(), 0.1, 0.22, curve=0.5, thick=0.02, ridges=6, tex="Chitin", color=(0.45, 0.4, 0.38))
    h = J(b, "Head")
    for k in range(7):
        x = (k - 3) * 0.022
        s, n = b.surf(h + Vector((x, 0.16, 0.02 - abs(x))), UP + FWD * 0.2)
        b.spike(s, n - FWD * 0.4, 0.07 + 0.05 * (1 - abs(k - 3) / 3), 0.012, bend=-FWD, bend_amt=0.3, color=BONE, anchor=h)


def lurker(b):
    """Ambusher: drained white, eyes grown over, a vertical mouth, fingers lengthened to blades."""
    b.paint(veins=0.2, bruise=0.1, seed=10)
    h = J(b, "Head")
    for sx in (-1, 1):
        s, n = b.surf(h + Vector((0.033 * sx, 0.035, 0.08)), FWD)
        b.blob(s, 0.022, lobes=2, squash=(1.2, 0.8, 0.5), tex="Skin", seed=10 + sx, color=(0.95, 0.92, 0.9), veins=0.4, rough=0.3, anchor=h)
    s, n = b.surf(h + Vector((0, -0.02, 0.09)), FWD)
    b.blob(s, 0.03, lobes=2, squash=(0.35, 1.6, 0.4), tex="Flesh", seed=12, color=DARK_BLOOD, rough=0.05, anchor=h)
    for k in range(8):
        b.spike(s + Vector((0.01 * (1 if k % 2 else -1), (k - 3.5) * 0.008, 0.005)), FWD + LEFT * (1 if k % 2 else -1) * -0.8, 0.02, 0.003, color=BONE, anchor=h)
    long_claws(b, "L", 0.42, 0.007)
    long_claws(b, "R", 0.42, 0.007)
    spine_ridge(b, 11, 0.06, 0.01)


def matron(b):
    """Summoner: the belly a translucent womb-sac crawling with larvae; arms sprouting from the ribs."""
    b.paint(veins=0.5, bruise=0.4, blood=[("Abdomen", 0.2)], glow_veins=0.4, glow_col=PINK_GLOW, seed=11)
    b.eyes(color=(1.0, 0.5, 0.6), glow=0.3, milky=False)
    a = J(b, "Abdomen")
    s, n = b.surf(a + Vector((0, -0.02, 0.08)), FWD)
    b.blob(s + n * 0.12, 0.19, lobes=5, squash=(1.0, 1.1, 1.0), tex="Vein", seed=13, color=(1.0, 0.62, 0.62), glow=0.18, veins=1.0, rough=0.05, anchor=a)
    # larvae pressing out against the membrane
    for k in range(9):
        d = Vector((math.sin(k * 2.1), math.cos(k * 1.7) * 0.8, 0.9 + 0.3 * math.sin(k))).normalized()
        b.blob(s + n * 0.12 + d * 0.17, 0.045, lobes=2, squash=(1.9, 0.75, 0.75), tex="Flesh", seed=40 + k, color=(0.95, 0.88, 0.72), veins=0.2, anchor=a)
    for side, sx in (("L", 1), ("R", -1)):
        for k in range(2):
            r = J(b, "Torso") + Vector((0.12 * sx, -0.08 - k * 0.1, -0.02))
            s2, n2 = b.surf(r, LEFT * sx)
            pts = [s2, s2 + n2 * 0.08, s2 + n2 * 0.2 + FWD * 0.12 - UP * 0.05, s2 + n2 * 0.28 + FWD * 0.3 - UP * 0.15]
            b.tentacle(pts, 0.025, 0.008, tex="Flesh", color=(0.8, 0.62, 0.6), anchor=J(b, "Torso"))
            b.spike(pts[-1], FWD - UP * 0.3, 0.07, 0.008, color=(0.2, 0.17, 0.14), anchor=J(b, "Torso"))
    b.pustules(J(b, "Head") + Vector((0, 0.02, 0.09)), 0.06, 8, 0.01, seed=14, color=(1.6, 1.2, 0.4), glow=0.3)


def digestor(b):
    """Boss (the Gut): the whole torso opened into a ringed stomach-maw, acid weeping from sacs."""
    b.inflate(["Torso", "Abdomen"], 0.09, lumpy=0.6, seed=15)
    b.paint(veins=0.6, bruise=0.5, blood=[("Abdomen", 0.3), ("Head", 0.2)], glow_veins=0.5, glow_col=GREEN_GLOW, gore=0.35, seed=15)
    b.eyes(color=(0.7, 1.0, 0.3), glow=0.5, milky=False)
    a = J(b, "Abdomen") + Vector((0, 0.05, 0))
    s, n = b.surf(a, FWD)
    b.maw(s, n, 0.17, teeth=26, tooth_len=0.07, color=(0.6, 0.25, 0.25), anchor=a, inner=(0.12, 0.02, 0.01))
    for k in range(6):
        at = J(b, "Torso") + Vector((math.sin(k * 1.1) * 0.18, 0.05 + 0.04 * k % 2, -0.1))
        s2, n2 = b.surf(at, -FWD + LEFT * math.sin(k * 1.1))
        b.blob(s2 + n2 * 0.03, 0.06, lobes=3, tex="GrowthGlow", seed=50 + k, color=(0.5, 1.7, 0.4), glow=0.6, veins=0.7)
    hanging_jaw(b, 0.1, 18)


def geometer(b):
    """Boss (the Lattice): crystal geometry forcing its way out through skin and skull."""
    b.paint(veins=0.3, bruise=0.2, glow_veins=0.7, glow_col=VIOLET_GLOW, seed=16)
    b.eyes(color=(1.2, 0.9, 2.6), glow=0.8, milky=False)
    h = J(b, "Head")
    for k in range(7):
        a = k / 7 * math.tau
        d = Vector((math.cos(a) * 0.6, 1.0, math.sin(a) * 0.6)).normalized()
        b.crystal(h + d * 0.06, d, 0.22 + 0.1 * (k % 3), 0.035, color=VIOLET_GLOW, anchor=h)
    for bone, out in (("Torso", FWD + LEFT), ("Torso", -FWD - LEFT * 0.5), ("UpperArm.L", LEFT + UP), ("UpperArm.R", -LEFT + UP), ("Abdomen", FWD - LEFT), ("UpperLeg.L", FWD)):
        s, n = b.surf(J(b, bone), out)
        for k in range(3):
            d = (n + Vector((math.sin(k * 2.3), math.cos(k * 1.7), math.sin(k))) * 0.4).normalized()
            b.crystal(s, d, 0.14 + 0.05 * k, 0.025, color=VIOLET_GLOW, anchor=J(b, bone))


def mother_cinder(b):
    """Boss (the Kiln): burnt to charcoal and still burning, horns of fused bone, ribs glowing."""
    b.inflate(["Torso", "UpperArm.L", "UpperArm.R"], 0.03, lumpy=0.6, seed=17)
    b.paint(veins=0.0, bruise=0.0, char=0.4, ember=1.0, seed=17)
    b.eyes(color=EMBER, glow=1.0, milky=False)
    h = J(b, "Head")
    for sx in (-1, 1):
        base = h + Vector((0.05 * sx, 0.08, 0.02))
        b.spike(base, UP + LEFT * sx * 0.8 - FWD * 0.3, 0.32, 0.03, bend=-FWD, bend_amt=0.5, tex="Char", color=(0.3, 0.25, 0.22), anchor=h)
    ribs_out(b, 5, 0.18, 0.2)
    for k in range(8):
        at = J(b, "Torso").lerp(J(b, "Hips"), k / 8) + Vector((math.sin(k * 2.0) * 0.12, 0, 0.1 * math.cos(k * 1.3)))
        s, n = b.surf(at, FWD * math.cos(k * 1.3) + LEFT * math.sin(k * 2.0))
        b.crystal(s, n + UP * 0.3, 0.08, 0.02, tex="Ember", glow=0.9, color=EMBER, anchor=at)


def cantor(b):
    """Boss (the Choir): mouths everywhere, singing — up the arms, across the chest, in the palms."""
    b.paint(veins=0.4, bruise=0.3, blood=[("Head", 0.15)], seed=18)
    b.eyes(color=(0.9, 0.9, 0.95))
    for bone, out, r in (("Torso", FWD, 0.07), ("Abdomen", FWD + LEFT * 0.4, 0.05), ("Abdomen", FWD - LEFT * 0.4, 0.05), ("UpperArm.L", FWD + LEFT, 0.04),
                         ("UpperArm.R", FWD - LEFT, 0.04), ("LowerArm.L", FWD, 0.035), ("LowerArm.R", FWD, 0.035), ("UpperLeg.L", FWD, 0.045), ("UpperLeg.R", FWD, 0.045)):
        s, n = b.surf(J(b, bone), out)
        b.maw(s, n, r, teeth=12, tooth_len=r * 0.5, color=(0.6, 0.3, 0.3), anchor=J(b, bone), inner=(0.06, 0.0, 0.01))
    h = J(b, "Head")
    for k in range(6):
        a = k / 6 * math.tau
        b.petal(h + Vector((0, 0.08, 0)), Vector((math.cos(a), 1.2, math.sin(a))).normalized(), 0.2, 0.08, curl=0.7, tex="Flesh", color=(0.8, 0.5, 0.55), anchor=h)
    hanging_jaw(b, 0.07, 16)


def seed_boss(b):
    """Final boss (the Seed): rooted, opened into a flower, everything it was hanging from it."""
    b.paint(veins=0.5, bruise=0.3, glow_veins=0.6, glow_col=PINK_GLOW, gore=0.3, seed=19)
    b.eyes(color=PINK_GLOW, glow=0.7, milky=False)
    h = J(b, "Head")
    for k in range(9):
        a = k / 9 * math.tau
        b.petal(h + Vector((0, 0.05, 0)), Vector((math.cos(a), 0.6, math.sin(a))).normalized(), 0.42, 0.14, curl=0.8, tex="Flesh", color=(0.85, 0.45, 0.5), anchor=h)
    b.blob(h + Vector((0, 0.12, 0)), 0.07, lobes=4, tex="GrowthGlow", seed=60, color=PINK_GLOW, glow=0.9, veins=0.6, anchor=h)
    for k in range(8):
        a = k / 8 * math.tau
        root = J(b, "Hips") + Vector((math.cos(a) * 0.12, -0.1, math.sin(a) * 0.12))
        pts = [root, root + Vector((math.cos(a) * 0.15, -0.3, math.sin(a) * 0.15)), root + Vector((math.cos(a) * 0.4, -0.7, math.sin(a) * 0.4)), root + Vector((math.cos(a) * 0.7, -0.9, math.sin(a) * 0.7))]
        b.tentacle(pts, 0.04, 0.012, tex="Growth", color=(0.6, 0.45, 0.4), anchor=J(b, "Hips"))
    gut_trail(b, 6, 0.8)
    ribs_out(b, 4, 0.2, 0.18)


# name: (body, base seed, rags, skin source, skin kind, recipe)
RECIPES = {
    "Strain_Carrier": ("Male", 1, "rags", "skin_male_light", "pale", carrier),
    "Strain_Shambler": ("Female", 2, "rags", "skin_female_light", "rot", shambler),
    "Strain_Bulk": ("Male", 3, "shorts", "skin_male_light", "pale", bulk),
    "Strain_Retch": ("Female", 4, "rags", "skin_female_dark", "pale", retch),
    "Strain_Larva": ("Male", 5, "none", "skin_male_light", "flayed", larva),
    "Strain_Bulwark": ("Male", 6, "rags", "skin_male_dark", "pale", bulwark),
    "Strain_Lurker": ("Female", 7, "none", "skin_female_light", "white", lurker),
    "Strain_Matron": ("Female", 8, "rags", "skin_female_light", "pale", matron),
    "Strain_Digestor": ("Male", 9, "none", "skin_male_light", "flayed", digestor),
    "Strain_Geometer": ("Male", 10, "rags", "skin_male_light", "white", geometer),
    "Strain_MotherCinder": ("Female", 11, "none", "skin_female_dark", "burnt", mother_cinder),
    "Strain_Cantor": ("Male", 12, "rags", "skin_male_dark", "rot", cantor),
    "Strain_Seed": ("Female", 13, "none", "skin_female_light", "flayed", seed_boss),
}

BODIES = {}


def garments_for(kind, seed):
    if kind == "none":
        return []
    if kind == "shorts":
        return [torn(bh.pants("Cloth", (0.25, 0.22, 0.18), cuff=0.45), seed, 0.5)]
    return [torn(bh.shirt("Cloth", (0.45, 0.42, 0.38), sleeves="short"), seed, 0.25), torn(bh.pants("Denim", (0.16, 0.18, 0.22)), seed + 1, 0.35)]


def main():
    import glob

    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    preview = sys.argv[sys.argv.index("--preview") + 1] if "--preview" in sys.argv else None
    args = [a for a in args if a != preview]
    ubc = bh.source_dir("STRAIN_UBC_DIR", "universal-base-characters")
    for s in ("Male", "Female"):
        BODIES[s] = next(p for p in glob.glob(os.path.join(glob.escape(ubc), "**", f"Superhero_{s}_FullBody.gltf"), recursive=True) if "Godot" in p)
    skins = {}
    for name in args or list(RECIPES):
        sex, seed, rags, src, kind, fn = RECIPES[name]
        skin = f"inf_{kind}_{src.split('_', 1)[1]}"
        if skin not in skins:
            diseased_skin(skin, src, kind, seed=len(skins) * 13 + 5)
            skins[skin] = True
        base = build_base(sex, seed, garments_for(rags, seed))
        clear_scene()
        b = RealBuilder(name, base, skin, seed)
        fn(b)
        out = os.path.join(bh.MODELS, name + ".glb")
        n = write_model(b, out)
        print(f"{name}: {len(b.parts)} parts, {n} new vertices")
        if preview:
            os.makedirs(preview, exist_ok=True)


if __name__ == "__main__":
    main()
