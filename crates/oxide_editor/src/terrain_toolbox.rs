//! Window → Terrain → Terrain Toolbox (Unity's Terrain Tools package): Create New Terrain
//! (tile grids, from a heightmap image), Terrain Settings (applied to many terrains at once)
//! and Terrain Utilities (heightmap and splatmap export).
//! Reference: <https://docs.unity3d.com/Packages/com.unity.terrain-tools@5.0/manual/toolbox-getting-started.html>

use crate::EditorState;
use oxide_assets::AssetServer;
use oxide_engine::Engine;
use oxide_engine::terrain::Terrain;
use oxide_engine::terrain::toolbox::{self, BatchSettings, Heightmap, NewTerrainSettings};
use oxide_scene::GameObjectId;
use oxide_ui::egui::{self, RichText};

/// Terrain fields the Settings tab can apply.
const SETTINGS: [&str; 12] = [
    "draw_heightmap",
    "cast_shadows",
    "lod_distance",
    "draw_trees",
    "tree_distance",
    "billboard_start",
    "fade_length",
    "max_mesh_trees",
    "detail_distance",
    "detail_density",
    "streaming",
    "stream_distance",
];

pub struct TerrainToolbox {
    tab: usize,
    create: NewTerrainSettings,
    heightmap: String,
    template: Terrain,
    apply: [bool; SETTINGS.len()],
    only_selected: bool,
    export_folder: String,
    message: Option<(bool, String)>,
}

impl Default for TerrainToolbox {
    fn default() -> Self {
        Self {
            tab: 0,
            create: NewTerrainSettings::default(),
            heightmap: String::new(),
            template: Terrain::default(),
            apply: [false; SETTINGS.len()],
            only_selected: false,
            export_folder: "Assets/Terrains/Export".into(),
            message: None,
        }
    }
}

const RESOLUTIONS: [u32; 7] = [33, 65, 129, 257, 513, 1025, 2049];

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for (i, name) in [
            "Create New Terrain",
            "Terrain Settings",
            "Terrain Utilities",
        ]
        .iter()
        .enumerate()
        {
            ui.selectable_value(&mut st.terrain_toolbox.tab, i, *name);
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| match st.terrain_toolbox.tab {
        0 => create_ui(st, engine, ui),
        1 => settings_ui(st, engine, ui),
        _ => utilities_ui(st, engine, ui),
    });
    if let Some((ok, m)) = &st.terrain_toolbox.message {
        let c = if *ok {
            egui::Color32::from_rgb(120, 200, 120)
        } else {
            egui::Color32::from_rgb(230, 110, 110)
        };
        ui.colored_label(c, m);
    }
}

fn vec3(ui: &mut egui::Ui, v: &mut oxide_math::Vector3) {
    ui.horizontal(|ui| {
        for (l, c) in [("X", &mut v.x), ("Y", &mut v.y), ("Z", &mut v.z)] {
            ui.label(l);
            ui.add(egui::DragValue::new(c).speed(1.0));
        }
    });
}

fn create_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let tb = &mut st.terrain_toolbox;
    let c = &mut tb.create;
    egui::Grid::new("toolbox_create")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut c.name);
            ui.end_row();
            ui.label("Tiles (X × Z)");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut c.tiles_x).range(1..=16));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut c.tiles_z).range(1..=16));
            });
            ui.end_row();
            ui.label("Tile Width / Height / Length");
            vec3(ui, &mut c.tile_size);
            ui.end_row();
            ui.label("Heightmap Resolution");
            egui::ComboBox::from_id_salt("toolbox_res")
                .selected_text(c.heightmap_resolution.to_string())
                .show_ui(ui, |ui| {
                    for r in RESOLUTIONS {
                        ui.selectable_value(&mut c.heightmap_resolution, r, r.to_string());
                    }
                });
            ui.end_row();
            ui.label("Grouping ID");
            ui.add(egui::DragValue::new(&mut c.grouping_id));
            ui.end_row();
            ui.label("Start Position");
            vec3(ui, &mut c.start);
            ui.end_row();
            ui.label("Folder");
            ui.text_edit_singleline(&mut c.folder);
            ui.end_row();
            ui.label("Heightmap (optional)");
            ui.add(
                egui::TextEdit::singleline(&mut tb.heightmap)
                    .hint_text("Assets/... .png (16-bit) or .raw"),
            );
            ui.end_row();
        });
    let total = (
        c.tile_size.x * c.tiles_x as f32,
        c.tile_size.z * c.tiles_z as f32,
    );
    ui.weak(format!(
        "{} tiles, {:.0} × {:.0} m in all",
        c.tiles_x * c.tiles_z,
        total.0,
        total.1
    ));
    if ui.button("Create").clicked() {
        let assets = engine.world.resource::<AssetServer>();
        let heightmap = if tb.heightmap.trim().is_empty() {
            Ok(None)
        } else {
            let path = tb.heightmap.trim();
            std::fs::read(assets.full_path(path))
                .map_err(|e| format!("{path}: {e}"))
                .and_then(|b| Heightmap::decode(&b, path.to_ascii_lowercase().ends_with(".raw")))
                .map(Some)
        };
        let settings = tb.create.clone();
        let message = match heightmap {
            Err(e) => (false, e),
            Ok(hm) => {
                let now = oxide_core::time::realtime_since_startup();
                st.record(&engine.world, "Create Terrain Tiles", None, now);
                match toolbox::create_terrain_grid(&mut engine.world, &settings, hm.as_ref()) {
                    Ok(tiles) => {
                        if let Some(p) = tiles.first().and_then(|t| engine.world.parent(*t)) {
                            st.select(p);
                        }
                        (true, format!("Created {} terrain tiles", tiles.len()))
                    }
                    Err(e) => (false, e),
                }
            }
        };
        st.terrain_toolbox.message = Some(message);
    }
}

