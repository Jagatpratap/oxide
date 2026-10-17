//! The Audio Mixer window: the project's mixers, a mixer's snapshots and group tree, a
//! channel strip per group (fader, Mute/Solo/Bypass, effects) and the selected group's
//! effect settings with "Expose to script". With a snapshot other than the first selected,
//! edits are stored in that snapshot. Edits save to the `.mixer` and play live.
//! Reference: <https://docs.unity3d.com/Manual/AudioMixerOverview.html>

use crate::EditorState;
use egui::{Color32, RichText, vec2};
use oxide_assets::{AssetServer, Handle};
use oxide_audio::mixer::{
    AudioEffectType, AudioMixer, AudioMixerEffect, AudioMixerGroup, AudioMixerValue,
    ExposedParameter,
};
use oxide_core::reflect::{Reflect, Value};
use oxide_engine::Engine;

#[derive(Default)]
pub struct AudioMixerWindow {
    path: Option<String>,
    snapshot: usize,
    group: usize,
    new_group: String,
    error: Option<String>,
}

impl AudioMixerWindow {
    pub fn open(&mut self, path: &str) {
        if self.path.as_deref() != Some(path) {
            *self = Self {
                path: Some(path.to_owned()),
                ..Default::default()
            };
        }
    }
}

/// Group names of a mixer file, loaded or not (for Audio Source Output pickers).
pub fn mixer_groups(assets: &AssetServer, path: &str) -> Vec<String> {
    let h: Handle<AudioMixer> = Handle::from_path(path);
    if let Some(m) = assets.data.get(&h) {
        return m.groups.iter().map(|g| g.name.clone()).collect();
    }
    let Ok(text) = std::fs::read_to_string(assets.full_path(path)) else {
        return Vec::new();
    };
    let Ok(v) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text) else {
        return Vec::new();
    };
    // `fields: groups: !List [ !Object { name: !String ... } ]`
    fn strip(v: &serde_yaml_ng::Value) -> &serde_yaml_ng::Value {
        match v {
            serde_yaml_ng::Value::Tagged(t) => strip(&t.value),
            other => other,
        }
    }
    let groups = strip(&v["fields"]["groups"]);
    groups
        .as_sequence()
        .map(|s| {
            s.iter()
                .filter_map(|g| strip(&strip(g)["name"]).as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// A parameter's value in `snapshot` (0 = the groups' own values).
fn value(m: &AudioMixer, snapshot: usize, param: &str) -> f32 {
    if snapshot > 0
        && let Some(v) = m
            .snapshots
            .get(snapshot)
            .and_then(|s| s.values.iter().find(|v| v.parameter == param))
    {
        return v.value;
    }
    m.base_values().get(param).copied().unwrap_or(0.0)
}

/// Sets a parameter: in the groups for the first snapshot, else as a snapshot override.
fn set_value(m: &mut AudioMixer, snapshot: usize, param: &str, x: f32) {
    if snapshot > 0 {
        if let Some(s) = m.snapshots.get_mut(snapshot) {
            match s.values.iter_mut().find(|v| v.parameter == param) {
                Some(v) => v.value = x,
                None => s.values.push(AudioMixerValue {
                    parameter: param.to_owned(),
                    value: x,
                }),
            }
        }
        return;
    }
    let mut parts = param.split('/');
    let (Some(g), Some(a)) = (parts.next(), parts.next()) else {
        return;
    };
    let Some(group) = m.groups.iter_mut().find(|x| x.name == g) else {
        return;
    };
    match (a, parts.next()) {
        ("volume", None) => group.volume = x,
        ("pitch", None) => group.pitch = x,
        (i, Some(field)) => {
            if let Ok(i) = i.parse::<usize>()
                && let Some(e) = group.effects.get_mut(i)
            {
                let _ = e.set_field(field, Value::Float(x as f64));
            }
        }
        _ => {}
    }
}

fn param_range(field: &str) -> (f32, f32, &'static str) {
    match field {
        "volume" | "drive" | "makeup_gain" | "send_level" => (-80.0, 20.0, " dB"),
        "threshold" => (-60.0, 0.0, " dB"),
        "pitch" => (0.1, 3.0, ""),
        "cutoff" => (10.0, 22000.0, " Hz"),
        "delay" | "attack" | "release" => (1.0, 2000.0, " ms"),
        "ratio" => (1.0, 20.0, ":1"),
        "feedback" => (-60.0, 1.0, ""),
        _ => (0.0, 1.0, ""),
    }
}

/// Groups in tree order with their depth.
fn tree(m: &AudioMixer) -> Vec<(usize, usize)> {
    fn walk(m: &AudioMixer, i: usize, depth: usize, out: &mut Vec<(usize, usize)>) {
        out.push((i, depth));
        let name = &m.groups[i].name;
        for (j, g) in m.groups.iter().enumerate() {
            if &g.parent == name && j != i && !out.iter().any(|(k, _)| *k == j) {
                walk(m, j, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    for (i, g) in m.groups.iter().enumerate() {
        if (g.parent.is_empty() || m.group(&g.parent).is_none())
            && !out.iter().any(|(k, _)| *k == i)
        {
            walk(m, i, 0, &mut out);
        }
    }
    out
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let mixers = engine
        .world
        .resource::<AssetServer>()
        .paths_of_type("AudioMixer");
    let w = &mut st.audio_mixer;
    if w.path.is_none()
        && let Some(first) = mixers.first()
    {
        w.path = Some(first.clone());
    }
    let Some(path) = w.path.clone() else {
        ui.weak("No Audio Mixer in the project. Create one with Create > Audio Mixer.");
        return;
    };
    let assets = engine.world.resource_mut::<AssetServer>();
    let h: Handle<AudioMixer> = Handle::from_path(path.clone());
    let Some(mut m) = assets.data_asset(&h).cloned() else {
        ui.colored_label(
            Color32::from_rgb(230, 120, 100),
            format!("cannot load {path}"),
        );
        return;
    };
    let before = m.clone();
    w.snapshot = w.snapshot.min(m.snapshots.len().saturating_sub(1));
    w.group = w.group.min(m.groups.len().saturating_sub(1));

    ui.horizontal(|ui| {
        ui.label(RichText::new(path.rsplit('/').next().unwrap_or(&path)).strong());
        if w.snapshot > 0 {
            ui.colored_label(
                Color32::from_rgb(230, 190, 90),
                format!("Editing snapshot \"{}\"", m.snapshots[w.snapshot].name),
            );
        }
        if let Some(e) = &w.error {
            ui.colored_label(Color32::from_rgb(230, 120, 100), e);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(
                format!("Exposed Parameters ({})", m.exposed_parameters.len()),
                |ui| {
                    let mut remove = None;
                    for (i, e) in m.exposed_parameters.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(110.0));
                            ui.weak(&e.parameter);
                            if ui.small_button("×").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        m.exposed_parameters.remove(i);
                    }
                    if m.exposed_parameters.is_empty() {
                        ui.weak("Right-click a fader or parameter to expose it.");
                    }
                },
            );
        });
    });
    ui.separator();
    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        // Left column: mixers, snapshots, groups.
        ui.allocate_ui(vec2(200.0, height), |ui| {
            ui.set_width(200.0);
            ui.vertical(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt("mixer-left")
                    .show(ui, |ui| {
                        ui.label(RichText::new("Mixers").strong());
                        for p in &mixers {
                            if ui
                                .selectable_label(*p == path, p.rsplit('/').next().unwrap_or(p))
                                .on_hover_text(p)
                                .clicked()
                            {
                                w.path = Some(p.clone());
                                w.snapshot = 0;
                                w.group = 0;
                            }
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Snapshots").strong());
                            if ui.small_button("+").clicked() {
                                let n = m.snapshots.len();
                                m.snapshots.push(oxide_audio::mixer::AudioMixerSnapshot {
                                    name: format!("Snapshot {n}"),
                                    values: vec![],
                                });
                                w.snapshot = n;
                            }
                        });
                        let mut remove = None;
                        for (i, s) in m.snapshots.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                let r = ui.selectable_label(
                                    w.snapshot == i,
                                    if i == 0 {
                                        format!("{} (start)", s.name)
                                    } else {
                                        s.name.clone()
                                    },
                                );
                                if r.clicked() {
                                    w.snapshot = i;
                                }
                                if i > 0 {
                                    r.context_menu(|ui| {
                                        ui.text_edit_singleline(&mut s.name);
                                        if ui.button("Delete").clicked() {
                                            remove = Some(i);
                                            ui.close();
                                        }
                                    });
                                }
                            });
                        }
                        if let Some(i) = remove {
                            m.snapshots.remove(i);
                            w.snapshot = 0;
                        }
                        ui.add_space(6.0);
                        ui.label(RichText::new("Groups").strong());
                        for (i, depth) in tree(&m) {
                            ui.horizontal(|ui| {
                                ui.add_space(depth as f32 * 12.0);
                                if ui
                                    .selectable_label(w.group == i, &m.groups[i].name)
                                    .clicked()
                                {
                                    w.group = i;
                                }
                            });
                        }
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut w.new_group)
                                    .hint_text("New group")
                                    .desired_width(110.0),
                            );
                            if ui
                                .small_button("+ Child")
                                .on_hover_text("Add a group under the selected one")
                                .clicked()
                            {
                                let name = w.new_group.trim().to_owned();
                                if !name.is_empty() && m.group(&name).is_none() {
                                    let parent = m.groups[w.group].name.clone();
                                    m.groups.push(AudioMixerGroup::new(&name, &parent));
                                    w.group = m.groups.len() - 1;
                                    w.new_group.clear();
                                }
                            }
                        });
                        if w.group > 0 && ui.small_button("Delete Group").clicked() {
                            let name = m.groups[w.group].name.clone();
                            let parent = m.groups[w.group].parent.clone();
                            m.groups.remove(w.group);
                            // Children move up to the deleted group's parent.
                            for g in &mut m.groups {
                                if g.parent == name {
                                    g.parent = parent.clone();
                                }
                            }
                            w.group = 0;
                        }
                    });
            });
        });
        ui.separator();
        // Channel strips.
        let strips_w = (ui.available_width() - 300.0).max(200.0);
        ui.allocate_ui(vec2(strips_w, height), |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("mixer-strips")
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        for (i, _) in tree(&m) {
                            strip(ui, w, &mut m, i, height - 20.0);
                        }
                    });
                });
        });
        ui.separator();
        ui.allocate_ui(vec2(290.0, height), |ui| {
            ui.set_width(290.0);
            ui.vertical(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt("mixer-props")
                    .show(ui, |ui| group_inspector(ui, w, &mut m));
            });
        });
    });

    if m != before {
        match engine
            .world
            .resource_mut::<AssetServer>()
            .save_data(&path, Box::new(m))
        {
            Ok(()) => st.audio_mixer.error = None,
            Err(e) => st.audio_mixer.error = Some(e),
        }
    }
}

