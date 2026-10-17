//! Oxide animation (Unity Mecanim): the `Animator` component plays an `AnimatorController`
//! — parameters, layers (Override/Additive, Avatar Masks, weights), states (clips, 1D and
//! 2D blend trees), conditional transitions with crossfades and exit times — with root
//! motion, animation events, humanoid bone mapping and IK. The legacy `Animation`
//! component plays a single clip. Curves bind to child transforms by path, like Unity.
//!
//! Runs after `Update` and before `LateUpdate` (Unity's "internal animation update"), or in
//! the fixed step for `AnimatorUpdateMode::AnimatePhysics`.
//! Reference: <https://docs.unity3d.com/Manual/AnimationOverview.html>

pub mod behaviours;
mod controller;
mod humanoid;
pub mod ik;
pub mod mirror;
pub mod retarget;

pub use behaviours::{StateMachineBehaviour, StateMachineBehaviours};
pub use controller::{ActiveTransition, motion_length, tree_weights};
pub use humanoid::{HumanBodyBones, auto_map, is_human};

use oxide_assets::{
    AnimationClip, AnimatorController, AnimatorLayerBlendingMode, AnimatorStateMachine,
    AssetServer, AvatarMask, BlendTree2DType, Handle, Motion, ParameterType,
};
use oxide_core::Reflect;
use oxide_core::Time;
use oxide_core::reflect::Value;
use oxide_math::{Quaternion, Vector2, Vector3};
use oxide_scene::{Component, GameObjectId, System, World};
use std::collections::HashMap;

/// Unity `AnimatorUpdateMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Reflect)]
pub enum AnimatorUpdateMode {
    #[default]
    Normal,
    /// Animates in the fixed step, in sync with physics.
    AnimatePhysics,
    /// Ignores `Time.timeScale` (pause menus).
    UnscaledTime,
}

/// Unity `AvatarIKGoal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvatarIKGoal {
    LeftFoot,
    RightFoot,
    LeftHand,
    RightHand,
}

/// Unity `AvatarIKHint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvatarIKHint {
    LeftKnee,
    RightKnee,
    LeftElbow,
    RightElbow,
}

/// Unity `AnimatorStateInfo`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimatorStateInfo {
    pub name: String,
    /// `Layer.SubMachine.State` (Unity `fullPathHash` as a string).
    pub full_path: String,
    pub tag: String,
    /// Integer part = completed loops, fraction = progress through the current loop.
    pub normalized_time: f32,
    /// Seconds.
    pub length: f32,
    pub speed: f32,
    pub looping: bool,
}

impl AnimatorStateInfo {
    /// Unity `IsName`: the state's name or its full path (`Base Layer.Locomotion.Run`).
    pub fn is_name(&self, name: &str) -> bool {
        self.name == name || self.full_path == name
    }
    pub fn is_tag(&self, tag: &str) -> bool {
        self.tag == tag
    }
}

#[derive(Clone, Copy, Debug, Default)]
#[doc(hidden)]
pub struct IkGoalState {
    pub position: Vector3,
    pub rotation: Quaternion,
    pub position_weight: f32,
    pub rotation_weight: f32,
}

#[derive(Clone, Copy, Debug, Default)]
#[doc(hidden)]
pub struct LookAtState {
    pub position: Vector3,
    pub weight: f32,
    pub body: f32,
    pub head: f32,
    pub clamp: f32,
}

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct Playing {
    pub state: usize,
    pub time: f32,
    /// `time` before this frame's step (for root motion and events).
    pub prev_time: f32,
}

impl Playing {
    fn start_at(state: usize, time: f32) -> Self {
        Self {
            state,
            time,
            prev_time: time,
        }
    }
}

/// A `Play` / `CrossFade` call waiting for the next update.
#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct PendingPlay {
    pub state: String,
    pub fade: f32,
    /// `fade` in seconds (else a fraction of the current state's length).
    pub fixed: bool,
    /// `None` = the first layer that has the state.
    pub layer: Option<usize>,
    pub normalized_time: Option<f32>,
}

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct LayerRuntime {
    pub current: Option<Playing>,
    /// Crossfade target, elapsed, duration.
    pub next: Option<(Playing, f32, f32)>,
    /// The transition `next` comes from (none for `CrossFade` calls).
    pub transition: Option<ActiveTransition>,
    /// An interrupted transition's destination and its share of the source side.
    pub interrupted: Option<(Playing, f32)>,
    pub info: AnimatorStateInfo,
    pub next_info: Option<AnimatorStateInfo>,
}

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct AnimatorRuntime {
    pub params: HashMap<String, f32>,
    pub layers: Vec<LayerRuntime>,
    pub layer_names: Vec<String>,
    /// The controller's layer weights (until a script sets one).
    pub default_weights: Vec<f32>,
    /// Weights set by scripts (`SetLayerWeight`); others use the controller's default.
    pub layer_weights: HashMap<usize, f32>,
    pub pending: Vec<PendingPlay>,
    pub bindings: HashMap<String, Option<GameObjectId>>,
    pub controller: Option<oxide_assets::AssetId>,
    pub delta_position: Vector3,
    pub delta_rotation: Quaternion,
    pub delta_time: f32,
    pub ik: [IkGoalState; 4],
    pub hints: [(Vector3, f32); 4],
    pub look_at: LookAtState,
    pub human: HashMap<HumanBodyBones, GameObjectId>,
    pub human_mapped: bool,
    /// Left/right bone pairs (built when a state mirrors).
    pub mirror: Option<std::sync::Arc<mirror::MirrorMap>>,
    /// Values animated transforms had before animating (Write Defaults).
    pub defaults: HashMap<GameObjectId, (Vector3, Quaternion, Vector3)>,
    pub prop_defaults: HashMap<(GameObjectId, String, String), f32>,
    /// Humanoid clips made for another rig, retargeted to this one (by clip).
    pub retargeted: HashMap<oxide_assets::AssetId, std::sync::Arc<oxide_assets::AnimationClip>>,
    /// The controller the retargeted clips were made for.
    pub retargeted_for: Option<oxide_assets::AssetId>,
}

/// Unity `Animator`.
#[derive(Clone, Debug, Reflect)]
pub struct Animator {
    pub controller: Option<Handle<AnimatorController>>,
    pub speed: f32,
    /// Moves the GameObject by the animation's root motion (Unity `applyRootMotion`).
    /// A script that handles `OnAnimatorMove` applies it instead (read `delta_position`).
    pub apply_root_motion: bool,
    pub update_mode: AnimatorUpdateMode,
    #[reflect(skip)]
    #[doc(hidden)]
    pub rt: AnimatorRuntime,
}

impl Default for Animator {
    fn default() -> Self {
        Self {
            controller: None,
            speed: 1.0,
            apply_root_motion: false,
            update_mode: AnimatorUpdateMode::Normal,
            rt: AnimatorRuntime::default(),
        }
    }
}

impl Animator {
    pub fn new(controller: Handle<AnimatorController>) -> Self {
        Self {
            controller: Some(controller),
            ..Default::default()
        }
    }

    // ---- parameters ------------------------------------------------------------------

