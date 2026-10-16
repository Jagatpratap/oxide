//! The Animation window: edits the clips of the selected object's Animator (or Animation
//! component). A dopesheet of keys per property, a playhead that previews the clip on the
//! object in Edit mode, Record mode (moving objects or changing component values while
//! recording writes keys), Add Property, and a Curves view for property curves.
//! Reference: <https://docs.unity3d.com/Manual/AnimationEditorGuide.html>

use crate::EditorState;
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, pos2, vec2};
use oxide_animation::{
    Animation, Animator, find_path, read_object_property, read_property, write_object_property,
    write_property,
};
use oxide_assets::{
    AnimationClip, AnimatorController, AssetServer, Handle, Interpolation, ObjectCurve,
    PropertyCurve, Track, TransformCurves,
};
use oxide_core::reflect::Value;
use oxide_core::{AnimationCurve, Keyframe};
use oxide_engine::Engine;
use oxide_math::{Quaternion, Vector3};
use oxide_scene::{GameObjectId, World};
use std::collections::HashMap;

/// Keys snap to frames at this rate (Unity's default sample rate).
const FPS: f32 = 60.0;
const ROW_H: f32 = 20.0;
const LEFT_W: f32 = 240.0;

/// One dopesheet row.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Row {
    Position(usize),
    Rotation(usize),
    Scale(usize),
    Property(usize),
    /// Object-reference keys (sprites).
    Object(usize),
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Local {
    p: Vector3,
    r: Quaternion,
    s: Vector3,
}

/// Values the preview last wrote, to spot edits made while recording.
#[derive(Default)]
pub(crate) struct Snapshot {
    pub(crate) transforms: HashMap<GameObjectId, Local>,
    /// (object, component, property) → value.
    pub(crate) props: HashMap<(GameObjectId, String, String), f32>,
    /// Asset and text fields (sprites, resolver labels).
    objs: HashMap<(GameObjectId, String, String), String>,
}

#[derive(Default)]
pub struct AnimationWindow {
    target: Option<GameObjectId>,
    clip: Option<String>,
    time: f32,
    playing: bool,
    pub recording: bool,
    preview: bool,
    /// Original state of the previewed object, restored when preview ends.
    backup: Option<(GameObjectId, Snapshot)>,
    last: Snapshot,
    selected: Option<(Row, usize)>,
    dragging: Option<(Row, usize)>,
    curves_view: bool,
    span: f32,
    error: Option<String>,
}

impl AnimationWindow {
    /// Shows `path` (from the clip's Inspector).
    pub fn open_clip(&mut self, path: &str) {
        self.clip = Some(path.to_owned());
    }

    /// The window's state for MCP.
    pub fn state_json(&self) -> serde_json::Value {
        serde_json::json!({
            "target": self.target.map(|t| t.to_bits()),
            "clip": self.clip,
            "time": self.time,
            "frame": (self.time * FPS).round(),
            "recording": self.recording,
            "preview": self.preview,
            "playing": self.playing,
        })
    }

    /// MCP: keys a sprite sheet's sprites as a flipbook on the current clip.
    pub fn add_sprites(&mut self, world: &mut World, sprites: &str) -> Result<usize, String> {
        let path = self
            .clip
            .clone()
            .ok_or("the Animation window has no clip")?;
        if !path.ends_with(".anim") {
            return Err(format!("{path} is read-only"));
        }
        let assets = world.resource_mut::<AssetServer>();
        if let Some(t) = sprites.split('#').next() {
            assets.ensure_loaded(t)?;
        }
        let h: Handle<AnimationClip> = Handle::from_path(path.clone());
        let mut clip = assets
            .animations
            .get(&h)
            .cloned()
            .ok_or("clip not loaded")?;
        let n = add_flipbook(&mut clip, assets, sprites);
        assets.save_animation_clip(&path, &clip)?;
        Ok(n)
    }

    /// MCP control: playhead (seconds), record, preview, clip.
    pub fn control(
        &mut self,
        world: &mut World,
        time: Option<f32>,
        record: Option<bool>,
        preview: Option<bool>,
        clip: Option<&str>,
    ) {
        if let Some(c) = clip {
            self.clip = Some(c.to_owned());
        }
        if let Some(t) = time {
            self.time = snap(t);
        }
        let want_preview = preview.unwrap_or(self.preview) || record == Some(true);
        if want_preview
            && !self.preview
            && let Some(root) = self.target
        {
            self.backup = Some((root, snapshot(world, root)));
            self.preview = true;
        }
        if !want_preview && self.preview {
            end_preview(self, world);
        }
        if let Some(r) = record {
            self.recording = r && self.preview;
        }
    }
}

fn snap(t: f32) -> f32 {
    ((t * FPS).round() / FPS).max(0.0)
}

fn local(world: &World, id: GameObjectId) -> Option<Local> {
    let t = world.transform(id)?;
    Some(Local {
        p: t.local_position,
        r: t.local_rotation,
        s: t.local_scale,
    })
}

/// The object whose clips the window edits: the selection or its nearest ancestor with an
/// Animator or Animation component.
fn animated_root(world: &World, id: GameObjectId) -> Option<GameObjectId> {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if world.has_component::<Animator>(c) || world.has_component::<Animation>(c) {
            return Some(c);
        }
        cur = world.parent(c);
    }
    None
}

fn target_clips(world: &World, assets: &AssetServer, root: GameObjectId) -> Vec<String> {
    if let Some(a) = world.get_component::<Animator>(root)
        && let Some(c) = a
            .controller
            .as_ref()
            .and_then(|h| assets.controllers.get(h))
    {
        return c
            .clips()
            .iter()
            .map(|h| h.path().to_owned())
            .filter(|p| !p.is_empty())
            .collect();
    }
    if let Some(a) = world.get_component::<Animation>(root)
        && let Some(c) = &a.clip
    {
        return vec![c.path().to_owned()];
    }
    Vec::new()
}

