//! Unity's constraint components: Position, Rotation, Scale, Parent, Aim and Look At.
//! Each one drives its object's transform from a weighted list of source objects.
//! Reference: <https://docs.unity3d.com/Manual/constraint-components.html>
//!
//! The values at rest are the transform used where the constraint has no effect (Weight 0,
//! or an axis that isn't constrained). Offsets are added on top of the sources' result.
//! When a constraint is active but not locked, moving the constrained object (with a tool
//! or a script) updates its offsets instead of snapping it back, as in Unity.

use crate::component::Component;
use crate::id::GameObjectId;
use crate::world::World;
use oxide_core::Reflect;
use oxide_math::{Matrix4x4, Quaternion, QuaternionExt, Vector3};

/// Unity `Axis` flags (which axes a constraint affects).
#[derive(Reflect, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConstraintAxes {
    pub x: bool,
    pub y: bool,
    pub z: bool,
}

impl Default for ConstraintAxes {
    fn default() -> Self {
        Self::ALL
    }
}

impl ConstraintAxes {
    pub const ALL: Self = Self {
        x: true,
        y: true,
        z: true,
    };

    /// Per component: the constrained value on affected axes, `rest` on the others.
    fn pick(self, constrained: Vector3, rest: Vector3) -> Vector3 {
        Vector3::new(
            if self.x { constrained.x } else { rest.x },
            if self.y { constrained.y } else { rest.y },
            if self.z { constrained.z } else { rest.z },
        )
    }
}

/// Unity `ConstraintSource`: a source object and its weight.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct ConstraintSource {
    pub source_transform: Option<GameObjectId>,
    #[reflect(range(0, 1))]
    pub weight: f32,
}

impl ConstraintSource {
    pub fn new(source: GameObjectId, weight: f32) -> Self {
        Self {
            source_transform: Some(source),
            weight,
        }
    }
}

/// Unity `Aim Constraint` / `Look At Constraint` world up type.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorldUpType {
    /// World +Y.
    #[default]
    SceneUp,
    /// Toward the World Up Object.
    ObjectUp,
    /// The World Up Vector rotated by the World Up Object.
    ObjectRotationUp,
    /// The World Up Vector.
    Vector,
    /// No up direction: the shortest rotation that aims.
    None,
}

/// Unity `PositionConstraint`.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct PositionConstraint {
    pub constraint_active: bool,
    pub locked: bool,
    #[reflect(range(0, 1))]
    pub weight: f32,
    /// Local position used at Weight 0 and on unconstrained axes.
    pub translation_at_rest: Vector3,
    /// Added to the sources' position (in the parent's space).
    pub translation_offset: Vector3,
    pub translation_axis: ConstraintAxes,
    pub sources: Vec<ConstraintSource>,
    #[reflect(skip)]
    applied: Option<Applied>,
}

/// Unity `RotationConstraint`.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct RotationConstraint {
    pub constraint_active: bool,
    pub locked: bool,
    #[reflect(range(0, 1))]
    pub weight: f32,
    /// Local Euler angles used at Weight 0 and on unconstrained axes.
    pub rotation_at_rest: Vector3,
    /// Euler angles applied after the sources' rotation.
    pub rotation_offset: Vector3,
    pub rotation_axis: ConstraintAxes,
    pub sources: Vec<ConstraintSource>,
    #[reflect(skip)]
    applied: Option<Applied>,
}

/// Unity `ScaleConstraint`.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct ScaleConstraint {
    pub constraint_active: bool,
    pub locked: bool,
    #[reflect(range(0, 1))]
    pub weight: f32,
    pub scale_at_rest: Vector3,
    /// Multiplies the sources' scale.
    pub scale_offset: Vector3,
    pub scaling_axis: ConstraintAxes,
    pub sources: Vec<ConstraintSource>,
    #[reflect(skip)]
    applied: Option<Applied>,
}

/// One source's offsets in a Parent Constraint (Unity keeps them in parallel arrays).
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct ParentConstraintOffset {
    /// In the source's space.
    pub translation: Vector3,
    /// Euler angles, in the source's space.
    pub rotation: Vector3,
}

/// Unity `ParentConstraint`: moves and rotates as if it were a child of the sources.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct ParentConstraint {
    pub constraint_active: bool,
    pub locked: bool,
    #[reflect(range(0, 1))]
    pub weight: f32,
    pub translation_at_rest: Vector3,
    pub rotation_at_rest: Vector3,
    pub translation_axis: ConstraintAxes,
    pub rotation_axis: ConstraintAxes,
    pub sources: Vec<ConstraintSource>,
    /// One per source (missing entries count as zero).
    pub source_offsets: Vec<ParentConstraintOffset>,
    #[reflect(skip)]
    applied: Option<Applied>,
}

