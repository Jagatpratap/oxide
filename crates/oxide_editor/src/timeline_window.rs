//! The Timeline window: edits the Timeline played by the selected PlayableDirector.
//!
//! Track headers on the left (binding fields, mute, lock, record, groups), the time ruler
//! and clips on the right (move, trim, ease with ⌘-drag on an edge, overlap to blend, drag
//! between tracks), markers (Signal Emitters), and a properties panel for whatever is
//! selected. The playhead previews the Timeline on the scene in Edit mode and puts
//! everything back when preview ends; Record on an animation track keys whatever you move
//! into a recorded clip. Drop Animation Clips, Audio Clips or prefabs from the Project
//! window onto tracks, or GameObjects from the Hierarchy to bind them or make a track.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.timeline@1.8/manual/tl-window.html>

use crate::EditorState;
use crate::animation_window::{Snapshot, record, restore, snapshot};
use crate::hierarchy::HierarchyDrag;
use crate::inspector::{field_row, nicify};
use crate::project::AssetPayload;
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, pos2, vec2};
use oxide_assets::{AnimationClip, AssetServer, Handle};
use oxide_core::reflect::Reflect;
use oxide_engine::Engine;
use oxide_engine::timeline::{
    self, ClipExtrapolation, PlayState, PlayableBehaviours, PlayableDirector, SignalAsset,
    SignalEmitter, SignalReaction, SignalReceiver, TimelineAsset, TimelineClip, TrackAsset,
    TrackType, mix,
};
use oxide_scene::{ComponentId, GameObjectId, World};
use std::collections::HashMap;

const HEADER_W: f32 = 236.0;
const PROPS_W: f32 = 270.0;
const RULER_H: f32 = 22.0;
const MARKERS_H: f32 = 18.0;
const ROW_H: f32 = 46.0;
const GROUP_H: f32 = 24.0;
const EDGE: f32 = 6.0;

