//! Physics components with Unity's names and fields.
//! References: <https://docs.unity3d.com/Manual/class-Rigidbody.html>,
//! <https://docs.unity3d.com/Manual/CollidersOverview.html>,
//! <https://docs.unity3d.com/Manual/class-CharacterController.html>,
//! <https://docs.unity3d.com/Manual/class-WheelCollider.html>

use oxide_assets::{Asset, Handle, Mesh};
use oxide_core::Reflect;
use oxide_math::{Quaternion, Vector3};
use oxide_scene::{Component, GameObjectId};

/// Unity `ForceMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum ForceMode {
    /// Continuous force, uses mass (N).
    #[default]
    Force,
    /// Continuous acceleration, ignores mass.
    Acceleration,
    /// Instant impulse, uses mass (N·s).
    Impulse,
    /// Instant velocity change, ignores mass.
    VelocityChange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum RigidbodyInterpolation {
    None,
    /// Smooths rendering between physics steps (recommended for player vehicles/characters).
    #[default]
    Interpolate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum CollisionDetectionMode {
    #[default]
    Discrete,
    /// Continuous collision detection for fast objects (bullets, cars).
    Continuous,
}

#[derive(Clone, Debug)]
#[doc(hidden)]
pub enum RbCommand {
    Force(Vector3, ForceMode),
    ForceAt(Vector3, Vector3, ForceMode),
    Torque(Vector3, ForceMode),
    SetVelocity(Vector3),
    SetAngularVelocity(Vector3),
    MovePosition(Vector3),
    MoveRotation(Quaternion),
    Sleep,
    WakeUp,
}

/// Unity `Rigidbody`.
#[derive(Clone, Debug, Reflect)]
pub struct Rigidbody {
    pub mass: f32,
    /// Linear damping (Unity "Linear Damping"/drag).
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub use_gravity: bool,
    pub is_kinematic: bool,
    pub interpolation: RigidbodyInterpolation,
    pub collision_detection: CollisionDetectionMode,
    /// Local center of mass offset (zero = computed from colliders).
    pub center_of_mass: Vector3,
    pub freeze_position_x: bool,
    pub freeze_position_y: bool,
    pub freeze_position_z: bool,
    pub freeze_rotation_x: bool,
    pub freeze_rotation_y: bool,
    pub freeze_rotation_z: bool,
    #[reflect(skip)]
    #[doc(hidden)]
    pub velocity: Vector3,
    #[reflect(skip)]
    #[doc(hidden)]
    pub angular_velocity: Vector3,
    #[reflect(skip)]
    #[doc(hidden)]
    pub sleeping: bool,
    #[reflect(skip)]
    #[doc(hidden)]
    pub commands: Vec<RbCommand>,
}

impl Default for Rigidbody {
    fn default() -> Self {
        Self {
            mass: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.05,
            use_gravity: true,
            is_kinematic: false,
            interpolation: RigidbodyInterpolation::None,
            collision_detection: CollisionDetectionMode::Discrete,
            center_of_mass: Vector3::ZERO,
            freeze_position_x: false,
            freeze_position_y: false,
            freeze_position_z: false,
            freeze_rotation_x: false,
            freeze_rotation_y: false,
            freeze_rotation_z: false,
            velocity: Vector3::ZERO,
            angular_velocity: Vector3::ZERO,
            sleeping: false,
            commands: Vec::new(),
        }
    }
}

impl Rigidbody {
    pub fn with_mass(mass: f32) -> Self {
        Self {
            mass,
            ..Default::default()
        }
    }
    /// World-space velocity (m/s) as of the last physics step (Unity `linearVelocity`).
    pub fn velocity(&self) -> Vector3 {
        self.velocity
    }
    pub fn set_velocity(&mut self, v: Vector3) {
        self.velocity = v;
        self.commands.push(RbCommand::SetVelocity(v));
    }
    /// World-space angular velocity in radians/s.
    pub fn angular_velocity(&self) -> Vector3 {
        self.angular_velocity
    }
    pub fn set_angular_velocity(&mut self, v: Vector3) {
        self.angular_velocity = v;
        self.commands.push(RbCommand::SetAngularVelocity(v));
    }
    /// Unity `AddForce(force, mode)` (world space).
    pub fn add_force(&mut self, force: Vector3, mode: ForceMode) {
        self.commands.push(RbCommand::Force(force, mode));
    }
    pub fn add_force_at_position(&mut self, force: Vector3, position: Vector3, mode: ForceMode) {
        self.commands
            .push(RbCommand::ForceAt(force, position, mode));
    }
    /// Unity `AddTorque` (world space).
    pub fn add_torque(&mut self, torque: Vector3, mode: ForceMode) {
        self.commands.push(RbCommand::Torque(torque, mode));
    }
    /// Unity `MovePosition` (kinematic bodies interpolate to it during the next step).
    pub fn move_position(&mut self, p: Vector3) {
        self.commands.push(RbCommand::MovePosition(p));
    }
    pub fn move_rotation(&mut self, q: Quaternion) {
        self.commands.push(RbCommand::MoveRotation(q));
    }
    pub fn sleep(&mut self) {
        self.commands.push(RbCommand::Sleep);
    }
    pub fn wake_up(&mut self) {
        self.commands.push(RbCommand::WakeUp);
    }
    pub fn is_sleeping(&self) -> bool {
        self.sleeping
    }
    /// Speed in m/s.
    pub fn speed(&self) -> f32 {
        self.velocity.length()
    }
}

impl Component for Rigidbody {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// How two touching colliders' values combine (Unity `PhysicsMaterialCombine`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum PhysicsMaterialCombine {
    #[default]
    Average,
    Minimum,
    Multiply,
    Maximum,
}

/// Unity's Physics Material asset (`.physicMaterial`): surface friction and bounce shared by
/// colliders (ice, rubber, tarmac). Rapier has one friction coefficient, so
/// `dynamic_friction` is used while sliding and the larger of the two while at rest is
/// approximated by `static_friction` when it is higher.
/// Reference: <https://docs.unity3d.com/Manual/class-PhysicsMaterial.html>
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct PhysicsMaterial {
    #[reflect(range(0, 1))]
    pub dynamic_friction: f32,
    #[reflect(range(0, 1))]
    pub static_friction: f32,
    #[reflect(range(0, 1))]
    pub bounciness: f32,
    pub friction_combine: PhysicsMaterialCombine,
    pub bounce_combine: PhysicsMaterialCombine,
}

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            dynamic_friction: 0.6,
            static_friction: 0.6,
            bounciness: 0.0,
            friction_combine: PhysicsMaterialCombine::Average,
            bounce_combine: PhysicsMaterialCombine::Average,
        }
    }
}

