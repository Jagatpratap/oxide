//! Unity's Terrain Inspector tools: Create Neighbor Terrains, Paint Terrain (Raise or Lower,
//! Paint Holes, Paint Texture, Set Height, Smooth Height, Stamp Terrain), Paint Trees,
//! Paint Details and Terrain Settings, with brushes painted in the Scene view.
//!
//! The Inspector draws the panel with the World read-only, so data changes are queued as
//! [`Cmd`]s and applied once per frame ([`apply_commands`]). Brush strokes paint across
//! connected terrains (same Grouping ID), are one Undo step each, and hold the collider
//! until the stroke ends. Edited terrains are written to their `.terraindata` files when
//! the scene is saved.
//! Reference: <https://docs.unity3d.com/Manual/terrain-Tools.html>

use crate::EditorState;
use crate::view::Projector;
use egui::{Color32, RichText, Sense, Stroke, pos2, vec2};
use oxide_assets::{
    AssetServer, Brush, BrushShape, DetailPrototype, DetailRenderMode, Handle, HeightOp,
    TerrainData, TerrainLayer, TreePrototype, WindSettings,
};
use oxide_engine::Engine;
use oxide_engine::terrain::{Terrain, TerrainHit};
use oxide_math::Vector3;
use oxide_scene::{GameObjectId, World};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    None,
    Neighbors,
    Paint,
    Trees,
    Details,
    Settings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintMode {
    #[default]
    RaiseLower,
    Holes,
    Texture,
    SetHeight,
    Smooth,
    Stamp,
}

impl PaintMode {
    const ALL: [Self; 6] = [
        Self::RaiseLower,
        Self::Holes,
        Self::Texture,
        Self::SetHeight,
        Self::Smooth,
        Self::Stamp,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::RaiseLower => "Raise or Lower Terrain",
            Self::Holes => "Paint Holes",
            Self::Texture => "Paint Texture",
            Self::SetHeight => "Set Height",
            Self::Smooth => "Smooth Height",
            Self::Stamp => "Stamp Terrain",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::RaiseLower => "Left click to raise. Hold Shift and left click to lower.",
            Self::Holes => "Left click to paint a hole. Hold Shift and left click to erase it.",
            Self::Texture => "Select a Terrain Layer, then left click to paint it.",
            Self::SetHeight => {
                "Left click to set the height. Hold Shift and left click to sample the target height."
            }
            Self::Smooth => "Left click to smooth the terrain height.",
            Self::Stamp => "Left click to stamp the brush shape onto the terrain.",
        }
    }
}

/// A change to a terrain, applied with the World writable.
#[derive(Clone, Debug)]
pub enum Cmd {
    AddLayer(GameObjectId, TerrainLayer, Option<String>),
    RemoveLayer(GameObjectId, usize),
    EditLayer(GameObjectId, usize, TerrainLayer),
    AddTree(GameObjectId, String),
    RemoveTree(GameObjectId, usize),
    EditTree(GameObjectId, usize, f32),
    AddDetail(GameObjectId, DetailPrototype),
    RemoveDetail(GameObjectId, usize),
    EditDetail(GameObjectId, usize, DetailPrototype),
    MassPlace(GameObjectId, usize, bool),
    Flatten(GameObjectId, f32),
    Resize {
        id: GameObjectId,
        size: Vector3,
        heightmap: u32,
        control: u32,
        detail: u32,
    },
    Wind(GameObjectId, WindSettings),
    ImportRaw(GameObjectId, String),
    /// Loads a brush texture that wasn't loaded yet (then uses it).
    LoadBrushTexture(String),
    ExportRaw(GameObjectId, String),
    CreateNeighbor(GameObjectId, Vector3),
}

pub struct TerrainTools {
    pub tool: Tool,
    pub paint: PaintMode,
    pub brush: Brush,
    /// A brush texture (alpha or brightness) instead of the built-in shapes.
    pub brush_texture: Option<String>,
    pub set_height: f32,
    pub stamp_height: f32,
    pub stamp_add: bool,
    pub layer: usize,
    pub target_strength: f32,
    pub tree: usize,
    pub tree_density: f32,
    pub tree_height: (f32, f32),
    pub tree_width: (f32, f32),
    pub tree_color_variation: f32,
    pub tree_random_rotation: bool,
    pub mass_count: usize,
    pub mass_keep: bool,
    pub detail: usize,
    pub detail_target: f32,
    pub raw_path: String,
    pub commands: Vec<Cmd>,
    /// `.terraindata` files edited since the scene was saved.
    pub dirty: BTreeSet<String>,
    stroke: Option<Vec<Handle<TerrainData>>>,
    last_tree_paint: f64,
    last_frame: f64,
    rng: u32,
    /// The brush's last hit (Scene view preview and MCP).
    pub last_hit: Option<TerrainHit>,
}

impl Default for TerrainTools {
    fn default() -> Self {
        Self {
            tool: Tool::None,
            paint: PaintMode::RaiseLower,
            brush: Brush {
                size: 25.0,
                strength: 0.5,
                ..Default::default()
            },
            brush_texture: None,
            set_height: 0.0,
            stamp_height: 10.0,
            stamp_add: false,
            layer: 0,
            target_strength: 1.0,
            tree: 0,
            tree_density: 3.0,
            tree_height: (0.9, 1.2),
            tree_width: (0.9, 1.2),
            tree_color_variation: 0.1,
            tree_random_rotation: true,
            mass_count: 1000,
            mass_keep: true,
            detail: 0,
            detail_target: 8.0,
            raw_path: "Assets/Terrains/heightmap.raw".into(),
            commands: vec![],
            dirty: BTreeSet::new(),
            stroke: None,
            last_tree_paint: 0.0,
            last_frame: 0.0,
            rng: 0x9e37_79b9,
            last_hit: None,
        }
    }
}

fn terrain_data(world: &World, id: GameObjectId) -> Option<(&Terrain, &TerrainData)> {
    let t = world.get_component::<Terrain>(id)?;
    let d = world
        .resource::<AssetServer>()
        .terrains
        .get(t.data.as_ref()?)?;
    Some((t, d))
}

// ---- Inspector ---------------------------------------------------------------------------

