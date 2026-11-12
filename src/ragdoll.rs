//! Verlet ragdolls built from a model's own skeleton.
//!
//! Every deforming joint becomes a point mass. Bones are distance constraints; the torso (hips,
//! spine, shoulders, neck, upper legs) is held as one rigid block; fingers, ears and mouth parts are
//! braced to their parents; limbs get a minimum-distance constraint to their grandparent, a coarse
//! joint limit that stops knees and elbows folding through themselves. Joint orientations are
//! rebuilt every step from the directions to their children, so the skinned mesh follows.
use crate::render::model::Skeleton;
use glam::{Mat3, Mat4, Quat, Vec3};

#[derive(Clone, Copy, Debug)]
struct Link {
    a: usize,
    b: usize,
    rest: f32,
    /// only push apart (a joint limit), never pull together
    min_only: bool,
}

#[derive(Clone, Debug)]
pub struct Ragdoll {
    pts: Vec<Vec3>,
    prev: Vec<Vec3>,
    sim: Vec<bool>,
    parent: Vec<Option<usize>>,
    links: Vec<Link>,
    /// per joint: (primary child, secondary reference joint) for rebuilding orientation
    frame: Vec<(Option<usize>, Option<usize>)>,
    init_pts: Vec<Vec3>,
    /// world-space joint matrices at the moment of death (translation removed)
    init_basis: Vec<Mat4>,
    init_global: Vec<Mat4>,
    radius: Vec<f32>,
    pub t: f32,
    pub asleep: bool,
    quiet: f32,
}

/// IK foot bones in the kit rigs hang off the root, not the shin: re-parent them for physics.
fn parent_override(name: &str) -> Option<&'static str> {
    Some(match name {
        "Foot.L" => "LowerLeg.L",
        "Foot.R" => "LowerLeg.R",
        "FF.L" => "FrontLowerLeg.L",
        "FF.R" => "FrontLowerLeg.R",
        "FFB.L" => "BackLowerLeg.L",
        "FFB.R" => "BackLowerLeg.R",
        _ => return None,
    })
}
fn is_core(name: &str) -> bool {
    ["Hips", "Abdomen", "Torso", "Neck", "Shoulder", "UpperLeg", "Body", "Back", "UpperArm"].iter().any(|k| name.starts_with(k)) && !name.starts_with("UpperArm") || name == "Body"
}
fn is_small(name: &str) -> bool {
    ["Pinky", "Middle", "Index", "Thumb", "Ear", "Tongue", "Eyelid", "TopMouth", "BottomMouth"].iter().any(|k| name.starts_with(k))
}

/// The physics parent used for a joint: an override, else the nearest simulated ancestor.
pub fn physics_parent(skel: &Skeleton, j: usize, sim: &[bool]) -> Option<usize> {
    if let Some(p) = parent_override(&skel.joints[j].name).and_then(|n| skel.find(n)) {
        return Some(p);
    }
    let mut p = skel.joints[j].parent;
    while let Some(q) = p {
        if sim[q] {
            return Some(q);
        }
        p = skel.joints[q].parent;
    }
    None
}

/// Joints that move skin (have vertex weight), plus their ancestors.
pub fn deforming(skel: &Skeleton, weights: impl Iterator<Item = ([u16; 4], [f32; 4])>) -> Vec<bool> {
    let mut d = vec![false; skel.joints.len()];
    for (j, w) in weights {
        for k in 0..4 {
            if w[k] > 0.05 && (j[k] as usize) < d.len() {
                d[j[k] as usize] = true;
            }
        }
    }
    d
}

fn basis(a: Vec3, b: Vec3) -> Option<Mat3> {
    let x = a.normalize_or_zero();
    let z = a.cross(b).normalize_or_zero();
    if x == Vec3::ZERO || z == Vec3::ZERO {
        return None;
    }
    let y = z.cross(x);
    Some(Mat3::from_cols(x, y, z))
}