/// One channel strip: name, dB, fader, M/S/B and the effect chain.
fn strip(ui: &mut egui::Ui, w: &mut AudioMixerWindow, m: &mut AudioMixer, i: usize, height: f32) {
    let sel = w.group == i;
    let frame = egui::Frame::new()
        .fill(if sel {
            Color32::from_gray(52)
        } else {
            Color32::from_gray(40)
        })
        .stroke(egui::Stroke::new(
            1.0,
            if sel {
                Color32::from_rgb(80, 140, 230)
            } else {
                Color32::from_gray(30)
            },
        ))
        .inner_margin(4.0)
        .corner_radius(3.0);
    frame.show(ui, |ui| {
        ui.set_width(92.0);
        ui.set_min_height(height.min(420.0));
        ui.vertical_centered(|ui| {
            let name = m.groups[i].name.clone();
            if ui
                .selectable_label(sel, RichText::new(&name).strong())
                .clicked()
            {
                w.group = i;
            }
            let param = format!("{name}/volume");
            let mut v = value(m, w.snapshot, &param);
            ui.label(format!("{v:.1} dB"));
            let fader = ui.add(
                egui::Slider::new(&mut v, -80.0..=20.0)
                    .vertical()
                    .show_value(false),
            );
            if fader.changed() {
                set_value(m, w.snapshot, &param, v);
            }
            fader.context_menu(|ui| expose_menu(ui, m, &param));
            let g = &mut m.groups[i];
            ui.horizontal(|ui| {
                ui.toggle_value(&mut g.mute, "M").on_hover_text("Mute");
                ui.toggle_value(&mut g.solo, "S").on_hover_text("Solo");
                ui.toggle_value(&mut g.bypass_effects, "B")
                    .on_hover_text("Bypass effects");
            });
            ui.separator();
            ui.weak("Attenuation");
            for e in &g.effects {
                match e.kind {
                    AudioEffectType::Send if !e.target.is_empty() => {
                        ui.label(format!("Send → {}", e.target))
                    }
                    AudioEffectType::DuckVolume => ui.label("Duck Volume"),
                    k => ui.label(format!("{k:?}")),
                };
            }
            ui.menu_button("Add Effect", |ui| {
                for k in [
                    AudioEffectType::Lowpass,
                    AudioEffectType::Highpass,
                    AudioEffectType::Echo,
                    AudioEffectType::Reverb,
                    AudioEffectType::Distortion,
                    AudioEffectType::Compressor,
                    AudioEffectType::Send,
                    AudioEffectType::Receive,
                    AudioEffectType::DuckVolume,
                ] {
                    if ui.button(format!("{k:?}")).clicked() {
                        g.effects.push(AudioMixerEffect::new(k));
                        w.group = i;
                        ui.close();
                    }
                }
            });
        });
    });
}