/// The toolbar and the selected tool's settings, drawn under the Terrain component.
pub fn inspector_ui(ui: &mut egui::Ui, tt: &mut TerrainTools, world: &World, id: GameObjectId) {
    let Some((terrain, data)) = terrain_data(world, id) else {
        ui.weak("Assign a Terrain Data asset (GameObject → 3D Object → Terrain creates one).");
        return;
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for (tool, label, tip) in [
            (Tool::Neighbors, "Neighbors", "Create Neighbor Terrains"),
            (Tool::Paint, "Paint", "Paint Terrain"),
            (Tool::Trees, "Trees", "Paint Trees"),
            (Tool::Details, "Details", "Paint Details"),
            (Tool::Settings, "Settings", "Terrain Settings"),
        ] {
            let on = tt.tool == tool;
            if ui.selectable_label(on, label).on_hover_text(tip).clicked() {
                tt.tool = if on { Tool::None } else { tool };
            }
        }
    });
    ui.separator();
    match tt.tool {
        Tool::None => {
            ui.weak("Pick a tool above. Paint tools paint in the Scene view with the left mouse button.");
        }
        Tool::Neighbors => {
            ui.label(RichText::new("Create Neighbor Terrains").strong());
            ui.weak("Click an outlined square next to the terrain in the Scene view (or a button below) to add a tile of the same size there. Neighbours share the Grouping ID, so brushes paint across them.");
            let slots = oxide_engine::terrain::free_neighbour_slots(world, id);
            let o = world.position(id);
            ui.horizontal_wrapped(|ui| {
                for p in slots {
                    let d = p - o;
                    let name = if d.x < -0.1 {
                        "Left"
                    } else if d.x > 0.1 {
                        "Right"
                    } else if d.z < -0.1 {
                        "Back"
                    } else {
                        "Front"
                    };
                    if ui.button(format!("+ {name}")).clicked() {
                        tt.commands.push(Cmd::CreateNeighbor(id, p));
                    }
                }
            });
        }
        Tool::Paint => paint_ui(ui, tt, world, id, data),
        Tool::Trees => trees_ui(ui, tt, world, id, data),
        Tool::Details => details_ui(ui, tt, world, id, data),
        Tool::Settings => settings_ui(ui, tt, id, terrain, data),
    }
}

fn brush_ui(ui: &mut egui::Ui, tt: &mut TerrainTools, world: &World, max_size: f32) {
    ui.label(RichText::new("Brushes").strong());
    ui.horizontal_wrapped(|ui| {
        for shape in BrushShape::ALL {
            let b = Brush {
                shape,
                ..Default::default()
            };
            let (r, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
            let p = ui.painter_at(r);
            p.rect_filled(r, 3.0, Color32::from_gray(25));
            let n = 14;
            for j in 0..n {
                for i in 0..n {
                    let x = (i as f32 + 0.5) / n as f32 * 2.0 - 1.0;
                    let y = (j as f32 + 0.5) / n as f32 * 2.0 - 1.0;
                    let w = b.weight(x, y);
                    if w > 0.02 {
                        let cell = r.width() / n as f32;
                        p.rect_filled(
                            egui::Rect::from_min_size(
                                pos2(r.left() + i as f32 * cell, r.top() + j as f32 * cell),
                                vec2(cell, cell),
                            ),
                            0.0,
                            Color32::from_gray((w * 230.0) as u8),
                        );
                    }
                }
            }
            let sel = tt.brush.shape == shape && tt.brush_texture.is_none();
            if sel {
                p.rect_stroke(
                    r,
                    3.0,
                    Stroke::new(2.0, Color32::from_rgb(90, 170, 255)),
                    egui::StrokeKind::Inside,
                );
            }
            if resp.on_hover_text(format!("{shape:?}")).clicked() {
                tt.brush.shape = shape;
                tt.brush_texture = None;
                tt.brush.texture = None;
            }
        }
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Brush Texture");
        let assets = world.resource::<AssetServer>();
        match crate::anim_inspectors::asset_picker(
            ui,
            assets,
            "brush-tex",
            "Texture2D",
            tt.brush_texture.as_deref(),
            true,
        ) {
            Some(Some(p)) => {
                tt.brush.texture = assets
                    .textures
                    .handle_of(&p)
                    .and_then(|h| assets.textures.get(&h))
                    .filter(|t| !t.mips.is_empty())
                    .map(|t| std::sync::Arc::new((t.width, t.height, t.mips[0].clone())));
                if tt.brush.texture.is_none() {
                    tt.commands.push(Cmd::LoadBrushTexture(p.clone()));
                }
                tt.brush_texture = Some(p);
            }
            Some(None) => {
                tt.brush_texture = None;
                tt.brush.texture = None;
            }
            None => {}
        }
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Brush Size");
        ui.add(egui::Slider::new(&mut tt.brush.size, 0.5..=max_size.max(1.0)).logarithmic(true));
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Opacity");
        ui.add(egui::Slider::new(&mut tt.brush.strength, 0.0..=1.0));
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Rotation");
        ui.add(egui::Slider::new(&mut tt.brush.rotation, -180.0..=180.0).suffix("°"));
    });
}

fn paint_ui(
    ui: &mut egui::Ui,
    tt: &mut TerrainTools,
    world: &World,
    id: GameObjectId,
    data: &TerrainData,
) {
    egui::ComboBox::from_id_salt("terrain-paint-mode")
        .selected_text(tt.paint.label())
        .width(ui.available_width().min(260.0))
        .show_ui(ui, |ui| {
            for m in PaintMode::ALL {
                ui.selectable_value(&mut tt.paint, m, m.label());
            }
        });
    ui.weak(tt.paint.help());
    ui.add_space(4.0);
    brush_ui(ui, tt, world, data.size.x.max(data.size.z));
    match tt.paint {
        PaintMode::SetHeight => {
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Height");
                ui.add(
                    egui::DragValue::new(&mut tt.set_height)
                        .speed(0.1)
                        .range(0.0..=data.size.y)
                        .suffix(" m"),
                );
            });
            ui.horizontal(|ui| {
                if ui
                    .button("Flatten Tile")
                    .on_hover_text("Set the whole terrain to this height")
                    .clicked()
                {
                    tt.commands.push(Cmd::Flatten(id, tt.set_height));
                }
            });
        }
        PaintMode::Stamp => {
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Stamp Height");
                ui.add(
                    egui::DragValue::new(&mut tt.stamp_height)
                        .speed(0.1)
                        .suffix(" m"),
                );
            });
            ui.checkbox(
                &mut tt.stamp_add,
                "Add (instead of taking the higher of terrain and stamp)",
            );
        }
        PaintMode::Texture => layers_ui(ui, tt, world, id, data),
        _ => {}
    }
}

