"""Recipes for the custom infected: base model, seed and a function that reshapes, repaints and grows
parts on a Builder. Coordinates are glTF model space: +Y up, +Z forward (the face), +X the
character's left."""
import math

from mathutils import Vector

from build_enemies import FWD, LEFT, UP

# skin tests for the base models' atlas colours (the default test matches the green zombie skin)


def BLUE(c):
    return c[2] > c[0] * 1.3 and c[2] > 0.5


def YELLOW(c):
    return c[0] > 0.5 and c[1] > 0.4 and c[2] < 0.25


def FUR(c):
    grey = abs(c[0] - c[1]) < 0.05 and abs(c[1] - c[2]) < 0.05 and c[0] > 0.45
    brown = abs(c[0] - 0.38) < 0.07 and abs(c[1] - 0.25) < 0.07 and abs(c[2] - 0.12) < 0.07
    return grey or brown


DOG_GEAR = [(0.25, 0.25, 0.12), (0.62, 0.5, 0.12), (0.38, 0.25, 0.0), (0.62, 0.25, 0.12), (0.5, 0.38, 0.12)]

BONE_Y = (1.0, 0.9, 0.72)
GREEN_GLOW = (0.7, 2.4, 0.5)
PINK_GLOW = (2.4, 0.7, 1.4)
VIOLET_GLOW = (1.4, 0.8, 2.8)
EMBER_GLOW = (3.0, 1.2, 0.3)


# ------------------------------------------------------------------------------------------------
# shared features

def head_box(b):
    return b.region(b.subtree("Head"), 0.5)


def side(s):
    return "L" if s > 0 else "R"


def arm_dir(b, s):
    return (b.J(f"Middle1.{side(s)}") - b.J(f"LowerArm.{side(s)}")).normalized()


def claws(b, s, n=3, length=0.22, radius=0.025, color=BONE_Y):
    """Bone talons growing out of the fingers of one hand."""
    hand = b.J(f"Middle1.{side(s)}")
    d = arm_dir(b, s)
    for k, f in enumerate(["Index1", "Middle1", "Pinky1"][:n]):
        p = b.J(f"{f}.{side(s)}") if f"{f}.{side(s)}" in b.jpos else hand
        tip = d + FWD * (0.25 * (k - 1)) - UP * 0.15
        b.spike(p + d * 0.08, tip, length * b.rng.uniform(0.9, 1.15), radius, bend=-UP, bend_amt=0.3, anchor=p + d * 0.1, color=color)


def rib_burst(b, at, pairs=3, length=0.2, radius=0.022, spread=0.1):
    """The chest split open: a dark wound with ribs curling outwards."""
    s, n = b.surf(at, FWD)
    b.blob(s - n * 0.02, spread * 0.9, lobes=2, squash=(0.6, 1.4, 0.35), tex="Flesh", seed=7, color=(0.35, 0.05, 0.06), veins=0.3, rough=0.15)
    for k in range(pairs):
        y = (k - (pairs - 1) / 2) * spread * 0.7
        for sx in (-1, 1):
            base = s + UP * y + LEFT * sx * spread * 0.25
            b.spike(base, n * 0.8 + LEFT * sx * 0.9 + UP * 0.1, length * (1 - abs(k - 1) * 0.15), radius, bend=-LEFT * sx, bend_amt=-0.25, color=BONE_Y)


def face_tendrils(b, n=6, length=0.35, r=0.025, tex="Flesh", color=(0.8, 0.55, 0.6)):
    c, lo, hi = head_box(b)
    for k in range(n):
        a = (k / (n - 1) - 0.5) * 2.2
        root, nrm = b.surf(Vector((math.sin(a) * 0.1, lo.y + (hi.y - lo.y) * 0.4, c.z)), FWD + LEFT * math.sin(a) * 0.6)
        ln = length * b.rng.uniform(0.75, 1.2)
        pts = [root, root + nrm * 0.05, root + nrm * 0.08 - UP * ln * 0.4 + LEFT * math.sin(a) * 0.03, root + nrm * 0.06 - UP * ln * 0.8, root + nrm * 0.1 - UP * ln]
        b.tentacle(pts, r, r * 0.25, tex=tex, sides=6, color=color, anchor=c)


