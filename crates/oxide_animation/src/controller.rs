//! Animator Controller evaluation (Unity Mecanim): state machines with sub-state machines,
//! Entry/Exit nodes and Any State; transitions with exit times, fixed or normalized
//! durations, offsets, interruption sources, solo and mute; blend trees (1D, the three 2D
//! types and Direct, nested); state speed, motion time, mirror and cycle offset
//! parameters; layers with weights, Avatar Masks (transforms and humanoid body parts),
//! Override/Additive blending and Sync; Write Defaults; and the events that drive
//! StateMachineBehaviours.

use super::*;
use oxide_assets::{
    AnimatorControllerLayer, AnimatorState, AvatarMaskBodyPart, BlendTree, BlendTreeType,
    InterruptionSource, StatePath, Transition, TransitionSource, TransitionTarget,
};
use std::collections::HashSet;

fn param(params: &HashMap<String, f32>, name: &str) -> f32 {
    params.get(name).copied().unwrap_or(0.0)
}

/// One clip a motion plays, with blend trees resolved. `clip: None` is the rest pose (a
/// Direct tree whose weights sum to less than 1 blends toward it).
pub(crate) struct Leaf<'m> {
    pub clip: Option<&'m Handle<AnimationClip>>,
    pub weight: f32,
    pub time_scale: f32,
    pub cycle_offset: f32,
    pub mirror: bool,
}

/// Each child's weight in a blend tree for the current parameters (Unity's blend
/// computation; the Animator window shows these live).
pub fn tree_weights(t: &BlendTree, params: &HashMap<String, f32>) -> Vec<f32> {
    let n = t.children.len();
    let mut w = vec![0.0; n];
    if n == 0 {
        return w;
    }
    match t.blend_type {
        BlendTreeType::Simple1D => {
            let th = t.effective_thresholds();
            // Unity sorts children by threshold.
            let mut order: Vec<usize> = (0..n).collect();
            order.sort_by(|a, b| th[*a].total_cmp(&th[*b]));
            let sorted = order.iter().map(|&i| th[i]);
            for (k, x) in blend_weights_1d(sorted, param(params, &t.blend_parameter)) {
                w[order[k]] = x;
            }
        }
        BlendTreeType::Direct => {
            for (i, c) in t.children.iter().enumerate() {
                w[i] = param(params, &c.direct_blend_parameter).max(0.0);
            }
            let sum: f32 = w.iter().sum();
            if t.normalized_blend_values && sum > 1e-6 {
                w.iter_mut().for_each(|x| *x /= sum);
            }
        }
        _ => {
            let pts: Vec<Vector2> = t.children.iter().map(|c| c.position).collect();
            let p = Vector2::new(
                param(params, &t.blend_parameter),
                param(params, &t.blend_parameter_y),
            );
            w = blend_weights_2d(&pts, p, t.blend_type.as_2d());
        }
    }
    w
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn leaves<'m>(
    m: &'m Motion,
    params: &HashMap<String, f32>,
    w: f32,
    ts: f32,
    co: f32,
    mirror: bool,
    depth: usize,
    out: &mut Vec<Leaf<'m>>,
) {
    if w <= 1e-5 || depth > 16 {
        return;
    }
    match m {
        Motion::Clip(h) => out.push(Leaf {
            clip: Some(h),
            weight: w,
            time_scale: ts,
            cycle_offset: co,
            mirror,
        }),
        Motion::BlendTree(t) => {
            let cw = tree_weights(t, params);
            for (c, x) in t.children.iter().zip(&cw) {
                leaves(
                    &c.motion,
                    params,
                    w * x,
                    ts * c.time_scale,
                    co + c.cycle_offset,
                    mirror ^ c.mirror,
                    depth + 1,
                    out,
                );
            }
            let sum: f32 = cw.iter().sum();
            if t.blend_type == BlendTreeType::Direct && sum < 1.0 {
                out.push(Leaf {
                    clip: None,
                    weight: w * (1.0 - sum),
                    time_scale: ts,
                    cycle_offset: co,
                    mirror,
                });
            }
        }
        Motion::BlendTree1D {
            parameter,
            children,
        } => {
            for (i, x) in blend_weights_1d(children.iter().map(|c| c.0), param(params, parameter)) {
                out.push(Leaf {
                    clip: Some(&children[i].1),
                    weight: w * x,
                    time_scale: ts,
                    cycle_offset: co,
                    mirror,
                });
            }
        }
        Motion::BlendTree2D {
            parameter_x,
            parameter_y,
            kind,
            children,
        } => {
            let pts: Vec<Vector2> = children.iter().map(|c| c.0).collect();
            let p = Vector2::new(param(params, parameter_x), param(params, parameter_y));
            for (i, x) in blend_weights_2d(&pts, p, *kind).into_iter().enumerate() {
                if x > 1e-4 {
                    out.push(Leaf {
                        clip: Some(&children[i].1),
                        weight: w * x,
                        time_scale: ts,
                        cycle_offset: co,
                        mirror,
                    });
                }
            }
        }
        Motion::Empty => {}
    }
}