fn layers_ui(
    ui: &mut egui::Ui,
    tt: &mut TerrainTools,
    world: &World,
    id: GameObjectId,
    data: &TerrainData,
) {
    ui.label(RichText::new("Layers").strong());
    let assets = world.resource::<AssetServer>();
    if data.layers.is_empty() {
        ui.weak("No Terrain Layers yet: add one below.");
    }
    tt.layer = tt.layer.min(data.layers.len().saturating_sub(1));
    ui.horizontal_wrapped(|ui| {
        for (i, l) in data.layers.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(56.0, 70.0), Sense::click());
            let p = ui.painter_at(r);
            let img = egui::Rect::from_min_size(r.min, vec2(56.0, 56.0));
            let c = l
                .diffuse
                .as_ref()
                .and_then(|h| assets.textures.get(h))
                .and_then(|t| t.mips.first().map(|m| (m, t.width, t.height)))
                .map(|(m, w, h)| {
                    // Average colour of the texture as a swatch.
                    let step = ((w * h) as usize / 64).max(1) * 4;
                    let (mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32);
                    for px in m.chunks(4).step_by(step / 4) {
                        sr += px[0] as u32;
                        sg += px[1] as u32;
                        sb += px[2] as u32;
                        n += 1;
                    }
                    let n = n.max(1);
                    Color32::from_rgb((sr / n) as u8, (sg / n) as u8, (sb / n) as u8)
                })
                .unwrap_or(Color32::from_gray(140));
            p.rect_filled(img, 3.0, c);
            if tt.layer == i {
                p.rect_stroke(
                    img,
                    3.0,
                    Stroke::new(2.0, Color32::from_rgb(90, 170, 255)),
                    egui::StrokeKind::Inside,
                );
            }
            let name = if l.name.is_empty() {
                format!("Layer {i}")
            } else {
                l.name.clone()
            };
            p.text(
                pos2(r.center().x, r.bottom() - 6.0),
                egui::Align2::CENTER_CENTER,
                &name,
                egui::FontId::proportional(10.0),
                Color32::from_gray(210),
            );
            if resp.on_hover_text(&name).clicked() {
                tt.layer = i;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.menu_button("Add Layer", |ui| {
            let layers = assets.paths_of_type("TerrainLayer");
            if !layers.is_empty() {
                ui.label(RichText::new("Terrain Layer assets").weak());
            }
            for p in layers {
                if ui
                    .button(p.rsplit('/').next().unwrap_or(&p))
                    .on_hover_text(&p)
                    .clicked()
                {
                    let h: Handle<TerrainLayer> = Handle::from_path(p.clone());
                    let mut l = assets.data.get(&h).cloned().unwrap_or_default();
                    if l.name.is_empty() {
                        l.name = p
                            .rsplit('/')
                            .next()
                            .unwrap_or("")
                            .trim_end_matches(".terrainlayer")
                            .to_owned();
                    }
                    tt.commands.push(Cmd::AddLayer(id, l, Some(p)));
                    ui.close();
                }
            }
            ui.separator();
            ui.label(RichText::new("From a texture").weak());
            let mut textures = assets.paths_of_type("Texture2D");
            textures.extend(
                [
                    "builtin:terrain/grass",
                    "builtin:terrain/rock",
                    "builtin:terrain/sand",
                    "builtin:terrain/snow",
                    "builtin:terrain/dirt",
                ]
                .map(String::from),
            );
            for p in textures {
                if ui
                    .button(p.rsplit('/').next().unwrap_or(&p))
                    .on_hover_text(&p)
                    .clicked()
                {
                    let l = TerrainLayer {
                        name: p
                            .rsplit('/')
                            .next()
                            .unwrap_or("Layer")
                            .split('.')
                            .next()
                            .unwrap_or("Layer")
                            .to_owned(),
                        diffuse: Some(Handle::from_path(p.clone())),
                        ..Default::default()
                    };
                    tt.commands.push(Cmd::AddLayer(id, l, None));
                    ui.close();
                }
            }
        });
        if ui
            .add_enabled(!data.layers.is_empty(), egui::Button::new("Remove Layer"))
            .clicked()
        {
            tt.commands.push(Cmd::RemoveLayer(id, tt.layer));
        }
    });
    if let Some(l) = data.layers.get(tt.layer) {
        let mut e = l.clone();
        egui::CollapsingHeader::new(format!("Layer: {}", l.name))
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Name");
                    ui.text_edit_singleline(&mut e.name);
                });
                for (label, slot, salt) in [
                    ("Diffuse", &mut e.diffuse, "tl-diffuse"),
                    ("Normal Map", &mut e.normal_map, "tl-normal"),
                ] {
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, label);
                        if let Some(choice) = crate::anim_inspectors::asset_picker(
                            ui,
                            assets,
                            salt,
                            "Texture2D",
                            slot.as_ref().map(Handle::path),
                            true,
                        ) {
                            *slot = choice.map(Handle::from_path);
                        }
                    });
                }
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Normal Scale");
                    ui.add(egui::Slider::new(&mut e.normal_scale, 0.0..=4.0));
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Tile Size");
                    ui.add(
                        egui::DragValue::new(&mut e.tile_size.x)
                            .speed(0.1)
                            .range(0.1..=1000.0),
                    );
                    ui.add(
                        egui::DragValue::new(&mut e.tile_size.y)
                            .speed(0.1)
                            .range(0.1..=1000.0),
                    );
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Tile Offset");
                    ui.add(egui::DragValue::new(&mut e.tile_offset.x).speed(0.1));
                    ui.add(egui::DragValue::new(&mut e.tile_offset.y).speed(0.1));
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Smoothness");
                    ui.add(egui::Slider::new(&mut e.smoothness, 0.0..=1.0));
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Metallic");
                    ui.add(egui::Slider::new(&mut e.metallic, 0.0..=1.0));
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Tint");
                    let mut c = [e.tint.r, e.tint.g, e.tint.b];
                    if ui.color_edit_button_rgb(&mut c).changed() {
                        e.tint = oxide_math::Color::rgb(c[0], c[1], c[2]);
                    }
                });
            });
        if e != *l {
            tt.commands.push(Cmd::EditLayer(id, tt.layer, e));
        }
    }
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Target Strength");
        ui.add(egui::Slider::new(&mut tt.target_strength, 0.0..=1.0));
    });
}

fn trees_ui(
    ui: &mut egui::Ui,
    tt: &mut TerrainTools,
    world: &World,
    id: GameObjectId,
    data: &TerrainData,
) {
    ui.weak("Left click to place trees. Hold Shift to erase trees, Shift+Ctrl to erase only the selected tree type.");
    ui.label(RichText::new("Trees").strong());
    tt.tree = tt.tree.min(data.tree_prototypes.len().saturating_sub(1));
    for (i, p) in data.tree_prototypes.iter().enumerate() {
        if ui.selectable_label(tt.tree == i, p.name()).clicked() {
            tt.tree = i;
        }
    }
    if data.tree_prototypes.is_empty() {
        ui.weak("No tree types yet: Add Tree picks a model or prefab.");
    }
    let assets = world.resource::<AssetServer>();
    ui.horizontal(|ui| {
        ui.menu_button("Add Tree", |ui| {
            if ui.button("Built-in Tree").clicked() {
                tt.commands
                    .push(Cmd::AddTree(id, "builtin:tree/simple".into()));
                ui.close();
            }
            let mut sources = assets.paths_of_type("Model");
            sources.extend(assets.paths_of_type("Prefab"));
            for p in sources {
                if ui
                    .button(p.rsplit('/').next().unwrap_or(&p))
                    .on_hover_text(&p)
                    .clicked()
                {
                    tt.commands.push(Cmd::AddTree(id, p));
                    ui.close();
                }
            }
        });
        if ui
            .add_enabled(
                !data.tree_prototypes.is_empty(),
                egui::Button::new("Remove Tree"),
            )
            .clicked()
        {
            tt.commands.push(Cmd::RemoveTree(id, tt.tree));
        }
    });
    if let Some(p) = data.tree_prototypes.get(tt.tree) {
        let mut bend = p.bend_factor;
        ui.horizontal(|ui| {
            crate::inspector::prop_label(ui, "Bend Factor");
            if ui.add(egui::Slider::new(&mut bend, 0.0..=2.0)).changed() {
                tt.commands.push(Cmd::EditTree(id, tt.tree, bend));
            }
        });
    }
    ui.add_space(4.0);
    brush_ui(ui, tt, world, data.size.x.max(data.size.z));
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Tree Density");
        ui.add(egui::Slider::new(&mut tt.tree_density, 0.1..=20.0).suffix(" /100m²"));
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Tree Height");
        ui.add(
            egui::DragValue::new(&mut tt.tree_height.0)
                .speed(0.01)
                .range(0.1..=5.0),
        );
        ui.add(
            egui::DragValue::new(&mut tt.tree_height.1)
                .speed(0.01)
                .range(0.1..=5.0),
        );
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Tree Width");
        ui.add(
            egui::DragValue::new(&mut tt.tree_width.0)
                .speed(0.01)
                .range(0.1..=5.0),
        );
        ui.add(
            egui::DragValue::new(&mut tt.tree_width.1)
                .speed(0.01)
                .range(0.1..=5.0),
        );
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Color Variation");
        ui.add(egui::Slider::new(&mut tt.tree_color_variation, 0.0..=1.0));
    });
    ui.checkbox(&mut tt.tree_random_rotation, "Random Tree Rotation");
    ui.separator();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !data.tree_prototypes.is_empty(),
                egui::Button::new("Mass Place Trees"),
            )
            .clicked()
        {
            tt.commands
                .push(Cmd::MassPlace(id, tt.mass_count, tt.mass_keep));
        }
        ui.add(egui::DragValue::new(&mut tt.mass_count).range(1..=1_000_000));
        ui.checkbox(&mut tt.mass_keep, "Keep existing");
    });
    ui.weak(format!("{} trees on this terrain.", data.trees.len()));
}

