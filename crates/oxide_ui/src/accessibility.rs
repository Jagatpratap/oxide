//! Unity's Accessibility module: an [`AccessibilityHierarchy`] of [`AccessibilityNode`]s
//! (label, value, hint, role, state, screen frame) that screen readers (VoiceOver, Narrator,
//! Orca/AT-SPI, TalkBack) read and act on, set as [`AssistiveSupport::active_hierarchy`];
//! announcements; and [`Accessible`], a component that labels Canvas UI objects. The game
//! UI's buttons, sliders and toggles are readable without it.
//!
//! The nodes go into egui's AccessKit tree, which the engine hands to the platform.
//! Reference: <https://docs.unity3d.com/Manual/accessibility/_index.html>

use crate::{TextMeshProUGUI, UIButton, UIImage, UISlider, UIText, UIToggle};
use egui::accesskit;
use oxide_core::Reflect;
use oxide_math::Rect;
use oxide_scene::{Component, GameObjectId, World};
use std::collections::{BTreeMap, HashMap};

/// Unity `AccessibilityRole`: what kind of element a node is.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessibilityRole {
    #[default]
    None,
    Button,
    Image,
    StaticText,
    SearchField,
    KeyboardKey,
    Header,
    TabBar,
    Slider,
    Toggle,
}

impl AccessibilityRole {
    fn to_accesskit(self) -> accesskit::Role {
        use accesskit::Role as R;
        match self {
            Self::None => R::GenericContainer,
            Self::Button | Self::KeyboardKey => R::Button,
            Self::Image => R::Image,
            Self::StaticText => R::Label,
            Self::SearchField => R::SearchInput,
            Self::Header => R::Heading,
            Self::TabBar => R::TabList,
            Self::Slider => R::Slider,
            Self::Toggle => R::CheckBox,
        }
    }
}

/// Unity `AccessibilityState` flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessibilityState {
    pub disabled: bool,
    pub selected: bool,
}

/// Unity `AccessibilityNode`.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityNode {
    pub id: i32,
    pub label: String,
    pub value: String,
    pub hint: String,
    pub role: AccessibilityRole,
    pub state: AccessibilityState,
    /// Where it is on screen: pixels from the game view's top-left.
    pub frame: Rect,
    /// Inactive nodes (and their children) are left out.
    pub is_active: bool,
    parent: Option<i32>,
    children: Vec<i32>,
}

impl AccessibilityNode {
    pub fn parent(&self) -> Option<i32> {
        self.parent
    }
    pub fn children(&self) -> &[i32] {
        &self.children
    }
}

/// Unity `AccessibilityHierarchy`: the tree of nodes a screen reader navigates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccessibilityHierarchy {
    nodes: BTreeMap<i32, AccessibilityNode>,
    roots: Vec<i32>,
    next_id: i32,
}

