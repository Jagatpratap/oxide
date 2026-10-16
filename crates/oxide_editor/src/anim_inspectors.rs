//! Inspectors for animation assets: Animation Clips (loop, root node, events, curves),
//! Animator Controllers (summary and Open Animator), Animator Override Controllers (clip
//! replacement table) and Avatar Masks (transform checklist, imported from a model).
//! References: <https://docs.unity3d.com/Manual/class-AnimationClip.html>,
//! <https://docs.unity3d.com/Manual/AnimatorOverrideController.html>,
//! <https://docs.unity3d.com/Manual/class-AvatarMask.html>

use crate::{Action, EditorState};
use egui::{Color32, RichText};
use oxide_assets::{
    AnimationClip, AnimatorController, AnimatorOverrideController, AssetServer, AvatarMask,
    ClipOverride, Handle,
};
use oxide_core::reflect::Value;
use oxide_engine::Engine;

fn error(ui: &mut egui::Ui, e: impl Into<String>) {
    ui.colored_label(Color32::from_rgb(230, 120, 100), e.into());
}

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, label);
        add(ui);
    });
}

/// A picker over assets of one type: project files and loaded assets.
fn asset_picker(
    ui: &mut egui::Ui,
    assets: &AssetServer,
    salt: &str,
    type_name: &str,
    current: Option<&str>,
    allow_none: bool,
) -> Option<Option<String>> {
    let mut paths = assets.paths_of_type(type_name);
    if type_name == "AnimationClip" {
        paths.extend(assets.animations.iter().map(|(p, _)| p.to_owned()));
        paths.sort();
        paths.dedup();
    }
    let label = |p: &str| {
        assets
            .animations
            .iter()
            .find(|(q, _)| *q == p)
            .map(|(_, c)| c.name.clone())
            .filter(|n| !n.is_empty() && type_name == "AnimationClip")
            .unwrap_or_else(|| p.rsplit('/').next().unwrap_or(p).to_owned())
    };
    let mut picked = None;
    egui::ComboBox::from_id_salt(("anim-pick", salt))
        .selected_text(current.map_or_else(|| format!("None ({type_name})"), label))
        .width(crate::inspector::avail(ui).max(60.0))
        .show_ui(ui, |ui| {
            if allow_none && ui.selectable_label(current.is_none(), "None").clicked() {
                picked = Some(None);
            }
            for p in &paths {
                if ui
                    .selectable_label(current == Some(p.as_str()), label(p))
                    .on_hover_text(p)
                    .clicked()
                {
                    picked = Some(Some(p.clone()));
                }
            }
        });
    picked
}

// ---------------------------------------------------------------------------------------

pub fn clip(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let assets = engine.world.resource_mut::<AssetServer>();
    if let Err(e) = assets.ensure_loaded(path) {
        return error(ui, e);
    }
    let h: Handle<AnimationClip> = Handle::from_path(path);
    let Some(mut c) = assets.animations.get(&h).cloned() else {
        return;
    };
    let before = c.clone();
    let editable = path.ends_with(".anim");
    if ui.button("Open Animation Window").clicked() {
        st.animation.open_clip(path);
        st.actions.push(Action::OpenTab(crate::Tab::Animation));
    }
    ui.add_enabled_ui(editable, |ui| {
        row(ui, "Length", |ui| {
            ui.label(format!("{:.2} s", c.length));
        });
        row(ui, "Loop Time", |ui| {
            ui.checkbox(&mut c.looping, "");
        });
        row(ui, "Root Node", |ui| {
            let mut paths: Vec<String> = c
                .curves
                .iter()
                .filter(|x| x.position.is_some())
                .map(|x| x.path.clone())
                .collect();
            paths.sort();
            egui::ComboBox::from_id_salt("clip-root")
                .selected_text(c.root_node.clone().unwrap_or_else(|| "Automatic".into()))
                .width(crate::inspector::avail(ui))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut c.root_node, None, "Automatic");
                    for p in paths {
                        ui.selectable_value(&mut c.root_node, Some(p.clone()), p);
                    }
                });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Events").strong());
            if ui.small_button("+").on_hover_text("Add Event").clicked() {
                c.add_event(0.0, "OnEvent", Value::Null);
            }
        });
        let mut remove = None;
        for (i, e) in c.events.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut e.time)
                        .speed(0.01)
                        .range(0.0..=f32::MAX)
                        .suffix(" s"),
                );
                ui.add(egui::TextEdit::singleline(&mut e.function_name).desired_width(120.0));
                let mut text = match &e.parameter {
                    Value::Null => String::new(),
                    Value::String(s) => s.clone(),
                    Value::Float(f) => f.to_string(),
                    Value::Int(i) => i.to_string(),
                    other => format!("{other:?}"),
                };
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut text)
                            .hint_text("parameter")
                            .desired_width(80.0),
                    )
                    .changed()
                {
                    e.parameter = if text.is_empty() {
                        Value::Null
                    } else if let Ok(i) = text.parse::<i64>() {
                        Value::Int(i)
                    } else if let Ok(f) = text.parse::<f64>() {
                        Value::Float(f)
                    } else {
                        Value::String(text)
                    };
                }
                if ui.small_button("×").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            c.events.remove(i);
        }
    });
    if !editable {
        ui.weak("Clips inside a model are read-only; use Extract Animations on the model to edit a copy.");
    }
    ui.add_space(4.0);
    egui::CollapsingHeader::new(format!(
        "Curves ({} transforms, {} properties)",
        c.curves.len(),
        c.properties.len()
    ))
    .show(ui, |ui| {
        for t in &c.curves {
            let mut parts = Vec::new();
            if t.position.is_some() {
                parts.push("Position");
            }
            if t.rotation.is_some() {
                parts.push("Rotation");
            }
            if t.scale.is_some() {
                parts.push("Scale");
            }
            ui.weak(format!(
                "{}: {}",
                if t.path.is_empty() { "(root)" } else { &t.path },
                parts.join(", ")
            ));
        }
        for p in &c.properties {
            ui.weak(format!(
                "{}: {}.{} ({} keys)",
                if p.path.is_empty() { "(root)" } else { &p.path },
                p.component,
                p.property,
                p.curve.len()
            ));
        }
    });
    if editable && c != before {
        c.events.sort_by(|a, b| a.time.total_cmp(&b.time));
        if let Err(e) = engine
            .world
            .resource_mut::<AssetServer>()
            .save_animation_clip(path, &c)
        {
            st.status = e;
        }
    }
}

