//! Oxide animation (Unity Mecanim subset): the `Animator` component plays an
//! `AnimatorController` — parameters, states (clips or 1D blend trees), conditional
//! transitions with crossfades and exit times — and the legacy `Animation` component plays
//! a single clip. Curves bind to child transforms by path, like Unity.
//!
//! Runs after `Update` and before `LateUpdate` (Unity's "internal animation update").
//! Reference: <https://docs.unity3d.com/Manual/AnimationOverview.html>

use oxide_assets::{
    AnimationClip, AnimatorController, AssetServer, ConditionMode, Handle, Motion, ParameterType,
};
use oxide_core::{Reflect, Time};
use oxide_math::{Quaternion, Vector3};
use oxide_scene::{Component, GameObjectId, System, World};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct Playing {
    pub state: usize,
    pub time: f32,
}

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct AnimatorRuntime {
    pub params: HashMap<String, f32>,
    pub current: Option<Playing>,
    /// Crossfade target, elapsed, duration.
    pub next: Option<(Playing, f32, f32)>,
    pub pending: Option<(String, f32)>,
    pub bindings: HashMap<String, Option<GameObjectId>>,
    pub controller: Option<oxide_assets::AssetId>,
}

/// Unity `Animator`.
#[derive(Clone, Debug, Reflect)]
pub struct Animator {
    pub controller: Option<Handle<AnimatorController>>,
    pub speed: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub rt: AnimatorRuntime,
}

