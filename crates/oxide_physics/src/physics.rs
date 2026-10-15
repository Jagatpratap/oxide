//! The `Physics` resource (Unity's `Physics` class): the Rapier world plus scene queries.
//! Reference: <https://docs.unity3d.com/ScriptReference/Physics.html>

use oxide_math::{Quaternion, Vector3};
use oxide_scene::{GameObjectId, World};
use rapier3d::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Unity `RaycastHit`.
#[derive(Clone, Copy, Debug)]
pub struct RaycastHit {
    pub point: Vector3,
    pub normal: Vector3,
    pub distance: f32,
    /// Object carrying the collider that was hit.
    pub game_object: GameObjectId,
    /// Object carrying the Rigidbody the collider belongs to (if any).
    pub rigidbody: Option<GameObjectId>,
}

/// Unity's default raycast mask: everything except "Ignore Raycast" (layer 2).
pub const DEFAULT_RAYCAST_LAYERS: u32 = !(1 << 2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BodyKind {
    Dynamic,
    Kinematic,
    Static,
    Character,
}

pub(crate) struct BodyEntry {
    pub handle: RigidBodyHandle,
    pub kind: BodyKind,
    /// Pose last written to (or read from) the Transform, to detect script teleports.
    pub synced: (Vector3, Quaternion),
    pub prev: (Vector3, Quaternion),
    pub curr: (Vector3, Quaternion),
    pub interpolate: bool,
    pub mass: f32,
    pub com: Vector3,
}

pub(crate) struct ColliderEntry {
    pub handle: ColliderHandle,
    pub body: GameObjectId,
    pub signature: u64,
}

#[derive(Default)]
pub(crate) struct EventSink {
    pub events: Mutex<Vec<CollisionEvent>>,
}

impl EventHandler for EventSink {
    fn handle_collision_event(
        &self,
        _: &RigidBodySet,
        _: &ColliderSet,
        event: CollisionEvent,
        _: Option<&ContactPair>,
    ) {
        if let Ok(mut e) = self.events.lock() {
            e.push(event);
        }
    }
    fn handle_contact_force_event(
        &self,
        _: Real,
        _: &RigidBodySet,
        _: &ColliderSet,
        _: &ContactPair,
        _: Real,
    ) {
    }
    fn handle_soft_body_tear_event(&self, _: &SoftBodySet, _: &SoftBodyTearEvent) {}
}

pub struct Physics {
    pub world: PhysicsWorld,
    pub(crate) bodies: HashMap<GameObjectId, BodyEntry>,
    pub(crate) colliders: HashMap<oxide_scene::ComponentId, ColliderEntry>,
    /// Collider → (collider object, rigidbody object).
    pub(crate) owners: HashMap<ColliderHandle, (GameObjectId, Option<GameObjectId>)>,
    pub(crate) vehicles: HashMap<GameObjectId, crate::vehicle::Vehicle>,
    pub(crate) joints: HashMap<oxide_scene::ComponentId, (ImpulseJointHandle, u64)>,
    pub(crate) touching: HashSet<(ColliderHandle, ColliderHandle)>,
    pub(crate) sink: EventSink,
    /// Unity Physics settings: which layers collide with which (bit j of row i).
    pub layer_collision_matrix: [u32; 32],
    /// Unity `Physics.queriesHitTriggers`.
    pub queries_hit_triggers: bool,
    /// Draw collider outlines every frame (green = static, blue = dynamic, yellow = trigger).
    pub debug_draw: bool,
}

impl Default for Physics {
    fn default() -> Self {
        let mut world = PhysicsWorld::new();
        world.gravity = Vector::new(0.0, -9.81, 0.0);
        Self {
            world,
            bodies: HashMap::new(),
            colliders: HashMap::new(),
            owners: HashMap::new(),
            vehicles: HashMap::new(),
            joints: HashMap::new(),
            touching: HashSet::new(),
            sink: EventSink::default(),
            layer_collision_matrix: [u32::MAX; 32],
            queries_hit_triggers: true,
            debug_draw: false,
        }
    }
}

pub(crate) fn pose(p: Vector3, r: Quaternion) -> Pose {
    Pose::from_parts(p, r)
}

impl Physics {
    /// Unity `Physics.gravity`.
    pub fn gravity(&self) -> Vector3 {
        self.world.gravity
    }

    pub fn set_gravity(&mut self, g: Vector3) {
        self.world.gravity = g;
    }

    /// Unity `Physics.IgnoreLayerCollision`.
    pub fn ignore_layer_collision(&mut self, a: u8, b: u8, ignore: bool) {
        let (a, b) = (a as usize & 31, b as usize & 31);
        if ignore {
            self.layer_collision_matrix[a] &= !(1 << b);
            self.layer_collision_matrix[b] &= !(1 << a);
        } else {
            self.layer_collision_matrix[a] |= 1 << b;
            self.layer_collision_matrix[b] |= 1 << a;
        }
    }

    /// Rigid-body object a collider handle belongs to.
    pub fn owner(&self, h: ColliderHandle) -> Option<(GameObjectId, Option<GameObjectId>)> {
        self.owners.get(&h).copied()
    }

    fn layer_of(&self, h: ColliderHandle) -> u32 {
        self.world
            .colliders
            .get(h)
            .map_or(0, |c| (c.user_data >> 64) as u32 & 31)
    }

    fn query<'a, F: Fn(ColliderHandle, &Collider) -> bool>(
        &'a self,
        mask: u32,
        exclude: Option<GameObjectId>,
        triggers: bool,
        pred: &'a F,
    ) -> QueryPipeline<'a> {
        let mut f = QueryFilter::default().predicate(pred);
        if !triggers {
            f = f.exclude_sensors();
        }
        if let Some(e) = exclude
            && let Some(b) = self.bodies.get(&e)
        {
            f = f.exclude_rigid_body(b.handle);
        }
        let _ = mask;
        self.world.query_pipeline_with_filter(f)
    }

    fn hit(
        &self,
        h: ColliderHandle,
        origin: Vector3,
        dir: Vector3,
        toi: f32,
        normal: Vector3,
    ) -> Option<RaycastHit> {
        let (obj, body) = self.owner(h)?;
        Some(RaycastHit {
            point: origin + dir * toi,
            normal,
            distance: toi,
            game_object: obj,
            rigidbody: body,
        })
    }

    /// Unity `Physics.Raycast`: closest hit along `direction` within `max_distance`.
    pub fn raycast(
        &self,
        origin: Vector3,
        direction: Vector3,
        max_distance: f32,
        layer_mask: u32,
    ) -> Option<RaycastHit> {
        self.raycast_excluding(origin, direction, max_distance, layer_mask, None)
    }

    /// Raycast ignoring the colliders of one rigidbody/object (e.g. the shooter).
    pub fn raycast_excluding(
        &self,
        origin: Vector3,
        direction: Vector3,
        max_distance: f32,
        layer_mask: u32,
        exclude: Option<GameObjectId>,
    ) -> Option<RaycastHit> {
        let dir = direction.normalize_or_zero();
        if dir == Vector3::ZERO {
            return None;
        }
        let pred = |h: ColliderHandle, _: &Collider| {
            layer_mask & (1 << self.layer_of(h)) != 0
                && exclude.is_none_or(|e| self.owner(h).is_none_or(|(o, b)| o != e && b != Some(e)))
        };
        let q = self.query(layer_mask, exclude, self.queries_hit_triggers, &pred);
        let ray = Ray::new(origin, dir);
        let (h, hit) = q.cast_ray_and_get_normal(&ray, max_distance, true)?;
        self.hit(h, origin, dir, hit.time_of_impact, hit.normal)
    }

    /// Unity `Physics.RaycastAll` (sorted by distance).
    pub fn raycast_all(
        &self,
        origin: Vector3,
        direction: Vector3,
        max_distance: f32,
        layer_mask: u32,
    ) -> Vec<RaycastHit> {
        let dir = direction.normalize_or_zero();
        let pred = |h: ColliderHandle, _: &Collider| layer_mask & (1 << self.layer_of(h)) != 0;
        let q = self.query(layer_mask, None, self.queries_hit_triggers, &pred);
        let ray = Ray::new(origin, dir);
        let mut out: Vec<RaycastHit> = q
            .intersect_ray(ray, max_distance, true)
            .filter_map(|(h, _, hit)| self.hit(h, origin, dir, hit.time_of_impact, hit.normal))
            .collect();
        out.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        out
    }

    /// Unity `Physics.SphereCast`: sweeps a sphere and returns the first hit.
    pub fn sphere_cast(
        &self,
        origin: Vector3,
        radius: f32,
        direction: Vector3,
        max_distance: f32,
        layer_mask: u32,
    ) -> Option<RaycastHit> {
        let dir = direction.normalize_or_zero();
        let pred = |h: ColliderHandle, _: &Collider| layer_mask & (1 << self.layer_of(h)) != 0;
        let q = self.query(layer_mask, None, self.queries_hit_triggers, &pred);
        let shape = Ball::new(radius);
        let opts = rapier3d::parry::query::ShapeCastOptions {
            max_time_of_impact: max_distance,
            stop_at_penetration: true,
            ..Default::default()
        };
        let (h, hit) = q.cast_shape(&pose(origin, Quaternion::IDENTITY), dir, &shape, opts)?;
        let (obj, body) = self.owner(h)?;
        let toi = hit.time_of_impact;
        let center = origin + dir * toi;
        // Closest point on the hit collider to the swept sphere's center.
        let col = self.world.colliders.get(h)?;
        let proj = col.shape().project_point(col.position(), center, true);
        let normal = (center - proj.point).normalize_or(-dir);
        Some(RaycastHit {
            point: proj.point,
            normal,
            distance: toi,
            game_object: obj,
            rigidbody: body,
        })
    }

    /// Unity `Physics.OverlapSphere`: objects whose colliders touch the sphere.
    pub fn overlap_sphere(
        &self,
        center: Vector3,
        radius: f32,
        layer_mask: u32,
    ) -> Vec<GameObjectId> {
        let pred = |h: ColliderHandle, _: &Collider| layer_mask & (1 << self.layer_of(h)) != 0;
        let q = self.query(layer_mask, None, self.queries_hit_triggers, &pred);
        let shape = Ball::new(radius);
        let mut out: Vec<GameObjectId> = q
            .intersect_shape(pose(center, Quaternion::IDENTITY), &shape)
            .filter_map(|(h, _)| self.owner(h).map(|o| o.0))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Unity `Physics.OverlapBox`.
    pub fn overlap_box(
        &self,
        center: Vector3,
        half_extents: Vector3,
        rotation: Quaternion,
        layer_mask: u32,
    ) -> Vec<GameObjectId> {
        let pred = |h: ColliderHandle, _: &Collider| layer_mask & (1 << self.layer_of(h)) != 0;
        let q = self.query(layer_mask, None, self.queries_hit_triggers, &pred);
        let shape = Cuboid::new(half_extents);
        let mut out: Vec<GameObjectId> = q
            .intersect_shape(pose(center, rotation), &shape)
            .filter_map(|(h, _)| self.owner(h).map(|o| o.0))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Unity `Physics.CheckSphere`.
    pub fn check_sphere(&self, center: Vector3, radius: f32, layer_mask: u32) -> bool {
        !self.overlap_sphere(center, radius, layer_mask).is_empty()
    }

    /// Number of simulated bodies and colliders.
    pub fn counts(&self) -> (usize, usize) {
        (self.world.bodies.len(), self.world.colliders.len())
    }
}

/// Unity-style physics calls from scripts: `ctx.physics().raycast(...)`.
pub trait PhysicsExt {
    fn physics(&self) -> &Physics;
    fn physics_mut(&mut self) -> &mut Physics;
}

impl PhysicsExt for World {
    fn physics(&self) -> &Physics {
        self.resource::<Physics>()
    }
    fn physics_mut(&mut self) -> &mut Physics {
        self.resource_mut::<Physics>()
    }
}

impl PhysicsExt for oxide_scene::Ctx<'_> {
    fn physics(&self) -> &Physics {
        self.world.resource::<Physics>()
    }
    fn physics_mut(&mut self) -> &mut Physics {
        self.world.resource_mut::<Physics>()
    }
}
