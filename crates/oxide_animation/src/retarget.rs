//! Humanoid retargeting (Unity Mecanim): an animation made for one humanoid rig plays on
//! another — different bone names (Mixamo, Unreal, Blender), bone axes, proportions and
//! armature scale.
//!
//! Each rig's [`Avatar`] keeps its rest pose. Per frame, every humanoid bone's motion is
//! taken as a rotation from its rest orientation in model space on the source
//! (`D = M_src · Rest_src⁻¹`) and applied to the target's rest orientation
//! (`M_dst = D · Rest_dst`), so bone roll conventions don't matter; the hips' movement is
//! scaled by the ratio of the hips heights. Rigs should share a rest pose (both T-pose or both
//! A-pose), as Unity's Avatar configuration expects.
//! <https://docs.unity3d.com/Manual/Retargeting.html>

use crate::humanoid::{HumanBodyBones, auto_map_names};
use oxide_assets::animation::{AnimationClip, Interpolation, Track, TransformCurves};
use oxide_math::{Matrix4x4, Matrix4x4Ext, Quaternion, Vector3};
use oxide_scene::{GameObjectId, World};
use std::collections::HashMap;

/// A rig's rest pose and humanoid bone map (Unity's `Avatar`). Transforms are listed parents
/// first; paths are relative to the rig's root (as in clips).
#[derive(Clone, Debug, Default)]
pub struct Avatar {
    pub paths: Vec<String>,
    pub parents: Vec<Option<usize>>,
    pub positions: Vec<Vector3>,
    pub rotations: Vec<Quaternion>,
    pub scales: Vec<Vector3>,
    pub human: HashMap<HumanBodyBones, usize>,
}

impl Avatar {
    fn finish(mut self, names: &[String]) -> Self {
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        self.human = auto_map_names(&refs, &self.parents);
        self
    }

    /// From the objects below `root` as they are now (the rest pose before animating).
    pub fn from_world(world: &World, root: GameObjectId) -> Self {
        Self::from_world_rest(world, root, &HashMap::new())
    }

    /// [`Avatar::from_world`], with `rest` values for transforms that are already animated.
    pub fn from_world_rest(
        world: &World,
        root: GameObjectId,
        rest: &HashMap<GameObjectId, (Vector3, Quaternion, Vector3)>,
    ) -> Self {
        let mut a = Avatar::default();
        let mut names = Vec::new();
        let mut index: HashMap<GameObjectId, usize> = HashMap::new();
        // `descendants` lists parents before children.
        for id in world.descendants(root).into_iter().skip(1) {
            let Some(o) = world.get(id) else { continue };
            let parent = world.parent(id).and_then(|p| index.get(&p).copied());
            let path = match parent {
                Some(p) => format!("{}/{}", a.paths[p], o.name),
                None => o.name.clone(),
            };
            let t = world.transform(id).cloned().unwrap_or_default();
            let (p, r, s) = rest.get(&id).copied().unwrap_or((
                t.local_position,
                t.local_rotation,
                t.local_scale,
            ));
            index.insert(id, a.paths.len());
            a.paths.push(path);
            a.parents.push(parent);
            a.positions.push(p);
            a.rotations.push(r);
            a.scales.push(s);
            names.push(o.name.clone());
        }
        a.finish(&names)
    }

    /// From an imported model's nodes (the rig its clips were made for).
    pub fn from_model(model: &oxide_assets::Model) -> Self {
        let mut a = Avatar::default();
        let mut names = Vec::new();
        let mut stack: Vec<(usize, Option<usize>)> =
            model.roots.iter().rev().map(|&r| (r, None)).collect();
        while let Some((n, parent)) = stack.pop() {
            let node = &model.nodes[n];
            let path = match parent {
                Some(p) => format!("{}/{}", a.paths[p], node.name),
                None => node.name.clone(),
            };
            let i = a.paths.len();
            a.paths.push(path);
            a.parents.push(parent);
            a.positions.push(node.position);
            a.rotations.push(node.rotation);
            a.scales.push(node.scale);
            names.push(node.name.clone());
            for &c in node.children.iter().rev() {
                stack.push((c, Some(i)));
            }
        }
        a.finish(&names)
    }

    /// Whether the rig has every bone Unity requires of a humanoid.
    pub fn is_human(&self) -> bool {
        HumanBodyBones::REQUIRED
            .iter()
            .all(|b| self.human.contains_key(b))
    }