/// A motion's length in seconds: the weighted average of its clips' lengths (each divided
/// by its time scale), as Unity times blend trees.
pub fn motion_length(assets: &AssetServer, m: &Motion, params: &HashMap<String, f32>) -> f32 {
    let mut ls = vec![];
    leaves(m, params, 1.0, 1.0, 0.0, false, 0, &mut ls);
    let (mut sum, mut wsum) = (0.0, 0.0);
    for l in &ls {
        if let Some(c) = l.clip.and_then(|h| assets.animations.get(h)) {
            sum += l.weight * c.length / l.time_scale.abs().max(1e-4);
            wsum += l.weight;
        }
    }
    if wsum > 0.0 { sum / wsum } else { 0.0 }
}

pub(crate) fn state_speed(s: &AnimatorState, params: &HashMap<String, f32>) -> f32 {
    let mult = if s.speed_parameter_active {
        param(params, &s.speed_parameter)
    } else {
        1.0
    };
    s.speed * mult
}

fn state_mirror(s: &AnimatorState, params: &HashMap<String, f32>) -> bool {
    if s.mirror_parameter_active {
        param(params, &s.mirror_parameter) != 0.0
    } else {
        s.mirror
    }
}

fn state_cycle_offset(s: &AnimatorState, params: &HashMap<String, f32>) -> f32 {
    if s.cycle_offset_parameter_active {
        param(params, &s.cycle_offset_parameter)
    } else {
        s.cycle_offset
    }
}

/// The motion a layer plays for a state (a synced layer's own motion for it).
fn layer_motion<'a>(layer: &'a AnimatorControllerLayer, s: &'a AnimatorState) -> &'a Motion {
    if layer.synced_layer_index.is_some() {
        layer.synced_motion(&s.name).unwrap_or(&Motion::Empty)
    } else {
        &s.motion
    }
}

/// The layer whose state machine `li` runs (itself, or its sync source).
fn machine_layer(ctrl: &AnimatorController, li: usize) -> usize {
    match ctrl.layers.get(li).and_then(|l| l.synced_layer_index) {
        Some(src) if src < ctrl.layers.len() && src != li => src,
        _ => li,
    }
}

/// A state's length for timing: its own motion, mixed with synced layers that affect
/// timing (by their weights).
fn state_length(
    ctrl: &AnimatorController,
    li: usize,
    s: &AnimatorState,
    assets: &AssetServer,
    params: &HashMap<String, f32>,
    weights: &HashMap<usize, f32>,
) -> f32 {
    let mut len = motion_length(assets, &s.motion, params);
    for (i, l) in ctrl.layers.iter().enumerate() {
        if l.synced_layer_index == Some(li) && l.synced_layer_affects_timing {
            let w = weights.get(&i).copied().unwrap_or(l.default_weight);
            let other = motion_length(assets, layer_motion(l, s), params);
            if other > 0.0 {
                len = len + (other - len) * w.clamp(0.0, 1.0);
            }
        }
    }
    len
}

/// The transition being played.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ActiveTransition {
    /// Machine holding the transition (sub-machine path).
    pub machine: Vec<usize>,
    pub index: usize,
}

/// Events for StateMachineBehaviours, in order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StateEvent {
    Enter(usize),
    Update(usize),
    Exit(usize),
    MachineEnter(Vec<usize>),
    MachineExit(Vec<usize>),
    /// After root motion (`OnStateMove`).
    Move(usize),
    /// In the layer's IK pass (`OnStateIK`).
    Ik(usize),
}

fn exit_time_reached(t: &Transition, prev_n: f32, n: f32, looping: bool) -> bool {
    if !t.has_exit_time {
        return true;
    }
    let e = t.exit_time;
    if looping && e < 1.0 {
        // Checked once per loop, when the playhead passes `e` (Unity).
        (n - e).floor() > (prev_n - e).floor() || (prev_n <= 0.0 && n >= e && e <= 0.0)
    } else {
        n >= e
    }
}

/// Enters the machine at `path` through its Entry node: the first entry transition whose
/// conditions hold, else its default state.
fn enter_machine(
    root: &AnimatorStateMachine,
    path: &[usize],
    params: &HashMap<String, f32>,
    depth: usize,
) -> Option<StatePath> {
    let m = root.machine(path)?;
    if depth < 16 {
        for t in m.transitions.iter().filter(|t| t.is_entry() && !t.mute) {
            if t.conditions.iter().all(|c| c.met(params))
                && let Some(p) = resolve(root, path, t, params, depth + 1)
            {
                return Some(p);
            }
        }
    }
    let mut p = m.default_state_path()?;
    let mut full = path.to_vec();
    full.extend(p.machine);
    p.machine = full;
    Some(p)
}