impl Ragdoll {
    /// `globals` are world-space joint matrices at death; `vel` is the body's velocity and
    /// `impulse` the killing blow (direction * speed), strongest near `hit`.
    pub fn new(skel: &Skeleton, globals: &[Mat4], deform: &[bool], vel: Vec3, impulse: Vec3, hit: Vec3, scale: f32) -> Ragdoll {
        let n = skel.joints.len();
        let sim: Vec<bool> = (0..n).map(|j| deform[j] && !skel.joints[j].name.starts_with("PoleTarget") && !skel.joints[j].name.starts_with("IK")).collect();
        let parent: Vec<Option<usize>> = (0..n).map(|j| if sim[j] { physics_parent(skel, j, &sim) } else { None }).collect();
        let pts: Vec<Vec3> = globals.iter().map(|g| g.w_axis.truncate()).collect();
        let dt = 1.0 / 120.0;
        let prev: Vec<Vec3> = pts
            .iter()
            .map(|p| {
                let near = (-(*p - hit).length_squared() / (0.5 * scale * scale).max(0.05)).exp();
                let v = vel + impulse * (0.35 + 0.65 * near);
                *p - v * dt
            })
            .collect();
        let mut links = Vec::new();
        let dist = |a: usize, b: usize| (pts[a] - pts[b]).length();
        let name = |j: usize| skel.joints[j].name.as_str();
        for j in 0..n {
            let Some(p) = parent[j] else { continue };
            links.push(Link { a: j, b: p, rest: dist(j, p), min_only: false });
            if let Some(g) = parent[p] {
                let rigid = is_small(name(j)) || is_small(name(p)) || (is_core(name(j)) && is_core(name(p)) && is_core(name(g)));
                if rigid {
                    links.push(Link { a: j, b: g, rest: dist(j, g), min_only: false });
                } else {
                    // a limb may bend to roughly 110 degrees but not fold through itself
                    let full = dist(j, p) + dist(p, g);
                    links.push(Link { a: j, b: g, rest: full * 0.55, min_only: true });
                }
            }
        }
        // rigid torso block
        let core: Vec<usize> = (0..n).filter(|&j| sim[j] && is_core(name(j))).collect();
        for (i, &a) in core.iter().enumerate() {
            for &b in &core[i + 1..] {
                links.push(Link { a, b, rest: dist(a, b), min_only: false });
            }
        }
        // orientation frames
        let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
        for j in 0..n {
            if let Some(p) = parent[j] {
                kids[p].push(j);
            }
        }
        let size = |j: usize| -> usize {
            let mut c = 0;
            let mut st = vec![j];
            while let Some(k) = st.pop() {
                c += 1;
                st.extend(kids[k].iter().copied());
            }
            c
        };
        let frame = (0..n)
            .map(|j| {
                let mut ks = kids[j].clone();
                ks.sort_by_key(|&k| std::cmp::Reverse((size(k), (dist(j, k) * 1000.0) as i64)));
                let primary = ks.first().copied();
                let secondary = ks.get(1).copied().or(parent[j]);
                (primary, secondary)
            })
            .collect();
        let radius = (0..n).map(|j| if name(j) == "Head" { 0.11 } else if is_core(name(j)) { 0.12 } else { 0.05 } * scale).collect();
        let init_basis = globals.iter().map(|g| Mat4::from_cols(g.x_axis, g.y_axis, g.z_axis, glam::Vec4::W)).collect();
        Ragdoll { pts: pts.clone(), prev, sim, parent, links, frame, init_pts: pts, init_basis, init_global: globals.to_vec(), radius, t: 0.0, asleep: false, quiet: 0.0 }
    }

    /// Advance the simulation. `collide` pushes a point (with radius) out of the world and
    /// returns the corrected position and whether it touched something.
    pub fn step(&mut self, dt: f32, collide: &dyn Fn(Vec3, f32) -> (Vec3, bool)) {
        if self.asleep || dt <= 0.0 {
            return;
        }
        self.t += dt;
        let sub = ((dt / (1.0 / 120.0)).ceil() as usize).clamp(1, 4);
        let h = dt / sub as f32;
        let mut moved = 0.0f32;
        for _ in 0..sub {
            for j in 0..self.pts.len() {
                if !self.sim[j] {
                    continue;
                }
                let v = (self.pts[j] - self.prev[j]) * 0.995;
                self.prev[j] = self.pts[j];
                self.pts[j] += v + Vec3::NEG_Y * 9.81 * h * h;
            }
            for _ in 0..8 {
                for l in &self.links {
                    let d = self.pts[l.b] - self.pts[l.a];
                    let len = d.length();
                    if len < 1e-6 || (l.min_only && len >= l.rest) {
                        continue;
                    }
                    let corr = d * ((len - l.rest) / len * 0.5);
                    self.pts[l.a] += corr;
                    self.pts[l.b] -= corr;
                }
                for j in 0..self.pts.len() {
                    if !self.sim[j] {
                        continue;
                    }
                    let (p, hit) = collide(self.pts[j], self.radius[j]);
                    if hit {
                        // contact friction: bleed off sliding speed
                        let v = self.pts[j] - self.prev[j];
                        self.pts[j] = p;
                        self.prev[j] = p - Vec3::new(v.x * 0.55, v.y.max(0.0) * 0.2, v.z * 0.55);
                    }
                }
            }
            for j in 0..self.pts.len() {
                if self.sim[j] {
                    moved = moved.max((self.pts[j] - self.prev[j]).length() / h);
                }
            }
        }
        self.quiet = if moved < 0.06 { self.quiet + dt } else { 0.0 };
        if self.t > 1.5 && self.quiet > 0.6 {
            self.asleep = true;
        }
    }