    pub fn set_float(&mut self, name: &str, v: f32) {
        self.rt.params.insert(name.into(), v);
    }
    pub fn get_float(&self, name: &str) -> f32 {
        self.rt.params.get(name).copied().unwrap_or(0.0)
    }
    pub fn set_integer(&mut self, name: &str, v: i32) {
        self.rt.params.insert(name.into(), v as f32);
    }
    pub fn get_integer(&self, name: &str) -> i32 {
        self.get_float(name) as i32
    }
    pub fn set_bool(&mut self, name: &str, v: bool) {
        self.rt.params.insert(name.into(), v as u8 as f32);
    }
    pub fn get_bool(&self, name: &str) -> bool {
        self.get_float(name) != 0.0
    }
    pub fn set_trigger(&mut self, name: &str) {
        self.rt.params.insert(name.into(), 1.0);
    }
    pub fn reset_trigger(&mut self, name: &str) {
        self.rt.params.insert(name.into(), 0.0);
    }

    // ---- states ----------------------------------------------------------------------

    fn queue(&mut self, state: &str, fade: f32, fixed: bool, layer: Option<usize>, t: Option<f32>) {
        self.rt.pending.push(PendingPlay {
            state: state.into(),
            fade: fade.max(0.0),
            fixed,
            layer,
            normalized_time: t,
        });
    }
    /// Unity `Play(stateName)`: jump to a state immediately (in whichever layer has it).
    /// Accepts the state's name or its full path (`Base Layer.Locomotion.Run`).
    pub fn play(&mut self, state: &str) {
        self.queue(state, 0.0, true, None, None);
    }
    /// Unity `Play(stateName, layer)`.
    pub fn play_in_layer(&mut self, state: &str, layer: usize) {
        self.queue(state, 0.0, true, Some(layer), None);
    }
    /// Unity `Play(stateName, layer, normalizedTime)`.
    pub fn play_at(&mut self, state: &str, layer: usize, normalized_time: f32) {
        self.queue(state, 0.0, true, Some(layer), Some(normalized_time));
    }
    /// Unity `CrossFadeInFixedTime(stateName, seconds)`.
    pub fn cross_fade(&mut self, state: &str, duration: f32) {
        self.queue(state, duration, true, None, None);
    }
    /// Unity `CrossFadeInFixedTime(stateName, seconds, layer)`.
    pub fn cross_fade_in_layer(&mut self, state: &str, duration: f32, layer: usize) {
        self.queue(state, duration, true, Some(layer), None);
    }
    /// Unity `CrossFade(stateName, normalizedDuration)`: the duration is a fraction of the
    /// current state's length.
    pub fn cross_fade_normalized(&mut self, state: &str, normalized_duration: f32) {
        self.queue(state, normalized_duration, false, None, None);
    }
    /// Unity `CrossFadeInFixedTime(stateName, seconds, layer, fixedTimeOffset)` with the
    /// offset as normalized time.
    pub fn cross_fade_at(&mut self, state: &str, seconds: f32, layer: usize, normalized_time: f32) {
        self.queue(state, seconds, true, Some(layer), Some(normalized_time));
    }
    /// Unity `GetAnimatorTransitionInfo(layer)`: normalized progress of the current
    /// transition (`None` when not in one).
    pub fn get_animator_transition_info(&self, layer: usize) -> Option<f32> {
        let l = self.rt.layers.get(layer)?;
        let (_, elapsed, dur) = l.next.as_ref()?;
        Some((elapsed / dur.max(1e-4)).clamp(0.0, 1.0))
    }
    /// Unity `parameters`: names and current values.
    pub fn parameters(&self) -> &HashMap<String, f32> {
        &self.rt.params
    }
    /// Base Layer crossfade in progress.
    pub fn is_in_transition(&self) -> bool {
        self.is_in_transition_in_layer(0)
    }
    pub fn is_in_transition_in_layer(&self, layer: usize) -> bool {
        self.rt.layers.get(layer).is_some_and(|l| l.next.is_some())
    }
    /// Unity `GetCurrentAnimatorStateInfo(layer)` (as of the last animation update).
    pub fn get_current_animator_state_info(&self, layer: usize) -> AnimatorStateInfo {
        self.rt
            .layers
            .get(layer)
            .map(|l| l.info.clone())
            .unwrap_or_default()
    }
    /// Unity `GetNextAnimatorStateInfo(layer)`: the crossfade target, if any.
    pub fn get_next_animator_state_info(&self, layer: usize) -> Option<AnimatorStateInfo> {
        self.rt.layers.get(layer).and_then(|l| l.next_info.clone())
    }

    // ---- layers ----------------------------------------------------------------------

    pub fn layer_count(&self) -> usize {
        self.rt.layer_names.len()
    }
    pub fn get_layer_name(&self, layer: usize) -> Option<&str> {
        self.rt.layer_names.get(layer).map(String::as_str)
    }
    pub fn get_layer_index(&self, name: &str) -> Option<usize> {
        self.rt.layer_names.iter().position(|n| n == name)
    }
    /// Unity `SetLayerWeight` (the Base Layer is always fully weighted).
    pub fn set_layer_weight(&mut self, layer: usize, weight: f32) {
        self.rt.layer_weights.insert(layer, weight.clamp(0.0, 1.0));
    }
    pub fn get_layer_weight(&self, layer: usize) -> f32 {
        if layer == 0 {
            return 1.0;
        }
        self.rt
            .layer_weights
            .get(&layer)
            .or_else(|| self.rt.default_weights.get(layer))
            .copied()
            .unwrap_or(1.0)
    }

    // ---- root motion -----------------------------------------------------------------

    /// World-space root motion of the last update (Unity `deltaPosition`).
    pub fn delta_position(&self) -> Vector3 {
        self.rt.delta_position
    }
    /// Root rotation of the last update (Unity `deltaRotation`).
    pub fn delta_rotation(&self) -> Quaternion {
        self.rt.delta_rotation
    }
    /// Unity `velocity`.
    pub fn velocity(&self) -> Vector3 {
        if self.rt.delta_time > 0.0 {
            self.rt.delta_position / self.rt.delta_time
        } else {
            Vector3::ZERO
        }
    }

    // ---- humanoid & IK ---------------------------------------------------------------

