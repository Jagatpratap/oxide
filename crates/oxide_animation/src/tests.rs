use super::*;
use oxide_assets::{BlendTree2DType, Condition, Interpolation, Track, TransformCurves};
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