impl AccessibilityHierarchy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a node at the end of `parent`'s children (or the roots).
    pub fn add_node(&mut self, label: &str, parent: Option<i32>) -> i32 {
        let n = self.siblings(parent).map_or(0, |s| s.len());
        self.insert_node(n, label, parent)
    }

    /// Adds a node at `index` among `parent`'s children (or the roots).
    pub fn insert_node(&mut self, index: usize, label: &str, parent: Option<i32>) -> i32 {
        let parent = parent.filter(|p| self.nodes.contains_key(p));
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.insert(
            id,
            AccessibilityNode {
                id,
                label: label.to_owned(),
                value: String::new(),
                hint: String::new(),
                role: AccessibilityRole::None,
                state: AccessibilityState::default(),
                frame: Rect::default(),
                is_active: true,
                parent,
                children: Vec::new(),
            },
        );
        let list = self.siblings_mut(parent);
        let i = index.min(list.len());
        list.insert(i, id);
        id
    }

    fn siblings(&self, parent: Option<i32>) -> Option<&Vec<i32>> {
        match parent {
            Some(p) => self.nodes.get(&p).map(|n| &n.children),
            None => Some(&self.roots),
        }
    }

    fn siblings_mut(&mut self, parent: Option<i32>) -> &mut Vec<i32> {
        match parent.and_then(|p| self.nodes.get_mut(&p)) {
            Some(n) => &mut n.children,
            None => &mut self.roots,
        }
    }

    /// Removes a node; its children go with it, or move up to its parent.
    pub fn remove_node(&mut self, id: i32, remove_children: bool) {
        let Some(node) = self.nodes.remove(&id) else {
            return;
        };
        let list = self.siblings_mut(node.parent);
        let at = list.iter().position(|&c| c == id).unwrap_or(list.len());
        list.retain(|&c| c != id);
        if remove_children {
            let mut stack = node.children;
            while let Some(c) = stack.pop() {
                if let Some(n) = self.nodes.remove(&c) {
                    stack.extend(n.children);
                }
            }
        } else {
            for (k, &c) in node.children.iter().enumerate() {
                if let Some(n) = self.nodes.get_mut(&c) {
                    n.parent = node.parent;
                }
                let list = self.siblings_mut(node.parent);
                list.insert((at + k).min(list.len()), c);
            }
        }
    }

    /// Moves a node under `new_parent` at `index`. False if that would make a cycle.
    pub fn move_node(&mut self, id: i32, new_parent: Option<i32>, index: usize) -> bool {
        if !self.nodes.contains_key(&id) || new_parent.is_some_and(|p| !self.nodes.contains_key(&p))
        {
            return false;
        }
        let mut up = new_parent;
        while let Some(p) = up {
            if p == id {
                return false;
            }
            up = self.nodes[&p].parent;
        }
        let old = self.nodes[&id].parent;
        self.siblings_mut(old).retain(|&c| c != id);
        let list = self.siblings_mut(new_parent);
        let i = index.min(list.len());
        list.insert(i, id);
        self.nodes.get_mut(&id).expect("checked").parent = new_parent;
        true
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.roots.clear();
    }

    pub fn contains(&self, id: i32) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn try_get_node(&self, id: i32) -> Option<&AccessibilityNode> {
        self.nodes.get(&id)
    }

    pub fn node_mut(&mut self, id: i32) -> Option<&mut AccessibilityNode> {
        self.nodes.get_mut(&id)
    }

    pub fn roots(&self) -> &[i32] {
        &self.roots
    }

    /// Active nodes depth first (parents before children), the reading order.
    pub fn nodes_in_order(&self) -> Vec<i32> {
        let mut out = Vec::new();
        let mut stack: Vec<i32> = self.roots.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(n) = self.nodes.get(&id) else {
                continue;
            };
            if !n.is_active {
                continue;
            }
            out.push(id);
            stack.extend(n.children.iter().rev());
        }
        out
    }
}

/// What a node or [`Accessible`] object the screen reader acted on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityTarget {
    Node(i32),
    Object(GameObjectId),
}

/// Screen reader actions (Unity's node events: `invoked`, `incremented`, `decremented`,
/// `focusChanged`, `dismissed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityAction {
    Invoked,
    Incremented,
    Decremented,
    Focused,
    Dismissed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessibilityEvent {
    pub target: AccessibilityTarget,
    pub action: AccessibilityAction,
}

/// Unity `AssistiveSupport` (a World resource).
#[derive(Default)]
pub struct AssistiveSupport {
    /// The hierarchy screen readers read (Unity `AssistiveSupport.activeHierarchy`).
    pub active_hierarchy: Option<AccessibilityHierarchy>,
    screen_reader: bool,
    /// The latest announcement, re-sent while it changes.
    announcement: Option<(u64, String)>,
    announcements: u64,
    events: Vec<AccessibilityEvent>,
}

impl AssistiveSupport {
    /// Unity `AssistiveSupport.isScreenReaderEnabled`.
    pub fn is_screen_reader_enabled(&self) -> bool {
        self.screen_reader
    }

    /// Unity `notificationDispatcher.SendAnnouncement`: the screen reader speaks `text`.
    pub fn send_announcement(&mut self, text: &str) {
        self.announcements += 1;
        self.announcement = Some((self.announcements, text.to_owned()));
    }