    /// Whether the rig maps to a humanoid (Unity `isHuman`).
    pub fn is_human(&self) -> bool {
        is_human(&self.rt.human)
    }
    /// Unity `GetBoneTransform`.
    pub fn get_bone_transform(&self, bone: HumanBodyBones) -> Option<GameObjectId> {
        self.rt.human.get(&bone).copied()
    }
    pub fn set_ik_position(&mut self, goal: AvatarIKGoal, position: Vector3) {
        self.rt.ik[goal as usize].position = position;
    }
    pub fn set_ik_position_weight(&mut self, goal: AvatarIKGoal, weight: f32) {
        self.rt.ik[goal as usize].position_weight = weight.clamp(0.0, 1.0);
    }
    pub fn set_ik_rotation(&mut self, goal: AvatarIKGoal, rotation: Quaternion) {
        self.rt.ik[goal as usize].rotation = rotation;
    }
    pub fn set_ik_rotation_weight(&mut self, goal: AvatarIKGoal, weight: f32) {
        self.rt.ik[goal as usize].rotation_weight = weight.clamp(0.0, 1.0);
    }
    pub fn get_ik_position_weight(&self, goal: AvatarIKGoal) -> f32 {
        self.rt.ik[goal as usize].position_weight
    }
    pub fn set_ik_hint_position(&mut self, hint: AvatarIKHint, position: Vector3) {
        self.rt.hints[hint as usize].0 = position;
    }
    pub fn set_ik_hint_position_weight(&mut self, hint: AvatarIKHint, weight: f32) {
        self.rt.hints[hint as usize].1 = weight.clamp(0.0, 1.0);
    }
    pub fn set_look_at_position(&mut self, position: Vector3) {
        self.rt.look_at.position = position;
    }
    /// Unity `SetLookAtWeight(weight, bodyWeight, headWeight, eyesWeight, clampWeight)`
    /// (eyes are not mapped).
    pub fn set_look_at_weight(&mut self, weight: f32, body: f32, head: f32, clamp: f32) {
        let l = &mut self.rt.look_at;
        (l.weight, l.body, l.head, l.clamp) = (weight, body, head, clamp);
    }
}

impl Component for Animator {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity legacy `Animation`: plays one clip on its hierarchy.
#[derive(Clone, Debug, Reflect)]
pub struct Animation {
    pub clip: Option<Handle<AnimationClip>>,
    pub play_automatically: bool,
    pub looping: bool,
    pub speed: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub time: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub playing: Option<bool>,
    #[reflect(skip)]
    #[doc(hidden)]
    pub bindings: HashMap<String, Option<GameObjectId>>,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            clip: None,
            play_automatically: true,
            looping: true,
            speed: 1.0,
            time: 0.0,
            playing: None,
            bindings: HashMap::new(),
        }
    }
}

impl Animation {
    pub fn play(&mut self) {
        self.time = 0.0;
        self.playing = Some(true);
    }
    pub fn stop(&mut self) {
        self.playing = Some(false);
    }
    pub fn is_playing(&self) -> bool {
        self.playing == Some(true)
    }
}

impl Component for Animation {}

// ---- sampling ----------------------------------------------------------------------------

/// Weighted sums of one transform's sampled channels.
#[derive(Clone, Copy)]
struct Channel {
    pos: Vector3,
    pw: f32,
    rot: Quaternion,
    rw: f32,
    scale: Vector3,
    sw: f32,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            pos: Vector3::ZERO,
            pw: 0.0,
            rot: Quaternion::from_xyzw(0.0, 0.0, 0.0, 0.0),
            rw: 0.0,
            scale: Vector3::ZERO,
            sw: 0.0,
        }
    }
}

/// Local position / rotation / scale; `None` = not animated.
type Trs = (Option<Vector3>, Option<Quaternion>, Option<Vector3>);

fn finalize(pose: HashMap<GameObjectId, Channel>) -> HashMap<GameObjectId, Trs> {
    pose.into_iter()
        .map(|(id, c)| {
            (
                id,
                (
                    (c.pw > 0.0).then(|| c.pos / c.pw),
                    (c.rw > 0.0).then(|| c.rot.normalize()),
                    (c.sw > 0.0).then(|| c.scale / c.sw),
                ),
            )
        })
        .collect()
}

fn bind(
    world: &World,
    root: GameObjectId,
    cache: &mut HashMap<String, Option<GameObjectId>>,
    path: &str,
) -> Option<GameObjectId> {
    if let Some(Some(id)) = cache.get(path)
        && world.is_alive(*id)
    {
        return Some(*id);
    }
    let id = if path.is_empty() {
        Some(root)
    } else {
        world.find_child(root, path).or_else(|| {
            // glTF roots may sit one level down (under the model root).
            path.split_once('/')
                .and_then(|(_, rest)| world.find_child(root, rest))
        })
    };
    cache.insert(path.to_owned(), id);
    id
}

fn wrap(t: f32, len: f32, looping: bool) -> f32 {
    if len <= 0.0 {
        0.0
    } else if looping {
        t.rem_euclid(len)
    } else {
        t.clamp(0.0, len)
    }
}

fn wrap_pi(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (a + PI).rem_euclid(TAU) - PI
}

/// Rotation about +Y (radians) of a quaternion (swing-twist decomposition).
fn yaw_of(q: Quaternion) -> f32 {
    wrap_pi(2.0 * q.y.atan2(q.w))
}

/// Indices of `clip.events` passed going from `prev` to `now` (unwrapped seconds).
fn events_between(clip: &AnimationClip, prev: f32, now: f32, looping: bool) -> Vec<usize> {
    let len = clip.length;
    if clip.events.is_empty() || now <= prev {
        return vec![];
    }
    let fresh = prev <= 0.0;
    let in_range = |e: f32, a: f32, b: f32, incl_a: bool| (e > a || (incl_a && e >= a)) && e <= b;
    let mut out = vec![];
    if !looping || len <= 0.0 {
        let (a, b) = (prev.clamp(0.0, len), now.clamp(0.0, len));
        if a < b || fresh {
            for (i, e) in clip.events.iter().enumerate() {
                if in_range(e.time, a, b, fresh) {
                    out.push(i);
                }
            }
        }
        return out;
    }
    let (n0, n1) = ((prev / len).floor(), (now / len).floor());
    let (a, b) = (prev - n0 * len, now - n1 * len);
    if n0 == n1 {
        for (i, e) in clip.events.iter().enumerate() {
            if in_range(e.time, a, b, fresh) {
                out.push(i);
            }
        }
        return out;
    }
    for (i, e) in clip.events.iter().enumerate() {
        if in_range(e.time, a, len, fresh) {
            out.push(i);
        }
    }
    for _ in 0..((n1 - n0 - 1.0) as usize).min(4) {
        out.extend(0..clip.events.len());
    }
    for (i, e) in clip.events.iter().enumerate() {
        if in_range(e.time, 0.0, b, true) && e.time < len {
            out.push(i);
        }
    }
    out
}

#[derive(Default)]
struct RootAcc {
    /// Motion in the animated object's local space.
    position: Vector3,
    yaw: f32,
}

/// Samples clips of one layer into a weighted pose.
struct Sampler<'a> {
    world: &'a World,
    assets: &'a AssetServer,
    root: GameObjectId,
    bindings: &'a mut HashMap<String, Option<GameObjectId>>,
    mask: Option<&'a AvatarMask>,
    /// Transforms of humanoid body parts the mask leaves out.
    excluded: Option<&'a std::collections::HashSet<GameObjectId>>,
    mirror_map: Option<&'a mirror::MirrorMap>,
    /// Clips retargeted to this rig, used instead of the originals.
    retargeted:
        Option<&'a HashMap<oxide_assets::AssetId, std::sync::Arc<oxide_assets::AnimationClip>>>,
    /// The rest pose (Direct blend trees whose weights sum below 1 blend toward it).
    rest: Option<&'a HashMap<GameObjectId, (Vector3, Quaternion, Vector3)>>,
    /// Extract root motion (Base Layer with `apply_root_motion`).
    root_motion: bool,
    /// Sample every clip at this time instead (additive reference pose).
    time_override: Option<f32>,
    collect_events: bool,
    pose: HashMap<GameObjectId, Channel>,
    rm: RootAcc,
    events: Vec<(String, Value)>,
    /// Property curves: (weighted sum, weight) per animated property.
    props: HashMap<PropKey, (f32, f32)>,
    /// Object curves: (value, weight); the heaviest motion's value wins.
    objs: HashMap<PropKey, (String, f32)>,
}

