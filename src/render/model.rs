//! glTF (GLB) models: static meshes for props/weapons/vehicles, skinned meshes with skeletons
//! and animation clips for characters. All models share one texture atlas (assets/models/atlas.png),
//! referenced through the reserved texture layer `ATLAS_LAYER`.
use super::mesh::{MeshBuilder, Vertex};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;

/// Texture-array layer value that tells the shader to sample the model atlas instead.
pub const ATLAS_LAYER: f32 = 255.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct SkinVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 3],
    pub color: [u8; 4],
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct Joint {
    pub name: String,
    pub parent: Option<usize>,
    pub t: Vec3,
    pub r: Quat,
    pub s: Vec3,
    pub inverse_bind: Mat4,
}

#[derive(Clone, Debug)]
pub enum Track {
    T(Vec<Vec3>),
    R(Vec<Quat>),
    S(Vec<Vec3>),
}
#[derive(Clone, Debug)]
pub struct Channel {
    pub joint: usize,
    pub times: Vec<f32>,
    pub track: Track,
    pub step: bool,
}
#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Debug, Default)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
    /// transform of the armature above the root joints
    pub root: Mat4,
    pub by_name: HashMap<String, usize>,
    /// joint indices with every parent before its children
    pub order: Vec<usize>,
}
impl Skeleton {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }
}

pub struct Model {
    pub name: String,
    /// static geometry (baked node transforms), in model space
    pub mesh: MeshBuilder,
    /// skinned geometry
    pub skin_verts: Vec<SkinVertex>,
    pub skin_idx: Vec<u32>,
    pub skeleton: Skeleton,
    pub clips: Vec<Clip>,
    pub min: Vec3,
    pub max: Vec3,
}
impl Model {
    pub fn clip(&self, name: &str) -> Option<usize> {
        self.clips.iter().position(|c| c.name == name)
    }
    pub fn skinned(&self) -> bool {
        !self.skin_idx.is_empty()
    }
}

fn node_local(n: &gltf::Node) -> Mat4 {
    Mat4::from_cols_array_2d(&n.transform().matrix())
}

