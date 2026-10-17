//! Unity's RectTransform and uGUI auto layout: Horizontal / Vertical / Grid Layout Groups
//! position (and optionally size) their children, Layout Element overrides a child's
//! min / preferred / flexible sizes, Content Size Fitter sizes an object to its content.
//!
//! Rects are in canvas units (reference pixels before the Canvas Scaler), y down from the
//! screen's top-left. Anchors, pivots and anchored positions follow Unity (y up).
//! Objects without a RectTransform that sit under one (or under a layout group) get one
//! from their own anchor / position / size fields.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.ugui@2.0/manual/comp-UIAutoLayout.html>

use crate::{Anchor, Canvas, TextMeshProUGUI, UIButton, UIImage, UISlider, UIText, UIToggle};
use oxide_core::Reflect;
use oxide_math::Vector2;
use oxide_scene::{Component, GameObjectId, World};
use std::collections::HashMap;

/// A rectangle in canvas units, y down.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl UiRect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    fn size(&self, axis: usize) -> f32 {
        if axis == 0 { self.w } else { self.h }
    }
}

/// Unity `RectTransform`: the rectangle a UI object occupies, relative to its parent's.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct RectTransform {
    /// Anchors as fractions of the parent rect, (0, 0) = bottom-left.
    pub anchor_min: Vector2,
    pub anchor_max: Vector2,
    /// Pivot offset from the anchors' reference point (+Y up).
    pub anchored_position: Vector2,
    /// Size minus the anchors' span (the size itself when the anchors meet).
    pub size_delta: Vector2,
    /// The point the rect is positioned and sized around, (0, 0) = bottom-left.
    pub pivot: Vector2,
}

impl Default for RectTransform {
    fn default() -> Self {
        Self {
            anchor_min: Vector2::new(0.5, 0.5),
            anchor_max: Vector2::new(0.5, 0.5),
            anchored_position: Vector2::ZERO,
            size_delta: Vector2::new(100.0, 100.0),
            pivot: Vector2::new(0.5, 0.5),
        }
    }
}

impl Component for RectTransform {
    fn allow_multiple(&self) -> bool {
        false
    }
}

impl RectTransform {
    /// Anchored to a preset point (pivot there too), like the legacy `anchor` fields.
    pub fn from_anchor(anchor: Anchor, position: Vector2, size: Vector2) -> Self {
        let a = anchor.align();
        let p = Vector2::new(a.x().to_factor(), 1.0 - a.y().to_factor());
        Self {
            anchor_min: p,
            anchor_max: p,
            anchored_position: position,
            size_delta: size,
            pivot: p,
        }
    }

    /// Stretched over the whole parent.
    pub fn stretch() -> Self {
        Self {
            anchor_min: Vector2::ZERO,
            anchor_max: Vector2::new(1.0, 1.0),
            size_delta: Vector2::ZERO,
            ..Default::default()
        }
    }

    /// Unity `rect` in `parent`'s space.
    pub fn rect_in(&self, p: UiRect) -> UiRect {
        let (ax0, ax1) = (p.x + self.anchor_min.x * p.w, p.x + self.anchor_max.x * p.w);
        let w = ax1 - ax0 + self.size_delta.x;
        let x = ax0 + (ax1 - ax0) * self.pivot.x + self.anchored_position.x - w * self.pivot.x;
        // Anchors and pivot count y up from the parent's bottom edge.
        let bottom = p.y + p.h;
        let (ay0, ay1) = (
            bottom - self.anchor_min.y * p.h,
            bottom - self.anchor_max.y * p.h,
        );
        let h = ay0 - ay1 + self.size_delta.y;
        let pivot_y = ay0 - (ay0 - ay1) * self.pivot.y - self.anchored_position.y;
        let y = pivot_y - h * (1.0 - self.pivot.y);
        UiRect { x, y, w, h }
    }

    fn anchor_span(&self, axis: usize) -> f32 {
        if axis == 0 {
            self.anchor_max.x - self.anchor_min.x
        } else {
            self.anchor_max.y - self.anchor_min.y
        }
    }
}

/// Unity `RectOffset`: padding inside a layout group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct RectOffset {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl RectOffset {
    fn along(&self, axis: usize) -> (f32, f32) {
        if axis == 0 {
            (self.left, self.left + self.right)
        } else {
            (self.top, self.top + self.bottom)
        }
    }
}

/// Unity `HorizontalLayoutGroup`: children side by side.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct HorizontalLayoutGroup {
    pub padding: RectOffset,
    pub spacing: f32,
    pub child_alignment: Anchor,
    pub reverse_arrangement: bool,
    /// Size children from their layout sizes (else they keep their own size).
    pub control_child_width: bool,
    pub control_child_height: bool,
    /// Children take the leftover space (counts them as flexible).
    pub child_force_expand_width: bool,
    pub child_force_expand_height: bool,
}

