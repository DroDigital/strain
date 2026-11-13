"""Photo materials for the kit's static models (vehicles, street props, weapons).

    python3 tools/blender/props.py [names...]

The Zombie Apocalypse Kit paints every model with flat swatches from one atlas. This pass maps
each swatch to the game's photo material layers by colour and context: car paint and painted
containers become `Paint` tinted to the swatch, greys metal, browns wood, blacks rubber, pale
blues glass; jersey barriers are concrete, couches upholstery, cones and bags plastic. Surfaces
get box-projected UVs and grime that gathers toward the ground. Geometry is untouched. A marker
in `asset.extras` makes the pass idempotent.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import numpy as np  # noqa: E402
from PIL import Image  # noqa: E402

from glb import Glb  # noqa: E402

ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
MODELS = os.path.join(ROOT, "assets", "models")
TEXTURES = os.path.join(ROOT, "assets", "textures")
SKIP = ("Strain_", "Human_", "Clips_", "Characters_", "Zombie_", "Blood_")


def srgb_to_lin(c):
    c = np.asarray(c, dtype=np.float64)
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def photo_mean(name, f=lambda c: c):
    a = np.asarray(Image.open(os.path.join(TEXTURES, name + ".png")).convert("RGB")).astype(np.float64) / 255
    ao = np.asarray(Image.open(os.path.join(TEXTURES, name + "_n.png")).convert("RGBA")).astype(np.float64)[..., 3] / 255
    c = np.clip(f(a), 0, 1) * (0.35 + 0.65 * ao)[..., None]
    return srgb_to_lin(c).reshape(-1, 3).mean(0)


def grey(a):
    lum = (a * [0.3, 0.55, 0.15]).sum(-1, keepdims=True)
    return np.repeat(lum, 3, -1)


# linear mean albedo of each target layer, mirroring src/render/textures.rs
MEANS = {
    "Paint": np.full(3, float(srgb_to_lin(0.8 * 0.94))),
    "Metal": photo_mean("metal"),
    "Rust": photo_mean("rust"),
    "Wood": photo_mean("wood"),
    "Concrete": photo_mean("concrete"),
    "Cloth": photo_mean("fabric", lambda a: grey(a) * 1.35),
    "Char": photo_mean("gravel", lambda a: grey(a) * 0.35),
    "White": np.ones(3),
    "Glass": np.array([0.04, 0.06, 0.08]),
}

OVERRIDES = {
    "CinderBlock": lambda c, d: "Concrete",
    "Couch": lambda c, d: "Wood" if c[0] < 0.3 and c[2] < 0.15 else "Cloth",
    "TrafficCone_1": lambda c, d: "White",
    "TrafficCone_2": lambda c, d: "White",
    "PlasticBarrier": lambda c, d: "White",
    "TrafficBarrier_1": lambda c, d: "Concrete" if abs(c[0] - c[2]) < 0.05 and c[0] < 0.6 else "White",
    "TrafficBarrier_2": lambda c, d: "Concrete" if abs(c[0] - c[2]) < 0.05 and c[0] < 0.6 else "White",
    "TrashBag_1": lambda c, d: "White",
    "TrashBag_2": lambda c, d: "White",
    "Wheels_Stack": lambda c, d: "Metal" if c[0] > 0.45 else "Char",
    "Pallet": lambda c, d: "Wood",
    "Pallet_Broken": lambda c, d: "Wood",
    "WaterTower": lambda c, d: "Rust",
    "Pipes": lambda c, d: "Rust" if abs(c[0] - c[2]) > 0.1 or c[0] > 0.4 else d,
}


def classify(c, vehicle):
    lum = (c * [0.3, 0.55, 0.15]).sum()
    sat = c.max() - c.min()
    if (c[2] > c[0] + 0.15 and c[1] > c[0]) or (c[1] > c[0] + 0.2 and c[2] > c[0] + 0.15):
        return "Glass"
    if c[0] > 0.6 and c[1] < 0.1:
        return "Glass"  # tail and signal lenses
    if lum < 0.13:
        return "Char"
    if c[0] > c[2] + 0.12 and sat < 0.4 and 0.2 < lum < 0.65 and c[1] > c[2]:
        return "Wood"
    if c[0] > 0.5 and c[1] < 0.35 and c[2] < 0.15:
        return "Rust" if lum < 0.35 else "Paint"
    if sat > 0.2:
        return "Paint"
    return "Paint" if vehicle and lum > 0.3 else "Metal"


def process(name):
    path = os.path.join(MODELS, name + ".glb")
    g = Glb(path)
    extras = g.j.setdefault("asset", {}).setdefault("extras", {})
    if extras.get("materials"):
        return False
    at = np.asarray(Image.open(os.path.join(MODELS, "atlas.png")).convert("RGB")).astype(np.float64) / 255
    h, w = at.shape[:2]
    mats = g.j.setdefault("materials", [])
    names = [m.get("name", "") for m in mats]
    vehicle = name.startswith("Vehicle_")
    rng = np.random.default_rng(len(name))
    changed = False
    for mesh in g.j["meshes"]:
        new = []
        for p in mesh["primitives"]:
            a = p["attributes"]
            if "material" not in p or names[p["material"]] != "Atlas" or "TEXCOORD_0" not in a:
                new.append(p)
                continue
            uv = g.acc(a["TEXCOORD_0"])
            pos = g.acc(a["POSITION"]).astype(np.float64)
            nrm = g.acc(a["NORMAL"]).astype(np.float64) if "NORMAL" in a else np.tile([0, 1, 0], (len(pos), 1))
            idx = g.acc(p["indices"]).astype(np.int64) if "indices" in p else np.arange(len(pos))
            c = at[np.clip(((uv[:, 1] % 1) * h).astype(int), 0, h - 1), np.clip(((uv[:, 0] % 1) * w).astype(int), 0, w - 1)]
            ov = OVERRIDES.get(name)
            cls = [ov(ci, classify(ci, vehicle)) if ov else classify(ci, vehicle) for ci in c]
            # colours: the swatch's hue on the layer, a little grime toward the ground
            lin = srgb_to_lin(c)
            col = np.ones((len(pos), 4))
            ext = max(np.ptp(pos[:, 1]), 1e-3)
            hgt = (pos[:, 1] - pos[:, 1].min()) / ext
            for i in range(len(pos)):
                k = cls[i]
                v = lin[i] / MEANS[k] if k not in ("Glass", "Char") else (np.ones(3) if k == "Char" else lin[i] / 0.35)
                grime = 0.82 + 0.18 * hgt[i] if k not in ("Glass",) else 1.0
                col[i, :3] = np.clip(v * grime * rng.uniform(0.96, 1.04), 0, 1) ** 2.2
            ax = np.abs(nrm).argmax(1)
            box = np.where(ax[:, None] == 0, pos[:, [2, 1]], np.where(ax[:, None] == 1, pos[:, [0, 2]], pos[:, [0, 1]])) * 1.6
            tri = idx.reshape(-1, 3)
            tcls = np.array([cls[t[0]] for t in tri])
            for layer in sorted(set(tcls)):
                sel = tri[tcls == layer]
                used, local = np.unique(sel.ravel(), return_inverse=True)
                mname = "T:" + layer
                if mname not in names:
                    mats.append({"name": mname, "pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 0.8}})
                    names.append(mname)
                na = {
                    "POSITION": g.add(pos[used].astype(np.float32), "pos", 5126, target=34962, minmax=True),
                    "NORMAL": g.add(nrm[used].astype(np.float32), "nrm", 5126, target=34962),
                    "TEXCOORD_0": g.add(box[used].astype(np.float32), "uv", 5126, target=34962),
                    "COLOR_0": g.add(np.round(col[used] * 255), "color", 5121, target=34962),
                }
                new.append({"attributes": na, "indices": g.add(local.ravel().astype(np.uint32), "idx", 5125, target=34963), "material": names.index(mname)})
            changed = True
        mesh["primitives"] = new
    if changed:
        extras["materials"] = 1
        g.save(path)
    return changed


def main():
    names = [a for a in sys.argv[1:]] or sorted(os.path.basename(f)[:-4] for f in os.listdir(MODELS) if f.endswith(".glb") and not f.startswith(SKIP))
    for n in names:
        if process(n):
            print("materials", n)


if __name__ == "__main__":
    main()