    /// Model-space matrices for local transforms `local(i)`.
    fn model_matrices(
        &self,
        local: impl Fn(usize) -> (Vector3, Quaternion, Vector3),
    ) -> Vec<Matrix4x4> {
        let mut out: Vec<Matrix4x4> = Vec::with_capacity(self.paths.len());
        for i in 0..self.paths.len() {
            let (p, r, s) = local(i);
            let m = Matrix4x4::trs(p, r, s);
            out.push(match self.parents[i] {
                Some(par) => out[par] * m,
                None => m,
            });
        }
        out
    }

    fn rest(&self, i: usize) -> (Vector3, Quaternion, Vector3) {
        (self.positions[i], self.rotations[i], self.scales[i])
    }
}

fn rotation_of(m: &Matrix4x4) -> Quaternion {
    m.to_scale_rotation_translation().1
}

/// The clip retargeted from `src`'s rig to `dst`'s: rotation curves for every humanoid bone
/// both rigs have, and the hips' position. `None` when either rig isn't humanoid.
// `i` indexes several per-transform arrays at once.
#[allow(clippy::needless_range_loop)]
pub fn retarget_clip(clip: &AnimationClip, src: &Avatar, dst: &Avatar) -> Option<AnimationClip> {
    if !src.is_human() || !dst.is_human() {
        return None;
    }
    let by_path: HashMap<&str, &TransformCurves> =
        clip.curves.iter().map(|c| (c.path.as_str(), c)).collect();
    let src_curve: Vec<Option<&TransformCurves>> = src
        .paths
        .iter()
        .map(|p| by_path.get(p.as_str()).copied())
        .collect();
    let src_rest = src.model_matrices(|i| src.rest(i));
    let dst_rest = dst.model_matrices(|i| dst.rest(i));
    // Bones both rigs have, in the target's parent-first order.
    let mut bones: Vec<(HumanBodyBones, usize, usize)> = dst
        .human
        .iter()
        .filter_map(|(b, &d)| src.human.get(b).map(|&s| (*b, s, d)))
        .collect();
    bones.sort_by_key(|&(_, _, d)| d);
    let src_hips = src.human[&HumanBodyBones::Hips];
    let dst_hips = dst.human[&HumanBodyBones::Hips];
    let height = |m: &[Matrix4x4], i: usize| m[i].w_axis.y.abs().max(1e-4);
    let scale = height(&dst_rest, dst_hips) / height(&src_rest, src_hips);

    let frames = ((clip.length * 30.0).ceil() as usize).max(1) + 1;
    let times: Vec<f32> = (0..frames)
        .map(|f| (f as f32 / (frames - 1) as f32 * clip.length).min(clip.length))
        .collect();
    let mut rot_keys: Vec<Vec<Quaternion>> = vec![Vec::with_capacity(frames); bones.len()];
    let mut hips_keys: Vec<Vector3> = Vec::with_capacity(frames);
    let dst_index: HashMap<usize, usize> =
        bones.iter().enumerate().map(|(k, b)| (b.2, k)).collect();
    for &t in &times {
        // Source pose: animated where the clip has curves, rest elsewhere.
        let src_m = src.model_matrices(|i| {
            let (mut p, mut r, mut s) = src.rest(i);
            if let Some(c) = src_curve[i] {
                p = c.position.as_ref().and_then(|tr| tr.sample(t)).unwrap_or(p);
                r = c.rotation.as_ref().and_then(|tr| tr.sample(t)).unwrap_or(r);
                s = c.scale.as_ref().and_then(|tr| tr.sample(t)).unwrap_or(s);
            }
            (p, r, s)
        });
        // Target: humanoid bones take the source's model-space rotation from rest; the rest
        // of the rig stays at rest. Parents come first, so `out` is ready for each child.
        let mut out: Vec<Matrix4x4> = Vec::with_capacity(dst.paths.len());
        for i in 0..dst.paths.len() {
            let parent = dst.parents[i].map(|p| out[p]);
            let (mut p, mut r, s) = dst.rest(i);
            if let Some(&k) = dst_index.get(&i) {
                let (_, si, _) = bones[k];
                let delta = rotation_of(&src_m[si]) * rotation_of(&src_rest[si]).inverse();
                let model_rot = delta * rotation_of(&dst_rest[i]);
                let parent_rot = parent.map_or(Quaternion::IDENTITY, |m| rotation_of(&m));
                r = (parent_rot.inverse() * model_rot).normalize();
                rot_keys[k].push(r);
                if i == dst_hips {
                    // Hips travel, scaled to the target's size, in the target's model space.
                    let moved = src_m[si].w_axis.truncate() - src_rest[si].w_axis.truncate();
                    let model_pos = dst_rest[i].w_axis.truncate() + moved * scale;
                    p = match parent {
                        Some(m) => m.inverse().transform_point3(model_pos),
                        None => model_pos,
                    };
                    hips_keys.push(p);
                }
            }
            let m = Matrix4x4::trs(p, r, s);
            out.push(match parent {
                Some(pm) => pm * m,
                None => m,
            });
        }
    }
    let curves = bones
        .iter()
        .enumerate()
        .map(|(k, &(_, _, d))| TransformCurves {
            path: dst.paths[d].clone(),
            position: (d == dst_hips).then(|| Track {
                times: times.clone(),
                values: hips_keys.clone(),
                interpolation: Interpolation::Linear,
            }),
            rotation: Some(Track {
                times: times.clone(),
                values: std::mem::take(&mut rot_keys[k]),
                interpolation: Interpolation::Linear,
            }),
            scale: None,
        })
        .collect();
    Some(AnimationClip {
        name: clip.name.clone(),
        length: clip.length,
        looping: clip.looping,
        curves,
        events: clip.events.clone(),
        // The hips (shallowest bone with a position curve) carry root motion.
        root_node: None,
        properties: Vec::new(),
        object_curves: Vec::new(),
    })
}