/// What is selected in the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Sel {
    #[default]
    None,
    Track(usize),
    Clip(usize, usize),
    /// A marker: (track, or `None` for the Timeline's own Markers row, index).
    Marker(Option<usize>, usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DragKind {
    Move,
    TrimStart,
    TrimEnd,
    EaseIn,
    EaseOut,
    Marker,
}

#[derive(Clone, Debug)]
struct Drag {
    kind: DragKind,
    sel: Sel,
    /// Timeline time under the pointer when the drag started.
    grab: f32,
    clip: TimelineClip,
}

/// Window state.
pub struct TimelineWindow {
    /// The Director being edited (follows the selection unless locked).
    pub director: Option<GameObjectId>,
    pub locked: bool,
    /// A Timeline opened from the Project window without a Director (no bindings).
    pub asset: Option<String>,
    pub time: f64,
    pub preview: bool,
    pub playing: bool,
    /// The bound objects as they were before preview (restored when it ends).
    backup: Vec<(GameObjectId, Snapshot)>,
    actives: HashMap<GameObjectId, bool>,
    /// Pixels per second, and the time at the left edge of the clip area.
    zoom: f32,
    scroll: f32,
    pub sel: Sel,
    drag: Option<Drag>,
    pub show_frames: bool,
    /// Record mode on this track (animation tracks).
    pub recording: Option<usize>,
    last: Snapshot,
    undo: Vec<TimelineAsset>,
    redo: Vec<TimelineAsset>,
    /// The Timeline before the drag or typing burst in progress (one undo step).
    pending: Option<(TimelineAsset, f64)>,
    /// Pointer time of the last right-click on a track (Add clip/marker there).
    menu_time: f32,
    /// Chosen in menus this frame; applied after the UI pass.
    pending_binding: Option<(String, Option<GameObjectId>)>,
    pending_reference: Option<(String, GameObjectId)>,
    /// A clip to open in the Animation window.
    open_clip: Option<String>,
    /// Match Content: set this clip's length to its source's.
    match_content: Option<(usize, usize)>,
    pub error: Option<String>,
}

impl Default for TimelineWindow {
    fn default() -> Self {
        Self {
            director: None,
            locked: false,
            asset: None,
            time: 0.0,
            preview: false,
            playing: false,
            backup: Vec::new(),
            actives: HashMap::new(),
            zoom: 120.0,
            scroll: -0.2,
            sel: Sel::None,
            drag: None,
            show_frames: true,
            recording: None,
            last: Snapshot::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
            menu_time: 0.0,
            pending_binding: None,
            pending_reference: None,
            open_clip: None,
            match_content: None,
            error: None,
        }
    }
}

fn track_color(t: TrackType) -> Color32 {
    match t {
        TrackType::Animation => Color32::from_rgb(80, 150, 230),
        TrackType::Activation => Color32::from_rgb(110, 200, 110),
        TrackType::Audio => Color32::from_rgb(240, 170, 70),
        TrackType::Signal => Color32::from_rgb(230, 210, 90),
        TrackType::Control => Color32::from_rgb(70, 200, 200),
        TrackType::Playable => Color32::from_rgb(200, 120, 220),
        TrackType::Group => Color32::from_gray(150),
    }
}

fn director_cid(w: &World, id: GameObjectId) -> Option<ComponentId> {
    w.query_ids::<PlayableDirector>()
        .into_iter()
        .find(|(o, _)| *o == id)
        .map(|(_, c)| c)
}

impl TimelineWindow {
    /// The Timeline asset path shown (the Director's, or one opened directly).
    pub fn asset_path(&self, w: &World) -> Option<String> {
        if let Some(d) = self
            .director
            .and_then(|id| w.get_component::<PlayableDirector>(id))
        {
            return d.playable_asset.as_ref().map(|h| h.path().to_owned());
        }
        self.asset.clone()
    }

    /// Opens a `.playable` from the Project window: the selected Director's own Timeline is
    /// edited with its bindings, any other Timeline without them.
    pub fn open_asset(&mut self, w: &World, path: &str) {
        if self
            .director
            .and_then(|id| w.get_component::<PlayableDirector>(id))
            .and_then(|d| d.playable_asset.as_ref())
            .is_some_and(|h| h.path() == path)
        {
            return;
        }
        self.director = w
            .query::<PlayableDirector>()
            .find(|(_, d)| d.playable_asset.as_ref().is_some_and(|h| h.path() == path))
            .map(|(id, _)| id);
        self.asset = Some(path.to_owned());
        self.locked = self.director.is_some();
        self.sel = Sel::None;
    }

    fn snap(&self, t: f32, fps: f32) -> f32 {
        if fps > 0.0 {
            (t * fps).round() / fps
        } else {
            t
        }
    }

    /// The window's state for MCP.
    pub fn state_json(&self, w: &mut World) -> serde_json::Value {
        let path = self.asset_path(w);
        let tl = path.as_deref().and_then(|p| load(w, p));
        serde_json::json!({
            "director": self.director.map(|d| d.to_bits()),
            "asset": path,
            "time": self.time,
            "preview": self.preview,
            "playing": self.playing,
            "recording": self.recording,
            "selection": format!("{:?}", self.sel),
            "duration": tl.as_ref().map(|t| t.duration()),
            "tracks": tl.map(|t| t.tracks.iter().map(|tr| serde_json::json!({
                "name": tr.name,
                "type": format!("{:?}", tr.track_type),
                "group": tr.group,
                "muted": tr.muted,
                "locked": tr.locked,
                "clips": tr.clips.iter().map(|c| serde_json::json!({
                    "name": c.display_name, "start": c.start, "duration": c.duration,
                    "ease_in": c.ease_in_duration, "ease_out": c.ease_out_duration,
                })).collect::<Vec<_>>(),
                "markers": tr.markers.iter().map(|m| m.time).collect::<Vec<_>>(),
            })).collect::<Vec<_>>()),
            "undo_steps": self.undo.len(),
        })
    }
}

/// A copy of the Timeline at `path`.
pub fn load(w: &mut World, path: &str) -> Option<TimelineAsset> {
    let h: Handle<TimelineAsset> = Handle::from_path(path);
    w.try_resource_mut::<AssetServer>()?.data_asset(&h).cloned()
}

/// Writes the Timeline to its file (and the loaded copy).
pub fn save(w: &mut World, path: &str, tl: &TimelineAsset) -> Result<(), String> {
    w.resource_mut::<AssetServer>()
        .save_data(path, Box::new(tl.clone()))
}

// ---- preview ------------------------------------------------------------------------------

/// Objects a Timeline touches (bound objects, exposed references, the Director).
fn touched(w: &World, id: GameObjectId) -> Vec<GameObjectId> {
    let Some(d) = w.get_component::<PlayableDirector>(id) else {
        return vec![];
    };
    let mut out: Vec<GameObjectId> = d
        .bindings
        .iter()
        .filter_map(|b| b.object)
        .chain(d.exposed_references.iter().filter_map(|r| r.object))
        .filter(|o| w.is_alive(*o))
        .collect();
    out.sort_by_key(|o| o.to_bits());
    out.dedup();
    out
}

fn begin_preview(win: &mut TimelineWindow, w: &mut World) {
    let Some(id) = win.director.filter(|d| w.is_alive(*d)) else {
        return;
    };
    if win.preview {
        return;
    }
    win.backup = touched(w, id)
        .into_iter()
        .map(|o| (o, snapshot(w, o)))
        .collect();
    win.actives = touched(w, id)
        .into_iter()
        .flat_map(|o| w.descendants(o))
        .filter_map(|o| Some((o, w.get(o)?.active_self())))
        .collect();
    win.preview = true;
}

fn end_preview(win: &mut TimelineWindow, w: &mut World) {
    if !win.preview {
        return;
    }
    if let Some(id) = win.director.filter(|d| w.is_alive(*d))
        && let Some(cid) = director_cid(w, id)
    {
        timeline::release(w, id, cid);
    }
    for (o, s) in std::mem::take(&mut win.backup) {
        if w.is_alive(o) {
            restore(w, &s);
        }
    }
    for (o, a) in std::mem::take(&mut win.actives) {
        if w.is_alive(o) && w.get(o).is_some_and(|x| x.active_self() != a) {
            w.set_active(o, a);
        }
    }
    win.preview = false;
    win.playing = false;
    win.recording = None;
}

/// Called every frame: preview ends when Play starts, the window closes or the Director
/// goes away; in Play mode the playhead follows the running Director.
pub fn tick(st: &mut EditorState, engine: &mut Engine, open: bool) {
    let w = &mut engine.world;
    let win = &mut st.timeline;
    let edit = st.play == crate::PlayState::Edit;
    if win.preview && (!open || !edit || !win.director.is_some_and(|d| w.is_alive(d))) {
        end_preview(win, w);
    }
    if !edit
        && let Some(d) = win
            .director
            .and_then(|id| w.get_component::<PlayableDirector>(id))
    {
        win.time = d.time();
    }
}

/// MCP control of the window.
#[allow(clippy::too_many_arguments)]
pub fn control(
    st: &mut EditorState,
    engine: &mut Engine,
    director: Option<GameObjectId>,
    time: Option<f64>,
    preview: Option<bool>,
    playing: Option<bool>,
    record: Option<Option<usize>>,
) -> Result<(), String> {
    let w = &mut engine.world;
    let win = &mut st.timeline;
    if let Some(d) = director {
        if w.get_component::<PlayableDirector>(d).is_none() {
            return Err("that object has no PlayableDirector".into());
        }
        if win.director != Some(d) {
            end_preview(win, w);
        }
        win.director = Some(d);
        win.locked = true;
    }
    if let Some(t) = time {
        win.time = t.max(0.0);
        if st.play == crate::PlayState::Edit {
            begin_preview(win, w);
        }
    }
    if let Some(p) = preview {
        if p {
            begin_preview(win, w);
        } else {
            end_preview(win, w);
        }
    }
    if let Some(p) = playing {
        if p {
            begin_preview(win, w);
        }
        win.playing = p && win.preview;
    }
    if let Some(r) = record {
        if r.is_some() {
            begin_preview(win, w);
        }
        win.recording = r.filter(|_| win.preview);
    }
    if win.preview
        && let Some(id) = win.director
    {
        timeline::preview(w, id, win.time, false, 0.0);
        win.last = record_base(w, id, win.recording);
    }
    Ok(())
}

/// The bound object of animation track `track` as the preview left it (Record compares
/// against this).
fn record_base(w: &mut World, id: GameObjectId, track: Option<usize>) -> Snapshot {
    let Some(ti) = track else {
        return Snapshot::default();
    };
    let Some(path) = w
        .get_component::<PlayableDirector>(id)
        .and_then(|d| d.playable_asset.as_ref().map(|h| h.path().to_owned()))
    else {
        return Snapshot::default();
    };
    let Some(name) = load(w, &path).and_then(|t| t.tracks.get(ti).map(|t| t.name.clone())) else {
        return Snapshot::default();
    };
    match w
        .get_component::<PlayableDirector>(id)
        .and_then(|d| d.binding(&name))
    {
        Some(o) => snapshot(w, o),
        None => Snapshot::default(),
    }
}

// ---- edits --------------------------------------------------------------------------------

/// Adds a track (MCP and the Add menu). Returns its index.
pub fn add_track(tl: &mut TimelineAsset, t: TrackType, group: &str) -> usize {
    let base = match t {
        TrackType::Group => "Track Group",
        other => other.label(),
    };
    let mut track = TrackAsset::new(&tl.unique_track_name(base), t);
    track.group = group.to_owned();
    tl.tracks.push(track);
    tl.tracks.len() - 1
}

/// A new clip of `track`'s kind at `start`, for `source` (an asset path or object name).
pub fn new_clip(
    assets: &AssetServer,
    track: &TrackAsset,
    start: f32,
    source: &str,
) -> TimelineClip {
    let mut c = TimelineClip {
        start: start.max(0.0),
        ..Default::default()
    };
    let name = source.rsplit(['/', '#']).next().unwrap_or(source);
    let name = name.rsplit_once('.').map_or(name, |(n, _)| n).to_owned();
    match track.track_type {
        TrackType::Animation => {
            let h: Handle<AnimationClip> = Handle::from_path(source);
            if let Some(a) = assets.animations.get(&h) {
                c.duration = a.length.max(1.0 / 60.0);
            }
            c.animation_clip = (!source.is_empty()).then_some(h);
            c.display_name = name;
        }
        TrackType::Audio => {
            c.audio_clip = (!source.is_empty()).then(|| Handle::from_path(source));
            c.duration = audio_length(assets, source).unwrap_or(2.0);
            c.display_name = name;
        }
        TrackType::Activation => c.display_name = "Active".into(),
        TrackType::Control => {
            if source.ends_with(".prefab") {
                c.prefab = source.to_owned();
            } else {
                c.source_game_object = source.to_owned();
            }
            c.duration = 2.0;
            c.display_name = name;
        }
        TrackType::Playable => {
            c.behaviour = source.to_owned();
            c.display_name = source.to_owned();
        }
        TrackType::Signal | TrackType::Group => {}
    }
    if track.track_type == TrackType::Animation {
        c.post_extrapolation = ClipExtrapolation::Hold;
    }
    c
}

/// The length of a decoded WAV, if the header says (other formats default to 2 s).
fn audio_length(assets: &AssetServer, path: &str) -> Option<f32> {
    let h: Handle<oxide_assets::AudioClip> = Handle::from_path(path);
    let b = &assets.audio.get(&h)?.bytes;
    if b.len() < 44 || &b[0..4] != b"RIFF" {
        return None;
    }
    let bytes_per_sec = u32::from_le_bytes(b[28..32].try_into().ok()?) as f32;
    // Find the data chunk.
    let mut i = 12;
    while i + 8 <= b.len() {
        let size = u32::from_le_bytes(b[i + 4..i + 8].try_into().ok()?) as usize;
        if &b[i..i + 4] == b"data" {
            return Some(size as f32 / bytes_per_sec.max(1.0));
        }
        i += 8 + size;
    }
    None
}

/// Signal tracks keep markers only; markers on any track go to its binding's receiver.
fn add_marker(tl: &mut TimelineAsset, track: Option<usize>, time: f32) -> Sel {
    let m = SignalEmitter {
        time: time.max(0.0),
        ..Default::default()
    };
    match track.and_then(|t| tl.tracks.get_mut(t)) {
        Some(t) => {
            t.markers.push(m);
            Sel::Marker(track, t.markers.len() - 1)
        }
        None => {
            tl.markers.push(m);
            Sel::Marker(None, tl.markers.len() - 1)
        }
    }
}

fn delete_sel(tl: &mut TimelineAsset, sel: Sel) {
    match sel {
        Sel::Track(t) if t < tl.tracks.len() => {
            let name = tl.tracks[t].name.clone();
            let removed = tl.tracks.remove(t);
            if removed.track_type == TrackType::Group {
                // Its tracks go with it (Unity deletes the group's contents).
                let mut doomed = vec![name];
                while let Some(g) = doomed.pop() {
                    let inner: Vec<String> = tl
                        .tracks
                        .iter()
                        .filter(|x| x.group == g)
                        .map(|x| x.name.clone())
                        .collect();
                    tl.tracks.retain(|x| x.group != g);
                    doomed.extend(inner);
                }
            }
        }
        Sel::Clip(t, c) => {
            if let Some(tr) = tl.tracks.get_mut(t)
                && c < tr.clips.len()
            {
                tr.clips.remove(c);
            }
        }
        Sel::Marker(Some(t), m) => {
            if let Some(tr) = tl.tracks.get_mut(t)
                && m < tr.markers.len()
            {
                tr.markers.remove(m);
            }
        }
        Sel::Marker(None, m) if m < tl.markers.len() => {
            tl.markers.remove(m);
        }
        _ => {}
    }
}

fn duplicate_sel(tl: &mut TimelineAsset, sel: Sel) -> Sel {
    match sel {
        Sel::Clip(t, c) => {
            let Some(tr) = tl.tracks.get_mut(t) else {
                return sel;
            };
            let Some(mut n) = tr.clips.get(c).cloned() else {
                return sel;
            };
            n.start = tr.clips.iter().map(TimelineClip::end).fold(0.0, f32::max);
            tr.clips.push(n);
            Sel::Clip(t, tr.clips.len() - 1)
        }
        Sel::Track(t) => {
            let Some(mut n) = tl.tracks.get(t).cloned() else {
                return sel;
            };
            n.name = tl.unique_track_name(&n.name);
            tl.tracks.insert(t + 1, n);
            Sel::Track(t + 1)
        }
        _ => sel,
    }
}

/// Makes a Timeline for `obj` (the window's Create button): a `.playable` next to the
/// scene's assets, a PlayableDirector on the object, and an Animation track bound to it.
pub fn create_for(w: &mut World, obj: GameObjectId) -> Result<String, String> {
    let name = w.get(obj).map(|o| o.name.clone()).ok_or("no object")?;
    let assets = w.resource_mut::<AssetServer>();
    let base = format!("Assets/{name}Timeline");
    let mut path = format!("{base}.playable");
    let mut i = 1;
    while assets.exists(&path) {
        path = format!("{base} ({i}).playable");
        i += 1;
    }
    let mut tl = TimelineAsset::default();
    add_track(&mut tl, TrackType::Animation, "");
    assets.save_data(&path, Box::new(tl.clone()))?;
    let mut d = PlayableDirector::new(&path);
    d.set_binding(&tl.tracks[0].name, Some(obj));
    w.add_component(obj, d);
    Ok(path)
}

// ---- the window ----------------------------------------------------------------------------

/// Track rows in display order with their heights (hidden rows left out).
fn rows(tl: &TimelineAsset) -> Vec<(usize, f32)> {
    tl.display_order()
        .into_iter()
        .filter(|&i| !tl.is_hidden(&tl.tracks[i]))
        .map(|i| {
            let h = if tl.tracks[i].track_type == TrackType::Group {
                GROUP_H
            } else {
                ROW_H
            };
            (i, h)
        })
        .collect()
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let in_edit = st.play == crate::PlayState::Edit;
    // Follow the selected Director unless locked.
    if !st.timeline.locked
        && let Some(&sel) = st.selection.last()
        && engine
            .world
            .get_component::<PlayableDirector>(sel)
            .is_some()
        && st.timeline.director != Some(sel)
    {
        end_preview(&mut st.timeline, &mut engine.world);
        st.timeline.director = Some(sel);
        st.timeline.asset = None;
        st.timeline.sel = Sel::None;
        st.timeline.undo.clear();
        st.timeline.redo.clear();
    }
    if let Some(d) = st.timeline.director
        && !engine.world.is_alive(d)
    {
        st.timeline.director = None;
    }
    let Some(path) = st.timeline.asset_path(&engine.world) else {
        empty_state(st, engine, ui);
        return;
    };
    let Some(mut tl) = load(&mut engine.world, &path) else {
        ui.label(format!("Cannot load {path}"));
        return;
    };
    let before = tl.clone();
    let win = &mut st.timeline;
    let fps = tl.frame_rate.max(1.0);

    // Undo / redo (the window's own history) and shortcuts while hovered.
    let hovered = ui.rect_contains_pointer(ui.max_rect());
    let typing = ui.ctx().egui_wants_keyboard_input();
    if hovered && !typing {
        crate::shortcuts::claim_keys(ui.ctx());
        let keys = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        for (key, m) in keys {
            match key {
                egui::Key::Z if m.command && m.shift => {
                    if let Some(n) = win.redo.pop() {
                        win.undo.push(tl.clone());
                        tl = n;
                    }
                }
                egui::Key::Z if m.command => {
                    if let Some(n) = win.undo.pop() {
                        win.redo.push(tl.clone());
                        tl = n;
                    }
                }
                egui::Key::D if m.command => win.sel = duplicate_sel(&mut tl, win.sel),
                egui::Key::Delete | egui::Key::Backspace => {
                    delete_sel(&mut tl, win.sel);
                    win.sel = Sel::None;
                }
                egui::Key::Space => {
                    if in_edit {
                        begin_preview(win, &mut engine.world);
                        win.playing = !win.playing && win.preview;
                    }
                }
                egui::Key::Comma => win.time = (win.time - 1.0 / f64::from(fps)).max(0.0),
                egui::Key::Period => win.time += 1.0 / f64::from(fps),
                _ => {}
            }
        }
    }
    let undo_base = tl.clone();

    // Toolbar.
    let director = win.director;
    let duration = tl.duration();
    let mut scrubbed = false;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                in_edit && director.is_some(),
                egui::Button::new("Preview").selected(win.preview),
            )
            .on_hover_text("Preview the Timeline on the scene (Edit mode)")
            .clicked()
        {
            if win.preview {
                end_preview(win, &mut engine.world);
            } else {
                begin_preview(win, &mut engine.world);
                scrubbed = true;
            }
        }
        ui.separator();
        let step = 1.0 / f64::from(fps);
        if ui
            .button("⏮")
            .on_hover_text("Go to the beginning")
            .clicked()
        {
            win.time = 0.0;
            scrubbed = true;
        }
        if ui.button("◀").on_hover_text("Previous frame (,)").clicked() {
            win.time = (win.time - step).max(0.0);
            scrubbed = true;
        }
        let play_label = if win.playing { "❚❚" } else { "▶" };
        if ui
            .add_enabled(director.is_some(), egui::Button::new(play_label))
            .on_hover_text("Play (Space)")
            .clicked()
        {
            if in_edit {
                begin_preview(win, &mut engine.world);
                win.playing = !win.playing && win.preview;
            } else if let Some(d) =
                director.and_then(|id| engine.world.get_component_mut::<PlayableDirector>(id))
            {
                match d.state() {
                    PlayState::Playing => d.pause(),
                    PlayState::Paused => d.resume(),
                    PlayState::Stopped => d.play(),
                }
            }
        }
        if ui.button("▶").on_hover_text("Next frame (.)").clicked() {
            win.time += step;
            scrubbed = true;
        }
        if ui.button("⏭").on_hover_text("Go to the end").clicked() {
            win.time = f64::from(duration);
            scrubbed = true;
        }
        let mut shown = if win.show_frames {
            win.time * f64::from(fps)
        } else {
            win.time
        };
        let speed = if win.show_frames { 1.0 } else { 0.01 };
        if ui
            .add(
                egui::DragValue::new(&mut shown)
                    .speed(speed)
                    .range(0.0..=1e6)
                    .max_decimals(if win.show_frames { 0 } else { 3 }),
            )
            .changed()
        {
            win.time = if win.show_frames {
                shown / f64::from(fps)
            } else {
                shown
            };
            scrubbed = true;
        }
        ui.separator();
        ui.menu_button("+ Add", |ui| {
            for t in TrackType::ALL {
                if ui.button(t.label()).clicked() {
                    let group = match win.sel {
                        Sel::Track(g)
                            if tl
                                .tracks
                                .get(g)
                                .is_some_and(|x| x.track_type == TrackType::Group) =>
                        {
                            tl.tracks[g].name.clone()
                        }
                        _ => String::new(),
                    };
                    let i = add_track(&mut tl, t, &group);
                    win.sel = Sel::Track(i);
                    ui.close();
                }
            }
        });
        ui.separator();
        ui.label(egui::RichText::new(path.rsplit('/').next().unwrap_or(&path)).strong());
        if director.is_none() {
            ui.weak("(no Director: bindings and preview unavailable)");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::theme::lock_button(ui, win.locked)
                .on_hover_text("Lock: keep editing this Director when the selection changes")
                .clicked()
            {
                win.locked = !win.locked;
            }
            ui.menu_button("⚙", |ui| {
                ui.checkbox(&mut win.show_frames, "Frames (off: seconds)");
                ui.horizontal(|ui| {
                    ui.label("Frame Rate");
                    ui.add(egui::DragValue::new(&mut tl.frame_rate).range(1.0..=240.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Duration Mode");
                    egui::ComboBox::from_id_salt("tl_dur")
                        .selected_text(format!("{:?}", tl.duration_mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut tl.duration_mode,
                                timeline::DurationMode::BasedOnClips,
                                "Based On Clips",
                            );
                            ui.selectable_value(
                                &mut tl.duration_mode,
                                timeline::DurationMode::FixedLength,
                                "Fixed Length",
                            );
                        });
                });
                if tl.duration_mode == timeline::DurationMode::FixedLength {
                    ui.horizontal(|ui| {
                        ui.label("Duration (s)");
                        ui.add(
                            egui::DragValue::new(&mut tl.fixed_duration)
                                .speed(0.05)
                                .range(0.0..=36000.0),
                        );
                    });
                }
            });
            if ui.small_button("Frame All").clicked() {
                let span = duration.max(1.0);
                win.scroll = -span * 0.03;
                let width = (ui.max_rect().width() - HEADER_W - PROPS_W).max(100.0);
                win.zoom = width / (span * 1.08);
            }
        });
    });
    if let Some(e) = &win.error {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
    }

    // Layout: headers | clips | properties.
    let full = ui.available_rect_before_wrap();
    let props_rect = Rect::from_min_max(pos2(full.right() - PROPS_W, full.top()), full.max);
    let main = Rect::from_min_max(full.min, pos2(props_rect.left() - 4.0, full.bottom()));
    let header_rect = Rect::from_min_max(main.min, pos2(main.left() + HEADER_W, main.bottom()));
    let area = Rect::from_min_max(pos2(header_rect.right() + 1.0, main.top()), main.max);
    ui.allocate_rect(full, Sense::hover());
    let painter = ui.painter_at(main);
    painter.rect_filled(main, 0.0, Color32::from_gray(38));
    painter.rect_filled(header_rect, 0.0, Color32::from_gray(46));

    let t2x = |t: f32, z: f32, s: f32| area.left() + (t - s) * z;
    let x2t = |x: f32, z: f32, s: f32| (x - area.left()) / z + s;

    // Zoom (⌘/Ctrl + scroll) and pan (scroll) over the clip area.
    if ui.rect_contains_pointer(area) {
        let (scroll, zoom_d, pointer) =
            ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
        if zoom_d != 1.0 {
            if let Some(p) = pointer {
                let at = x2t(p.x, win.zoom, win.scroll);
                win.zoom = (win.zoom * zoom_d).clamp(4.0, 4000.0);
                win.scroll = at - (p.x - area.left()) / win.zoom;
            }
        } else if scroll.x != 0.0 || scroll.y != 0.0 {
            win.scroll -= (scroll.x + scroll.y) / win.zoom;
        }
        win.scroll = win.scroll.max(-1.0);
    }
    let (zoom, scr) = (win.zoom, win.scroll);

    // Ruler.
    let ruler = Rect::from_min_size(area.min, vec2(area.width(), RULER_H));
    painter.rect_filled(ruler, 0.0, Color32::from_gray(30));
    let px_per_frame = zoom / fps;
    let label_every_s = [
        1.0 / fps,
        5.0 / fps,
        10.0 / fps,
        0.5,
        1.0,
        2.0,
        5.0,
        10.0,
        30.0,
        60.0,
    ]
    .into_iter()
    .find(|s| s * zoom >= 60.0)
    .unwrap_or(120.0);
    let start_t = x2t(area.left(), zoom, scr).max(0.0);
    let end_t = x2t(area.right(), zoom, scr);
    let mut t = (start_t / label_every_s).floor() * label_every_s;
    while t <= end_t {
        let x = t2x(t, zoom, scr);
        if x >= area.left() {
            painter.line_segment(
                [pos2(x, ruler.top() + 8.0), pos2(x, ruler.bottom())],
                Stroke::new(1.0, Color32::from_gray(110)),
            );
            painter.line_segment(
                [pos2(x, ruler.bottom()), pos2(x, area.bottom())],
                Stroke::new(1.0, Color32::from_gray(48)),
            );
            let text = if win.show_frames {
                format!("{}", (t * fps).round() as i64)
            } else {
                format!("{t:.2}")
            };
            painter.text(
                pos2(x + 2.0, ruler.top() + 1.0),
                Align2::LEFT_TOP,
                text,
                FontId::proportional(10.0),
                Color32::from_gray(170),
            );
        }
        if px_per_frame > 6.0 {
            let mut f = t;
            let step = 1.0 / fps;
            while f < t + label_every_s - step * 0.5 {
                f += step;
                let fx = t2x(f, zoom, scr);
                painter.line_segment(
                    [pos2(fx, ruler.bottom() - 4.0), pos2(fx, ruler.bottom())],
                    Stroke::new(1.0, Color32::from_gray(80)),
                );
            }
        }
        t += label_every_s;
    }
    // End of the Timeline.
    let ex = t2x(duration, zoom, scr);
    painter.line_segment(
        [pos2(ex, ruler.bottom()), pos2(ex, area.bottom())],
        Stroke::new(1.0, Color32::from_rgb(70, 110, 200)),
    );

    let ruler_resp = ui.interact(ruler, ui.id().with("tl_ruler"), Sense::click_and_drag());
    if (ruler_resp.dragged() || ruler_resp.clicked())
        && let Some(p) = ruler_resp.interact_pointer_pos()
    {
        win.time = f64::from(win.snap(x2t(p.x, zoom, scr), fps).max(0.0));
        win.playing = false;
        scrubbed = true;
    }

    // Markers row (the Timeline's own markers).
    let mrow = Rect::from_min_size(
        pos2(area.left(), ruler.bottom()),
        vec2(area.width(), MARKERS_H),
    );
    painter.rect_filled(mrow, 0.0, Color32::from_gray(34));
    painter.text(
        pos2(header_rect.left() + 8.0, mrow.center().y),
        Align2::LEFT_CENTER,
        "Markers",
        FontId::proportional(11.0),
        Color32::from_gray(150),
    );
    let mresp = ui.interact(mrow, ui.id().with("tl_mrow"), Sense::click());
    mresp.context_menu(|ui| {
        if ui.button("Add Signal Emitter").clicked() {
            win.sel = add_marker(&mut tl, None, win.menu_time);
            ui.close();
        }
    });
    if mresp.secondary_clicked()
        && let Some(p) = mresp.interact_pointer_pos()
    {
        win.menu_time = win.snap(x2t(p.x, zoom, scr), fps);
    }
    let markers: Vec<SignalEmitter> = tl.markers.clone();
    for (mi, m) in markers.iter().enumerate() {
        marker_ui(
            ui, win, &mut tl, &painter, None, mi, m, mrow, zoom, scr, fps, area,
        );
    }

    // Track rows.
    let mut y = mrow.bottom() + 2.0;
    let row_list = rows(&tl);
    let all_rows = row_rects_snapshot(&row_list, mrow.bottom() + 2.0, area);
    let mut header_changes: Vec<Change> = Vec::new();
    let director_comp =
        director.and_then(|id| engine.world.get_component::<PlayableDirector>(id).cloned());
    for &(ti, h) in &row_list {
        let track = tl.tracks[ti].clone();
        let depth = tl.depth(&track) as f32;
        let hr = Rect::from_min_size(pos2(header_rect.left(), y), vec2(HEADER_W, h - 2.0));
        let rr = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), h - 2.0));
        let selected = win.sel == Sel::Track(ti);
        let muted = tl.is_muted(&track);
        painter.rect_filled(
            hr,
            2.0,
            if selected {
                Color32::from_rgb(56, 76, 110)
            } else {
                Color32::from_gray(58)
            },
        );
        painter.rect_filled(
            rr,
            0.0,
            if track.track_type == TrackType::Group {
                Color32::from_gray(44)
            } else {
                Color32::from_gray(40)
            },
        );
        let bar = Rect::from_min_size(
            pos2(hr.left() + depth * 10.0, hr.top()),
            vec2(4.0, hr.height()),
        );
        painter.rect_filled(bar, 1.0, track_color(track.track_type));

        // Header contents.
        let inner = Rect::from_min_max(
            pos2(bar.right() + 4.0, hr.top() + 2.0),
            pos2(hr.right() - 4.0, hr.bottom() - 2.0),
        );
        let line1 = Rect::from_min_max(inner.min, pos2(inner.right(), inner.top() + 20.0));
        let hresp = ui.interact(hr, ui.id().with(("tl_head", ti)), Sense::click());
        if hresp.clicked() {
            win.sel = Sel::Track(ti);
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(if h >= ROW_H { line1 } else { inner })
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 3.0;
        if track.track_type == TrackType::Group {
            let arrow = if track.collapsed { "⏵" } else { "⏷" };
            if child.small_button(arrow).clicked() {
                header_changes.push(Box::new(move |tl, _| {
                    tl.tracks[ti].collapsed = !tl.tracks[ti].collapsed
                }));
            }
        }
        let name_color = if muted {
            Color32::from_gray(120)
        } else {
            Color32::from_gray(220)
        };
        child.add(
            egui::Label::new(
                egui::RichText::new(&track.name)
                    .color(name_color)
                    .size(11.5),
            )
            .truncate()
            .selectable(false),
        );
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::theme::lock_button(ui, track.locked)
                .on_hover_text("Lock")
                .clicked()
            {
                header_changes.push(Box::new(move |tl, _| {
                    tl.tracks[ti].locked = !tl.tracks[ti].locked
                }));
            }
            if ui
                .selectable_label(track.muted, "M")
                .on_hover_text("Mute")
                .clicked()
            {
                header_changes.push(Box::new(move |tl, _| {
                    tl.tracks[ti].muted = !tl.tracks[ti].muted
                }));
            }
            if track.track_type == TrackType::Animation && director.is_some() {
                let rec = win.recording == Some(ti);
                let b = egui::Button::new(egui::RichText::new("⏺").color(if rec {
                    Color32::from_rgb(240, 70, 70)
                } else {
                    Color32::from_gray(160)
                }))
                .selected(rec);
                if ui
                    .add_enabled(in_edit, b)
                    .on_hover_text("Record: keys what you change into a clip")
                    .clicked()
                {
                    header_changes.push(Box::new(move |_, w| {
                        w.recording = if rec { None } else { Some(ti) };
                    }));
                }
            }
        });
        // Binding field (second line).
        if track.track_type.has_binding()
            && let Some(dc) = &director_comp
            && h >= ROW_H
        {
            let br = Rect::from_min_max(
                pos2(inner.left(), line1.bottom() + 1.0),
                pos2(inner.right(), inner.bottom()),
            );
            let bound = dc.binding(&track.name);
            let label = bound
                .and_then(|b| engine.world.get(b).map(|o| o.name.clone()))
                .unwrap_or_else(|| "None (GameObject)".into());
            let mut bui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(br)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let r = egui::ComboBox::from_id_salt(("tl_bind", ti))
                .selected_text(egui::RichText::new(label).size(10.5))
                .width(br.width() - 4.0)
                .height(300.0)
                .show_ui(&mut bui, |ui| {
                    if ui.selectable_label(bound.is_none(), "None").clicked() {
                        bind_later(&mut header_changes, &track.name, None);
                    }
                    for o in engine.world.hierarchy_order().into_iter().take(2000) {
                        if ui
                            .selectable_label(bound == Some(o), engine.world.path_of(o))
                            .clicked()
                        {
                            bind_later(&mut header_changes, &track.name, Some(o));
                        }
                    }
                });
            if let Some(p) = r.response.dnd_release_payload::<HierarchyDrag>()
                && let Some(&o) = p.0.first()
            {
                bind_later(&mut header_changes, &track.name, Some(o));
            }
        }
        hresp.context_menu(|ui| {
            if ui.button("Duplicate").clicked() {
                header_changes.push(Box::new(move |tl, w| {
                    w.sel = duplicate_sel(tl, Sel::Track(ti))
                }));
                ui.close();
            }
            if ui.button("Delete").clicked() {
                header_changes.push(Box::new(move |tl, w| {
                    delete_sel(tl, Sel::Track(ti));
                    w.sel = Sel::None;
                }));
                ui.close();
            }
            ui.separator();
            if track.track_type == TrackType::Group {
                ui.menu_button("Add Track", |ui| {
                    for t in TrackType::ALL {
                        if ui.button(t.label()).clicked() {
                            let g = track.name.clone();
                            header_changes.push(Box::new(move |tl, w| {
                                w.sel = Sel::Track(add_track(tl, t, &g))
                            }));
                            ui.close();
                        }
                    }
                });
            }
            let groups: Vec<String> = tl
                .tracks
                .iter()
                .filter(|g| g.track_type == TrackType::Group && g.name != track.name)
                .map(|g| g.name.clone())
                .collect();
            ui.menu_button("Move to Group", |ui| {
                if ui.button("(Top level)").clicked() {
                    header_changes.push(Box::new(move |tl, _| tl.tracks[ti].group.clear()));
                    ui.close();
                }
                for g in groups {
                    if ui.button(&g).clicked() {
                        header_changes.push(Box::new(move |tl, _| tl.tracks[ti].group = g));
                        ui.close();
                    }
                }
            });
            if ui.button("Move Up").clicked() && ti > 0 {
                header_changes.push(Box::new(move |tl, w| {
                    tl.tracks.swap(ti, ti - 1);
                    w.sel = Sel::Track(ti - 1);
                }));
                ui.close();
            }
            if ui.button("Move Down").clicked() {
                header_changes.push(Box::new(move |tl, w| {
                    if ti + 1 < tl.tracks.len() {
                        tl.tracks.swap(ti, ti + 1);
                        w.sel = Sel::Track(ti + 1);
                    }
                }));
                ui.close();
            }
        });
        // Drops on the header: GameObjects bind.
        if let Some(p) = hresp.dnd_release_payload::<HierarchyDrag>()
            && let Some(&o) = p.0.first()
            && track.track_type.has_binding()
        {
            bind_later(&mut header_changes, &track.name, Some(o));
        }

        // Track body: right-click menu, drops, clips, markers.
        if track.track_type != TrackType::Group {
            let body = ui.interact(rr, ui.id().with(("tl_body", ti)), Sense::click());
            if body.clicked() {
                win.sel = Sel::Track(ti);
            }
            if body.secondary_clicked()
                && let Some(p) = body.interact_pointer_pos()
            {
                win.menu_time = win.snap(x2t(p.x, zoom, scr), fps).max(0.0);
            }
            let menu_t = win.menu_time;
            let assets = engine.world.resource::<AssetServer>();
            body.context_menu(|ui| {
                track_menu(
                    ui,
                    &mut header_changes,
                    assets,
                    &engine.world,
                    &track,
                    ti,
                    menu_t,
                )
            });
            if let Some(p) = body.dnd_release_payload::<AssetPayload>()
                && let Some(pos) = ui.ctx().pointer_interact_pos()
                && !track.locked
            {
                let t = win.snap(x2t(pos.x, zoom, scr), fps).max(0.0);
                let ok = match track.track_type {
                    TrackType::Animation => {
                        is_type(assets, &p.0, "AnimationClip") || p.0.contains('#')
                    }
                    TrackType::Audio => is_type(assets, &p.0, "AudioClip"),
                    TrackType::Control => p.0.ends_with(".prefab"),
                    _ => false,
                };
                if ok {
                    let c = new_clip(assets, &track, t, &p.0);
                    header_changes.push(Box::new(move |tl, w| {
                        tl.tracks[ti].clips.push(c);
                        w.sel = Sel::Clip(ti, tl.tracks[ti].clips.len() - 1);
                    }));
                }
            }
            // Clips.
            let blends = mix::blends(&track.clips);
            for (ci, c) in track.clips.iter().enumerate() {
                clip_ui(
                    ui, win, &mut tl, &painter, ti, ci, c, blends[ci], rr, zoom, scr, fps, area,
                    &all_rows,
                );
            }
            for (mi, m) in track.markers.iter().enumerate() {
                marker_ui(
                    ui,
                    win,
                    &mut tl,
                    &painter,
                    Some(ti),
                    mi,
                    m,
                    rr,
                    zoom,
                    scr,
                    fps,
                    area,
                );
            }
        }
        y += h;
    }
    // Empty space below the tracks: drops make new tracks.
    let rest = Rect::from_min_max(pos2(main.left(), y), main.max);
    if rest.height() > 4.0 {
        let er = ui.interact(rest, ui.id().with("tl_rest"), Sense::click());
        if er.clicked() {
            win.sel = Sel::None;
        }
        er.context_menu(|ui| {
            for t in TrackType::ALL {
                if ui.button(t.label()).clicked() {
                    header_changes.push(Box::new(move |tl, w| {
                        w.sel = Sel::Track(add_track(tl, t, ""))
                    }));
                    ui.close();
                }
            }
        });
        if let Some(p) = er.dnd_release_payload::<HierarchyDrag>()
            && let Some(&o) = p.0.first()
        {
            let anim = engine
                .world
                .get_component::<oxide_animation::Animator>(o)
                .is_some();
            let t = if anim {
                TrackType::Animation
            } else {
                TrackType::Activation
            };
            header_changes.push(Box::new(move |tl, w| {
                let i = add_track(tl, t, "");
                let name = tl.tracks[i].name.clone();
                w.sel = Sel::Track(i);
                w.pending_binding = Some((name, Some(o)));
            }));
        }
        if let Some(p) = er.dnd_release_payload::<AssetPayload>() {
            let assets = engine.world.resource::<AssetServer>();
            let kind = if is_type(assets, &p.0, "AnimationClip") || p.0.contains('#') {
                Some(TrackType::Animation)
            } else if is_type(assets, &p.0, "AudioClip") {
                Some(TrackType::Audio)
            } else if p.0.ends_with(".prefab") {
                Some(TrackType::Control)
            } else {
                None
            };
            if let Some(k) = kind {
                let mut proto = TrackAsset::new("", k);
                proto.track_type = k;
                let c = new_clip(assets, &proto, 0.0, &p.0);
                header_changes.push(Box::new(move |tl, w| {
                    let i = add_track(tl, k, "");
                    tl.tracks[i].clips.push(c);
                    w.sel = Sel::Clip(i, 0);
                }));
            }
        }
    }
    for f in header_changes {
        f(&mut tl, win);
    }
    if let Some((ti, ci)) = win.match_content.take()
        && let Some(c) = tl.tracks.get_mut(ti).and_then(|t| t.clips.get_mut(ci))
    {
        let assets = engine.world.resource::<AssetServer>();
        let len = match (&c.animation_clip, &c.audio_clip) {
            (Some(h), _) => assets.animations.get(h).map(|a| a.length),
            (_, Some(h)) => audio_length(assets, h.path()),
            _ => None,
        };
        if let Some(len) = len {
            c.duration = ((len - c.clip_in).max(1.0 / fps)) / c.time_scale.max(0.01);
        }
    }
    if let Some(p) = win.open_clip.take() {
        st.animation.open_clip(&p);
        st.actions
            .push(crate::Action::OpenTab(crate::Tab::Animation));
    }

    // Playhead.
    let px = t2x(win.time as f32, zoom, scr);
    if px >= area.left() && px <= area.right() {
        painter.line_segment(
            [pos2(px, ruler.top()), pos2(px, area.bottom())],
            Stroke::new(1.5, Color32::from_rgb(255, 255, 255)),
        );
        let tri = vec![
            pos2(px - 5.0, ruler.top()),
            pos2(px + 5.0, ruler.top()),
            pos2(px, ruler.top() + 7.0),
        ];
        painter.add(egui::Shape::convex_polygon(
            tri,
            Color32::WHITE,
            Stroke::NONE,
        ));
    }

    // Properties panel.
    let mut pui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(props_rect.shrink(4.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    egui::ScrollArea::vertical()
        .id_salt("tl_props")
        .show(&mut pui, |ui| {
            properties_ui(ui, &mut engine.world, win, &mut tl, director);
        });

    // Bindings chosen this frame.
    if let Some((track, obj)) = win.pending_binding.take()
        && let Some(id) = director
        && let Some(d) = engine.world.get_component_mut::<PlayableDirector>(id)
    {
        d.set_binding(&track, obj);
        st.dirty = true;
    }

    // Record: what changed since the last preview pose becomes keys (before saving, and
    // before the preview below poses the object again).
    if in_edit
        && win.preview
        && !win.playing
        && let Some(ti) = win.recording
        && let Some(id) = director
    {
        record_into(win, &mut engine.world, &path, &mut tl, ti, id);
    }

    // Undo steps: drags and typing bursts are one step each.
    let now = ui.input(|i| i.time);
    if tl != undo_base {
        match &mut win.pending {
            Some((_, t)) => *t = now,
            None => win.pending = Some((undo_base.clone(), now)),
        }
        if win.drag.is_none() && !ui.ctx().egui_wants_keyboard_input() {
            flush_undo(win);
        }
    } else if let Some((_, t)) = &win.pending
        && win.drag.is_none()
        && (now - *t > 0.6 || !ui.ctx().egui_wants_keyboard_input())
    {
        flush_undo(win);
    }
    let changed = tl != before;
    if changed {
        match save(&mut engine.world, &path, &tl) {
            Ok(()) => win.error = None,
            Err(e) => win.error = Some(e),
        }
    }

    // Preview: play, scrub, record.
    if st.play == crate::PlayState::Playing {
        if scrubbed
            && let Some(id) = director
            && let Some(d) = engine.world.get_component_mut::<PlayableDirector>(id)
        {
            d.set_time(win.time);
            d.evaluate();
        }
        return;
    }
    let Some(id) = director else { return };
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    if win.playing {
        win.time += f64::from(dt);
        let dur = f64::from(duration.max(1e-3));
        if win.time > dur {
            win.time = win.time.rem_euclid(dur);
        }
        ui.ctx().request_repaint();
    }
    if scrubbed {
        begin_preview(win, &mut engine.world);
    }
    if win.preview {
        timeline::preview(
            &mut engine.world,
            id,
            win.time,
            win.playing,
            if win.playing { dt } else { 0.0 },
        );
        if let Some(ti) = win.recording
            && let Some(track) = tl.tracks.get(ti)
            && let Some(o) = engine
                .world
                .get_component::<PlayableDirector>(id)
                .and_then(|d| d.binding(&track.name))
        {
            win.last = snapshot(&engine.world, o);
        }
    }
}

fn row_rects_snapshot(rows: &[(usize, f32)], top: f32, area: Rect) -> Vec<(usize, Rect)> {
    let mut y = top;
    rows.iter()
        .map(|&(i, h)| {
            let r = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), h - 2.0));
            y += h;
            (i, r)
        })
        .collect()
}