/// Unity `AimConstraint`: turns so `aim_vector` points at the sources.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct AimConstraint {
    pub constraint_active: bool,
    pub locked: bool,
    #[reflect(range(0, 1))]
    pub weight: f32,
    pub rotation_at_rest: Vector3,
    pub rotation_offset: Vector3,
    pub rotation_axis: ConstraintAxes,
    /// Local axis that points at the target.
    pub aim_vector: Vector3,
    /// Local axis that points toward the world up direction.
    pub up_vector: Vector3,
    pub world_up_type: WorldUpType,
    pub world_up_vector: Vector3,
    pub world_up_object: Option<GameObjectId>,
    pub sources: Vec<ConstraintSource>,
    #[reflect(skip)]
    applied: Option<Applied>,
}

/// Unity `LookAtConstraint`: a simpler Aim Constraint that always aims +Z, with roll.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct LookAtConstraint {
    pub constraint_active: bool,
    pub locked: bool,
    #[reflect(range(0, 1))]
    pub weight: f32,
    /// Degrees around the aim direction.
    pub roll: f32,
    pub rotation_at_rest: Vector3,
    pub rotation_offset: Vector3,
    /// Use World Up Object's up instead of world +Y.
    pub use_up_object: bool,
    pub world_up_object: Option<GameObjectId>,
    pub sources: Vec<ConstraintSource>,
    #[reflect(skip)]
    applied: Option<Applied>,
}

/// The local transform a constraint wrote last time, to notice when something else moved
/// the object.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Applied {
    position: Vector3,
    rotation: Quaternion,
    scale: Vector3,
}

macro_rules! constraint_defaults {
    ($($t:ident { $($f:ident : $v:expr),* $(,)? })*) => {$(
        impl Default for $t {
            fn default() -> Self {
                Self {
                    constraint_active: false,
                    locked: false,
                    weight: 1.0,
                    sources: Vec::new(),
                    applied: None,
                    $($f: $v),*
                }
            }
        }
        impl Component for $t {}
    )*};
}

constraint_defaults! {
    PositionConstraint {
        translation_at_rest: Vector3::ZERO,
        translation_offset: Vector3::ZERO,
        translation_axis: ConstraintAxes::ALL,
    }
    RotationConstraint {
        rotation_at_rest: Vector3::ZERO,
        rotation_offset: Vector3::ZERO,
        rotation_axis: ConstraintAxes::ALL,
    }
    ScaleConstraint {
        scale_at_rest: Vector3::ONE,
        scale_offset: Vector3::ONE,
        scaling_axis: ConstraintAxes::ALL,
    }
    ParentConstraint {
        translation_at_rest: Vector3::ZERO,
        rotation_at_rest: Vector3::ZERO,
        translation_axis: ConstraintAxes::ALL,
        rotation_axis: ConstraintAxes::ALL,
        source_offsets: Vec::new(),
    }
    AimConstraint {
        rotation_at_rest: Vector3::ZERO,
        rotation_offset: Vector3::ZERO,
        rotation_axis: ConstraintAxes::ALL,
        aim_vector: Vector3::Z,
        up_vector: Vector3::Y,
        world_up_type: WorldUpType::SceneUp,
        world_up_vector: Vector3::Y,
        world_up_object: None,
    }
    LookAtConstraint {
        roll: 0.0,
        rotation_at_rest: Vector3::ZERO,
        rotation_offset: Vector3::ZERO,
        use_up_object: false,
        world_up_object: None,
    }
}

/// Which constraint type; used by the generic helpers (Activate, Zero, the MCP tool).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintKind {
    Position,
    Rotation,
    Scale,
    Parent,
    Aim,
    LookAt,
}

impl ConstraintKind {
    pub const ALL: [Self; 6] = [
        Self::Position,
        Self::Rotation,
        Self::Scale,
        Self::Parent,
        Self::Aim,
        Self::LookAt,
    ];

    /// Rust type name, as stored in scene files.
    pub fn type_name(self) -> &'static str {
        match self {
            Self::Position => "PositionConstraint",
            Self::Rotation => "RotationConstraint",
            Self::Scale => "ScaleConstraint",
            Self::Parent => "ParentConstraint",
            Self::Aim => "AimConstraint",
            Self::LookAt => "LookAtConstraint",
        }
    }

    pub fn from_type_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.type_name() == name || k.type_name().trim_end_matches("Constraint") == name)
    }
}