pub fn controller(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let assets = engine.world.resource_mut::<AssetServer>();
    if let Err(e) = assets.ensure_loaded(path) {
        return error(ui, e);
    }
    let h: Handle<AnimatorController> = Handle::from_path(path);
    let Some(c) = assets.controllers.get(&h).cloned() else {
        return;
    };
    if ui.button("Open Animator").clicked() {
        st.animator.open(path);
        st.actions.push(Action::OpenTab(crate::Tab::Animator));
    }
    ui.add_space(4.0);
    ui.label(RichText::new("Parameters").strong());
    if c.parameters.is_empty() {
        ui.weak("None");
    }
    for p in &c.parameters {
        ui.weak(format!("{} ({:?})", p.name, p.kind));
    }
    ui.label(RichText::new("Layers").strong());
    for l in &c.layers {
        let states: Vec<&str> = l
            .state_machine
            .states
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        ui.weak(format!(
            "{}: {}",
            l.name,
            if states.is_empty() {
                "(empty)".into()
            } else {
                states.join(", ")
            }
        ));
    }
}

pub fn override_controller(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &mut egui::Ui,
    path: &str,
) {
    let assets = engine.world.resource_mut::<AssetServer>();
    if let Err(e) = assets.ensure_loaded(path) {
        return error(ui, e);
    }
    let h: Handle<AnimatorOverrideController> = Handle::from_path(path);
    let Some(mut oc) = assets.override_controllers.get(&h).cloned() else {
        return;
    };
    let before = oc.clone();
    let assets = engine.world.resource::<AssetServer>();
    row(ui, "Controller", |ui| {
        if let Some(p) = asset_picker(
            ui,
            assets,
            "oc-base",
            "AnimatorController",
            oc.controller.as_ref().map(|c| c.path()),
            true,
        ) {
            oc.controller = p.map(Handle::from_path);
        }
    });
    let base = oc
        .controller
        .as_ref()
        .and_then(|b| assets.controllers.get(b));
    match base {
        None => {
            ui.weak("Pick the Animator Controller whose clips to override.");
        }
        Some(base) => {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Original").strong());
                ui.add_space(110.0);
                ui.label(RichText::new("Override").strong());
            });
            for original in base.clips() {
                let current = oc
                    .overrides
                    .iter()
                    .find(|o| o.original == original)
                    .and_then(|o| o.replacement.clone());
                ui.horizontal(|ui| {
                    let name = assets
                        .animations
                        .get(&original)
                        .map_or_else(|| original.path().to_owned(), |c| c.name.clone());
                    crate::inspector::prop_label(ui, name).on_hover_text(original.path());
                    let salt = format!("oc-{}", original.path());
                    if let Some(p) = asset_picker(
                        ui,
                        assets,
                        &salt,
                        "AnimationClip",
                        current.as_ref().map(|c| c.path()),
                        true,
                    ) {
                        oc.overrides.retain(|o| o.original != original);
                        if let Some(p) = p {
                            oc.overrides.push(ClipOverride {
                                original: original.clone(),
                                replacement: Some(Handle::from_path(p)),
                            });
                        }
                    }
                });
            }
        }
    }
    if oc != before
        && let Err(e) = engine
            .world
            .resource_mut::<AssetServer>()
            .save_override_controller(path, &oc)
    {
        st.status = e;
    }
}