fn targets(st: &EditorState, engine: &Engine) -> Vec<GameObjectId> {
    let w = &engine.world;
    let all: Vec<GameObjectId> = w.query::<Terrain>().map(|(id, _)| id).collect();
    if st.terrain_toolbox.only_selected {
        // Selected terrains, and terrains under selected objects (a tile group).
        all.into_iter()
            .filter(|&id| {
                let mut cur = Some(id);
                while let Some(c) = cur {
                    if st.selection.contains(&c) {
                        return true;
                    }
                    cur = w.parent(c);
                }
                false
            })
            .collect()
    } else {
        all
    }
}

fn settings_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    ui.weak("Tick the settings to apply, set their values, then apply them to the terrains.");
    {
        let tb = &mut st.terrain_toolbox;
        egui::Grid::new("toolbox_settings_ticks")
            .num_columns(4)
            .show(ui, |ui| {
                for (i, f) in SETTINGS.iter().enumerate() {
                    ui.checkbox(&mut tb.apply[i], crate::inspector::nicify(f));
                    if i % 4 == 3 {
                        ui.end_row();
                    }
                }
            });
        ui.separator();
        let apply = tb.apply;
        let mut template = tb.template.clone();
        crate::inspector::reflect_grid_where(
            ui,
            &engine.world,
            &mut template,
            "toolbox-settings",
            &|f| {
                SETTINGS
                    .iter()
                    .position(|s| *s == f)
                    .is_some_and(|i| apply[i])
            },
        );
        tb.template = template;
        ui.checkbox(
            &mut tb.only_selected,
            "Only selected terrains (and their children)",
        );
    }
    let targets = targets(st, engine);
    let tb = &st.terrain_toolbox;
    let fields: Vec<&'static str> = SETTINGS
        .iter()
        .zip(tb.apply)
        .filter(|(_, a)| *a)
        .map(|(f, _)| *f)
        .collect();
    let label = format!(
        "Apply to {} Terrain{}",
        targets.len(),
        if targets.len() == 1 { "" } else { "s" }
    );
    if ui
        .add_enabled(
            !fields.is_empty() && !targets.is_empty(),
            egui::Button::new(label),
        )
        .clicked()
    {
        let now = oxide_core::time::realtime_since_startup();
        st.record(&engine.world, "Terrain Settings", None, now);
        let s = BatchSettings {
            template: st.terrain_toolbox.template.clone(),
            fields,
        };
        let n = toolbox::apply_settings(&mut engine.world, &targets, &s);
        st.terrain_toolbox.message = Some((true, format!("Updated {n} terrains")));
    }
}

fn utilities_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let tb = &mut st.terrain_toolbox;
    ui.label(RichText::new("Export").strong());
    ui.horizontal(|ui| {
        ui.label("Folder");
        ui.text_edit_singleline(&mut tb.export_folder);
    });
    let heights = ui.button("Export Heightmaps (16-bit PNG)").clicked();
    let splats = ui
        .button("Export Splatmaps (RGBA PNG per 4 layers)")
        .clicked();
    if !(heights || splats) {
        return;
    }
    let folder = tb.export_folder.trim_end_matches('/').to_owned();
    let w = &engine.world;
    let assets = w.resource::<AssetServer>();
    let mut written = 0;
    let mut error = None;
    for (id, t) in w.query::<Terrain>() {
        let Some(d) = t.data.as_ref().and_then(|h| assets.terrains.get(h)) else {
            continue;
        };
        let name = w.get(id).map_or("Terrain".to_owned(), |o| o.name.clone());
        let files: Result<Vec<(String, Vec<u8>)>, String> = if heights {
            toolbox::heightmap_png(d).map(|b| vec![(format!("{folder}/{name}_height.png"), b)])
        } else {
            toolbox::splatmap_pngs(d).map(|maps| {
                maps.into_iter()
                    .enumerate()
                    .map(|(i, b)| (format!("{folder}/{name}_splat{i}.png"), b))
                    .collect()
            })
        };
        match files {
            Ok(files) => {
                for (path, bytes) in files {
                    let full = assets.full_path(&path);
                    if let Some(dir) = full.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    match std::fs::write(&full, bytes) {
                        Ok(()) => written += 1,
                        Err(e) => error = Some(format!("{path}: {e}")),
                    }
                }
            }
            Err(e) => error = Some(e),
        }
    }
    tb.message = Some(match error {
        Some(e) => (false, e),
        None => (true, format!("Wrote {written} files to {folder}")),
    });
}