/// Leaves the machine at `path` through its Exit node: the parent's transitions from this
/// machine whose conditions hold, else the parent's Entry.
fn exit_machine(
    root: &AnimatorStateMachine,
    path: &[usize],
    params: &HashMap<String, f32>,
    depth: usize,
) -> Option<StatePath> {
    let Some((_, parent)) = path.split_last() else {
        return enter_machine(root, &[], params, depth + 1);
    };
    let name = &root.machine(path)?.name;
    let pm = root.machine(parent)?;
    for t in pm.transitions.iter().filter(|t| {
        t.source == TransitionSource::StateMachine && t.from.as_deref() == Some(name) && !t.mute
    }) {
        if t.conditions.iter().all(|c| c.met(params))
            && let Some(p) = resolve(root, parent, t, params, depth + 1)
        {
            return Some(p);
        }
    }
    enter_machine(root, parent, params, depth + 1)
}

/// The state a transition (held by the machine at `holder`) ends in.
fn resolve(
    root: &AnimatorStateMachine,
    holder: &[usize],
    t: &Transition,
    params: &HashMap<String, f32>,
    depth: usize,
) -> Option<StatePath> {
    if depth > 16 {
        return None;
    }
    match t.target {
        TransitionTarget::State => root.find_state(&t.to),
        TransitionTarget::StateMachine => {
            let p = root.find_machine(&t.to)?;
            enter_machine(root, &p, params, depth + 1)
        }
        TransitionTarget::Exit => exit_machine(root, holder, params, depth + 1),
    }
}

/// Transitions out of a state, in priority order (solo and mute applied): Any State
/// transitions first, then the state's own (Unity's evaluation order).
fn candidates<'a>(
    root: &'a AnimatorStateMachine,
    at: &StatePath,
    name: &str,
    with_any: bool,
) -> Vec<(Vec<usize>, usize, &'a Transition)> {
    let filter = |list: Vec<(Vec<usize>, usize, &'a Transition)>| {
        let solo = list.iter().any(|(_, _, t)| t.solo);
        list.into_iter()
            .filter(|(_, _, t)| !t.mute && (!solo || t.solo))
            .collect::<Vec<_>>()
    };
    let mut out = vec![];
    if with_any {
        let mut any = vec![];
        let mut stack = vec![vec![]];
        while let Some(p) = stack.pop() {
            let Some(m) = root.machine(&p) else { continue };
            for (i, t) in m.transitions.iter().enumerate() {
                if t.is_any_state() {
                    any.push((p.clone(), i, t));
                }
            }
            for i in (0..m.state_machines.len()).rev() {
                let mut c = p.clone();
                c.push(i);
                stack.push(c);
            }
        }
        out.extend(filter(any));
    }
    if let Some(m) = root.machine(&at.machine) {
        let own = m
            .transitions
            .iter()
            .enumerate()
            .filter(|(_, t)| t.source == TransitionSource::State && t.from.as_deref() == Some(name))
            .map(|(i, t)| (at.machine.clone(), i, t))
            .collect();
        out.extend(filter(own));
    }
    out
}

/// Behaviour events for moving between machines (exits deepest first, then entries).
fn machine_events(from: &[usize], to: &[usize], ev: &mut Vec<StateEvent>) {
    let common = from.iter().zip(to).take_while(|(a, b)| a == b).count();
    for d in (common + 1..=from.len()).rev() {
        ev.push(StateEvent::MachineExit(from[..d].to_vec()));
    }
    for d in common + 1..=to.len() {
        ev.push(StateEvent::MachineEnter(to[..d].to_vec()));
    }
}

pub(crate) struct StepCtx<'a> {
    pub ctrl: &'a AnimatorController,
    pub assets: &'a AssetServer,
    pub weights: &'a HashMap<usize, f32>,
}

