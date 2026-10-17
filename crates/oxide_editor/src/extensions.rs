//! Hosts the Scripts library's editor extensions ([`oxide_engine::editor_ext`]): custom
//! windows as dockable tabs, menu items in the main menu (with validation and shortcuts),
//! property drawers in the Inspector, overlays in the Scene view and Scene view tools.

use crate::{Action, EditorState, PlayState, Tab, Tool};
use oxide_engine::Engine;
use oxide_engine::editor_ext::{EditorCommand, EditorContext, EditorExtensions};
use oxide_scene::World;
use std::collections::HashMap;
use std::sync::RwLock;

/// Titles of custom window tabs by id ([`Tab::Custom`] holds the id so `Tab` stays `Copy`).
static TITLES: RwLock<Option<HashMap<u32, &'static str>>> = RwLock::new(None);

fn hash(title: &str) -> u32 {
    title.bytes().fold(2_166_136_261u32, |h, b| {
        (h ^ b as u32).wrapping_mul(16_777_619)
    })
}

/// The tab of a custom window.
pub fn tab(title: &str) -> Tab {
    let id = hash(title);
    let mut t = TITLES.write().expect("titles");
    let map = t.get_or_insert_with(HashMap::new);
    map.entry(id)
        .or_insert_with(|| Box::leak(title.to_owned().into_boxed_str()));
    Tab::Custom(id)
}

/// A custom window's title (`"Missing Window"` until its script registers it).
pub fn title(id: u32) -> &'static str {
    TITLES
        .read()
        .ok()
        .and_then(|t| t.as_ref().and_then(|m| m.get(&id).copied()))
        .unwrap_or("Missing Window")
}

/// Runs `f` with an [`EditorContext`], then applies the selection it set and queues its
/// commands. Returns `None` when there are no extensions.
pub fn with_context<R>(
    st: &mut EditorState,
    world: &mut World,
    f: impl FnOnce(&mut EditorContext, &mut EditorExtensions) -> R,
) -> Option<R> {
    let mut ext = world.resources.remove::<EditorExtensions>()?;
    let now = oxide_core::time::realtime_since_startup();
    let playing = st.play != PlayState::Edit;
    let selection = st.selection.clone();
    // The same label again in quick succession (a handle drag) is one undo step.
    let mut undo = |w: &World, label: &str| {
        let key = 0xE7_0000_0000u64 | u64::from(hash(label));
        st.record(w, label, Some(key), now)
    };
    let mut ctx = EditorContext::new(world, selection, playing, &mut undo);
    let r = f(&mut ctx, &mut ext);
    let (sel, cmds) = ctx.finish();
    world.resources.insert(ext);
    if let Some(s) = sel {
        st.selection = s
            .into_iter()
            .filter(|&id| world.get(id).is_some())
            .collect();
    }
    st.ext_commands.extend(cmds);
    Some(r)
}

/// Draws a custom window tab.
pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, id: u32) {
    let name = title(id);
    let selection = st.selection.clone();
    let drawn = with_context(st, &mut engine.world, |ctx, ext| {
        let Some((mut w, created)) = ext.take_window(name) else {
            return false;
        };
        if created {
            w.on_enable(ctx);
        }
        let changed = ui.data_mut(|d| {
            let key = egui::Id::new(("ext-window-selection", id));
            let prev: Option<Vec<oxide_scene::GameObjectId>> = d.get_temp(key);
            d.insert_temp(key, selection.clone());
            prev.is_some_and(|p| p != selection)
        });
        if changed {
            w.on_selection_change(ctx);
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| w.on_gui(ctx, ui));
        ext.put_window(name, w);
        true
    });
    if drawn != Some(true) {
        ui.weak(format!(
            "“{name}” comes from the Scripts library, which hasn't registered it (not compiled \
             yet, or the window was removed)."
        ));
    }
}

/// Forgets a closed custom window's state.
pub fn closed(engine: &mut Engine, id: u32) {
    if let Some(x) = engine.world.try_resource_mut::<EditorExtensions>() {
        x.close_window(title(id));
    }
}