/// An animated component property: (object, component type, property with channel).
type PropKey = (GameObjectId, String, String);

/// Finished property values (weighted sums divided by their weights).
fn finalize_props(props: HashMap<PropKey, (f32, f32)>) -> HashMap<PropKey, f32> {
    props
        .into_iter()
        .filter(|(_, (_, w))| *w > 0.0)
        .map(|(k, (v, w))| (k, v / w))
        .collect()
}

/// Splits `color.r` into the field and a channel index (`x/r` = 0 ... `w/a` = 3).
fn split_channel(property: &str) -> (&str, Option<usize>) {
    match property.rsplit_once('.') {
        Some((f, c)) => match c {
            "x" | "r" => (f, Some(0)),
            "y" | "g" => (f, Some(1)),
            "z" | "b" => (f, Some(2)),
            "w" | "a" => (f, Some(3)),
            _ => (property, None),
        },
        None => (property, None),
    }
}

/// The current value of an animated property (to blend layers onto), if it exists.
pub fn read_property(
    world: &World,
    id: GameObjectId,
    component: &str,
    property: &str,
) -> Option<f32> {
    let o = world.get(id)?;
    let (cid, _, _) = o.component_list().into_iter().find(|c| c.1 == component)?;
    let (field, ch) = split_channel(property);
    let v = world.component_dyn(id, cid)?.get_field(field)?;
    Some(match (v, ch) {
        (Value::Float(f), None) => f as f32,
        (Value::Int(i), None) => i as f32,
        (Value::Bool(b), None) => f32::from(u8::from(b)),
        (Value::Vector2(a), Some(i)) => *a.get(i)?,
        (Value::Vector3(a), Some(i)) => *a.get(i)?,
        (Value::Vector4(a) | Value::Color(a) | Value::Quaternion(a), Some(i)) => *a.get(i)?,
        _ => return None,
    })
}

/// Writes animated property values through reflection.
fn apply_props(world: &mut World, props: HashMap<PropKey, f32>) {
    for ((id, component, property), x) in props {
        write_property(world, id, &component, &property, x);
    }
}

/// Writes object keys: asset fields get the asset path, text fields the text.
fn apply_objs(world: &mut World, objs: HashMap<PropKey, String>) {
    for ((id, component, property), v) in objs {
        write_object_property(world, id, &component, &property, &v);
    }
}

/// Sets an asset or string field from an object key. Returns whether it changed.
pub fn write_object_property(
    world: &mut World,
    id: GameObjectId,
    component: &str,
    property: &str,
    v: &str,
) -> bool {
    let Some(o) = world.get(id) else {
        return false;
    };
    let Some((cid, _, _)) = o.component_list().into_iter().find(|c| c.1 == component) else {
        return false;
    };
    if let Some(p) = v.split('#').next()
        && p.starts_with("Assets/")
        && let Some(a) = world.try_resource_mut::<AssetServer>()
    {
        let _ = a.ensure_loaded(p);
    }
    let Some(c) = world.component_dyn_mut(id, cid) else {
        return false;
    };
    let nv = match c.get_field(property) {
        Some(Value::String(old)) if old == v => return false,
        Some(Value::Asset(old)) if old == v => return false,
        Some(Value::String(_)) => Value::String(v.to_owned()),
        Some(Value::Asset(_) | Value::Null) => Value::Asset(v.to_owned()),
        _ => return false,
    };
    c.set_field(property, nv).is_ok()
}

/// The current value of an asset or string field, for object keys.
pub fn read_object_property(
    world: &World,
    id: GameObjectId,
    component: &str,
    property: &str,
) -> Option<String> {
    let o = world.get(id)?;
    let (cid, _, _) = o.component_list().into_iter().find(|c| c.1 == component)?;
    match world.component_dyn(id, cid)?.get_field(property)? {
        Value::String(s) | Value::Asset(s) => Some(s),
        _ => None,
    }
}

/// The transform at `path` below `root` (`""` is the root), by child names like Unity's
/// binding paths.
pub fn find_path(world: &World, root: GameObjectId, path: &str) -> Option<GameObjectId> {
    let mut cur = root;
    for part in path.split('/').filter(|p| !p.is_empty()) {
        cur = world
            .children(cur)
            .iter()
            .copied()
            .find(|&c| world.get(c).is_some_and(|o| o.name == part))?;
    }
    Some(cur)
}

/// Sets one animated property (`intensity`, `color.r`) through reflection, as a clip does.
/// Returns whether it was written.
pub fn write_property(
    world: &mut World,
    id: GameObjectId,
    component: &str,
    property: &str,
    x: f32,
) -> bool {
    {
        let Some(o) = world.get(id) else {
            return false;
        };
        let Some((cid, _, _)) = o.component_list().into_iter().find(|c| c.1 == component) else {
            return false;
        };
        let (field, ch) = split_channel(property);
        let Some(c) = world.component_dyn_mut(id, cid) else {
            return false;
        };
        let Some(v) = c.get_field(field) else {
            return false;
        };
        let nv = match (v, ch) {
            (Value::Float(_), None) => Value::Float(x as f64),
            (Value::Int(_), None) => Value::Int(x.round() as i64),
            (Value::Bool(_), None) => Value::Bool(x >= 0.5),
            (Value::Vector2(mut a), Some(i)) if i < 2 => {
                a[i] = x;
                Value::Vector2(a)
            }
            (Value::Vector3(mut a), Some(i)) if i < 3 => {
                a[i] = x;
                Value::Vector3(a)
            }
            (Value::Vector4(mut a), Some(i)) => {
                a[i] = x;
                Value::Vector4(a)
            }
            (Value::Color(mut a), Some(i)) => {
                a[i] = x;
                Value::Color(a)
            }
            _ => return false,
        };
        c.set_field(field, nv).is_ok()
    }
}

impl<'a> Sampler<'a> {
    fn new(
        world: &'a World,
        assets: &'a AssetServer,
        root: GameObjectId,
        bindings: &'a mut HashMap<String, Option<GameObjectId>>,
    ) -> Self {
        Self {
            world,
            assets,
            root,
            bindings,
            mask: None,
            excluded: None,
            mirror_map: None,
            retargeted: None,
            rest: None,
            root_motion: false,
            time_override: None,
            collect_events: true,
            pose: HashMap::new(),
            rm: RootAcc::default(),
            events: vec![],
            props: HashMap::new(),
            objs: HashMap::new(),
        }
    }