def sac_cluster(b, at, out, n, size, color=GREEN_GLOW, glow=0.75, seed=0, tex="GrowthGlow"):
    s, nrm = b.surf(at, out)
    for k in range(n):
        d = (nrm + Vector((b.rng.uniform(-0.7, 0.7), b.rng.uniform(-0.5, 0.7), b.rng.uniform(-0.5, 0.5)))).normalized()
        r = size * b.rng.uniform(0.6, 1.1)
        b.blob(s + d * r * 0.6, r, lobes=2, tex=tex, seed=seed + k, mode="anchor", color=color, glow=glow, veins=0.9, rough=0.12, anchor=s)
    # a fleshy collar holding them
    b.blob(s, size * 0.9, lobes=4, tex="Flesh", seed=seed + 50, color=(0.8, 0.5, 0.55), veins=0.8)


def tumour(b, at, out, size, seed=0, pustule_col=(2.0, 1.8, 0.4), n_pust=5, color=(1.0, 0.72, 0.72)):
    s, nrm = b.surf(at, out)
    b.blob(s + nrm * size * 0.2, size, lobes=5, seed=seed, color=color, rough=0.35)
    b.pustules(s + nrm * size * 0.2, size * 0.95, n_pust, size * 0.22, seed=seed + 1, color=pustule_col, normal=nrm)


def petals(b, centre, n, length, width, up=UP, color=(0.9, 0.5, 0.55), tex="Flesh", open_=0.9, glow_core=PINK_GLOW, core=0.09):
    up = Vector(up).normalized()
    x = up.cross(LEFT if abs(up.dot(LEFT)) < 0.9 else FWD).normalized()
    y = up.cross(x)
    for k in range(n):
        a = k / n * math.tau
        rd = x * math.cos(a) + y * math.sin(a)
        b.petal(centre + rd * core * 0.6, up * (1 - open_ * 0.6) + rd * open_, length * b.rng.uniform(0.85, 1.1), width, curl=-0.9, tex=tex, color=color, anchor=centre)
    if glow_core:
        b.blob(centre, core, lobes=3, tex="GrowthGlow", seed=9, mode="anchor", color=glow_core, glow=0.85, veins=0.5, rough=0.1, anchor=centre)


def crystal_cluster(b, at, out, n, length, radius, color=VIOLET_GLOW, spread=0.6, glow=0.75):
    s, nrm = b.surf(at, out)
    for k in range(n):
        d = (nrm + Vector((b.rng.uniform(-spread, spread), b.rng.uniform(-spread, spread) + 0.2, b.rng.uniform(-spread, spread)))).normalized()
        b.crystal(s - nrm * 0.02, d, length * b.rng.uniform(0.6, 1.1), radius * b.rng.uniform(0.7, 1.2), color=color, glow=glow, anchor=s)


def tail(b, root_bone, length, r, segs=5, tex="Flesh", color=(0.85, 0.55, 0.6), down=0.4):
    root = b.J(root_bone)
    s, _ = b.surf(root, -FWD + UP * 0.2)
    pts = [s + (-FWD * (k / segs) * length) + UP * (-down * (k / segs) ** 2 * length) + LEFT * 0.06 * math.sin(k * 1.3) for k in range(segs + 1)]
    b.tentacle(pts, r, r * 0.15, tex=tex, color=color, anchor=s)


def mandibles(b, centre, fwd, length, r, spread=0.1):
    for sx in (-1, 1):
        b.spike(centre + LEFT * sx * spread, fwd + LEFT * sx * 0.45 - UP * 0.3, length, r, bend=-LEFT * sx, bend_amt=0.4, anchor=centre, color=BONE_Y)


# ------------------------------------------------------------------------------------------------
# archetypes

