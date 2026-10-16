//! Humanoid rigs: Unity's `HumanBodyBones`, mapped automatically from bone names the way
//! Unity's Avatar configuration does on import. Recognizes Mixamo (`mixamorig:LeftForeArm`),
//! Unreal (`lowerarm_l`), Blender/Rigify (`forearm.L`) and common generic names.
//! Reference: <https://docs.unity3d.com/Manual/ConfiguringtheAvatar.html>

use oxide_core::Reflect;
use oxide_scene::{GameObjectId, World};
use std::collections::HashMap;

/// Unity `HumanBodyBones` (body bones; fingers are not mapped).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Reflect)]
pub enum HumanBodyBones {
    #[default]
    Hips,
    LeftUpperLeg,
    RightUpperLeg,
    LeftLowerLeg,
    RightLowerLeg,
    LeftFoot,
    RightFoot,
    Spine,
    Chest,
    Neck,
    Head,
    LeftShoulder,
    RightShoulder,
    LeftUpperArm,
    RightUpperArm,
    LeftLowerArm,
    RightLowerArm,
    LeftHand,
    RightHand,
    LeftToes,
    RightToes,
    UpperChest,
}

impl HumanBodyBones {
    pub const ALL: [HumanBodyBones; 22] = [
        Self::Hips,
        Self::LeftUpperLeg,
        Self::RightUpperLeg,
        Self::LeftLowerLeg,
        Self::RightLowerLeg,
        Self::LeftFoot,
        Self::RightFoot,
        Self::Spine,
        Self::Chest,
        Self::Neck,
        Self::Head,
        Self::LeftShoulder,
        Self::RightShoulder,
        Self::LeftUpperArm,
        Self::RightUpperArm,
        Self::LeftLowerArm,
        Self::RightLowerArm,
        Self::LeftHand,
        Self::RightHand,
        Self::LeftToes,
        Self::RightToes,
        Self::UpperChest,
    ];

    /// Bones Unity requires for a valid humanoid Avatar.
    pub const REQUIRED: [HumanBodyBones; 15] = [
        Self::Hips,
        Self::LeftUpperLeg,
        Self::RightUpperLeg,
        Self::LeftLowerLeg,
        Self::RightLowerLeg,
        Self::LeftFoot,
        Self::RightFoot,
        Self::Spine,
        Self::Head,
        Self::LeftUpperArm,
        Self::RightUpperArm,
        Self::LeftLowerArm,
        Self::RightLowerArm,
        Self::LeftHand,
        Self::RightHand,
    ];