/// Main-menu items from extensions: custom windows' menu paths and menu items (in
/// priority order). `enabled` comes from validate functions when `world` is given.
pub fn menu_items(world: &World) -> Vec<(String, Action, Option<String>)> {
    let Some(x) = world.try_resource::<EditorExtensions>() else {
        return Vec::new();
    };
    let mut out: Vec<(String, Action, Option<String>)> = x
        .windows()
        .iter()
        .map(|w| (w.menu.clone(), Action::OpenTab(tab(&w.title)), None))
        .collect();
    let mut items: Vec<_> = x.menu_items().to_vec();
    items.sort_by_key(|m| m.priority);
    out.extend(
        items
            .into_iter()
            .map(|m| (m.path.clone(), Action::ExtensionMenu(m.path), m.shortcut)),
    );
    out
}

/// Whether an extension menu item's validate function allows it now.
pub fn validate(st: &mut EditorState, world: &mut World, path: &str) -> bool {
    let Some(f) = world
        .try_resource::<EditorExtensions>()
        .and_then(|x| x.menu_item_def(path))
        .and_then(|m| m.validate)
    else {
        return true;
    };
    with_context(st, world, |ctx, _| f(ctx)).unwrap_or(true)
}

/// Runs an extension menu item.
pub fn run_menu(st: &mut EditorState, engine: &mut Engine, path: &str) -> Result<String, String> {
    let Some(def) = engine
        .world
        .try_resource::<EditorExtensions>()
        .and_then(|x| x.menu_item_def(path))
        .cloned()
    else {
        return Err(format!("no menu item `{path}`"));
    };
    if !validate(st, &mut engine.world, path) {
        return Err(format!("`{path}` isn't available now"));
    }
    with_context(st, &mut engine.world, |ctx, _| (def.run)(ctx));
    Ok(String::new())
}

/// Extension menu shortcuts pressed this frame.
pub fn pressed_shortcuts(ctx: &egui::Context, world: &World) -> Vec<String> {
    let Some(x) = world.try_resource::<EditorExtensions>() else {
        return Vec::new();
    };
    if ctx.egui_wants_keyboard_input() {
        return Vec::new();
    }
    x.menu_items()
        .iter()
        .filter_map(|m| {
            let s = oxide_engine::editor_ext::parse_shortcut(m.shortcut.as_deref()?)?;
            ctx.input_mut(|i| i.consume_shortcut(&s))
                .then(|| m.path.clone())
        })
        .collect()
}

/// The custom overlays to show (contextual ones only when they apply).
pub fn overlay_ids(st: &mut EditorState, world: &mut World) -> Vec<String> {
    with_context(st, world, |ctx, ext| {
        ext.overlay_ids()
            .into_iter()
            .filter(|id| match ext.take_overlay(id) {
                Some(o) => {
                    let a = o.applies(ctx);
                    ext.put_overlay(id, o);
                    a
                }
                None => false,
            })
            .collect()
    })
    .unwrap_or_default()
}

pub fn all_overlay_ids(world: &World) -> Vec<String> {
    world
        .try_resource::<EditorExtensions>()
        .map(|x| x.overlay_ids())
        .unwrap_or_default()
}

pub fn overlay_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, id: &str) {
    with_context(st, &mut engine.world, |ctx, ext| {
        if let Some(mut o) = ext.take_overlay(id) {
            o.on_gui(ctx, ui);
            ext.put_overlay(id, o);
        }
    });
}

/// (name, icon, available) of each custom tool.
pub fn tools(st: &mut EditorState, world: &mut World) -> Vec<(String, String, bool)> {
    with_context(st, world, |ctx, ext| {
        ext.tool_names()
            .into_iter()
            .map(|(n, icon)| {
                let ok = match ext.take_tool(&n) {
                    Some(t) => {
                        let ok = t.is_available(ctx);
                        ext.put_tool(&n, t);
                        ok
                    }
                    None => false,
                };
                (n, icon, ok)
            })
            .collect()
    })
    .unwrap_or_default()
}

/// Switches to a custom tool (or back to a built-in one with `None`), calling the tools'
/// activation callbacks.
pub fn set_custom_tool(st: &mut EditorState, world: &mut World, name: Option<String>) {
    if st.custom_tool == name {
        return;
    }
    let old = std::mem::replace(&mut st.custom_tool, name.clone());
    with_context(st, world, |ctx, ext| {
        if let Some(o) = old
            && let Some(mut t) = ext.take_tool(&o)
        {
            t.on_deactivated(ctx);
            ext.put_tool(&o, t);
        }
        if let Some(n) = name
            && let Some(mut t) = ext.take_tool(&n)
        {
            t.on_activated(ctx);
            ext.put_tool(&n, t);
        }
    });
}