def carrier(b):
    """Rusher: an emaciated carrier, jaw split into mandibles, ribs and spine breaking out."""
    b.paint(skin=(0.72, 0.8, 0.92), veins=0.8, bruise=0.5, blood=[("Head", 0.24), ("Middle1.L", 0.12), ("Middle1.R", 0.16)], gore=0.3, seed=1)
    b.scale_about(["Torso", "Abdomen"], b.J("Torso"), (0.9, 1.0, 0.85))
    b.scale_about(b.subtree("Head"), b.J("Neck"), (0.78, 0.78, 0.78))
    c, lo, hi = head_box(b)
    mandibles(b, Vector((0, lo.y + (hi.y - lo.y) * 0.22, hi.z - 0.05)), FWD, 0.24, 0.032, spread=0.07)
    b.spine_row(["Hips", "Abdomen", "Torso", "Neck"], 7, 0.2, 0.035, out=-1.0)
    rib_burst(b, b.J("Torso") + UP * 0.02, pairs=3, length=0.16)
    claws(b, 1)
    claws(b, -1)
    tumour(b, b.J("Shoulder.L") + UP * 0.05, UP + LEFT * 0.5 - FWD * 0.3, 0.08, seed=4)


def shambler(b):
    """Rusher variant: one arm swollen into a calcified club of tumours and bone."""
    b.reskin(YELLOW)
    b.paint(skin=(0.7, 0.82, 0.86), veins=0.9, bruise=0.6, blood=[("Head", 0.2), ("Middle1.L", 0.14)], gore=0.45, seed=2)
    arm = b.subtree("UpperArm.R")
    b.inflate(arm, 0.035, lumpy=0.8, seed=3)
    d = arm_dir(b, -1)
    lo = b.J("LowerArm.R")
    for k in range(4):
        p = lo + d * (0.1 + k * 0.12)
        tumour(b, p, UP * (0.7 if k % 2 else 0.2) - FWD * 0.6 + (FWD * 0.6 if k % 2 else Vector()), 0.07 + 0.015 * k, seed=10 + k, n_pust=3)
    for k in range(5):
        p = lo + d * (0.05 + k * 0.1)
        s, n = b.surf(p, UP + FWD * (0.3 * (k % 3 - 1)))
        b.spike(s, n + d * 0.3, 0.14 + 0.04 * (k % 2), 0.03)
    claws(b, 1, length=0.18)
    b.spine_row(["Hips", "Abdomen", "Torso", "Neck"], 6, 0.17, 0.03)
    c, lo2, hi = head_box(b)
    for k in range(3):
        a = (k - 1) * 0.9
        s, n = b.surf(c + UP * 0.05, UP + LEFT * math.sin(a) * 0.8 - FWD * 0.2)
        b.tentacle([s, s + n * 0.08, s + n * 0.16 + LEFT * 0.03 * k, s + n * 0.22 - FWD * 0.04], 0.022, 0.006, color=(0.85, 0.6, 0.6), anchor=c)
        b.blob(s + n * 0.23 - FWD * 0.04, 0.025, lobes=1, tex="GrowthGlow", seed=k, mode="anchor", color=GREEN_GLOW, glow=0.8, veins=0.0, rough=0.05, anchor=c)


def bulk(b):
    """Brute: a bloated mass; right arm a tumorous hammer, calcified plates over the left side."""
    b.reskin(BLUE)
    b.paint(skin=(0.78, 0.74, 0.86), veins=0.8, bruise=0.7, blood=[("Middle1.R", 0.2), ("TopMouth", 0.25)], gore=0.35, seed=3)
    b.inflate(b.subtree("UpperArm.R"), 0.06, lumpy=0.9, seed=1)
    b.inflate(["Torso", "Abdomen"], 0.03, lumpy=0.6, seed=2)
    tumour(b, b.J("UpperArm.R"), UP - FWD * 0.3, 0.16, seed=20, pustule_col=(2.2, 1.9, 0.4), n_pust=7)
    tumour(b, b.J("LowerArm.R") + arm_dir(b, -1) * 0.12, -FWD + UP * 0.3, 0.12, seed=22, n_pust=4)
    hand = b.J("Middle1.R")
    for k in range(4):
        s, n = b.surf(hand + arm_dir(b, -1) * 0.04 + FWD * (k - 1.5) * 0.04, UP + FWD * 0.2)
        b.spike(s, n + arm_dir(b, -1) * 0.5, 0.12, 0.028)
    for at, out, up in [(b.J("UpperArm.L"), UP + LEFT * 0.4, -FWD), (b.J("Torso") + LEFT * 0.12, -FWD + LEFT * 0.4, UP), (b.J("Torso") - LEFT * 0.08 + UP * 0.12, -FWD, UP)]:
        s, n = b.surf(at, out)
        b.plate(s + n * 0.01, n, up, 0.16, 0.13, curve=0.9, thick=0.035, ridges=4, tex="Bone", color=(0.92, 0.86, 0.75))
    b.spine_row(["Abdomen", "Torso", "Neck"], 5, 0.16, 0.05, lean=0.6)
    claws(b, 1, length=0.15, radius=0.03)