    fn add(
        &mut self,
        id: GameObjectId,
        p: Option<Vector3>,
        r: Option<Quaternion>,
        s: Option<Vector3>,
        w: f32,
    ) {
        if self.excluded.is_some_and(|x| x.contains(&id)) {
            return;
        }
        let e = self.pose.entry(id).or_default();
        if let Some(p) = p {
            e.pos += p * w;
            e.pw += w;
        }
        if let Some(r) = r {
            let r = if e.rot.dot(r) < 0.0 { -r } else { r };
            e.rot += r * w;
            e.rw += w;
        }
        if let Some(s) = s {
            e.scale += s * w;
            e.sw += w;
        }
    }

    /// The rest pose with weight `w`.
    fn rest_pose(&mut self, w: f32) {
        let Some(rest) = self.rest else { return };
        for (id, (p, r, s)) in rest {
            self.add(*id, Some(*p), Some(*r), Some(*s), w);
        }
    }

    /// Samples a clip, mirrored left/right when `mirror` is set.
    #[allow(clippy::too_many_arguments)]
    fn clip_mirrored(
        &mut self,
        clip: &AnimationClip,
        prev: f32,
        now: f32,
        looping: bool,
        w: f32,
        events: bool,
        mirror: bool,
    ) {
        let Some(map) = self.mirror_map.filter(|m| mirror && !m.is_empty()) else {
            self.clip(clip, prev, now, looping, w, events);
            return;
        };
        // Sample into a scratch pose, mirror it, then add it.
        let pose = std::mem::take(&mut self.pose);
        let rm = std::mem::take(&mut self.rm);
        self.clip(clip, prev, now, looping, 1.0, events);
        let sampled = std::mem::replace(&mut self.pose, pose);
        let d = std::mem::replace(&mut self.rm, rm);
        let mut samples: Vec<mirror::Sample> = finalize(sampled)
            .into_iter()
            .map(|(id, (p, r, s))| (id, p, r, s))
            .collect();
        map.apply(&mut samples);
        for (id, p, r, s) in samples {
            self.add(id, p, r, s, w);
        }
        self.rm.position += Vector3::new(-d.position.x, d.position.y, d.position.z) * w;
        self.rm.yaw -= d.yaw * w;
    }

    #[allow(clippy::too_many_arguments)]
    fn clip(
        &mut self,
        clip: &AnimationClip,
        prev: f32,
        now: f32,
        looping: bool,
        w: f32,
        events: bool,
    ) {
        if w <= 0.0 {
            return;
        }
        let len = clip.length;
        let t = self
            .time_override
            .unwrap_or_else(|| wrap(now, len, looping));
        let rc = if self.root_motion && self.time_override.is_none() {
            clip.root_curve()
        } else {
            None
        };
        for (ci, c) in clip.curves.iter().enumerate() {
            if let Some(m) = self.mask
                && !m.includes(&c.path)
            {
                continue;
            }
            let Some(id) = bind(self.world, self.root, self.bindings, &c.path) else {
                continue;
            };
            let mut p = c.position.as_ref().and_then(|tr| tr.sample(t));
            let mut r = c.rotation.as_ref().and_then(|tr| tr.sample(t));
            let s = c.scale.as_ref().and_then(|tr| tr.sample(t));
            if rc == Some(ci) && id != self.root {
                (p, r) = self.extract_root(c, id, prev, now, len, looping, w, p, r);
            }
            self.add(id, p, r, s, w);
        }
        for pc in &clip.properties {
            if let Some(m) = self.mask
                && !pc.path.is_empty()
                && !m.includes(&pc.path)
            {
                continue;
            }
            let id = if pc.path.is_empty() {
                Some(self.root)
            } else {
                bind(self.world, self.root, self.bindings, &pc.path)
            };
            let Some(id) = id else {
                continue;
            };
            let e = self
                .props
                .entry((id, pc.component.clone(), pc.property.clone()))
                .or_insert((0.0, 0.0));
            e.0 += pc.curve.evaluate(t) * w;
            e.1 += w;
        }
        for oc in &clip.object_curves {
            let id = if oc.path.is_empty() {
                Some(self.root)
            } else {
                bind(self.world, self.root, self.bindings, &oc.path)
            };
            let (Some(id), Some(v)) = (id, oc.sample(t)) else {
                continue;
            };
            let e = self
                .objs
                .entry((id, oc.component.clone(), oc.property.clone()))
                .or_insert((String::new(), -1.0));
            if w > e.1 {
                *e = (v.to_owned(), w);
            }
        }
        if events && self.collect_events && self.time_override.is_none() {
            for i in events_between(clip, prev, now, looping) {
                let e = &clip.events[i];
                self.events
                    .push((e.function_name.clone(), e.parameter.clone()));
            }
        }
    }

    /// Moves the root node's horizontal motion and yaw out of the pose into `rm` (Unity
    /// root motion with "Bake Into Pose" off for XZ and rotation, on for Y).
    #[allow(clippy::too_many_arguments)]
    fn extract_root(
        &mut self,
        c: &oxide_assets::TransformCurves,
        bone: GameObjectId,
        prev: f32,
        now: f32,
        len: f32,
        looping: bool,
        w: f32,
        p: Option<Vector3>,
        r: Option<Quaternion>,
    ) -> (Option<Vector3>, Option<Quaternion>) {
        // Bone-parent space → animated-object space.
        let to_root = match self.world.parent(bone) {
            Some(parent) => {
                self.world.world_to_local(self.root) * self.world.local_to_world(parent)
            }
            None => return (p, r),
        };
        let (_, mrot, _) = to_root.to_scale_rotation_translation();
        let loops = if looping && len > 0.0 {
            (now / len).floor() - (prev / len).floor()
        } else {
            0.0
        };
        let (tp, tn) = (wrap(prev, len, looping), wrap(now, len, looping));
        let yaw = |t: f32| -> f32 {
            c.rotation
                .as_ref()
                .and_then(|tr| Some((tr.sample(t)?, tr.sample(0.0)?)))
                .map_or(0.0, |(q, q0)| yaw_of((mrot * q) * (mrot * q0).inverse()))
        };
        let (yaw_prev, yaw_now) = (yaw(tp), yaw(tn));
        let d_yaw = wrap_pi(yaw_now - yaw_prev) + loops * yaw(len);
        let mut out_p = p;
        if let Some(tr) = c.position.as_ref()
            && let (Some(pn), Some(pp), Some(p0), Some(pe)) =
                (tr.sample(tn), tr.sample(tp), tr.sample(0.0), tr.sample(len))
        {
            let mut d = to_root.transform_vector3(pn - pp + (pe - p0) * loops);
            d.y = 0.0;
            self.rm.position += (Quaternion::from_rotation_y(-yaw_prev) * d) * w;
            let mut off = to_root.transform_vector3(pn - p0);
            off.y = 0.0;
            out_p = p.map(|p| p - to_root.inverse().transform_vector3(off));
        }
        self.rm.yaw += d_yaw * w;
        let out_r = r.map(|r| mrot.inverse() * Quaternion::from_rotation_y(-yaw_now) * mrot * r);
        (out_p, out_r)
    }
}