/// Advances one layer's state machine by `dt`: playback, crossfades, transitions and
/// interruptions. Synced layers are not stepped (they follow their source).
pub(crate) fn step_layer(
    cx: &StepCtx,
    li: usize,
    lrt: &mut LayerRuntime,
    params: &mut HashMap<String, f32>,
    dt: f32,
    ev: &mut Vec<StateEvent>,
) {
    let ctrl = cx.ctrl;
    let root = &ctrl.layers[li].state_machine;
    let flat = root.flatten();
    if flat.is_empty() {
        lrt.current = None;
        lrt.next = None;
        return;
    }
    let index_of = |p: &StatePath| flat.iter().position(|(q, _)| q == p);
    let len_of = |i: usize, params: &HashMap<String, f32>| {
        state_length(ctrl, li, flat[i].1, cx.assets, params, cx.weights).max(1e-4)
    };
    if lrt.current.as_ref().is_none_or(|c| c.state >= flat.len()) {
        let start = enter_machine(root, &[], params, 0)
            .and_then(|p| index_of(&p))
            .unwrap_or(0);
        machine_events(&[], &flat[start].0.machine, ev);
        ev.push(StateEvent::Enter(start));
        let co = state_cycle_offset(flat[start].1, params);
        lrt.current = Some(Playing::start_at(start, co * len_of(start, params)));
        lrt.next = None;
        lrt.transition = None;
        lrt.interrupted = None;
    }
    if lrt
        .next
        .as_ref()
        .is_some_and(|(n, _, _)| n.state >= flat.len())
    {
        lrt.next = None;
        lrt.transition = None;
    }
    // Advance the playing states.
    let advance = |p: &mut Playing, params: &HashMap<String, f32>| {
        let s = flat[p.state].1;
        p.prev_time = p.time;
        if s.time_parameter_active {
            p.time = param(params, &s.time_parameter) * len_of(p.state, params);
        } else {
            p.time += dt * state_speed(s, params);
        }
    };
    advance(lrt.current.as_mut().expect("current"), params);
    if let Some((n, elapsed, _)) = lrt.next.as_mut() {
        advance(n, params);
        *elapsed += dt;
    }
    if let Some((p, _)) = lrt.interrupted.as_mut() {
        advance(p, params);
    }
    let cur = lrt.current.clone().expect("current");
    if let Some((n, elapsed, dur)) = lrt.next.clone() {
        if elapsed >= dur {
            ev.push(StateEvent::Exit(cur.state));
            if let Some((p, _)) = lrt.interrupted.take() {
                ev.push(StateEvent::Exit(p.state));
            }
            machine_events(&flat[cur.state].0.machine, &flat[n.state].0.machine, ev);
            lrt.current = Some(n);
            lrt.next = None;
            lrt.transition = None;
        } else {
            ev.push(StateEvent::Update(cur.state));
            ev.push(StateEvent::Update(n.state));
            interrupt(cx, li, lrt, params, &flat, ev);
            return;
        }
    } else {
        ev.push(StateEvent::Update(cur.state));
    }

    // Not in a transition: the first transition whose exit time and conditions are met.
    let cur = lrt.current.clone().expect("current");
    let (at, cs) = (&flat[cur.state].0, flat[cur.state].1);
    let len = len_of(cur.state, params);
    let (prev_n, n) = (cur.prev_time / len, cur.time / len);
    for (holder, ti, t) in candidates(root, at, &cs.name, true) {
        if !exit_time_reached(t, prev_n, n, cs.looping)
            || !t.conditions.iter().all(|c| c.met(params))
        {
            continue;
        }
        let Some(dest) = resolve(root, &holder, t, params, 0).and_then(|p| index_of(&p)) else {
            continue;
        };
        if t.is_any_state() && dest == cur.state && !t.can_transition_to_self {
            continue;
        }
        consume_triggers(ctrl, t, params);
        start_transition(cx, li, lrt, &flat, holder, ti, t, dest, len, params, ev);
        return;
    }
}