fn rel_path(world: &World, root: GameObjectId, id: GameObjectId) -> Option<String> {
    let mut parts = Vec::new();
    let mut cur = id;
    while cur != root {
        parts.push(world.get(cur)?.name.clone());
        cur = world.parent(cur)?;
    }
    parts.reverse();
    Some(parts.join("/"))
}

// ---- track editing ----------------------------------------------------------------------

/// glTF cubic tracks keep tangents beside each value; edited tracks become linear.
fn linearize<T: Copy>(tr: &mut Track<T>) {
    if tr.interpolation == Interpolation::CubicSpline {
        tr.values = tr
            .values
            .chunks(3)
            .filter_map(|c| c.get(1).copied())
            .collect();
        tr.interpolation = Interpolation::Linear;
    }
}

fn set_key<T: Copy>(tr: &mut Track<T>, t: f32, v: T) {
    linearize(tr);
    match tr.times.iter().position(|&k| (k - t).abs() < 0.5 / FPS) {
        Some(i) => tr.values[i] = v,
        None => {
            let i = tr.times.partition_point(|&k| k < t);
            tr.times.insert(i, t);
            tr.values.insert(i, v);
        }
    }
}

fn remove_key<T: Copy>(tr: &mut Track<T>, i: usize) {
    linearize(tr);
    if i < tr.times.len() {
        tr.times.remove(i);
        tr.values.remove(i);
    }
}

fn move_key<T: Copy>(tr: &mut Track<T>, i: usize, t: f32) -> usize {
    linearize(tr);
    if i >= tr.times.len() {
        return i;
    }
    let v = tr.values.remove(i);
    tr.times.remove(i);
    let j = tr.times.partition_point(|&k| k < t);
    tr.times.insert(j, t);
    tr.values.insert(j, v);
    j
}

fn new_track<T: Copy>(t: f32, v: T) -> Track<T> {
    Track {
        times: vec![t],
        values: vec![v],
        interpolation: Interpolation::Linear,
    }
}

fn row_times(c: &AnimationClip, row: &Row) -> Vec<f32> {
    match row {
        Row::Position(i) => c
            .curves
            .get(*i)
            .and_then(|x| x.position.as_ref())
            .map(|t| t.times.clone()),
        Row::Rotation(i) => c
            .curves
            .get(*i)
            .and_then(|x| x.rotation.as_ref())
            .map(|t| t.times.clone()),
        Row::Scale(i) => c
            .curves
            .get(*i)
            .and_then(|x| x.scale.as_ref())
            .map(|t| t.times.clone()),
        Row::Property(i) => c
            .properties
            .get(*i)
            .map(|p| p.curve.keys.iter().map(|k| k.time).collect()),
        Row::Object(i) => c
            .object_curves
            .get(*i)
            .map(|o| o.keys.iter().map(|k| k.time).collect()),
    }
    .unwrap_or_default()
}

fn delete_key(c: &mut AnimationClip, row: &Row, k: usize) {
    match row {
        Row::Position(i) => {
            if let Some(tr) = c.curves.get_mut(*i).and_then(|x| x.position.as_mut()) {
                remove_key(tr, k);
            }
        }
        Row::Rotation(i) => {
            if let Some(tr) = c.curves.get_mut(*i).and_then(|x| x.rotation.as_mut()) {
                remove_key(tr, k);
            }
        }
        Row::Scale(i) => {
            if let Some(tr) = c.curves.get_mut(*i).and_then(|x| x.scale.as_mut()) {
                remove_key(tr, k);
            }
        }
        Row::Property(i) => {
            if let Some(p) = c.properties.get_mut(*i) {
                p.curve.remove_key(k);
            }
        }
        Row::Object(i) => {
            if let Some(o) = c.object_curves.get_mut(*i)
                && k < o.keys.len()
            {
                o.keys.remove(k);
            }
        }
    }
}

fn shift_key(c: &mut AnimationClip, row: &Row, k: usize, t: f32) -> usize {
    match row {
        Row::Position(i) => c
            .curves
            .get_mut(*i)
            .and_then(|x| x.position.as_mut())
            .map_or(k, |tr| move_key(tr, k, t)),
        Row::Rotation(i) => c
            .curves
            .get_mut(*i)
            .and_then(|x| x.rotation.as_mut())
            .map_or(k, |tr| move_key(tr, k, t)),
        Row::Scale(i) => c
            .curves
            .get_mut(*i)
            .and_then(|x| x.scale.as_mut())
            .map_or(k, |tr| move_key(tr, k, t)),
        Row::Property(i) => c.properties.get_mut(*i).map_or(k, |p| {
            let v = p.curve.keys.get(k).map_or(0.0, |x| x.value);
            p.curve.move_key(k, t, v)
        }),
        Row::Object(i) => c.object_curves.get_mut(*i).map_or(k, |o| {
            if k >= o.keys.len() {
                return k;
            }
            let key = o.keys.remove(k);
            let j = o.keys.partition_point(|x| x.time < t);
            o.keys.insert(
                j,
                oxide_assets::ObjectKey {
                    time: t,
                    value: key.value,
                },
            );
            j
        }),
    }
}