// ---- shared maths ------------------------------------------------------------------------

/// Sources with a live object and positive weight, weights normalised to sum to 1.
fn live_sources(world: &World, sources: &[ConstraintSource]) -> Vec<(usize, GameObjectId, f32)> {
    let mut out: Vec<(usize, GameObjectId, f32)> = sources
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let id = s.source_transform?;
            (world.get(id).is_some() && s.weight > 0.0).then_some((i, id, s.weight))
        })
        .collect();
    let total: f32 = out.iter().map(|s| s.2).sum();
    if total > 0.0 {
        for s in &mut out {
            s.2 /= total;
        }
    }
    out
}

/// Weighted average of unit quaternions (sign-aligned to the first).
fn average_rotation(items: impl IntoIterator<Item = (Quaternion, f32)>) -> Option<Quaternion> {
    let mut acc = oxide_math::glam::Vec4::ZERO;
    let mut first: Option<oxide_math::glam::Vec4> = None;
    for (q, w) in items {
        let mut v = oxide_math::glam::Vec4::from(q);
        let f = *first.get_or_insert(v);
        if v.dot(f) < 0.0 {
            v = -v;
        }
        acc += v * w;
    }
    (acc.length_squared() > 1e-12).then(|| Quaternion::from_vec4(acc).normalize())
}

fn parent_matrix(world: &World, id: GameObjectId) -> Matrix4x4 {
    world
        .parent(id)
        .map_or(Matrix4x4::IDENTITY, |p| world.local_to_world(p))
}

fn parent_rotation(world: &World, id: GameObjectId) -> Quaternion {
    world
        .parent(id)
        .map_or(Quaternion::IDENTITY, |p| world.rotation(p))
}

/// Euler-axis masking for rotations: unconstrained axes keep the rest angles.
fn pick_rotation(axes: ConstraintAxes, constrained: Quaternion, rest: Vector3) -> Quaternion {
    if axes == ConstraintAxes::ALL {
        return constrained;
    }
    let e = constrained.euler_angles();
    Quaternion::euler_v(axes.pick(e, rest))
}

/// Unity's aim: rotation turning local `aim` toward `dir`, with local `up` toward `world_up`.
fn aim_rotation(dir: Vector3, world_up: Option<Vector3>, aim: Vector3, up: Vector3) -> Quaternion {
    let aim = aim.normalize_or(Vector3::Z);
    let Some(world_up) = world_up else {
        return Quaternion::from_rotation_arc(aim, dir);
    };
    // Frame built from the local axes, and the same frame in world space.
    let local = Quaternion::look_rotation(aim, up);
    let target = Quaternion::look_rotation(dir, world_up);
    (target * local.inverse()).normalize()
}

fn local_transform(world: &World, id: GameObjectId) -> Option<Applied> {
    let t = world.transform(id)?;
    Some(Applied {
        position: t.local_position,
        rotation: t.local_rotation,
        scale: t.local_scale,
    })
}

/// Whether the object was moved since this constraint last wrote it.
fn moved(applied: Option<Applied>, now: Applied) -> bool {
    applied.is_some_and(|a| {
        (a.position - now.position).length_squared() > 1e-10
            || a.rotation.dot(now.rotation).abs() < 1.0 - 1e-6
            || (a.scale - now.scale).length_squared() > 1e-10
    })
}

// ---- per-type solve ----------------------------------------------------------------------

impl PositionConstraint {
    /// Sources' weighted position in the parent's space (before offset), or `None`.
    fn source_local(&self, world: &World, id: GameObjectId) -> Option<Vector3> {
        let src = live_sources(world, &self.sources);
        if src.is_empty() {
            return None;
        }
        let p: Vector3 = src.iter().map(|&(_, s, w)| world.position(s) * w).sum();
        Some(parent_matrix(world, id).inverse().transform_point3(p))
    }

    fn solve(&self, world: &World, id: GameObjectId, now: Applied) -> Option<Applied> {
        let target = self.source_local(world, id)? + self.translation_offset;
        let p = self
            .translation_at_rest
            .lerp(target, self.weight.clamp(0.0, 1.0));
        Some(Applied {
            position: self.translation_axis.pick(p, self.translation_at_rest),
            ..now
        })
    }

    pub fn activate(&mut self, world: &World, id: GameObjectId) {
        let Some(now) = local_transform(world, id) else {
            return;
        };
        self.translation_at_rest = now.position;
        if let Some(src) = self.source_local(world, id) {
            self.translation_offset = now.position - src;
        }
        self.constraint_active = true;
    }