    /// World-space joint matrices for the current state.
    pub fn globals(&self, skel: &Skeleton) -> Vec<Mat4> {
        let n = self.pts.len();
        let mut rot: Vec<Option<Quat>> = vec![None; n];
        for j in 0..n {
            if !self.sim[j] {
                continue;
            }
            let Some(c) = self.frame[j].0 else { continue };
            let a0 = self.init_pts[c] - self.init_pts[j];
            let a1 = self.pts[c] - self.pts[j];
            let q = match self.frame[j].1 {
                Some(s) => {
                    let b0 = self.init_pts[s] - self.init_pts[j];
                    let b1 = self.pts[s] - self.pts[j];
                    match (basis(a0, b0), basis(a1, b1)) {
                        (Some(f0), Some(f1)) => Some(Quat::from_mat3(&(f1 * f0.transpose())).normalize()),
                        _ => None,
                    }
                }
                None => None,
            };
            rot[j] = Some(q.unwrap_or_else(|| Quat::from_rotation_arc(a0.normalize_or_zero(), a1.normalize_or_zero())));
        }
        // leaves take their parent's rotation
        for j in 0..n {
            if self.sim[j] && rot[j].is_none() {
                rot[j] = Some(self.parent[j].and_then(|p| rot[p]).unwrap_or(Quat::IDENTITY));
            }
        }
        let mut out = self.init_global.clone();
        for j in 0..n {
            if let Some(q) = rot[j] {
                out[j] = Mat4::from_translation(self.pts[j]) * Mat4::from_quat(q) * self.init_basis[j];
            }
        }
        // non-simulated joints ride rigidly on their nearest simulated ancestor
        for &j in &skel.order {
            if self.sim[j] {
                continue;
            }
            let mut p = skel.joints[j].parent;
            while let Some(q) = p {
                if self.sim[q] {
                    break;
                }
                p = skel.joints[q].parent;
            }
            if let Some(a) = p {
                out[j] = out[a] * self.init_global[a].inverse() * self.init_global[j];
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::model::{load_all, Pose};

    #[test]
    fn ragdoll_falls_and_settles_on_the_floor() {
        let all = load_all();
        for name in ["Characters_Matt_SingleWeapon", "Strain_Bulk", "Strain_Skitter", "Strain_Larva"] {
            let m = all.iter().find(|m| m.name == name).unwrap();
            let skel = &m.skeleton;
            let mut p = Pose::rest(skel);
            let c = &m.clips[m.clip("Idle").or(m.clip("Walk")).unwrap_or(0)];
            p.sample(skel, skel, c, 0.0, 1.0);
            let world = Mat4::from_scale(Vec3::splat(1.75 / (m.max.y - m.min.y)));
            let g: Vec<Mat4> = p.globals(skel).iter().map(|g| world * *g).collect();
            let d = deforming(skel, m.skin_verts.iter().map(|v| (v.joints, v.weights)));
            let mut r = Ragdoll::new(skel, &g, &d, Vec3::ZERO, Vec3::new(3.0, 1.0, 0.0), Vec3::Y, 1.0);
            let floor = |q: Vec3, rad: f32| if q.y < rad { (Vec3::new(q.x, rad, q.z), true) } else { (q, false) };
            for _ in 0..600 {
                r.step(1.0 / 60.0, &floor);
            }
            let gl = r.globals(skel);
            assert!(gl.iter().all(|m| m.is_finite()), "{name}: non-finite pose");
            let top = r.pts.iter().zip(&r.sim).filter(|(_, s)| **s).map(|(p, _)| p.y).fold(f32::MIN, f32::max);
            eprintln!("{name}: settled={} top={top:.2}", r.asleep);
            assert!(top < 1.0, "{name}: ragdoll did not collapse (top {top})");
            assert!(r.pts.iter().all(|p| p.y > -0.05), "{name}: fell through the floor");
            // bones keep their length
            for l in r.links.iter().filter(|l| !l.min_only) {
                let len = (r.pts[l.a] - r.pts[l.b]).length();
                assert!((len - l.rest).abs() < 0.08 + l.rest * 0.15, "{name}: link stretched {len} vs {}", l.rest);
            }
        }
    }
}
