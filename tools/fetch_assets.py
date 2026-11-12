#!/usr/bin/env python3
"""Fetch and convert STRAIN's third-party assets into ./assets.

Every source is CC0 (public domain). The processed files are committed to the
repository, so building the game never needs the network; this script exists
so the asset set can be reproduced, extended or audited.

Requirements: Python 3.9+, Pillow, ffmpeg on PATH.

    python3 tools/fetch_assets.py            # everything
    python3 tools/fetch_assets.py textures   # one group: textures | models | audio
"""
import base64
import io
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import urllib.parse
import urllib.request
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets")
CACHE = os.environ.get("STRAIN_ASSET_CACHE", os.path.join(ROOT, "target", "asset-cache"))
UA = {"User-Agent": "strain-asset-fetch/1.0"}
TEX_SIZE = 512

CREDITS = []


def fetch(url, dest):
    """Download url to dest (cached)."""
    if os.path.exists(dest) and os.path.getsize(dest) > 0:
        return dest
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    req = urllib.request.Request(url, headers=UA)
    with urllib.request.urlopen(req, timeout=180) as r, open(dest + ".part", "wb") as f:
        shutil.copyfileobj(r, f)
    os.replace(dest + ".part", dest)
    return dest


# ---------------------------------------------------------------------------------------------
# Textures: ambientCG (CC0). Each surface keeps two TEX_SIZE maps: `<name>.png` (colour) and
# `<name>_n.png` (material: R/G tangent-space normal in OpenGL convention, B roughness, A ambient
# occlusion). The game's PBR shader reads both.
TEXTURES = {
    "asphalt": "Asphalt031",
    "paving": "PavingStones128",
    "brick": "Bricks097",
    "brick_alt": "Bricks059",
    "concrete": "Concrete042A",
    "concrete_light": "Concrete034",
    "plaster": "Plaster007",
    "metal": "Metal063",
    "diamond_plate": "DiamondPlate008C",
    "rust": "Metal041B",
    "corrugated": "CorrugatedSteel005",
    "wood": "Planks037A",
    "crate": "WoodFloor043",
    "fabric": "Fabric061",
    "fabric_coarse": "Fabric086",
    "leather": "Leather037",
    "chitin": "Rock058",
    "rock": "Rock051",
    "moss_rock": "Rock064",
    "bone": "Onyx015",
    "lava": "Lava001",
    "gravel": "Ground110",
    "tiles": "Tiles141",
    "paper": "Paper005",
    "grass": "Grass004",
    "roof": "Asphalt033",
}


def _material_map(zf, size):
    """Pack NormalGL (RG), Roughness (B) and AmbientOcclusion (A) into one RGBA image."""
    from PIL import Image, ImageOps

    def get(pat, mode):
        n = next((n for n in zf.namelist() if re.search(pat, n)), None)
        if n is None:
            return None
        return Image.open(io.BytesIO(zf.read(n))).convert(mode).resize((size, size), Image.LANCZOS)

    nrm = get(r"_NormalGL\.jpg$", "RGB")
    if nrm is None:
        nrm = get(r"_NormalDX\.jpg$", "RGB")
        if nrm is not None:
            r, g, b = nrm.split()
            nrm = Image.merge("RGB", (r, ImageOps.invert(g), b))
    if nrm is None:
        nrm = Image.new("RGB", (size, size), (128, 128, 255))
    rough = get(r"_Roughness\.jpg$", "L") or Image.new("L", (size, size), 180)
    ao = get(r"_AmbientOcclusion\.jpg$", "L") or Image.new("L", (size, size), 255)
    r, g, _ = nrm.split()
    return Image.merge("RGBA", (r, g, rough, ao))


def textures():
    out = os.path.join(OUT, "textures")
    os.makedirs(out, exist_ok=True)
    from PIL import Image

    for name, asset in TEXTURES.items():
        z = fetch(f"https://ambientcg.com/get?file={asset}_1K-JPG.zip", os.path.join(CACHE, "acg", f"{asset}.zip"))
        with zipfile.ZipFile(z) as zf:
            color = next(n for n in zf.namelist() if re.search(r"_(Color|Diffuse)\.jpg$", n))
            im = Image.open(io.BytesIO(zf.read(color))).convert("RGB")
            mat = _material_map(zf, TEX_SIZE)
        im = im.resize((TEX_SIZE, TEX_SIZE), Image.LANCZOS)
        im.save(os.path.join(out, f"{name}.png"), optimize=True)
        mat.save(os.path.join(out, f"{name}_n.png"), optimize=True)
        CREDITS.append(("texture", f"textures/{name}.png, textures/{name}_n.png", f"ambientCG {asset}", f"https://ambientcg.com/view?id={asset}", "CC0 1.0"))
        print("texture", name, asset)