/// Keys the current value of `row` at time `t`.
fn key_row_now(c: &mut AnimationClip, world: &World, root: GameObjectId, row: &Row, t: f32) {
    match row {
        Row::Position(i) | Row::Rotation(i) | Row::Scale(i) => {
            let Some(curves) = c.curves.get_mut(*i) else {
                return;
            };
            let Some(l) = find_path(world, root, &curves.path).and_then(|id| local(world, id))
            else {
                return;
            };
            match row {
                Row::Position(_) => set_key(
                    curves.position.get_or_insert_with(|| new_track(t, l.p)),
                    t,
                    l.p,
                ),
                Row::Rotation(_) => set_key(
                    curves.rotation.get_or_insert_with(|| new_track(t, l.r)),
                    t,
                    l.r,
                ),
                _ => set_key(
                    curves.scale.get_or_insert_with(|| new_track(t, l.s)),
                    t,
                    l.s,
                ),
            }
        }
        Row::Property(i) => {
            let Some(p) = c.properties.get_mut(*i) else {
                return;
            };
            let Some(id) = find_path(world, root, &p.path) else {
                return;
            };
            if let Some(v) = read_property(world, id, &p.component, &p.property) {
                set_curve_key(&mut p.curve, t, v);
            }
        }
        Row::Object(i) => {
            let Some(o) = c.object_curves.get_mut(*i) else {
                return;
            };
            let Some(id) = find_path(world, root, &o.path) else {
                return;
            };
            if let Some(v) = read_object_property(world, id, &o.component, &o.property) {
                o.set_key(t, &v);
            }
        }
    }
}

fn set_curve_key(curve: &mut AnimationCurve, t: f32, v: f32) {
    match curve
        .keys
        .iter()
        .position(|k| (k.time - t).abs() < 0.5 / FPS)
    {
        Some(i) => {
            curve.keys[i].value = v;
            curve.update_tangents();
        }
        None => {
            curve.add_key(Keyframe::new(t, v));
        }
    }
}

// ---- preview and recording --------------------------------------------------------------

/// Every animatable value under `root`: transforms and numeric component fields.
pub(crate) fn snapshot(world: &World, root: GameObjectId) -> Snapshot {
    let mut s = Snapshot::default();
    let mut ids = vec![root];
    ids.extend(world.descendants(root));
    for id in ids {
        if let Some(l) = local(world, id) {
            s.transforms.insert(id, l);
        }
        let Some(o) = world.get(id) else { continue };
        for (cid, name, _) in o.component_list() {
            if matches!(name, "Animator" | "Animation") {
                continue;
            }
            let Some(c) = world.component_dyn(id, cid) else {
                continue;
            };
            for (field, v) in c.fields() {
                if let Value::Asset(a) | Value::String(a) = &v {
                    s.objs
                        .insert((id, name.to_owned(), field.to_owned()), a.clone());
                    continue;
                }
                let chans: &[&str] = match v {
                    Value::Float(_) | Value::Int(_) => &[""],
                    Value::Vector2(_) => &["x", "y"],
                    Value::Vector3(_) => &["x", "y", "z"],
                    Value::Vector4(_) => &["x", "y", "z", "w"],
                    Value::Color(_) => &["r", "g", "b", "a"],
                    _ => continue,
                };
                for ch in chans {
                    let prop = if ch.is_empty() {
                        field.to_owned()
                    } else {
                        format!("{field}.{ch}")
                    };
                    if let Some(x) = read_property(world, id, name, &prop) {
                        s.props.insert((id, name.to_owned(), prop), x);
                    }
                }
            }
        }
    }
    s
}

pub(crate) fn restore(world: &mut World, s: &Snapshot) {
    for (id, l) in &s.transforms {
        if let Some(t) = world.transform_mut(*id) {
            t.local_position = l.p;
            t.local_rotation = l.r;
            t.local_scale = l.s;
        }
    }
    for ((id, c, p), x) in &s.props {
        write_property(world, *id, c, p, *x);
    }
    for ((id, c, p), v) in &s.objs {
        write_object_property(world, *id, c, p, v);
    }
}

/// Poses `root` with `clip` at `t` (the editor preview).
fn sample(world: &mut World, root: GameObjectId, clip: &AnimationClip, t: f32) {
    for c in &clip.curves {
        let Some(id) = find_path(world, root, &c.path) else {
            continue;
        };
        let p = c.position.as_ref().and_then(|tr| tr.sample(t));
        let r = c.rotation.as_ref().and_then(|tr| tr.sample(t));
        let s = c.scale.as_ref().and_then(|tr| tr.sample(t));
        if let Some(tf) = world.transform_mut(id) {
            if let Some(p) = p {
                tf.local_position = p;
            }
            if let Some(r) = r {
                tf.local_rotation = r.normalize();
            }
            if let Some(s) = s {
                tf.local_scale = s;
            }
        }
    }
    for p in &clip.properties {
        if let Some(id) = find_path(world, root, &p.path) {
            write_property(world, id, &p.component, &p.property, p.curve.evaluate(t));
        }
    }
    for o in &clip.object_curves {
        if let (Some(id), Some(v)) = (find_path(world, root, &o.path), o.sample(t)) {
            write_object_property(world, id, &o.component, &o.property, v);
        }
    }
}