    /// Actions since the last call.
    pub fn take_events(&mut self) -> Vec<AccessibilityEvent> {
        std::mem::take(&mut self.events)
    }

    /// Actions not taken yet.
    pub fn pending_events(&self) -> &[AccessibilityEvent] {
        &self.events
    }
}

/// Labels a Canvas UI object for screen readers (empty fields come from its UI component:
/// a Text's text, a Button's caption, a Slider's value...). On a Button, Slider or Toggle it
/// changes what the control announces; on text and images it adds an element.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct Accessible {
    pub label: String,
    pub hint: String,
    pub value: String,
    /// `None` picks it from the UI component.
    pub role: AccessibilityRole,
    /// Leave the object out (decorative images).
    pub hidden: bool,
}

impl Default for Accessible {
    fn default() -> Self {
        Self {
            label: String::new(),
            hint: String::new(),
            value: String::new(),
            role: AccessibilityRole::None,
            hidden: false,
        }
    }
}

impl Component for Accessible {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// The AccessKit tree the engine last handed to the platform (filled while a screen reader
/// or the `accessibility` MCP tool has the tree on): every node with a label or value.
#[derive(Clone, Debug, Default)]
pub struct AccessKitSnapshot {
    pub nodes: Vec<SnapshotNode>,
}

#[derive(Clone, Debug)]
pub struct SnapshotNode {
    pub id: accesskit::NodeId,
    pub role: String,
    pub label: String,
    pub value: String,
    pub description: String,
    pub actions: Vec<String>,
}

impl AccessKitSnapshot {
    /// Records the labelled nodes of a tree update.
    pub fn record(update: &accesskit::TreeUpdate) -> Self {
        use accesskit::Action as A;
        Self {
            nodes: update
                .nodes
                .iter()
                .filter(|(_, n)| n.label().is_some() || n.value().is_some())
                .map(|(id, n)| SnapshotNode {
                    id: *id,
                    role: format!("{:?}", n.role()),
                    label: n.label().unwrap_or_default().to_owned(),
                    value: n.value().unwrap_or_default().to_owned(),
                    description: n.description().unwrap_or_default().to_owned(),
                    actions: [
                        (A::Click, "click"),
                        (A::Increment, "increment"),
                        (A::Decrement, "decrement"),
                        (A::Focus, "focus"),
                    ]
                    .into_iter()
                    .filter(|(a, _)| n.supports_action(*a))
                    .map(|(_, s)| s.to_owned())
                    .collect(),
                })
                .collect(),
        }
    }
}

/// Where the UI system drew each object this frame, and the egui widget behind it.
#[derive(Default)]
pub struct DrawnUi {
    pub rects: HashMap<GameObjectId, egui::Rect>,
    pub widgets: HashMap<GameObjectId, egui::Id>,
}

/// Plain text of rich text (tags dropped).
fn plain(text: &str) -> String {
    oxide_text::rich::parse(text, true, 10.0)
        .into_iter()
        .map(|c| c.c)
        .filter(|&c| c != oxide_text::rich::SPRITE_CHAR)
        .collect()
}

/// The label, value and role an [`Accessible`] object gets.
fn describe(
    world: &World,
    id: GameObjectId,
    a: &Accessible,
) -> (String, String, AccessibilityRole) {
    let (label, value, role) = if let Some(b) = world.get_component::<UIButton>(id) {
        (b.text.clone(), String::new(), AccessibilityRole::Button)
    } else if let Some(s) = world.get_component::<UISlider>(id) {
        (
            s.label.clone(),
            format!("{}", s.value),
            AccessibilityRole::Slider,
        )
    } else if let Some(t) = world.get_component::<UIToggle>(id) {
        (
            t.text.clone(),
            if t.is_on { "on" } else { "off" }.into(),
            AccessibilityRole::Toggle,
        )
    } else if let Some(t) = world.get_component::<UIText>(id) {
        (t.text.clone(), String::new(), AccessibilityRole::StaticText)
    } else if let Some(t) = world.get_component::<TextMeshProUGUI>(id) {
        (plain(&t.text), String::new(), AccessibilityRole::StaticText)
    } else if world.get_component::<UIImage>(id).is_some() {
        (String::new(), String::new(), AccessibilityRole::Image)
    } else {
        (String::new(), String::new(), AccessibilityRole::None)
    };
    let pick = |own: &str, auto: String| if own.is_empty() { auto } else { own.to_owned() };
    (
        pick(&a.label, label),
        pick(&a.value, value),
        if a.role == AccessibilityRole::None {
            role
        } else {
            a.role
        },
    )
}

struct NodeOut<'a> {
    label: &'a str,
    value: &'a str,
    hint: &'a str,
    role: AccessibilityRole,
    state: AccessibilityState,
}

