use super::*;
use oxide_assets::{Condition, Interpolation, Track, TransformCurves};

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
        length: 0.0,
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
    assert_eq!(a.rt.current.as_ref().unwrap().state, 1, "moved to Jump");
    assert!(!a.get_bool("Jump"), "trigger consumed");
}