/// Record mode: anything under `root` that changed since the preview last wrote it gets a
/// key at `t`. Returns whether the clip changed.
pub(crate) fn record(
    world: &World,
    root: GameObjectId,
    clip: &mut AnimationClip,
    last: &Snapshot,
    t: f32,
) -> bool {
    let now = snapshot(world, root);
    let mut changed = false;
    for (id, l) in &now.transforms {
        let Some(before) = last.transforms.get(id) else {
            continue;
        };
        let Some(path) = rel_path(world, root, *id) else {
            continue;
        };
        let moved = (l.p - before.p).length() > 1e-5;
        let turned = l.r.dot(before.r).abs() < 1.0 - 1e-7;
        let scaled = (l.s - before.s).length() > 1e-5;
        if !(moved || turned || scaled) {
            continue;
        }
        let i = match clip.curves.iter().position(|c| c.path == path) {
            Some(i) => i,
            None => {
                clip.curves.push(TransformCurves {
                    path,
                    ..Default::default()
                });
                clip.curves.len() - 1
            }
        };
        let c = &mut clip.curves[i];
        if moved {
            set_key(c.position.get_or_insert_with(|| new_track(t, l.p)), t, l.p);
        }
        if turned {
            set_key(c.rotation.get_or_insert_with(|| new_track(t, l.r)), t, l.r);
        }
        if scaled {
            set_key(c.scale.get_or_insert_with(|| new_track(t, l.s)), t, l.s);
        }
        changed = true;
    }
    for ((id, comp, prop), v) in &now.objs {
        let Some(before) = last.objs.get(&(*id, comp.clone(), prop.clone())) else {
            continue;
        };
        if v == before {
            continue;
        }
        let Some(path) = rel_path(world, root, *id) else {
            continue;
        };
        let existing = clip
            .object_curves
            .iter_mut()
            .find(|o| o.path == path && &o.component == comp && &o.property == prop);
        match existing {
            Some(o) => o.set_key(t, v),
            None => {
                let mut o = ObjectCurve {
                    path,
                    component: comp.clone(),
                    property: prop.clone(),
                    keys: vec![],
                };
                o.set_key(t, v);
                clip.object_curves.push(o);
            }
        }
        changed = true;
    }
    for ((id, comp, prop), x) in &now.props {
        let Some(before) = last.props.get(&(*id, comp.clone(), prop.clone())) else {
            continue;
        };
        if (x - before).abs() < 1e-6 {
            continue;
        }
        let Some(path) = rel_path(world, root, *id) else {
            continue;
        };
        match clip
            .properties
            .iter_mut()
            .find(|p| p.path == path && &p.component == comp && &p.property == prop)
        {
            Some(p) => set_curve_key(&mut p.curve, t, *x),
            None => clip.properties.push(PropertyCurve {
                path,
                component: comp.clone(),
                property: prop.clone(),
                curve: AnimationCurve::new([Keyframe::new(t, *x)]),
            }),
        }
        changed = true;
    }
    changed
}

fn end_preview(w: &mut AnimationWindow, world: &mut World) {
    if let Some((root, snap)) = w.backup.take()
        && world.is_alive(root)
    {
        restore(world, &snap);
    }
    w.preview = false;
    w.recording = false;
    w.playing = false;
}

/// Called every frame (window open or not) so preview ends when Play starts or the window
/// closes.
pub fn end_preview_if(st: &mut EditorState, engine: &mut Engine, open: bool) {
    if st.animation.preview && (!open || st.play != crate::PlayState::Edit) {
        end_preview(&mut st.animation, &mut engine.world);
    }
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &mut st.animation;
    let selected = st.selection.last().copied();
    let root = selected.and_then(|id| animated_root(&engine.world, id));
    if root != w.target {
        end_preview(w, &mut engine.world);
        w.target = root;
        w.selected = None;
        w.clip = None;
    }
    let Some(root) = root else {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| match selected {
            Some(id) => {
                let name = engine
                    .world
                    .get(id)
                    .map_or(String::new(), |o| o.name.clone());
                ui.label(format!(
                    "To begin animating {name}, create an Animation Clip."
                ));
                if ui.button("Create").clicked() {
                    match create_first_clip(engine, id) {
                        Ok(()) => {}
                        Err(e) => st.status = e,
                    }
                }
            }
            None => {
                ui.weak("Select a GameObject to animate.");
            }
        });
        return;
    };
    let clips = target_clips(&engine.world, engine.world.resource::<AssetServer>(), root);
    let w = &mut st.animation;
    if w.clip.as_ref().is_none_or(|c| !clips.contains(c)) {
        w.clip = clips.first().cloned();
    }
    let Some(path) = w.clip.clone() else {
        ui.weak("This object's Animator Controller has no clips yet. Create one with Create > Animation > Animation Clip and drag it into the Animator window.");
        return;
    };
    let assets = engine.world.resource_mut::<AssetServer>();
    if let Err(e) = assets.ensure_loaded(&path) {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
        return;
    }
    let h: Handle<AnimationClip> = Handle::from_path(path.clone());
    let Some(mut clip) = assets.animations.get(&h).cloned() else {
        return;
    };
    let before = clip.clone();
    let editable = path.ends_with(".anim");
    let in_edit_mode = st.play == crate::PlayState::Edit;

    // Record: keys whatever changed since the preview last posed the object.
    if w.preview && w.recording && editable && in_edit_mode {
        let t = w.time;
        record(&engine.world, root, &mut clip, &w.last, t);
    }

    // Toolbar.
    let mut start_preview = false;
    ui.horizontal(|ui| {
        // The record button's dot is drawn: "●" isn't in the editor fonts.
        let rec = ui
            .add_enabled(
                editable && in_edit_mode,
                egui::Button::new("   ").selected(w.recording),
            )
            .on_hover_text("Record: changes to the object become keys at the playhead");
        let dot = if w.recording {
            Color32::from_rgb(240, 60, 60)
        } else {
            Color32::from_rgb(170, 90, 90)
        };
        ui.painter().circle_filled(rec.rect.center(), 5.0, dot);
        if rec.clicked() {
            w.recording = !w.recording;
            if w.recording {
                start_preview = true;
            }
        }
        if ui
            .add_enabled(
                in_edit_mode,
                egui::Button::new("Preview").selected(w.preview),
            )
            .clicked()
        {
            if w.preview {
                end_preview(w, &mut engine.world);
            } else {
                start_preview = true;
            }
        }
        if ui
            .add_enabled(
                in_edit_mode,
                egui::Button::new(if w.playing { "⏸" } else { "▶" }),
            )
            .on_hover_text("Play the clip in the Scene")
            .clicked()
        {
            w.playing = !w.playing;
            start_preview |= w.playing;
        }
        ui.separator();
        egui::ComboBox::from_id_salt("anim-clip")
            .selected_text(path.rsplit('/').next().unwrap_or(&path))
            .show_ui(ui, |ui| {
                for c in &clips {
                    if ui
                        .selectable_label(
                            Some(c) == w.clip.as_ref(),
                            c.rsplit('/').next().unwrap_or(c),
                        )
                        .on_hover_text(c)
                        .clicked()
                    {
                        w.clip = Some(c.clone());
                    }
                }
            });
        ui.label("Time");
        let mut frame = (w.time * FPS).round() as i64;
        if ui
            .add(
                egui::DragValue::new(&mut frame)
                    .range(0..=100_000)
                    .suffix(" f"),
            )
            .changed()
        {
            w.time = frame as f32 / FPS;
            start_preview = true;
        }
        ui.weak(format!("{:.2} s", w.time));
        ui.separator();
        ui.add_enabled_ui(editable, |ui| {
            ui.menu_button("Add Property", |ui| {
                add_property_menu(ui, &engine.world, root, &mut clip, w.time)
            });
            if ui
                .button("Add Keyframe")
                .on_hover_text("Key every property at the playhead")
                .clicked()
            {
                let rows = rows(&clip);
                for r in &rows {
                    key_row_now(&mut clip, &engine.world, root, r, w.time);
                }
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.selectable_value(&mut w.curves_view, true, "Curves");
            ui.selectable_value(&mut w.curves_view, false, "Dopesheet");
        });
    });
    if !editable {
        ui.weak("This clip is inside a model and is read-only. Use Extract Animations on the model to get an editable .anim copy.");
    }
    if let Some(e) = &w.error {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
    }
    if start_preview && !w.preview && in_edit_mode {
        w.backup = Some((root, snapshot(&engine.world, root)));
        w.preview = true;
    }
    if w.playing {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        w.time += dt;
        if w.time > clip.length.max(1.0 / FPS) {
            w.time = 0.0;
        }
        ui.ctx().request_repaint();
    }

    let rows = rows(&clip);
    if w.curves_view {
        curves_view(ui, w, &mut clip, &rows, editable);
    } else {
        dopesheet(ui, w, &mut clip, &rows, editable, &engine.world, root);
    }

    // Save edits, then pose the object with the clip at the playhead.
    if editable && !clips_equal(&clip, &before) {
        clip.recalculate_length();
        if let Err(e) = engine
            .world
            .resource_mut::<AssetServer>()
            .save_animation_clip(&path, &clip)
        {
            w.error = Some(e);
        }
    }
    if w.preview && in_edit_mode {
        sample(&mut engine.world, root, &clip, w.time);
        w.last = snapshot(&engine.world, root);
    }
}