impl Default for Animator {
    fn default() -> Self {
        Self {
            controller: None,
            speed: 1.0,
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
    pub fn set_float(&mut self, name: &str, v: f32) {
        self.rt.params.insert(name.into(), v);
    }
    pub fn get_float(&self, name: &str) -> f32 {
        self.rt.params.get(name).copied().unwrap_or(0.0)
    }
    pub fn set_integer(&mut self, name: &str, v: i32) {
        self.rt.params.insert(name.into(), v as f32);
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
    /// Unity `Play(stateName)`: jump to a state immediately.
    pub fn play(&mut self, state: &str) {
        self.rt.pending = Some((state.into(), 0.0));
    }
    /// Unity `CrossFade(stateName, duration)` (seconds).
    pub fn cross_fade(&mut self, state: &str, duration: f32) {
        self.rt.pending = Some((state.into(), duration.max(0.0)));
    }
    pub fn is_in_transition(&self) -> bool {
        self.rt.next.is_some()
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

/// Per-object local TRS produced by sampling.
type Pose = HashMap<GameObjectId, (Option<Vector3>, Option<Quaternion>, Option<Vector3>)>;

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

fn sample_clip(
    world: &World,
    root: GameObjectId,
    cache: &mut HashMap<String, Option<GameObjectId>>,
    clip: &AnimationClip,
    t: f32,
    w: f32,
    pose: &mut Pose,
) {
    for c in &clip.curves {
        let Some(id) = bind(world, root, cache, &c.path) else {
            continue;
        };
        let e = pose.entry(id).or_insert((None, None, None));
        let p = c.position.as_ref().and_then(|tr| tr.sample(t));
        let r = c.rotation.as_ref().and_then(|tr| tr.sample(t));
        let s = c.scale.as_ref().and_then(|tr| tr.sample(t));
        // Accumulate a weighted blend (weights of all contributions sum to 1).
        if let Some(p) = p {
            e.0 = Some(e.0.map_or(p * w, |a| a + p * w));
        }
        if let Some(r) = r {
            e.1 = Some(match e.1 {
                None => r * w,
                Some(a) => {
                    let r = if a.dot(r) < 0.0 { -r } else { r };
                    a + r * w
                }
            });
        }
        if let Some(s) = s {
            e.2 = Some(e.2.map_or(s * w, |a| a + s * w));
        }
    }
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

/// Length of a state's motion (weighted for blend trees).
fn motion_length(assets: &AssetServer, m: &Motion, params: &HashMap<String, f32>) -> f32 {
    match m {
        Motion::Clip(h) => assets.animations.get(h).map_or(0.0, |c| c.length),
        Motion::BlendTree1D {
            parameter,
            children,
        } => blend_weights(children, params.get(parameter).copied().unwrap_or(0.0))
            .into_iter()
            .map(|(i, w)| {
                assets
                    .animations
                    .get(&children[i].1)
                    .map_or(0.0, |c| c.length)
                    * w
            })
            .sum(),
        Motion::Empty => 0.0,
    }
}

fn blend_weights(children: &[(f32, Handle<AnimationClip>)], v: f32) -> Vec<(usize, f32)> {
    if children.is_empty() {
        return vec![];
    }
    if v <= children[0].0 {
        return vec![(0, 1.0)];
    }
    for i in 0..children.len() - 1 {
        let (a, b) = (children[i].0, children[i + 1].0);
        if v <= b {
            let t = if b > a { (v - a) / (b - a) } else { 0.0 };
            return vec![(i, 1.0 - t), (i + 1, t)];
        }
    }
    vec![(children.len() - 1, 1.0)]
}

/// Samples one state into `pose` with weight `w`; time is normalized for blend trees so
/// differently sized clips stay in sync (Unity behaviour).
fn sample_state(
    world: &World,
    assets: &AssetServer,
    root: GameObjectId,
    cache: &mut HashMap<String, Option<GameObjectId>>,
    ctrl: &AnimatorController,
    p: &Playing,
    params: &HashMap<String, f32>,
    w: f32,
    pose: &mut Pose,
) {
    let Some(state) = ctrl.states.get(p.state) else {
        return;
    };
    match &state.motion {
        Motion::Clip(h) => {
            if let Some(c) = assets.animations.get(h) {
                sample_clip(
                    world,
                    root,
                    cache,
                    c,
                    wrap(p.time, c.length, state.looping),
                    w,
                    pose,
                );
            }
        }
        Motion::BlendTree1D {
            parameter,
            children,
        } => {
            let len = motion_length(assets, &state.motion, params).max(1e-4);
            let norm = wrap(p.time, len, state.looping) / len;
            for (i, bw) in blend_weights(children, params.get(parameter).copied().unwrap_or(0.0)) {
                if let Some(c) = assets.animations.get(&children[i].1) {
                    sample_clip(world, root, cache, c, norm * c.length, w * bw, pose);
                }
            }
        }
        Motion::Empty => {}
    }
}

fn conditions_met(t: &oxide_assets::Transition, params: &HashMap<String, f32>) -> bool {
    t.conditions.iter().all(|c| {
        let v = params.get(&c.parameter).copied().unwrap_or(0.0);
        match c.mode {
            ConditionMode::If => v != 0.0,
            ConditionMode::IfNot => v == 0.0,
            ConditionMode::Greater(x) => v > x,
            ConditionMode::Less(x) => v < x,
            ConditionMode::Equals(x) => (v - x).abs() < 1e-4,
            ConditionMode::NotEqual(x) => (v - x).abs() >= 1e-4,
        }
    })
}

/// Advances one Animator and returns the pose to apply.
fn evaluate(
    world: &World,
    assets: &AssetServer,
    root: GameObjectId,
    a: &mut Animator,
    dt: f32,
) -> Option<Pose> {
    let ctrl = assets.controllers.get(a.controller.as_ref()?)?;
    let rt = &mut a.rt;
    let cid = a.controller.as_ref().map(|h| h.id());
    if rt.controller != cid {
        rt.controller = cid;
        rt.current = Some(Playing {
            state: ctrl.default_state.min(ctrl.states.len().saturating_sub(1)),
            time: 0.0,
        });
        rt.next = None;
        rt.bindings.clear();
        for p in &ctrl.parameters {
            rt.params.entry(p.name.clone()).or_insert(p.default);
        }
    }
    if ctrl.states.is_empty() {
        return None;
    }
    if let Some((name, fade)) = rt.pending.take()
        && let Some(i) = ctrl.state_index(&name)
    {
        if fade <= 0.0 || rt.current.is_none() {
            rt.current = Some(Playing {
                state: i,
                time: 0.0,
            });
            rt.next = None;
        } else {
            rt.next = Some((
                Playing {
                    state: i,
                    time: 0.0,
                },
                0.0,
                fade,
            ));
        }
    }
    let speed = |i: usize| ctrl.states.get(i).map_or(1.0, |s| s.speed);
    let cur = rt.current.get_or_insert(Playing {
        state: ctrl.default_state,
        time: 0.0,
    });
    cur.time += dt * speed(cur.state);
    let mut finished = false;
    if let Some((n, elapsed, dur)) = rt.next.as_mut() {
        n.time += dt * speed(n.state);
        *elapsed += dt;
        finished = *elapsed >= *dur;
    }
    if finished && let Some((n, _, _)) = rt.next.take() {
        rt.current = Some(n);
    }
    // Transitions (only when not already crossfading).
    if rt.next.is_none() {
        let cur = rt.current.clone().expect("current");
        let len = motion_length(assets, &ctrl.states[cur.state].motion, &rt.params).max(1e-4);
        let norm = cur.time / len;
        let pick = ctrl.transitions.iter().find(|t| {
            let from_ok = match &t.from {
                Some(f) => ctrl.states[cur.state].name == *f,
                None => ctrl.states.get(cur.state).is_none_or(|s| s.name != t.to),
            };
            from_ok && (!t.has_exit_time || norm >= t.exit_time) && conditions_met(t, &rt.params)
        });
        if let Some(t) = pick
            && let Some(to) = ctrl.state_index(&t.to)
        {
            // Consume the triggers this transition used.
            for c in &t.conditions {
                if ctrl
                    .parameters
                    .iter()
                    .any(|p| p.name == c.parameter && p.kind == ParameterType::Trigger)
                {
                    rt.params.insert(c.parameter.clone(), 0.0);
                }
            }
            if t.duration > 0.0 {
                rt.next = Some((
                    Playing {
                        state: to,
                        time: 0.0,
                    },
                    0.0,
                    t.duration,
                ));
            } else {
                rt.current = Some(Playing {
                    state: to,
                    time: 0.0,
                });
            }
        }
    }
    let mut pose = Pose::new();
    let cur = rt.current.clone().expect("current");
    match rt.next.clone() {
        Some((n, elapsed, dur)) => {
            let w = (elapsed / dur.max(1e-4)).clamp(0.0, 1.0);
            sample_state(
                world,
                assets,
                root,
                &mut rt.bindings,
                ctrl,
                &cur,
                &rt.params,
                1.0 - w,
                &mut pose,
            );
            sample_state(
                world,
                assets,
                root,
                &mut rt.bindings,
                ctrl,
                &n,
                &rt.params,
                w,
                &mut pose,
            );
        }
        None => sample_state(
            world,
            assets,
            root,
            &mut rt.bindings,
            ctrl,
            &cur,
            &rt.params,
            1.0,
            &mut pose,
        ),
    }
    Some(pose)
}

fn apply(world: &mut World, pose: Pose) {
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

#[derive(Default)]
pub struct AnimationSystem;

impl System for AnimationSystem {
    fn name(&self) -> &'static str {
        "Animation"
    }

    fn update(&mut self, world: &mut World) {
        let dt = world.resource::<Time>().delta_time();
        let Some(assets) = world.resources.remove::<AssetServer>() else {
            return;
        };
        for (id, cid) in world.query_ids::<Animator>() {
            let Some(mut a) = world.component::<Animator>(id, cid).cloned() else {
                continue;
            };
            let step = dt * a.speed;
            let pose = evaluate(world, &assets, id, &mut a, step);
            if let Some(c) = world.component_mut::<Animator>(id, cid) {
                c.rt = a.rt;
            }
            if let Some(p) = pose {
                apply(world, p);
            }
        }
        for (id, cid) in world.query_ids::<Animation>() {
            let Some(mut a) = world.component::<Animation>(id, cid).cloned() else {
                continue;
            };
            if a.playing.is_none() {
                a.playing = Some(a.play_automatically);
            }
            let mut pose = Pose::new();
            if a.playing == Some(true)
                && let Some(clip) = a.clip.as_ref().and_then(|h| assets.animations.get(h))
            {
                a.time += dt * a.speed;
                if !a.looping && a.time >= clip.length {
                    a.playing = Some(false);
                }
                sample_clip(
                    world,
                    id,
                    &mut a.bindings,
                    clip,
                    wrap(a.time, clip.length, a.looping),
                    1.0,
                    &mut pose,
                );
            }
            if let Some(c) = world.component_mut::<Animation>(id, cid) {
                c.time = a.time;
                c.playing = a.playing;
                c.bindings = a.bindings;
            }
            apply(world, pose);
        }
        world.resources.insert(assets);
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
    Box::new(AnimationSystem)
}

#[cfg(test)]
mod tests;