/// The source rig of an imported clip (`Assets/Run.fbx#Run` → the model `Assets/Run.fbx`).
pub fn clip_source_model(clip_path: &str) -> Option<&str> {
    clip_path
        .split_once('#')
        .map(|(m, _)| m)
        .filter(|m| oxide_assets::is_model_path(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_math::QuaternionExt;

    /// Bone name per rig (Mixamo, Unreal), parent, and model-space rest position (1 m tall).
    const BONES: [(&str, &str, Option<usize>, [f32; 3]); 20] = [
        ("mixamorig:Hips", "pelvis", None, [0.0, 1.0, 0.0]),
        ("mixamorig:Spine", "spine_01", Some(0), [0.0, 1.2, 0.0]),
        ("mixamorig:Spine1", "spine_02", Some(1), [0.0, 1.35, 0.0]),
        ("mixamorig:Spine2", "spine_03", Some(2), [0.0, 1.5, 0.0]),
        ("mixamorig:Neck", "neck_01", Some(3), [0.0, 1.65, 0.0]),
        ("mixamorig:Head", "head", Some(4), [0.0, 1.75, 0.0]),
        (
            "mixamorig:LeftShoulder",
            "clavicle_l",
            Some(3),
            [-0.1, 1.6, 0.0],
        ),
        (
            "mixamorig:LeftArm",
            "upperarm_l",
            Some(6),
            [-0.25, 1.6, 0.0],
        ),
        (
            "mixamorig:LeftForeArm",
            "lowerarm_l",
            Some(7),
            [-0.5, 1.6, 0.0],
        ),
        ("mixamorig:LeftHand", "hand_l", Some(8), [-0.75, 1.6, 0.0]),
        (
            "mixamorig:RightShoulder",
            "clavicle_r",
            Some(3),
            [0.1, 1.6, 0.0],
        ),
        (
            "mixamorig:RightArm",
            "upperarm_r",
            Some(10),
            [0.25, 1.6, 0.0],
        ),
        (
            "mixamorig:RightForeArm",
            "lowerarm_r",
            Some(11),
            [0.5, 1.6, 0.0],
        ),
        ("mixamorig:RightHand", "hand_r", Some(12), [0.75, 1.6, 0.0]),
        ("mixamorig:LeftUpLeg", "thigh_l", Some(0), [-0.1, 0.95, 0.0]),
        ("mixamorig:LeftLeg", "calf_l", Some(14), [-0.1, 0.5, 0.0]),
        ("mixamorig:LeftFoot", "foot_l", Some(15), [-0.1, 0.05, 0.0]),
        ("mixamorig:RightUpLeg", "thigh_r", Some(0), [0.1, 0.95, 0.0]),
        ("mixamorig:RightLeg", "calf_r", Some(17), [0.1, 0.5, 0.0]),
        ("mixamorig:RightFoot", "foot_r", Some(18), [0.1, 0.05, 0.0]),
    ];

    /// A rig with names from column `col`, `size` times the reference, and each bone's
    /// rest orientation turned by `twist(i)` (different bone axes per rig).
    fn rig(
        w: &mut World,
        col: usize,
        size: f32,
        twist: impl Fn(usize) -> Quaternion,
    ) -> (GameObjectId, Vec<GameObjectId>) {
        let root = w.create(if col == 0 { "Mixamo" } else { "Unreal" });
        let mut ids = Vec::new();
        for (i, b) in BONES.iter().enumerate() {
            let id = w.create(if col == 0 { b.0 } else { b.1 });
            w.set_parent(id, Some(b.2.map_or(root, |p| ids[p])), false)
                .unwrap();
            w.set_position(id, Vector3::from_array(b.3) * size);
            w.set_rotation(id, twist(i));
            ids.push(id);
        }
        // Children were placed in world space before their parents turned: place again.
        for (i, b) in BONES.iter().enumerate() {
            w.set_position(ids[i], Vector3::from_array(b.3) * size);
        }
        (root, ids)
    }

    /// Applies a clip's pose at `t` to the objects below `root`.
    fn pose(w: &mut World, root: GameObjectId, clip: &AnimationClip, t: f32) {
        for c in &clip.curves {
            let Some(id) = crate::find_path(w, root, &c.path) else {
                continue;
            };
            if let Some(r) = c.rotation.as_ref().and_then(|tr| tr.sample(t)) {
                w.transform_mut(id).unwrap().local_rotation = r;
            }
            if let Some(p) = c.position.as_ref().and_then(|tr| tr.sample(t)) {
                w.transform_mut(id).unwrap().local_position = p;
            }
        }
    }

    #[test]
    fn clips_retarget_across_names_axes_and_sizes() {
        let mut w = World::new();
        let (a_root, a) = rig(&mut w, 0, 1.0, |_| Quaternion::IDENTITY);
        // Unreal-style: every bone's axes rolled differently, and twice as big.
        let (b_root, b) = rig(&mut w, 1, 2.0, |i| {
            Quaternion::euler(90.0, (i * 37 % 360) as f32, -90.0 + i as f32 * 11.0)
        });
        let src = Avatar::from_world(&w, a_root);
        let dst = Avatar::from_world(&w, b_root);
        assert!(src.is_human() && dst.is_human());
        // A clip for rig A: left arm down, spine leaning, hips moving forward.
        let track = |v0: Quaternion, v1: Quaternion| Track {
            times: vec![0.0, 1.0],
            values: vec![v0, v1],
            interpolation: Interpolation::Linear,
        };
        let id = Quaternion::IDENTITY;
        let clip = AnimationClip {
            name: "Wave".into(),
            length: 1.0,
            curves: vec![
                TransformCurves {
                    path: src.paths[src.human[&HumanBodyBones::LeftUpperArm]].clone(),
                    rotation: Some(track(id, Quaternion::euler(0.0, 0.0, 80.0))),
                    ..Default::default()
                },
                TransformCurves {
                    path: src.paths[src.human[&HumanBodyBones::Spine]].clone(),
                    rotation: Some(track(id, Quaternion::euler(25.0, 0.0, 0.0))),
                    ..Default::default()
                },
                TransformCurves {
                    path: src.paths[src.human[&HumanBodyBones::Hips]].clone(),
                    position: Some(Track {
                        times: vec![0.0, 1.0],
                        values: vec![Vector3::new(0.0, 1.0, 0.0), Vector3::new(0.0, 1.0, 0.5)],
                        interpolation: Interpolation::Linear,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let out = retarget_clip(&clip, &src, &dst).expect("both humanoid");
        assert!(
            out.curves.iter().all(|c| !c.path.starts_with("mixamo")),
            "target paths"
        );
        for t in [0.0, 0.5, 1.0] {
            pose(&mut w, a_root, &clip, t);
            pose(&mut w, b_root, &out, t);
            // Same motion, twice the size: every joint lands at twice A's position.
            for (i, (&ai, &bi)) in a.iter().zip(&b).enumerate() {
                let (pa, pb) = (w.position(ai), w.position(bi));
                assert!(
                    (pa * 2.0 - pb).length() < 2e-3,
                    "t {t}, bone {}: {pa:?} ×2 vs {pb:?}",
                    BONES[i].1
                );
            }
        }
        // The left hand really moved (the arm swung down).
        let hand = w.position(a[9]);
        assert!(hand.y < 1.5, "{hand:?}");
    }

    #[test]
    fn non_humanoid_rigs_are_not_retargeted() {
        let mut w = World::new();
        let r = w.create("Prop");
        let c = w.create("Lid");
        w.set_parent(c, Some(r), false).unwrap();
        let prop = Avatar::from_world(&w, r);
        assert!(!prop.is_human());
        assert!(retarget_clip(&AnimationClip::default(), &prop, &prop).is_none());
        assert_eq!(
            clip_source_model("Assets/Run.fbx#Run"),
            Some("Assets/Run.fbx")
        );
        assert_eq!(clip_source_model("Assets/Walk.anim"), None);
    }

    /// An Animator on rig B plays a clip imported with rig A's model: it is retargeted.
    #[test]
    fn animators_retarget_clips_from_other_rigs() {
        use oxide_assets::{AnimatorController, AssetServer, Motion};
        let mut w = World::new();
        w.resources.insert(oxide_core::Time::new());
        let (a_root, a) = rig(&mut w, 0, 1.0, |_| Quaternion::IDENTITY);
        let (b_root, b) = rig(&mut w, 1, 2.0, |i| {
            Quaternion::euler(0.0, 90.0, i as f32 * 13.0)
        });
        // Rig A as an imported model, its clip "Assets/A.fbx#Wave".
        let model = oxide_assets::Model {
            name: "A".into(),
            nodes: a
                .iter()
                .map(|&id| {
                    let t = w.transform(id).unwrap();
                    oxide_assets::ModelNode {
                        name: w.get(id).unwrap().name.clone(),
                        position: t.local_position,
                        rotation: t.local_rotation,
                        scale: t.local_scale,
                        children: a
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| w.parent(**c) == Some(id))
                            .map(|(k, _)| k)
                            .collect(),
                        mesh: None,
                        materials: Vec::new(),
                        skin: None,
                    }
                })
                .collect(),
            roots: vec![0],
            skins: Vec::new(),
            animations: Vec::new(),
        };
        let src = Avatar::from_model(&model);
        let mut clip = AnimationClip {
            name: "Wave".into(),
            length: 1.0,
            curves: vec![TransformCurves {
                path: src.paths[src.human[&HumanBodyBones::LeftUpperArm]].clone(),
                rotation: Some(Track {
                    times: vec![0.0, 1.0],
                    values: vec![Quaternion::IDENTITY, Quaternion::euler(0.0, 0.0, 80.0)],
                    interpolation: Interpolation::Linear,
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        clip.looping = false;
        let mut assets = AssetServer::new(".");
        assets.models.insert("Assets/A.fbx", model);
        let h = assets.animations.insert("Assets/A.fbx#Wave", clip.clone());
        let mut ctrl = AnimatorController::default();
        ctrl.add_state("Wave", Motion::Clip(h));
        let ch = assets.controllers.insert("ctrl", ctrl);
        w.resources.insert(assets);
        let mut sys = crate::install(&mut w);
        w.add_component(b_root, crate::Animator::new(ch));
        // Four frames: 0.96 s into the clip.
        for _ in 0..4 {
            w.resource_mut::<oxide_core::Time>().begin_frame(0.24);
            sys.update(&mut w);
        }
        let retargeted = w
            .get_component::<crate::Animator>(b_root)
            .unwrap()
            .rt
            .retargeted
            .len();
        assert_eq!(retargeted, 1, "the clip was retargeted for rig B");
        // Rig A posed by the original clip at the same time: B's hand follows at twice the
        // size.
        pose(&mut w, a_root, &clip, 0.96);
        let (ha, hb) = (w.position(a[9]), w.position(b[9]));
        assert!((ha * 2.0 - hb).length() < 0.01, "{ha:?} ×2 vs {hb:?}");
        assert!(ha.y < 1.5, "the arm swung down: {ha:?}");
    }
}