fn consume_triggers(ctrl: &AnimatorController, t: &Transition, params: &mut HashMap<String, f32>) {
    for c in &t.conditions {
        if ctrl
            .parameter(&c.parameter)
            .is_some_and(|p| p.kind == ParameterType::Trigger)
        {
            params.insert(c.parameter.clone(), 0.0);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn start_transition(
    cx: &StepCtx,
    li: usize,
    lrt: &mut LayerRuntime,
    flat: &[(StatePath, &AnimatorState)],
    holder: Vec<usize>,
    ti: usize,
    t: &Transition,
    dest: usize,
    source_len: f32,
    params: &HashMap<String, f32>,
    ev: &mut Vec<StateEvent>,
) {
    let dest_len = state_length(cx.ctrl, li, flat[dest].1, cx.assets, params, cx.weights).max(1e-4);
    let co = state_cycle_offset(flat[dest].1, params);
    let start = Playing::start_at(dest, (t.offset + co) * dest_len);
    let dur = if t.fixed_duration {
        t.duration
    } else {
        t.duration * source_len
    };
    let cur = lrt.current.clone().expect("current");
    ev.push(StateEvent::Enter(dest));
    if dur <= 0.0 {
        ev.push(StateEvent::Exit(cur.state));
        machine_events(&flat[cur.state].0.machine, &flat[dest].0.machine, ev);
        lrt.current = Some(start);
        lrt.next = None;
        lrt.transition = None;
        lrt.interrupted = None;
    } else {
        lrt.next = Some((start, 0.0, dur));
        lrt.transition = Some(ActiveTransition {
            machine: holder,
            index: ti,
        });
    }
}

/// During a transition: transitions allowed by its interruption source may cut in. The
/// blend so far keeps playing as the source side of the new transition.
fn interrupt(
    cx: &StepCtx,
    li: usize,
    lrt: &mut LayerRuntime,
    params: &mut HashMap<String, f32>,
    flat: &[(StatePath, &AnimatorState)],
    ev: &mut Vec<StateEvent>,
) {
    let root = &cx.ctrl.layers[li].state_machine;
    let Some(active) = lrt.transition.clone() else {
        return;
    };
    let Some(at) = root
        .machine(&active.machine)
        .and_then(|m| m.transitions.get(active.index))
    else {
        return;
    };
    if at.interruption_source == InterruptionSource::None {
        return;
    }
    let cur = lrt.current.clone().expect("current");
    let (next, elapsed, dur) = lrt.next.clone().expect("next");
    let active_machine = active.machine.clone();
    let from_source = |only_before: Option<usize>| {
        let (p, s) = (&flat[cur.state].0, flat[cur.state].1);
        candidates(root, p, &s.name, false)
            .into_iter()
            .filter(move |(h, i, _)| only_before.is_none_or(|b| *h == active_machine && *i < b))
            .map(|c| (c, cur.state))
            .collect::<Vec<_>>()
    };
    let from_dest = || {
        let (p, s) = (&flat[next.state].0, flat[next.state].1);
        candidates(root, p, &s.name, false)
            .into_iter()
            .map(|c| (c, next.state))
            .collect::<Vec<_>>()
    };
    let ordered = at.ordered_interruption && !at.is_any_state();
    let src = || from_source(ordered.then_some(active.index));
    let list = match at.interruption_source {
        InterruptionSource::None => vec![],
        InterruptionSource::Source => src(),
        InterruptionSource::Destination => from_dest(),
        InterruptionSource::SourceThenDestination => {
            let mut v = src();
            v.extend(from_dest());
            v
        }
        InterruptionSource::DestinationThenSource => {
            let mut v = from_dest();
            v.extend(src());
            v
        }
    };
    for ((holder, ti, t), from_state) in list {
        if holder == active.machine && ti == active.index {
            continue;
        }
        let p = if from_state == cur.state { &cur } else { &next };
        let len = state_length(
            cx.ctrl,
            li,
            flat[from_state].1,
            cx.assets,
            params,
            cx.weights,
        )
        .max(1e-4);
        if !exit_time_reached(
            t,
            p.prev_time / len,
            p.time / len,
            flat[from_state].1.looping,
        ) || !t.conditions.iter().all(|c| c.met(params))
        {
            continue;
        }
        let Some(dest) = resolve(root, &holder, t, params, 0)
            .and_then(|p| flat.iter().position(|(q, _)| *q == p))
        else {
            continue;
        };
        if dest == next.state {
            continue;
        }
        consume_triggers(cx.ctrl, t, params);
        // The interrupted destination keeps its share of the source side.
        let w = (elapsed / dur.max(1e-4)).clamp(0.0, 1.0);
        if let Some((old, _)) = lrt.interrupted.take() {
            ev.push(StateEvent::Exit(old.state));
        }
        lrt.interrupted = Some((next.clone(), w));
        ev.push(StateEvent::Enter(dest));
        let dest_len =
            state_length(cx.ctrl, li, flat[dest].1, cx.assets, params, cx.weights).max(1e-4);
        let co = state_cycle_offset(flat[dest].1, params);
        let d = if t.fixed_duration {
            t.duration
        } else {
            t.duration * len
        };
        lrt.next = Some((
            Playing::start_at(dest, (t.offset + co) * dest_len),
            0.0,
            d.max(1e-4),
        ));
        lrt.transition = Some(ActiveTransition {
            machine: holder,
            index: ti,
        });
        return;
    }
}

pub(crate) fn state_info(
    cx: &StepCtx,
    li: usize,
    p: &Playing,
    params: &HashMap<String, f32>,
) -> AnimatorStateInfo {
    let ml = machine_layer(cx.ctrl, li);
    let root = &cx.ctrl.layers[ml].state_machine;
    let flat = root.flatten();
    let Some((path, s)) = flat.get(p.state) else {
        return AnimatorStateInfo::default();
    };
    let len = state_length(cx.ctrl, ml, s, cx.assets, params, cx.weights);
    let mut full = cx.ctrl.layers[li].name.clone();
    for d in 1..=path.machine.len() {
        if let Some(m) = root.machine(&path.machine[..d]) {
            full.push('.');
            full.push_str(&m.name);
        }
    }
    full.push('.');
    full.push_str(&s.name);
    AnimatorStateInfo {
        name: s.name.clone(),
        full_path: full,
        tag: s.tag.clone(),
        normalized_time: if len > 0.0 { p.time / len } else { 0.0 },
        length: len,
        speed: state_speed(s, params),
        looping: s.looping,
    }
}

/// The result of one Animator update.
#[derive(Default)]
pub(crate) struct Output {
    pub pose: HashMap<GameObjectId, Trs>,
    /// Root motion in the object's local space.
    pub root_position: Vector3,
    pub root_yaw: f32,
    pub events: Vec<(String, Value)>,
    pub props: HashMap<PropKey, f32>,
    pub objs: HashMap<PropKey, String>,
    /// Behaviour events per layer.
    pub state_events: Vec<(usize, StateEvent)>,
}

impl Sampler<'_> {
    /// Samples a motion at normalized times (blend tree children stay in sync).
    #[allow(clippy::too_many_arguments)]
    fn motion(
        &mut self,
        m: &Motion,
        prev_n: f32,
        now_n: f32,
        looping: bool,
        w: f32,
        mirror: bool,
        params: &HashMap<String, f32>,
    ) {
        let mut ls = vec![];
        leaves(m, params, w, 1.0, 0.0, mirror, 0, &mut ls);
        let dominant = ls
            .iter()
            .enumerate()
            .filter(|(_, l)| l.clip.is_some())
            .max_by(|a, b| a.1.weight.total_cmp(&b.1.weight))
            .map(|(i, _)| i);
        for (i, l) in ls.iter().enumerate() {
            match l.clip {
                Some(h) => {
                    if let Some(c) = self.assets.animations.get(h) {
                        let len = c.length;
                        let t = |n: f32| (n * l.time_scale + l.cycle_offset) * len;
                        self.clip_mirrored(
                            c,
                            t(prev_n),
                            t(now_n),
                            looping,
                            l.weight,
                            Some(i) == dominant,
                            l.mirror,
                        );
                    }
                }
                None => self.rest_pose(l.weight),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sample_layer(
    s: &mut Sampler,
    cx: &StepCtx,
    li: usize,
    lrt: &LayerRuntime,
    params: &HashMap<String, f32>,
) {
    let layer = &cx.ctrl.layers[li];
    let ml = machine_layer(cx.ctrl, li);
    let flat = cx.ctrl.layers[ml].state_machine.flatten();
    let one = |s: &mut Sampler, p: &Playing, w: f32| {
        let Some((_, st)) = flat.get(p.state) else {
            return;
        };
        let len = state_length(cx.ctrl, ml, st, cx.assets, params, cx.weights).max(1e-4);
        s.motion(
            layer_motion(layer, st),
            p.prev_time / len,
            p.time / len,
            st.looping,
            w,
            state_mirror(st, params),
            params,
        );
    };
    let cur = lrt.current.clone().unwrap_or_default();
    let (src_w, next) = match &lrt.next {
        Some((n, elapsed, dur)) => {
            let w = (elapsed / dur.max(1e-4)).clamp(0.0, 1.0);
            (1.0 - w, Some((n, w)))
        }
        None => (1.0, None),
    };
    match &lrt.interrupted {
        Some((p, iw)) => {
            one(s, &cur, src_w * (1.0 - iw));
            one(s, p, src_w * iw);
        }
        None => one(s, &cur, src_w),
    }
    if let Some((n, w)) = next {
        one(s, n, w);
    }
}

/// Bones of the humanoid body parts a mask leaves out.
fn excluded_bones(
    world: &World,
    human: &HashMap<HumanBodyBones, GameObjectId>,
    mask: &AvatarMask,
) -> HashSet<GameObjectId> {
    use AvatarMaskBodyPart as P;
    use HumanBodyBones as B;
    let mut out = HashSet::new();
    let add_tree = |out: &mut HashSet<GameObjectId>, id: GameObjectId, with_self: bool| {
        let mut stack = if with_self {
            vec![id]
        } else {
            world.children(id).to_vec()
        };
        while let Some(i) = stack.pop() {
            out.insert(i);
            stack.extend_from_slice(world.children(i));
        }
    };
    for part in &mask.disabled_body_parts {
        let bones: &[B] = match part {
            P::Body => &[B::Hips, B::Spine, B::Chest, B::UpperChest],
            P::Head => &[B::Neck],
            P::LeftLeg => &[B::LeftUpperLeg, B::LeftLowerLeg, B::LeftFoot],
            P::RightLeg => &[B::RightUpperLeg, B::RightLowerLeg, B::RightFoot],
            P::LeftArm => &[
                B::LeftShoulder,
                B::LeftUpperArm,
                B::LeftLowerArm,
                B::LeftHand,
            ],
            P::RightArm => &[
                B::RightShoulder,
                B::RightUpperArm,
                B::RightLowerArm,
                B::RightHand,
            ],
            _ => &[],
        };
        for b in bones {
            if let Some(id) = human.get(b) {
                out.insert(*id);
            }
        }
        match part {
            P::Head => {
                if let Some(h) = human.get(&B::Head) {
                    add_tree(&mut out, *h, true);
                }
            }
            P::LeftLeg | P::RightLeg => {
                let toes = if *part == P::LeftLeg {
                    B::LeftToes
                } else {
                    B::RightToes
                };
                let foot = if *part == P::LeftLeg {
                    B::LeftFoot
                } else {
                    B::RightFoot
                };
                if let Some(t) = human.get(&toes).or(human.get(&foot)) {
                    add_tree(&mut out, *t, true);
                }
            }
            P::LeftFingers | P::RightFingers => {
                let hand = if *part == P::LeftFingers {
                    B::LeftHand
                } else {
                    B::RightHand
                };
                if let Some(h) = human.get(&hand) {
                    add_tree(&mut out, *h, false);
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
pub(crate) fn excluded_bones_for_test(
    world: &World,
    human: &HashMap<HumanBodyBones, GameObjectId>,
    mask: &AvatarMask,
) -> HashSet<GameObjectId> {
    excluded_bones(world, human, mask)
}

/// Advances one Animator and computes its pose.
pub(crate) fn evaluate(
    world: &World,
    assets: &AssetServer,
    root: GameObjectId,
    a: &mut Animator,
    dt: f32,
) -> Option<Output> {
    let ctrl = assets.controllers.get(a.controller.as_ref()?)?;
    let rt = &mut a.rt;
    let cid = a.controller.as_ref().map(|h| h.id());
    if rt.controller != cid || rt.layers.len() != ctrl.layers.len() {
        rt.controller = cid;
        rt.layers = vec![LayerRuntime::default(); ctrl.layers.len()];
        rt.layer_names = ctrl.layers.iter().map(|l| l.name.clone()).collect();
        rt.bindings.clear();
        rt.human_mapped = false;
        for p in &ctrl.parameters {
            rt.params.entry(p.name.clone()).or_insert(p.default);
        }
    }
    rt.default_weights = ctrl.layers.iter().map(|l| l.default_weight).collect();
    if !rt.human_mapped {
        rt.human = auto_map(world, root);
        rt.human_mapped = true;
    }
    let needs_mirror = ctrl.layers.iter().any(|l| {
        l.state_machine
            .flatten()
            .iter()
            .any(|(_, s)| s.mirror || s.mirror_parameter_active || motion_mirrors(&s.motion))
    });
    if needs_mirror && rt.mirror.is_none() {
        rt.mirror = Some(std::sync::Arc::new(crate::mirror::MirrorMap::build(
            world, root,
        )));
    }
    let weights: HashMap<usize, f32> = (0..ctrl.layers.len())
        .map(|i| {
            let w = if i == 0 {
                1.0
            } else {
                rt.layer_weights
                    .get(&i)
                    .copied()
                    .unwrap_or(ctrl.layers[i].default_weight)
            };
            (i, w)
        })
        .collect();
    let cx = StepCtx {
        ctrl,
        assets,
        weights: &weights,
    };
    let mut out = Output::default();
    for pending in std::mem::take(&mut rt.pending) {
        let li = pending.layer.or_else(|| {
            ctrl.layers
                .iter()
                .position(|l| l.state_machine.find_state(&pending.state).is_some())
        });
        let Some(li) = li.filter(|&l| l < ctrl.layers.len()) else {
            continue;
        };
        let root_sm = &ctrl.layers[li].state_machine;
        let flat = root_sm.flatten();
        let Some(i) = root_sm
            .find_state(&pending.state)
            .and_then(|p| flat.iter().position(|(q, _)| *q == p))
        else {
            continue;
        };
        let len = state_length(ctrl, li, flat[i].1, assets, &rt.params, &weights).max(1e-4);
        let start = Playing::start_at(i, pending.normalized_time.unwrap_or(0.0) * len);
        let lrt = &mut rt.layers[li];
        let fade = if pending.fixed {
            pending.fade
        } else {
            let cur_len = lrt
                .current
                .as_ref()
                .and_then(|c| flat.get(c.state))
                .map_or(1.0, |(_, s)| {
                    state_length(ctrl, li, s, assets, &rt.params, &weights)
                });
            pending.fade * cur_len
        };
        out.state_events.push((li, StateEvent::Enter(i)));
        if fade <= 0.0 || lrt.current.is_none() {
            if let Some(c) = &lrt.current {
                out.state_events.push((li, StateEvent::Exit(c.state)));
            }
            lrt.current = Some(start);
            lrt.next = None;
        } else {
            lrt.next = Some((start, 0.0, fade));
        }
        lrt.transition = None;
        lrt.interrupted = None;
    }
    // Defaults of everything animated so far (Write Defaults restores them).
    let write_defaults = |rt: &AnimatorRuntime| {
        let flat = ctrl.layers[0].state_machine.flatten();
        rt.layers
            .first()
            .and_then(|l| l.current.as_ref())
            .and_then(|c| flat.get(c.state))
            .is_none_or(|(_, s)| s.write_defaults)
    };
    for li in 0..ctrl.layers.len() {
        let layer = &ctrl.layers[li];
        let ml = machine_layer(ctrl, li);
        if ml == li {
            let mut ev = vec![];
            step_layer(&cx, li, &mut rt.layers[li], &mut rt.params, dt, &mut ev);
            out.state_events.extend(ev.into_iter().map(|e| (li, e)));
        } else {
            // Synced: the source layer's states and timing.
            let src = rt.layers[ml].clone();
            let l = &mut rt.layers[li];
            l.current = src.current;
            l.next = src.next;
            l.interrupted = src.interrupted;
            l.transition = src.transition;
        }
        let lrt = &mut rt.layers[li];
        if let Some(c) = lrt.current.clone() {
            lrt.info = state_info(&cx, li, &c, &rt.params);
        }
        lrt.next_info = lrt
            .next
            .clone()
            .map(|(n, _, _)| state_info(&cx, li, &n, &rt.params));
        let weight = weights[&li];
        if weight <= 0.0 || lrt.current.is_none() {
            continue;
        }
        // The mask: the `.mask` asset as it is now (edits apply live), else the inline one.
        let mask = layer
            .avatar_mask_asset
            .as_ref()
            .and_then(|h| assets.masks.get(h))
            .or(layer.avatar_mask.as_ref());
        let excluded = mask
            .filter(|m| !m.disabled_body_parts.is_empty())
            .map(|m| excluded_bones(world, &rt.human, m));
        let root_part = mask.is_none_or(|m| m.body_part_active(AvatarMaskBodyPart::Root));
        let lrt = rt.layers[li].clone();
        let mirror = rt.mirror.clone();
        let mut s = Sampler::new(world, assets, root, &mut rt.bindings);
        s.mask = mask;
        s.excluded = excluded.as_ref();
        s.mirror_map = mirror.as_deref();
        s.rest = Some(&rt.defaults);
        s.root_motion = li == 0 && a.apply_root_motion && root_part;
        sample_layer(&mut s, &cx, li, &lrt, &rt.params);
        let (pose, rm, events) = (finalize(s.pose), s.rm, s.events);
        let props = finalize_props(s.props);
        // Later layers with any weight replace object keys (they can't blend).
        out.objs
            .extend(s.objs.into_iter().map(|(k, (v, _))| (k, v)));
        out.events.extend(events);
        if li == 0 {
            out.pose = pose;
            out.props = props;
            out.root_position = rm.position;
            out.root_yaw = rm.yaw;
            continue;
        }
        // Properties blend like positions: over the layers below by the layer weight.
        let additive = layer.blending_mode == AnimatorLayerBlendingMode::Additive;
        for ((pid, comp, prop), v) in props {
            let base = out
                .props
                .get(&(pid, comp.clone(), prop.clone()))
                .copied()
                .or_else(|| read_property(world, pid, &comp, &prop))
                .unwrap_or(0.0);
            let nv = if additive {
                base + v * weight
            } else {
                base + (v - base) * weight
            };
            out.props.insert((pid, comp, prop), nv);
        }
        let base_of = |out: &Output, id: GameObjectId| -> (Vector3, Quaternion, Vector3) {
            let t = world.transform(id);
            let cur = out.pose.get(&id);
            (
                cur.and_then(|c| c.0)
                    .or(t.map(|t| t.local_position))
                    .unwrap_or_default(),
                cur.and_then(|c| c.1)
                    .or(t.map(|t| t.local_rotation))
                    .unwrap_or_default(),
                cur.and_then(|c| c.2)
                    .or(t.map(|t| t.local_scale))
                    .unwrap_or(Vector3::ONE),
            )
        };
        match layer.blending_mode {
            AnimatorLayerBlendingMode::Override => {
                for (id, (p, r, sc)) in pose {
                    let (bp, br, bs) = base_of(&out, id);
                    let e = out.pose.entry(id).or_default();
                    if let Some(p) = p {
                        e.0 = Some(bp.lerp(p, weight));
                    }
                    if let Some(r) = r {
                        e.1 = Some(br.slerp(r, weight));
                    }
                    if let Some(sc) = sc {
                        e.2 = Some(bs.lerp(sc, weight));
                    }
                }
            }
            AnimatorLayerBlendingMode::Additive => {
                let mirror = rt.mirror.clone();
                let mut rs = Sampler::new(world, assets, root, &mut rt.bindings);
                rs.mask = mask;
                rs.excluded = excluded.as_ref();
                rs.mirror_map = mirror.as_deref();
                rs.time_override = Some(0.0);
                rs.collect_events = false;
                sample_layer(&mut rs, &cx, li, &lrt, &rt.params);
                let reference = finalize(rs.pose);
                for (id, (p, r, sc)) in pose {
                    let (rp, rr, rsc) = reference.get(&id).copied().unwrap_or_default();
                    let (bp, br, bs) = base_of(&out, id);
                    let e = out.pose.entry(id).or_default();
                    if let (Some(p), Some(rp)) = (p, rp) {
                        e.0 = Some(bp + (p - rp) * weight);
                    }
                    if let (Some(r), Some(rr)) = (r, rr) {
                        let d = Quaternion::IDENTITY.slerp(rr.inverse() * r, weight);
                        e.1 = Some((br * d).normalize());
                    }
                    if let (Some(sc), Some(rsc)) = (sc, rsc) {
                        e.2 = Some(bs + (sc - rsc) * weight);
                    }
                }
            }
        }
    }
    // Remember the scene's values the first time something is animated, and with Write
    // Defaults on, put back whatever the current states leave unanimated.
    for id in out.pose.keys() {
        if !rt.defaults.contains_key(id)
            && let Some(t) = world.transform(*id)
        {
            rt.defaults
                .insert(*id, (t.local_position, t.local_rotation, t.local_scale));
        }
    }
    for k in out.props.keys() {
        if !rt.prop_defaults.contains_key(k)
            && let Some(v) = read_property(world, k.0, &k.1, &k.2)
        {
            rt.prop_defaults.insert(k.clone(), v);
        }
    }
    if write_defaults(rt) {
        for (id, (p, r, s)) in &rt.defaults {
            let e = out.pose.entry(*id).or_insert((None, None, None));
            e.0.get_or_insert(*p);
            e.1.get_or_insert(*r);
            e.2.get_or_insert(*s);
        }
        for (k, v) in &rt.prop_defaults {
            out.props.entry(k.clone()).or_insert(*v);
        }
    }
    Some(out)
}

fn motion_mirrors(m: &Motion) -> bool {
    match m {
        Motion::BlendTree(t) => t
            .children
            .iter()
            .any(|c| c.mirror || motion_mirrors(&c.motion)),
        _ => false,
    }
}