def retch(b):
    """Spitter: belly and throat swollen with acid; glowing sacs crowd its back."""
    b.reskin(BLUE)
    b.paint(skin=(0.75, 0.85, 0.72), veins=0.9, bruise=0.4, blood=[("TopMouth", 0.15)], glow_veins=0.3, glow_col=(0.5, 1.6, 0.3), seed=4)
    b.inflate(["Abdomen"], 0.07, lumpy=0.7, seed=4)
    for k in range(3):
        sac_cluster(b, b.J("Torso") + LEFT * (k - 1) * 0.16 + UP * (0.05 * k), -FWD + UP * 0.4 + LEFT * (k - 1) * 0.4, 3, 0.1 - 0.015 * abs(k - 1), seed=30 + k * 5)
    s, n = b.surf(b.J("TopMouth") - UP * 0.12, FWD)
    b.tentacle([s, s + n * 0.08, s + n * 0.16 - UP * 0.05, s + n * 0.22 - UP * 0.14], 0.05, 0.03, color=(0.9, 0.55, 0.55), anchor=b.J("TopMouth"))
    b.blob(s + n * 0.22 - UP * 0.16, 0.035, lobes=1, tex="GrowthGlow", seed=3, mode="anchor", color=GREEN_GLOW, glow=0.9, veins=0, anchor=b.J("TopMouth"))
    for k in range(5):
        a = k * 1.3
        tumour(b, b.J("Abdomen") + LEFT * math.sin(a) * 0.2 + UP * math.cos(a) * 0.1, FWD + LEFT * math.sin(a) * 0.5, 0.035, seed=40 + k, pustule_col=GREEN_GLOW, n_pust=2)


def larva(b):
    """Crawler swarm: a pair of legs carrying a segmented grub that ends in a lamprey mouth."""
    b.drop([(0.88, 0.75, 0.5)])
    b.paint(skin=(0.8, 0.75, 0.8), veins=0.8, bruise=0.6, gore=0.5, seed=5)
    base = b.J("Abdomen") + UP * 0.02
    pts = []
    for k in range(6):
        t = k / 5
        pts.append(base + UP * (0.06 + t * 0.32) + FWD * (t * t * 0.32))
    for k, p in enumerate(pts):
        r = 0.13 * (1 - k * 0.1)
        b.blob(p, r, lobes=2, squash=(1.0, 0.75, 1.0), tex="Growth", seed=50 + k, mode="anchor", color=(1.0, 0.85, 0.8), veins=1.0, rough=0.15, anchor=base)
    tip = pts[-1] + FWD * 0.1 + UP * 0.02
    b.maw(tip, FWD + UP * 0.3, 0.08, teeth=9, anchor=base)
    for k in range(4):
        p = pts[k + 1]
        for sx in (-1, 1):
            b.spike(p + LEFT * sx * 0.1, LEFT * sx - UP * 0.4 + FWD * 0.2, 0.14, 0.018, bend=-UP, bend_amt=0.4, anchor=base)