type Change = Box<dyn FnOnce(&mut TimelineAsset, &mut TimelineWindow)>;

fn bind_later(changes: &mut Vec<Change>, track: &str, obj: Option<GameObjectId>) {
    let name = track.to_owned();
    changes.push(Box::new(move |_, w| w.pending_binding = Some((name, obj))));
}

fn is_type(assets: &AssetServer, path: &str, type_name: &str) -> bool {
    assets
        .types
        .for_path(path.split('#').next().unwrap_or(path))
        .is_some_and(|t| t.type_name == type_name)
}

fn flush_undo(win: &mut TimelineWindow) {
    if let Some((tl, _)) = win.pending.take() {
        win.undo.push(tl);
        if win.undo.len() > 200 {
            win.undo.remove(0);
        }
        win.redo.clear();
    }
}

fn track_menu(
    ui: &mut egui::Ui,
    changes: &mut Vec<Change>,
    assets: &AssetServer,
    w: &World,
    track: &TrackAsset,
    ti: usize,
    t: f32,
) {
    let add = |changes: &mut Vec<Change>, c: TimelineClip| {
        changes.push(Box::new(move |tl, win| {
            tl.tracks[ti].clips.push(c);
            win.sel = Sel::Clip(ti, tl.tracks[ti].clips.len() - 1);
        }));
    };
    let proto = track.clone();
    match track.track_type {
        TrackType::Animation => {
            ui.menu_button("Add From Animation Clip", |ui| {
                let mut paths: Vec<String> = assets.paths_of_type("AnimationClip");
                paths.extend(assets.animations.iter().map(|(p, _)| p.to_owned()));
                paths.sort();
                paths.dedup();
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for p in paths {
                            if ui.button(&p).clicked() {
                                let mut c = new_clip(assets, &proto, t, &p);
                                c.start = t;
                                add(changes, c);
                                ui.close();
                            }
                        }
                    });
            });
        }
        TrackType::Audio => {
            ui.menu_button("Add From Audio Clip", |ui| {
                let mut paths = assets.paths_of_type("AudioClip");
                paths.extend(assets.audio.iter().map(|(p, _)| p.to_owned()));
                paths.sort();
                paths.dedup();
                for p in paths {
                    if ui.button(&p).clicked() {
                        add(changes, new_clip(assets, &proto, t, &p));
                        ui.close();
                    }
                }
            });
        }
        TrackType::Activation => {
            if ui.button("Add Activation Clip").clicked() {
                add(changes, new_clip(assets, &proto, t, ""));
                ui.close();
            }
        }
        TrackType::Control => {
            ui.menu_button("Add Control Clip", |ui| {
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for o in w.hierarchy_order().into_iter().take(2000) {
                            let name = w.get(o).map(|x| x.name.clone()).unwrap_or_default();
                            if ui.button(w.path_of(o)).clicked() {
                                let c = new_clip(assets, &proto, t, &name);
                                changes.push(Box::new(move |tl, win| {
                                    tl.tracks[ti].clips.push(c);
                                    win.sel = Sel::Clip(ti, tl.tracks[ti].clips.len() - 1);
                                    win.pending_reference = Some((name, o));
                                }));
                                ui.close();
                            }
                        }
                    });
            });
            ui.menu_button("Add From Prefab", |ui| {
                for p in assets.paths_of_type("Prefab") {
                    if ui.button(&p).clicked() {
                        add(changes, new_clip(assets, &proto, t, &p));
                        ui.close();
                    }
                }
            });
        }
        TrackType::Playable => {
            let names = w
                .try_resource::<PlayableBehaviours>()
                .map(PlayableBehaviours::names)
                .unwrap_or_default();
            ui.menu_button("Add Playable Clip", |ui| {
                if names.is_empty() {
                    ui.weak("No PlayableBehaviour types (scripts! { ...; playables: Type })");
                }
                for n in names {
                    if ui.button(&n).clicked() {
                        let mut c = new_clip(assets, &proto, t, &n);
                        if let Some(b) = w
                            .try_resource::<PlayableBehaviours>()
                            .and_then(|r| r.create(&n))
                        {
                            c.behaviour_fields = timeline::save_fields(b.as_ref());
                        }
                        add(changes, c);
                        ui.close();
                    }
                }
            });
        }
        TrackType::Signal | TrackType::Group => {}
    }
    if ui.button("Add Signal Emitter").clicked() {
        changes.push(Box::new(move |tl, win| {
            win.sel = add_marker(tl, Some(ti), t)
        }));
        ui.close();
    }
}