    pub fn zero(&mut self, world: &World, id: GameObjectId) {
        if let Some(now) = local_transform(world, id) {
            self.translation_at_rest = now.position;
        }
        self.translation_offset = Vector3::ZERO;
        self.constraint_active = true;
    }
}

impl RotationConstraint {
    fn source_local(&self, world: &World, id: GameObjectId) -> Option<Quaternion> {
        let src = live_sources(world, &self.sources);
        let q = average_rotation(src.iter().map(|&(_, s, w)| (world.rotation(s), w)))?;
        Some((parent_rotation(world, id).inverse() * q).normalize())
    }

    fn solve(&self, world: &World, id: GameObjectId, now: Applied) -> Option<Applied> {
        let target = self.source_local(world, id)? * Quaternion::euler_v(self.rotation_offset);
        let rest = Quaternion::euler_v(self.rotation_at_rest);
        let q = rest.slerp(target, self.weight.clamp(0.0, 1.0));
        Some(Applied {
            rotation: pick_rotation(self.rotation_axis, q, self.rotation_at_rest),
            ..now
        })
    }

    pub fn activate(&mut self, world: &World, id: GameObjectId) {
        let Some(now) = local_transform(world, id) else {
            return;
        };
        self.rotation_at_rest = now.rotation.euler_angles();
        if let Some(src) = self.source_local(world, id) {
            self.rotation_offset = (src.inverse() * now.rotation).euler_angles();
        }
        self.constraint_active = true;
    }

    pub fn zero(&mut self, world: &World, id: GameObjectId) {
        if let Some(now) = local_transform(world, id) {
            self.rotation_at_rest = now.rotation.euler_angles();
        }
        self.rotation_offset = Vector3::ZERO;
        self.constraint_active = true;
    }
}

impl ScaleConstraint {
    fn source_scale(&self, world: &World) -> Option<Vector3> {
        let src = live_sources(world, &self.sources);
        (!src.is_empty()).then(|| src.iter().map(|&(_, s, w)| world.lossy_scale(s) * w).sum())
    }

    fn solve(&self, world: &World, _id: GameObjectId, now: Applied) -> Option<Applied> {
        let target = self.source_scale(world)? * self.scale_offset;
        let s = self.scale_at_rest.lerp(target, self.weight.clamp(0.0, 1.0));
        Some(Applied {
            scale: self.scaling_axis.pick(s, self.scale_at_rest),
            ..now
        })
    }

    pub fn activate(&mut self, world: &World, id: GameObjectId) {
        let Some(now) = local_transform(world, id) else {
            return;
        };
        self.scale_at_rest = now.scale;
        if let Some(src) = self.source_scale(world) {
            self.scale_offset = now.scale / src.max(Vector3::splat(1e-6));
        }
        self.constraint_active = true;
    }

    pub fn zero(&mut self, world: &World, id: GameObjectId) {
        if let Some(now) = local_transform(world, id) {
            self.scale_at_rest = now.scale;
        }
        self.scale_offset = Vector3::ONE;
        self.constraint_active = true;
    }
}

impl ParentConstraint {
    fn offset(&self, i: usize) -> ParentConstraintOffset {
        self.source_offsets.get(i).cloned().unwrap_or_default()
    }

    fn solve(&self, world: &World, id: GameObjectId, now: Applied) -> Option<Applied> {
        let src = live_sources(world, &self.sources);
        if src.is_empty() {
            return None;
        }
        let mut pos = Vector3::ZERO;
        let mut rots = Vec::with_capacity(src.len());
        for &(i, s, w) in &src {
            let o = self.offset(i);
            let m = world.local_to_world(s);
            pos += m.transform_point3(o.translation) * w;
            rots.push((world.rotation(s) * Quaternion::euler_v(o.rotation), w));
        }
        let rot = average_rotation(rots)?;
        let local_pos = parent_matrix(world, id).inverse().transform_point3(pos);
        let local_rot = (parent_rotation(world, id).inverse() * rot).normalize();
        let w = self.weight.clamp(0.0, 1.0);
        let p = self.translation_at_rest.lerp(local_pos, w);
        let q = Quaternion::euler_v(self.rotation_at_rest).slerp(local_rot, w);
        Some(Applied {
            position: self.translation_axis.pick(p, self.translation_at_rest),
            rotation: pick_rotation(self.rotation_axis, q, self.rotation_at_rest),
            scale: now.scale,
        })
    }

