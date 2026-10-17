//! Oxide video: Unity's `VideoClip` (H.264 MP4 / MOV, animated GIF) and `VideoPlayer`.
//! Clips decode on a background thread per player; frames go into a Render Texture
//! (`runtime:video/...`) updated in place, which the render modes show: behind or in front
//! of a camera, on a renderer's material, in a target Render Texture, or only through
//! [`VideoPlayer::texture`]. MP4 audio (AAC) plays through the audio system in sync.
//! Reference: <https://docs.unity3d.com/Manual/class-VideoPlayer.html>

pub mod encode;
pub mod source;

use oxide_assets::{Asset, AssetId, AssetServer, AudioClip, Handle, Material, RenderTexture};
use oxide_core::{Reflect, Time};
use oxide_math::{Quaternion, Vector3};
use oxide_render::{ParticleBatch, ParticleBlend, ParticleInstance, TextureWrite};
use oxide_scene::{Camera, Component, ComponentId, GameObjectId, MeshRenderer, System, World};
use source::{Frame, FrameSource, VideoInfo};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, TrySendError};

/// Unity `VideoClip`: a video file in the project (`.mp4`, `.m4v`, `.mov`, `.gif`).
#[derive(Clone, Debug)]
pub struct VideoClip;

impl Asset for VideoClip {
    const TYPE_NAME: &'static str = "VideoClip";
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum VideoSource {
    #[default]
    VideoClip,
    /// `url`: a file path (absolute, or relative to the project).
    Url,
}

/// Unity `VideoRenderMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum VideoRenderMode {
    /// Behind everything the camera sees.
    CameraFarPlane,
    /// In front of everything the camera sees.
    CameraNearPlane,
    /// Into `target_texture`.
    RenderTexture,
    /// Into a renderer's material (`target_material_renderer`).
    #[default]
    MaterialOverride,
    /// Only through `texture`.
    ApiOnly,
}

/// Unity `VideoAspectRatio`: how frames fit a camera plane or Render Texture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum VideoAspectRatio {
    NoScaling,
    FitVertically,
    FitHorizontally,
    #[default]
    FitInside,
    FitOutside,
    Stretch,
}

/// Unity `VideoAudioOutputMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum VideoAudioOutputMode {
    None,
    /// Through `target_audio_source` (its volume and position).
    AudioSource,
    #[default]
    Direct,
}

#[derive(Clone, Debug, PartialEq)]
#[doc(hidden)]
pub enum VideoCommand {
    Prepare,
    Play,
    Pause,
    Stop,
    Seek(f64),
}

/// What the player is doing (kept up to date by the video system).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoStatus {
    pub is_prepared: bool,
    pub is_playing: bool,
    pub is_paused: bool,
    /// Seconds into the clip.
    pub time: f64,
    pub frame: i64,
    pub frame_count: u64,
    pub frame_rate: f64,
    pub length: f64,
    pub width: u32,
    pub height: u32,
    pub error: Option<String>,
}

/// Unity `VideoPlayer`.
#[derive(Clone, Debug, Reflect)]
pub struct VideoPlayer {
    pub source: VideoSource,
    pub clip: Option<Handle<VideoClip>>,
    pub url: String,
    pub play_on_awake: bool,
    /// Time waits for the first frame (no jump at the start).
    pub wait_for_first_frame: bool,
    pub is_looping: bool,
    /// Frames are skipped to keep up with time (else the video slows down).
    pub skip_on_drop: bool,
    pub playback_speed: f32,
    pub render_mode: VideoRenderMode,
    /// Camera planes: the camera (empty = the main camera) and the video's opacity.
    pub target_camera: Option<GameObjectId>,
    pub target_camera_alpha: f32,
    pub target_texture: Option<Handle<RenderTexture>>,
    /// Material Override: the renderer (empty = this object's).
    pub target_material_renderer: Option<GameObjectId>,
    /// Material Override: the texture slot (`base_map` or `emission_map`).
    pub target_material_property: String,
    pub aspect_ratio: VideoAspectRatio,
    pub audio_output_mode: VideoAudioOutputMode,
    pub target_audio_source: Option<GameObjectId>,
    pub direct_audio_volume: f32,
    pub direct_audio_mute: bool,
    #[reflect(skip)]
    #[doc(hidden)]
    pub commands: Vec<VideoCommand>,
    #[reflect(skip)]
    pub status: VideoStatus,
}

