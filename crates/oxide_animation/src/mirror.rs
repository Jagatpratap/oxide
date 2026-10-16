//! Mirroring motions left/right (Unity's Mirror on states and blend tree children).
//!
//! Unity mirrors humanoid motions in muscle space. Here any rig works: bones pair up by
//! name (`LeftArm`/`RightArm`, `arm.L`/`arm.R`, `l_hand`/`r_hand`, ...), each sampled pose
//! is reflected across the character's YZ plane in model space, and a per-bone correction
//! (from the rest pose) maps the reflected frame onto the counterpart bone's own axes, so
//! rigs whose left and right bones have different local axes still mirror correctly.

use oxide_math::{Matrix4x4, Quaternion, Vector3};
use oxide_scene::{GameObjectId, World};
use std::collections::HashMap;

/// One sampled transform: local position, rotation and scale (`None` = not animated).
pub(crate) type Sample = (
    GameObjectId,
    Option<Vector3>,
    Option<Quaternion>,
    Option<Vector3>,
);

#[derive(Clone, Debug, Default)]
pub struct MirrorMap {
    bones: Vec<GameObjectId>,
    /// Index of each bone's parent in `bones` (`None` = the animated root).
    parent: Vec<Option<usize>>,
    index: HashMap<GameObjectId, usize>,
    rest: Vec<(Vector3, Quaternion, Vector3)>,
    counterpart: Vec<usize>,
    correction: Vec<Quaternion>,
}

/// Reflection across the YZ plane, as a rotation: `S R S` for `S = diag(-1, 1, 1)`.
fn refl(q: Quaternion) -> Quaternion {
    Quaternion::from_xyzw(q.x, -q.y, -q.z, q.w)
}

/// The other side's name, if `name` names a left or right part.
pub fn mirrored_name(name: &str) -> Option<String> {
    for (l, r) in [("Left", "Right"), ("left", "right"), ("LEFT", "RIGHT")] {
        for (a, b) in [(l, r), (r, l)] {
            if name.contains(a) {
                return Some(name.replacen(a, b, 1));
            }
        }
    }
    let swap = |c: char| match c {
        'L' => 'R',
        'R' => 'L',
        'l' => 'r',
        'r' => 'l',
        c => c,
    };
    let mut ch = name.chars();
    // Suffixes: `arm.L`, `thigh_r`, `hand-L`.
    if let (Some(last), Some(sep)) = (ch.next_back(), ch.next_back())
        && matches!(sep, '.' | '_' | '-' | ' ')
        && matches!(last, 'L' | 'R' | 'l' | 'r')
    {
        return Some(format!("{}{sep}{}", &name[..name.len() - 2], swap(last)));
    }
    // Prefixes: `L_Foot`, `r.hand`.
    let mut ch = name.chars();
    if let (Some(first), Some(sep)) = (ch.next(), ch.next())
        && matches!(sep, '.' | '_' | '-' | ' ')
        && matches!(first, 'L' | 'R' | 'l' | 'r')
    {
        return Some(format!("{}{sep}{}", swap(first), &name[2..]));
    }
    None
}