fn details_ui(
    ui: &mut egui::Ui,
    tt: &mut TerrainTools,
    world: &World,
    id: GameObjectId,
    data: &TerrainData,
) {
    ui.weak("Left click to paint grass and details. Hold Shift to erase.");
    ui.label(RichText::new("Details").strong());
    tt.detail = tt
        .detail
        .min(data.detail_prototypes.len().saturating_sub(1));
    for (i, p) in data.detail_prototypes.iter().enumerate() {
        if ui.selectable_label(tt.detail == i, p.name()).clicked() {
            tt.detail = i;
        }
    }
    let assets = world.resource::<AssetServer>();
    ui.horizontal(|ui| {
        ui.menu_button("Add Grass Texture", |ui| {
            if ui.button("Built-in Grass").clicked() {
                tt.commands.push(Cmd::AddDetail(
                    id,
                    DetailPrototype {
                        texture: Some(Handle::from_path("builtin:terrain/grass_blades")),
                        ..Default::default()
                    },
                ));
                ui.close();
            }
            for p in assets.paths_of_type("Texture2D") {
                if ui
                    .button(p.rsplit('/').next().unwrap_or(&p))
                    .on_hover_text(&p)
                    .clicked()
                {
                    tt.commands.push(Cmd::AddDetail(
                        id,
                        DetailPrototype {
                            texture: Some(Handle::from_path(p.clone())),
                            ..Default::default()
                        },
                    ));
                    ui.close();
                }
            }
        });
        ui.menu_button("Add Detail Mesh", |ui| {
            let mut sources = assets.paths_of_type("Model");
            sources.extend(assets.paths_of_type("Prefab"));
            if sources.is_empty() {
                ui.weak("No models in the project.");
            }
            for p in sources {
                if ui
                    .button(p.rsplit('/').next().unwrap_or(&p))
                    .on_hover_text(&p)
                    .clicked()
                {
                    tt.commands.push(Cmd::AddDetail(
                        id,
                        DetailPrototype {
                            source: Some(p.clone()),
                            render_mode: DetailRenderMode::VertexLit,
                            min_width: 0.8,
                            max_width: 1.2,
                            min_height: 0.8,
                            max_height: 1.2,
                            healthy_color: oxide_math::Color::WHITE,
                            dry_color: oxide_math::Color::WHITE,
                            ..Default::default()
                        },
                    ));
                    ui.close();
                }
            }
        });
        if ui
            .add_enabled(
                !data.detail_prototypes.is_empty(),
                egui::Button::new("Remove"),
            )
            .clicked()
        {
            tt.commands.push(Cmd::RemoveDetail(id, tt.detail));
        }
    });
    if let Some(p) = data.detail_prototypes.get(tt.detail) {
        let mut e = p.clone();
        egui::CollapsingHeader::new("Edit Detail")
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Render Mode");
                    egui::ComboBox::from_id_salt("detail-mode")
                        .selected_text(format!("{:?}", e.render_mode))
                        .show_ui(ui, |ui| {
                            for m in [
                                DetailRenderMode::Grass,
                                DetailRenderMode::GrassBillboard,
                                DetailRenderMode::VertexLit,
                            ] {
                                ui.selectable_value(&mut e.render_mode, m, format!("{m:?}"));
                            }
                        });
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Width (min, max)");
                    ui.add(
                        egui::DragValue::new(&mut e.min_width)
                            .speed(0.01)
                            .range(0.01..=50.0),
                    );
                    ui.add(
                        egui::DragValue::new(&mut e.max_width)
                            .speed(0.01)
                            .range(0.01..=50.0),
                    );
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Height (min, max)");
                    ui.add(
                        egui::DragValue::new(&mut e.min_height)
                            .speed(0.01)
                            .range(0.01..=50.0),
                    );
                    ui.add(
                        egui::DragValue::new(&mut e.max_height)
                            .speed(0.01)
                            .range(0.01..=50.0),
                    );
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Noise Spread");
                    ui.add(
                        egui::DragValue::new(&mut e.noise_spread)
                            .speed(0.1)
                            .range(0.1..=200.0),
                    );
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Density");
                    ui.add(egui::Slider::new(&mut e.density, 0.0..=5.0));
                });
                for (label, c) in [
                    ("Healthy Color", &mut e.healthy_color),
                    ("Dry Color", &mut e.dry_color),
                ] {
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, label);
                        let mut v = [c.r, c.g, c.b];
                        if ui.color_edit_button_rgb(&mut v).changed() {
                            *c = oxide_math::Color::rgb(v[0], v[1], v[2]);
                        }
                    });
                }
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Align To Ground");
                    ui.add(egui::Slider::new(&mut e.align_to_ground, 0.0..=1.0));
                });
            });
        if e != *p {
            tt.commands.push(Cmd::EditDetail(id, tt.detail, e));
        }
    }
    ui.add_space(4.0);
    brush_ui(ui, tt, world, data.size.x.max(data.size.z));
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Target Strength");
        ui.add(egui::Slider::new(&mut tt.detail_target, 0.0..=16.0));
    });
}