impl Default for VideoPlayer {
    fn default() -> Self {
        Self {
            source: VideoSource::VideoClip,
            clip: None,
            url: String::new(),
            play_on_awake: true,
            wait_for_first_frame: true,
            is_looping: false,
            skip_on_drop: true,
            playback_speed: 1.0,
            render_mode: VideoRenderMode::MaterialOverride,
            target_camera: None,
            target_camera_alpha: 1.0,
            target_texture: None,
            target_material_renderer: None,
            target_material_property: "base_map".into(),
            aspect_ratio: VideoAspectRatio::FitInside,
            audio_output_mode: VideoAudioOutputMode::Direct,
            target_audio_source: None,
            direct_audio_volume: 1.0,
            direct_audio_mute: false,
            commands: Vec::new(),
            status: VideoStatus::default(),
        }
    }
}

impl Component for VideoPlayer {}

impl VideoPlayer {
    /// Opens the clip and starts decoding (Unity `Prepare`); `OnVideoPrepared` follows.
    pub fn prepare(&mut self) {
        self.commands.push(VideoCommand::Prepare);
    }
    pub fn play(&mut self) {
        self.commands.push(VideoCommand::Play);
    }
    pub fn pause(&mut self) {
        self.commands.push(VideoCommand::Pause);
    }
    /// Stops and goes back to the start.
    pub fn stop(&mut self) {
        self.commands.push(VideoCommand::Stop);
    }
    /// Unity `time = t`: jumps to `t` seconds.
    pub fn set_time(&mut self, t: f64) {
        self.commands.push(VideoCommand::Seek(t));
    }
    /// Unity `frame = n`.
    pub fn set_frame(&mut self, frame: i64) {
        let rate = self.status.frame_rate.max(1e-3);
        self.commands
            .push(VideoCommand::Seek(frame.max(0) as f64 / rate));
    }
    pub fn is_playing(&self) -> bool {
        self.status.is_playing
    }
    pub fn is_prepared(&self) -> bool {
        self.status.is_prepared
    }
    pub fn time(&self) -> f64 {
        self.status.time
    }
    pub fn frame(&self) -> i64 {
        self.status.frame
    }
    pub fn length(&self) -> f64 {
        self.status.length
    }
}

/// The Render Texture a player's frames go into (Unity `VideoPlayer.texture`).
pub fn texture_path(object: GameObjectId) -> String {
    format!("runtime:video/{}", object.to_bits())
}

// ---- the decode thread ---------------------------------------------------------------------

enum Cmd {
    Seek(f64, u64),
}

enum Msg {
    Frame(Frame, u64),
    End(u64),
    Error(String),
}

/// A background decoder: frames come ahead of time through a small queue.
struct Decoding {
    cmd: Sender<Cmd>,
    frames: Receiver<Msg>,
}