/// Load a GLB. For skinned characters, only skinned meshes are kept (weapon props baked into
/// the character files are dropped; weapons are attached by the game).
pub fn load(name: &str, bytes: &[u8]) -> Option<Model> {
    let g = gltf::Gltf::from_slice(bytes).ok()?;
    let blob = g.blob.as_deref()?;
    let get_buf = |b: gltf::Buffer| -> Option<&[u8]> { if b.index() == 0 { Some(blob) } else { None } };
    let doc = &g.document;
    let scene = doc.default_scene().or_else(|| doc.scenes().next())?;
    // global transforms of every node
    let mut global: HashMap<usize, Mat4> = HashMap::new();
    let mut parent: HashMap<usize, usize> = HashMap::new();
    fn walk(n: gltf::Node, m: Mat4, global: &mut HashMap<usize, Mat4>, parent: &mut HashMap<usize, usize>) {
        let gm = m * node_local(&n);
        global.insert(n.index(), gm);
        for c in n.children() {
            parent.insert(c.index(), n.index());
            walk(c, gm, global, parent);
        }
    }
    for n in scene.nodes() {
        walk(n, Mat4::IDENTITY, &mut global, &mut parent);
    }
    let has_skin = doc.skins().next().is_some();
    let mut model = Model { name: name.to_string(), mesh: MeshBuilder::new(), skin_verts: Vec::new(), skin_idx: Vec::new(), skeleton: Skeleton::default(), clips: Vec::new(), min: Vec3::splat(f32::MAX), max: Vec3::splat(f32::MIN) };
    let mut joint_of_node: HashMap<usize, usize> = HashMap::new();
    if let Some(skin) = doc.skins().next() {
        let reader = skin.reader(get_buf);
        let ibms: Vec<Mat4> = reader.read_inverse_bind_matrices().map(|it| it.map(|m| Mat4::from_cols_array_2d(&m)).collect()).unwrap_or_default();
        let joints: Vec<gltf::Node> = skin.joints().collect();
        for (i, j) in joints.iter().enumerate() {
            joint_of_node.insert(j.index(), i);
        }
        let mut sk = Skeleton::default();
        for (i, j) in joints.iter().enumerate() {
            let (t, r, s) = j.transform().decomposed();
            let p = parent.get(&j.index()).and_then(|p| joint_of_node.get(p)).copied();
            sk.by_name.insert(j.name().unwrap_or("").to_string(), i);
            sk.joints.push(Joint { name: j.name().unwrap_or("").to_string(), parent: p, t: Vec3::from(t), r: Quat::from_array(r), s: Vec3::from(s), inverse_bind: ibms.get(i).copied().unwrap_or(Mat4::IDENTITY) });
        }
        // armature transform above the first root joint
        if let Some(rj) = joints.iter().find(|j| !parent.get(&j.index()).map(|p| joint_of_node.contains_key(p)).unwrap_or(false)) {
            sk.root = parent.get(&rj.index()).and_then(|p| global.get(p)).copied().unwrap_or(Mat4::IDENTITY);
        }
        let depth = |mut i: usize| {
            let mut d = 0;
            while let Some(p) = sk.joints[i].parent {
                i = p;
                d += 1;
            }
            d
        };
        let mut order: Vec<usize> = (0..sk.joints.len()).collect();
        order.sort_by_key(|&i| depth(i));
        sk.order = order;
        model.skeleton = sk;
        // clips
        for a in doc.animations() {
            let mut clip = Clip { name: a.name().unwrap_or("").to_string(), duration: 0.0, channels: Vec::new() };
            for ch in a.channels() {
                let Some(&joint) = joint_of_node.get(&ch.target().node().index()) else { continue };
                let r = ch.reader(get_buf);
                let Some(times) = r.read_inputs().map(|t| t.collect::<Vec<f32>>()) else { continue };
                clip.duration = clip.duration.max(times.last().copied().unwrap_or(0.0));
                let step = ch.sampler().interpolation() == gltf::animation::Interpolation::Step;
                let cubic = ch.sampler().interpolation() == gltf::animation::Interpolation::CubicSpline;
                let track = match r.read_outputs() {
                    Some(gltf::animation::util::ReadOutputs::Translations(it)) => Track::T(it.map(Vec3::from).collect()),
                    Some(gltf::animation::util::ReadOutputs::Rotations(it)) => Track::R(it.into_f32().map(Quat::from_array).collect()),
                    Some(gltf::animation::util::ReadOutputs::Scales(it)) => Track::S(it.map(Vec3::from).collect()),
                    _ => continue,
                };
                // cubic splines store (in-tangent, value, out-tangent); keep the values
                let track = if cubic {
                    match track {
                        Track::T(v) => Track::T(v.chunks(3).map(|c| c[1]).collect()),
                        Track::R(v) => Track::R(v.chunks(3).map(|c| c[1]).collect()),
                        Track::S(v) => Track::S(v.chunks(3).map(|c| c[1]).collect()),
                    }
                } else {
                    track
                };
                clip.channels.push(Channel { joint, times, track, step });
            }
            if !clip.channels.is_empty() {
                model.clips.push(clip);
            }
        }
    }
    // meshes
    for n in doc.nodes() {
        let Some(mesh) = n.mesh() else { continue };
        let skinned = n.skin().is_some();
        if has_skin && !skinned {
            continue;
        }
        let gm = global.get(&n.index()).copied().unwrap_or(Mat4::IDENTITY);
        for prim in mesh.primitives() {
            let r = prim.reader(get_buf);
            let Some(pos) = r.read_positions().map(|p| p.collect::<Vec<[f32; 3]>>()) else { continue };
            let nrm: Vec<[f32; 3]> = r.read_normals().map(|p| p.collect()).unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; pos.len()]);
            let uv: Vec<[f32; 2]> = r.read_tex_coords(0).map(|t| t.into_f32().collect()).unwrap_or_else(|| vec![[0.0; 2]; pos.len()]);
            let idx: Vec<u32> = r.read_indices().map(|i| i.into_u32().collect()).unwrap_or_else(|| (0..pos.len() as u32).collect());
            let mat = prim.material();
            let atlas = mat.name() == Some("Atlas");
            let f = mat.pbr_metallic_roughness().base_color_factor();
            let emis = mat.emissive_factor();
            let e = (emis[0].max(emis[1]).max(emis[2]) * 255.0) as u8;
            let color = [(f[0].powf(1.0 / 2.2) * 255.0) as u8, (f[1].powf(1.0 / 2.2) * 255.0) as u8, (f[2].powf(1.0 / 2.2) * 255.0) as u8, e];
            let layer = if atlas { ATLAS_LAYER } else { 0.0 };
            if skinned {
                let joints: Vec<[u16; 4]> = r.read_joints(0).map(|j| j.into_u16().collect()).unwrap_or_else(|| vec![[0; 4]; pos.len()]);
                let weights: Vec<[f32; 4]> = r.read_weights(0).map(|w| w.into_f32().collect()).unwrap_or_else(|| vec![[1.0, 0.0, 0.0, 0.0]; pos.len()]);
                let base = model.skin_verts.len() as u32;
                for i in 0..pos.len() {
                    model.skin_verts.push(SkinVertex { pos: pos[i], normal: nrm[i], uv: [uv[i][0], uv[i][1], layer], color, joints: joints[i], weights: weights[i] });
                }
                model.skin_idx.extend(idx.iter().map(|i| i + base));
            } else {
                let nm = gm.inverse().transpose();
                let base = model.mesh.verts.len() as u32;
                for i in 0..pos.len() {
                    let p = gm.transform_point3(Vec3::from(pos[i]));
                    model.min = model.min.min(p);
                    model.max = model.max.max(p);
                    model.mesh.verts.push(Vertex { pos: p.into(), normal: nm.transform_vector3(Vec3::from(nrm[i])).normalize_or_zero().into(), uv: [uv[i][0], uv[i][1], layer], color });
                }
                model.mesh.idx.extend(idx.iter().map(|i| i + base));
            }
        }
    }
    if model.skinned() {
        // bounds of the bind pose
        for v in &model.skin_verts {
            model.min = model.min.min(Vec3::from(v.pos));
            model.max = model.max.max(Vec3::from(v.pos));
        }
    }
    Some(model)
}