/// The active custom tool's Tool Settings contents. Returns false with no custom tool.
pub fn tool_settings_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) -> bool {
    let Some(name) = st.custom_tool.clone() else {
        return false;
    };
    with_context(st, &mut engine.world, |ctx, ext| {
        if let Some(mut t) = ext.take_tool(&name) {
            ui.label(egui::RichText::new(&name).strong());
            t.tool_settings(ctx, ui);
            ext.put_tool(&name, t);
        }
    });
    true
}

/// Runs the active custom tool in the Scene view. Returns whether it used the pointer.
pub fn tool_scene_gui(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &mut egui::Ui,
    projector: &crate::view::Projector,
) -> bool {
    let Some(name) = st.custom_tool.clone() else {
        return false;
    };
    with_context(st, &mut engine.world, |ctx, ext| {
        let Some(mut t) = ext.take_tool(&name) else {
            return false;
        };
        let mut gui = oxide_engine::SceneGui::new(ctx, projector, ui);
        t.on_tool_gui(&mut gui);
        let used = gui.used;
        ext.put_tool(&name, t);
        used
    })
    .unwrap_or(false)
}

impl oxide_engine::editor_ext::SceneProjection for crate::view::Projector {
    fn project(&self, p: oxide_math::Vector3) -> Option<egui::Pos2> {
        crate::view::Projector::project(self, p)
    }
    fn ray(&self, p: egui::Pos2) -> oxide_math::Ray {
        crate::view::Projector::ray(self, p)
    }
    fn world_per_pixels(&self, p: oxide_math::Vector3, pixels: f32) -> f32 {
        crate::view::Projector::world_per_pixels(self, p, pixels)
    }
    fn camera_position(&self) -> oxide_math::Vector3 {
        self.cam_pos
    }
    fn camera_forward(&self) -> oxide_math::Vector3 {
        self.cam_forward
    }
    fn rect(&self) -> egui::Rect {
        self.rect
    }
}

/// Built-in tool names (for [`EditorCommand::SetTool`]).
fn builtin_tool(name: &str) -> Option<Tool> {
    [
        Tool::Hand,
        Tool::Move,
        Tool::Rotate,
        Tool::Scale,
        Tool::Rect,
        Tool::Transform,
    ]
    .into_iter()
    .find(|t| t.name().eq_ignore_ascii_case(name))
}

/// Turns queued extension commands into editor actions.
pub fn take_actions(st: &mut EditorState, engine: &mut Engine) -> Vec<Action> {
    let mut out = Vec::new();
    for c in std::mem::take(&mut st.ext_commands) {
        match c {
            EditorCommand::OpenWindow(t) => {
                let all = Tab::ALL.into_iter().chain(crate::mcp::extra_windows());
                if let Some(tab) = all.into_iter().find(|x| x.title().eq_ignore_ascii_case(&t)) {
                    out.push(Action::OpenTab(tab));
                } else if engine
                    .world
                    .try_resource::<EditorExtensions>()
                    .is_some_and(|x| x.has_window(&t))
                {
                    out.push(Action::OpenTab(tab(&t)));
                } else {
                    st.status = format!("no window `{t}`");
                }
            }
            EditorCommand::ExecuteMenu(p) => {
                let items = crate::menus::menu_items(&engine.world, &st.keymap);
                match items.into_iter().find(|m| m.path.eq_ignore_ascii_case(&p)) {
                    Some(m) if m.action.is_some() => out.extend(m.action),
                    _ => st.status = format!("no menu item `{p}`"),
                }
            }
            EditorCommand::Status(s) => {
                // The status bar shows the Console's last line.
                oxide_core::debug_log!("{s}");
                st.status = s;
            }
            EditorCommand::SetTool(n) => {
                if let Some(t) = builtin_tool(&n) {
                    set_custom_tool(st, &mut engine.world, None);
                    st.tool = t;
                } else {
                    set_custom_tool(st, &mut engine.world, Some(n));
                }
            }
        }
    }
    out
}