fn fill(b: &mut accesskit::Node, n: &NodeOut) {
    b.set_role(n.role.to_accesskit());
    if !n.label.is_empty() {
        b.set_label(n.label);
    }
    if !n.value.is_empty() {
        b.set_value(n.value);
    }
    if !n.hint.is_empty() {
        b.set_description(n.hint);
    }
    if n.state.disabled {
        b.set_disabled();
    }
    if n.state.selected {
        b.set_selected(true);
    }
    match n.role {
        AccessibilityRole::Button | AccessibilityRole::KeyboardKey | AccessibilityRole::Toggle => {
            b.add_action(accesskit::Action::Click)
        }
        AccessibilityRole::Slider => {
            b.add_action(accesskit::Action::Increment);
            b.add_action(accesskit::Action::Decrement);
        }
        _ => {}
    }
    b.add_action(accesskit::Action::Focus);
}

fn actions(
    ctx: &egui::Context,
    id: egui::Id,
    target: AccessibilityTarget,
    out: &mut Vec<AccessibilityEvent>,
) {
    use accesskit::Action as A;
    for (a, action) in [
        (A::Click, AccessibilityAction::Invoked),
        (A::Increment, AccessibilityAction::Incremented),
        (A::Decrement, AccessibilityAction::Decremented),
        (A::Focus, AccessibilityAction::Focused),
        (A::Blur, AccessibilityAction::Dismissed),
    ] {
        if ctx.input(|i| i.has_accesskit_action_request(id, a)) {
            out.push(AccessibilityEvent { target, action });
        }
    }
}