fn spawn(mut src: Box<dyn FrameSource>) -> Decoding {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
    let (tx, rx): (SyncSender<Msg>, Receiver<Msg>) = std::sync::mpsc::sync_channel(4);
    std::thread::Builder::new()
        .name("video decode".into())
        .spawn(move || {
            let mut generation = 0u64;
            let mut ended = false;
            let mut pending: Option<Msg> = None;
            loop {
                // Commands first (seeking drops what was queued).
                loop {
                    let cmd = if ended && pending.is_none() {
                        match cmd_rx.recv() {
                            Ok(c) => Some(c),
                            Err(_) => return,
                        }
                    } else {
                        match cmd_rx.try_recv() {
                            Ok(c) => Some(c),
                            Err(TryRecvError::Empty) => None,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    };
                    match cmd {
                        Some(Cmd::Seek(t, g)) => {
                            generation = g;
                            ended = false;
                            pending = None;
                            if let Err(e) = src.seek(t) {
                                let _ = tx.send(Msg::Error(e));
                                return;
                            }
                        }
                        None => break,
                    }
                }
                let msg = match pending.take() {
                    Some(m) => m,
                    None => match src.next_frame() {
                        Ok(Some(f)) => Msg::Frame(f, generation),
                        Ok(None) => {
                            ended = true;
                            Msg::End(generation)
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::Error(e));
                            return;
                        }
                    },
                };
                match tx.try_send(msg) {
                    Ok(()) => {}
                    Err(TrySendError::Full(m)) => {
                        // The queue is full: wait a little (still answering seeks).
                        pending = Some(m);
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    Err(TrySendError::Disconnected(_)) => return,
                }
            }
        })
        .expect("video thread");
    Decoding {
        cmd: cmd_tx,
        frames: rx,
    }
}

/// One player's playback state.
struct Playback {
    decoding: Decoding,
    info: VideoInfo,
    generation: u64,
    time: f64,
    shown: Option<Frame>,
    next: Option<Frame>,
    ended: bool,
    playing: bool,
    paused: bool,
    started: bool,
    serial: u64,
    audio: Option<Handle<AudioClip>>,
    source_key: String,
}

impl Playback {
    fn seek(&mut self, t: f64) {
        self.generation += 1;
        self.time = t.clamp(0.0, self.info.length);
        self.next = None;
        self.ended = false;
        let _ = self
            .decoding
            .cmd
            .send(Cmd::Seek(self.time, self.generation));
    }

    /// Takes the frames due by now; returns whether a new one is shown.
    fn pull(&mut self, skip: bool) -> Result<bool, String> {
        let mut new = false;
        loop {
            if self.next.is_none() {
                match self.decoding.frames.try_recv() {
                    Ok(Msg::Frame(f, g)) if g == self.generation => self.next = Some(f),
                    Ok(Msg::End(g)) if g == self.generation => self.ended = true,
                    Ok(Msg::Error(e)) => return Err(e),
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
            let Some(n) = &self.next else { break };
            // The first frame shows at once; later ones when their time comes.
            if self.shown.is_none() || n.time <= self.time + 1e-6 {
                self.shown = self.next.take();
                new = true;
                if !skip {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(new)
    }
}

/// Every player's playback (a World resource).
#[derive(Default)]
pub struct VideoPlayers {
    playing: HashMap<ComponentId, Playback>,
    /// Players whose Play On Awake has been handled.
    awake: std::collections::HashSet<ComponentId>,
    /// Pixels to write this frame (extracted into the render frame).
    writes: Vec<TextureWrite>,
    /// Camera-plane quads (extracted into the render frame).
    planes: Vec<(GameObjectId, AssetId, f32, f32)>,
}

// ---- helpers --------------------------------------------------------------------------------

/// Fits `src` (`sw` × `sh`) into `dw` × `dh` by `mode`, black around it.
pub fn fit_frame(
    src: &[u8],
    sw: u32,
    sh: u32,
    dw: u32,
    dh: u32,
    mode: VideoAspectRatio,
) -> Vec<u8> {
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    for p in out.as_chunks_mut::<4>().0 {
        p[3] = 255;
    }
    let (sw_f, sh_f, dw_f, dh_f) = (sw as f32, sh as f32, dw as f32, dh as f32);
    let (sx, sy) = match mode {
        VideoAspectRatio::Stretch => (dw_f / sw_f, dh_f / sh_f),
        VideoAspectRatio::NoScaling => (1.0, 1.0),
        VideoAspectRatio::FitVertically => (dh_f / sh_f, dh_f / sh_f),
        VideoAspectRatio::FitHorizontally => (dw_f / sw_f, dw_f / sw_f),
        VideoAspectRatio::FitInside => {
            let k = (dw_f / sw_f).min(dh_f / sh_f);
            (k, k)
        }
        VideoAspectRatio::FitOutside => {
            let k = (dw_f / sw_f).max(dh_f / sh_f);
            (k, k)
        }
    };
    let (w, h) = (sw_f * sx, sh_f * sy);
    let (ox, oy) = ((dw_f - w) * 0.5, (dh_f - h) * 0.5);
    for y in 0..dh {
        let fy = (y as f32 + 0.5 - oy) / sy;
        if fy < 0.0 || fy >= sh_f {
            continue;
        }
        for x in 0..dw {
            let fx = (x as f32 + 0.5 - ox) / sx;
            if fx < 0.0 || fx >= sw_f {
                continue;
            }
            let s = ((fy as u32 * sw + fx as u32) * 4) as usize;
            let d = ((y * dw + x) * 4) as usize;
            out[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    out
}

fn source_path(p: &VideoPlayer, assets: &AssetServer) -> Option<(String, std::path::PathBuf)> {
    match p.source {
        VideoSource::VideoClip => p
            .clip
            .as_ref()
            .map(|h| (h.path().to_owned(), assets.full_path(h.path()))),
        VideoSource::Url => {
            let u = p.url.trim().trim_start_matches("file://");
            if u.is_empty() {
                return None;
            }
            let path = std::path::Path::new(u);
            Some((
                u.to_owned(),
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    assets.full_path(u)
                },
            ))
        }
    }
}

fn open_player(world: &mut World, id: GameObjectId, p: &VideoPlayer) -> Result<Playback, String> {
    let assets = world.resource_mut::<AssetServer>();
    let (name, path) = source_path(p, assets).ok_or("no video clip or URL")?;
    let bytes: Arc<[u8]> = std::fs::read(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .into();
    let src = source::open(bytes.clone(), &name)?;
    let info = src.info().clone();
    // The frames' Render Texture.
    assets.render_textures.insert(
        texture_path(id),
        RenderTexture {
            name: format!("Video {name}"),
            width: info.width.max(1),
            height: info.height.max(1),
        },
    );
    // The soundtrack, decoded by the audio system from the same file.
    let audio = (info.has_audio && p.audio_output_mode != VideoAudioOutputMode::None).then(|| {
        let path = format!("runtime:video-audio/{name}");
        assets.audio.insert(
            path,
            AudioClip {
                name: format!("{name} (audio)"),
                bytes,
            },
        )
    });
    Ok(Playback {
        decoding: spawn(src),
        info,
        generation: 0,
        time: 0.0,
        shown: None,
        next: None,
        ended: false,
        playing: false,
        paused: false,
        started: false,
        serial: 0,
        audio,
        source_key: name,
    })
}

/// Material Override: the renderer's first material, with the video in `property`.
fn override_material(
    world: &mut World,
    renderer: GameObjectId,
    player: GameObjectId,
    property: &str,
) {
    let tex: Handle<oxide_assets::Texture2D> = Handle::from_path(texture_path(player));
    let Some(base) = world
        .get_component::<MeshRenderer>(renderer)
        .and_then(|r| r.materials.first().cloned())
    else {
        return;
    };
    let path = format!("runtime:video-material/{}", player.to_bits());
    if base.path() == path {
        return;
    }
    let assets = world.resource_mut::<AssetServer>();
    let mut m: Material = assets.materials.get(&base).cloned().unwrap_or_default();
    match property {
        "emission_map" | "_EmissionMap" => {
            m.emission_map = Some(tex);
            if m.emission_intensity <= 0.0 {
                m.emission_intensity = 1.0;
            }
        }
        _ => m.base_map = Some(tex),
    }
    let h = assets.materials.insert(path, m);
    if let Some(r) = world.get_component_mut::<MeshRenderer>(renderer)
        && let Some(slot) = r.materials.first_mut()
    {
        *slot = h;
    }
}

// ---- the system ---------------------------------------------------------------------------

/// Plays every VideoPlayer (Play mode).
#[derive(Default)]
pub struct VideoSystem;

impl System for VideoSystem {
    fn name(&self) -> &'static str {
        "Video"
    }

    fn update(&mut self, world: &mut World) {
        update(world);
    }
}

/// Runs players' commands, advances playback, collects frames and syncs audio.
pub fn update(world: &mut World) {
    let dt = world.try_resource::<Time>().map_or(0.0, |t| t.delta_time()) as f64;
    if world.try_resource::<VideoPlayers>().is_none() {
        world.resources.insert(VideoPlayers::default());
    }
    let mut players = world.resources.remove::<VideoPlayers>().expect("inserted");
    players.writes.clear();
    players.planes.clear();
    let ids = world.query_ids::<VideoPlayer>();
    let alive: std::collections::HashSet<ComponentId> = ids.iter().map(|(_, c)| *c).collect();
    players.playing.retain(|c, _| alive.contains(c));
    players.awake.retain(|c| alive.contains(c));
    let mut messages: Vec<(GameObjectId, &'static str)> = Vec::new();
    let mut audio_sync = Vec::new();
    for (id, cid) in ids {
        if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        let Some(p) = world.component_mut::<VideoPlayer>(id, cid) else {
            continue;
        };
        let mut cmds = std::mem::take(&mut p.commands);
        if players.awake.insert(cid) && p.play_on_awake {
            cmds.insert(0, VideoCommand::Play);
        }
        let p = p.clone();
        // Open on the first command that needs the clip.
        if !players.playing.contains_key(&cid)
            && cmds.iter().any(|c| {
                matches!(
                    c,
                    VideoCommand::Prepare | VideoCommand::Play | VideoCommand::Seek(_)
                )
            })
        {
            match open_player(world, id, &p) {
                Ok(pb) => {
                    players.playing.insert(cid, pb);
                    if p.render_mode == VideoRenderMode::MaterialOverride {
                        let r = p.target_material_renderer.unwrap_or(id);
                        override_material(world, r, id, &p.target_material_property);
                    }
                }
                Err(e) => {
                    oxide_core::debug_log_error!("VideoPlayer: {e}");
                    if let Some(c) = world.component_mut::<VideoPlayer>(id, cid) {
                        c.status.error = Some(e);
                    }
                    messages.push((id, "OnVideoError"));
                    continue;
                }
            }
        }
        let Some(pb) = players.playing.get_mut(&cid) else {
            continue;
        };
        for c in cmds {
            match c {
                VideoCommand::Prepare => {}
                VideoCommand::Play => {
                    pb.playing = true;
                    pb.paused = false;
                }
                VideoCommand::Pause => pb.paused = true,
                VideoCommand::Stop => {
                    pb.playing = false;
                    pb.paused = false;
                    pb.started = false;
                    pb.seek(0.0);
                    pb.shown = None;
                }
                VideoCommand::Seek(t) => pb.seek(t),
            }
        }
        let running = pb.playing && !pb.paused;
        let waiting = p.wait_for_first_frame && pb.shown.is_none();
        if running && !waiting {
            pb.time += dt * p.playback_speed.max(0.0) as f64;
        }
        let was_prepared = pb.shown.is_some();
        let new = match pb.pull(p.skip_on_drop) {
            Ok(n) => n,
            Err(e) => {
                oxide_core::debug_log_error!("VideoPlayer {}: {e}", pb.source_key);
                false
            }
        };
        if !p.skip_on_drop
            && let Some(f) = &pb.shown
        {
            // Without skipping, time can't run ahead of the frames shown.
            pb.time = pb.time.min(f.time + 1.0 / pb.info.frame_rate.max(1.0));
        }
        if !was_prepared && pb.shown.is_some() {
            messages.push((id, "OnVideoPrepared"));
        }
        if running && pb.shown.is_some() && !pb.started {
            pb.started = true;
            messages.push((id, "OnVideoStarted"));
        }
        // The end of the clip.
        if running && pb.ended && pb.next.is_none() && pb.time >= pb.info.length - 1e-6 {
            messages.push((id, "OnVideoLoopPointReached"));
            if p.is_looping {
                let over = pb.time - pb.info.length;
                pb.seek(over.max(0.0));
                pb.shown = None;
            } else {
                pb.playing = false;
                pb.started = false;
            }
        }
        if new && let Some(f) = &pb.shown {
            pb.serial += 1;
            players.writes.push(TextureWrite {
                target: AssetId::from_path(&texture_path(id)),
                width: pb.info.width,
                height: pb.info.height,
                rgba: f.rgba.clone(),
                serial: pb.serial,
            });
            if p.render_mode == VideoRenderMode::RenderTexture
                && let Some(rt) = &p.target_texture
                && let Some(t) = world.resource::<AssetServer>().render_textures.get(rt)
            {
                let (dw, dh) = (t.width.max(1), t.height.max(1));
                let px = fit_frame(
                    &f.rgba,
                    pb.info.width,
                    pb.info.height,
                    dw,
                    dh,
                    p.aspect_ratio,
                );
                players.writes.push(TextureWrite {
                    target: rt.id(),
                    width: dw,
                    height: dh,
                    rgba: Arc::new(px),
                    serial: pb.serial,
                });
            }
        }
        if matches!(
            p.render_mode,
            VideoRenderMode::CameraFarPlane | VideoRenderMode::CameraNearPlane
        ) && pb.shown.is_some()
        {
            let near = p.render_mode == VideoRenderMode::CameraNearPlane;
            players.planes.push((
                id,
                AssetId::from_path(&texture_path(id)),
                p.target_camera_alpha.clamp(0.0, 1.0),
                if near { -1.0 } else { 1.0 },
            ));
        }
        if let Some(clip) = &pb.audio {
            let (volume, at) = match p.audio_output_mode {
                VideoAudioOutputMode::AudioSource => {
                    let src = p.target_audio_source.unwrap_or(id);
                    let v = world
                        .get_component::<oxide_audio::AudioSource>(src)
                        .map_or(1.0, |a| if a.mute { 0.0 } else { a.volume });
                    let spatial = world
                        .get_component::<oxide_audio::AudioSource>(src)
                        .is_some_and(|a| a.spatial_blend >= 0.5);
                    (v, spatial.then(|| world.position(src)))
                }
                _ => (
                    if p.direct_audio_mute {
                        0.0
                    } else {
                        p.direct_audio_volume
                    },
                    None,
                ),
            };
            audio_sync.push((
                0x5649_4445u64 << 32 | (id.to_bits() & 0xffff_ffff),
                clip.clone(),
                pb.time,
                volume,
                p.playback_speed,
                p.is_looping,
                running && pb.shown.is_some(),
                at,
            ));
        }
        let status = VideoStatus {
            is_prepared: pb.shown.is_some(),
            is_playing: pb.playing && !pb.paused,
            is_paused: pb.paused,
            time: pb.time,
            frame: pb
                .shown
                .as_ref()
                .map_or(-1, |f| (f.time * pb.info.frame_rate).round() as i64),
            frame_count: pb.info.frame_count,
            frame_rate: pb.info.frame_rate,
            length: pb.info.length,
            width: pb.info.width,
            height: pb.info.height,
            error: None,
        };
        if let Some(c) = world.component_mut::<VideoPlayer>(id, cid) {
            c.status = status;
        }
    }
    world.resources.insert(players);
    // The soundtracks follow the video's time.
    if !audio_sync.is_empty()
        && let Some(mut audio) = world.resources.remove::<oxide_audio::Audio>()
    {
        let assets = world.resource::<AssetServer>();
        for (key, clip, time, volume, speed, looping, playing, at) in audio_sync {
            audio.sync_timed_voice(
                assets, key, &clip, time, volume, speed, looping, playing, at,
            );
        }
        world.resources.insert(audio);
    }
    for (id, m) in messages {
        world.send_message(id, m, oxide_core::reflect::Value::Null);
    }
}

/// This frame's video pixels and camera-plane videos, into the render frame.
pub fn extract(world: &World, writes: &mut Vec<TextureWrite>, particles: &mut Vec<ParticleBatch>) {
    let Some(players) = world.try_resource::<VideoPlayers>() else {
        return;
    };
    writes.extend(players.writes.iter().cloned());
    for &(id, tex, alpha, side) in &players.planes {
        let Some(p) = world.get_component::<VideoPlayer>(id) else {
            continue;
        };
        let cam = p.target_camera.or_else(|| {
            world
                .find_with_tag("MainCamera")
                .into_iter()
                .next()
                .or_else(|| world.query::<Camera>().map(|(c, _)| c).next())
        });
        let Some(cam) = cam else { continue };
        let Some(c) = world.get_component::<Camera>(cam) else {
            continue;
        };
        let aspect = world
            .try_resource::<oxide_scene::Screen>()
            .filter(|s| s.height > 0)
            .map_or(16.0 / 9.0, |s| s.width as f32 / s.height as f32);
        // Just inside the near plane, or just before the far plane.
        let d = if side < 0.0 {
            c.near_clip_plane * 1.05
        } else {
            c.far_clip_plane * 0.95
        };
        let half_h = match c.projection {
            oxide_scene::Projection::Perspective => d * (c.field_of_view.to_radians() * 0.5).tan(),
            oxide_scene::Projection::Orthographic => c.orthographic_size,
        };
        let (vw, vh) = (half_h * 2.0 * aspect, half_h * 2.0);
        let (fw, fh) = (p.status.width.max(1) as f32, p.status.height.max(1) as f32);
        let (w, h) = match p.aspect_ratio {
            VideoAspectRatio::Stretch => (vw, vh),
            VideoAspectRatio::FitVertically => (vh * fw / fh, vh),
            VideoAspectRatio::FitHorizontally => (vw, vw * fh / fw),
            VideoAspectRatio::FitOutside => {
                let k = (vw / fw).max(vh / fh);
                (fw * k, fh * k)
            }
            VideoAspectRatio::FitInside | VideoAspectRatio::NoScaling => {
                let k = (vw / fw).min(vh / fh);
                (fw * k, fh * k)
            }
        };
        let (pos, rot): (Vector3, Quaternion) = (world.position(cam), world.rotation(cam));
        let center = pos + rot * Vector3::Z * d;
        particles.push(ParticleBatch {
            texture: Some(tex),
            blend: ParticleBlend::Alpha,
            particles: vec![
                ParticleInstance::billboard(center.to_array(), w, [1.0, 1.0, 1.0, alpha], 0.0)
                    .with_aspect(h / w.max(1e-6)),
            ],
        });
    }
}

/// Registers the VideoClip asset type and the VideoPlayer component; returns the system.
pub fn install(world: &mut World) -> Box<dyn System> {
    world.registry.register::<VideoPlayer>("Video/Video Player");
    if let Some(a) = world.try_resource_mut::<AssetServer>() {
        a.types.register(
            oxide_assets::AssetTypeInfo::new("VideoClip", &["mp4", "m4v", "mov", "gif"])
                .color(120, 170, 230),
        );
    }
    Box::new(VideoSystem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fit_by_aspect_mode() {
        // A 2×1 red frame into a 4×4 target.
        let src = [255u8, 0, 0, 255, 255, 0, 0, 255];
        let inside = fit_frame(&src, 2, 1, 4, 4, VideoAspectRatio::FitInside);
        let red = |px: &[u8], x: u32, y: u32| px[((y * 4 + x) * 4) as usize] == 255;
        assert!(red(&inside, 0, 1) && red(&inside, 3, 2), "letterboxed band");
        assert!(!red(&inside, 0, 0) && !red(&inside, 0, 3), "black bars");
        let stretch = fit_frame(&src, 2, 1, 4, 4, VideoAspectRatio::Stretch);
        assert!((0..4).all(|y| red(&stretch, 0, y)));
        let outside = fit_frame(&src, 2, 1, 4, 4, VideoAspectRatio::FitOutside);
        assert!(red(&outside, 0, 0) && red(&outside, 3, 3), "fills, cropped");
    }
}