/// One clip sampled by [`play_clips`] at `time` (seconds into the clip) with `weight`.
pub struct ClipSample<'a> {
    pub clip: &'a AnimationClip,
    pub time: f32,
    pub weight: f32,
}

/// The pose objects had before something animated them (Timeline blends toward it when no
/// Animator Controller writes a fresh pose each frame).
#[derive(Clone, Debug, Default)]
pub struct RestPose {
    pub transforms: HashMap<GameObjectId, (Vector3, Quaternion, Vector3)>,
    pub props: HashMap<(GameObjectId, String, String), f32>,
}

/// Samples `samples` (weights normalised) onto the hierarchy under `root`, blended over its
/// current pose (or over `rest`, recorded the first time each value is written) by `blend`
/// (0..1). Used by Timeline animation tracks: their output
/// overrides the Animator's (which ran earlier in the frame) and fades into it while a clip
/// eases in or out. `offset` (position, rotation) moves the root's animated position and
/// rotation (Timeline track offsets).
#[allow(clippy::too_many_arguments)]
pub fn play_clips(
    world: &mut World,
    assets: &AssetServer,
    root: GameObjectId,
    bindings: &mut HashMap<String, Option<GameObjectId>>,
    samples: &[ClipSample],
    blend: f32,
    offset: Option<(Vector3, Quaternion)>,
    mask: Option<&AvatarMask>,
    mut rest: Option<&mut RestPose>,
) {
    let blend = blend.clamp(0.0, 1.0);
    if blend <= 0.0 || samples.is_empty() {
        return;
    }
    let (pose, props, objs) = {
        let mut s = Sampler::new(world, assets, root, bindings);
        s.collect_events = false;
        s.mask = mask;
        for c in samples {
            s.clip(c.clip, c.time, c.time, false, c.weight.max(0.0), false);
        }
        let objs: HashMap<PropKey, String> = s.objs.into_iter().map(|(k, (v, _))| (k, v)).collect();
        (finalize(s.pose), finalize_props(s.props), objs)
    };
    for (id, (mut p, mut r, s)) in pose {
        if id == root
            && let Some((op, or)) = offset
        {
            p = p.map(|p| or * p + op);
            r = r.map(|r| or * r);
        }
        let Some(t) = world.transform_mut(id) else {
            continue;
        };
        let cur = (t.local_position, t.local_rotation, t.local_scale);
        let base = match rest.as_deref_mut() {
            Some(r) => *r.transforms.entry(id).or_insert(cur),
            None => cur,
        };
        if let Some(p) = p {
            t.local_position = base.0.lerp(p, blend);
        }
        if let Some(r) = r {
            t.local_rotation = base.1.slerp(r.normalize(), blend).normalize();
        }
        if let Some(s) = s {
            t.local_scale = base.2.lerp(s, blend);
        }
    }
    for ((id, c, p), x) in props {
        let cur = read_property(world, id, &c, &p).unwrap_or(x);
        let base = match rest.as_deref_mut() {
            Some(r) => *r.props.entry((id, c.clone(), p.clone())).or_insert(cur),
            None => cur,
        };
        write_property(world, id, &c, &p, base + (x - base) * blend);
    }
    if blend >= 0.5 {
        apply_objs(world, objs);
    }
}

fn blend_weights_1d(thresholds: impl Iterator<Item = f32>, v: f32) -> Vec<(usize, f32)> {
    let t: Vec<f32> = thresholds.collect();
    if t.is_empty() {
        return vec![];
    }
    if v <= t[0] {
        return vec![(0, 1.0)];
    }
    for i in 0..t.len() - 1 {
        let (a, b) = (t[i], t[i + 1]);
        if v <= b {
            let f = if b > a { (v - a) / (b - a) } else { 0.0 };
            return vec![(i, 1.0 - f), (i + 1, f)];
        }
    }
    vec![(t.len() - 1, 1.0)]
}

/// Gradient band interpolation (Johansen 2009, what Unity's 2D blend trees use): cartesian
/// for Freeform Cartesian, polar (speed and direction blended separately) for the
/// directional types. Returns one weight per point, summing to 1.
pub fn blend_weights_2d(points: &[Vector2], p: Vector2, kind: BlendTree2DType) -> Vec<f32> {
    let n = points.len();
    if n == 0 {
        return vec![];
    }
    if n == 1 {
        return vec![1.0];
    }
    const ALPHA: f32 = 2.0;
    let signed = |a: Vector2, b: Vector2| a.perp_dot(b).atan2(a.dot(b));
    let mut w = vec![0.0f32; n];
    for i in 0..n {
        let pi = points[i];
        let mut h = f32::MAX;
        for (j, &pj) in points.iter().enumerate() {
            if i == j {
                continue;
            }
            let v = match kind {
                BlendTree2DType::FreeformCartesian => {
                    let ij = pj - pi;
                    let d = ij.length_squared();
                    if d < 1e-8 {
                        continue;
                    }
                    1.0 - (p - pi).dot(ij) / d
                }
                _ => {
                    let (li, lj, lp) = (pi.length(), pj.length(), p.length());
                    let mean = ((li + lj) * 0.5).max(1e-6);
                    // A point at the origin has no direction: measure from the other one.
                    let (dir_i, ang_ij) = if li < 1e-6 {
                        (pj, 0.0)
                    } else if lj < 1e-6 {
                        (pi, 0.0)
                    } else {
                        (pi, signed(pi, pj))
                    };
                    let ang_ip = if lp < 1e-6 { 0.0 } else { signed(dir_i, p) };
                    let vij = Vector2::new((lj - li) / mean, ang_ij * ALPHA);
                    let vip = Vector2::new((lp - li) / mean, ang_ip * ALPHA);
                    let d = vij.length_squared();
                    if d < 1e-8 {
                        continue;
                    }
                    1.0 - vip.dot(vij) / d
                }
            };
            h = h.min(v);
        }
        w[i] = if h == f32::MAX { 0.0 } else { h.max(0.0) };
    }
    let sum: f32 = w.iter().sum();
    if sum > 1e-6 {
        w.iter_mut().for_each(|x| *x /= sum);
    } else {
        // Outside every band: the nearest point.
        let best = (0..n)
            .min_by(|&a, &b| points[a].distance(p).total_cmp(&points[b].distance(p)))
            .expect("n > 1");
        w = vec![0.0; n];
        w[best] = 1.0;
    }
    w
}

fn apply(world: &mut World, pose: HashMap<GameObjectId, Trs>) {
    for (id, (p, r, s)) in pose {
        if let Some(t) = world.transform_mut(id) {
            if let Some(p) = p {
                t.local_position = p;
            }
            if let Some(r) = r {
                t.local_rotation = r.normalize();
            }
            if let Some(s) = s {
                t.local_scale = s;
            }
        }
    }
}