    pub fn activate(&mut self, world: &World, id: GameObjectId) {
        let Some(now) = local_transform(world, id) else {
            return;
        };
        self.translation_at_rest = now.position;
        self.rotation_at_rest = now.rotation.euler_angles();
        let wpos = world.position(id);
        let wrot = world.rotation(id);
        self.source_offsets = self
            .sources
            .iter()
            .map(
                |s| match s.source_transform.filter(|&s| world.get(s).is_some()) {
                    Some(s) => ParentConstraintOffset {
                        translation: world.local_to_world(s).inverse().transform_point3(wpos),
                        rotation: (world.rotation(s).inverse() * wrot).euler_angles(),
                    },
                    None => ParentConstraintOffset::default(),
                },
            )
            .collect();
        self.constraint_active = true;
    }

    pub fn zero(&mut self, world: &World, id: GameObjectId) {
        if let Some(now) = local_transform(world, id) {
            self.translation_at_rest = now.position;
            self.rotation_at_rest = now.rotation.euler_angles();
        }
        self.source_offsets = vec![ParentConstraintOffset::default(); self.sources.len()];
        self.constraint_active = true;
    }
}

/// Weighted target point of aim-style constraints (world space).
fn aim_target(world: &World, sources: &[ConstraintSource]) -> Option<Vector3> {
    let src = live_sources(world, sources);
    (!src.is_empty()).then(|| src.iter().map(|&(_, s, w)| world.position(s) * w).sum())
}

impl AimConstraint {
    fn world_up(&self, world: &World) -> Option<Vector3> {
        let obj = self.world_up_object.filter(|&o| world.get(o).is_some());
        match self.world_up_type {
            WorldUpType::SceneUp => Some(Vector3::Y),
            WorldUpType::Vector => Some(self.world_up_vector),
            WorldUpType::ObjectUp => obj.map(|o| world.position(o)), // turned into a direction below
            WorldUpType::ObjectRotationUp => Some(obj.map_or(self.world_up_vector, |o| {
                world.rotation(o) * self.world_up_vector
            })),
            WorldUpType::None => None,
        }
    }

    /// The aimed rotation in the parent's space, before the offset.
    fn source_local(&self, world: &World, id: GameObjectId) -> Option<Quaternion> {
        let target = aim_target(world, &self.sources)?;
        let me = world.position(id);
        let dir = (target - me).try_normalize()?;
        let up = match (self.world_up_type, self.world_up(world)) {
            (WorldUpType::ObjectUp, Some(p)) => Some((p - me).normalize_or(Vector3::Y)),
            (WorldUpType::ObjectUp, None) => Some(Vector3::Y),
            (_, up) => up,
        };
        let q = aim_rotation(dir, up, self.aim_vector, self.up_vector);
        Some((parent_rotation(world, id).inverse() * q).normalize())
    }

    fn solve(&self, world: &World, id: GameObjectId, now: Applied) -> Option<Applied> {
        let target = self.source_local(world, id)? * Quaternion::euler_v(self.rotation_offset);
        let q =
            Quaternion::euler_v(self.rotation_at_rest).slerp(target, self.weight.clamp(0.0, 1.0));
        Some(Applied {
            rotation: pick_rotation(self.rotation_axis, q, self.rotation_at_rest),
            ..now
        })
    }

    pub fn activate(&mut self, world: &World, id: GameObjectId) {
        let Some(now) = local_transform(world, id) else {
            return;
        };
        self.rotation_at_rest = now.rotation.euler_angles();
        if let Some(src) = self.source_local(world, id) {
            self.rotation_offset = (src.inverse() * now.rotation).euler_angles();
        }
        self.constraint_active = true;
    }

    pub fn zero(&mut self, world: &World, id: GameObjectId) {
        if let Some(now) = local_transform(world, id) {
            self.rotation_at_rest = now.rotation.euler_angles();
        }
        self.rotation_offset = Vector3::ZERO;
        self.constraint_active = true;
    }
}

impl LookAtConstraint {
    fn source_local(&self, world: &World, id: GameObjectId) -> Option<Quaternion> {
        let target = aim_target(world, &self.sources)?;
        let dir = (target - world.position(id)).try_normalize()?;
        let up = match self
            .world_up_object
            .filter(|&o| self.use_up_object && world.get(o).is_some())
        {
            Some(o) => world.up(o),
            None => Vector3::Y,
        };
        let q = Quaternion::look_rotation(dir, up)
            * Quaternion::from_rotation_z(self.roll.to_radians());
        Some((parent_rotation(world, id).inverse() * q).normalize())
    }

