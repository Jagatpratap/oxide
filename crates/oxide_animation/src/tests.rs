use super::*;
use oxide_assets::{
    BlendTree2DType, Condition, ConditionMode, Interpolation, Track, TransformCurves,
};
use oxide_math::Vector2;

fn clip(name: &str, y: f32) -> AnimationClip {
    let mut c = AnimationClip {
        name: name.into(),
        looping: true,
        curves: vec![TransformCurves {
            path: "Bone".into(),
            position: Some(Track {
                times: vec![0.0, 1.0],
                values: vec![Vector3::ZERO, Vector3::Y * y],
                interpolation: Interpolation::Linear,
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    c.recalculate_length();
    c
}

fn setup() -> (World, Box<dyn System>, GameObjectId, GameObjectId) {
    let mut w = World::new();
    w.resources.insert(Time::new());
    let mut assets = AssetServer::new(".");
    let idle = assets.animations.insert("idle", clip("Idle", 1.0));
    let run = assets.animations.insert("run", clip("Run", 10.0));
    let mut ctrl = AnimatorController::default();
    ctrl.add_parameter("Speed", ParameterType::Float)
        .add_parameter("Jump", ParameterType::Trigger);
    ctrl.add_state(
        "Locomotion",
        Motion::BlendTree1D {
            parameter: "Speed".into(),
            children: vec![(0.0, idle.clone()), (1.0, run)],
        },
    );
    ctrl.add_state("Jump", Motion::Clip(idle));
    ctrl.add_transition(
        Some("Locomotion"),
        "Jump",
        vec![Condition {
            parameter: "Jump".into(),
            mode: ConditionMode::If,
        }],
        0.0,
    );
    let h = assets.controllers.insert("ctrl", ctrl);
    w.resources.insert(assets);
    let sys = install(&mut w);
    let root = w.create("Character");
    let bone = w.create("Bone");
    w.set_parent(bone, Some(root), false).unwrap();
    w.add_component(root, Animator::new(h));
    (w, sys, root, bone)
}

fn tick(w: &mut World, sys: &mut Box<dyn System>, dt: f64) {
    w.resource_mut::<Time>().begin_frame(dt);
    sys.update(w);
}

#[test]
fn blend_tree_and_trigger_transition() {
    let (mut w, mut sys, root, bone) = setup();
    tick(&mut w, &mut sys, 0.25);
    tick(&mut w, &mut sys, 0.25);
    // Speed 0 → idle only: y = 0.5 at t=0.5.
    assert!((w.transform(bone).unwrap().local_position.y - 0.5).abs() < 1e-3);
    w.get_component_mut::<Animator>(root)
        .unwrap()
        .set_float("Speed", 0.5);
    tick(&mut w, &mut sys, 0.25);
    // Half idle (y=0.75) half run (y=7.5) at t=0.75.
    let y = w.transform(bone).unwrap().local_position.y;
    assert!((y - (0.375 + 3.75)).abs() < 1e-2, "{y}");
    w.get_component_mut::<Animator>(root)
        .unwrap()
        .set_trigger("Jump");
    tick(&mut w, &mut sys, 0.1);
    let a = w.get_component::<Animator>(root).unwrap();
    assert_eq!(
        a.rt.layers[0].current.as_ref().unwrap().state,
        1,
        "moved to Jump"
    );
    assert!(a.get_current_animator_state_info(0).is_name("Jump"));
    assert!(!a.get_bool("Jump"), "trigger consumed");
}

fn track(path: &str, from: Vector3, to: Vector3, len: f32) -> TransformCurves {
    TransformCurves {
        path: path.into(),
        position: Some(Track {
            times: vec![0.0, len],
            values: vec![from, to],
            interpolation: Interpolation::Linear,
        }),
        ..Default::default()
    }
}

fn clip_of(curves: Vec<TransformCurves>) -> AnimationClip {
    let mut c = AnimationClip {
        name: "c".into(),
        looping: true,
        curves,
        ..Default::default()
    };
    c.recalculate_length();
    c
}

/// A world with Character → Bone → Arm and an Animator using `ctrl`.
fn rig(
    ctrl: impl FnOnce(&mut AssetServer) -> AnimatorController,
) -> (World, Box<dyn System>, [GameObjectId; 3]) {
    let mut w = World::new();
    w.resources.insert(Time::new());
    let mut assets = AssetServer::new(".");
    let c = ctrl(&mut assets);
    let h = assets.controllers.insert("ctrl", c);
    w.resources.insert(assets);
    let sys = install(&mut w);
    let root = w.create("Character");
    let bone = w.create("Bone");
    let arm = w.create("Arm");
    w.set_parent(bone, Some(root), false).unwrap();
    w.set_parent(arm, Some(bone), false).unwrap();
    w.add_component(root, Animator::new(h));
    (w, sys, [root, bone, arm])
}

#[test]
fn blend_2d_weights() {
    let pts = [
        Vector2::ZERO,
        Vector2::new(0.0, 1.0),
        Vector2::new(1.0, 0.0),
        Vector2::new(-1.0, 0.0),
    ];
    for kind in [
        BlendTree2DType::SimpleDirectional,
        BlendTree2DType::FreeformCartesian,
    ] {
        let w = blend_weights_2d(&pts, Vector2::new(0.0, 1.0), kind);
        assert!((w[1] - 1.0).abs() < 1e-4, "{kind:?} {w:?}");
        let w = blend_weights_2d(&pts, Vector2::new(0.0, 0.5), kind);
        assert!(
            (w[0] - 0.5).abs() < 1e-3 && (w[1] - 0.5).abs() < 1e-3,
            "{kind:?} {w:?}"
        );
        let w = blend_weights_2d(&pts, Vector2::new(0.5, 0.5), kind);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        assert!((w[1] - w[2]).abs() < 1e-3 && w[3] < 1e-4, "{kind:?} {w:?}");
    }
}

#[test]
fn layers_override_with_mask_and_additive() {
    let (mut w, mut sys, [root, bone, arm]) = rig(|assets| {
        let base = assets.animations.insert(
            "base",
            clip_of(vec![
                track("Bone", Vector3::Y, Vector3::Y, 1.0),
                track("Bone/Arm", Vector3::X, Vector3::X, 1.0),
            ]),
        );
        let upper = assets.animations.insert(
            "upper",
            clip_of(vec![
                track("Bone", Vector3::Y * 100.0, Vector3::Y * 100.0, 1.0),
                track("Bone/Arm", Vector3::X * 11.0, Vector3::X * 11.0, 1.0),
            ]),
        );
        let add = assets.animations.insert(
            "add",
            clip_of(vec![track("Bone", Vector3::ZERO, Vector3::Z * 4.0, 1.0)]),
        );
        let mut c = AnimatorController::default();
        c.add_state("Base", Motion::Clip(base));
        let l = c.add_layer("Upper");
        l.avatar_mask = Some(oxide_assets::AvatarMask {
            name: "Arm".into(),
            transform_paths: vec!["Bone/Arm".into()],
            ..Default::default()
        });
        l.default_weight = 0.5;
        l.state_machine.add_state("Aim", Motion::Clip(upper));
        let l = c.add_layer("Breathe");
        l.blending_mode = oxide_assets::AnimatorLayerBlendingMode::Additive;
        l.state_machine.add_state("Breathe", Motion::Clip(add));
        c
    });
    tick(&mut w, &mut sys, 0.25);
    let (b, a) = (
        w.transform(bone).unwrap().local_position,
        w.transform(arm).unwrap().local_position,
    );
    assert!((a.x - 6.0).abs() < 1e-3, "override at 0.5: {a}");
    assert!((b.y - 1.0).abs() < 1e-3, "Bone is masked out of Upper: {b}");
    assert!((b.z - 1.0).abs() < 1e-3, "additive adds +1 at t=0.25: {b}");
    let an = w.get_component_mut::<Animator>(root).unwrap();
    assert_eq!(an.get_layer_index("Upper"), Some(1));
    an.set_layer_weight(1, 0.0);
    tick(&mut w, &mut sys, 0.25);
    assert!((w.transform(arm).unwrap().local_position.x - 1.0).abs() < 1e-3);
}

#[test]
fn root_motion_moves_the_object_across_loops() {
    let (mut w, mut sys, [root, bone, _]) = rig(|assets| {
        let walk = assets.animations.insert(
            "walk",
            clip_of(vec![track(
                "Bone",
                Vector3::new(0.0, 1.0, 0.0),
                Vector3::new(0.0, 1.5, 2.0),
                1.0,
            )]),
        );
        let mut c = AnimatorController::default();
        c.add_state("Walk", Motion::Clip(walk));
        c
    });
    w.get_component_mut::<Animator>(root)
        .unwrap()
        .apply_root_motion = true;
    w.transform_mut(root).unwrap().local_rotation = Quaternion::from_rotation_y(90f32.to_radians());
    for _ in 0..6 {
        tick(&mut w, &mut sys, 0.25);
    }
    // 1.5 s of a 2 m/s walk, facing +X.
    let p = w.position(root);
    assert!((p - Vector3::new(3.0, 0.0, 0.0)).length() < 1e-2, "{p}");
    let b = w.transform(bone).unwrap().local_position;
    assert!(b.z.abs() < 1e-3, "bone held in place: {b}");
    assert!(
        (b.y - 1.25).abs() < 1e-3,
        "vertical motion stays in the pose: {b}"
    );
    let v = w.get_component::<Animator>(root).unwrap().velocity();
    assert!((v.length() - 2.0).abs() < 1e-2, "{v}");
}

#[derive(Clone, Debug, Default, oxide_core::Reflect)]
struct Steps {
    count: i64,
}

impl Component for Steps {
    fn on_message(&mut self, _: &mut oxide_scene::Ctx, message: &str, arg: &Value) -> bool {
        if message == "Footstep"
            && let Value::Int(n) = arg
        {
            self.count += n;
            return true;
        }
        false
    }
}

#[test]
fn animation_events_fire_once_per_pass() {
    let (mut w, mut sys, [root, _, _]) = rig(|assets| {
        let mut c = clip_of(vec![track("Bone", Vector3::ZERO, Vector3::X, 1.0)]);
        c.add_event(0.5, "Footstep", Value::Int(1));
        c.add_event(0.0, "Footstep", Value::Int(100));
        let walk = assets.animations.insert("walk", c);
        let mut ctrl = AnimatorController::default();
        ctrl.add_state("Walk", Motion::Clip(walk));
        ctrl
    });
    w.add_component(root, Steps::default());
    w.start_scripts();
    let count = |w: &World| w.get_component::<Steps>(root).unwrap().count;
    tick(&mut w, &mut sys, 0.25); // 0 → 0.25: the event at 0
    assert_eq!(count(&w), 100);
    tick(&mut w, &mut sys, 0.25); // → 0.5
    assert_eq!(count(&w), 101);
    tick(&mut w, &mut sys, 0.25); // → 0.75
    tick(&mut w, &mut sys, 0.3); // → 1.05: wraps past 0 again
    assert_eq!(count(&w), 201);
}

/// Clips animate any component property (Unity's generic curves): a child's Light
/// intensity follows the curve, and an override layer at half weight blends halfway.
#[test]
fn property_curves_drive_component_fields() {
    let (mut w, mut sys, root, bone) = setup();
    w.registry.register::<oxide_scene::Light>("Rendering/Light");
    w.add_component(
        bone,
        oxide_scene::Light {
            intensity: 1.0,
            ..Default::default()
        },
    );
    let mut flicker = AnimationClip {
        name: "Flicker".into(),
        looping: true,
        properties: vec![
            oxide_assets::PropertyCurve {
                path: "Bone".into(),
                component: "Light".into(),
                property: "intensity".into(),
                curve: oxide_core::AnimationCurve::linear(0.0, 0.0, 2.0, 8.0),
            },
            oxide_assets::PropertyCurve {
                path: "Bone".into(),
                component: "Light".into(),
                property: "color.g".into(),
                curve: oxide_core::AnimationCurve::constant(0.0, 2.0, 0.25),
            },
        ],
        ..Default::default()
    };
    flicker.recalculate_length();
    let h = w
        .resource_mut::<AssetServer>()
        .animations
        .insert("flicker", flicker);
    // Legacy Animation component, 1 s in: intensity 4, green channel 0.25.
    w.add_component(
        root,
        Animation {
            clip: Some(h.clone()),
            ..Default::default()
        },
    );
    for _ in 0..4 {
        tick(&mut w, &mut sys, 0.25);
    }
    let l = w.get_component::<oxide_scene::Light>(bone).unwrap();
    assert!(
        (l.intensity - 4.0).abs() < 1e-3,
        "intensity {}",
        l.intensity
    );
    assert!((l.color.g - 0.25).abs() < 1e-5, "green {}", l.color.g);
    assert_eq!(read_property(&w, bone, "Light", "color.g"), Some(0.25));
}

/// Sprite animation: object keys swap a SpriteRenderer's sprite (stepped, no blending).
#[test]
fn object_curves_flip_sprites() {
    let (mut w, mut sys, root, _) = setup();
    w.registry
        .register::<oxide_scene::SpriteRenderer>("Rendering/Sprite Renderer");
    w.add_component(root, oxide_scene::SpriteRenderer::default());
    let mut curve = oxide_assets::ObjectCurve {
        path: String::new(),
        component: "SpriteRenderer".into(),
        property: "sprite".into(),
        keys: vec![],
    };
    for i in 0..3 {
        curve.set_key(i as f32 * 0.25, &format!("builtin:frame{i}"));
    }
    let mut clip = AnimationClip {
        name: "Run".into(),
        looping: true,
        object_curves: vec![curve],
        ..Default::default()
    };
    clip.recalculate_length();
    assert!(
        (clip.length - (0.5 + 1.0 / 12.0)).abs() < 1e-5,
        "last frame lasts one frame"
    );
    let h = w
        .resource_mut::<AssetServer>()
        .animations
        .insert("run", clip);
    w.add_component(
        root,
        Animation {
            clip: Some(h),
            ..Default::default()
        },
    );
    let sprite = |w: &World| {
        w.get_component::<oxide_scene::SpriteRenderer>(root)
            .unwrap()
            .sprite
            .as_ref()
            .map(|s| s.path().to_owned())
    };
    tick(&mut w, &mut sys, 0.1);
    assert_eq!(sprite(&w).as_deref(), Some("builtin:frame0"));
    tick(&mut w, &mut sys, 0.2);
    assert_eq!(sprite(&w).as_deref(), Some("builtin:frame1"));
    tick(&mut w, &mut sys, 0.25);
    assert_eq!(sprite(&w).as_deref(), Some("builtin:frame2"));
}

/// A constant pose: Bone at `y` (one-second looping clip).
fn hold(assets: &mut AssetServer, name: &str, y: f32) -> Handle<AnimationClip> {
    assets.animations.insert(
        name,
        clip_of(vec![track("Bone", Vector3::Y * y, Vector3::Y * y, 1.0)]),
    )
}

fn cond(p: &str, mode: ConditionMode) -> Condition {
    Condition {
        parameter: p.into(),
        mode,
    }
}

fn bone_y(w: &World, bone: GameObjectId) -> f32 {
    w.transform(bone).unwrap().local_position.y
}

fn state_name(w: &World, root: GameObjectId) -> String {
    w.get_component::<Animator>(root)
        .unwrap()
        .get_current_animator_state_info(0)
        .name
}

#[test]
fn sub_state_machines_entry_and_exit() {
    use oxide_assets::{TransitionSource, TransitionTarget};
    let (mut w, mut sys, [root, bone, _]) = rig(|a| {
        let (idle, walk, run, land) = (
            hold(a, "i", 1.0),
            hold(a, "w", 2.0),
            hold(a, "r", 3.0),
            hold(a, "l", 4.0),
        );
        let mut c = AnimatorController::default();
        c.add_parameter("Move", ParameterType::Bool)
            .add_parameter("Fast", ParameterType::Bool)
            .add_parameter("Stop", ParameterType::Trigger);
        c.add_state("Idle", Motion::Clip(idle));
        c.add_state("Land", Motion::Clip(land));
        let loco = c.base().add_state_machine("Locomotion");
        loco.add_state("Walk", Motion::Clip(walk));
        loco.add_state("Run", Motion::Clip(run));
        // Entry picks Run when Fast is set, else the default (Walk).
        loco.transitions.push(oxide_assets::Transition {
            source: TransitionSource::Entry,
            to: "Run".into(),
            conditions: vec![cond("Fast", ConditionMode::If)],
            ..Default::default()
        });
        // Walk → Exit on Stop.
        loco.transitions.push(oxide_assets::Transition {
            from: Some("Walk".into()),
            target: TransitionTarget::Exit,
            conditions: vec![cond("Stop", ConditionMode::If)],
            duration: 0.0,
            ..Default::default()
        });
        let sm = c.base();
        sm.transitions.push(oxide_assets::Transition {
            from: Some("Idle".into()),
            to: "Locomotion".into(),
            target: TransitionTarget::StateMachine,
            conditions: vec![cond("Move", ConditionMode::If)],
            duration: 0.0,
            ..Default::default()
        });
        // Leaving Locomotion goes to Land (state machine transition).
        sm.transitions.push(oxide_assets::Transition {
            from: Some("Locomotion".into()),
            source: TransitionSource::StateMachine,
            to: "Land".into(),
            ..Default::default()
        });
        c
    });
    tick(&mut w, &mut sys, 0.1);
    assert_eq!(state_name(&w, root), "Idle");
    w.get_component_mut::<Animator>(root)
        .unwrap()
        .set_bool("Move", true);
    tick(&mut w, &mut sys, 0.1);
    assert_eq!(state_name(&w, root), "Walk");
    let info = w
        .get_component::<Animator>(root)
        .unwrap()
        .get_current_animator_state_info(0);
    assert!(
        info.is_name("Base Layer.Locomotion.Walk"),
        "{}",
        info.full_path
    );
    tick(&mut w, &mut sys, 0.1);
    assert!((bone_y(&w, bone) - 2.0).abs() < 1e-3);
    {
        let a = w.get_component_mut::<Animator>(root).unwrap();
        a.set_bool("Move", false);
        a.set_trigger("Stop");
    }
    tick(&mut w, &mut sys, 0.1);
    assert_eq!(
        state_name(&w, root),
        "Land",
        "exit → state machine transition"
    );
    // Entering again with Fast set takes the entry transition to Run.
    {
        let a = w.get_component_mut::<Animator>(root).unwrap();
        a.set_bool("Fast", true);
        a.play("Idle");
    }
    tick(&mut w, &mut sys, 0.1);
    w.get_component_mut::<Animator>(root)
        .unwrap()
        .set_bool("Move", true);
    tick(&mut w, &mut sys, 0.1);
    assert_eq!(state_name(&w, root), "Run");
}

#[test]
fn interruption_solo_mute_and_offsets() {
    use oxide_assets::InterruptionSource;
    let (mut w, mut sys, [root, bone, _]) = rig(|a| {
        let (x, y, z) = (hold(a, "x", 0.0), hold(a, "y", 10.0), hold(a, "z", 20.0));
        let mut c = AnimatorController::default();
        c.add_parameter("Go", ParameterType::Bool)
            .add_parameter("Cut", ParameterType::Bool);
        c.add_state("A", Motion::Clip(x));
        c.add_state("B", Motion::Clip(y));
        c.add_state("C", Motion::Clip(z.clone()));
        c.add_state("Muted", Motion::Clip(z));
        let t = c.add_transition(Some("A"), "B", vec![cond("Go", ConditionMode::If)], 1.0);
        t.interruption_source = InterruptionSource::Source;
        t.ordered_interruption = false;
        c.add_transition(Some("A"), "C", vec![cond("Cut", ConditionMode::If)], 0.0);
        c.add_transition(Some("A"), "Muted", vec![cond("Go", ConditionMode::If)], 0.0)
            .mute = true;
        c
    });
    tick(&mut w, &mut sys, 0.1);
    w.get_component_mut::<Animator>(root)
        .unwrap()
        .set_bool("Go", true);
    tick(&mut w, &mut sys, 0.1);
    tick(&mut w, &mut sys, 0.25);
    tick(&mut w, &mut sys, 0.25);
    // Halfway through A → B (the muted transition never fires).
    let y = bone_y(&w, bone);
    assert!((y - 5.0).abs() < 0.6, "{y}");
    w.get_component_mut::<Animator>(root)
        .unwrap()
        .set_bool("Cut", true);
    tick(&mut w, &mut sys, 0.01);
    tick(&mut w, &mut sys, 0.01);
    assert_eq!(
        state_name(&w, root),
        "C",
        "the source's transition interrupted"
    );
    // Ordered interruption: a transition listed after the active one cannot interrupt.
    let mut c2 = AnimatorController::default();
    c2.add_parameter("Go", ParameterType::Bool)
        .add_parameter("Cut", ParameterType::Bool);
    c2.add_state("A", Motion::Empty);
    c2.add_state("B", Motion::Empty);
    c2.add_state("C", Motion::Empty);
    let t = c2.add_transition(Some("A"), "B", vec![cond("Go", ConditionMode::If)], 1.0);
    t.interruption_source = InterruptionSource::Source;
    c2.add_transition(Some("A"), "C", vec![cond("Cut", ConditionMode::If)], 0.0);
    let h = w
        .resource_mut::<AssetServer>()
        .controllers
        .insert("ordered", c2);
    let other = w.create("Other");
    let mut an = Animator::new(h);
    an.set_bool("Go", true);
    w.add_component(other, an);
    tick(&mut w, &mut sys, 0.1);
    w.get_component_mut::<Animator>(other)
        .unwrap()
        .set_bool("Cut", true);
    tick(&mut w, &mut sys, 0.1);
    let a = w.get_component::<Animator>(other).unwrap();
    assert!(
        a.is_in_transition(),
        "lower-priority transition must not interrupt"
    );
    assert!(a.get_next_animator_state_info(0).unwrap().is_name("B"));
}

#[test]
fn state_parameters_and_direct_trees() {
    use oxide_assets::{BlendTree, BlendTreeType, ChildMotion};
    let (mut w, mut sys, [root, bone, _]) = rig(|a| {
        let ramp = a.animations.insert(
            "ramp",
            clip_of(vec![track("Bone", Vector3::ZERO, Vector3::Y * 10.0, 1.0)]),
        );
        let ten = hold(a, "ten", 10.0);
        let mut c = AnimatorController::default();
        c.add_parameter("T", ParameterType::Float)
            .add_parameter("Speed", ParameterType::Float)
            .add_parameter("D", ParameterType::Float);
        let s = c.add_state("Scrub", Motion::Clip(ramp.clone()));
        s.time_parameter_active = true;
        s.time_parameter = "T".into();
        let s = c.add_state("Fast", Motion::Clip(ramp));
        s.speed_parameter_active = true;
        s.speed_parameter = "Speed".into();
        s.looping = false;
        c.add_state(
            "Direct",
            Motion::tree(BlendTree {
                blend_type: BlendTreeType::Direct,
                children: vec![ChildMotion {
                    motion: Motion::Clip(ten),
                    direct_blend_parameter: "D".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        );
        c
    });
    w.get_component_mut::<Animator>(root)
        .unwrap()
        .set_float("T", 0.25);
    tick(&mut w, &mut sys, 0.1);
    assert!((bone_y(&w, bone) - 2.5).abs() < 1e-3, "motion time from T");
    {
        let a = w.get_component_mut::<Animator>(root).unwrap();
        a.set_float("Speed", 2.0);
        a.play("Fast");
    }
    tick(&mut w, &mut sys, 0.1);
    tick(&mut w, &mut sys, 0.1);
    assert!(
        (bone_y(&w, bone) - 4.0).abs() < 1e-3,
        "speed × 2: {}",
        bone_y(&w, bone)
    );
    {
        let a = w.get_component_mut::<Animator>(root).unwrap();
        a.set_float("D", 0.25);
        a.play("Direct");
    }
    tick(&mut w, &mut sys, 0.1);
    tick(&mut w, &mut sys, 0.1);
    // A quarter of the way from the rest pose (y = 0) to 10.
    assert!(
        (bone_y(&w, bone) - 2.5).abs() < 1e-2,
        "{}",
        bone_y(&w, bone)
    );
}

#[test]
fn write_defaults_and_synced_layers() {
    let (mut w, mut sys, [root, bone, arm]) = rig(|a| {
        let arm_up = a.animations.insert(
            "arm",
            clip_of(vec![track(
                "Bone/Arm",
                Vector3::Y * 5.0,
                Vector3::Y * 5.0,
                1.0,
            )]),
        );
        let bone_up = hold(a, "bone", 3.0);
        let hurt = hold(a, "hurt", 7.0);
        let mut c = AnimatorController::default();
        c.add_parameter("Next", ParameterType::Trigger);
        c.add_state("Arm", Motion::Clip(arm_up));
        c.add_state("Bone", Motion::Clip(bone_up.clone()));
        c.add_transition(
            Some("Arm"),
            "Bone",
            vec![cond("Next", ConditionMode::If)],
            0.0,
        );
        let l = c.add_layer("Hurt");
        l.synced_layer_index = Some(0);
        *l.synced_motion_mut("Bone") = Motion::Clip(hurt);
        l.default_weight = 0.0;
        let _ = bone_up;
        c
    });
    w.transform_mut(arm).unwrap().local_position = Vector3::new(0.0, 1.0, 0.0);
    tick(&mut w, &mut sys, 0.1);
    assert!((w.transform(arm).unwrap().local_position.y - 5.0).abs() < 1e-4);
    w.get_component_mut::<Animator>(root)
        .unwrap()
        .set_trigger("Next");
    tick(&mut w, &mut sys, 0.1);
    // "Bone" doesn't animate the arm: Write Defaults puts it back to 1.
    assert!((w.transform(arm).unwrap().local_position.y - 1.0).abs() < 1e-4);
    assert!((bone_y(&w, bone) - 3.0).abs() < 1e-4);
    // The synced layer follows the base layer's state with its own motion.
    w.get_component_mut::<Animator>(root)
        .unwrap()
        .set_layer_weight(1, 1.0);
    tick(&mut w, &mut sys, 0.1);
    assert!(
        (bone_y(&w, bone) - 7.0).abs() < 1e-4,
        "{}",
        bone_y(&w, bone)
    );
    let a = w.get_component::<Animator>(root).unwrap();
    assert!(a.get_current_animator_state_info(1).is_name("Bone"));
}

#[test]
fn humanoid_body_part_masks() {
    let mut m = oxide_assets::AvatarMask::default();
    m.set_body_part_active(oxide_assets::AvatarMaskBodyPart::LeftArm, false);
    let mut w = World::new();
    let root = w.create("R");
    let mut human = HashMap::new();
    let arm = w.create("LeftUpperArm");
    let hand = w.create("LeftHand");
    let finger = w.create("Index");
    w.set_parent(arm, Some(root), false).unwrap();
    w.set_parent(hand, Some(arm), false).unwrap();
    w.set_parent(finger, Some(hand), false).unwrap();
    human.insert(HumanBodyBones::LeftUpperArm, arm);
    human.insert(HumanBodyBones::LeftHand, hand);
    let x = controller::excluded_bones_for_test(&w, &human, &m);
    assert!(x.contains(&arm) && x.contains(&hand));
    assert!(!x.contains(&finger), "fingers are their own part");
    m.set_body_part_active(oxide_assets::AvatarMaskBodyPart::LeftFingers, false);
    assert!(controller::excluded_bones_for_test(&w, &human, &m).contains(&finger));
}

#[derive(Clone, Default, oxide_core::Reflect)]
struct Counter {
    pub tag: String,
}

static LOG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

impl StateMachineBehaviour for Counter {
    fn on_state_enter(&mut self, _: &mut oxide_scene::Ctx, s: &AnimatorStateInfo, _: usize) {
        LOG.lock()
            .unwrap()
            .push(format!("{} enter {}", self.tag, s.name));
    }
    fn on_state_exit(&mut self, _: &mut oxide_scene::Ctx, s: &AnimatorStateInfo, _: usize) {
        LOG.lock()
            .unwrap()
            .push(format!("{} exit {}", self.tag, s.name));
    }
    fn on_state_machine_enter(&mut self, _: &mut oxide_scene::Ctx, m: &str) {
        LOG.lock()
            .unwrap()
            .push(format!("{} machine-enter {m}", self.tag));
    }
}

#[test]
fn state_machine_behaviours_get_callbacks() {
    let (mut w, mut sys, [root, _, _]) = rig(|a| {
        let x = hold(a, "x", 0.0);
        let mut c = AnimatorController::default();
        c.add_parameter("Go", ParameterType::Trigger);
        let s = c.add_state("A", Motion::Clip(x.clone()));
        s.behaviours.push(oxide_assets::StateBehaviour {
            type_name: "Counter".into(),
            fields: [("tag".to_owned(), Value::String("sA".into()))].into(),
        });
        let sub = c.base().add_state_machine("Sub");
        sub.behaviours.push(oxide_assets::StateBehaviour {
            type_name: "Counter".into(),
            fields: [("tag".to_owned(), Value::String("mSub".into()))].into(),
        });
        sub.add_state("B", Motion::Clip(x));
        c.add_transition(Some("A"), "B", vec![cond("Go", ConditionMode::If)], 0.0);
        c
    });
    let mut reg = StateMachineBehaviours::default();
    reg.register::<Counter>();
    w.resources.insert(reg);
    w.run_scripts = true;
    LOG.lock().unwrap().clear();
    tick(&mut w, &mut sys, 0.1);
    w.get_component_mut::<Animator>(root)
        .unwrap()
        .set_trigger("Go");
    tick(&mut w, &mut sys, 0.1);
    let log = LOG.lock().unwrap().clone();
    assert_eq!(
        log,
        [
            "sA enter A",
            "mSub enter B",
            "sA exit A",
            "mSub machine-enter Sub"
        ],
        "{log:?}"
    );
}