def bulwark(b):
    """Shielded: chitin has grown into a carapace shield over the left forearm, and a helm."""
    b.paint(skin=(0.7, 0.74, 0.86), veins=0.6, bruise=0.5, blood=[("Middle1.R", 0.12)], seed=6)
    d = arm_dir(b, 1)
    centre = b.J("LowerArm.L") + d * 0.14 + FWD * 0.12
    b.plate(centre, FWD + d * 0.1, -d * 0.3 + UP, 0.24, 0.3, curve=2.6, thick=0.035, ridges=7, taper=0.35, tex="Chitin", color=(0.75, 0.68, 0.62), mode="anchor", anchor=b.J("LowerArm.L") + d * 0.1, segs=(10, 12))
    for k in range(3):
        b.spike(centre + FWD * 0.04 + UP * (k - 1) * 0.12, FWD + UP * 0.2, 0.1, 0.025, tex="Chitin", color=(0.7, 0.62, 0.55), anchor=b.J("LowerArm.L") + d * 0.1)
    c, lo, hi = head_box(b)
    s, n = b.surf(c + UP * 0.1, UP + FWD * 0.2)
    b.plate(s + n * 0.01, n, FWD, 0.17, 0.16, curve=1.2, thick=0.03, ridges=5, tex="Chitin", color=(0.7, 0.62, 0.58))
    for sx in (-1, 1):
        s, n = b.surf(b.J(f"UpperArm.{side(sx)}"), UP + LEFT * sx * 0.5)
        b.plate(s + n * 0.01, n, LEFT * sx, 0.12, 0.1, curve=1.0, thick=0.03, ridges=3, tex="Chitin", color=(0.72, 0.64, 0.58))
    s, n = b.surf(b.J("Torso"), -FWD)
    b.plate(s + n * 0.01, n, UP, 0.17, 0.2, curve=0.7, thick=0.035, ridges=6, tex="Chitin", color=(0.7, 0.62, 0.58))
    claws(b, -1, length=0.16)


def lurker(b):
    """Ambusher: flayed, eyeless, its face a curtain of feelers; one forearm a bone scythe."""
    b.reskin(YELLOW)
    b.paint(skin=(0.8, 0.62, 0.62), skin_tex="Flesh", veins=0.6, bruise=0.3, blood=[("Middle1.R", 0.3)], seed=7)
    face_tendrils(b, n=7, length=0.4, r=0.022)
    d = arm_dir(b, -1)
    hand = b.J("Middle1.R")
    b.spike(hand - d * 0.15, d * 0.4 + FWD * 0.6 - UP * 0.2, 0.85, 0.06, bend=-UP + FWD * 0.3, bend_amt=0.35, sides=6, anchor=hand - d * 0.1, color=(1.0, 0.92, 0.78))
    for k in range(3):
        s, n = b.surf(b.J("LowerArm.R") + d * (k * 0.12), UP - FWD * 0.3)
        b.spike(s, n + d * 0.6, 0.1, 0.022, anchor=b.J("LowerArm.R") + d * 0.2)
    b.spine_row(["Hips", "Abdomen", "Torso", "Neck"], 8, 0.24, 0.03, lean=0.7)
    claws(b, 1, length=0.24, radius=0.02)