fn settings_ui(
    ui: &mut egui::Ui,
    tt: &mut TerrainTools,
    id: GameObjectId,
    _t: &Terrain,
    d: &TerrainData,
) {
    ui.weak("Basic, tree, detail and draw settings are the Terrain component's fields above.");
    ui.label(RichText::new("Wind Settings for Grass").strong());
    let mut wind = d.wind;
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Speed");
        ui.add(egui::Slider::new(&mut wind.speed, 0.0..=1.0));
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Size");
        ui.add(egui::Slider::new(&mut wind.size, 0.0..=1.0));
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Bending");
        ui.add(egui::Slider::new(&mut wind.bending, 0.0..=1.0));
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Grass Tint");
        let mut c = [wind.grass_tint.r, wind.grass_tint.g, wind.grass_tint.b];
        if ui.color_edit_button_rgb(&mut c).changed() {
            wind.grass_tint = oxide_math::Color::rgb(c[0], c[1], c[2]);
        }
    });
    if wind != d.wind {
        tt.commands.push(Cmd::Wind(id, wind));
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Mesh Resolution (on Terrain Data)").strong());
    let mut size = d.size;
    let mut heightmap = d.resolution;
    let mut control = d.splat_resolution;
    let mut detail = d.detail_resolution;
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Terrain Width");
        ui.add(
            egui::DragValue::new(&mut size.x)
                .speed(1.0)
                .range(1.0..=100_000.0),
        );
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Terrain Length");
        ui.add(
            egui::DragValue::new(&mut size.z)
                .speed(1.0)
                .range(1.0..=100_000.0),
        );
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Terrain Height");
        ui.add(
            egui::DragValue::new(&mut size.y)
                .speed(1.0)
                .range(1.0..=10_000.0),
        );
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Detail Resolution");
        egui::ComboBox::from_id_salt("terrain-detail-res")
            .selected_text(detail.to_string())
            .show_ui(ui, |ui| {
                for r in [128, 256, 512, 1024, 2048, 4096] {
                    ui.selectable_value(&mut detail, r, r.to_string());
                }
            });
    });
    ui.label(RichText::new("Texture Resolutions (on Terrain Data)").strong());
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Heightmap Resolution");
        egui::ComboBox::from_id_salt("terrain-hm-res")
            .selected_text(heightmap.to_string())
            .show_ui(ui, |ui| {
                for r in [33, 65, 129, 257, 513, 1025, 2049, 4097] {
                    ui.selectable_value(&mut heightmap, r, r.to_string());
                }
            });
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Control Texture Resolution");
        egui::ComboBox::from_id_salt("terrain-ctl-res")
            .selected_text(control.to_string())
            .show_ui(ui, |ui| {
                for r in [16, 32, 64, 128, 256, 512, 1024, 2048, 4096] {
                    ui.selectable_value(&mut control, r, r.to_string());
                }
            });
    });
    if size != d.size
        || heightmap != d.resolution
        || control != d.splat_resolution
        || detail != d.detail_resolution
    {
        tt.commands.push(Cmd::Resize {
            id,
            size,
            heightmap,
            control,
            detail,
        });
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Heightmap Import / Export (16-bit RAW, little-endian)").strong());
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut tt.raw_path);
    });
    ui.horizontal(|ui| {
        if ui.button("Import Raw...").clicked() {
            tt.commands.push(Cmd::ImportRaw(id, tt.raw_path.clone()));
        }
        if ui.button("Export Raw...").clicked() {
            tt.commands.push(Cmd::ExportRaw(id, tt.raw_path.clone()));
        }
    });
    ui.weak(format!(
        "{}×{} heights, {} layers, {} trees, {} detail types{}.",
        d.resolution,
        d.resolution,
        d.layers.len(),
        d.trees.len(),
        d.detail_prototypes.len(),
        if d.holes.is_empty() {
            ""
        } else {
            ", has holes"
        }
    ));
}

// ---- applying commands --------------------------------------------------------------------

/// Applies the queued commands (each one Undo step) and marks the terrains unsaved.
pub fn apply_commands(st: &mut EditorState, engine: &mut Engine) {
    let cmds = std::mem::take(&mut st.terrain.commands);
    for cmd in cmds {
        if let Cmd::LoadBrushTexture(p) = &cmd {
            let assets = engine.world.resource_mut::<AssetServer>();
            if assets.ensure_loaded(p).is_ok() {
                st.terrain.brush.texture = assets
                    .textures
                    .handle_of(p)
                    .and_then(|h| assets.textures.get(&h))
                    .filter(|t| !t.mips.is_empty())
                    .map(|t| std::sync::Arc::new((t.width, t.height, t.mips[0].clone())));
            }
            continue;
        }
        if let Err(e) = apply(st, engine, cmd) {
            st.status = e.clone();
            oxide_core::debug_log_warning!("{e}");
        }
    }
}

fn data_handle(world: &World, id: GameObjectId) -> Result<Handle<TerrainData>, String> {
    world
        .get_component::<Terrain>(id)
        .and_then(|t| t.data.clone())
        .ok_or_else(|| "the object has no Terrain Data".into())
}