/// Runs one Animator: pose, root motion (`OnAnimatorMove`), IK (`OnAnimatorIK`), events.
fn update_animator(
    world: &mut World,
    assets: &AssetServer,
    id: GameObjectId,
    cid: oxide_scene::ComponentId,
    dt: f32,
) {
    let Some(mut a) = world.component::<Animator>(id, cid).cloned() else {
        return;
    };
    let step = dt * a.speed;
    let out = controller::evaluate(world, assets, id, &mut a, step);
    let Some(out) = out else {
        if let Some(c) = world.component_mut::<Animator>(id, cid) {
            c.rt = a.rt;
        }
        return;
    };
    apply(world, out.pose);
    apply_props(world, out.props);
    apply_objs(world, out.objs);
    // Root motion, in world space.
    let to_world = world.local_to_world(id);
    a.rt.delta_position = to_world.transform_vector3(out.root_position);
    a.rt.delta_rotation = Quaternion::from_rotation_y(out.root_yaw);
    a.rt.delta_time = step;
    let ctrl = a
        .controller
        .as_ref()
        .and_then(|h| assets.controllers.get(h));
    let ik_layers: Vec<usize> = ctrl
        .map(|c| {
            c.layers
                .iter()
                .enumerate()
                .filter(|(_, l)| l.ik_pass)
                .map(|(i, _)| i)
                .collect()
        })
        .unwrap_or_default();
    // Current (and incoming) states per layer, for OnStateMove / OnStateIK.
    let playing: Vec<(usize, usize)> =
        a.rt.layers
            .iter()
            .enumerate()
            .flat_map(|(li, l)| {
                l.current
                    .iter()
                    .chain(l.next.iter().map(|n| &n.0))
                    .map(move |p| (li, p.state))
            })
            .collect();
    let apply_rm = a.apply_root_motion;
    if let Some(c) = world.component_mut::<Animator>(id, cid) {
        c.rt = a.rt;
    }
    behaviours_dispatch(world, assets, id, cid, &out.state_events);
    if apply_rm && !world.send_message(id, "OnAnimatorMove", Value::Null) {
        let (dp, dq) = {
            let c = world.component::<Animator>(id, cid).expect("alive");
            (c.rt.delta_position, c.rt.delta_rotation)
        };
        let (p, r) = (world.position(id), world.rotation(id));
        world.set_position(id, p + dp);
        world.set_rotation(id, (r * dq).normalize());
    }
    let moves: Vec<(usize, controller::StateEvent)> = playing
        .iter()
        .map(|&(l, s)| (l, controller::StateEvent::Move(s)))
        .collect();
    behaviours_dispatch(world, assets, id, cid, &moves);
    if !ik_layers.is_empty() {
        for &l in &ik_layers {
            world.send_message(id, "OnAnimatorIK", Value::Int(l as i64));
            let iks: Vec<(usize, controller::StateEvent)> = playing
                .iter()
                .filter(|(pl, _)| *pl == l)
                .map(|&(l, s)| (l, controller::StateEvent::Ik(s)))
                .collect();
            behaviours_dispatch(world, assets, id, cid, &iks);
        }
        if let Some(c) = world.component::<Animator>(id, cid).cloned() {
            solve_ik(world, id, &c);
        }
    }
    for (name, arg) in out.events {
        world.send_message(id, &name, arg);
    }
}

/// Runs StateMachineBehaviour callbacks for `events` (Play mode only).
fn behaviours_dispatch(
    world: &mut World,
    assets: &AssetServer,
    id: GameObjectId,
    cid: oxide_scene::ComponentId,
    events: &[(usize, controller::StateEvent)],
) {
    use controller::StateEvent as E;
    if events.is_empty() || !world.run_scripts {
        return;
    }
    let Some(ctrl) = world
        .component::<Animator>(id, cid)
        .and_then(|a| a.controller.as_ref())
        .and_then(|h| assets.controllers.get(h))
    else {
        return;
    };
    fn has_behaviours(m: &AnimatorStateMachine) -> bool {
        !m.behaviours.is_empty()
            || m.states.iter().any(|s| !s.behaviours.is_empty())
            || m.state_machines.iter().any(has_behaviours)
    }
    let has_any = ctrl.layers.iter().any(|l| has_behaviours(&l.state_machine));
    if !has_any {
        return;
    }
    let Some(mut reg) = world.resources.remove::<StateMachineBehaviours>() else {
        return;
    };
    let ctrl = ctrl.clone();
    for (li, ev) in events {
        let Some(layer) = ctrl.layers.get(*li) else {
            continue;
        };
        if layer.synced_layer_index.is_some() {
            continue;
        }
        let root = &layer.state_machine;
        let flat = root.flatten();
        let info_for = |world: &World, s: usize| -> AnimatorStateInfo {
            let a = world.component::<Animator>(id, cid);
            let st = flat[s].1;
            let name = &st.name;
            a.and_then(|a| a.rt.layers.get(*li))
                .and_then(|l| {
                    [Some(&l.info), l.next_info.as_ref()]
                        .into_iter()
                        .flatten()
                        .find(|i| &i.name == name)
                        .cloned()
                })
                .unwrap_or_else(|| AnimatorStateInfo {
                    name: name.clone(),
                    full_path: format!("{}.{}", layer.name, name),
                    tag: st.tag.clone(),
                    looping: st.looping,
                    speed: st.speed,
                    ..Default::default()
                })
        };
        match ev {
            E::Enter(s) | E::Update(s) | E::Exit(s) | E::Move(s) | E::Ik(s) => {
                let Some((path, st)) = flat.get(*s) else {
                    continue;
                };
                // The state's own behaviours, then those of the machines around it.
                let mut owners: Vec<(String, &[oxide_assets::StateBehaviour])> =
                    vec![(format!("s{s}"), st.behaviours.as_slice())];
                for d in 0..=path.machine.len() {
                    if let Some(m) = root.machine(&path.machine[..d]) {
                        owners.push((
                            format!("m{:?}", &path.machine[..d]),
                            m.behaviours.as_slice(),
                        ));
                    }
                }
                let info = info_for(world, *s);
                for (owner, list) in owners {
                    for (bi, b) in list.iter().enumerate() {
                        let key = behaviours::InstanceKey {
                            animator: (id, cid),
                            layer: *li,
                            owner: owner.clone(),
                            index: bi,
                        };
                        if !reg.instances.contains_key(&key) {
                            let Some(inst) = reg.instantiate(b) else {
                                continue;
                            };
                            reg.instances.insert(key.clone(), inst);
                        }
                        let inst = reg.instances.get_mut(&key).expect("inserted");
                        let mut ctx = oxide_scene::Ctx {
                            world,
                            this: id,
                            component: cid,
                        };
                        match ev {
                            E::Enter(_) => inst.on_state_enter(&mut ctx, &info, *li),
                            E::Update(_) => inst.on_state_update(&mut ctx, &info, *li),
                            E::Exit(_) => inst.on_state_exit(&mut ctx, &info, *li),
                            E::Move(_) => inst.on_state_move(&mut ctx, &info, *li),
                            E::Ik(_) => inst.on_state_ik(&mut ctx, &info, *li),
                            _ => {}
                        }
                    }
                }
            }
            E::MachineEnter(p) | E::MachineExit(p) => {
                let Some(m) = root.machine(p) else { continue };
                for (bi, b) in m.behaviours.iter().enumerate() {
                    let key = behaviours::InstanceKey {
                        animator: (id, cid),
                        layer: *li,
                        owner: format!("m{p:?}"),
                        index: bi,
                    };
                    if !reg.instances.contains_key(&key) {
                        let Some(inst) = reg.instantiate(b) else {
                            continue;
                        };
                        reg.instances.insert(key.clone(), inst);
                    }
                    let inst = reg.instances.get_mut(&key).expect("inserted");
                    let mut ctx = oxide_scene::Ctx {
                        world,
                        this: id,
                        component: cid,
                    };
                    if matches!(ev, E::MachineEnter(_)) {
                        inst.on_state_machine_enter(&mut ctx, &m.name);
                    } else {
                        inst.on_state_machine_exit(&mut ctx, &m.name);
                    }
                }
            }
        }
    }
    world.resources.insert(reg);
}