def matron(b):
    """Summoner: its head opened into a fleshy bloom; egg sacs cluster on its back."""
    b.reskin(BLUE)
    b.paint(skin=(0.86, 0.72, 0.8), veins=0.9, bruise=0.5, glow_veins=0.25, glow_col=(1.6, 0.5, 1.0), seed=8)
    b.inflate(["Abdomen"], 0.05, lumpy=0.5, seed=8)
    c, lo, hi = head_box(b)
    top = Vector((0, hi.y - 0.04, c.z - 0.02))
    petals(b, top, 7, 0.3, 0.09, up=UP - FWD * 0.15)
    for k in range(4):
        sx = (k % 2) * 2 - 1
        sac_cluster(b, b.J("Torso") + LEFT * sx * 0.14 + UP * (0.1 * (k // 2) - 0.05), -FWD + LEFT * sx * 0.3, 3, 0.075, color=PINK_GLOW, glow=0.7, seed=60 + k * 4)
    for sx in (-1, 1):
        s, n = b.surf(b.J(f"Shoulder.{side(sx)}") + UP * 0.05, UP + LEFT * sx * 0.3 - FWD * 0.3)
        b.tentacle([s, s + n * 0.1, s + n * 0.12 - UP * 0.2 + LEFT * sx * 0.1, s - UP * 0.45 + LEFT * sx * 0.15], 0.035, 0.008, color=(0.85, 0.55, 0.65))
    s, n = b.surf(b.J("Abdomen"), FWD)
    b.blob(s + n * 0.02, 0.12, lobes=3, squash=(1.0, 0.8, 0.6), tex="GrowthGlow", seed=66, color=PINK_GLOW, glow=0.45, veins=1.0)


def skitter(b):
    """Leaper: a flayed hound, head split into mandibles, ribs and spine through the skin."""
    b.reskin(FUR)
    b.drop(DOG_GEAR)
    b.paint(skin=(0.85, 0.7, 0.72), skin_tex="Flesh", veins=0.5, bruise=0.3, blood=[("Head", 0.2)], seed=9)
    head = b.J("Head")
    snout, n = b.surf(head + FWD * 0.15, FWD)
    mandibles(b, snout - FWD * 0.08, FWD, 0.2, 0.026, spread=0.07)
    for sx in (-1, 1):
        s, nn = b.surf(head + UP * 0.05, UP + LEFT * sx * 0.6)
        b.spike(s, nn - FWD * 0.6, 0.14, 0.022, bend=-FWD, bend_amt=0.3)
    spine = [b.J(n) for n in ("Tail1", "Back", "Torso", "Torso2", "Torso3", "Neck1", "Neck2", "Neck3")]
    for k in range(9):
        t = k / 8
        f = t * (len(spine) - 1)
        i = min(int(f), len(spine) - 2)
        p = spine[i].lerp(spine[i + 1], f - i)
        s, nn = b.surf(p, UP)
        b.spike(s - nn * 0.01, nn - FWD * 0.5, 0.12 + 0.08 * math.sin(t * math.pi), 0.024, bend=-FWD, bend_amt=0.3)
    for k in range(3):
        for sx in (-1, 1):
            p = b.J("Torso").lerp(b.J("Torso3"), 0.2 + k * 0.3)
            s, nn = b.surf(p, LEFT * sx + UP * 0.2)
            b.spike(s, nn + UP * 0.6, 0.12, 0.018, bend=-nn, bend_amt=0.4, color=BONE_Y)
    for sx in (-1, 1):
        s, nn = b.surf(b.J("Torso3") + UP * 0.05, UP + LEFT * sx)
        b.tentacle([s, s + nn * 0.1, s + nn * 0.15 - FWD * 0.15, s + nn * 0.12 - FWD * 0.3 - UP * 0.05], 0.025, 0.006, color=(0.85, 0.5, 0.55))


# ------------------------------------------------------------------------------------------------
# bosses

def digestor(b):
    """Rift of the Gut: a walking stomach, a toothed maw split across the belly."""
    b.reskin(BLUE)
    b.paint(skin=(0.9, 0.62, 0.62), skin_tex="Flesh", veins=0.9, bruise=0.5, blood=[("TopMouth", 0.3)], gore=0.5, seed=10)
    b.inflate(["Abdomen", "Torso", "Hips"], 0.08, lumpy=0.8, seed=10)
    s, n = b.surf(b.J("Abdomen") + UP * 0.05, FWD)
    b.maw(s + n * 0.02, n + UP * 0.1, 0.17, teeth=14, tooth_len=0.13)
    for k in range(3):
        tumour(b, b.J("Torso") + LEFT * (k - 1) * 0.2, -FWD + UP * 0.3 + LEFT * (k - 1) * 0.5, 0.12, seed=70 + k, pustule_col=(2.0, 1.6, 0.4))
    s, n = b.surf(b.J("Shoulder.L") + UP * 0.05, UP + LEFT)
    b.tentacle([s, s + n * 0.15, s + n * 0.25 + LEFT * 0.2 - UP * 0.1, s + LEFT * 0.55 - UP * 0.35, s + LEFT * 0.7 - UP * 0.7], 0.07, 0.015, color=(0.9, 0.5, 0.55))
    mandibles(b, b.J("TopMouth") + FWD * 0.05 - UP * 0.05, FWD, 0.22, 0.035, spread=0.12)
    tail(b, "Hips", 0.8, 0.08)
    sac_cluster(b, b.J("Torso") + UP * 0.15, UP - FWD * 0.5, 3, 0.09, color=(2.0, 0.6, 0.6), seed=75)


def geometer(b):
    """Rift of the Lattice: stone-grey flesh threaded with violet light, crystals bursting out."""
    b.paint(skin=(0.7, 0.68, 0.8), skin_tex="RiftStone", veins=0.3, bruise=0.2, glow_veins=0.6, glow_col=(1.0, 0.6, 2.0), seed=11)
    c, lo, hi = head_box(b)
    crystal_cluster(b, c + UP * 0.12, UP, 7, 0.36, 0.045, spread=0.5)
    for sx in (-1, 1):
        crystal_cluster(b, b.J(f"UpperArm.{side(sx)}"), UP + LEFT * sx * 0.3, 4, 0.25, 0.04)
        d = arm_dir(b, sx)
        hand = b.J(f"Middle1.{side(sx)}")
        b.crystal(hand - d * 0.2, d + FWD * 0.15, 0.7, 0.05, anchor=hand - d * 0.1)
    for k in range(5):
        crystal_cluster(b, b.J("Torso").lerp(b.J("Hips"), k / 4) + LEFT * ((k % 2) - 0.5) * 0.1, -FWD + UP * 0.3, 2, 0.3, 0.05, spread=0.3)
    s, n = b.surf(b.J("Torso") + UP * 0.03, FWD)
    for k in range(3):
        b.blob(s + LEFT * (k - 1) * 0.07 + UP * (0.03 if k == 1 else 0), 0.03, lobes=1, tex="RiftGlow", seed=k, mode="anchor", color=(1.6, 1.2, 2.8), glow=0.9, veins=0, anchor=s)


def mother_cinder(b):
    """Rift of the Kiln: charred, cracked with embers, a burning bloom where the head was."""
    b.reskin(BLUE)
    b.paint(skin=(0.6, 0.55, 0.55), skin_tex="Char", char=0.6, ember=0.9, veins=0.2, seed=12)
    c, lo, hi = head_box(b)
    top = Vector((0, hi.y - 0.04, c.z))
    petals(b, top, 8, 0.32, 0.1, color=(0.35, 0.3, 0.3), tex="Char", glow_core=EMBER_GLOW, core=0.11)
    for k in range(3):
        sac_cluster(b, b.J("Torso") + LEFT * (k - 1) * 0.15, -FWD + UP * 0.3, 3, 0.08, color=EMBER_GLOW, glow=0.85, seed=80 + k * 4, tex="Ember")
    for sx in (-1, 1):
        s, n = b.surf(c + UP * 0.05 + LEFT * sx * 0.1, UP + LEFT * sx)
        b.spike(s, n + UP, 0.32, 0.035, tex="Char", color=(0.4, 0.36, 0.35), bend=LEFT * sx, bend_amt=0.4)
    d = arm_dir(b, -1)
    lo_ = b.J("LowerArm.R")
    b.pipe(lo_ + d * 0.05, d, 0.42, 0.075, tex="Char", color=(0.45, 0.4, 0.38), anchor=lo_ + d * 0.15)
    b.blob(lo_ + d * 0.47, 0.06, lobes=2, tex="Ember", seed=5, mode="anchor", color=EMBER_GLOW, glow=0.95, veins=0.3, anchor=lo_ + d * 0.15)
    claws(b, 1, length=0.2, radius=0.03, color=(0.35, 0.32, 0.3))


def cantor(b):
    """Rift of the Choir: organ pipes of bone rise from its back; mouths sing along its chest."""
    b.paint(skin=(0.88, 0.84, 0.76), skin_tex="Skin", veins=0.7, bruise=0.3, blood=[("Head", 0.2)], seed=13)
    for k in range(6):
        x = (k - 2.5) * 0.06
        h = 0.35 + 0.18 * (1 - abs(k - 2.5) / 2.5)
        s, n = b.surf(b.J("Torso") + LEFT * x, -FWD + UP * 0.4)
        b.pipe(s - n * 0.02, UP + n * 0.15, h, 0.03, color=(0.95, 0.9, 0.78))
    for k in range(3):
        s, n = b.surf(b.J("Torso").lerp(b.J("Abdomen"), k / 2) + LEFT * (k - 1) * 0.07, FWD)
        b.maw(s + n * 0.01, n, 0.04, teeth=7, tooth_len=0.03)
    c, lo, hi = head_box(b)
    b.tentacle([c, c + UP * 0.15, c + UP * 0.3 + FWD * 0.08, c + UP * 0.4 + FWD * 0.18], 0.06, 0.04, color=(0.9, 0.82, 0.75), anchor=c)
    b.maw(c + UP * 0.42 + FWD * 0.22, FWD + UP * 0.4, 0.06, teeth=8, anchor=c)
    mandibles(b, Vector((0, lo.y + 0.06, hi.z - 0.04)), FWD, 0.2, 0.028, spread=0.07)
    for sx in (-1, 1):
        s, n = b.surf(b.J(f"Shoulder.{side(sx)}"), -FWD + LEFT * sx * 0.5)
        b.petal(s, n + UP * 0.5, 0.35, 0.1, curl=0.5, tex="Vein", color=(0.9, 0.7, 0.8), anchor=s)


def seed(b):
    """The crater's heart: a mass of heads, horns, crystals and sacs."""
    b.reskin(YELLOW)
    b.paint(skin=(0.82, 0.62, 0.66), skin_tex="Flesh", veins=0.8, bruise=0.5, gore=0.4, glow_veins=0.35, glow_col=(0.6, 1.8, 0.9), seed=14)
    c, lo, hi = head_box(b)
    for k in range(4):
        a = (k - 1.5) * 0.7
        s, n = b.surf(b.J("Neck") + LEFT * math.sin(a) * 0.25 + UP * 0.05, UP + LEFT * math.sin(a) - FWD * 0.2)
        hc = s + n * 0.12
        b.blob(hc, 0.1, lobes=3, seed=90 + k, color=(0.9, 0.62, 0.62), anchor=hc)
        b.maw(hc + FWD * 0.08, FWD + n * 0.3, 0.05, teeth=7, anchor=hc)
    for sx in (-1, 1):
        s, n = b.surf(c + UP * 0.1 + LEFT * sx * 0.08, UP + LEFT * sx)
        b.spike(s, n + UP, 0.45, 0.05, bend=LEFT * sx, bend_amt=0.5)
    crystal_cluster(b, b.J("Torso"), -FWD + UP * 0.4, 6, 0.4, 0.05, color=(0.8, 0.8, 2.2))
    sac_cluster(b, b.J("Abdomen") + UP * 0.05, FWD, 4, 0.08, seed=95)
    d = arm_dir(b, -1)
    b.spike(b.J("Middle1.R") - d * 0.1, d + FWD * 0.3, 0.6, 0.06)
    s, n = b.surf(b.J("Shoulder.L"), UP + LEFT)
    b.tentacle([s, s + n * 0.15, s + n * 0.2 + LEFT * 0.25, s + LEFT * 0.5 - UP * 0.4, s + LEFT * 0.6 - UP * 0.75], 0.06, 0.012, color=(0.85, 0.5, 0.55))
    tail(b, "Hips", 0.9, 0.08)


RECIPES = {
    "Strain_Carrier": ("Zombie_Basic", 11, carrier),
    "Strain_Shambler": ("Zombie_Arm", 12, shambler),
    "Strain_Bulk": ("Zombie_Chubby", 13, bulk),
    "Strain_Retch": ("Zombie_Chubby", 14, retch),
    "Strain_Larva": ("Zombie_Ribcage", 15, larva),
    "Strain_Bulwark": ("Zombie_Basic", 16, bulwark),
    "Strain_Lurker": ("Zombie_Arm", 17, lurker),
    "Strain_Matron": ("Zombie_Chubby", 18, matron),
    "Strain_Skitter": ("Characters_GermanShepherd", 19, skitter),
    "Strain_Digestor": ("Zombie_Chubby", 20, digestor),
    "Strain_Geometer": ("Zombie_Basic", 21, geometer),
    "Strain_MotherCinder": ("Zombie_Chubby", 22, mother_cinder),
    "Strain_Cantor": ("Zombie_Basic", 23, cantor),
    "Strain_Seed": ("Zombie_Arm", 24, seed),
}