#[allow(clippy::too_many_arguments)]
fn clip_ui(
    ui: &mut egui::Ui,
    win: &mut TimelineWindow,
    tl: &mut TimelineAsset,
    painter: &egui::Painter,
    ti: usize,
    ci: usize,
    c: &TimelineClip,
    blend: (f32, f32),
    row: Rect,
    zoom: f32,
    scr: f32,
    fps: f32,
    area: Rect,
    rows: &[(usize, Rect)],
) {
    let x0 = area.left() + (c.start - scr) * zoom;
    let x1 = area.left() + (c.end() - scr) * zoom;
    if x1 < area.left() || x0 > area.right() {
        return;
    }
    let r = Rect::from_min_max(
        pos2(x0, row.top() + 3.0),
        pos2(x1.max(x0 + 3.0), row.bottom() - 3.0),
    );
    let track_type = tl.tracks[ti].track_type;
    let locked = tl.tracks[ti].locked;
    let selected = win.sel == Sel::Clip(ti, ci);
    let base = track_color(track_type);
    let fill = Color32::from_rgb(
        (base.r() as f32 * 0.55) as u8,
        (base.g() as f32 * 0.55) as u8,
        (base.b() as f32 * 0.55) as u8,
    );
    painter.rect_filled(r, 3.0, fill);
    // Eases and blends: the ramp drawn as a darker wedge.
    let ein = c.ease_in_duration.max(blend.0).min(c.duration);
    let eout = c.ease_out_duration.max(blend.1).min(c.duration);
    if ein > 0.0 {
        let xe = (x0 + ein * zoom).min(r.right());
        painter.add(egui::Shape::convex_polygon(
            vec![r.left_top(), pos2(xe, r.top()), r.left_bottom()],
            Color32::from_black_alpha(90),
            Stroke::NONE,
        ));
        painter.line_segment(
            [r.left_bottom(), pos2(xe, r.top())],
            Stroke::new(1.0, Color32::from_gray(200)),
        );
    }
    if eout > 0.0 {
        let xs = (x1 - eout * zoom).max(r.left());
        painter.add(egui::Shape::convex_polygon(
            vec![pos2(xs, r.top()), r.right_top(), r.right_bottom()],
            Color32::from_black_alpha(90),
            Stroke::NONE,
        ));
        painter.line_segment(
            [pos2(xs, r.top()), r.right_bottom()],
            Stroke::new(1.0, Color32::from_gray(200)),
        );
    }
    // Extrapolation hints.
    if c.post_extrapolation != ClipExtrapolation::None {
        painter.text(
            pos2(r.right() + 3.0, r.center().y),
            Align2::LEFT_CENTER,
            format!("{:?}", c.post_extrapolation),
            FontId::proportional(9.0),
            Color32::from_gray(120),
        );
    }
    painter.rect_stroke(
        r,
        3.0,
        Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected {
                Color32::from_rgb(250, 190, 60)
            } else {
                base
            },
        ),
        egui::StrokeKind::Inside,
    );
    let name = if c.display_name.is_empty() {
        "Clip"
    } else {
        &c.display_name
    };
    painter.with_clip_rect(r.shrink(2.0)).text(
        pos2(r.left() + 5.0, r.center().y),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(11.0),
        Color32::WHITE,
    );

    let id = ui.id().with(("tl_clip", ti, ci));
    let resp = ui.interact(r, id, Sense::click_and_drag());
    let hover = resp.hover_pos();
    let near = |x: f32| hover.is_some_and(|p| (p.x - x).abs() <= EDGE);
    if near(r.left()) || near(r.right()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if resp.clicked() || resp.drag_started() {
        win.sel = Sel::Clip(ti, ci);
    }
    if resp.double_clicked()
        && let Some(h) = &c.animation_clip
    {
        win.open_clip = Some(h.path().to_owned());
    }
    if resp.drag_started()
        && !locked
        && let Some(p) = resp.interact_pointer_pos()
    {
        let ease = ui.input(|i| i.modifiers.command);
        let kind = if (p.x - r.left()).abs() <= EDGE {
            if ease {
                DragKind::EaseIn
            } else {
                DragKind::TrimStart
            }
        } else if (p.x - r.right()).abs() <= EDGE {
            if ease {
                DragKind::EaseOut
            } else {
                DragKind::TrimEnd
            }
        } else {
            DragKind::Move
        };
        win.drag = Some(Drag {
            kind,
            sel: Sel::Clip(ti, ci),
            grab: (p.x - area.left()) / zoom + scr,
            clip: c.clone(),
        });
    }
    if let Some(d) = win.drag.clone()
        && d.sel == Sel::Clip(ti, ci)
        && let Some(p) = ui.ctx().pointer_interact_pos()
    {
        let now = (p.x - area.left()) / zoom + scr;
        let dt = now - d.grab;
        let snap = |t: f32| (t * fps).round() / fps;
        // Snap to the playhead and other clips' edges within a few pixels.
        let edges: Vec<f32> = tl
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter().flat_map(|x| [x.start, x.end()]))
            .chain([win.time as f32])
            .collect();
        let magnet = |t: f32| {
            edges
                .iter()
                .copied()
                .filter(|e| (e - t).abs() * zoom < 6.0)
                .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
                .unwrap_or_else(|| snap(t))
        };
        let o = &d.clip;
        let Some(mut n) = tl.tracks.get(ti).and_then(|t| t.clips.get(ci)).cloned() else {
            win.drag = None;
            return;
        };
        match d.kind {
            DragKind::Move => {
                let s = magnet(o.start + dt).max(0.0);
                let e = magnet(o.end() + dt);
                n.start = if (e - (o.end() + dt)).abs() < (s - (o.start + dt)).abs() {
                    (e - o.duration).max(0.0)
                } else {
                    s
                };
            }
            DragKind::TrimStart => {
                let s = magnet(o.start + dt).clamp(0.0, o.end() - 1.0 / fps);
                let delta = s - o.start;
                n.start = s;
                n.duration = o.duration - delta;
                if matches!(track_type, TrackType::Animation | TrackType::Audio) {
                    n.clip_in = (o.clip_in + delta * o.time_scale).max(0.0);
                }
            }
            DragKind::TrimEnd => n.duration = (magnet(o.end() + dt) - o.start).max(1.0 / fps),
            DragKind::EaseIn => {
                n.ease_in_duration = snap((o.ease_in_duration + dt).clamp(0.0, o.duration))
            }
            DragKind::EaseOut => {
                n.ease_out_duration = snap((o.ease_out_duration - dt).clamp(0.0, o.duration))
            }
            DragKind::Marker => {}
        }
        tl.tracks[ti].clips[ci] = n;
        if resp.drag_stopped() || !ui.input(|i| i.pointer.any_down()) {
            // Dropped over another track of the same type: the clip moves there.
            if d.kind == DragKind::Move
                && let Some(&(target, _)) = rows.iter().find(|(_, rr)| rr.y_range().contains(p.y))
                && target != ti
                && tl
                    .tracks
                    .get(target)
                    .is_some_and(|t| t.track_type == track_type && !t.locked)
            {
                let moved = tl.tracks[ti].clips.remove(ci);
                tl.tracks[target].clips.push(moved);
                win.sel = Sel::Clip(target, tl.tracks[target].clips.len() - 1);
            }
            win.drag = None;
        }
    }
    resp.context_menu(|ui| {
        if c.animation_clip.is_some() && ui.button("Edit in Animation Window").clicked() {
            win.open_clip = c.animation_clip.as_ref().map(|h| h.path().to_owned());
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            win.sel = duplicate_sel(tl, Sel::Clip(ti, ci));
            ui.close();
        }
        if ui.button("Match Content (length of the source)").clicked() {
            win.match_content = Some((ti, ci));
            ui.close();
        }
        if ui.button("Delete").clicked() {
            delete_sel(tl, Sel::Clip(ti, ci));
            win.sel = Sel::None;
            ui.close();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn marker_ui(
    ui: &mut egui::Ui,
    win: &mut TimelineWindow,
    tl: &mut TimelineAsset,
    painter: &egui::Painter,
    track: Option<usize>,
    mi: usize,
    m: &SignalEmitter,
    row: Rect,
    zoom: f32,
    scr: f32,
    fps: f32,
    area: Rect,
) {
    let x = area.left() + (m.time - scr) * zoom;
    if x < area.left() - 6.0 || x > area.right() + 6.0 {
        return;
    }
    let sel = Sel::Marker(track, mi);
    let selected = win.sel == sel;
    let cy = row.top() + 8.0;
    let pts = vec![
        pos2(x, cy - 6.0),
        pos2(x + 5.0, cy),
        pos2(x, cy + 6.0),
        pos2(x - 5.0, cy),
    ];
    let col = if m.signal.is_some() {
        Color32::from_rgb(240, 210, 90)
    } else {
        Color32::from_gray(150)
    };
    painter.add(egui::Shape::convex_polygon(
        pts,
        col,
        Stroke::new(
            if selected { 2.0 } else { 0.5 },
            if selected {
                Color32::WHITE
            } else {
                Color32::BLACK
            },
        ),
    ));
    let r = Rect::from_center_size(pos2(x, cy), vec2(12.0, 14.0));
    let resp = ui.interact(
        r,
        ui.id().with(("tl_marker", track, mi)),
        Sense::click_and_drag(),
    );
    if resp.clicked() || resp.drag_started() {
        win.sel = sel;
    }
    let locked = track
        .and_then(|t| tl.tracks.get(t))
        .is_some_and(|t| t.locked);
    if resp.drag_started() && !locked {
        win.drag = Some(Drag {
            kind: DragKind::Marker,
            sel,
            grab: 0.0,
            clip: TimelineClip::default(),
        });
    }
    if let Some(d) = &win.drag
        && d.sel == sel
        && d.kind == DragKind::Marker
    {
        if let Some(p) = ui.ctx().pointer_interact_pos() {
            let t = ((((p.x - area.left()) / zoom + scr) * fps).round() / fps).max(0.0);
            let slot = match track {
                Some(ti) => tl.tracks.get_mut(ti).and_then(|x| x.markers.get_mut(mi)),
                None => tl.markers.get_mut(mi),
            };
            if let Some(m) = slot {
                m.time = t;
            }
        }
        if resp.drag_stopped() || !ui.input(|i| i.pointer.any_down()) {
            win.drag = None;
        }
    }
    resp.on_hover_text(
        m.signal
            .as_ref()
            .map_or("Signal Emitter (no signal)".to_owned(), |s| {
                s.path().to_owned()
            }),
    )
    .context_menu(|ui| {
        if ui.button("Delete").clicked() {
            delete_sel(tl, sel);
            win.sel = Sel::None;
            ui.close();
        }
    });
}

/// Fields of `TimelineClip` shown for each track type.
fn clip_fields(t: TrackType) -> &'static [&'static str] {
    match t {
        TrackType::Animation => &[
            "display_name",
            "start",
            "duration",
            "clip_in",
            "time_scale",
            "ease_in_duration",
            "ease_out_duration",
            "pre_extrapolation",
            "post_extrapolation",
            "animation_clip",
            "loop_mode",
            "position_offset",
            "rotation_offset",
        ],
        TrackType::Audio => &[
            "display_name",
            "start",
            "duration",
            "clip_in",
            "time_scale",
            "ease_in_duration",
            "ease_out_duration",
            "audio_clip",
            "volume",
            "looping",
        ],
        TrackType::Activation => &["display_name", "start", "duration"],
        TrackType::Control => &[
            "display_name",
            "start",
            "duration",
            "clip_in",
            "time_scale",
            "source_game_object",
            "prefab",
            "control_activation",
            "post_playback",
            "update_directors",
            "update_particles",
            "pre_extrapolation",
            "post_extrapolation",
        ],
        TrackType::Playable => &[
            "display_name",
            "start",
            "duration",
            "clip_in",
            "time_scale",
            "ease_in_duration",
            "ease_out_duration",
            "pre_extrapolation",
            "post_extrapolation",
        ],
        TrackType::Signal | TrackType::Group => &[],
    }
}

fn track_fields(t: TrackType) -> &'static [&'static str] {
    match t {
        TrackType::Animation => &[
            "name",
            "muted",
            "locked",
            "apply_offsets",
            "position",
            "rotation",
            "avatar_mask",
        ],
        TrackType::Activation => &["name", "muted", "locked", "post_playback_state"],
        TrackType::Audio => &["name", "muted", "locked", "volume"],
        _ => &["name", "muted", "locked"],
    }
}

fn fields_ui(
    ui: &mut egui::Ui,
    w: &World,
    obj: &mut dyn Reflect,
    names: &[&str],
    salt: &str,
) -> bool {
    let info = obj.type_info();
    let mut changed = false;
    for name in names {
        let Some(f) = info.fields.iter().find(|f| f.name == *name) else {
            continue;
        };
        let Some(v) = obj.get_field(f.name) else {
            continue;
        };
        let mut v2 = v.clone();
        if field_row(
            ui,
            w,
            f,
            nicify(f.name),
            &mut v2,
            false,
            &format!("{salt}{}", f.name),
        ) && v2 != v
            && obj.set_field(f.name, v2).is_ok()
        {
            changed = true;
        }
    }
    changed
}

fn properties_ui(
    ui: &mut egui::Ui,
    w: &mut World,
    win: &mut TimelineWindow,
    tl: &mut TimelineAsset,
    director: Option<GameObjectId>,
) {
    match win.sel {
        Sel::Clip(ti, ci) if tl.tracks.get(ti).is_some_and(|t| ci < t.clips.len()) => {
            let tt = tl.tracks[ti].track_type;
            ui.heading(
                egui::RichText::new(format!("{} Clip", nicify(&format!("{tt:?}")))).size(14.0),
            );
            ui.separator();
            let mut c = tl.tracks[ti].clips[ci].clone();
            let mut changed = fields_ui(ui, w, &mut c, clip_fields(tt), &format!("tlc{ti}_{ci}"));
            if tt == TrackType::Control
                && !c.source_game_object.is_empty()
                && let Some(id) = director
            {
                let current = w
                    .get_component::<PlayableDirector>(id)
                    .and_then(|d| d.reference(&c.source_game_object));
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Source Object");
                    let label = current
                        .and_then(|o| w.get(o).map(|x| x.name.clone()))
                        .unwrap_or_else(|| "None".into());
                    egui::ComboBox::from_id_salt("tl_src")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for o in w.hierarchy_order().into_iter().take(2000) {
                                if ui
                                    .selectable_label(current == Some(o), w.path_of(o))
                                    .clicked()
                                {
                                    win.pending_reference = Some((c.source_game_object.clone(), o));
                                }
                            }
                        });
                });
            }
            if tt == TrackType::Playable {
                changed |= playable_fields(ui, w, &mut c);
            }
            if changed {
                tl.tracks[ti].clips[ci] = c;
            }
        }
        Sel::Track(ti) if ti < tl.tracks.len() => {
            let tt = tl.tracks[ti].track_type;
            ui.heading(egui::RichText::new(tt.label()).size(14.0));
            ui.separator();
            let old_name = tl.tracks[ti].name.clone();
            let mut t = tl.tracks[ti].clone();
            if fields_ui(ui, w, &mut t, track_fields(tt), &format!("tlt{ti}")) {
                if t.name != old_name {
                    if t.name.is_empty() || tl.track(&t.name).is_some() {
                        t.name = old_name.clone();
                    } else {
                        // Bindings and grouped tracks follow the rename.
                        for x in &mut tl.tracks {
                            if x.group == old_name {
                                x.group = t.name.clone();
                            }
                        }
                        if let Some(id) = director
                            && let Some(d) = w.get_component_mut::<PlayableDirector>(id)
                        {
                            for b in &mut d.bindings {
                                if b.track == old_name {
                                    b.track = t.name.clone();
                                }
                            }
                        }
                    }
                }
                tl.tracks[ti] = t;
            }
        }
        Sel::Marker(track, mi) => {
            let m = match track {
                Some(t) => tl.tracks.get_mut(t).and_then(|t| t.markers.get_mut(mi)),
                None => tl.markers.get_mut(mi),
            };
            let Some(m) = m else {
                win.sel = Sel::None;
                return;
            };
            ui.heading(egui::RichText::new("Signal Emitter").size(14.0));
            ui.separator();
            let mut e = m.clone();
            if fields_ui(
                ui,
                w,
                &mut e,
                &["time", "signal", "retroactive", "emit_once"],
                &format!("tlm{track:?}{mi}"),
            ) {
                *m = e.clone();
            }
            if ui
                .button("New Signal Asset")
                .on_hover_text("Create a .signal file and use it")
                .clicked()
            {
                let assets = w.resource_mut::<AssetServer>();
                let mut p = "Assets/New Signal.signal".to_owned();
                let mut i = 1;
                while assets.exists(&p) {
                    p = format!("Assets/New Signal ({i}).signal");
                    i += 1;
                }
                if assets
                    .save_data(&p, Box::new(SignalAsset::default()))
                    .is_ok()
                {
                    m.signal = Some(Handle::from_path(p));
                }
            }
            // The receiver: the bound object (or the Director's for Timeline markers).
            let receiver = match track {
                Some(t) => director
                    .and_then(|id| w.get_component::<PlayableDirector>(id))
                    .and_then(|d| d.binding(&tl.tracks[t].name)),
                None => director,
            };
            ui.add_space(6.0);
            match receiver {
                None => {
                    ui.weak("Bind the track to an object to receive this signal.");
                }
                Some(r) => {
                    let name = w.get(r).map(|o| o.name.clone()).unwrap_or_default();
                    match w.get_component::<SignalReceiver>(r).cloned() {
                        None => {
                            ui.label(format!("{name} has no Signal Receiver."));
                            if ui.button("Add Signal Receiver").clicked() {
                                w.add_component(r, SignalReceiver::default());
                            }
                        }
                        Some(recv) => {
                            let sig = e.signal.as_ref().map(|h| h.path().to_owned());
                            let reactions: Vec<(usize, SignalReaction)> = recv
                                .reactions
                                .iter()
                                .cloned()
                                .enumerate()
                                .filter(|(_, x)| {
                                    x.signal.as_ref().map(|h| h.path().to_owned()) == sig
                                })
                                .collect();
                            ui.label(format!("Signal Receiver on {name}"));
                            let mut edits: Vec<(usize, SignalReaction)> = Vec::new();
                            for (i, mut x) in reactions {
                                ui.horizontal(|ui| {
                                    ui.label("Method");
                                    if ui.text_edit_singleline(&mut x.method).changed() {
                                        edits.push((i, x.clone()));
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Parameter");
                                    if ui.text_edit_singleline(&mut x.parameter).changed() {
                                        edits.push((i, x.clone()));
                                    }
                                });
                            }
                            if sig.is_some() && ui.button("Add Reaction").clicked() {
                                edits.push((
                                    usize::MAX,
                                    SignalReaction {
                                        signal: e.signal.clone(),
                                        ..Default::default()
                                    },
                                ));
                            }
                            if !edits.is_empty()
                                && let Some(rc) = w.get_component_mut::<SignalReceiver>(r)
                            {
                                for (i, x) in edits {
                                    if i == usize::MAX {
                                        rc.reactions.push(x);
                                    } else if let Some(slot) = rc.reactions.get_mut(i) {
                                        *slot = x;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {
            ui.heading(egui::RichText::new("Timeline").size(14.0));
            ui.separator();
            ui.label(format!(
                "Duration: {:.2} s ({} frames)",
                tl.duration(),
                (tl.duration() * tl.frame_rate).round()
            ));
            ui.label(format!("{} tracks", tl.tracks.len()));
            ui.add_space(6.0);
            ui.weak("Select a track, clip or marker to edit it. Right-click a track to add clips; drop Animation Clips, Audio Clips or prefabs onto tracks, or GameObjects from the Hierarchy to bind them.");
        }
    }
    if let Some((name, o)) = win.pending_reference.take()
        && let Some(id) = director
        && let Some(d) = w.get_component_mut::<PlayableDirector>(id)
    {
        d.set_reference(&name, Some(o));
    }
}

/// A Playable clip's behaviour type and its fields (an instance shows them).
fn playable_fields(ui: &mut egui::Ui, w: &World, c: &mut TimelineClip) -> bool {
    let Some(reg) = w.try_resource::<PlayableBehaviours>() else {
        return false;
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Behaviour");
        egui::ComboBox::from_id_salt("tl_beh")
            .selected_text(if c.behaviour.is_empty() {
                "None"
            } else {
                &c.behaviour
            })
            .show_ui(ui, |ui| {
                for n in reg.names() {
                    if ui.selectable_label(c.behaviour == n, &n).clicked() {
                        c.behaviour = n.clone();
                        c.behaviour_fields = reg
                            .create(&n)
                            .map(|b| timeline::save_fields(b.as_ref()))
                            .unwrap_or_default();
                        changed = true;
                    }
                }
            });
    });
    let Some(mut b) = reg.instantiate(&c.behaviour, &c.behaviour_fields) else {
        if !c.behaviour.is_empty() {
            ui.colored_label(
                Color32::from_rgb(230, 160, 90),
                format!("`{}` is not registered (scripts not loaded?)", c.behaviour),
            );
        }
        return changed;
    };
    ui.separator();
    if crate::inspector::reflect_grid(ui, w, b.as_mut(), "tl_beh_fields") {
        c.behaviour_fields = timeline::save_fields(b.as_ref());
        changed = true;
    }
    changed
}

/// Record mode: changes to the bound object since the last preview pose become keys in a
/// recorded clip under the playhead (one is made when there is none).
fn record_into(
    win: &mut TimelineWindow,
    w: &mut World,
    tl_path: &str,
    tl: &mut TimelineAsset,
    ti: usize,
    id: GameObjectId,
) {
    let Some(track) = tl.tracks.get(ti).cloned() else {
        return;
    };
    let Some(root) = w
        .get_component::<PlayableDirector>(id)
        .and_then(|d| d.binding(&track.name))
    else {
        win.error = Some("Bind the track to an object before recording".into());
        return;
    };
    if win.last.transforms.is_empty() && win.last.props.is_empty() {
        win.last = snapshot(w, root);
        return;
    }
    let t = win.time as f32;
    let recorded = |c: &TimelineClip| {
        c.animation_clip
            .as_ref()
            .is_some_and(|h| h.path().ends_with(".anim"))
    };
    // The clip under the playhead, or the last recorded clip before it when nothing lies in
    // between (it grows to the playhead, like Unity's recorded clips).
    let under = track
        .clips
        .iter()
        .position(|c| t >= c.start && t <= c.end() && recorded(c));
    let before = track
        .clips
        .iter()
        .enumerate()
        .filter(|(_, c)| recorded(c) && c.end() < t)
        .max_by(|a, b| a.1.end().total_cmp(&b.1.end()))
        .filter(|(_, c)| {
            !track
                .clips
                .iter()
                .any(|o| o.start > c.end() && o.start <= t)
        })
        .map(|(i, _)| i);
    let ci = match under.or(before) {
        Some(ci) => ci,
        None => {
            // Only make a clip when something actually changed.
            let mut probe = AnimationClip::default();
            if !record(w, root, &mut probe, &win.last, 0.0) {
                return;
            }
            let dir = tl_path.rsplit_once('/').map_or("Assets", |(d, _)| d);
            let stem = tl_path
                .rsplit('/')
                .next()
                .unwrap_or("Timeline")
                .trim_end_matches(".playable");
            let assets = w.resource_mut::<AssetServer>();
            let mut p = format!("{dir}/{stem}_Recorded.anim");
            let mut i = 1;
            while assets.exists(&p) || assets.animations.iter().any(|(x, _)| x == p) {
                p = format!("{dir}/{stem}_Recorded ({i}).anim");
                i += 1;
            }
            let clip = AnimationClip {
                name: "Recorded".into(),
                ..Default::default()
            };
            if let Err(e) = assets.save_animation_clip(&p, &clip) {
                win.error = Some(e);
                return;
            }
            tl.tracks[ti].clips.push(TimelineClip {
                display_name: "Recorded".into(),
                start: t,
                duration: 1.0,
                animation_clip: Some(Handle::from_path(p)),
                post_extrapolation: ClipExtrapolation::Hold,
                ..Default::default()
            });
            tl.tracks[ti].clips.len() - 1
        }
    };
    let c = tl.tracks[ti].clips[ci].clone();
    let Some(h) = c.animation_clip.clone() else {
        return;
    };
    let local = (t - c.start) * c.time_scale + c.clip_in;
    let assets = w.resource::<AssetServer>();
    let Some(mut clip) = assets.animations.get(&h).cloned() else {
        return;
    };
    if record(w, root, &mut clip, &win.last, local) {
        clip.recalculate_length();
        if c.end() < t + 1.0 / tl.frame_rate {
            tl.tracks[ti].clips[ci].duration =
                (t - c.start).max(1.0 / tl.frame_rate) + 1.0 / tl.frame_rate;
        }
        if let Err(e) = w
            .resource_mut::<AssetServer>()
            .save_animation_clip(h.path(), &clip)
        {
            win.error = Some(e);
        }
    }
}

/// Shown when no Director is selected: Unity's "select a GameObject" and Create button.
fn empty_state(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        match st.selection.last().copied() {
            Some(obj) if engine.world.is_alive(obj) => {
                let name = engine.world.get(obj).map(|o| o.name.clone()).unwrap_or_default();
                ui.label(format!("To begin a new timeline with {name}, create a Director component and a Timeline asset"));
                ui.add_space(8.0);
                if ui.button("  Create  ").clicked() && st.play == crate::PlayState::Edit {
                    let now = ui.input(|i| i.time);
                    st.undo.record(&engine.world, "Create Timeline", &st.selection, None, now);
                    match create_for(&mut engine.world, obj) {
                        Ok(_) => {
                            st.timeline.director = Some(obj);
                            st.dirty = true;
                        }
                        Err(e) => st.timeline.error = Some(e),
                    }
                }
            }
            _ => {
                ui.label("To start creating a Timeline, select a GameObject");
            }
        }
        if let Some(e) = &st.timeline.error {
            ui.colored_label(Color32::from_rgb(230, 120, 100), e);
        }
    });
}

// ---- MCP --------------------------------------------------------------------------------

fn track_type_arg(s: &str) -> Option<TrackType> {
    let s = s.to_ascii_lowercase().replace([' ', '_'], "");
    TrackType::ALL.into_iter().find(|t| {
        let n = format!("{t:?}").to_ascii_lowercase();
        s == n || s == format!("{n}track") || (s == "group" && *t == TrackType::Group)
    })
}

/// `editor_timeline`: drives the window and edits the Timeline the way its menus do.
pub fn mcp(
    st: &mut EditorState,
    engine: &mut Engine,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    use oxide_engine::json_values::resolve_object;
    use serde_json::Value as Json;
    let obj = |w: &World, k: &str| -> Result<Option<GameObjectId>, String> {
        match args.get(k) {
            None | Some(Json::Null) => Ok(None),
            Some(v) => resolve_object(w, v)
                .map(Some)
                .ok_or_else(|| format!("no GameObject matches {v}")),
        }
    };
    if let Some(o) = obj(&engine.world, "create")? {
        if st.play != crate::PlayState::Edit {
            return Err("stop Play mode first".into());
        }
        if engine.world.get_component::<PlayableDirector>(o).is_none() {
            let now = oxide_core::time::realtime_since_startup();
            st.undo
                .record(&engine.world, "Create Timeline", &st.selection, None, now);
            create_for(&mut engine.world, o)?;
            st.dirty = true;
        }
        st.timeline.director = Some(o);
        st.timeline.locked = true;
    }
    if let Some(p) = args.get("open").and_then(Json::as_str) {
        engine
            .world
            .resource_mut::<AssetServer>()
            .ensure_loaded(p)?;
        st.timeline.open_asset(&engine.world, p);
    }
    let director = obj(&engine.world, "director")?;
    // Edits.
    let path = director
        .and_then(|d| engine.world.get_component::<PlayableDirector>(d))
        .and_then(|d| d.playable_asset.as_ref().map(|h| h.path().to_owned()))
        .or_else(|| st.timeline.asset_path(&engine.world));
    let mut edited = false;
    if let Some(path) = &path
        && let Some(mut tl) = load(&mut engine.world, path)
    {
        let before = tl.clone();
        let track_index = |tl: &TimelineAsset, v: Option<&Json>| -> Result<usize, String> {
            match v {
                Some(Json::Number(n)) => n
                    .as_u64()
                    .map(|n| n as usize)
                    .filter(|&n| n < tl.tracks.len())
                    .ok_or_else(|| format!("no track {n}")),
                Some(Json::String(s)) => tl
                    .tracks
                    .iter()
                    .position(|t| &t.name == s)
                    .ok_or_else(|| format!("no track named `{s}`")),
                _ => Err("`track` (index or name) is required".into()),
            }
        };
        if let Some(a) = args.get("add_track") {
            let ty = a.get("type").and_then(Json::as_str).unwrap_or("Animation");
            let ty = track_type_arg(ty).ok_or_else(|| format!("unknown track type `{ty}`"))?;
            let group = a.get("group").and_then(Json::as_str).unwrap_or("");
            let i = add_track(&mut tl, ty, group);
            if let Some(n) = a.get("name").and_then(Json::as_str)
                && tl.track(n).is_none()
            {
                tl.tracks[i].name = n.to_owned();
            }
            if let Some(o) = obj(&engine.world, "bind")?
                && let Some(d) = director.or(st.timeline.director)
                && let Some(dc) = engine.world.get_component_mut::<PlayableDirector>(d)
            {
                dc.set_binding(&tl.tracks[i].name, Some(o));
            }
            st.timeline.sel = Sel::Track(i);
        }
        if let Some(a) = args.get("add_clip") {
            let ti = track_index(&tl, a.get("track"))?;
            let source = a.get("source").and_then(Json::as_str).unwrap_or("");
            if !source.is_empty() && source.starts_with("Assets/") {
                let _ = engine
                    .world
                    .resource_mut::<AssetServer>()
                    .ensure_loaded(source.split('#').next().unwrap_or(source));
            }
            let start = a.get("start").and_then(Json::as_f64).unwrap_or(0.0) as f32;
            let mut c = new_clip(
                engine.world.resource::<AssetServer>(),
                &tl.tracks[ti],
                start,
                source,
            );
            if tl.tracks[ti].track_type == TrackType::Playable
                && let Some(b) = engine
                    .world
                    .try_resource::<PlayableBehaviours>()
                    .and_then(|r| r.create(source))
            {
                c.behaviour_fields = timeline::save_fields(b.as_ref());
            }
            if let Some(d) = a.get("duration").and_then(Json::as_f64) {
                c.duration = d as f32;
            }
            for (k, f) in [
                ("ease_in", &mut c.ease_in_duration),
                ("ease_out", &mut c.ease_out_duration),
                ("clip_in", &mut c.clip_in),
                ("speed", &mut c.time_scale),
            ] {
                if let Some(v) = a.get(k).and_then(Json::as_f64) {
                    *f = v as f32;
                }
            }
            if tl.tracks[ti].track_type == TrackType::Control
                && let Some(o) = obj(&engine.world, "source_object")?
                && let Some(d) = director.or(st.timeline.director)
            {
                let name = engine
                    .world
                    .get(o)
                    .map(|x| x.name.clone())
                    .unwrap_or_default();
                c.source_game_object = name.clone();
                if let Some(dc) = engine.world.get_component_mut::<PlayableDirector>(d) {
                    dc.set_reference(&name, Some(o));
                }
            }
            tl.tracks[ti].clips.push(c);
            st.timeline.sel = Sel::Clip(ti, tl.tracks[ti].clips.len() - 1);
        }
        if let Some(a) = args.get("add_marker") {
            let ti = match a.get("track") {
                None | Some(Json::Null) => None,
                v => Some(track_index(&tl, v)?),
            };
            let t = a.get("time").and_then(Json::as_f64).unwrap_or(0.0) as f32;
            let sel = add_marker(&mut tl, ti, t);
            if let Some(sig) = a.get("signal").and_then(Json::as_str) {
                let assets = engine.world.resource_mut::<AssetServer>();
                if !assets.exists(sig) {
                    assets.save_data(sig, Box::new(SignalAsset::default()))?;
                }
                let m = match sel {
                    Sel::Marker(Some(t), i) => &mut tl.tracks[t].markers[i],
                    Sel::Marker(None, i) => &mut tl.markers[i],
                    _ => unreachable!(),
                };
                m.signal = Some(Handle::from_path(sig));
                m.retroactive = a
                    .get("retroactive")
                    .and_then(Json::as_bool)
                    .unwrap_or(false);
                m.emit_once = a.get("emit_once").and_then(Json::as_bool).unwrap_or(false);
            }
            st.timeline.sel = sel;
        }
        if let Some(a) = args.get("set_clip") {
            let ti = track_index(&tl, a.get("track"))?;
            let ci = a
                .get("clip")
                .and_then(Json::as_u64)
                .ok_or("`clip` index is required")? as usize;
            let c = tl.tracks[ti].clips.get_mut(ci).ok_or("no such clip")?;
            for (k, v) in a.as_object().into_iter().flatten() {
                if matches!(k.as_str(), "track" | "clip") {
                    continue;
                }
                let info = c.type_info();
                let f = info
                    .fields
                    .iter()
                    .find(|f| f.name == k)
                    .ok_or_else(|| format!("no clip field `{k}`"))?;
                let val = oxide_engine::json_values::json_to_value(f, v, &engine.world)?;
                c.set_field(k, val).map_err(|e| e.to_string())?;
            }
        }
        if let Some(a) = args.get("delete") {
            let ti = track_index(&tl, a.get("track"))?;
            let sel = match (
                a.get("clip").and_then(Json::as_u64),
                a.get("marker").and_then(Json::as_u64),
            ) {
                (Some(c), _) => Sel::Clip(ti, c as usize),
                (_, Some(m)) => Sel::Marker(Some(ti), m as usize),
                _ => Sel::Track(ti),
            };
            delete_sel(&mut tl, sel);
            st.timeline.sel = Sel::None;
        }
        if let Some(b) = args.get("bind").filter(|_| args.get("add_track").is_none()) {
            let ti = track_index(&tl, args.get("track"))?;
            let o = resolve_object(&engine.world, b);
            let d = director.or(st.timeline.director).ok_or("no Director")?;
            if let Some(dc) = engine.world.get_component_mut::<PlayableDirector>(d) {
                dc.set_binding(&tl.tracks[ti].name, o);
            }
        }
        if tl != before {
            if let Some(p) = st.timeline.pending.take() {
                st.timeline.undo.push(p.0);
            }
            st.timeline.undo.push(before);
            st.timeline.redo.clear();
            save(&mut engine.world, path, &tl)?;
            edited = true;
        }
    }
    let record = match args.get("record") {
        None => None,
        Some(Json::Null) | Some(Json::Bool(false)) => Some(None),
        Some(v) => Some(Some(
            v.as_u64().ok_or("`record` is a track index or false")? as usize,
        )),
    };
    control(
        st,
        engine,
        director,
        args.get("time").and_then(Json::as_f64),
        args.get("preview").and_then(Json::as_bool),
        args.get("play").and_then(Json::as_bool),
        record,
    )?;
    let mut out = st.timeline.state_json(&mut engine.world);
    out["edited"] = Json::Bool(edited);
    Ok(out)
}