impl Asset for PhysicsMaterial {
    const TYPE_NAME: &'static str = "PhysicsMaterial";
}

impl PhysicsMaterial {
    /// The friction coefficient given to Rapier.
    pub fn friction(&self) -> f32 {
        self.dynamic_friction.max(self.static_friction * 0.9)
    }
}

/// Shared collider settings (Unity Collider + PhysicMaterial fields).
macro_rules! collider_common {
    () => {
        pub fn is_trigger(&self) -> bool {
            self.is_trigger
        }
    };
}

/// Unity `BoxCollider`.
#[derive(Clone, Debug, Reflect)]
pub struct BoxCollider {
    pub is_trigger: bool,
    /// Shared Physics Material (Unity `sharedMaterial`); overrides friction and bounciness.
    pub material: Option<Handle<PhysicsMaterial>>,
    pub center: Vector3,
    pub size: Vector3,
    #[reflect(range(0, 1))]
    pub friction: f32,
    #[reflect(range(0, 1))]
    pub bounciness: f32,
}

impl Default for BoxCollider {
    fn default() -> Self {
        Self {
            is_trigger: false,
            material: None,
            center: Vector3::ZERO,
            size: Vector3::ONE,
            friction: 0.6,
            bounciness: 0.0,
        }
    }
}