/// Transform paths of a model's hierarchy (`Armature/Hips/Spine`), depth-first.
fn model_paths(m: &oxide_assets::Model) -> Vec<String> {
    fn walk(m: &oxide_assets::Model, i: usize, prefix: &str, out: &mut Vec<String>) {
        let n = &m.nodes[i];
        let p = if prefix.is_empty() {
            n.name.clone()
        } else {
            format!("{prefix}/{}", n.name)
        };
        out.push(p.clone());
        for &c in &n.children {
            walk(m, c, &p, out);
        }
    }
    let mut out = Vec::new();
    for &r in &m.roots {
        walk(m, r, "", &mut out);
    }
    out
}

#[derive(Default)]
pub struct MaskState {
    /// Transform paths imported from a model, for the checklist.
    skeleton: Vec<String>,
    source: Option<String>,
}

pub fn avatar_mask(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let assets = engine.world.resource_mut::<AssetServer>();
    if let Err(e) = assets.ensure_loaded(path) {
        return error(ui, e);
    }
    let h: Handle<AvatarMask> = Handle::from_path(path);
    let Some(mut m) = assets.masks.get(&h).cloned() else {
        return;
    };
    let before = m.clone();
    ui.weak("A layer using this mask animates only the checked transforms (and everything below them). Empty = everything.");
    ui.add_space(4.0);
    // Import the transform tree from a model, like Unity's "Import skeleton".
    row(ui, "Skeleton from", |ui| {
        let assets = engine.world.resource::<AssetServer>();
        let current = st.mask.source.clone();
        if let Some(Some(p)) =
            asset_picker(ui, assets, "mask-src", "Model", current.as_deref(), false)
        {
            st.mask.source = Some(p);
            st.mask.skeleton.clear();
        }
    });
    if let Some(src) = st.mask.source.clone()
        && st.mask.skeleton.is_empty()
    {
        let assets = engine.world.resource_mut::<AssetServer>();
        match assets.ensure_loaded(&src) {
            Ok(()) => {
                if let Some(model) = assets
                    .models
                    .handle_of(&src)
                    .and_then(|h| assets.models.get(&h))
                {
                    st.mask.skeleton = model_paths(model);
                }
            }
            Err(e) => st.status = e,
        }
    }
    let shown: Vec<String> = if st.mask.skeleton.is_empty() {
        m.transform_paths.clone()
    } else {
        st.mask.skeleton.clone()
    };
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            for p in &shown {
                let depth = p.matches('/').count();
                let name = p.rsplit('/').next().unwrap_or(p);
                let mut on = m.transform_paths.contains(p);
                let covered = !on
                    && m.transform_paths
                        .iter()
                        .any(|q| p.starts_with(&format!("{q}/")));
                ui.horizontal(|ui| {
                    ui.add_space(depth as f32 * 12.0);
                    let r = ui.checkbox(
                        &mut on,
                        if covered {
                            RichText::new(name).weak()
                        } else {
                            RichText::new(name)
                        },
                    );
                    if r.changed() {
                        if on {
                            m.transform_paths.push(p.clone());
                        } else {
                            m.transform_paths.retain(|q| q != p);
                        }
                    }
                });
            }
        });
    if shown.is_empty() {
        ui.weak("Pick a model above to list its transforms.");
    }
    if m != before {
        m.transform_paths.sort();
        if let Err(e) = engine
            .world
            .resource_mut::<AssetServer>()
            .save_avatar_mask(path, &m)
        {
            st.status = e;
        }
    }
}

/// Unity's Extract Animations: copies a model's clips to editable `.anim` files beside it.
pub fn extract_model_clips(engine: &mut Engine, model_path: &str) -> Result<Vec<String>, String> {
    let assets = engine.world.resource_mut::<AssetServer>();
    assets.ensure_loaded(model_path)?;
    let model = assets
        .models
        .handle_of(model_path)
        .and_then(|h| assets.models.get(&h))
        .cloned()
        .ok_or("not a model")?;
    let dir = model_path
        .rsplit_once('/')
        .map_or("Assets", |(d, _)| d)
        .to_owned();
    let root = assets.full_path("");
    let mut out = Vec::new();
    for a in &model.animations {
        let Some(clip) = assets.animations.get(a).cloned() else {
            continue;
        };
        let name = if clip.name.is_empty() {
            "Clip".to_owned()
        } else {
            clip.name.clone()
        };
        let path = crate::asset_ops::unique_path(&root, &format!("{dir}/{name}.anim"));
        assets.save_animation_clip(&path, &clip)?;
        out.push(path);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_paths_are_depth_first() {
        use oxide_math::{Quaternion, Vector3};
        let node = |name: &str, children: Vec<usize>| oxide_assets::ModelNode {
            name: name.into(),
            position: Vector3::ZERO,
            rotation: Quaternion::IDENTITY,
            scale: Vector3::ONE,
            children,
            mesh: None,
            materials: vec![],
            skin: None,
        };
        let m = oxide_assets::Model {
            nodes: vec![
                node("Hips", vec![1, 2]),
                node("Spine", vec![]),
                node("Leg", vec![]),
            ],
            roots: vec![0],
            ..Default::default()
        };
        assert_eq!(model_paths(&m), vec!["Hips", "Hips/Spine", "Hips/Leg"]);
    }
}