    fn solve(&self, world: &World, id: GameObjectId, now: Applied) -> Option<Applied> {
        let target = self.source_local(world, id)? * Quaternion::euler_v(self.rotation_offset);
        let q =
            Quaternion::euler_v(self.rotation_at_rest).slerp(target, self.weight.clamp(0.0, 1.0));
        Some(Applied { rotation: q, ..now })
    }

    pub fn activate(&mut self, world: &World, id: GameObjectId) {
        let Some(now) = local_transform(world, id) else {
            return;
        };
        self.rotation_at_rest = now.rotation.euler_angles();
        if let Some(src) = self.source_local(world, id) {
            self.rotation_offset = (src.inverse() * now.rotation).euler_angles();
        }
        self.constraint_active = true;
    }

    pub fn zero(&mut self, world: &World, id: GameObjectId) {
        if let Some(now) = local_transform(world, id) {
            self.rotation_at_rest = now.rotation.euler_angles();
        }
        self.rotation_offset = Vector3::ZERO;
        self.constraint_active = true;
    }
}

// ---- evaluation --------------------------------------------------------------------------

macro_rules! for_each_kind {
    ($m:ident) => {
        $m!(PositionConstraint);
        $m!(RotationConstraint);
        $m!(ScaleConstraint);
        $m!(ParentConstraint);
        $m!(AimConstraint);
        $m!(LookAtConstraint);
    };
}

/// Evaluates every active constraint once (Unity runs them after animation, before
/// `LateUpdate`). Objects are processed top-down, so a constrained child sees its
/// constrained parent's new transform.
pub fn evaluate_constraints(world: &mut World) {
    macro_rules! run {
        ($t:ident) => {
            let mut ids = world.query_ids::<$t>();
            ids.sort_by_key(|&(id, _)| depth(world, id));
            for (id, cid) in ids {
                let Some(c) = world.component::<$t>(id, cid) else {
                    continue;
                };
                if !c.constraint_active {
                    continue;
                }
                let Some(now) = local_transform(world, id) else {
                    continue;
                };
                let c = c.clone();
                if !c.locked && moved(c.applied, now) {
                    // Moved by hand while unlocked: keep the new pose, update the offsets.
                    let mut c = c;
                    c.activate(world, id);
                    c.applied = Some(now);
                    if let Some(dst) = world.component_mut::<$t>(id, cid) {
                        *dst = c;
                    }
                    continue;
                }
                let Some(out) = c.solve(world, id, now) else {
                    continue;
                };
                if let Some(t) = world.transform_mut(id) {
                    t.local_position = out.position;
                    t.local_rotation = out.rotation;
                    t.local_scale = out.scale;
                }
                if let Some(dst) = world.component_mut::<$t>(id, cid) {
                    dst.applied = Some(out);
                }
            }
        };
    }
    for_each_kind!(run);
}

fn depth(world: &World, id: GameObjectId) -> usize {
    let mut d = 0;
    let mut cur = world.parent(id);
    while let Some(p) = cur {
        d += 1;
        cur = world.parent(p);
    }
    d
}

/// Unity's Activate button: keep the object where it is and compute offsets from the sources.
/// Returns `false` if the object has no constraint of that kind.
pub fn activate(world: &mut World, id: GameObjectId, kind: ConstraintKind) -> bool {
    with_constraint(world, id, kind, |c, w, id| c.activate_dyn(w, id))
}

/// Unity's Zero button: clear the offsets so the object snaps onto the sources.
pub fn zero(world: &mut World, id: GameObjectId, kind: ConstraintKind) -> bool {
    with_constraint(world, id, kind, |c, w, id| c.zero_dyn(w, id))
}

trait ConstraintDyn {
    fn activate_dyn(&mut self, world: &World, id: GameObjectId);
    fn zero_dyn(&mut self, world: &World, id: GameObjectId);
}

macro_rules! impl_dyn {
    ($t:ident) => {
        impl ConstraintDyn for $t {
            fn activate_dyn(&mut self, world: &World, id: GameObjectId) {
                self.activate(world, id);
                self.applied = None;
            }
            fn zero_dyn(&mut self, world: &World, id: GameObjectId) {
                self.zero(world, id);
                self.applied = None;
            }
        }
    };
}
for_each_kind!(impl_dyn);