/// Keys `path`'s sprites (a texture's slices in natural order, or one sprite) on the
/// root SpriteRenderer at 12 fps, as dropping sprites into Unity's Animation window does.
/// Returns how many frames were added.
pub fn add_flipbook(clip: &mut AnimationClip, assets: &AssetServer, path: &str) -> usize {
    let mut sprites: Vec<String> = if path.contains('#') {
        vec![path.to_owned()]
    } else {
        assets
            .sprite_paths()
            .into_iter()
            .filter(|s| s.starts_with(&format!("{path}#")))
            .collect()
    };
    // Natural order: walk_2 before walk_10.
    sprites.sort_by_key(|s| {
        let digits: String = s.chars().rev().take_while(|c| c.is_ascii_digit()).collect();
        let n: u64 = digits
            .chars()
            .rev()
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        (
            s.trim_end_matches(|c: char| c.is_ascii_digit()).to_owned(),
            n,
        )
    });
    if sprites.is_empty() {
        return 0;
    }
    let i = match clip.object_curves.iter().position(|o| {
        o.path.is_empty() && o.component == "SpriteRenderer" && o.property == "sprite"
    }) {
        Some(i) => i,
        None => {
            clip.object_curves.push(ObjectCurve {
                path: String::new(),
                component: "SpriteRenderer".into(),
                property: "sprite".into(),
                keys: vec![],
            });
            clip.object_curves.len() - 1
        }
    };
    for (k, sp) in sprites.iter().enumerate() {
        clip.object_curves[i].set_key(k as f32 / 12.0, sp);
    }
    clip.recalculate_length();
    sprites.len()
}

fn clips_equal(a: &AnimationClip, b: &AnimationClip) -> bool {
    a == b
}

fn rows(c: &AnimationClip) -> Vec<Row> {
    let mut out = Vec::new();
    for (i, t) in c.curves.iter().enumerate() {
        if t.position.is_some() {
            out.push(Row::Position(i));
        }
        if t.rotation.is_some() {
            out.push(Row::Rotation(i));
        }
        if t.scale.is_some() {
            out.push(Row::Scale(i));
        }
    }
    out.extend((0..c.properties.len()).map(Row::Property));
    out.extend((0..c.object_curves.len()).map(Row::Object));
    out
}

fn row_label(c: &AnimationClip, row: &Row) -> String {
    let name = |p: &str| {
        if p.is_empty() {
            "(root)".to_owned()
        } else {
            p.rsplit('/').next().unwrap_or(p).to_owned()
        }
    };
    match row {
        Row::Position(i) => format!("{} : Position", name(&c.curves[*i].path)),
        Row::Rotation(i) => format!("{} : Rotation", name(&c.curves[*i].path)),
        Row::Scale(i) => format!("{} : Scale", name(&c.curves[*i].path)),
        Row::Property(i) => {
            let p = &c.properties[*i];
            format!("{} : {}.{}", name(&p.path), p.component, p.property)
        }
        Row::Object(i) => {
            let o = &c.object_curves[*i];
            format!("{} : {}.{}", name(&o.path), o.component, o.property)
        }
    }
}