fn expose_menu(ui: &mut egui::Ui, m: &mut AudioMixer, param: &str) {
    let exposed = m
        .exposed_parameters
        .iter()
        .position(|e| e.parameter == param);
    match exposed {
        Some(i) => {
            if ui
                .button(format!("Unexpose \"{}\"", m.exposed_parameters[i].name))
                .clicked()
            {
                m.exposed_parameters.remove(i);
                ui.close();
            }
        }
        None => {
            if ui.button("Expose to script").clicked() {
                let base = param.replace('/', "");
                let mut name = base.clone();
                let mut n = 1;
                while m.exposed_parameters.iter().any(|e| e.name == name) {
                    name = format!("{base}{n}");
                    n += 1;
                }
                m.exposed_parameters.push(ExposedParameter {
                    name,
                    parameter: param.to_owned(),
                });
                ui.close();
            }
        }
    }
}

/// One parameter row with a slider and an expose button.
fn param_row(ui: &mut egui::Ui, m: &mut AudioMixer, snapshot: usize, label: &str, param: &str) {
    let field = param.rsplit('/').next().unwrap_or(param);
    let (lo, hi, suffix) = param_range(field);
    let mut v = value(m, snapshot, param);
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, label);
        let exposed = m.exposed_parameters.iter().any(|e| e.parameter == param);
        let r = ui.add(
            egui::Slider::new(&mut v, lo..=hi)
                .suffix(suffix)
                .logarithmic(field == "cutoff"),
        );
        if r.changed() {
            set_value(m, snapshot, param, v);
        }
        r.context_menu(|ui| expose_menu(ui, m, param));
        if exposed {
            ui.colored_label(Color32::from_rgb(120, 190, 255), "exposed")
                .on_hover_text("Exposed to scripts");
        }
    });
}