impl BoxCollider {
    collider_common!();
    pub fn new(size: Vector3) -> Self {
        Self {
            size,
            ..Default::default()
        }
    }
}

impl Component for BoxCollider {}

/// Unity `SphereCollider`.
#[derive(Clone, Debug, Reflect)]
pub struct SphereCollider {
    pub is_trigger: bool,
    /// Shared Physics Material (Unity `sharedMaterial`); overrides friction and bounciness.
    pub material: Option<Handle<PhysicsMaterial>>,
    pub center: Vector3,
    pub radius: f32,
    #[reflect(range(0, 1))]
    pub friction: f32,
    #[reflect(range(0, 1))]
    pub bounciness: f32,
}

impl Default for SphereCollider {
    fn default() -> Self {
        Self {
            is_trigger: false,
            material: None,
            center: Vector3::ZERO,
            radius: 0.5,
            friction: 0.6,
            bounciness: 0.0,
        }
    }
}

impl SphereCollider {
    collider_common!();
}

impl Component for SphereCollider {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum CapsuleDirection {
    XAxis,
    #[default]
    YAxis,
    ZAxis,
}

/// Unity `CapsuleCollider`.
#[derive(Clone, Debug, Reflect)]
pub struct CapsuleCollider {
    pub is_trigger: bool,
    /// Shared Physics Material (Unity `sharedMaterial`); overrides friction and bounciness.
    pub material: Option<Handle<PhysicsMaterial>>,
    pub center: Vector3,
    pub radius: f32,
    /// Total height including the caps.
    pub height: f32,
    pub direction: CapsuleDirection,
    #[reflect(range(0, 1))]
    pub friction: f32,
    #[reflect(range(0, 1))]
    pub bounciness: f32,
}

impl Default for CapsuleCollider {
    fn default() -> Self {
        Self {
            is_trigger: false,
            material: None,
            center: Vector3::ZERO,
            radius: 0.5,
            height: 2.0,
            direction: CapsuleDirection::YAxis,
            friction: 0.6,
            bounciness: 0.0,
        }
    }
}

impl CapsuleCollider {
    collider_common!();
}

impl Component for CapsuleCollider {}

/// Unity `MeshCollider`. Uses the MeshFilter's mesh when `mesh` is empty. Non-convex mesh
/// colliders only work on static objects or kinematic bodies, like in Unity.
#[derive(Clone, Debug, Reflect)]
pub struct MeshCollider {
    pub is_trigger: bool,
    /// Shared Physics Material (Unity `sharedMaterial`); overrides friction and bounciness.
    pub material: Option<Handle<PhysicsMaterial>>,
    pub mesh: Option<Handle<Mesh>>,
    pub convex: bool,
    #[reflect(range(0, 1))]
    pub friction: f32,
    #[reflect(range(0, 1))]
    pub bounciness: f32,
}

impl Default for MeshCollider {
    fn default() -> Self {
        Self {
            is_trigger: false,
            material: None,
            mesh: None,
            convex: false,
            friction: 0.6,
            bounciness: 0.0,
        }
    }
}

impl MeshCollider {
    collider_common!();
}

impl Component for MeshCollider {}

/// Height grid collider (used by Terrain). `heights` is row-major `resolution × resolution`,
/// rows along +Z, normalized 0..1 and scaled by `size.y`; the grid spans `size.x × size.z`
/// starting at the object's origin.
#[derive(Clone, Debug, Reflect)]
pub struct TerrainCollider {
    pub size: Vector3,
    pub resolution: u32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub heights: std::sync::Arc<Vec<f32>>,
    /// One byte per cell ((resolution - 1)², 0 = hole; empty = no holes).
    #[reflect(skip)]
    #[doc(hidden)]
    pub holes: std::sync::Arc<Vec<u8>>,
    /// Bumped when `heights` changes so the physics shape is rebuilt.
    #[reflect(skip)]
    #[doc(hidden)]
    pub version: u64,
    #[reflect(range(0, 1))]
    pub friction: f32,
    /// Shared Physics Material; overrides friction.
    pub material: Option<Handle<PhysicsMaterial>>,
}

impl Default for TerrainCollider {
    fn default() -> Self {
        Self {
            size: Vector3::new(100.0, 30.0, 100.0),
            resolution: 2,
            heights: std::sync::Arc::new(vec![0.0; 4]),
            holes: std::sync::Arc::new(Vec::new()),
            version: 0,
            material: None,
            friction: 0.8,
        }
    }
}

impl Component for TerrainCollider {}

/// Unity `CollisionFlags` returned by `CharacterController.Move`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollisionFlags {
    pub below: bool,
    pub sides: bool,
    pub above: bool,
}