fn add_property_menu(
    ui: &mut egui::Ui,
    world: &World,
    root: GameObjectId,
    clip: &mut AnimationClip,
    t: f32,
) {
    let mut ids = vec![root];
    ids.extend(world.descendants(root));
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            for id in ids {
                let Some(path) = rel_path(world, root, id) else {
                    continue;
                };
                let label = if path.is_empty() {
                    world.get(id).map_or("(root)".into(), |o| o.name.clone())
                } else {
                    path.clone()
                };
                ui.menu_button(label, |ui| {
                    ui.menu_button("Transform", |ui| {
                        for (name, which) in [("Position", 0), ("Rotation", 1), ("Scale", 2)] {
                            if ui.button(name).clicked() {
                                let l = local(world, id).unwrap_or_default();
                                let i = match clip.curves.iter().position(|c| c.path == path) {
                                    Some(i) => i,
                                    None => {
                                        clip.curves.push(TransformCurves {
                                            path: path.clone(),
                                            ..Default::default()
                                        });
                                        clip.curves.len() - 1
                                    }
                                };
                                let c = &mut clip.curves[i];
                                // Unity adds two keys: now and at the end of the clip.
                                let end = clip.length.max(1.0);
                                match which {
                                    0 => {
                                        let tr =
                                            c.position.get_or_insert_with(|| new_track(t, l.p));
                                        set_key(tr, end, l.p);
                                    }
                                    1 => {
                                        let tr =
                                            c.rotation.get_or_insert_with(|| new_track(t, l.r));
                                        set_key(tr, end, l.r);
                                    }
                                    _ => {
                                        let tr = c.scale.get_or_insert_with(|| new_track(t, l.s));
                                        set_key(tr, end, l.s);
                                    }
                                }
                                ui.close();
                            }
                        }
                    });
                    let Some(o) = world.get(id) else { return };
                    for (cid, comp, _) in o.component_list() {
                        if matches!(comp, "Animator" | "Animation") {
                            continue;
                        }
                        let Some(c) = world.component_dyn(id, cid) else {
                            continue;
                        };
                        let fields: Vec<(String, Value)> = c
                            .fields()
                            .into_iter()
                            .map(|(k, v)| (k.to_owned(), v))
                            .collect();
                        let objects: Vec<String> = fields
                            .iter()
                            .filter(|(_, v)| matches!(v, Value::Asset(_)))
                            .map(|(f, _)| f.clone())
                            .collect();
                        let props: Vec<String> = fields
                            .iter()
                            .flat_map(|(f, v)| {
                                let chans: &[&str] = match v {
                                    Value::Float(_) | Value::Int(_) | Value::Bool(_) => &[""],
                                    Value::Vector2(_) => &["x", "y"],
                                    Value::Vector3(_) => &["x", "y", "z"],
                                    Value::Vector4(_) => &["x", "y", "z", "w"],
                                    Value::Color(_) => &["r", "g", "b", "a"],
                                    _ => &[],
                                };
                                chans.iter().map(move |ch| {
                                    if ch.is_empty() {
                                        f.clone()
                                    } else {
                                        format!("{f}.{ch}")
                                    }
                                })
                            })
                            .collect();
                        if props.is_empty() && objects.is_empty() {
                            continue;
                        }
                        ui.menu_button(crate::inspector::nicify(comp), |ui| {
                            for f in &objects {
                                if ui.button(format!("{f} (keys)")).clicked() {
                                    let v = read_object_property(world, id, comp, f)
                                        .unwrap_or_default();
                                    let mut o = ObjectCurve {
                                        path: path.clone(),
                                        component: comp.to_owned(),
                                        property: f.clone(),
                                        keys: vec![],
                                    };
                                    o.set_key(t, &v);
                                    clip.object_curves.push(o);
                                    ui.close();
                                }
                            }
                            for p in props {
                                if ui.button(&p).clicked() {
                                    let v = read_property(world, id, comp, &p).unwrap_or(0.0);
                                    let end = clip.length.max(1.0);
                                    clip.properties.push(PropertyCurve {
                                        path: path.clone(),
                                        component: comp.to_owned(),
                                        property: p.clone(),
                                        curve: AnimationCurve::new([
                                            Keyframe::new(t.min(end), v),
                                            Keyframe::new(end, v),
                                        ]),
                                    });
                                    ui.close();
                                }
                            }
                        });
                    }
                });
            }
        });
}