fn apply(st: &mut EditorState, engine: &mut Engine, cmd: Cmd) -> Result<(), String> {
    let now = oxide_core::time::realtime_since_startup();
    let id = match &cmd {
        Cmd::AddLayer(id, ..)
        | Cmd::RemoveLayer(id, _)
        | Cmd::EditLayer(id, ..)
        | Cmd::AddTree(id, _)
        | Cmd::RemoveTree(id, _)
        | Cmd::EditTree(id, ..)
        | Cmd::AddDetail(id, _)
        | Cmd::RemoveDetail(id, _)
        | Cmd::EditDetail(id, ..)
        | Cmd::MassPlace(id, ..)
        | Cmd::Flatten(id, _)
        | Cmd::Wind(id, _)
        | Cmd::ImportRaw(id, _)
        | Cmd::ExportRaw(id, _)
        | Cmd::CreateNeighbor(id, _) => *id,
        Cmd::Resize { id, .. } => *id,
        Cmd::LoadBrushTexture(_) => return Ok(()),
    };
    let h = data_handle(&engine.world, id)?;
    let label = match &cmd {
        Cmd::AddLayer(..) => "Add Terrain Layer",
        Cmd::RemoveLayer(..) => "Remove Terrain Layer",
        Cmd::EditLayer(..) => "Edit Terrain Layer",
        Cmd::AddTree(..) => "Add Tree",
        Cmd::RemoveTree(..) => "Remove Tree",
        Cmd::EditTree(..) => "Edit Tree",
        Cmd::AddDetail(..) => "Add Detail",
        Cmd::RemoveDetail(..) => "Remove Detail",
        Cmd::EditDetail(..) => "Edit Detail",
        Cmd::MassPlace(..) => "Mass Place Trees",
        Cmd::Flatten(..) => "Flatten Terrain",
        Cmd::Resize { .. } => "Terrain Resolution",
        Cmd::Wind(..) => "Terrain Wind",
        Cmd::ImportRaw(..) => "Import Heightmap",
        Cmd::ExportRaw(..) => "",
        Cmd::CreateNeighbor(..) => "Create Neighbor Terrain",
        Cmd::LoadBrushTexture(_) => "",
    };
    // Field tweaks (layer, detail, wind edits) merge into one step while dragging.
    let coalesce = matches!(
        cmd,
        Cmd::EditLayer(..) | Cmd::EditDetail(..) | Cmd::Wind(..) | Cmd::EditTree(..)
    );
    if !label.is_empty() && !(coalesce && st.terrain_edit_merge(now)) {
        st.undo.record_terrain(
            &engine.world,
            label,
            &st.selection,
            std::slice::from_ref(&h),
        );
        st.dirty = true;
    }
    // Model / prefab trees and detail meshes: their first mesh and materials.
    let resolve = |engine: &mut Engine,
                   src: &str|
     -> Result<
        (
            Handle<oxide_assets::Mesh>,
            Vec<Handle<oxide_assets::Material>>,
        ),
        String,
    > { resolve_mesh_source(engine, src) };
    match cmd {
        Cmd::AddTree(_, src) => {
            let (mesh, mats) = resolve(engine, &src)?;
            let d = terrain_mut(engine, &h)?;
            let mut p = TreePrototype::new(mesh, mats);
            p.source = Some(src);
            d.tree_prototypes.push(p);
            d.touch_trees();
        }
        Cmd::AddDetail(_, mut p) => {
            if let Some(src) = p.source.clone() {
                let (mesh, mats) = resolve(engine, &src)?;
                p.mesh = Some(mesh);
                p.materials = mats;
            }
            if let Some(t) = &p.texture
                && !t.path().starts_with("builtin:")
            {
                let _ = engine
                    .world
                    .resource_mut::<AssetServer>()
                    .ensure_loaded(t.path());
            }
            let d = terrain_mut(engine, &h)?;
            d.detail_prototypes.push(p);
            d.fit_details();
            d.touch_details();
        }
        Cmd::CreateNeighbor(src, origin) => {
            let (size, res, layers, assets_paths, protos, group, path) = {
                let (t, d) = terrain_data(&engine.world, src).ok_or("no terrain")?;
                (
                    d.size,
                    d.resolution,
                    d.layers.clone(),
                    d.layer_assets.clone(),
                    d.tree_prototypes.clone(),
                    t.grouping_id,
                    t.data
                        .as_ref()
                        .map(|h| h.path().to_owned())
                        .unwrap_or_default(),
                )
            };
            let dir = path
                .rsplit_once('/')
                .map_or("Assets", |(d, _)| d)
                .to_owned();
            let assets = engine.world.resource::<AssetServer>();
            let file = (0..)
                .map(|i| format!("{dir}/Terrain {}.terraindata", i + 1))
                .find(|p| !assets.full_path(p).exists() && assets.terrains.handle_of(p).is_none())
                .expect("free name");
            let name = file
                .rsplit('/')
                .next()
                .unwrap_or("Terrain")
                .trim_end_matches(".terraindata")
                .to_owned();
            let nid = oxide_engine::terrain::create_terrain_object(
                &mut engine.world,
                &name,
                size,
                res,
                Some(&file),
            )?;
            engine.world.set_position(nid, origin);
            if let Some(t) = engine.world.get_component_mut::<Terrain>(nid) {
                t.grouping_id = group;
            }
            let nh = data_handle(&engine.world, nid)?;
            let d = terrain_mut(engine, &nh)?;
            d.layers = layers;
            d.layer_assets = assets_paths;
            d.tree_prototypes = protos;
            d.fit_alphamaps();
            d.touch_layers();
            // Match the shared edge heights so the seam is closed.
            stitch_edges(engine, src, nid);
            st.terrain.dirty.insert(file);
            st.select(nid);
        }
        Cmd::ExportRaw(_, path) => {
            let d = engine
                .world
                .resource::<AssetServer>()
                .terrains
                .get(&h)
                .ok_or("no data")?;
            let bytes: Vec<u8> = d
                .heights
                .iter()
                .flat_map(|v| ((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes())
                .collect();
            let full = engine.world.resource::<AssetServer>().full_path(&path);
            if let Some(dir) = full.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            std::fs::write(&full, bytes)
                .map_err(|e| format!("cannot write {}: {e}", full.display()))?;
            st.status = format!(
                "Exported {}×{} heightmap to {path}",
                d.resolution, d.resolution
            );
        }
        Cmd::ImportRaw(_, path) => {
            let full = engine.world.resource::<AssetServer>().full_path(&path);
            let bytes =
                std::fs::read(&full).map_err(|e| format!("cannot read {}: {e}", full.display()))?;
            let n = ((bytes.len() / 2) as f64).sqrt() as u32;
            if n < 2 || (n * n * 2) as usize != bytes.len() {
                return Err(format!("{path}: not a square 16-bit RAW heightmap"));
            }
            let heights: Vec<f32> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes(*b) as f32 / 65535.0)
                .collect();
            let d = terrain_mut(engine, &h)?;
            if d.resolution != n {
                d.resolution = n;
                if !d.holes.is_empty() {
                    d.holes = std::sync::Arc::new(Vec::new());
                    d.touch_holes();
                }
            }
            d.set_heights(heights);
            d.snap_trees_to_ground();
        }
        other => {
            let d = terrain_mut(engine, &h)?;
            match other {
                Cmd::AddLayer(_, l, asset) => {
                    d.add_layer(l, asset);
                }
                Cmd::RemoveLayer(_, i) => d.remove_layer(i),
                Cmd::EditLayer(_, i, l) => {
                    if let Some(slot) = d.layers.get_mut(i) {
                        *slot = l;
                        // An edited layer is this terrain's own copy now.
                        if let Some(a) = d.layer_assets.get_mut(i) {
                            *a = None;
                        }
                        d.touch_layers();
                    }
                }
                Cmd::RemoveTree(_, i) => {
                    if i < d.tree_prototypes.len() {
                        d.tree_prototypes.remove(i);
                        d.trees.retain(|t| t.prototype as usize != i);
                        for t in &mut d.trees {
                            if t.prototype as usize > i {
                                t.prototype -= 1;
                            }
                        }
                        d.touch_trees();
                    }
                }
                Cmd::EditTree(_, i, bend) => {
                    if let Some(p) = d.tree_prototypes.get_mut(i) {
                        p.bend_factor = bend;
                        d.touch_trees();
                    }
                }
                Cmd::RemoveDetail(_, i) => {
                    if i < d.detail_prototypes.len() {
                        d.detail_prototypes.remove(i);
                        if i < d.details.len() {
                            d.details.remove(i);
                        }
                        d.touch_details();
                    }
                }
                Cmd::EditDetail(_, i, p) => {
                    if let Some(slot) = d.detail_prototypes.get_mut(i) {
                        *slot = p;
                        d.touch_details();
                    }
                }
                Cmd::MassPlace(_, count, keep) => {
                    d.mass_place_trees(count, now.to_bits() as u32, keep)
                }
                Cmd::Flatten(_, height) => {
                    let n = d.resolution as usize;
                    d.set_heights(vec![(height / d.size.y.max(1e-4)).clamp(0.0, 1.0); n * n]);
                    d.snap_trees_to_ground();
                }
                Cmd::Resize {
                    size,
                    heightmap,
                    control,
                    detail,
                    ..
                } => {
                    if size != d.size {
                        // Keep world heights when the height range changes.
                        let k = d.size.y / size.y.max(1e-4);
                        let scaled: Vec<f32> =
                            d.heights.iter().map(|h| (h * k).clamp(0.0, 1.0)).collect();
                        d.size = size;
                        d.set_heights(scaled);
                    }
                    d.resize_heightmap(heightmap);
                    d.resize_alphamaps(control);
                    if detail != d.detail_resolution {
                        d.detail_resolution = detail;
                        d.details.clear();
                        d.fit_details();
                        d.touch_details();
                    }
                    d.snap_trees_to_ground();
                }
                Cmd::Wind(_, w) => {
                    d.wind = w;
                    d.touch_details();
                }
                _ => {}
            }
        }
    }
    st.terrain.dirty.insert(h.path().to_owned());
    Ok(())
}

fn terrain_mut<'a>(
    engine: &'a mut Engine,
    h: &Handle<TerrainData>,
) -> Result<&'a mut TerrainData, String> {
    engine
        .world
        .resource_mut::<AssetServer>()
        .terrains
        .get_mut(h)
        .ok_or_else(|| "terrain data is not loaded".into())
}