/// Every embedded model, by file stem.
pub fn load_all() -> Vec<Model> {
    let mut out = Vec::new();
    for (path, bytes) in crate::assets::FILES.iter() {
        if let Some(stem) = path.strip_prefix("models/").and_then(|p| p.strip_suffix(".glb")) {
            if let Some(m) = load(stem, bytes) {
                out.push(m);
            }
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------
// Animation sampling and blending

fn sample_vec(times: &[f32], v: &[Vec3], t: f32, step: bool) -> Vec3 {
    if v.is_empty() {
        return Vec3::ZERO;
    }
    let i = times.partition_point(|x| *x <= t);
    if i == 0 {
        return v[0];
    }
    if i >= times.len() {
        return v[v.len() - 1];
    }
    if step {
        return v[i - 1];
    }
    let (a, b) = (times[i - 1], times[i]);
    let k = ((t - a) / (b - a).max(1e-6)).clamp(0.0, 1.0);
    v[i - 1].lerp(v[i], k)
}
fn sample_quat(times: &[f32], v: &[Quat], t: f32, step: bool) -> Quat {
    if v.is_empty() {
        return Quat::IDENTITY;
    }
    let i = times.partition_point(|x| *x <= t);
    if i == 0 {
        return v[0];
    }
    if i >= times.len() {
        return v[v.len() - 1];
    }
    if step {
        return v[i - 1];
    }
    let (a, b) = (times[i - 1], times[i]);
    let k = ((t - a) / (b - a).max(1e-6)).clamp(0.0, 1.0);
    v[i - 1].slerp(v[i], k)
}

/// Local pose of a skeleton: translation, rotation, scale per joint.
#[derive(Clone, Debug)]
pub struct Pose {
    pub t: Vec<Vec3>,
    pub r: Vec<Quat>,
    pub s: Vec<Vec3>,
}
impl Pose {
    pub fn rest(sk: &Skeleton) -> Pose {
        Pose { t: sk.joints.iter().map(|j| j.t).collect(), r: sk.joints.iter().map(|j| j.r).collect(), s: sk.joints.iter().map(|j| j.s).collect() }
    }
    /// Sample `clip` (authored on `src` skeleton) onto this pose's skeleton by joint name.
    /// Foreign clips only contribute rotations (and the hip height), so proportions survive.
    pub fn sample(&mut self, sk: &Skeleton, src: &Skeleton, clip: &Clip, time: f32, weight: f32) {
        let same = std::ptr::eq(sk, src);
        for ch in &clip.channels {
            let j = if same {
                ch.joint
            } else {
                match sk.find(&src.joints[ch.joint].name) {
                    Some(j) => j,
                    None => continue,
                }
            };
            match &ch.track {
                Track::R(v) => {
                    let q = sample_quat(&ch.times, v, time, ch.step);
                    self.r[j] = if weight >= 0.999 { q } else { self.r[j].slerp(q, weight) };
                }
                Track::T(v) if same => {
                    let p = sample_vec(&ch.times, v, time, ch.step);
                    self.t[j] = self.t[j].lerp(p, weight);
                }
                Track::S(v) if same => {
                    let p = sample_vec(&ch.times, v, time, ch.step);
                    self.s[j] = self.s[j].lerp(p, weight);
                }
                _ => {}
            }
        }
    }
    /// Global (model-space) joint matrices.
    pub fn globals(&self, sk: &Skeleton) -> Vec<Mat4> {
        let mut out = vec![Mat4::IDENTITY; sk.joints.len()];
        for &i in &sk.order {
            let local = Mat4::from_scale_rotation_translation(self.s[i], self.r[i], self.t[i]);
            out[i] = match sk.joints[i].parent {
                Some(p) => out[p] * local,
                None => sk.root * local,
            };
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn models_load_and_animate() {
        let all = load_all();
        assert!(all.len() > 40);
        let get = |n: &str| all.iter().find(|m| m.name == n).unwrap();
        let human = get("Characters_Matt_SingleWeapon");
        let zombie = get("Zombie_Chubby");
        assert!(human.skinned() && zombie.skinned());
        assert!(human.clip("Slash").is_some() && zombie.clip("Walk").is_some());
        let mut p = Pose::rest(&zombie.skeleton);
        let c = &human.clips[human.clip("Slash").unwrap()];
        p.sample(&zombie.skeleton, &human.skeleton, c, c.duration * 0.5, 1.0);
        let g = p.globals(&zombie.skeleton);
        assert!(g.iter().all(|m| m.is_finite()));
        // skinning the mesh with its own idle clip keeps it the size of its bind pose
        for m in [human, zombie] {
            let mut p = Pose::rest(&m.skeleton);
            let c = &m.clips[m.clip("Idle").unwrap()];
            p.sample(&m.skeleton, &m.skeleton, c, 0.0, 1.0);
            let g = p.globals(&m.skeleton);
            let pal: Vec<Mat4> = g.iter().zip(&m.skeleton.joints).map(|(g, j)| *g * j.inverse_bind).collect();
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for v in &m.skin_verts {
                let mut q = Vec3::ZERO;
                for k in 0..4 {
                    q += pal[v.joints[k] as usize].transform_point3(Vec3::from(v.pos)) * v.weights[k];
                }
                lo = lo.min(q.y);
                hi = hi.max(q.y);
            }
            let bind = m.max.y - m.min.y;
            eprintln!("{}: bind height {bind:.2}, idle height {:.2}", m.name, hi - lo);
            assert!(((hi - lo) / bind - 1.0).abs() < 0.25);
        }
        let h = human.max.y - human.min.y;
        eprintln!("human height {h:.2}, bounds {:?} {:?}, clips {:?}", human.min, human.max, human.clips.iter().map(|c| (&c.name, c.duration)).collect::<Vec<_>>());
        let car = get("Vehicle_Pickup");
        eprintln!("pickup bounds {:?} {:?}", car.min, car.max);
    }
}

/// All models, with their mesh ids in the renderer: static models are extra entries in the
/// instanced mesh table (after the primitives); skinned models have their own index.
pub struct Library {
    pub models: Vec<Model>,
    pub static_id: HashMap<String, u32>,
    pub skin_id: HashMap<String, u32>,
    pub index: HashMap<String, usize>,
}
impl Library {
    /// Load every model, appending static meshes to the primitive buffer.
    pub fn build(prims: &mut MeshBuilder, ranges: &mut Vec<super::mesh::MeshRange>) -> Library {
        let models = load_all();
        let mut lib = Library { models: Vec::new(), static_id: HashMap::new(), skin_id: HashMap::new(), index: HashMap::new() };
        let mut skin_n = 0u32;
        for m in models {
            if !m.mesh.idx.is_empty() {
                lib.static_id.insert(m.name.clone(), ranges.len() as u32);
                ranges.push(super::mesh::MeshRange { first: prims.idx.len() as u32, count: m.mesh.idx.len() as u32, base: prims.verts.len() as i32 });
                prims.verts.extend_from_slice(&m.mesh.verts);
                prims.idx.extend_from_slice(&m.mesh.idx);
            }
            if m.skinned() {
                lib.skin_id.insert(m.name.clone(), skin_n);
                skin_n += 1;
            }
            lib.index.insert(m.name.clone(), lib.models.len());
            lib.models.push(m);
        }
        lib
    }
    /// Skinned geometry in skin-id order, for `Renderer::upload_skinned`.
    pub fn skin_data(&self) -> Vec<(Vec<SkinVertex>, Vec<u32>)> {
        let mut v: Vec<(u32, &Model)> = self.models.iter().filter(|m| m.skinned()).map(|m| (self.skin_id[&m.name], m)).collect();
        v.sort_by_key(|x| x.0);
        v.into_iter().map(|(_, m)| (m.skin_verts.clone(), m.skin_idx.clone())).collect()
    }
}
#[cfg(test)]
mod size_tests {
    #[test]
    fn print_prop_bounds() {
        for m in super::load_all() {
            if !m.skinned() {
                eprintln!("{:24} min {:6.2} {:6.2} {:6.2}  max {:6.2} {:6.2} {:6.2}", m.name, m.min.x, m.min.y, m.min.z, m.max.x, m.max.y, m.max.z);
            }
        }
    }
}
