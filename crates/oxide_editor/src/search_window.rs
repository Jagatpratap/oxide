//! Search (Unity's Search window, ⌘K / Ctrl+K): one box that finds assets, scene objects,
//! menu items and settings pages.
//!
//! Prefixes pick one provider, as in Unity: `p:` assets (with `t:Type` and `l:Label`),
//! `h:` the open scenes (`t:Camera` finds objects with that component), `m:` menu items and
//! `set:` Project Settings and Preferences pages. Without a prefix every provider is searched.
//! Reference: <https://docs.unity3d.com/Manual/search-overview.html>

use crate::{Action, EditorState, menus, project};
use egui::{Color32, Key, Modifiers, RichText};
use oxide_assets::AssetServer;
use oxide_engine::Engine;
use oxide_scene::{GameObjectId, World};

/// Results shown per provider.
const LIMIT: usize = 40;

#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    selected: usize,
    /// Every asset, and when it was read (seconds).
    index: Option<(f64, Vec<project::Entry>)>,
}

#[derive(Clone, Debug)]
pub enum Hit {
    Asset(project::Entry),
    Object {
        id: GameObjectId,
        path: String,
    },
    Menu {
        path: String,
        shortcut: String,
        action: Action,
    },
    Setting {
        name: String,
        action: Action,
    },
}

impl Hit {
    pub fn provider(&self) -> &'static str {
        match self {
            Hit::Asset(_) => "Project",
            Hit::Object { .. } => "Hierarchy",
            Hit::Menu { .. } => "Menu",
            Hit::Setting { .. } => "Settings",
        }
    }
}

/// How well `name` matches the words: 0 is best (starts with the first word); `None` when a
/// word is missing.
fn score(name: &str, words: &[String]) -> Option<usize> {
    let lower = name.to_lowercase();
    let mut best = usize::MAX;
    for w in words {
        let at = lower.find(w.as_str())?;
        best = best.min(at);
    }
    Some(if words.is_empty() { 0 } else { best })
}

/// Everything matching `text`, grouped by provider (Project, Hierarchy, Menu, Settings).
pub fn search(st: &mut EditorState, engine: &Engine, text: &str) -> Vec<Hit> {
    let text = text.trim();
    let (only, rest) = match text.split_once(':') {
        Some((p @ ("p" | "h" | "m" | "set"), r)) => (Some(p), r.trim()),
        _ => (None, text),
    };
    let want = |p: &str| only.is_none_or(|o| o == p);
    let q = project::Query::parse(rest);
    if q.is_empty() && only.is_none() {
        return Vec::new();
    }
    let mut out = Vec::new();

    if want("p") {
        let now = oxide_core::time::realtime_since_startup();
        let root = engine.world.resource::<AssetServer>().full_path("");
        let s = &mut st.search;
        if s.index.as_ref().is_none_or(|(t, _)| now - t > 2.0) {
            s.index = Some((now, project::list_all(&root, "Assets")));
        }
        let mut hits: Vec<(usize, &project::Entry)> = s
            .index
            .iter()
            .flat_map(|(_, v)| v.iter())
            .filter(|e| q.matches(e))
            .filter_map(|e| score(&e.name, &q.words).map(|sc| (sc, e)))
            .collect();
        hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.name.len().cmp(&b.1.name.len())));
        out.extend(
            hits.into_iter()
                .take(LIMIT)
                .map(|(_, e)| Hit::Asset(e.clone())),
        );
    }

    if want("h") {
        let w = &engine.world;
        let mut hits: Vec<(usize, GameObjectId)> = w
            .hierarchy_order()
            .into_iter()
            .filter(|&id| object_has_types(w, id, &q.types))
            .filter_map(|id| {
                let name = &w.get(id)?.name;
                score(name, &q.words).map(|sc| (sc, id))
            })
            .collect();
        hits.sort_by_key(|h| h.0);
        out.extend(hits.into_iter().take(LIMIT).map(|(_, id)| Hit::Object {
            id,
            path: w.path_of(id),
        }));
    }

    // Menus and settings have no types or labels.
    let plain = q.types.is_empty() && q.labels.is_empty();
    if want("m") && plain {
        let items = menus::menu_items(&engine.world, &st.keymap);
        out.extend(
            items
                .into_iter()
                .filter(|m| !m.path.ends_with("/-"))
                .filter_map(|m| {
                    let action = m.action?;
                    score(&m.path, &q.words)?;
                    Some(Hit::Menu {
                        path: m.path,
                        shortcut: m.shortcut,
                        action,
                    })
                })
                .take(LIMIT),
        );
    }
    if want("set") && plain {
        for (page, name) in crate::settings_window::PAGES {
            let full = format!("Project Settings/{name}");
            if score(&full, &q.words).is_some() {
                out.push(Hit::Setting {
                    name: full,
                    action: Action::OpenProjectSettings(page),
                });
            }
        }
        for (page, name) in crate::prefs::PAGES {
            let full = format!("Preferences/{name}");
            if score(&full, &q.words).is_some() {
                out.push(Hit::Setting {
                    name: full,
                    action: Action::OpenPreferencesPage(page),
                });
            }
        }
    }
    out
}