fn resolve_mesh_source(
    engine: &mut Engine,
    src: &str,
) -> Result<
    (
        Handle<oxide_assets::Mesh>,
        Vec<Handle<oxide_assets::Material>>,
    ),
    String,
> {
    oxide_engine::terrain::resolve_mesh_source(&mut engine.world, src)
}

/// Copies the shared edge of `src` onto the new neighbour `dst` (they then match).
fn stitch_edges(engine: &mut Engine, src: GameObjectId, dst: GameObjectId) {
    let (Ok(hs), Ok(hd)) = (
        data_handle(&engine.world, src),
        data_handle(&engine.world, dst),
    ) else {
        return;
    };
    let (os, od) = (engine.world.position(src), engine.world.position(dst));
    let assets = engine.world.resource_mut::<AssetServer>();
    let Some(s) = assets.terrains.get(&hs).cloned() else {
        return;
    };
    let Some(d) = assets.terrains.get_mut(&hd) else {
        return;
    };
    let n = d.resolution;
    let mut heights = (*d.heights).clone();
    for i in 0..n {
        let f = i as f32 / (n - 1) as f32;
        let (dx, dz) = (od.x - os.x, od.z - os.z);
        // Edge of dst touching src, and the matching normalized point on src.
        let (x, z, u, v) = if dx > 0.1 {
            (0, i, 1.0, f)
        } else if dx < -0.1 {
            (n - 1, i, 0.0, f)
        } else if dz > 0.1 {
            (i, 0, f, 1.0)
        } else {
            (i, n - 1, f, 0.0)
        };
        let world_h = s.interpolated_height(u, v) + os.y - od.y;
        heights[(z * n + x) as usize] = (world_h / d.size.y.max(1e-4)).clamp(0.0, 1.0);
    }
    d.set_heights(heights);
}

impl EditorState {
    /// Whether a terrain field edit continues the last one (half a second).
    fn terrain_edit_merge(&mut self, now: f64) -> bool {
        let merge = now - self.terrain.last_frame < 0.5 && self.terrain.stroke.is_none();
        self.terrain.last_frame = now;
        merge
    }
}

// ---- Scene view painting -------------------------------------------------------------------

/// The selected terrain while a paint tool is active.
fn active_terrain(st: &EditorState, world: &World) -> Option<GameObjectId> {
    if !matches!(
        st.terrain.tool,
        Tool::Paint | Tool::Trees | Tool::Details | Tool::Neighbors
    ) {
        return None;
    }
    let id = *st.selection.last()?;
    world.get_component::<Terrain>(id)?;
    Some(id)
}

/// Brush preview and painting in the Scene view. Returns true when the terrain tool
/// handled the pointer (the Scene view then doesn't select or move objects).
pub fn scene_paint(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &egui::Ui,
    resp: &egui::Response,
    proj: &Projector,
    painter: &egui::Painter,
) -> bool {
    let Some(id) = active_terrain(st, &engine.world) else {
        st.terrain.stroke = None;
        return false;
    };
    if st.terrain.tool == Tool::Neighbors {
        return neighbour_slots(st, engine, resp, proj, painter, id);
    }
    // Camera navigation (right / middle drag, Alt) keeps working.
    let navigating =
        ui.input(|i| i.pointer.secondary_down() || i.pointer.middle_down() || i.modifiers.alt);
    let pointer = resp.hover_pos().or(resp.interact_pointer_pos());
    let hit = pointer
        .and_then(|p| oxide_engine::terrain::raycast(&engine.world, &proj.ray(p), 100_000.0));
    st.terrain.last_hit = hit;
    let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command || i.modifiers.ctrl));
    if let Some(h) = hit {
        draw_brush(&engine.world, proj, painter, &st.terrain, h, shift);
    }
    let down = ui.input(|i| i.pointer.primary_down()) && resp.hovered()
        || resp.dragged_by(egui::PointerButton::Primary);
    if !down || navigating {
        if let Some(handles) = st.terrain.stroke.take() {
            end_stroke(engine, &handles);
        }
        return hit.is_some() && !navigating;
    }
    let Some(h) = hit else {
        return true;
    };
    let now = oxide_core::time::realtime_since_startup();
    let dt = (now - st.terrain.last_frame).clamp(0.0, 0.1) as f32;
    st.terrain.last_frame = now;
    // Set Height: Shift+click samples the height under the cursor.
    if st.terrain.tool == Tool::Paint && st.terrain.paint == PaintMode::SetHeight && shift {
        st.terrain.set_height = h.point.y - engine.world.position(h.terrain).y;
        return true;
    }
    if st.terrain.stroke.is_none() {
        let handles: Vec<Handle<TerrainData>> =
            oxide_engine::terrain::connected(&engine.world, h.terrain)
                .into_iter()
                .map(|(_, d, _)| d)
                .collect();
        let label = match st.terrain.tool {
            Tool::Trees => "Paint Trees",
            Tool::Details => "Paint Details",
            _ => st.terrain.paint.label(),
        };
        st.undo
            .record_terrain(&engine.world, label, &st.selection, &handles);
        st.dirty = true;
        st.terrain.stroke = Some(handles);
        st.terrain.last_tree_paint = 0.0;
    }
    paint(st, engine, h, dt.max(1.0 / 120.0), shift, ctrl, now);
    let _ = id;
    true
}