/// Unity `CharacterController`: a capsule moved with `move_character` that slides along
/// walls, climbs steps and slopes, and snaps to the ground.
#[derive(Clone, Debug, Reflect)]
pub struct CharacterController {
    #[reflect(range(0, 90))]
    pub slope_limit: f32,
    pub step_offset: f32,
    pub skin_width: f32,
    pub center: Vector3,
    pub radius: f32,
    pub height: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub is_grounded: bool,
    #[reflect(skip)]
    #[doc(hidden)]
    pub velocity: Vector3,
    #[reflect(skip)]
    #[doc(hidden)]
    pub collision_flags: CollisionFlags,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            slope_limit: 45.0,
            step_offset: 0.3,
            skin_width: 0.02,
            center: Vector3::new(0.0, 1.0, 0.0),
            radius: 0.5,
            height: 2.0,
            is_grounded: false,
            velocity: Vector3::ZERO,
            collision_flags: CollisionFlags::default(),
        }
    }
}

impl Component for CharacterController {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity's `WheelFrictionCurve`: tyre force (as a multiple of the load on the tyre) by
/// slip. It rises from zero to an extremum, where the tyre grips hardest, then falls to an
/// asymptote while sliding. Reference:
/// <https://docs.unity3d.com/ScriptReference/WheelFrictionCurve.html>
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct WheelFrictionCurve {
    pub extremum_slip: f32,
    pub extremum_value: f32,
    pub asymptote_slip: f32,
    pub asymptote_value: f32,
    /// Multiplies the whole curve (Unity changes this per surface: ice, grass, ...).
    pub stiffness: f32,
}

impl Default for WheelFrictionCurve {
    fn default() -> Self {
        Self::FORWARD
    }
}

impl WheelFrictionCurve {
    /// Default forward friction. Slip here is the true slip ratio
    /// `(wheel speed − road speed) / road speed`, so the peak sits where road tyres peak
    /// (about 12%) rather than at Unity's 0.4, which is in PhysX's own slip units.
    pub const FORWARD: Self = Self {
        extremum_slip: 0.12,
        extremum_value: 1.0,
        asymptote_slip: 0.5,
        asymptote_value: 0.6,
        stiffness: 1.0,
    };
    /// Default sideways friction; slip is the slip angle in radians (peak near 8.6°).
    pub const SIDEWAYS: Self = Self {
        extremum_slip: 0.15,
        extremum_value: 1.0,
        asymptote_slip: 0.45,
        asymptote_value: 0.75,
        stiffness: 1.0,
    };

    /// Force factor at `slip` (sign ignored), stiffness included: a two-piece spline that
    /// is flat at the extremum and the asymptote, as Unity documents.
    pub fn evaluate(&self, slip: f32) -> f32 {
        let s = slip.abs();
        let es = self.extremum_slip.max(1e-4);
        let as_ = self.asymptote_slip.max(es + 1e-4);
        let v = if s <= es {
            // Starts at slope 2·value/slip (the linear "cornering stiffness"), flat at the peak.
            let x = s / es;
            self.extremum_value * (2.0 * x - x * x)
        } else if s < as_ {
            let x = (s - es) / (as_ - es);
            let smooth = x * x * (3.0 - 2.0 * x);
            self.extremum_value + (self.asymptote_value - self.extremum_value) * smooth
        } else {
            self.asymptote_value
        };
        v * self.stiffness
    }

