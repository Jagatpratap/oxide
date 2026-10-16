use super::*;
use oxide_assets::{AnimationClip, Interpolation, Track, TransformCurves};
use oxide_scene::Ctx;

fn clip_at(start: f32, duration: f32) -> TimelineClip {
    TimelineClip {
        start,
        duration,
        ..Default::default()
    }
}

#[test]
fn overlapping_clips_blend_and_weights_sum_to_one() {
    let mut track = TrackAsset::new("A", TrackType::Animation);
    track.clips = vec![clip_at(0.0, 2.0), clip_at(1.5, 2.0)];
    assert_eq!(mix::blends(&track.clips), vec![(0.0, 0.5), (0.5, 0.0)]);
    let a = evaluate_track(&track, 1.75);
    assert_eq!(a.len(), 2);
    let sum: f32 = a.iter().map(|c| c.weight).sum();
    assert!((sum - 1.0).abs() < 1e-5, "{a:?}");
    assert!((a[0].weight - 0.5).abs() < 1e-5);
    // Outside the overlap only one clip plays at full weight.
    let a = evaluate_track(&track, 1.0);
    assert_eq!((a.len(), a[0].weight), (1, 1.0));
}

#[test]
fn ease_speed_clip_in_and_extrapolation() {
    let mut c = clip_at(1.0, 2.0);
    c.ease_in_duration = 1.0;
    c.clip_in = 0.5;
    c.time_scale = 2.0;
    c.post_extrapolation = ClipExtrapolation::Loop;
    c.pre_extrapolation = ClipExtrapolation::Hold;
    let mut track = TrackAsset::new("A", TrackType::Animation);
    track.clips = vec![c];
    let a = evaluate_track(&track, 1.5)[0];
    assert!((a.weight - 0.5).abs() < 1e-5);
    assert!((a.source_time - (0.5 + 0.5 * 2.0)).abs() < 1e-5);
    // Before: held on the first frame; after: looping.
    let a = evaluate_track(&track, 0.2)[0];
    assert!(a.extrapolated && a.local == 0.0 && a.weight == 1.0);
    let a = evaluate_track(&track, 3.5)[0];
    assert!((a.local - 0.5).abs() < 1e-5, "{a:?}");
    track.clips[0].post_extrapolation = ClipExtrapolation::PingPong;
    let a = evaluate_track(&track, 4.5)[0];
    assert!((a.local - 0.5).abs() < 1e-5, "{a:?}");
    track.clips[0].post_extrapolation = ClipExtrapolation::None;
    assert!(evaluate_track(&track, 3.5).is_empty());
}

#[test]
fn markers_crossed_with_wrap() {
    let times = [0.5f32, 1.0, 2.5];
    assert_eq!(
        mix::crossed(times.iter().copied(), 0.4, 1.0, 3.0),
        vec![0, 1]
    );
    assert_eq!(
        mix::crossed(times.iter().copied(), 2.0, 0.6, 3.0),
        vec![0, 2]
    );
}