fn group_inspector(ui: &mut egui::Ui, w: &mut AudioMixerWindow, m: &mut AudioMixer) {
    let i = w.group;
    let Some(name) = m.groups.get(i).map(|g| g.name.clone()) else {
        return;
    };
    ui.label(RichText::new(&name).strong());
    ui.weak("Right-click a value to expose it to scripts.");
    param_row(ui, m, w.snapshot, "Volume", &format!("{name}/volume"));
    param_row(ui, m, w.snapshot, "Pitch", &format!("{name}/pitch"));
    let effects = m.groups[i].effects.clone();
    let mut remove = None;
    let mut swap = None;
    for (k, e) in effects.iter().enumerate() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{:?}", e.kind)).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Remove effect")
                    .clicked()
                {
                    remove = Some(k);
                }
                if k + 1 < effects.len()
                    && ui.small_button("v").on_hover_text("Move down").clicked()
                {
                    swap = Some(k);
                }
            });
        });
        if e.kind == AudioEffectType::Send {
            // Unity's Send target: a group with a Receive (aux bus) or a Duck Volume
            // (sidechain).
            let targets: Vec<String> = m
                .groups
                .iter()
                .filter(|g| {
                    g.name != name
                        && g.effects.iter().any(|x| {
                            matches!(
                                x.kind,
                                AudioEffectType::Receive | AudioEffectType::DuckVolume
                            )
                        })
                })
                .map(|g| g.name.clone())
                .collect();
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Receive");
                let cur = &mut m.groups[i].effects[k].target;
                egui::ComboBox::from_id_salt(("mixer_send", i, k))
                    .selected_text(if cur.is_empty() { "None" } else { cur.as_str() })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(cur, String::new(), "None");
                        for t in &targets {
                            ui.selectable_value(cur, t.clone(), t);
                        }
                    });
            });
        }
        for p in e.parameters() {
            param_row(
                ui,
                m,
                w.snapshot,
                &crate::inspector::nicify(p),
                &format!("{name}/{k}/{p}"),
            );
        }
    }
    if let Some(k) = remove {
        m.groups[i].effects.remove(k);
    }
    if let Some(k) = swap {
        m.groups[i].effects.swap(k, k + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_edits_go_to_overrides() {
        let mut m = AudioMixer::default();
        m.groups.push(AudioMixerGroup::new("Music", "Master"));
        m.groups[1]
            .effects
            .push(AudioMixerEffect::new(AudioEffectType::Lowpass));
        m.snapshots.push(oxide_audio::mixer::AudioMixerSnapshot {
            name: "Paused".into(),
            values: vec![],
        });
        set_value(&mut m, 0, "Music/volume", -6.0);
        assert_eq!(m.groups[1].volume, -6.0);
        set_value(&mut m, 1, "Music/volume", -30.0);
        assert_eq!(m.groups[1].volume, -6.0, "base untouched");
        assert_eq!(value(&m, 1, "Music/volume"), -30.0);
        assert_eq!(value(&m, 0, "Music/volume"), -6.0);
        set_value(&mut m, 0, "Music/0/cutoff", 1200.0);
        assert_eq!(m.groups[1].effects[0].cutoff, 1200.0);
        assert_eq!(tree(&m), vec![(0, 0), (1, 1)]);
    }
}