fn with_constraint(
    world: &mut World,
    id: GameObjectId,
    kind: ConstraintKind,
    f: impl Fn(&mut dyn ConstraintDyn, &World, GameObjectId),
) -> bool {
    macro_rules! go {
        ($t:ident) => {{
            let Some(cid) = world.component_id::<$t>(id) else {
                return false;
            };
            let Some(mut c) = world.component::<$t>(id, cid).cloned() else {
                return false;
            };
            f(&mut c, world, id);
            if let Some(dst) = world.component_mut::<$t>(id, cid) {
                *dst = c;
            }
            true
        }};
    }
    match kind {
        ConstraintKind::Position => go!(PositionConstraint),
        ConstraintKind::Rotation => go!(RotationConstraint),
        ConstraintKind::Scale => go!(ScaleConstraint),
        ConstraintKind::Parent => go!(ParentConstraint),
        ConstraintKind::Aim => go!(AimConstraint),
        ConstraintKind::LookAt => go!(LookAtConstraint),
    }
}

pub fn register(reg: &mut crate::ComponentRegistry) {
    reg.register::<AimConstraint>("Miscellaneous/Aim Constraint");
    reg.register::<LookAtConstraint>("Miscellaneous/Look At Constraint");
    reg.register::<ParentConstraint>("Miscellaneous/Parent Constraint");
    reg.register::<PositionConstraint>("Miscellaneous/Position Constraint");
    reg.register::<RotationConstraint>("Miscellaneous/Rotation Constraint");
    reg.register::<ScaleConstraint>("Miscellaneous/Scale Constraint");
}

/// Runs constraints after animation each frame, and in Edit mode (Unity previews them live).
#[derive(Default)]
pub struct ConstraintSystem {
    ran_in_update: bool,
}