    pub fn name(self) -> String {
        format!("{self:?}")
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|b| b.name().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

/// Unsided body parts, and limb parts that get a side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Hips,
    /// Spine segment; the number orders segments (`spine`, `spine1`, `spine_02`, ...).
    Spine(u32),
    Chest,
    UpperChest,
    Neck,
    Head,
    Shoulder,
    UpperArm,
    LowerArm,
    Hand,
    UpperLeg,
    LowerLeg,
    Foot,
    Toes,
}

fn classify(name: &str) -> Option<(Option<Side>, Part)> {
    let base = name.rsplit([':', '|']).next().unwrap_or(name);
    let n: String = base
        .chars()
        .map(|c| match c {
            '.' | '-' | ' ' => '_',
            c => c.to_ascii_lowercase(),
        })
        .collect();
    let n = n
        .trim_start_matches("mixamorig_")
        .trim_start_matches("mixamorig")
        .trim_start_matches("bip01_")
        .trim_start_matches("bip001_")
        .trim_start_matches("def_")
        .trim_start_matches("b_")
        .trim_start_matches("j_")
        .to_owned();
    const SKIP: [&str; 12] = [
        "index", "middle", "ring", "pinky", "little", "thumb", "finger", "twist", "end", "ik",
        "pole", "target",
    ];
    if SKIP.iter().any(|s| n.contains(s)) || n.contains("headtop") {
        return None;
    }
    // Side: a `left`/`right` word, or an `l`/`r` token (`upperarm_l`, `l_hand`, `forearm.L`).
    let mut tokens: Vec<String> = n
        .split('_')
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect();
    let mut side = None;
    tokens.retain(|t| match t.as_str() {
        "l" | "left" => {
            side = Some(Side::Left);
            false
        }
        "r" | "right" => {
            side = Some(Side::Right);
            false
        }
        _ => true,
    });
    let mut core = tokens.concat();
    for (word, s) in [("left", Side::Left), ("right", Side::Right)] {
        if let Some(rest) = core.strip_prefix(word) {
            side = Some(s);
            core = rest.to_owned();
        } else if let Some(rest) = core.strip_suffix(word) {
            side = Some(s);
            core = rest.to_owned();
        }
    }
    let digits: String = core
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect();
    let num: u32 = digits
        .chars()
        .rev()
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    let core = core.trim_end_matches(|c: char| c.is_ascii_digit());
    let part = match side {
        Some(_) => match core {
            "shoulder" | "clavicle" | "collar" | "collarbone" => Part::Shoulder,
            "upperarm" | "arm" | "uparm" | "humerus" => Part::UpperArm,
            "forearm" | "lowerarm" | "elbow" | "loarm" => Part::LowerArm,
            "hand" | "wrist" => Part::Hand,
            "upleg" | "upperleg" | "thigh" | "hip" | "femur" => Part::UpperLeg,
            "leg" | "lowerleg" | "shin" | "calf" | "knee" | "loleg" => Part::LowerLeg,
            "foot" | "ankle" => Part::Foot,
            "toebase" | "toe" | "toes" | "ball" => Part::Toes,
            _ => return None,
        },
        None => match core {
            "hips" | "pelvis" | "hip" | "root_hips" => Part::Hips,
            "spine" | "torso" | "abdomen" => Part::Spine(num),
            "chest" => Part::Chest,
            "upperchest" => Part::UpperChest,
            "neck" => Part::Neck,
            "head" => Part::Head,
            _ => return None,
        },
    };
    Some((side, part))
}

fn bone_for(side: Option<Side>, part: Part) -> Option<HumanBodyBones> {
    use HumanBodyBones as B;
    let lr = |l, r| match side {
        Some(Side::Left) => Some(l),
        Some(Side::Right) => Some(r),
        None => None,
    };
    match part {
        Part::Hips => Some(B::Hips),
        Part::Spine(_) => Some(B::Spine),
        Part::Chest => Some(B::Chest),
        Part::UpperChest => Some(B::UpperChest),
        Part::Neck => Some(B::Neck),
        Part::Head => Some(B::Head),
        Part::Shoulder => lr(B::LeftShoulder, B::RightShoulder),
        Part::UpperArm => lr(B::LeftUpperArm, B::RightUpperArm),
        Part::LowerArm => lr(B::LeftLowerArm, B::RightLowerArm),
        Part::Hand => lr(B::LeftHand, B::RightHand),
        Part::UpperLeg => lr(B::LeftUpperLeg, B::RightUpperLeg),
        Part::LowerLeg => lr(B::LeftLowerLeg, B::RightLowerLeg),
        Part::Foot => lr(B::LeftFoot, B::RightFoot),
        Part::Toes => lr(B::LeftToes, B::RightToes),
    }
}

/// Maps the humanoid bones below `root` by name. Spine segments (in hierarchy order) become
/// Spine, Chest and UpperChest when there is no explicit chest bone.
pub fn auto_map(world: &World, root: GameObjectId) -> HashMap<HumanBodyBones, GameObjectId> {
    let mut map = HashMap::new();
    let mut spines = Vec::new();
    for id in world.descendants(root) {
        let Some(o) = world.get(id) else { continue };
        let Some((side, part)) = classify(&o.name) else {
            continue;
        };
        if let Part::Spine(_) = part {
            spines.push(id);
            continue;
        }
        if let Some(b) = bone_for(side, part) {
            map.entry(b).or_insert(id);
        }
    }
    // Spine chain: first → Spine, then Chest, then the last of 3+ → UpperChest.
    let hips = map.get(&HumanBodyBones::Hips).copied();
    spines.retain(|&s| hips.is_none_or(|h| is_below(world, s, h)));
    if let Some(&s) = spines.first() {
        map.entry(HumanBodyBones::Spine).or_insert(s);
    }
    if let Some(&s) = spines.get(1) {
        map.entry(HumanBodyBones::Chest).or_insert(s);
    }
    if spines.len() >= 3 {
        map.entry(HumanBodyBones::UpperChest)
            .or_insert(*spines.last().expect("len >= 3"));
    }
    map
}

fn is_below(world: &World, id: GameObjectId, ancestor: GameObjectId) -> bool {
    let mut cur = world.parent(id);
    while let Some(c) = cur {
        if c == ancestor {
            return true;
        }
        cur = world.parent(c);
    }
    false
}

/// Whether a bone map has everything Unity requires of a humanoid.
pub fn is_human(map: &HashMap<HumanBodyBones, GameObjectId>) -> bool {
    HumanBodyBones::REQUIRED.iter().all(|b| map.contains_key(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use HumanBodyBones as B;

    fn one(name: &str) -> Option<HumanBodyBones> {
        classify(name).and_then(|(s, p)| bone_for(s, p))
    }

    #[test]
    fn naming_conventions() {
        // Mixamo
        assert_eq!(one("mixamorig:Hips"), Some(B::Hips));
        assert_eq!(one("mixamorig:LeftArm"), Some(B::LeftUpperArm));
        assert_eq!(one("mixamorig:LeftForeArm"), Some(B::LeftLowerArm));
        assert_eq!(one("mixamorig:RightUpLeg"), Some(B::RightUpperLeg));
        assert_eq!(one("mixamorig:RightLeg"), Some(B::RightLowerLeg));
        assert_eq!(one("mixamorig:LeftToeBase"), Some(B::LeftToes));
        assert_eq!(one("mixamorig:LeftHandIndex1"), None);
        assert_eq!(one("mixamorig:HeadTop_End"), None);
        // Unreal
        assert_eq!(one("upperarm_l"), Some(B::LeftUpperArm));
        assert_eq!(one("lowerarm_r"), Some(B::RightLowerArm));
        assert_eq!(one("calf_l"), Some(B::LeftLowerLeg));
        assert_eq!(one("clavicle_r"), Some(B::RightShoulder));
        assert_eq!(one("upperarm_twist_01_l"), None);
        assert_eq!(one("pelvis"), Some(B::Hips));
        // Blender / Rigify
        assert_eq!(one("thigh.L"), Some(B::LeftUpperLeg));
        assert_eq!(one("shin.R"), Some(B::RightLowerLeg));
        assert_eq!(one("forearm.L"), Some(B::LeftLowerArm));
        assert_eq!(one("DEF-hand.R"), Some(B::RightHand));
        // Generic
        assert_eq!(one("Left Foot"), Some(B::LeftFoot));
        assert_eq!(one("Neck"), Some(B::Neck));
        assert_eq!(one("Root"), None);
    }

    #[test]
    fn maps_a_mixamo_hierarchy() {
        let mut w = World::new();
        let root = w.create("Soldier");
        let add = |w: &mut World, name: &str, parent| {
            let id = w.create(name);
            w.set_parent(id, Some(parent), false).unwrap();
            id
        };
        let hips = add(&mut w, "mixamorig:Hips", root);
        let s0 = add(&mut w, "mixamorig:Spine", hips);
        let s1 = add(&mut w, "mixamorig:Spine1", s0);
        let s2 = add(&mut w, "mixamorig:Spine2", s1);
        let neck = add(&mut w, "mixamorig:Neck", s2);
        add(&mut w, "mixamorig:Head", neck);
        for side in ["Left", "Right"] {
            let sh = add(&mut w, &format!("mixamorig:{side}Shoulder"), s2);
            let a = add(&mut w, &format!("mixamorig:{side}Arm"), sh);
            let f = add(&mut w, &format!("mixamorig:{side}ForeArm"), a);
            add(&mut w, &format!("mixamorig:{side}Hand"), f);
            let u = add(&mut w, &format!("mixamorig:{side}UpLeg"), hips);
            let l = add(&mut w, &format!("mixamorig:{side}Leg"), u);
            add(&mut w, &format!("mixamorig:{side}Foot"), l);
        }
        let m = auto_map(&w, root);
        assert!(is_human(&m));
        assert_eq!(m[&B::Spine], s0);
        assert_eq!(m[&B::Chest], s1);
        assert_eq!(m[&B::UpperChest], s2);
    }
}