    /// The curve as an [`oxide_core::AnimationCurve`] up to twice the asymptote slip (for
    /// drawing it).
    pub fn to_animation_curve(&self) -> oxide_core::AnimationCurve {
        let end = self.asymptote_slip.max(self.extremum_slip) * 2.0;
        oxide_core::AnimationCurve::new((0..=32).map(|i| {
            let s = end * i as f32 / 32.0;
            let mut k = oxide_core::Keyframe::new(s, self.evaluate(s));
            k.in_mode = oxide_core::TangentMode::Linear;
            k.out_mode = oxide_core::TangentMode::Linear;
            k
        }))
    }

    /// Largest force factor anywhere on the curve.
    pub fn peak(&self) -> f32 {
        self.extremum_value.max(self.asymptote_value) * self.stiffness
    }
}

/// Unity `JointSpring` used by the wheel suspension.
#[derive(Clone, Debug, Reflect)]
pub struct WheelCollider {
    /// Wheel mass (affects rolling inertia in Unity; informational here).
    pub mass: f32,
    pub radius: f32,
    /// Max suspension extension in meters.
    pub suspension_distance: f32,
    pub center: Vector3,
    /// Spring stiffness (N/m per kg of sprung mass, Rapier scale).
    pub suspension_spring: f32,
    pub suspension_damper: f32,
    /// Tyre grip along the rolling direction, by slip ratio (Unity `forwardFriction`).
    pub forward_friction: WheelFrictionCurve,
    /// Tyre grip across the wheel, by slip angle in radians (Unity `sidewaysFriction`).
    pub sideways_friction: WheelFrictionCurve,
    /// How quickly a free-spinning wheel slows down (Unity `wheelDampingRate`, N·m·s).
    pub wheel_damping_rate: f32,
    /// Height above the contact point where tyre forces push the body (Unity
    /// `forceAppPointDistance`); at the centre of mass height the tyres can't roll the car.
    pub force_app_point_distance: f32,
    /// Driving torque in N·m (set every frame by the car script).
    pub motor_torque: f32,
    pub brake_torque: f32,
    /// Steering angle in degrees (positive = right).
    pub steer_angle: f32,
    /// Optional wheel mesh object posed automatically every frame (spin, steer, suspension).
    pub wheel_model: Option<GameObjectId>,
    #[reflect(skip)]
    #[doc(hidden)]
    pub state: WheelState,
}

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct WheelState {
    pub grounded: bool,
    pub rpm: f32,
    pub position: Vector3,
    pub rotation: Quaternion,
    pub contact_point: Vector3,
    pub contact_normal: Vector3,
    pub ground: Option<GameObjectId>,
    pub forward_slip: f32,
    pub sideways_slip: f32,
    /// Tyre load (N) from the suspension.
    pub load: f32,
    /// Wheel angular speed (rad/s, positive rolling forwards).
    pub angular_velocity: f32,
    pub suspension_length: f32,
    /// Wheel pose relative to the vehicle body.
    pub local_position: Vector3,
    pub local_rotation: Quaternion,
    pub spin: f32,
}

impl Default for WheelCollider {
    fn default() -> Self {
        Self {
            mass: 20.0,
            radius: 0.35,
            suspension_distance: 0.3,
            center: Vector3::ZERO,
            suspension_spring: 35.0,
            suspension_damper: 4.5,
            forward_friction: WheelFrictionCurve::FORWARD,
            sideways_friction: WheelFrictionCurve::SIDEWAYS,
            wheel_damping_rate: 0.25,
            force_app_point_distance: 0.4,
            motor_torque: 0.0,
            brake_torque: 0.0,
            steer_angle: 0.0,
            wheel_model: None,
            state: WheelState {
                rotation: Quaternion::IDENTITY,
                local_rotation: Quaternion::IDENTITY,
                ..Default::default()
            },
        }
    }
}

impl WheelCollider {
    pub fn is_grounded(&self) -> bool {
        self.state.grounded
    }
    pub fn rpm(&self) -> f32 {
        self.state.rpm
    }
    /// Unity `GetWorldPose`: where to draw the wheel mesh (center and spin/steer rotation).
    pub fn world_pose(&self) -> (Vector3, Quaternion) {
        (self.state.position, self.state.rotation)
    }
    /// Unity `GetGroundHit`: (point, normal, object) while grounded.
    pub fn ground_hit(&self) -> Option<(Vector3, Vector3, Option<GameObjectId>)> {
        self.state.grounded.then_some((
            self.state.contact_point,
            self.state.contact_normal,
            self.state.ground,
        ))
    }
    /// Unity `WheelHit.force`: the tyre load in newtons while grounded.
    pub fn load(&self) -> f32 {
        self.state.load
    }
    /// Slip (forward slip ratio, sideways slip angle in radians), for tyre smoke and
    /// skid sounds. Positive forward slip means wheelspin, negative means locking.
    pub fn slip(&self) -> (f32, f32) {
        (self.state.forward_slip, self.state.sideways_slip)
    }
    pub fn suspension_compression(&self) -> f32 {
        1.0 - (self.state.suspension_length / self.suspension_distance.max(1e-4)).clamp(0.0, 1.0)
    }
}

impl Component for WheelCollider {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity `FixedJoint`: welds this body to `connected_body` (or the world).
#[derive(Clone, Debug, Default, Reflect)]
pub struct FixedJoint {
    pub connected_body: Option<GameObjectId>,
    /// Force above which the joint breaks (0 = unbreakable).
    pub break_force: f32,
}

impl Component for FixedJoint {}

/// Unity `HingeJoint`: rotation around `axis` through `anchor` (local), with optional limits
/// and motor.
#[derive(Clone, Debug, Reflect)]
pub struct HingeJoint {
    pub connected_body: Option<GameObjectId>,
    pub anchor: Vector3,
    pub axis: Vector3,
    pub use_limits: bool,
    pub min_angle: f32,
    pub max_angle: f32,
    pub use_motor: bool,
    /// Degrees per second.
    pub motor_target_velocity: f32,
    pub motor_force: f32,
}

impl Default for HingeJoint {
    fn default() -> Self {
        Self {
            connected_body: None,
            anchor: Vector3::ZERO,
            axis: Vector3::X,
            use_limits: false,
            min_angle: -90.0,
            max_angle: 90.0,
            use_motor: false,
            motor_target_velocity: 0.0,
            motor_force: 0.0,
        }
    }
}

impl Component for HingeJoint {}

/// Unity `CharacterJoint`: a ball-and-socket joint with twist and swing limits, made for
/// ragdolls. `axis` is the twist axis and `swing_axis` the first swing axis (both local).
/// Bodies joined by it do not collide with each other (Unity `enableCollision` off).
#[derive(Clone, Debug, Reflect)]
pub struct CharacterJoint {
    pub connected_body: Option<GameObjectId>,
    pub anchor: Vector3,
    pub axis: Vector3,
    pub swing_axis: Vector3,
    /// Degrees.
    pub low_twist_limit: f32,
    pub high_twist_limit: f32,
    /// Swing around `swing_axis`, degrees either way.
    pub swing1_limit: f32,
    /// Swing around the third axis, degrees either way.
    pub swing2_limit: f32,
    pub enable_collision: bool,
}

impl Default for CharacterJoint {
    fn default() -> Self {
        Self {
            connected_body: None,
            anchor: Vector3::ZERO,
            axis: Vector3::X,
            swing_axis: Vector3::Y,
            low_twist_limit: -20.0,
            high_twist_limit: 70.0,
            swing1_limit: 40.0,
            swing2_limit: 40.0,
            enable_collision: false,
        }
    }
}

impl Component for CharacterJoint {}