impl Default for HorizontalLayoutGroup {
    fn default() -> Self {
        Self {
            padding: RectOffset::default(),
            spacing: 0.0,
            child_alignment: Anchor::TopLeft,
            reverse_arrangement: false,
            control_child_width: false,
            control_child_height: false,
            child_force_expand_width: true,
            child_force_expand_height: true,
        }
    }
}

impl Component for HorizontalLayoutGroup {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity `VerticalLayoutGroup`: children stacked top to bottom.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct VerticalLayoutGroup {
    pub padding: RectOffset,
    pub spacing: f32,
    pub child_alignment: Anchor,
    pub reverse_arrangement: bool,
    pub control_child_width: bool,
    pub control_child_height: bool,
    pub child_force_expand_width: bool,
    pub child_force_expand_height: bool,
}

impl Default for VerticalLayoutGroup {
    fn default() -> Self {
        let h = HorizontalLayoutGroup::default();
        Self {
            padding: h.padding,
            spacing: h.spacing,
            child_alignment: h.child_alignment,
            reverse_arrangement: h.reverse_arrangement,
            control_child_width: h.control_child_width,
            control_child_height: h.control_child_height,
            child_force_expand_width: h.child_force_expand_width,
            child_force_expand_height: h.child_force_expand_height,
        }
    }
}

impl Component for VerticalLayoutGroup {
    fn allow_multiple(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum GridCorner {
    #[default]
    UpperLeft,
    UpperRight,
    LowerLeft,
    LowerRight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum GridAxis {
    /// Fill rows first.
    #[default]
    Horizontal,
    /// Fill columns first.
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum GridConstraint {
    /// As many columns as fit the width.
    #[default]
    Flexible,
    FixedColumnCount,
    FixedRowCount,
}

/// Unity `GridLayoutGroup`: children in equal cells.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct GridLayoutGroup {
    pub padding: RectOffset,
    pub cell_size: Vector2,
    pub spacing: Vector2,
    pub start_corner: GridCorner,
    pub start_axis: GridAxis,
    pub child_alignment: Anchor,
    pub constraint: GridConstraint,
    pub constraint_count: u32,
}

impl Default for GridLayoutGroup {
    fn default() -> Self {
        Self {
            padding: RectOffset::default(),
            cell_size: Vector2::new(100.0, 100.0),
            spacing: Vector2::ZERO,
            start_corner: GridCorner::UpperLeft,
            start_axis: GridAxis::Horizontal,
            child_alignment: Anchor::TopLeft,
            constraint: GridConstraint::Flexible,
            constraint_count: 2,
        }
    }
}

impl Component for GridLayoutGroup {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity `LayoutElement`: overrides the sizes a layout group sees (negative = not set).
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct LayoutElement {
    /// Layout groups skip this object.
    pub ignore_layout: bool,
    pub min_width: f32,
    pub min_height: f32,
    pub preferred_width: f32,
    pub preferred_height: f32,
    /// Share of leftover space (relative to siblings).
    pub flexible_width: f32,
    pub flexible_height: f32,
}

impl Default for LayoutElement {
    fn default() -> Self {
        Self {
            ignore_layout: false,
            min_width: -1.0,
            min_height: -1.0,
            preferred_width: -1.0,
            preferred_height: -1.0,
            flexible_width: -1.0,
            flexible_height: -1.0,
        }
    }
}

impl Component for LayoutElement {
    fn allow_multiple(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum FitMode {
    #[default]
    Unconstrained,
    MinSize,
    PreferredSize,
}

/// Unity `ContentSizeFitter`: sizes the object to its layout's min or preferred size.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct ContentSizeFitter {
    pub horizontal_fit: FitMode,
    pub vertical_fit: FitMode,
}

impl Component for ContentSizeFitter {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Computed rects of the UI objects that use RectTransforms (a World resource, rebuilt
/// each frame by the UI system).
#[derive(Clone, Debug, Default)]
pub struct UiLayout {
    pub rects: HashMap<GameObjectId, UiRect>,
}

impl UiLayout {
    pub fn rect(&self, id: GameObjectId) -> Option<UiRect> {
        self.rects.get(&id).copied()
    }
}

/// Fraction of the free space an alignment puts before the content, per axis (y down).
fn alignment(a: Anchor, axis: usize) -> f32 {
    let al = a.align();
    if axis == 0 {
        al.x().to_factor()
    } else {
        al.y().to_factor()
    }
}

struct Linear {
    main: usize,
    padding: RectOffset,
    spacing: f32,
    align: Anchor,
    reverse: bool,
    control: [bool; 2],
    expand: [bool; 2],
}

enum Group {
    Linear(Linear),
    Grid(GridLayoutGroup),
}

/// (min, preferred, flexible) along one axis.
type Sizes = (f32, f32, f32);

/// Lays out the RectTransform hierarchy.
pub struct LayoutEngine<'a> {
    world: &'a World,
    /// Content sizes (text) in canvas units.
    intrinsic: &'a HashMap<GameObjectId, Vector2>,
    pub rects: HashMap<GameObjectId, UiRect>,
}

impl<'a> LayoutEngine<'a> {
    pub fn new(world: &'a World, intrinsic: &'a HashMap<GameObjectId, Vector2>) -> Self {
        Self {
            world,
            intrinsic,
            rects: HashMap::new(),
        }
    }

    fn has_layout(&self, id: GameObjectId) -> bool {
        let w = self.world;
        w.get_component::<RectTransform>(id).is_some()
            || w.get_component::<HorizontalLayoutGroup>(id).is_some()
            || w.get_component::<VerticalLayoutGroup>(id).is_some()
            || w.get_component::<GridLayoutGroup>(id).is_some()
            || w.get_component::<ContentSizeFitter>(id).is_some()
            || w.get_component::<LayoutElement>(id).is_some()
    }

    /// The object's RectTransform, or one made from its UI component's anchor fields.
    pub fn rect_transform(&self, id: GameObjectId) -> Option<RectTransform> {
        let w = self.world;
        if let Some(rt) = w.get_component::<RectTransform>(id) {
            return Some(rt.clone());
        }
        let fit = |size: Vector2| -> Vector2 {
            let i = self.intrinsic.get(&id).copied().unwrap_or(Vector2::ZERO);
            Vector2::new(
                if size.x > 0.0 { size.x } else { i.x },
                if size.x > 0.0 { size.y.max(i.y) } else { i.y },
            )
        };
        if let Some(c) = w.get_component::<UIImage>(id) {
            return Some(RectTransform::from_anchor(c.anchor, c.position, c.size));
        }
        if let Some(c) = w.get_component::<UIButton>(id) {
            return Some(RectTransform::from_anchor(c.anchor, c.position, c.size));
        }
        if let Some(c) = w.get_component::<UIText>(id) {
            return Some(RectTransform::from_anchor(
                c.anchor,
                c.position,
                fit(c.size),
            ));
        }
        if let Some(c) = w.get_component::<TextMeshProUGUI>(id) {
            return Some(RectTransform::from_anchor(
                c.anchor,
                c.position,
                fit(c.size),
            ));
        }
        if let Some(c) = w.get_component::<UISlider>(id) {
            let i = self.intrinsic.get(&id).copied().unwrap_or(Vector2::ZERO);
            return Some(RectTransform::from_anchor(
                c.anchor,
                c.position,
                Vector2::new(c.width + i.x, 20.0),
            ));
        }
        if let Some(c) = w.get_component::<UIToggle>(id) {
            return Some(RectTransform::from_anchor(
                c.anchor,
                c.position,
                fit(Vector2::ZERO),
            ));
        }
        if self.has_layout(id) {
            return Some(RectTransform::default());
        }
        None
    }

    fn active(&self, id: GameObjectId) -> bool {
        self.world.get(id).is_some_and(|o| o.active_in_hierarchy())
    }

    /// Active children that are UI objects.
    fn ui_children(&self, id: GameObjectId) -> Vec<GameObjectId> {
        self.world
            .children(id)
            .iter()
            .copied()
            .filter(|&c| self.active(c) && self.rect_transform(c).is_some())
            .collect()
    }

    /// Children a layout group arranges.
    fn layout_children(&self, id: GameObjectId) -> Vec<GameObjectId> {
        self.ui_children(id)
            .into_iter()
            .filter(|&c| {
                !self
                    .world
                    .get_component::<LayoutElement>(c)
                    .is_some_and(|e| e.ignore_layout)
            })
            .collect()
    }

    fn group(&self, id: GameObjectId) -> Option<Group> {
        let w = self.world;
        if let Some(g) = w.get_component::<HorizontalLayoutGroup>(id) {
            return Some(Group::Linear(Linear {
                main: 0,
                padding: g.padding,
                spacing: g.spacing,
                align: g.child_alignment,
                reverse: g.reverse_arrangement,
                control: [g.control_child_width, g.control_child_height],
                expand: [g.child_force_expand_width, g.child_force_expand_height],
            }));
        }
        if let Some(g) = w.get_component::<VerticalLayoutGroup>(id) {
            return Some(Group::Linear(Linear {
                main: 1,
                padding: g.padding,
                spacing: g.spacing,
                align: g.child_alignment,
                reverse: g.reverse_arrangement,
                control: [g.control_child_width, g.control_child_height],
                expand: [g.child_force_expand_width, g.child_force_expand_height],
            }));
        }
        w.get_component::<GridLayoutGroup>(id)
            .cloned()
            .map(Group::Grid)
    }

    /// Unity's layout sizes of `id` along `axis` (LayoutUtility.GetMinSize / GetPreferredSize
    /// / GetFlexibleSize). `width` is the object's width when known (for heights that depend
    /// on it, like a flexible grid's).
    pub fn sizes(&self, id: GameObjectId, axis: usize, width: Option<f32>) -> Sizes {
        let (mut mn, mut pf, mut fl) = match self.group(id) {
            Some(Group::Linear(l)) => self.linear_sizes(id, &l, axis),
            Some(Group::Grid(g)) => self.grid_sizes(id, &g, axis, width),
            None => {
                let i = self.intrinsic.get(&id).copied().unwrap_or(Vector2::ZERO);
                (0.0, if axis == 0 { i.x } else { i.y }, 0.0)
            }
        };
        if let Some(e) = self.world.get_component::<LayoutElement>(id) {
            let (m, p, f) = if axis == 0 {
                (e.min_width, e.preferred_width, e.flexible_width)
            } else {
                (e.min_height, e.preferred_height, e.flexible_height)
            };
            if m >= 0.0 {
                mn = m;
            }
            if p >= 0.0 {
                pf = p;
            }
            if f >= 0.0 {
                fl = f;
            }
        }
        (mn, pf.max(mn), fl)
    }

    fn child_sizes(
        &self,
        l: &Linear,
        child: GameObjectId,
        axis: usize,
        parent: UiRect,
        width: Option<f32>,
    ) -> Sizes {
        let (mn, pf, mut fl) = if l.control[axis] {
            self.sizes(child, axis, width)
        } else {
            let s = self.natural_rect(child, parent).size(axis);
            (s, s, 0.0)
        };
        if l.expand[axis] {
            fl = fl.max(1.0);
        }
        (mn, pf, fl)
    }

    fn linear_sizes(&self, id: GameObjectId, l: &Linear, axis: usize) -> Sizes {
        let kids = self.layout_children(id);
        let (_, pad) = l.padding.along(axis);
        let sizes: Vec<Sizes> = kids
            .iter()
            .map(|&c| self.child_sizes(l, c, axis, UiRect::default(), None))
            .collect();
        if axis == l.main {
            let gaps = l.spacing * kids.len().saturating_sub(1) as f32;
            (
                pad + gaps + sizes.iter().map(|s| s.0).sum::<f32>(),
                pad + gaps + sizes.iter().map(|s| s.1.max(s.0)).sum::<f32>(),
                sizes.iter().map(|s| s.2).sum(),
            )
        } else {
            (
                pad + sizes.iter().map(|s| s.0).fold(0.0, f32::max),
                pad + sizes.iter().map(|s| s.1.max(s.0)).fold(0.0, f32::max),
                sizes.iter().map(|s| s.2).fold(0.0, f32::max),
            )
        }
    }

    fn grid_columns_for_width(g: &GridLayoutGroup, width: f32) -> usize {
        let step = g.cell_size.x + g.spacing.x;
        if step <= 0.0 {
            return usize::MAX;
        }
        (((width - g.padding.left - g.padding.right + g.spacing.x + 0.001) / step).floor() as i64)
            .max(1) as usize
    }

    fn grid_sizes(
        &self,
        id: GameObjectId,
        g: &GridLayoutGroup,
        axis: usize,
        width: Option<f32>,
    ) -> Sizes {
        let n = self.layout_children(id).len();
        let count = g.constraint_count.max(1) as usize;
        let span = |cells: usize, axis: usize| {
            let (_, pad) = g.padding.along(axis);
            let (cell, sp) = if axis == 0 {
                (g.cell_size.x, g.spacing.x)
            } else {
                (g.cell_size.y, g.spacing.y)
            };
            (pad + (cell + sp) * cells as f32 - sp).max(pad)
        };
        if axis == 0 {
            let (min_cols, pref_cols) = match g.constraint {
                GridConstraint::FixedColumnCount => (count, count),
                GridConstraint::FixedRowCount => (n.div_ceil(count), n.div_ceil(count)),
                GridConstraint::Flexible => (1, (n as f32).sqrt().ceil() as usize),
            };
            (span(min_cols, 0), span(pref_cols, 0), 0.0)
        } else {
            let rows = match g.constraint {
                GridConstraint::FixedColumnCount => n.div_ceil(count),
                GridConstraint::FixedRowCount => count,
                GridConstraint::Flexible => {
                    let w = width.unwrap_or_else(|| self.grid_sizes(id, g, 0, None).1);
                    n.div_ceil(Self::grid_columns_for_width(g, w).min(n.max(1)))
                }
            };
            let s = span(rows, 1);
            (s, s, 0.0)
        }
    }

    /// The object's rect under `parent` from its RectTransform, resized by a Content Size
    /// Fitter.
    pub fn natural_rect(&self, id: GameObjectId, parent: UiRect) -> UiRect {
        let Some(mut rt) = self.rect_transform(id) else {
            return parent;
        };
        if let Some(f) = self.world.get_component::<ContentSizeFitter>(id) {
            for (axis, fit) in [(0, f.horizontal_fit), (1, f.vertical_fit)] {
                if fit == FitMode::Unconstrained {
                    continue;
                }
                let width = (axis == 1).then(|| rt.rect_in(parent).w);
                let (mn, pf, _) = self.sizes(id, axis, width);
                let target = if fit == FitMode::MinSize { mn } else { pf };
                let d = target - rt.anchor_span(axis) * parent.size(axis);
                if axis == 0 {
                    rt.size_delta.x = d;
                } else {
                    rt.size_delta.y = d;
                }
            }
        }
        rt.rect_in(parent)
    }

    /// Places `id` at `rect`, then its children (arranged by its layout group, if any).
    pub fn place(&mut self, id: GameObjectId, rect: UiRect) {
        self.rects.insert(id, rect);
        let arranged: HashMap<GameObjectId, UiRect> = match self.group(id) {
            Some(Group::Linear(l)) => self.arrange_linear(&l, id, rect),
            Some(Group::Grid(g)) => self.arrange_grid(&g, id, rect),
            None => HashMap::new(),
        };
        for c in self.ui_children(id) {
            let r = arranged
                .get(&c)
                .copied()
                .unwrap_or_else(|| self.natural_rect(c, rect));
            self.place(c, r);
        }
    }

    /// Unity's HorizontalOrVerticalLayoutGroup.SetChildrenAlongAxis, both axes.
    fn arrange_linear(
        &self,
        l: &Linear,
        id: GameObjectId,
        rect: UiRect,
    ) -> HashMap<GameObjectId, UiRect> {
        let mut kids = self.layout_children(id);
        if l.reverse {
            kids.reverse();
        }
        let n = kids.len();
        let mut pos = vec![[0.0f32; 2]; n];
        let mut size = vec![[0.0f32; 2]; n];
        for axis in [0, 1] {
            let avail = rect.size(axis);
            let (pad_start, pad) = l.padding.along(axis);
            let align = alignment(l.align, axis);
            let start_offset = |required: f32| pad_start + (avail - (required + pad)) * align;
            let cs: Vec<Sizes> = kids
                .iter()
                .enumerate()
                .map(|(i, &c)| {
                    let width = (axis == 1).then_some(size[i][0]);
                    self.child_sizes(l, c, axis, rect, width)
                })
                .collect();
            let natural = |c: GameObjectId| self.natural_rect(c, rect).size(axis);
            if axis == l.main {
                let gaps = l.spacing * n.saturating_sub(1) as f32;
                let total_min = pad + gaps + cs.iter().map(|s| s.0).sum::<f32>();
                let total_pref = pad + gaps + cs.iter().map(|s| s.1).sum::<f32>();
                let total_flex: f32 = cs.iter().map(|s| s.2).sum();
                let mut p = pad_start;
                let mut per_flex = 0.0;
                let surplus = avail - total_pref;
                if surplus > 0.0 {
                    if total_flex == 0.0 {
                        p = start_offset(total_pref - pad);
                    } else {
                        per_flex = surplus / total_flex;
                    }
                }
                let t = if total_min != total_pref {
                    ((avail - total_min) / (total_pref - total_min)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                for (i, &c) in kids.iter().enumerate() {
                    let (mn, pf, fl) = cs[i];
                    let s = mn + (pf - mn) * t + fl * per_flex;
                    if l.control[axis] {
                        pos[i][axis] = p;
                        size[i][axis] = s;
                    } else {
                        let cur = natural(c);
                        pos[i][axis] = p + (s - cur) * align;
                        size[i][axis] = cur;
                    }
                    p += s + l.spacing;
                }
            } else {
                let inner = avail - pad;
                for (i, &c) in kids.iter().enumerate() {
                    let (mn, pf, fl) = cs[i];
                    let hi = if fl > 0.0 { avail } else { pf };
                    let required = if inner < mn {
                        mn
                    } else if inner > hi {
                        hi
                    } else {
                        inner
                    };
                    let start = start_offset(required);
                    if l.control[axis] {
                        pos[i][axis] = start;
                        size[i][axis] = required;
                    } else {
                        let cur = natural(c);
                        pos[i][axis] = start + (required - cur) * align;
                        size[i][axis] = cur;
                    }
                }
            }
        }
        kids.iter()
            .enumerate()
            .map(|(i, &c)| {
                (
                    c,
                    UiRect::new(
                        rect.x + pos[i][0],
                        rect.y + pos[i][1],
                        size[i][0],
                        size[i][1],
                    ),
                )
            })
            .collect()
    }

    /// Unity's GridLayoutGroup.SetCellsAlongAxis.
    fn arrange_grid(
        &self,
        g: &GridLayoutGroup,
        id: GameObjectId,
        rect: UiRect,
    ) -> HashMap<GameObjectId, UiRect> {
        let kids = self.layout_children(id);
        let n = kids.len();
        if n == 0 {
            return HashMap::new();
        }
        let count = g.constraint_count.max(1) as usize;
        let (cols, rows) = match g.constraint {
            GridConstraint::FixedColumnCount => (count, n.div_ceil(count)),
            GridConstraint::FixedRowCount => (n.div_ceil(count), count),
            GridConstraint::Flexible => {
                let rows = if g.cell_size.y + g.spacing.y <= 0.0 {
                    usize::MAX
                } else {
                    (((rect.h - g.padding.top - g.padding.bottom + g.spacing.y + 0.001)
                        / (g.cell_size.y + g.spacing.y))
                        .floor() as i64)
                        .max(1) as usize
                };
                (Self::grid_columns_for_width(g, rect.w), rows)
            }
        };
        let (per_main, actual_x, actual_y) = if g.start_axis == GridAxis::Horizontal {
            let x = cols.clamp(1, n);
            (
                cols.max(1),
                x,
                rows.clamp(1, n.div_ceil(cols.max(1)).max(1)),
            )
        } else {
            let y = rows.clamp(1, n);
            (
                rows.max(1),
                cols.clamp(1, n.div_ceil(rows.max(1)).max(1)),
                y,
            )
        };
        let required = [
            actual_x as f32 * g.cell_size.x + (actual_x as f32 - 1.0) * g.spacing.x,
            actual_y as f32 * g.cell_size.y + (actual_y as f32 - 1.0) * g.spacing.y,
        ];
        let start = [0, 1].map(|axis| {
            let (pad_start, pad) = g.padding.along(axis);
            pad_start
                + (rect.size(axis) - (required[axis] + pad)) * alignment(g.child_alignment, axis)
        });
        let (flip_x, flip_y) = match g.start_corner {
            GridCorner::UpperLeft => (false, false),
            GridCorner::UpperRight => (true, false),
            GridCorner::LowerLeft => (false, true),
            GridCorner::LowerRight => (true, true),
        };
        kids.iter()
            .enumerate()
            .map(|(i, &c)| {
                let (mut x, mut y) = if g.start_axis == GridAxis::Horizontal {
                    (i % per_main, i / per_main)
                } else {
                    (i / per_main, i % per_main)
                };
                if flip_x {
                    x = actual_x.saturating_sub(1 + x);
                }
                if flip_y {
                    y = actual_y.saturating_sub(1 + y);
                }
                (
                    c,
                    UiRect::new(
                        rect.x + start[0] + (g.cell_size.x + g.spacing.x) * x as f32,
                        rect.y + start[1] + (g.cell_size.y + g.spacing.y) * y as f32,
                        g.cell_size.x,
                        g.cell_size.y,
                    ),
                )
            })
            .collect()
    }
}

/// Lays out every RectTransform tree: objects with a RectTransform or layout component whose
/// parent has none are placed in `root(id)` (the screen in that object's canvas units).
pub fn compute(
    world: &World,
    intrinsic: &HashMap<GameObjectId, Vector2>,
    root: impl Fn(GameObjectId) -> UiRect,
) -> UiLayout {
    let mut e = LayoutEngine::new(world, intrinsic);
    for id in world.hierarchy_order() {
        if e.rects.contains_key(&id) || !e.active(id) || !e.has_layout(id) {
            continue;
        }
        if world.parent(id).is_some_and(|p| e.rects.contains_key(&p)) {
            continue;
        }
        let r = if world.get_component::<Canvas>(id).is_some() {
            root(id)
        } else {
            e.natural_rect(id, root(id))
        };
        e.place(id, r);
    }
    UiLayout { rects: e.rects }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        let mut w = World::new();
        w.registry.register::<RectTransform>("UI/Rect Transform");
        w
    }

    fn child(w: &mut World, parent: GameObjectId, size: Vector2) -> GameObjectId {
        let c = w.create("Item");
        w.set_parent(c, Some(parent), false).unwrap();
        w.add_component(
            c,
            RectTransform {
                size_delta: size,
                ..Default::default()
            },
        );
        c
    }

    const SCREEN: UiRect = UiRect {
        x: 0.0,
        y: 0.0,
        w: 800.0,
        h: 600.0,
    };

    #[test]
    fn rect_transform_anchors_and_pivots() {
        // Centered 100x100.
        assert_eq!(
            RectTransform::default().rect_in(SCREEN),
            UiRect::new(350.0, 250.0, 100.0, 100.0)
        );
        // Top-left corner, 10 in from each edge (y up).
        let tl = RectTransform::from_anchor(
            Anchor::TopLeft,
            Vector2::new(10.0, -10.0),
            Vector2::new(50.0, 20.0),
        );
        assert_eq!(tl.rect_in(SCREEN), UiRect::new(10.0, 10.0, 50.0, 20.0));
        // Bottom-right.
        let br = RectTransform::from_anchor(
            Anchor::BottomRight,
            Vector2::ZERO,
            Vector2::new(50.0, 20.0),
        );
        assert_eq!(br.rect_in(SCREEN), UiRect::new(750.0, 580.0, 50.0, 20.0));
        // Stretched with 20 px margins.
        let s = RectTransform {
            size_delta: Vector2::new(-40.0, -40.0),
            ..RectTransform::stretch()
        };
        assert_eq!(s.rect_in(SCREEN), UiRect::new(20.0, 20.0, 760.0, 560.0));
        // A top bar: stretched horizontally, 60 tall at the top.
        let bar = RectTransform {
            anchor_min: Vector2::new(0.0, 1.0),
            anchor_max: Vector2::new(1.0, 1.0),
            pivot: Vector2::new(0.5, 1.0),
            size_delta: Vector2::new(0.0, 60.0),
            ..Default::default()
        };
        assert_eq!(bar.rect_in(SCREEN), UiRect::new(0.0, 0.0, 800.0, 60.0));
    }

    #[test]
    fn horizontal_group_positions_and_expands() {
        let mut w = world();
        let row = w.create("Row");
        w.add_component(
            row,
            RectTransform {
                size_delta: Vector2::new(400.0, 100.0),
                ..Default::default()
            },
        );
        w.add_component(
            row,
            HorizontalLayoutGroup {
                padding: RectOffset {
                    left: 10.0,
                    right: 10.0,
                    top: 5.0,
                    bottom: 5.0,
                },
                spacing: 20.0,
                child_force_expand_width: false,
                child_force_expand_height: false,
                ..Default::default()
            },
        );
        let a = child(&mut w, row, Vector2::new(50.0, 30.0));
        let b = child(&mut w, row, Vector2::new(80.0, 40.0));
        let none = HashMap::new();
        let l = compute(&w, &none, |_| SCREEN);
        let r = l.rect(row).unwrap();
        assert_eq!(r, UiRect::new(200.0, 250.0, 400.0, 100.0));
        // Upper left, own sizes.
        assert_eq!(l.rect(a).unwrap(), UiRect::new(210.0, 255.0, 50.0, 30.0));
        assert_eq!(l.rect(b).unwrap(), UiRect::new(280.0, 255.0, 80.0, 40.0));

        // Controlling width with force expand: the 330 inner px minus spacing split evenly.
        w.get_component_mut::<HorizontalLayoutGroup>(row)
            .unwrap()
            .control_child_width = true;
        w.get_component_mut::<HorizontalLayoutGroup>(row)
            .unwrap()
            .child_force_expand_width = true;
        let l = compute(&w, &none, |_| SCREEN);
        assert_eq!(l.rect(a).unwrap().w, 180.0);
        assert_eq!(l.rect(b).unwrap().x, 210.0 + 180.0 + 20.0);
        // A Layout Element's flexible width takes a bigger share.
        w.add_component(
            b,
            LayoutElement {
                flexible_width: 3.0,
                ..Default::default()
            },
        );
        let l = compute(&w, &none, |_| SCREEN);
        assert_eq!(l.rect(a).unwrap().w, 90.0);
        assert_eq!(l.rect(b).unwrap().w, 270.0);
        // Middle-right alignment without expansion.
        let g = w.get_component_mut::<HorizontalLayoutGroup>(row).unwrap();
        g.child_force_expand_width = false;
        g.control_child_width = false;
        g.child_alignment = Anchor::MiddleRight;
        w.get_component_mut::<LayoutElement>(b)
            .unwrap()
            .flexible_width = -1.0;
        let l = compute(&w, &none, |_| SCREEN);
        // 50 + 20 + 80 = 150 wide, right edge at 600 - 10.
        assert_eq!(l.rect(a).unwrap().x, 590.0 - 150.0);
        assert_eq!(l.rect(b).unwrap().y, 250.0 + 30.0, "centered vertically");
        // Ignore Layout leaves a child where its RectTransform puts it.
        w.get_component_mut::<LayoutElement>(b)
            .unwrap()
            .ignore_layout = true;
        let l = compute(&w, &none, |_| SCREEN);
        assert_eq!(l.rect(a).unwrap().x, 590.0 - 50.0);
        assert_eq!(l.rect(b).unwrap(), UiRect::new(360.0, 280.0, 80.0, 40.0));
    }

    #[test]
    fn vertical_group_with_content_size_fitter_and_text() {
        let mut w = world();
        let list = w.create("List");
        w.add_component(
            list,
            RectTransform::from_anchor(Anchor::TopLeft, Vector2::ZERO, Vector2::new(10.0, 10.0)),
        );
        w.add_component(
            list,
            VerticalLayoutGroup {
                spacing: 4.0,
                control_child_width: true,
                control_child_height: true,
                child_force_expand_width: false,
                child_force_expand_height: false,
                ..Default::default()
            },
        );
        w.add_component(
            list,
            ContentSizeFitter {
                horizontal_fit: FitMode::PreferredSize,
                vertical_fit: FitMode::PreferredSize,
            },
        );
        let mut texts = Vec::new();
        let mut intrinsic = HashMap::new();
        for (i, width) in [120.0, 200.0, 80.0].into_iter().enumerate() {
            let t = w.create("Label");
            w.set_parent(t, Some(list), false).unwrap();
            w.add_component(t, UIText::default());
            intrinsic.insert(t, Vector2::new(width, 30.0 + i as f32));
            texts.push(t);
        }
        let l = compute(&w, &intrinsic, |_| SCREEN);
        // Fits the widest label and the stacked heights.
        assert_eq!(
            l.rect(list).unwrap(),
            UiRect::new(0.0, 0.0, 200.0, 30.0 + 31.0 + 32.0 + 8.0)
        );
        assert_eq!(
            l.rect(texts[1]).unwrap(),
            UiRect::new(0.0, 34.0, 200.0, 31.0)
        );
        assert_eq!(l.rect(texts[2]).unwrap().y, 34.0 + 31.0 + 4.0);
        // Inactive children drop out.
        w.set_active(texts[1], false);
        let l = compute(&w, &intrinsic, |_| SCREEN);
        assert_eq!(l.rect(list).unwrap().h, 30.0 + 32.0 + 4.0);
        assert_eq!(l.rect(list).unwrap().w, 120.0);
    }

    #[test]
    fn grid_layout_fills_cells() {
        let mut w = world();
        let grid = w.create("Inventory");
        // Stretched horizontally across 330 px, height fitted to the rows.
        w.add_component(
            grid,
            RectTransform {
                anchor_min: Vector2::new(0.0, 1.0),
                anchor_max: Vector2::new(1.0, 1.0),
                pivot: Vector2::new(0.5, 1.0),
                size_delta: Vector2::new(-470.0, 0.0),
                ..Default::default()
            },
        );
        w.add_component(
            grid,
            GridLayoutGroup {
                cell_size: Vector2::new(100.0, 50.0),
                spacing: Vector2::new(10.0, 10.0),
                ..Default::default()
            },
        );
        w.add_component(
            grid,
            ContentSizeFitter {
                horizontal_fit: FitMode::Unconstrained,
                vertical_fit: FitMode::PreferredSize,
            },
        );
        let items: Vec<GameObjectId> = (0..7)
            .map(|_| child(&mut w, grid, Vector2::new(10.0, 10.0)))
            .collect();
        let none = HashMap::new();
        let l = compute(&w, &none, |_| SCREEN);
        let r = l.rect(grid).unwrap();
        assert_eq!(r.w, 330.0);
        // 3 columns fit, 7 items -> 3 rows.
        assert_eq!(r.h, 3.0 * 50.0 + 2.0 * 10.0);
        assert_eq!(
            l.rect(items[0]).unwrap(),
            UiRect::new(r.x, r.y, 100.0, 50.0)
        );
        assert_eq!(l.rect(items[4]).unwrap().x, r.x + 110.0);
        assert_eq!(l.rect(items[4]).unwrap().y, r.y + 60.0);
        // Fixed 2 rows, filled column by column from the lower right.
        let g = w.get_component_mut::<GridLayoutGroup>(grid).unwrap();
        g.constraint = GridConstraint::FixedRowCount;
        g.constraint_count = 2;
        g.start_axis = GridAxis::Vertical;
        g.start_corner = GridCorner::LowerRight;
        let l = compute(&w, &none, |_| SCREEN);
        let r = l.rect(grid).unwrap();
        assert_eq!(r.h, 110.0);
        // 4 columns needed: 430 > 330 overflows to the right of the start offset.
        assert_eq!(l.rect(items[0]).unwrap().x, r.x + 3.0 * 110.0);
        assert_eq!(l.rect(items[0]).unwrap().y, r.y + 60.0);
        assert_eq!(l.rect(items[1]).unwrap().y, r.y);
    }

    #[test]
    fn nested_groups_and_legacy_children() {
        let mut w = world();
        let canvas = w.create("Canvas");
        w.add_component(canvas, Canvas::default());
        let panel = w.create("Panel");
        w.set_parent(panel, Some(canvas), false).unwrap();
        w.add_component(panel, RectTransform::stretch());
        w.add_component(
            panel,
            VerticalLayoutGroup {
                control_child_width: true,
                control_child_height: true,
                ..Default::default()
            },
        );
        // Two rows that share the height; the second holds a button and an image.
        let top = child(&mut w, panel, Vector2::ZERO);
        let bottom = child(&mut w, panel, Vector2::ZERO);
        w.add_component(
            bottom,
            HorizontalLayoutGroup {
                control_child_width: true,
                control_child_height: true,
                ..Default::default()
            },
        );
        let button = w.create("Button");
        w.set_parent(button, Some(bottom), false).unwrap();
        w.add_component(button, UIButton::default());
        let image = w.create("Image");
        w.set_parent(image, Some(bottom), false).unwrap();
        w.add_component(image, UIImage::default());
        let none = HashMap::new();
        let l = compute(&w, &none, |_| SCREEN);
        assert_eq!(l.rect(panel).unwrap(), SCREEN);
        assert_eq!(l.rect(top).unwrap(), UiRect::new(0.0, 0.0, 800.0, 300.0));
        assert_eq!(
            l.rect(bottom).unwrap(),
            UiRect::new(0.0, 300.0, 800.0, 300.0)
        );
        assert_eq!(
            l.rect(image).unwrap(),
            UiRect::new(400.0, 300.0, 400.0, 300.0)
        );
        // Legacy elements outside RectTransform trees keep their own placement.
        let lone = w.create("HUD");
        w.add_component(lone, UIText::default());
        assert!(compute(&w, &none, |_| SCREEN).rect(lone).is_none());
    }
}