fn dopesheet(
    ui: &mut egui::Ui,
    w: &mut AnimationWindow,
    clip: &mut AnimationClip,
    rows: &[Row],
    editable: bool,
    world: &World,
    root: GameObjectId,
) {
    if w.span <= 0.0 {
        w.span = (clip.length * 1.25).max(1.0);
    }
    let avail = ui.available_size();
    let (rect, _) = ui.allocate_exact_size(avail, Sense::hover());
    let left = Rect::from_min_size(rect.min, vec2(LEFT_W, rect.height()));
    let ruler = Rect::from_min_max(
        pos2(left.right(), rect.top()),
        pos2(rect.right(), rect.top() + 22.0),
    );
    let sheet = Rect::from_min_max(pos2(left.right(), ruler.bottom()), rect.max);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(34));
    painter.rect_filled(left, 0.0, Color32::from_gray(40));
    painter.rect_filled(ruler, 0.0, Color32::from_gray(46));
    let span = w.span;
    let x_of = |t: f32| sheet.left() + 8.0 + t / span * (sheet.width() - 16.0);
    let t_of = |x: f32| ((x - sheet.left() - 8.0) / (sheet.width() - 16.0) * span).max(0.0);

    // Ruler with frame labels.
    let step = [
        1.0 / FPS,
        5.0 / FPS,
        10.0 / FPS,
        0.25,
        0.5,
        1.0,
        2.0,
        5.0,
        10.0,
    ]
    .into_iter()
    .find(|s| s / span * sheet.width() > 50.0)
    .unwrap_or(10.0);
    let mut t = 0.0;
    while t <= span + 1e-4 {
        let x = x_of(t);
        painter.line_segment(
            [pos2(x, ruler.bottom() - 6.0), pos2(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_gray(52)),
        );
        // Unity's seconds:frames labels.
        let frames = (t * FPS).round() as i64;
        let label = format!("{}:{:02}", frames / FPS as i64, frames % FPS as i64);
        painter.text(
            pos2(x + 2.0, ruler.top() + 3.0),
            Align2::LEFT_TOP,
            label,
            FontId::proportional(10.0),
            Color32::from_gray(150),
        );
        t += step;
    }
    // Clip end.
    let end_x = x_of(clip.length);
    painter.line_segment(
        [pos2(end_x, sheet.top()), pos2(end_x, sheet.bottom())],
        Stroke::new(1.0, Color32::from_gray(90)),
    );

    let ruler_resp = ui.interact(ruler, ui.id().with("anim-ruler"), Sense::click_and_drag());
    if (ruler_resp.clicked() || ruler_resp.dragged())
        && let Some(p) = ruler_resp.interact_pointer_pos()
    {
        w.time = snap(t_of(p.x));
        w.playing = false;
    }
    let sheet_resp = ui.interact(sheet, ui.id().with("anim-sheet"), Sense::click_and_drag());
    // Dropping a sprite sheet (or sprites) makes a flipbook (Unity).
    if editable
        && let Some(payload) = sheet_resp.dnd_release_payload::<crate::project::AssetPayload>()
        && world.has_component::<oxide_scene::SpriteRenderer>(root)
    {
        let n = add_flipbook(clip, world.resource::<AssetServer>(), &payload.0);
        if n > 0 {
            w.span = (n as f32 / 12.0 * 1.25).max(1.0);
        }
    }
    if sheet_resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            w.span = (w.span * (-scroll * 0.003).exp()).clamp(0.1, 600.0);
        }
        crate::shortcuts::claim_keys(ui.ctx());
    }

    // Rows.
    let pointer = sheet_resp.interact_pointer_pos().or(sheet_resp.hover_pos());
    let mut hit = None;
    for (ri, row) in rows.iter().enumerate() {
        let y = sheet.top() + ri as f32 * ROW_H;
        if y > rect.bottom() {
            break;
        }
        let label_rect = Rect::from_min_size(pos2(left.left(), y), vec2(LEFT_W, ROW_H));
        let sel_row = w.selected.as_ref().is_some_and(|(r, _)| r == row);
        if sel_row {
            painter.rect_filled(label_rect, 0.0, Color32::from_rgb(44, 70, 110));
        }
        painter.text(
            pos2(left.left() + 8.0, y + ROW_H / 2.0),
            Align2::LEFT_CENTER,
            row_label(clip, row),
            FontId::proportional(12.0),
            Color32::from_gray(210),
        );
        painter.line_segment(
            [pos2(rect.left(), y + ROW_H), pos2(rect.right(), y + ROW_H)],
            Stroke::new(1.0, Color32::from_gray(44)),
        );
        for (k, kt) in row_times(clip, row).into_iter().enumerate() {
            let c = pos2(x_of(kt), y + ROW_H / 2.0);
            let sel = w.selected.as_ref() == Some(&(row.clone(), k));
            let d = 5.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + vec2(0.0, -d),
                    c + vec2(d, 0.0),
                    c + vec2(0.0, d),
                    c + vec2(-d, 0.0),
                ],
                if sel {
                    Color32::from_rgb(90, 170, 255)
                } else {
                    Color32::from_gray(200)
                },
                Stroke::new(1.0, Color32::BLACK),
            ));
            if pointer.is_some_and(|p| p.distance(c) < 7.0) {
                hit = Some((row.clone(), k));
            }
        }
        // Clicking a label selects the row (for the Curves view).
        let lr = ui.interact(label_rect, ui.id().with(("anim-row", ri)), Sense::click());
        if lr.clicked() {
            w.selected = Some((row.clone(), usize::MAX));
        }
    }
    if rows.is_empty() {
        painter.text(
            sheet.center(),
            Align2::CENTER_CENTER,
            "No properties. Use Add Property, or press ● and move the object.",
            FontId::proportional(13.0),
            Color32::from_gray(140),
        );
    }
    // Playhead.
    let px = x_of(w.time);
    painter.line_segment(
        [pos2(px, ruler.top()), pos2(px, rect.bottom())],
        Stroke::new(
            1.5,
            if w.recording {
                Color32::from_rgb(240, 60, 60)
            } else {
                Color32::from_rgb(240, 240, 240)
            },
        ),
    );

    if !editable {
        return;
    }
    if sheet_resp.drag_started() {
        let at = ui.input(|i| i.pointer.press_origin());
        w.dragging = at.and_then(|p| {
            rows.iter().enumerate().find_map(|(ri, row)| {
                let y = sheet.top() + ri as f32 * ROW_H + ROW_H / 2.0;
                row_times(clip, row)
                    .into_iter()
                    .position(|kt| pos2(x_of(kt), y).distance(p) < 7.0)
                    .map(|k| (row.clone(), k))
            })
        });
        if w.dragging.is_some() {
            w.selected = w.dragging.clone();
        }
    }
    if sheet_resp.dragged()
        && let (Some((row, k)), Some(p)) = (w.dragging.clone(), pointer)
    {
        let nk = shift_key(clip, &row, k, snap(t_of(p.x)));
        w.dragging = Some((row.clone(), nk));
        w.selected = Some((row, nk));
    }
    if sheet_resp.drag_stopped() {
        w.dragging = None;
    }
    if sheet_resp.clicked() {
        w.selected = hit.clone();
    }
    if sheet_resp.double_clicked()
        && let Some(p) = pointer
    {
        // Key the row under the pointer at that time with the object's current value.
        let ri = ((p.y - sheet.top()) / ROW_H).floor() as usize;
        if let Some(row) = rows.get(ri) {
            key_row_now(clip, world, root, row, snap(t_of(p.x)));
        }
    }
    if sheet_resp.hovered()
        && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        && let Some((row, k)) = w.selected.take()
        && k != usize::MAX
    {
        delete_key(clip, &row, k);
    }
}