fn move_clip() -> AnimationClip {
    let mut c = AnimationClip {
        name: "Move".into(),
        curves: vec![TransformCurves {
            path: String::new(),
            position: Some(Track {
                times: vec![0.0, 1.0],
                values: vec![Vector3::ZERO, Vector3::X * 10.0],
                interpolation: Interpolation::Linear,
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    c.recalculate_length();
    c
}

#[derive(Clone, Default, Reflect)]
struct Counter {
    calls: Vec<String>,
}

impl Component for Counter {
    fn on_message(&mut self, _ctx: &mut Ctx, message: &str, arg: &Value) -> bool {
        self.calls.push(format!("{message}:{arg:?}"));
        true
    }
}

struct Scene {
    w: World,
    sys: Box<dyn System>,
    dir: GameObjectId,
    cube: GameObjectId,
    light: GameObjectId,
}

fn setup(timeline: TimelineAsset) -> Scene {
    let mut w = World::new();
    w.resources.insert(Time::new());
    let mut assets = AssetServer::new(".");
    assets.animations.insert("Assets/Move.anim", move_clip());
    assets.data.insert(
        "Assets/Door.signal",
        "SignalAsset",
        Box::new(SignalAsset::default()),
    );
    assets
        .data
        .insert("Assets/Intro.playable", "TimelineAsset", Box::new(timeline));
    w.resources.insert(assets);
    let sys = install(&mut w);
    let dir = w.create("Director");
    let cube = w.create("Cube");
    let light = w.create("Light");
    w.set_active(light, false);
    let mut d = PlayableDirector::new("Assets/Intro.playable");
    d.set_binding("Move", Some(cube));
    d.set_binding("Show", Some(light));
    d.set_binding("Signals", Some(cube));
    w.add_component(dir, d);
    w.add_component(cube, Counter::default());
    w.add_component(
        cube,
        SignalReceiver {
            reactions: vec![SignalReaction {
                signal: Some(Handle::from_path("Assets/Door.signal")),
                target: None,
                method: "OnDoor".into(),
                parameter: "open".into(),
            }],
        },
    );
    w.run_scripts = true;
    Scene {
        w,
        sys,
        dir,
        cube,
        light,
    }
}

fn intro() -> TimelineAsset {
    let mut move_track = TrackAsset::new("Move", TrackType::Animation);
    move_track.clips.push(TimelineClip {
        animation_clip: Some(Handle::from_path("Assets/Move.anim")),
        ..clip_at(0.0, 1.0)
    });
    let mut show = TrackAsset::new("Show", TrackType::Activation);
    show.post_playback_state = PostPlaybackState::Revert;
    show.clips.push(clip_at(0.5, 0.5));
    let mut signals = TrackAsset::new("Signals", TrackType::Signal);
    signals.markers.push(SignalEmitter {
        time: 0.6,
        signal: Some(Handle::from_path("Assets/Door.signal")),
        ..Default::default()
    });
    TimelineAsset {
        tracks: vec![move_track, show, signals],
        duration_mode: DurationMode::FixedLength,
        fixed_duration: 2.0,
        ..Default::default()
    }
}

fn tick(s: &mut Scene, dt: f64) {
    s.w.resource_mut::<Time>().begin_frame(dt);
    s.sys.update(&mut s.w);
}

#[test]
fn director_plays_animation_activation_and_signals() {
    let mut s = setup(intro());
    tick(&mut s, 0.25); // starts at 0 (play on awake)
    tick(&mut s, 0.25);
    let x = s.w.transform(s.cube).unwrap().local_position.x;
    assert!((x - 2.5).abs() < 1e-3, "{x}");
    assert!(!s.w.get(s.light).unwrap().active_self());
    tick(&mut s, 0.25); // 0.5: light on
    assert!(s.w.get(s.light).unwrap().active_self());
    tick(&mut s, 0.25); // 0.75: the signal at 0.6 fired once
    let calls = s.w.get_component::<Counter>(s.cube).unwrap().calls.clone();
    assert_eq!(calls, vec![r#"OnDoor:String("open")"#.to_owned()]);
    tick(&mut s, 0.25); // 1.0: light clip ended
    assert!(!s.w.get(s.light).unwrap().active_self());
    // Past the end (fixed length 2 s, wrap None): stops and reverts.
    for _ in 0..6 {
        tick(&mut s, 0.25);
    }
    let d = s.w.get_component::<PlayableDirector>(s.dir).unwrap();
    assert_eq!(d.state(), PlayState::Stopped);
    assert_eq!(s.w.get_component::<Counter>(s.cube).unwrap().calls.len(), 1);
}

#[test]
fn wrap_modes_and_pause() {
    let mut s = setup(intro());
    s.w.get_component_mut::<PlayableDirector>(s.dir)
        .unwrap()
        .wrap_mode = DirectorWrapMode::Loop;
    for _ in 0..12 {
        tick(&mut s, 0.25); // the first frame starts at 0; 2.0 loops to 0, then 0.75
    }
    let d = s.w.get_component::<PlayableDirector>(s.dir).unwrap();
    assert!((d.time() - 0.75).abs() < 1e-6, "{}", d.time());
    assert_eq!(d.state(), PlayState::Playing);
    // Looping sent the signal again.
    assert_eq!(s.w.get_component::<Counter>(s.cube).unwrap().calls.len(), 2);
    s.w.get_component_mut::<PlayableDirector>(s.dir)
        .unwrap()
        .pause();
    tick(&mut s, 0.25);
    tick(&mut s, 0.25);
    let d = s.w.get_component::<PlayableDirector>(s.dir).unwrap();
    assert_eq!((d.state(), d.time()), (PlayState::Paused, 0.75));
}

#[test]
fn ease_in_blends_over_current_pose_and_offsets_apply() {
    let mut t = intro();
    let track = &mut t.tracks[0];
    track.clips[0].ease_in_duration = 1.0;
    track.apply_offsets = true;
    track.position = Vector3::new(0.0, 5.0, 0.0);
    let mut s = setup(t);
    for _ in 0..3 {
        tick(&mut s, 0.25); // t = 0.5: clip x = 5 (+ offset y 5), weight = ease(0.5) = 0.5
    }
    let p = s.w.transform(s.cube).unwrap().local_position;
    assert!(
        (p.x - 2.5).abs() < 1e-3 && (p.y - 2.5).abs() < 1e-3,
        "{p:?}"
    );
}

#[test]
fn editor_preview_scrubs_without_signals() {
    let mut s = setup(intro());
    s.w.run_scripts = false;
    preview(&mut s.w, s.dir, 0.75, false, 0.0);
    assert!((s.w.transform(s.cube).unwrap().local_position.x - 7.5).abs() < 1e-3);
    assert!(s.w.get(s.light).unwrap().active_self());
    assert!(
        s.w.get_component::<Counter>(s.cube)
            .unwrap()
            .calls
            .is_empty()
    );
    let cid = find_director(&s.w, s.dir).unwrap();
    release(&mut s.w, s.dir, cid);
    // Revert puts the light back to inactive.
    assert!(!s.w.get(s.light).unwrap().active_self());
}

#[derive(Clone, Default, Reflect)]
struct Fader {
    target: f32,
    log: Vec<String>,
}

impl PlayableBehaviour for Fader {
    fn on_behaviour_play(&mut self, world: &mut World, info: &FrameInfo) {
        let _ = world;
        self.log.push(format!("play {}", self.target));
        let _ = info;
    }
    fn process_frame(&mut self, world: &mut World, info: &FrameInfo) {
        if let Some(b) = info.binding
            && let Some(t) = world.transform_mut(b)
        {
            t.local_scale = Vector3::splat(self.target * info.weight);
        }
    }
}

#[test]
fn control_track_drives_nested_director_and_playable_clips_run() {
    let mut s = setup(intro());
    // A child Timeline driven by a Control clip of the parent.
    let mut child = TrackAsset::new("Move", TrackType::Animation);
    child.clips.push(TimelineClip {
        animation_clip: Some(Handle::from_path("Assets/Move.anim")),
        ..clip_at(0.0, 1.0)
    });
    let child_tl = TimelineAsset {
        tracks: vec![child],
        ..Default::default()
    };
    s.w.resource_mut::<AssetServer>().data.insert(
        "Assets/Child.playable",
        "TimelineAsset",
        Box::new(child_tl),
    );
    let holder = s.w.create("Holder");
    let mover = s.w.create("Mover");
    s.w.set_active(holder, false);
    let mut cd = PlayableDirector::new("Assets/Child.playable");
    cd.play_on_awake = false;
    cd.set_binding("Move", Some(mover));
    s.w.add_component(holder, cd);
    {
        let mut parent: TimelineAsset = intro();
        let mut control = TrackAsset::new("Control", TrackType::Control);
        control.clips.push(TimelineClip {
            source_game_object: "Holder".into(),
            ..clip_at(1.0, 1.0)
        });
        let mut pt = TrackAsset::new("Custom", TrackType::Playable);
        pt.clips.push(TimelineClip {
            behaviour: "Fader".into(),
            behaviour_fields: vec![BehaviourField {
                name: "target".into(),
                json: serde_json::to_string(&Value::Float(2.0)).unwrap(),
            }],
            ..clip_at(0.0, 1.0)
        });
        parent.tracks.push(control);
        parent.tracks.push(pt);
        s.w.resource_mut::<AssetServer>().data.insert(
            "Assets/Intro.playable",
            "TimelineAsset",
            Box::new(parent),
        );
    }
    s.w.resource_mut::<PlayableBehaviours>().register::<Fader>();
    {
        let d = s.w.get_component_mut::<PlayableDirector>(s.dir).unwrap();
        d.set_reference("Holder", Some(holder));
        d.set_binding("Custom", Some(s.light));
    }
    tick(&mut s, 0.25);
    tick(&mut s, 0.25);
    let sc = s.w.transform(s.light).unwrap().local_scale;
    assert!((sc.x - 2.0).abs() < 1e-4, "{sc:?}");
    for _ in 0..5 {
        tick(&mut s, 0.25); // 1.5: holder on, child at 0.5
    }
    assert!(s.w.get(holder).unwrap().active_self());
    let x = s.w.transform(mover).unwrap().local_position.x;
    assert!((x - 5.0).abs() < 1e-3, "{x}");
    for _ in 0..3 {
        tick(&mut s, 0.25); // 2.25: control clip over → holder reverts to inactive
    }
    assert!(!s.w.get(holder).unwrap().active_self());
}