/// Adds this frame's nodes to egui's AccessKit tree and collects screen reader actions
/// (called by the UI system after drawing). `screen` is the game view in points; node frames
/// are in its pixels.
pub fn update(world: &mut World, ctx: &egui::Context, screen: egui::Rect, drawn: &DrawnUi) {
    if world.try_resource::<AssistiveSupport>().is_none() {
        world.resources.insert(AssistiveSupport::default());
    }
    let enabled = ctx
        .accesskit_node_builder(egui::accesskit_root_id(), |_| ())
        .is_some();
    let mut support = world
        .resources
        .remove::<AssistiveSupport>()
        .expect("inserted");
    support.screen_reader = enabled;
    if !enabled {
        world.resources.insert(support);
        return;
    }
    let ppp = ctx.pixels_per_point();
    let mut events = Vec::new();
    let accessible: Vec<(GameObjectId, Accessible)> = world
        .query::<Accessible>()
        .filter(|(id, a)| !a.hidden && world.get(*id).is_some_and(|o| o.active_in_hierarchy()))
        .map(|(id, a)| (id, a.clone()))
        .collect();
    let mut clicked: Vec<GameObjectId> = Vec::new();
    egui::Area::new(egui::Id::new("oxide-accessibility"))
        .order(egui::Order::Background)
        .fixed_pos(screen.min)
        .interactable(false)
        .show(ctx, |ui| {
            // Script hierarchy, depth first so parents exist before their children.
            if let Some(h) = &support.active_hierarchy {
                for nid in h.nodes_in_order() {
                    let n = &h.nodes[&nid];
                    let id = egui::Id::new(("oxide-ax-node", nid));
                    let rect = egui::Rect::from_min_size(
                        screen.min + egui::vec2(n.frame.x, n.frame.y) / ppp,
                        egui::vec2(n.frame.width, n.frame.height) / ppp,
                    );
                    let mut b = egui::UiBuilder::new()
                        .id_salt(("oxide-ax-ui", nid))
                        .max_rect(rect);
                    if let Some(p) = n.parent {
                        b = b.accessibility_parent(egui::Id::new(("oxide-ax-node", p)));
                    }
                    let child = ui.new_child(b);
                    child.interact(rect, id, egui::Sense::hover());
                    ctx.accesskit_node_builder(id, |b| {
                        fill(
                            b,
                            &NodeOut {
                                label: &n.label,
                                value: &n.value,
                                hint: &n.hint,
                                role: n.role,
                                state: n.state,
                            },
                        )
                    });
                    actions(ctx, id, AccessibilityTarget::Node(nid), &mut events);
                }
            }
            // Accessible components.
            for (oid, a) in &accessible {
                let (label, value, role) = describe(world, *oid, a);
                let node = NodeOut {
                    label: &label,
                    value: &value,
                    hint: &a.hint,
                    role,
                    state: AccessibilityState::default(),
                };
                let id = match drawn.widgets.get(oid) {
                    // A Button / Slider / Toggle: relabel its own node.
                    Some(w) => *w,
                    None => {
                        let Some(rect) = drawn.rects.get(oid) else {
                            continue;
                        };
                        let id = egui::Id::new(("oxide-ax-object", oid.to_bits()));
                        ui.interact(*rect, id, egui::Sense::hover());
                        id
                    }
                };
                ctx.accesskit_node_builder(id, |b| fill(b, &node));
                let before = events.len();
                actions(ctx, id, AccessibilityTarget::Object(*oid), &mut events);
                if events[before..]
                    .iter()
                    .any(|e| e.action == AccessibilityAction::Invoked)
                {
                    clicked.push(*oid);
                }
            }
            // The latest announcement, as an assertive live region.
            if let Some((n, text)) = &support.announcement {
                let id = egui::Id::new(("oxide-ax-announcement", *n));
                ui.interact(
                    egui::Rect::from_min_size(screen.min, egui::Vec2::ZERO),
                    id,
                    egui::Sense::hover(),
                );
                ctx.accesskit_node_builder(id, |b| {
                    b.set_role(accesskit::Role::Label);
                    b.set_label(text.as_str());
                    b.set_live(accesskit::Live::Assertive);
                });
            }
        });
    support.events.extend(events);
    world.resources.insert(support);
    // Activating an Accessible object counts as a click on it.
    for id in clicked {
        world.send_message(
            id,
            "OnAccessibilityInvoked",
            oxide_core::reflect::Value::Null,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy_add_insert_move_remove() {
        let mut h = AccessibilityHierarchy::new();
        let menu = h.add_node("Menu", None);
        let play = h.add_node("Play", Some(menu));
        let quit = h.add_node("Quit", Some(menu));
        let opts = h.insert_node(1, "Options", Some(menu));
        let footer = h.add_node("Footer", None);
        assert_eq!(h.try_get_node(menu).unwrap().children(), [play, opts, quit]);
        assert_eq!(h.nodes_in_order(), [menu, play, opts, quit, footer]);
        // No cycles.
        assert!(!h.move_node(menu, Some(play), 0));
        assert!(h.move_node(quit, Some(footer), 0));
        assert_eq!(h.try_get_node(quit).unwrap().parent(), Some(footer));
        assert_eq!(h.nodes_in_order(), [menu, play, opts, footer, quit]);
        // Inactive nodes drop out with their children.
        h.node_mut(footer).unwrap().is_active = false;
        assert_eq!(h.nodes_in_order(), [menu, play, opts]);
        h.node_mut(footer).unwrap().is_active = true;
        // Removing keeps or drops the children.
        h.remove_node(menu, false);
        assert_eq!(h.roots(), [play, opts, footer]);
        assert_eq!(h.try_get_node(play).unwrap().parent(), None);
        h.remove_node(footer, true);
        assert!(!h.contains(quit));
        assert_eq!(h.nodes_in_order(), [play, opts]);
        h.clear();
        assert!(h.nodes_in_order().is_empty());
    }

    /// Runs one egui frame with AccessKit on; returns the tree's nodes.
    fn frame(
        world: &mut World,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        drawn: &DrawnUi,
    ) -> Vec<(accesskit::NodeId, accesskit::Node)> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            update(world, ui.ctx(), ui.ctx().content_rect(), drawn);
        });
        out.textures_delta.clear();
        out.platform_output
            .accesskit_update
            .map(|u| u.nodes)
            .unwrap_or_default()
    }

    #[test]
    fn nodes_reach_the_accesskit_tree_and_actions_come_back() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut world = World::new();
        let mut h = AccessibilityHierarchy::new();
        let menu = h.add_node("Pause menu", None);
        let resume = h.add_node("Resume", Some(menu));
        let volume = h.add_node("Volume", Some(menu));
        {
            let n = h.node_mut(resume).unwrap();
            n.role = AccessibilityRole::Button;
            n.hint = "Back to the race".into();
            n.frame = Rect::new(10.0, 10.0, 200.0, 40.0);
        }
        {
            let n = h.node_mut(volume).unwrap();
            n.role = AccessibilityRole::Slider;
            n.value = "80%".into();
        }
        let mut support = AssistiveSupport {
            active_hierarchy: Some(h),
            ..Default::default()
        };
        support.send_announcement("Lap 2");
        world.resources.insert(support);
        // A text object labelled for screen readers.
        let label = world.create("Speed");
        world.add_component(
            label,
            UIText {
                text: "120 km/h".into(),
                ..Default::default()
            },
        );
        world.add_component(
            label,
            Accessible {
                hint: "Current speed".into(),
                ..Default::default()
            },
        );
        let mut drawn = DrawnUi::default();
        drawn.rects.insert(
            label,
            egui::Rect::from_min_size(egui::pos2(5.0, 500.0), egui::vec2(100.0, 30.0)),
        );

        let nodes = frame(&mut world, &ctx, vec![], &drawn);
        assert!(
            world
                .resource::<AssistiveSupport>()
                .is_screen_reader_enabled()
        );
        let find = |label: &str| {
            nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .unwrap_or_else(|| panic!("no node `{label}` in {nodes:#?}"))
                .clone()
        };
        let (menu_id, _) = find("Pause menu");
        let (resume_id, resume_node) = find("Resume");
        assert_eq!(resume_node.role(), accesskit::Role::Button);
        assert_eq!(resume_node.description(), Some("Back to the race"));
        assert!(resume_node.supports_action(accesskit::Action::Click));
        let (_, vol) = find("Volume");
        assert_eq!(
            (vol.role(), vol.value()),
            (accesskit::Role::Slider, Some("80%"))
        );
        // Resume sits under the menu (possibly through a container node).
        let is_under = |child: accesskit::NodeId, ancestor: accesskit::NodeId| {
            let mut cur = child;
            for _ in 0..8 {
                match nodes.iter().find(|(_, n)| n.children().contains(&cur)) {
                    Some((p, _)) if *p == ancestor => return true,
                    Some((p, _)) => cur = *p,
                    None => return false,
                }
            }
            false
        };
        assert!(is_under(resume_id, menu_id));
        let (_, speed) = find("120 km/h");
        assert_eq!(
            (speed.role(), speed.description()),
            (accesskit::Role::Label, Some("Current speed"))
        );
        let (_, ann) = find("Lap 2");
        assert_eq!(ann.live(), Some(accesskit::Live::Assertive));

        // The screen reader activates Resume.
        let request = egui::Event::AccessKitActionRequest(accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: resume_id,
            data: None,
        });
        frame(&mut world, &ctx, vec![request], &drawn);
        let events = world.resource_mut::<AssistiveSupport>().take_events();
        assert_eq!(
            events,
            [AccessibilityEvent {
                target: AccessibilityTarget::Node(resume),
                action: AccessibilityAction::Invoked,
            }]
        );
    }

    #[test]
    fn off_without_a_screen_reader() {
        let ctx = egui::Context::default();
        let mut world = World::new();
        frame(&mut world, &ctx, vec![], &DrawnUi::default());
        assert!(
            !world
                .resource::<AssistiveSupport>()
                .is_screen_reader_enabled()
        );
    }
}