/// Which way a humanoid faces, from its rig: the hip line (left → right) crossed with up.
/// Models are not always authored facing their root's +Z.
fn facing(world: &World, h: &HashMap<HumanBodyBones, GameObjectId>) -> Option<Vector3> {
    let (l, r) = (
        h.get(&HumanBodyBones::LeftUpperLeg)?,
        h.get(&HumanBodyBones::RightUpperLeg)?,
    );
    let mut right = world.position(*r) - world.position(*l);
    right.y = 0.0;
    right.try_normalize()?.cross(Vector3::Y).try_normalize()
}

fn solve_ik(world: &mut World, root: GameObjectId, a: &Animator) {
    use HumanBodyBones as B;
    let h = &a.rt.human;
    let bone = |b: B| h.get(&b).copied();
    let limbs = [
        (
            AvatarIKGoal::LeftFoot,
            [B::LeftUpperLeg, B::LeftLowerLeg, B::LeftFoot],
            AvatarIKHint::LeftKnee,
        ),
        (
            AvatarIKGoal::RightFoot,
            [B::RightUpperLeg, B::RightLowerLeg, B::RightFoot],
            AvatarIKHint::RightKnee,
        ),
        (
            AvatarIKGoal::LeftHand,
            [B::LeftUpperArm, B::LeftLowerArm, B::LeftHand],
            AvatarIKHint::LeftElbow,
        ),
        (
            AvatarIKGoal::RightHand,
            [B::RightUpperArm, B::RightLowerArm, B::RightHand],
            AvatarIKHint::RightElbow,
        ),
    ];
    // LookAt first: it moves the shoulders the hands hang from.
    let l = a.rt.look_at;
    if l.weight > 0.0
        && let Some(head) = bone(B::Head)
    {
        let spine: Vec<GameObjectId> = [B::Spine, B::Chest, B::UpperChest]
            .into_iter()
            .filter_map(bone)
            .collect();
        let neck: Vec<GameObjectId> = [B::Neck]
            .into_iter()
            .filter_map(bone)
            .chain([head])
            .collect();
        let fwd = facing(world, h).unwrap_or_else(|| world.forward(root));
        ik::look_at(
            world,
            fwd,
            &spine,
            &neck,
            &ik::LookAt {
                target: l.position,
                weight: l.weight,
                body: l.body,
                head: l.head,
                clamp: l.clamp,
            },
        );
    }
    for (goal, bones, hint) in limbs {
        let g = a.rt.ik[goal as usize];
        let (Some(u), Some(m), Some(e)) = (bone(bones[0]), bone(bones[1]), bone(bones[2])) else {
            continue;
        };
        let (hp, hw) = a.rt.hints[hint as usize];
        let hint = (hw > 0.0).then(|| {
            let cur = world.position(m);
            cur.lerp(hp, hw)
        });
        ik::solve_two_bone(world, [u, m, e], g.position, g.position_weight, hint);
        ik::set_end_rotation(world, e, g.rotation, g.rotation_weight);
    }
}

#[derive(Default)]
pub struct AnimationSystem;

impl AnimationSystem {
    fn run(world: &mut World, physics_step: bool) {
        let time = world.resource::<Time>();
        let (dt, unscaled, fixed) = (
            time.delta_time(),
            time.unscaled_delta_time(),
            time.fixed_delta_time(),
        );
        let Some(assets) = world.resources.remove::<AssetServer>() else {
            return;
        };
        for (id, cid) in world.query_ids::<Animator>() {
            if !world.is_component_enabled(id, cid)
                || !world.get(id).is_some_and(|o| o.active_in_hierarchy())
            {
                continue;
            }
            let Some(mode) = world.component::<Animator>(id, cid).map(|a| a.update_mode) else {
                continue;
            };
            let step = match (mode, physics_step) {
                (AnimatorUpdateMode::AnimatePhysics, true) => fixed,
                (AnimatorUpdateMode::Normal, false) => dt,
                (AnimatorUpdateMode::UnscaledTime, false) => unscaled,
                _ => continue,
            };
            update_animator(world, &assets, id, cid, step);
        }
        if !physics_step {
            update_legacy(world, &assets, dt);
        }
        world.resources.insert(assets);
    }
}

fn update_legacy(world: &mut World, assets: &AssetServer, dt: f32) {
    for (id, cid) in world.query_ids::<Animation>() {
        let Some(mut a) = world.component::<Animation>(id, cid).cloned() else {
            continue;
        };
        if a.playing.is_none() {
            a.playing = Some(a.play_automatically);
        }
        let mut events = vec![];
        let mut pose = HashMap::new();
        let mut props = HashMap::new();
        let mut objs = HashMap::new();
        if a.playing == Some(true)
            && let Some(clip) = a.clip.as_ref().and_then(|h| assets.animations.get(h))
        {
            let prev = a.time;
            a.time += dt * a.speed;
            if !a.looping && a.time >= clip.length {
                a.playing = Some(false);
            }
            let mut s = Sampler::new(world, assets, id, &mut a.bindings);
            s.clip(clip, prev, a.time, a.looping, 1.0, true);
            pose = finalize(s.pose);
            props = finalize_props(s.props);
            objs = s.objs.into_iter().map(|(k, (v, _))| (k, v)).collect();
            events = s.events;
        }
        if let Some(c) = world.component_mut::<Animation>(id, cid) {
            c.time = a.time;
            c.playing = a.playing;
            c.bindings = a.bindings;
        }
        apply(world, pose);
        apply_props(world, props);
        apply_objs(world, objs);
        for (name, arg) in events {
            world.send_message(id, &name, arg);
        }
    }
}

impl System for AnimationSystem {
    fn name(&self) -> &'static str {
        "Animation"
    }

    fn fixed_update(&mut self, world: &mut World) {
        Self::run(world, true);
    }

    fn update(&mut self, world: &mut World) {
        Self::run(world, false);
    }
}

/// Installs the Animator/Animation components; returns the system.
pub fn install(world: &mut World) -> Box<dyn System> {
    world
        .registry
        .register::<Animator>("Miscellaneous/Animator");
    world
        .registry
        .register::<Animation>("Miscellaneous/Animation");
    if !world.resources.contains::<StateMachineBehaviours>() {
        world.resources.insert(StateMachineBehaviours::default());
    }
    Box::new(AnimationSystem)
}

#[cfg(test)]
mod tests;
