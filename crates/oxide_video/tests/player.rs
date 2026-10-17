//! VideoPlayer playback: preparing, frames over time, looping, events, material override.

use oxide_assets::{AssetServer, Handle, Material};
use oxide_core::Time;
use oxide_scene::{Component, Ctx, MeshRenderer, World};
use oxide_video::encode::encode_mp4;
use oxide_video::*;
use std::sync::{Arc, Mutex};

/// Records the video messages an object receives.
#[derive(Clone, Default, oxide_core::Reflect)]
struct Events {
    #[reflect(skip)]
    log: Arc<Mutex<Vec<String>>>,
}

impl Component for Events {
    fn on_message(
        &mut self,
        _ctx: &mut Ctx,
        name: &str,
        _arg: &oxide_core::reflect::Value,
    ) -> bool {
        self.log.lock().unwrap().push(name.to_owned());
        true
    }
}

fn frames(n: usize, w: u32, h: u32) -> Vec<Vec<u8>> {
    (0..n)
        .map(|i| {
            let c = [(i * 40).min(255) as u8, 60, 200, 255];
            c.iter()
                .copied()
                .cycle()
                .take((w * h * 4) as usize)
                .collect()
        })
        .collect()
}

fn world() -> (World, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("oxide-video-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Assets")).unwrap();
    std::fs::write(
        dir.join("Assets/clip.mp4"),
        encode_mp4(&frames(6, 32, 32), 32, 32, 10, 3).unwrap(),
    )
    .unwrap();
    let mut w = World::new();
    oxide_scene::register_builtin(&mut w.registry);
    w.resources.insert(Time::new());
    w.resources.insert(AssetServer::new(&dir));
    w.registry.register::<Events>("Test/Events");
    (w, dir)
}

/// Steps until `done` (the decoder runs on another thread) or a second passes.
fn step_until(w: &mut World, dt: f64, mut done: impl FnMut(&World) -> bool) {
    for _ in 0..200 {
        w.resource_mut::<Time>().begin_frame(dt);
        update(w);
        if done(w) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("timed out");
}

#[test]
fn plays_loops_and_overrides_materials() {
    let (mut w, dir) = world();
    let screen = w.create("Screen");
    let mat = w.resource_mut::<AssetServer>().materials.insert(
        "test:screen",
        Material::unlit("screen", oxide_math::Color::WHITE),
    );
    w.add_component(
        screen,
        MeshRenderer {
            materials: vec![mat],
            ..Default::default()
        },
    );
    let log = Arc::new(Mutex::new(Vec::new()));
    w.add_component(screen, Events { log: log.clone() });
    w.add_component(
        screen,
        VideoPlayer {
            clip: Some(Handle::from_path("Assets/clip.mp4")),
            is_looping: true,
            ..Default::default()
        },
    );
    let status = |w: &World| {
        w.get_component::<VideoPlayer>(screen)
            .unwrap()
            .status
            .clone()
    };
    // Play On Awake: prepared once the first frame arrives.
    step_until(&mut w, 0.0, |w| status(w).is_prepared);
    let s = status(&w);
    assert_eq!((s.width, s.height, s.frame_count), (32, 32, 6));
    assert!((s.length - 0.6).abs() < 1e-3, "{}", s.length);
    assert_eq!(s.frame, 0);
    // Its frames go into the Render Texture, and the renderer's material shows it.
    let tex = texture_path(screen);
    assert!(
        w.resource::<AssetServer>()
            .render_textures
            .handle_of(&tex)
            .is_some()
    );
    let r = w.get_component::<MeshRenderer>(screen).unwrap();
    let m = w
        .resource::<AssetServer>()
        .materials
        .get(&r.materials[0])
        .unwrap();
    assert_eq!(
        m.base_map.as_ref().map(|h| h.path().to_owned()),
        Some(tex.clone())
    );
    let mut writes = Vec::new();
    extract(&w, &mut writes, &mut Vec::new());
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].rgba[0], 0, "frame 0 is the least red");
    // Time runs; later frames show.
    step_until(&mut w, 0.05, |w| status(w).frame >= 3);
    let s = status(&w);
    assert!(s.is_playing && s.time > 0.25, "{s:?}");
    // It loops at the end.
    step_until(&mut w, 0.05, |_| {
        log.lock()
            .unwrap()
            .iter()
            .any(|m| m == "OnVideoLoopPointReached")
    });
    step_until(&mut w, 0.05, |w| status(w).frame == 1);
    assert!(status(&w).is_playing, "still playing after the loop");
    {
        let l = log.lock().unwrap();
        assert!(l.iter().any(|m| m == "OnVideoPrepared"));
        assert!(l.iter().any(|m| m == "OnVideoStarted"));
    }
    // Pause holds the time; Stop goes back to the start.
    w.get_component_mut::<VideoPlayer>(screen).unwrap().pause();
    step_until(&mut w, 0.05, |w| status(w).is_paused);
    let t = status(&w).time;
    for _ in 0..5 {
        w.resource_mut::<Time>().begin_frame(0.05);
        update(&mut w);
    }
    assert_eq!(status(&w).time, t);
    w.get_component_mut::<VideoPlayer>(screen).unwrap().stop();
    step_until(&mut w, 0.05, |w| !status(w).is_playing);
    assert_eq!(status(&w).time, 0.0);
    // Seeking.
    w.get_component_mut::<VideoPlayer>(screen)
        .unwrap()
        .set_frame(4);
    step_until(&mut w, 0.0, |w| status(w).frame == 4);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_missing_clip_reports_an_error() {
    let (mut w, dir) = world();
    let id = w.create("Broken");
    w.add_component(
        id,
        VideoPlayer {
            clip: Some(Handle::from_path("Assets/missing.mp4")),
            ..Default::default()
        },
    );
    w.resource_mut::<Time>().begin_frame(0.016);
    update(&mut w);
    let s = &w.get_component::<VideoPlayer>(id).unwrap().status;
    assert!(
        s.error
            .as_deref()
            .is_some_and(|e| e.contains("missing.mp4")),
        "{s:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
}