impl crate::System for ConstraintSystem {
    fn name(&self) -> &'static str {
        "constraints"
    }
    fn update(&mut self, world: &mut World) {
        evaluate_constraints(world);
        self.ran_in_update = true;
    }
    fn always(&mut self, world: &mut World) {
        // Edit mode and pause have no `update`; in Play the pass above already ran.
        if !std::mem::take(&mut self.ran_in_update) {
            evaluate_constraints(world);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vector3, b: Vector3) -> bool {
        (a - b).length() < 1e-3
    }

    fn setup() -> (World, GameObjectId, GameObjectId, GameObjectId) {
        let mut w = World::new();
        crate::register_builtin(&mut w.registry);
        let a = w.create("A");
        let b = w.create("B");
        let me = w.create("Me");
        w.set_position(a, Vector3::new(2.0, 0.0, 0.0));
        w.set_position(b, Vector3::new(0.0, 4.0, 0.0));
        (w, a, b, me)
    }

    #[test]
    fn position_blends_sources_weight_and_axes() {
        let (mut w, a, b, me) = setup();
        let c = PositionConstraint {
            constraint_active: true,
            locked: true,
            sources: vec![ConstraintSource::new(a, 1.0), ConstraintSource::new(b, 1.0)],
            ..Default::default()
        };
        let cid = w.add_component(me, c).unwrap();
        evaluate_constraints(&mut w);
        assert!(near(w.position(me), Vector3::new(1.0, 2.0, 0.0)));
        // Half weight from rest (origin); Y not constrained.
        let c = w.component_mut::<PositionConstraint>(me, cid).unwrap();
        c.weight = 0.5;
        c.translation_axis.y = false;
        evaluate_constraints(&mut w);
        assert!(near(w.position(me), Vector3::new(0.5, 0.0, 0.0)));
    }

    #[test]
    fn activate_keeps_pose_and_follows_with_offset() {
        let (mut w, a, _, me) = setup();
        w.set_position(me, Vector3::new(5.0, 1.0, 0.0));
        let c = PositionConstraint {
            locked: true,
            sources: vec![ConstraintSource::new(a, 1.0)],
            ..Default::default()
        };
        w.add_component(me, c).unwrap();
        assert!(activate(&mut w, me, ConstraintKind::Position));
        evaluate_constraints(&mut w);
        assert!(near(w.position(me), Vector3::new(5.0, 1.0, 0.0)));
        w.set_position(a, Vector3::new(3.0, 0.0, 0.0));
        evaluate_constraints(&mut w);
        assert!(near(w.position(me), Vector3::new(6.0, 1.0, 0.0)));
        assert!(zero(&mut w, me, ConstraintKind::Position));
        evaluate_constraints(&mut w);
        assert!(near(w.position(me), Vector3::new(3.0, 0.0, 0.0)));
    }

    #[test]
    fn unlocked_move_updates_offset() {
        let (mut w, a, _, me) = setup();
        let c = PositionConstraint {
            constraint_active: true,
            sources: vec![ConstraintSource::new(a, 1.0)],
            ..Default::default()
        };
        let cid = w.add_component(me, c).unwrap();
        evaluate_constraints(&mut w);
        assert!(near(w.position(me), Vector3::new(2.0, 0.0, 0.0)));
        w.set_position(me, Vector3::new(2.0, 3.0, 0.0)); // dragged by hand
        evaluate_constraints(&mut w);
        let c = w.component::<PositionConstraint>(me, cid).unwrap();
        assert!(near(c.translation_offset, Vector3::new(0.0, 3.0, 0.0)));
        w.set_position(a, Vector3::ZERO);
        evaluate_constraints(&mut w);
        assert!(near(w.position(me), Vector3::new(0.0, 3.0, 0.0)));
    }

    #[test]
    fn rotation_and_scale_follow_sources() {
        let (mut w, a, _, me) = setup();
        w.set_rotation(a, Quaternion::euler(0.0, 90.0, 0.0));
        w.transform_mut(a).unwrap().local_scale = Vector3::splat(3.0);
        let src = vec![ConstraintSource::new(a, 1.0)];
        w.add_component(
            me,
            RotationConstraint {
                constraint_active: true,
                locked: true,
                sources: src.clone(),
                ..Default::default()
            },
        );
        w.add_component(
            me,
            ScaleConstraint {
                constraint_active: true,
                locked: true,
                sources: src,
                ..Default::default()
            },
        );
        evaluate_constraints(&mut w);
        assert!(near(w.forward(me), Vector3::X));
        assert!(near(w.lossy_scale(me), Vector3::splat(3.0)));
    }

    #[test]
    fn parent_constraint_acts_like_a_child() {
        let (mut w, a, _, me) = setup();
        w.set_position(me, Vector3::new(2.0, 0.0, 1.0)); // 1 m in front of A
        w.add_component(
            me,
            ParentConstraint {
                locked: true,
                sources: vec![ConstraintSource::new(a, 1.0)],
                ..Default::default()
            },
        );
        activate(&mut w, me, ConstraintKind::Parent);
        w.set_rotation(a, Quaternion::euler(0.0, 90.0, 0.0));
        evaluate_constraints(&mut w);
        // A turned to face +X, so "in front" is now +X of A.
        assert!(near(w.position(me), Vector3::new(3.0, 0.0, 0.0)));
        assert!(near(w.forward(me), Vector3::X));
    }

    #[test]
    fn aim_and_look_at_point_at_target() {
        let (mut w, a, b, me) = setup();
        w.add_component(
            me,
            AimConstraint {
                constraint_active: true,
                locked: true,
                aim_vector: Vector3::X,
                sources: vec![ConstraintSource::new(b, 1.0)],
                ..Default::default()
            },
        );
        evaluate_constraints(&mut w);
        assert!(near(w.right(me), Vector3::Y));

        let other = w.create("Other");
        w.add_component(
            other,
            LookAtConstraint {
                constraint_active: true,
                locked: true,
                sources: vec![ConstraintSource::new(a, 1.0)],
                ..Default::default()
            },
        );
        evaluate_constraints(&mut w);
        assert!(near(w.forward(other), Vector3::X));
        assert!(near(w.up(other), Vector3::Y));
    }

    #[test]
    fn constraints_round_trip_through_scene_files() {
        let (mut w, a, _, me) = setup();
        w.add_component(
            me,
            ParentConstraint {
                constraint_active: true,
                sources: vec![ConstraintSource::new(a, 0.5)],
                source_offsets: vec![ParentConstraintOffset {
                    translation: Vector3::new(0.0, 1.0, 0.0),
                    rotation: Vector3::ZERO,
                }],
                ..Default::default()
            },
        );
        let roots: Vec<GameObjectId> = [a, me].into();
        let text = w.serialize(&roots, "t").to_yaml().unwrap();
        let back = crate::SceneFile::from_yaml(&text).unwrap();
        let mut w2 = World::new();
        crate::register_builtin(&mut w2.registry);
        let (_, warnings) = w2.spawn(&back, None);
        assert!(warnings.is_empty(), "{warnings:?}");
        let me2 = w2.find("Me").unwrap();
        let c = w2.get_component::<ParentConstraint>(me2).unwrap();
        assert_eq!(c.sources.len(), 1);
        assert_eq!(c.sources[0].weight, 0.5);
        assert_eq!(c.sources[0].source_transform, w2.find("A"));
        assert_eq!(c.source_offsets[0].translation, Vector3::new(0.0, 1.0, 0.0));
    }
}