/// The object has a component of each `t:` type (case-insensitive).
fn object_has_types(w: &World, id: GameObjectId, types: &[String]) -> bool {
    if types.is_empty() {
        return true;
    }
    let Some(o) = w.get(id) else {
        return false;
    };
    let names: Vec<String> = o
        .component_list()
        .into_iter()
        .map(|(_, n, _)| n.to_lowercase())
        .chain(["transform".to_owned(), "gameobject".to_owned()])
        .collect();
    types.iter().all(|t| names.contains(t))
}

/// Does what selecting the result means: assets and objects are selected (and pinged),
/// menu items run, settings pages open. `open` (Alt+Enter, double-click) opens assets.
pub fn activate(st: &mut EditorState, engine: &mut Engine, hit: &Hit, open: bool) {
    match hit {
        Hit::Asset(e) => {
            if open {
                let root = engine.world.resource::<AssetServer>().full_path("");
                if let Err(err) = project::open(st, engine, &root, &e.path) {
                    st.status = err;
                }
            } else {
                st.project.show(&e.path);
                st.selection.clear();
                st.actions.push(Action::OpenTab(crate::Tab::Project));
            }
        }
        Hit::Object { id, .. } => {
            st.select(*id);
            st.hierarchy.reveal(&engine.world, *id);
            if open {
                st.scene_ui.frame_request = true;
            }
        }
        Hit::Menu { action, .. } | Hit::Setting { action, .. } => st.actions.push(action.clone()),
    }
}