impl MirrorMap {
    /// Builds the map from the hierarchy below `root` as it is now (call before animating,
    /// so the rest pose is the bind pose).
    pub fn build(world: &World, root: GameObjectId) -> Self {
        let mut m = Self::default();
        let mut stack: Vec<(GameObjectId, Option<usize>)> = world
            .children(root)
            .iter()
            .rev()
            .map(|c| (*c, None))
            .collect();
        while let Some((id, parent)) = stack.pop() {
            let i = m.bones.len();
            m.bones.push(id);
            m.parent.push(parent);
            m.index.insert(id, i);
            let t = world.transform(id);
            m.rest.push(
                t.map_or((Vector3::ZERO, Quaternion::IDENTITY, Vector3::ONE), |t| {
                    (t.local_position, t.local_rotation, t.local_scale)
                }),
            );
            for c in world.children(id).iter().rev() {
                stack.push((*c, Some(i)));
            }
        }
        let names: Vec<String> = m
            .bones
            .iter()
            .map(|id| world.get(*id).map(|o| o.name.clone()).unwrap_or_default())
            .collect();
        let by_name: HashMap<&str, usize> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        m.counterpart = names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                mirrored_name(n)
                    .and_then(|o| by_name.get(o.as_str()).copied())
                    .unwrap_or(i)
            })
            .collect();
        let rest_model = m.model(&m.rest_locals());
        m.correction = (0..m.bones.len())
            .map(|t| {
                let s = m.counterpart[t];
                let (_, rs, _) = rest_model[s].to_scale_rotation_translation();
                let (_, rt, _) = rest_model[t].to_scale_rotation_translation();
                (refl(rs).inverse() * rt).normalize()
            })
            .collect();
        m
    }

    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }

    fn rest_locals(&self) -> Vec<(Vector3, Quaternion, Vector3)> {
        self.rest.clone()
    }

    /// Model-space (root-relative) matrices of every bone.
    fn model(&self, locals: &[(Vector3, Quaternion, Vector3)]) -> Vec<Matrix4x4> {
        let mut out: Vec<Matrix4x4> = Vec::with_capacity(self.bones.len());
        for (i, (p, r, s)) in locals.iter().enumerate() {
            let local = Matrix4x4::from_scale_rotation_translation(*s, *r, *p);
            out.push(match self.parent[i] {
                Some(pi) => out[pi] * local,
                None => local,
            });
        }
        out
    }

    /// Replaces `samples` (one clip's sampled pose) with its mirror image.
    pub(crate) fn apply(&self, samples: &mut Vec<Sample>) {
        if self.bones.is_empty() {
            return;
        }
        let mut locals = self.rest_locals();
        let mut animated: Vec<(bool, bool, bool)> = vec![(false, false, false); self.bones.len()];
        let mut others: Vec<Sample> = vec![];
        for (id, p, r, s) in samples.drain(..) {
            let Some(&i) = self.index.get(&id) else {
                // The root itself (or something outside the rig): mirror in place.
                others.push((id, p.map(|p| Vector3::new(-p.x, p.y, p.z)), r.map(refl), s));
                continue;
            };
            if let Some(p) = p {
                locals[i].0 = p;
                animated[i].0 = true;
            }
            if let Some(r) = r {
                locals[i].1 = r;
                animated[i].1 = true;
            }
            if let Some(s) = s {
                locals[i].2 = s;
                animated[i].2 = true;
            }
        }
        let model = self.model(&locals);
        // Mirrored model matrices: bone t takes its counterpart's reflected transform.
        let mirrored: Vec<Matrix4x4> = (0..self.bones.len())
            .map(|t| {
                let s = self.counterpart[t];
                let (sc, r, p) = model[s].to_scale_rotation_translation();
                Matrix4x4::from_scale_rotation_translation(
                    sc,
                    (refl(r) * self.correction[t]).normalize(),
                    Vector3::new(-p.x, p.y, p.z),
                )
            })
            .collect();
        for t in 0..self.bones.len() {
            let s = self.counterpart[t];
            let (ap, ar, asc) = animated[s];
            if !(ap || ar || asc) {
                continue;
            }
            let local = match self.parent[t] {
                Some(pi) => mirrored[pi].inverse() * mirrored[t],
                None => mirrored[t],
            };
            let (lsc, lr, lp) = local.to_scale_rotation_translation();
            samples.push((
                self.bones[t],
                ap.then_some(lp),
                ar.then_some(lr.normalize()),
                asc.then_some(lsc),
            ));
        }
        samples.extend(others);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_pair_up() {
        assert_eq!(mirrored_name("LeftArm").as_deref(), Some("RightArm"));
        assert_eq!(
            mirrored_name("mixamorig:RightHand").as_deref(),
            Some("mixamorig:LeftHand")
        );
        assert_eq!(mirrored_name("upper_arm.L").as_deref(), Some("upper_arm.R"));
        assert_eq!(mirrored_name("thigh_r").as_deref(), Some("thigh_l"));
        assert_eq!(mirrored_name("L_Foot").as_deref(), Some("R_Foot"));
        assert_eq!(mirrored_name("Spine"), None);
        assert_eq!(mirrored_name("Head"), None);
    }

    #[test]
    fn mirroring_swaps_and_reflects() {
        let mut w = World::new();
        let root = w.create("Root");
        let l = w.create("LeftArm");
        let r = w.create("RightArm");
        w.set_parent(l, Some(root), false).unwrap();
        w.set_parent(r, Some(root), false).unwrap();
        w.transform_mut(l).unwrap().local_position = Vector3::new(-1.0, 0.0, 0.0);
        w.transform_mut(r).unwrap().local_position = Vector3::new(1.0, 0.0, 0.0);
        let m = MirrorMap::build(&w, root);
        // Left arm raised forward 90° about X; mirrored, the right arm does the same.
        let q = Quaternion::from_rotation_x(1.0);
        let mut s: Vec<Sample> = vec![(l, Some(Vector3::new(-1.0, 0.5, 0.0)), Some(q), None)];
        m.apply(&mut s);
        let (id, p, rot, _) = s[0];
        assert_eq!(id, r);
        assert!((p.unwrap() - Vector3::new(1.0, 0.5, 0.0)).length() < 1e-4);
        assert!(rot.unwrap().angle_between(q) < 1e-4);
        // A yaw to the left becomes a yaw to the right.
        let mut s: Vec<Sample> = vec![(l, None, Some(Quaternion::from_rotation_y(0.5)), None)];
        m.apply(&mut s);
        assert!(
            s[0].2
                .unwrap()
                .angle_between(Quaternion::from_rotation_y(-0.5))
                < 1e-4
        );
    }
}