fn curves_view(
    ui: &mut egui::Ui,
    w: &mut AnimationWindow,
    clip: &mut AnimationClip,
    rows: &[Row],
    editable: bool,
) {
    let Some(Row::Property(i)) = w
        .selected
        .as_ref()
        .map(|(r, _)| r.clone())
        .or_else(|| rows.iter().find(|r| matches!(r, Row::Property(_))).cloned())
    else {
        ui.weak("Curves edit property curves. Pick a property row in the Dopesheet (or add one with Add Property).");
        return;
    };
    let Some(p) = clip.properties.get_mut(i) else {
        return;
    };
    ui.label(format!(
        "{} : {}.{}",
        if p.path.is_empty() { "(root)" } else { &p.path },
        p.component,
        p.property
    ));
    ui.add_enabled_ui(editable, |ui| {
        crate::curve_editor::curves_inline(
            ui,
            &format!("anim-curve-{i}"),
            &mut [(&mut p.curve, Color32::from_rgb(230, 80, 80))],
        );
    });
    let _ = w;
}

/// Unity's "Create" in an empty Animation window: a clip and controller for the object,
/// and an Animator that plays it.
fn create_first_clip(engine: &mut Engine, id: GameObjectId) -> Result<(), String> {
    let name = engine
        .world
        .get(id)
        .map_or("Object".into(), |o| o.name.clone());
    let assets = engine.world.resource_mut::<AssetServer>();
    let root = assets.full_path("");
    let clip_path = crate::asset_ops::unique_path(&root, &format!("Assets/Animations/{name}.anim"));
    let ctrl_path =
        crate::asset_ops::unique_path(&root, &format!("Assets/Animations/{name}.controller"));
    let clip = AnimationClip {
        name: name.clone(),
        looping: true,
        length: 1.0,
        ..Default::default()
    };
    let ch = assets.save_animation_clip(&clip_path, &clip)?;
    let ctrl = AnimatorController::from_clips(&name, &[(name.clone(), ch)]);
    let h = assets.save_animator_controller(&ctrl_path, &ctrl)?;
    if !engine.world.has_component::<Animator>(id) {
        engine.world.add_component(id, Animator::new(h));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_key_editing_keeps_order_and_drops_cubic_tangents() {
        let mut tr = Track {
            times: vec![0.0, 1.0],
            values: vec![
                Vector3::ZERO,
                Vector3::ZERO,
                Vector3::ZERO,
                Vector3::X,
                Vector3::X,
                Vector3::X,
            ],
            interpolation: Interpolation::CubicSpline,
        };
        set_key(&mut tr, 0.5, Vector3::Y);
        assert_eq!(tr.interpolation, Interpolation::Linear);
        assert_eq!(tr.times, vec![0.0, 0.5, 1.0]);
        assert_eq!(tr.values[1], Vector3::Y);
        set_key(&mut tr, 0.5, Vector3::Z);
        assert_eq!(tr.times.len(), 3, "same frame replaces");
        let j = move_key(&mut tr, 0, 2.0);
        assert_eq!(j, 2);
        assert_eq!(tr.times, vec![0.5, 1.0, 2.0]);
        remove_key(&mut tr, 1);
        assert_eq!(tr.values, vec![Vector3::Z, Vector3::ZERO]);
    }

    #[test]
    fn recording_keys_moved_objects_and_changed_fields() {
        let mut w = World::new();
        oxide_scene::register_builtin(&mut w.registry);
        let root = w.create("Hero");
        let arm = w.create("Arm");
        w.set_parent(arm, Some(root), false).unwrap();
        w.add_component(arm, oxide_scene::Light::default());
        let last = snapshot(&w, root);
        w.transform_mut(arm).unwrap().local_position = Vector3::new(0.0, 2.0, 0.0);
        w.get_component_mut::<oxide_scene::Light>(arm)
            .unwrap()
            .intensity = 7.0;
        let mut clip = AnimationClip::default();
        assert!(record(&w, root, &mut clip, &last, 0.5));
        assert_eq!(clip.curves.len(), 1);
        assert_eq!(clip.curves[0].path, "Arm");
        assert_eq!(
            clip.curves[0].position.as_ref().unwrap().sample(0.5),
            Some(Vector3::new(0.0, 2.0, 0.0))
        );
        let p = clip
            .properties
            .iter()
            .find(|p| p.property == "intensity")
            .expect("intensity keyed");
        assert_eq!(p.component, "Light");
        assert_eq!(p.curve.evaluate(0.5), 7.0);
        // Nothing changed since: nothing more is recorded.
        let now = snapshot(&w, root);
        assert!(!record(&w, root, &mut clip, &now, 0.5));
    }
}