/// The popup: query field, grouped results, ↑/↓ to move, Enter to select, Alt+Enter to
/// open, Esc to close.
pub fn ui(st: &mut EditorState, engine: &mut Engine, ctx: &egui::Context) {
    if !st.search.open {
        return;
    }
    let query = st.search.query.clone();
    let hits = search(st, engine, &query);
    let root = Some(engine.world.resource::<AssetServer>().full_path(""));
    let previews = &mut st.previews;
    let s = &mut st.search;
    s.selected = s.selected.min(hits.len().saturating_sub(1));
    let (down, up, enter, alt_enter, esc) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::ALT, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    if down && !hits.is_empty() {
        s.selected = (s.selected + 1).min(hits.len() - 1);
    }
    if up {
        s.selected = s.selected.saturating_sub(1);
    }
    let mut chosen: Option<(usize, bool)> = None;
    if (enter || alt_enter) && !hits.is_empty() {
        chosen = Some((s.selected, alt_enter));
    }
    let screen = ctx.content_rect();
    let width = (screen.width() - 40.0).clamp(320.0, 680.0);
    let mut keep_open = !esc;
    let area = egui::Window::new("Search")
        .id(egui::Id::new("oxide-search"))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .fixed_pos(egui::pos2(
            screen.center().x - width * 0.5,
            screen.min.y + 80.0,
        ))
        .fixed_size(egui::vec2(width, 420.0))
        .show(ctx, |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut s.query)
                    .hint_text(
                        "Search assets, objects, menus, settings   (p: h: m: set:  t:Type l:Label)",
                    )
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Heading),
            );
            if !r.has_focus() {
                r.request_focus();
            }
            if r.changed() {
                s.selected = 0;
            }
            ui.separator();
            if hits.is_empty() {
                ui.weak(if query.trim().is_empty() {
                    "Type to search. Prefixes: p: assets, h: hierarchy, m: menus, set: settings."
                } else {
                    "No results."
                });
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut last = "";
                    for (i, hit) in hits.iter().enumerate() {
                        if hit.provider() != last {
                            last = hit.provider();
                            let n = hits.iter().filter(|h| h.provider() == last).count();
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!("{last}  ({n})"))
                                    .strong()
                                    .color(Color32::from_gray(160)),
                            );
                        }
                        let selected = i == s.selected;
                        let (text, detail) = match hit {
                            Hit::Asset(e) => (e.name.clone(), project::entry_kind(e).0.to_owned()),
                            Hit::Object { path, .. } => (path.clone(), "GameObject".into()),
                            Hit::Menu { path, shortcut, .. } => (path.clone(), shortcut.clone()),
                            Hit::Setting { name, .. } => (name.clone(), String::new()),
                        };
                        let (rect, r) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 20.0),
                            egui::Sense::click(),
                        );
                        let painter = ui.painter_at(rect);
                        if selected {
                            painter.rect_filled(rect, 2.0, crate::theme::SELECTION_BG);
                        } else if r.hovered() {
                            painter.rect_filled(rect, 2.0, Color32::from_gray(69));
                        }
                        let icon = egui::Rect::from_min_size(
                            rect.min + egui::vec2(4.0, 2.0),
                            egui::vec2(16.0, 16.0),
                        );
                        if let (Hit::Asset(e), Some(root)) = (hit, root.as_deref())
                            && let Some(t) = previews.get(root, &e.path)
                        {
                            let uv = egui::Rect::from_min_max(
                                egui::pos2(0.0, 0.0),
                                egui::pos2(1.0, 1.0),
                            );
                            painter.image(t, icon, uv, Color32::WHITE);
                        }
                        painter.text(
                            egui::pos2(icon.max.x + 6.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            text,
                            egui::FontId::proportional(13.0),
                            Color32::from_gray(220),
                        );
                        painter.text(
                            rect.right_center() - egui::vec2(8.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            detail,
                            egui::FontId::proportional(11.5),
                            Color32::from_gray(140),
                        );
                        if selected && (up || down) {
                            r.scroll_to_me(None);
                        }
                        if let Hit::Asset(e) = hit {
                            let r = r.clone().on_hover_text(&e.path);
                            if r.double_clicked() {
                                chosen = Some((i, true));
                                continue;
                            }
                        }
                        if r.clicked() {
                            chosen = Some((i, false));
                        }
                    }
                });
        });
    // Clicking outside closes it, like Unity's popup.
    if let Some(a) = area
        && ctx.input(|i| i.pointer.any_pressed())
        && !ctx
            .pointer_hover_pos()
            .is_some_and(|p| a.response.rect.contains(p))
    {
        keep_open = false;
    }
    if let Some((i, open)) = chosen
        && let Some(hit) = hits.get(i)
    {
        activate(st, engine, hit, open);
        keep_open = false;
    }
    st.search.open = keep_open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring_prefers_early_matches_and_needs_every_word() {
        let w = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(score("Car Paint.mat", &w(&["car"])), Some(0));
        assert_eq!(score("Red Car.prefab", &w(&["car"])), Some(4));
        assert_eq!(score("Red Car.prefab", &w(&["car", "blue"])), None);
        assert_eq!(score("anything", &[]), Some(0));
    }
}
