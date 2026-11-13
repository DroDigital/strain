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
    /// A joint by name. Gameplay code names bones the kit way (`Torso`, `UpperArm.L`, ...); on the
    /// realistic (Unreal-style) skeletons those resolve to their counterparts.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied().or_else(|| alias(name).and_then(|a| self.by_name.get(&a).copied()))
    }
    /// True for the realistic humanoid skeleton (Universal Base Characters / Animation Library).
    pub fn realistic(&self) -> bool {
        self.by_name.contains_key("pelvis")
    }
}

/// Kit bone name -> Unreal-style humanoid bone name.
pub fn alias(name: &str) -> Option<String> {
    let (base, side) = match name.rsplit_once('.') {
        Some((b, s)) if s == "L" || s == "R" => (b, Some(s.to_lowercase())),
        _ => (name, None),
    };
    let ue = match base {
        "Root" => "root",
        "Body" | "Hips" => "pelvis",
        "Abdomen" => "spine_01",
        "Torso" | "Torso2" => "spine_03",
        "Neck" | "Neck1" => "neck_01",
        "Head" => "Head",
        "Shoulder" => "clavicle",
        "UpperArm" => "upperarm",
        "LowerArm" => "lowerarm",
        "Middle1" => "middle_01",
        "Index1" => "index_01",
        "Pinky1" => "pinky_01",
        "Thumb1" => "thumb_01",
        "UpperLeg" => "thigh",
        "LowerLeg" => "calf",
        "Foot" => "foot",
        _ => return None,
    };
    Some(match side {
        Some(s) => format!("{ue}_{s}"),
        None => ue.to_string(),
    })
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
            let e = emis[0].max(emis[1]).max(emis[2]);
            // `T:<Tex>` materials (the custom infected) sample one of the game's texture layers
            let layer = if atlas { ATLAS_LAYER } else { mat.name().and_then(tex_layer).unwrap_or(0.0) };
            // vertex colours multiply the material colour; alpha below 1 marks emission
            let vcol: Vec<[f32; 4]> = r.read_colors(0).map(|c| c.into_rgba_f32().collect()).unwrap_or_else(|| vec![[1.0; 4]; pos.len()]);
            let colour = |i: usize| -> [u8; 4] {
                let c = vcol[i];
                let ch = |k: usize| ((f[k] * c[k]).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
                [ch(0), ch(1), ch(2), (e.max(1.0 - c[3]).clamp(0.0, 1.0) * 255.0) as u8]
            };
            if skinned {
                let joints: Vec<[u16; 4]> = r.read_joints(0).map(|j| j.into_u16().collect()).unwrap_or_else(|| vec![[0; 4]; pos.len()]);
                let weights: Vec<[f32; 4]> = r.read_weights(0).map(|w| w.into_f32().collect()).unwrap_or_else(|| vec![[1.0, 0.0, 0.0, 0.0]; pos.len()]);
                let base = model.skin_verts.len() as u32;
                for i in 0..pos.len() {
                    model.skin_verts.push(SkinVertex { pos: pos[i], normal: nrm[i], uv: [uv[i][0], uv[i][1], layer], color: colour(i), joints: joints[i], weights: weights[i] });
                }
                model.skin_idx.extend(idx.iter().map(|i| i + base));
            } else {
                let nm = gm.inverse().transpose();
                let base = model.mesh.verts.len() as u32;
                for i in 0..pos.len() {
                    let p = gm.transform_point3(Vec3::from(pos[i]));
                    model.min = model.min.min(p);
                    model.max = model.max.max(p);
                    model.mesh.verts.push(Vertex { pos: p.into(), normal: nm.transform_vector3(Vec3::from(nrm[i])).normalize_or_zero().into(), uv: [uv[i][0], uv[i][1], layer], color: colour(i) });
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

/// Texture-array layer for a material named `T:<Tex>`.
fn tex_layer(name: &str) -> Option<f32> {
    if let Some(c) = name.strip_prefix("C:") {
        return crate::render::textures::char_layer(c).map(|i| crate::render::textures::CHAR_BASE + i as f32);
    }
    let t: crate::render::textures::Tex = ron::from_str(name.strip_prefix("T:")?).ok()?;
    Some(t as u32 as f32)
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
                Track::T(v) if same || (src.joints[ch.joint].t - sk.joints[j].t).length() < 1e-3 => {
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
        let zombie = get("Zombie_Basic");
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
    /// breakable props: fragment mesh ids and each fragment's centre in model space
    pub fragments: HashMap<String, Vec<(u32, Vec3)>>,
    /// realistic models: the clip library retargeted onto their skeleton
    pub clip_donor: HashMap<usize, usize>,
}

/// Kit props that shatter, with roughly how many pieces.
pub const BREAKABLE: [(&str, usize); 9] = [
    ("Pallet", 7),
    ("Barrel", 6),
    ("PlasticBarrier", 5),
    ("TrafficBarrier_1", 6),
    ("TrafficBarrier_2", 6),
    ("Couch", 8),
    ("Chest", 6),
    ("FireHydrant", 4),
    ("Pipes", 5),
];

/// Split a mesh into `k` pieces by clustering its triangles (k-means on triangle centres).
/// Each piece is re-centred on its own centre so it can tumble about it.
pub fn fracture(m: &MeshBuilder, k: usize) -> Vec<(MeshBuilder, Vec3)> {
    let tris: Vec<[u32; 3]> = m.idx.chunks(3).filter(|t| t.len() == 3).map(|t| [t[0], t[1], t[2]]).collect();
    if tris.is_empty() {
        return Vec::new();
    }
    let cen: Vec<Vec3> = tris.iter().map(|t| (Vec3::from(m.verts[t[0] as usize].pos) + Vec3::from(m.verts[t[1] as usize].pos) + Vec3::from(m.verts[t[2] as usize].pos)) / 3.0).collect();
    let k = k.min(tris.len()).max(1);
    // farthest-point seeds, then a few Lloyd iterations
    let mut seeds = vec![cen[0]];
    while seeds.len() < k {
        let far = cen.iter().copied().max_by(|a, b| {
            let da = seeds.iter().map(|s| (*s - *a).length_squared()).fold(f32::MAX, f32::min);
            let db = seeds.iter().map(|s| (*s - *b).length_squared()).fold(f32::MAX, f32::min);
            da.total_cmp(&db)
        });
        seeds.push(far.unwrap());
    }
    let mut owner = vec![0usize; tris.len()];
    for _ in 0..8 {
        for (i, c) in cen.iter().enumerate() {
            owner[i] = (0..k).min_by(|&a, &b| (seeds[a] - *c).length_squared().total_cmp(&(seeds[b] - *c).length_squared())).unwrap();
        }
        let mut sum = vec![(Vec3::ZERO, 0.0f32); k];
        for (i, c) in cen.iter().enumerate() {
            sum[owner[i]].0 += *c;
            sum[owner[i]].1 += 1.0;
        }
        for j in 0..k {
            if sum[j].1 > 0.0 {
                seeds[j] = sum[j].0 / sum[j].1;
            }
        }
    }
    let mut out = Vec::new();
    for j in 0..k {
        let mut mb = MeshBuilder::new();
        let mut map: HashMap<u32, u32> = HashMap::new();
        for (i, t) in tris.iter().enumerate() {
            if owner[i] != j {
                continue;
            }
            for &v in t {
                let n = *map.entry(v).or_insert_with(|| {
                    let mut vv = m.verts[v as usize];
                    vv.pos = (Vec3::from(vv.pos) - seeds[j]).into();
                    mb.verts.push(vv);
                    mb.verts.len() as u32 - 1
                });
                mb.idx.push(n);
            }
        }
        if !mb.idx.is_empty() {
            out.push((mb, seeds[j]));
        }
    }
    out
}
impl Library {
    /// Load every model, appending static meshes to the primitive buffer.
    pub fn build(prims: &mut MeshBuilder, ranges: &mut Vec<super::mesh::MeshRange>) -> Library {
        let models = load_all();
        let mut lib = Library { models: Vec::new(), static_id: HashMap::new(), skin_id: HashMap::new(), index: HashMap::new(), fragments: HashMap::new(), clip_donor: HashMap::new() };
        let mut skin_n = 0u32;
        for m in models {
            if !m.mesh.idx.is_empty() {
                lib.static_id.insert(m.name.clone(), ranges.len() as u32);
                ranges.push(super::mesh::MeshRange { first: prims.idx.len() as u32, count: m.mesh.idx.len() as u32, base: prims.verts.len() as i32 });
                prims.verts.extend_from_slice(&m.mesh.verts);
                prims.idx.extend_from_slice(&m.mesh.idx);
                if let Some(&(_, k)) = BREAKABLE.iter().find(|b| b.0 == m.name) {
                    let mut frags = Vec::new();
                    for (mb, c) in fracture(&m.mesh, k) {
                        frags.push((ranges.len() as u32, c));
                        ranges.push(super::mesh::MeshRange { first: prims.idx.len() as u32, count: mb.idx.len() as u32, base: prims.verts.len() as i32 });
                        prims.verts.extend_from_slice(&mb.verts);
                        prims.idx.extend_from_slice(&mb.idx);
                    }
                    lib.fragments.insert(m.name.clone(), frags);
                }
            }
            if m.skinned() {
                lib.skin_id.insert(m.name.clone(), skin_n);
                skin_n += 1;
            }
            lib.index.insert(m.name.clone(), lib.models.len());
            lib.models.push(m);
        }
        // each realistic body takes its clips from the library whose skeleton matches its own
        let libs: Vec<usize> = lib.models.iter().enumerate().filter(|(_, m)| m.name.starts_with("Clips_")).map(|(i, _)| i).collect();
        for (i, m) in lib.models.iter().enumerate() {
            if !m.skeleton.realistic() || m.name.starts_with("Clips_") {
                continue;
            }
            let diff = |d: usize| -> f32 {
                let s = &lib.models[d].skeleton;
                m.skeleton.joints.iter().map(|j| s.by_name.get(&j.name).map(|&k| (s.joints[k].t - j.t).length()).unwrap_or(1.0)).sum()
            };
            if let Some(&d) = libs.iter().min_by(|a, b| diff(**a).total_cmp(&diff(**b))) {
                lib.clip_donor.insert(i, d);
            }
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
mod fracture_tests {
    /// Breaking a prop keeps every triangle, in roughly the asked number of pieces.
    #[test]
    fn breakables_fracture_into_pieces() {
        let all = super::load_all();
        for (name, k) in super::BREAKABLE {
            let m = all.iter().find(|m| m.name == name).unwrap_or_else(|| panic!("missing breakable {name}"));
            let frags = super::fracture(&m.mesh, k);
            let tris: usize = frags.iter().map(|f| f.0.idx.len()).sum();
            assert_eq!(tris, m.mesh.idx.len(), "{name}: triangles lost");
            assert!(frags.len() >= 2.min(k) && frags.len() <= k, "{name}: {} pieces", frags.len());
        }
    }
}

#[cfg(test)]
mod infected_tests {
    use glam::{Mat4, Vec3};
    /// The Blender-built humans and infected stand on the realistic skeleton, draw their clips from
    /// a retargeted library that has everything the game asks for, and every material resolves.
    #[test]
    fn realistic_characters_are_complete() {
        let mut prims = crate::render::mesh::MeshBuilder::new();
        let mut ranges = Vec::new();
        let lib = super::Library::build(&mut prims, &mut ranges);
        let names = [
            "Human_Player", "Human_Soldier_M", "Human_Soldier_F", "Human_Civilian_M1", "Human_Civilian_M2", "Human_Civilian_F1", "Human_Civilian_F2",
            "Strain_Carrier", "Strain_Shambler", "Strain_Bulk", "Strain_Retch", "Strain_Larva", "Strain_Bulwark", "Strain_Lurker", "Strain_Matron",
            "Strain_Digestor", "Strain_Geometer", "Strain_MotherCinder", "Strain_Cantor", "Strain_Seed", "Strain_Skitter",
        ];
        for n in names {
            let i = *lib.index.get(n).unwrap_or_else(|| panic!("missing model {n}"));
            let c = &lib.models[i];
            assert!(c.skinned() && c.skeleton.realistic(), "{n} is not a realistic skinned model");
            let d = *lib.clip_donor.get(&i).unwrap_or_else(|| panic!("{n} has no clip library"));
            for clip in ["Idle_Loop", "Walk_Loop", "Jog_Fwd_Loop", "Sword_Attack", "Hit_Chest", "Z_Walk", "Z_Run", "Z_Attack", "Z_Crawl"] {
                assert!(lib.models[d].clip(clip).is_some(), "{n}: library {} lacks {clip}", lib.models[d].name);
            }
            let layers: std::collections::HashSet<i32> = c.skin_verts.iter().map(|v| v.uv[2] as i32).collect();
            assert!(!layers.contains(&0), "{n} has a part with an unknown material");
            assert!(layers.iter().any(|&l| l >= 300), "{n} has no per-character skin");
            let nv = c.skin_verts.len() as u32;
            assert!(c.skin_idx.iter().all(|&k| k < nv), "{n} index out of range");
            assert!(c.skin_verts.iter().all(|v| (v.weights.iter().sum::<f32>() - 1.0).abs() < 0.02), "{n} weights not normalised");
            // a sensible human stature
            let h = c.max.y - c.min.y;
            assert!((1.3..2.6).contains(&h), "{n} is {h} m tall");
        }
        // clips keep the feet on the floor: the lowest skinned vertex stays near the ground
        for n in ["Human_Player", "Human_Civilian_F1", "Strain_MotherCinder", "Strain_Carrier"] {
            let i = lib.index[n];
            let m = &lib.models[i];
            let d = &lib.models[lib.clip_donor[&i]];
            for clip in ["Idle_Loop", "Walk_Loop", "Z_Idle", "Z_Walk", "Z_Attack", "Z_Scream", "Sword_Attack"] {
                let c = &d.clips[d.clip(clip).unwrap()];
                let mut lo = f32::MAX;
                for t in [0.0, 0.3, 0.6] {
                    let mut p = super::Pose::rest(&m.skeleton);
                    p.sample(&m.skeleton, &d.skeleton, c, c.duration * t, 1.0);
                    let g = p.globals(&m.skeleton);
                    let pal: Vec<Mat4> = g.iter().zip(&m.skeleton.joints).map(|(g, j)| *g * j.inverse_bind).collect();
                    for v in m.skin_verts.iter().step_by(7) {
                        let mut q = Vec3::ZERO;
                        for k in 0..4 {
                            q += pal[v.joints[k] as usize].transform_point3(Vec3::from(v.pos)) * v.weights[k];
                        }
                        lo = lo.min(q.y);
                    }
                }
                eprintln!("{n} {clip}: lowest point {lo:.3} m (bind {:.3})", m.min.y);
                assert!(lo.abs() < 0.12, "{n} {clip}: feet {lo} m off the floor");
            }
        }
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