# ---------------------------------------------------------------------------------------------
# Models: Quaternius "Zombie Apocalypse Kit" (CC0), glTF from the author's public Drive folder.
QZ_ROOT = "1mWP6sCHun7OUMHQeDNZLrXTteXlzWg_t"
QZ_GLTF_FOLDERS = {
    "characters": "1wtzW_rZvTQ0yiYb6-4P0zwohF6KwCQ6H",
    "environment": "1GBJC9SeyXAYwhGhqaolbDgkgnzaMUMBc",
    "vehicles": "1gU4EDvCbI5DCXBUskqYRzmps5Du3KUvF",
    "weapons": "1QuDXe9dfk-kkrzLzUVd8GZgoWTT50Owc",
}
MODELS = [
    # characters (skinned, animated)
    # the humans and infected are built on Universal Base Characters (tools/blender/build_humans.py,
    # build_infected.py); from the kit we keep a reference human, the dog rig and the zombie whose
    # clips are retargeted onto the realistic skeletons
    "Characters_Matt_SingleWeapon", "Characters_GermanShepherd", "Zombie_Basic",
    # weapons
    "Axe", "Knife", "Rifle", "Spear", "WoodenBat_Barbed", "WoodenBat_Saw", "Pistol",
    # street props
    "Barrel", "Blood_1", "Blood_2", "Blood_3", "CinderBlock", "Container_Green", "Container_Red", "Couch",
    "FireHydrant", "Pallet", "Pallet_Broken", "Pipes", "PlasticBarrier", "StreetLights", "TrafficBarrier_1",
    "TrafficBarrier_2", "TrafficCone_1", "TrafficCone_2", "TrafficLight_1", "TrashBag_1", "TrashBag_2",
    "WaterTower", "Wheels_Stack", "Chest",
    # vehicles
    "Vehicle_Pickup", "Vehicle_Pickup_Armored", "Vehicle_Sports", "Vehicle_Truck", "Vehicle_Truck_Armored",
]


def drive_list(folder):
    html = urllib.request.urlopen(urllib.request.Request(f"https://drive.google.com/embeddedfolderview?id={folder}", headers=UA), timeout=60).read().decode("utf8", "replace")
    out = {}
    for m in re.finditer(r'href="https://drive.google.com/(file/d|drive/folders)/([^"/?]+)[^"]*".*?flip-entry-title">([^<]*)<', html, re.S):
        out[m.group(3)] = m.group(2)
    return out


def drive_file(fid, dest):
    return fetch(f"https://drive.usercontent.google.com/download?id={fid}&export=download&confirm=t", dest)