/// One frame of a stroke, on every connected terrain under the brush.
fn paint(
    st: &mut EditorState,
    engine: &mut Engine,
    hit: TerrainHit,
    dt: f32,
    shift: bool,
    ctrl: bool,
    now: f64,
) {
    let tt = &mut st.terrain;
    let terrains = oxide_engine::terrain::connected(&engine.world, hit.terrain);
    let set_height_world = engine.world.position(hit.terrain).y + tt.set_height;
    let assets = engine.world.resource_mut::<AssetServer>();
    let tree_tick = tt.tool == Tool::Trees && now - tt.last_tree_paint > 0.06;
    if tree_tick {
        tt.last_tree_paint = now;
    }
    for (_, h, origin) in terrains {
        let Some(d) = assets.terrains.get_mut(&h) else {
            continue;
        };
        let u = (hit.point.x - origin.x) / d.size.x.max(1e-4);
        let v = (hit.point.z - origin.z) / d.size.z.max(1e-4);
        // Skip terrains the brush doesn't reach.
        let r = tt.brush.size * 0.5;
        if u * d.size.x < -r
            || v * d.size.z < -r
            || (u - 1.0) * d.size.x > r
            || (v - 1.0) * d.size.z > r
        {
            continue;
        }
        let mut brush = tt.brush.clone();
        d.hold_collider = true;
        match tt.tool {
            Tool::Paint => match tt.paint {
                PaintMode::RaiseLower => {
                    // About a quarter of the terrain height per second at full opacity.
                    let m = d.size.y * 0.25 * dt * if shift { -1.0 } else { 1.0 };
                    d.paint_height(u, v, &brush, HeightOp::Raise(m));
                }
                PaintMode::SetHeight => {
                    brush.strength = (brush.strength * dt * 8.0).min(1.0);
                    // The target is relative to the clicked terrain; neighbours at other
                    // heights get the same world height.
                    d.paint_height(
                        u,
                        v,
                        &brush,
                        HeightOp::SetHeight(set_height_world - origin.y),
                    );
                }
                PaintMode::Smooth => {
                    brush.strength = (brush.strength * dt * 12.0).min(1.0);
                    d.paint_height(u, v, &brush, HeightOp::Smooth);
                }
                PaintMode::Stamp => {
                    d.paint_height(
                        u,
                        v,
                        &brush,
                        HeightOp::Stamp {
                            height: tt.stamp_height,
                            add: tt.stamp_add,
                        },
                    );
                }
                PaintMode::Texture => {
                    brush.strength = (brush.strength * dt * 6.0).min(1.0);
                    d.paint_texture(u, v, &brush, tt.layer, tt.target_strength);
                }
                PaintMode::Holes => d.paint_holes(u, v, &brush, shift),
            },
            Tool::Trees => {
                if tree_tick {
                    let proto = if shift && !ctrl {
                        u32::MAX
                    } else {
                        tt.tree as u32
                    };
                    if !d.tree_prototypes.is_empty() || shift {
                        d.paint_trees(
                            u,
                            v,
                            &brush,
                            proto,
                            tt.tree_density,
                            tt.tree_height,
                            tt.tree_width,
                            tt.tree_color_variation,
                            tt.tree_random_rotation,
                            shift,
                            &mut tt.rng,
                        );
                    }
                }
            }
            Tool::Details => {
                brush.strength = (brush.strength * dt * 10.0).min(1.0);
                d.paint_details(u, v, &brush, tt.detail, tt.detail_target, shift);
            }
            _ => {}
        }
        tt.dirty.insert(h.path().to_owned());
    }
}

fn end_stroke(engine: &mut Engine, handles: &[Handle<TerrainData>]) {
    let assets = engine.world.resource_mut::<AssetServer>();
    for h in handles {
        if let Some(d) = assets.terrains.get_mut(h) {
            d.hold_collider = false;
            d.snap_trees_to_ground();
            d.touch_trees();
        }
    }
}

fn draw_brush(
    world: &World,
    proj: &Projector,
    painter: &egui::Painter,
    tt: &TerrainTools,
    hit: TerrainHit,
    shift: bool,
) {
    let assets = world.resource::<AssetServer>();
    let Some((_, d)) = terrain_data(world, hit.terrain) else {
        return;
    };
    let o = world.position(hit.terrain);
    let color = if shift {
        Color32::from_rgb(240, 110, 90)
    } else {
        Color32::from_rgb(90, 180, 255)
    };
    let ring = |radius: f32, c: Color32, width: f32| {
        let mut pts = vec![];
        for k in 0..=64 {
            let a = k as f32 / 64.0 * std::f32::consts::TAU;
            let x = hit.point.x + a.cos() * radius;
            let z = hit.point.z + a.sin() * radius;
            let (u, v) = ((x - o.x) / d.size.x, (z - o.z) / d.size.z);
            let y = o.y + d.interpolated_height(u, v) + 0.05;
            if let Some(p) = proj.project(Vector3::new(x, y, z)) {
                pts.push(p);
            }
        }
        painter.add(egui::Shape::line(pts, Stroke::new(width, c)));
    };
    ring(tt.brush.size * 0.5, color, 2.0);
    ring(tt.brush.size * 0.25, color.gamma_multiply(0.5), 1.0);
    let _ = assets;
}

/// Outlined squares where a neighbour tile can go; clicking one creates it.
fn neighbour_slots(
    st: &mut EditorState,
    engine: &mut Engine,
    resp: &egui::Response,
    proj: &Projector,
    painter: &egui::Painter,
    id: GameObjectId,
) -> bool {
    let Some((_, d)) = terrain_data(&engine.world, id) else {
        return false;
    };
    let size = d.size;
    let slots = oxide_engine::terrain::free_neighbour_slots(&engine.world, id);
    let pointer = resp.hover_pos();
    let mut hovered = None;
    for p in &slots {
        let corners = [
            *p,
            *p + Vector3::new(size.x, 0.0, 0.0),
            *p + Vector3::new(size.x, 0.0, size.z),
            *p + Vector3::new(0.0, 0.0, size.z),
            *p,
        ];
        let pts: Vec<egui::Pos2> = corners.iter().filter_map(|c| proj.project(*c)).collect();
        if pts.len() == 5 {
            let inside = pointer.is_some_and(|q| point_in_poly(q, &pts[..4]));
            if inside {
                hovered = Some(*p);
            }
            painter.add(egui::Shape::line(
                pts,
                Stroke::new(
                    if inside { 3.0 } else { 1.5 },
                    Color32::from_rgb(120, 200, 255),
                ),
            ));
        }
    }
    if resp.clicked()
        && let Some(p) = hovered
    {
        st.terrain.commands.push(Cmd::CreateNeighbor(id, p));
    }
    hovered.is_some()
}

fn point_in_poly(p: egui::Pos2, poly: &[egui::Pos2]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Writes every edited terrain to its `.terraindata` file (on scene save).
pub fn save_dirty(tt: &mut TerrainTools, world: &mut World) -> Vec<String> {
    let assets = world.resource_mut::<AssetServer>();
    let mut saved = vec![];
    for path in std::mem::take(&mut tt.dirty) {
        if !path.ends_with(".terraindata") {
            continue;
        }
        let Some(d) = assets
            .terrains
            .handle_of(&path)
            .and_then(|h| assets.terrains.get(&h))
            .cloned()
        else {
            continue;
        };
        match assets.save_terrain_data(&path, &d) {
            Ok(_) => saved.push(path),
            Err(e) => oxide_core::debug_log_error!("{e}"),
        }
    }
    saved
}

/// A unique `Assets/…/New Terrain.terraindata` path for GameObject → 3D Object → Terrain.
pub fn new_terrain_path(assets: &AssetServer) -> String {
    (0..)
        .map(|i| {
            if i == 0 {
                "Assets/New Terrain.terraindata".to_owned()
            } else {
                format!("Assets/New Terrain {i}.terraindata")
            }
        })
        .find(|p| !assets.full_path(p).exists() && assets.terrains.handle_of(p).is_none())
        .expect("free name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_in_polygon() {
        let sq = [
            pos2(0.0, 0.0),
            pos2(10.0, 0.0),
            pos2(10.0, 10.0),
            pos2(0.0, 10.0),
        ];
        assert!(point_in_poly(pos2(5.0, 5.0), &sq));
        assert!(!point_in_poly(pos2(15.0, 5.0), &sq));
    }

    #[test]
    fn paint_modes_have_help() {
        for m in PaintMode::ALL {
            assert!(!m.help().is_empty() && !m.label().is_empty());
        }
    }
}