def gltf_to_glb(src_path, dst_path):
    """Pack a .gltf with data-URI buffers into .glb, dropping embedded images
    (every model in the kit shares one atlas, stored once as atlas.png)."""
    g = json.load(open(src_path))
    blob = bytearray()
    old = [base64.b64decode(b["uri"].split(",", 1)[1]) for b in g["buffers"]]
    image_views = {img["bufferView"] for img in g.get("images", []) if "bufferView" in img}
    remap = {}
    new_views = []
    for i, bv in enumerate(g["bufferViews"]):
        if i in image_views:
            continue
        data = old[bv.get("buffer", 0)][bv.get("byteOffset", 0): bv.get("byteOffset", 0) + bv["byteLength"]]
        while len(blob) % 4:
            blob.append(0)
        nb = dict(bv)
        nb["buffer"] = 0
        nb["byteOffset"] = len(blob)
        blob += data
        remap[i] = len(new_views)
        new_views.append(nb)
    g["bufferViews"] = new_views
    for a in g.get("accessors", []):
        if "bufferView" in a:
            a["bufferView"] = remap[a["bufferView"]]
    g.pop("images", None)
    g.pop("textures", None)
    g.pop("samplers", None)
    for m in g.get("materials", []):
        pbr = m.get("pbrMetallicRoughness", {})
        if "baseColorTexture" in pbr:
            pbr.pop("baseColorTexture")
            m.setdefault("extras", {})["atlas"] = True
    g["buffers"] = [{"byteLength": len(blob)}]
    js = json.dumps(g, separators=(",", ":")).encode()
    while len(js) % 4:
        js += b" "
    while len(blob) % 4:
        blob.append(0)
    total = 12 + 8 + len(js) + 8 + len(blob)
    with open(dst_path, "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, total))
        f.write(struct.pack("<II", len(js), 0x4E4F534A) + js)
        f.write(struct.pack("<II", len(blob), 0x004E4942) + bytes(blob))


def models():
    out = os.path.join(OUT, "models")
    os.makedirs(out, exist_ok=True)
    root = drive_list(QZ_ROOT)
    files = {}
    for folder in QZ_GLTF_FOLDERS.values():
        files.update(drive_list(folder))
    atlas = drive_file(root["Zombie_Atlas.png"], os.path.join(CACHE, "qz", "Zombie_Atlas.png"))
    from PIL import Image

    Image.open(atlas).convert("RGBA").save(os.path.join(out, "atlas.png"), optimize=True)
    lic = drive_file(root["License.txt"], os.path.join(CACHE, "qz", "License.txt"))
    shutil.copy(lic, os.path.join(out, "QUATERNIUS_LICENSE.txt"))
    for name in MODELS:
        src = drive_file(files[name + ".gltf"], os.path.join(CACHE, "qz", name + ".gltf"))
        gltf_to_glb(src, os.path.join(out, name + ".glb"))
        print("model", name)
    CREDITS.append(("models", "models/*.glb, models/atlas.png", "Quaternius - Zombie Apocalypse Kit", "https://quaternius.com/packs/zombieapocalypsekit.html", "CC0 1.0"))


# ---------------------------------------------------------------------------------------------
# Audio: OpenGameArt packs released under CC0. Each game sound gets several variants.
OGA = {
    "creature": ("80-cc0-creature-sfx", "80 CC0 creature SFX (rubberduck)"),
    "rpg": ("80-cc0-rpg-sfx", "80 CC0 RPG SFX (rubberduck)"),
    "bfh": ("75-cc0-breaking-falling-hit-sfx", "75 CC0 breaking/falling/hit SFX (rubberduck)"),
    "misc": ("100-cc0-sfx", "100 CC0 SFX (rubberduck)"),
    "water": ("40-cc0-water-splash-slime-sfx", "40 CC0 water/splash/slime SFX (rubberduck)"),
    "bang": ("25-cc0-bang-firework-sfx", "25 CC0 bang/firework SFX (rubberduck)"),
    "sword": ("20-sword-sound-effects-attacks-and-clashes", "20 Sword Sound Effects (StarNinjas)"),
    "swish": ("swishes-sound-pack", "Swishes Sound Pack (artisticdude)"),
    "zombie": ("zombies-sound-pack", "Zombies Sound Pack (artisticdude)"),
    "moan": ("zombie-moans", "Zombie moans (darsycho)"),
    "gun": ("gunshots", "Gunshots"),
    "rain": ("rain-gutter-loop", "Rain in the Gutter Loop"),
    "fire": ("fireplace-sound-loop", "Fireplace Sound loop"),
    "steps": ("footsteps-leather-cloth-armor", "Footsteps Leather, Cloth, Armor"),
    "squish": ("squish-sounds-effects", "Squish Sounds Effects"),
    "boom": ("muffled-distant-explosion", "Muffled Distant Explosion (NenadSimic)"),
}

# game sound -> list of (pack, filename regex)
SOUNDS = {
    "swing": [("swish", r"swish-[1-6]\.wav$")],
    "swing_heavy": [("swish", r"swish-(7|8|9|10|11|12|13)\.wav$")],
    "hit_flesh": [("squish", r"squish_0[1-6].*\.mp3$")],
    "hit_heavy": [("squish", r"squishsplat_impact\.mp3$"), ("bfh", r"bfh1_hit_(10|11|12|13)\.ogg$")],
    "block": [("sword", r"sword_clash\.[1-5]\.ogg$"), ("bfh", r"bfh1_metal_hit_0[1-3]\.ogg$")],
    "parry": [("sword", r"sword_clash\.(6|7|8|9|10)\.ogg$")],
    "rupture": [("squish", r"squishpop\.mp3$"), ("water", r"slime_0[1-4]\.ogg$")],
    "step": [("steps", r"step_(cloth|lth)\d+\.ogg$")],
    "dodge": [("steps", r"step_cloth[1-4]\.ogg$")],
    "hurt": [("creature", r"grunt_0[1-5]\.ogg$")],
    "death": [("rpg", r"creature_die_01\.ogg$"), ("creature", r"hurt_0[1-5]\.ogg$")],
    "growl": [("zombie", r"zombies/zombie-\d+\.wav$"), ("creature", r"monster_0[1-7]\.ogg$")],
    "shriek": [("creature", r"(scream_0[12]|alien_0[1-3])\.ogg$")],
    "spit": [("creature", r"spit_0[1-3]\.ogg$")],
    "cast": [("rpg", r"spell_0[12]\.ogg$")],
    "fire": [("rpg", r"spell_fire_0[1-7]\.ogg$")],
    "explode": [("misc", r"x/explosion\.ogg$"), ("bang", r"(bang_0[1-6]|cannon_0[1-3])\.ogg$")],
    "rift": [("misc", r"weird_0[1-5]\.ogg$"), ("creature", r"alien_0[4-6]\.ogg$")],
    "pickup": [("rpg", r"item_(coins|gem)_0[1-4]\.ogg$")],
    "gunshot": [("gun", r"(22 Pistol|22 Magnum|Black Powder|Unkown)\.wav$"), ("bang", r"shot_0[1-3]\.ogg$")],
    "burn_pit": [("fire", r"fire\.wav$")],
    "boss_roar": [("creature", r"roar_0[1-3]\.ogg$"), ("rpg", r"creature_roar_0[1-3]\.ogg$")],
    "thunder": [("boom", r"Muffled Distant Explosion\.wav$"), ("bang", r"cannon_0[4-5]\.ogg$")],
    "bloom": [("water", r"slime_(0[5-9]|1[0-2])\.ogg$"), ("creature", r"burble_0[12]\.ogg$")],
    "larva": [("creature", r"bug_0[1-4]\.ogg$"), ("rpg", r"creature_slime_0[1-4]\.ogg$")],
    "heal": [("rpg", r"item_gem_0[1-4]\.ogg$")],
    "ambient_rain": [("rain", r"rain-gutter-loop.*\.mp3$")],
    "ambient_fire": [("fire", r"fire\.wav$")],
}


def oga_pack(key):
    page, _ = OGA[key]
    d = os.path.join(CACHE, "oga", page)
    html_path = fetch(f"https://opengameart.org/content/{page}", os.path.join(d, "page.html"))
    html = open(html_path, encoding="utf8", errors="replace").read()
    assert "license-name'>CC0" in html or 'license-name">CC0' in html, f"{page} is not CC0"
    urls = sorted(set(u for u in re.findall(r"https://opengameart.org/sites/default/files/[^\"' ]+\.(?:zip|wav|ogg|mp3)", html) if "audio_preview" not in u))
    files = []
    for u in urls:
        fn = urllib.parse.unquote(u.rsplit("/", 1)[1])
        p = fetch(u, os.path.join(d, fn))
        if fn.endswith(".zip"):
            x = os.path.join(d, "x")
            with zipfile.ZipFile(p) as zf:
                zf.extractall(x)
        else:
            files.append(p)
    for base, _, fs in os.walk(os.path.join(d, "x")):
        if "__MACOSX" in base:
            continue
        files += [os.path.join(base, f) for f in fs if f.lower().endswith((".wav", ".ogg", ".mp3"))]
    return files


def audio():
    out = os.path.join(OUT, "audio")
    os.makedirs(out, exist_ok=True)
    packs = {}
    used = set()
    for name, rules in SOUNDS.items():
        n = 0
        loop = name.startswith("ambient")
        for pack, rx in rules:
            if pack not in packs:
                packs[pack] = oga_pack(pack)
            for src in sorted(packs[pack]):
                rel = os.path.relpath(src, os.path.join(CACHE, "oga", OGA[pack][0]))
                if not re.search(rx, rel.replace("\\", "/")):
                    continue
                dst = os.path.join(out, f"{name}_{n}.wav")
                af = "loudnorm=I=-16:TP=-1.5,aresample=32000" if loop else "silenceremove=start_periods=1:start_threshold=-50dB,areverse,silenceremove=start_periods=1:start_threshold=-50dB,areverse,loudnorm=I=-14:TP=-1,aresample=32000"
                cmd = ["ffmpeg", "-v", "error", "-y", "-i", src, "-ac", "1", "-af", af, "-sample_fmt", "s16"]
                if loop:
                    cmd += ["-t", "24"]
                else:
                    cmd += ["-t", "4"]
                subprocess.run(cmd + [dst], check=True)
                used.add(pack)
                n += 1
        print("sound", name, n)
    for pack in sorted(used):
        page, title = OGA[pack]
        CREDITS.append(("audio", f"audio/* ({pack})", title, f"https://opengameart.org/content/{page}", "CC0 1.0"))


def write_credits():
    lines = ["# Third-party assets", "", "Every asset below is released under CC0 1.0 (public domain).", "Regenerate with `python3 tools/fetch_assets.py`.", "", "| Kind | Files | Source | Link | License |", "| --- | --- | --- | --- | --- |"]
    for kind, files, src, link, lic in CREDITS:
        lines.append(f"| {kind} | {files} | {src} | {link} | {lic} |")
    path = os.path.join(OUT, "CREDITS.md")
    # merge with existing credits for groups not regenerated this run
    old = []
    if os.path.exists(path):
        old = [l for l in open(path).read().splitlines() if l.startswith("| ") and not l.startswith("| Kind") and not l.startswith("| ---")]
    kinds = {c[0] for c in CREDITS}
    keep = [l for l in old if l.split("|")[1].strip() not in kinds]
    open(path, "w").write("\n".join(lines[:7] + keep + lines[7:]) + "\n")


if __name__ == "__main__":
    groups = sys.argv[1:] or ["textures", "models", "audio"]
    for gname in groups:
        {"textures": textures, "models": models, "audio": audio}[gname]()
    write_credits()
