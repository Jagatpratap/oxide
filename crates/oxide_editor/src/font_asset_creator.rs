//! Window → TextMeshPro → Font Asset Creator: choose a font file, sampling size (or auto
//! sizing), padding, atlas resolution and character set, generate and preview the atlas
//! with its creation report, then save it as a Static Font Asset.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.ugui@2.0/manual/TextMeshPro/FontAssetsCreator.html>

use crate::EditorState;
use oxide_assets::AssetServer;
use oxide_engine::Engine;
use oxide_engine::text::creator::{self, CreatorCharacterSet, CreatorResult, CreatorSettings};
use oxide_ui::egui::{self, RichText};

#[derive(Default)]
pub struct FontAssetCreator {
    settings: CreatorSettings,
    /// Characters from File: a text asset path.
    characters_file: String,
    result: Option<CreatorResult>,
    preview: Option<egui::TextureHandle>,
    report: String,
    save_path: String,
    message: Option<(bool, String)>,
}

const RESOLUTIONS: [u32; 7] = [64, 128, 256, 512, 1024, 2048, 4096];

fn resolution(ui: &mut egui::Ui, id: &str, v: &mut u32) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(v.to_string())
        .width(70.0)
        .show_ui(ui, |ui| {
            for r in RESOLUTIONS {
                ui.selectable_value(v, r, r.to_string());
            }
        });
}

impl FontAssetCreator {
    fn generate(&mut self, assets: &AssetServer) {
        self.message = None;
        let file_text = (self.settings.character_set == CreatorCharacterSet::CharactersFromFile)
            .then(|| std::fs::read_to_string(assets.full_path(&self.characters_file)).ok())
            .flatten();
        let chars = match creator::characters(
            self.settings.character_set,
            &self.settings.character_sequence,
            file_text.as_deref(),
        ) {
            Ok(c) => c,
            Err(e) => {
                self.message = Some((false, e));
                return;
            }
        };
        let bytes = match &self.settings.source_font_file {
            None => None,
            Some(p) => match std::fs::read(assets.full_path(p)) {
                Ok(b) => Some(b),
                Err(e) => {
                    self.message = Some((false, format!("{p}: {e}")));
                    return;
                }
            },
        };
        match creator::generate(&self.settings, &chars, bytes) {
            Ok(r) => {
                let font = self
                    .settings
                    .source_font_file
                    .clone()
                    .unwrap_or_else(|| "Built-in".into());
                self.report = r.report(&font);
                let stem = font
                    .rsplit('/')
                    .next()
                    .and_then(|f| f.rsplit_once('.').map(|(s, _)| s))
                    .unwrap_or("Font");
                if self.save_path.is_empty() || self.result.is_none() {
                    self.save_path = format!("Assets/{stem} SDF.fontasset");
                }
                self.result = Some(r);
                self.preview = None;
            }
            Err(e) => self.message = Some((false, e)),
        }
    }
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &mut st.font_asset_creator;
    let assets = engine.world.resource_mut::<AssetServer>();
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("font_asset_creator")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Source Font File");
                let fonts = assets.paths_of_type("Font");
                let s = &mut w.settings;
                egui::ComboBox::from_id_salt("fac_font")
                    .selected_text(s.source_font_file.as_deref().unwrap_or("Built-in"))
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.source_font_file, None, "Built-in");
                        for f in fonts {
                            ui.selectable_value(&mut s.source_font_file, Some(f.clone()), f);
                        }
                    });
                ui.end_row();

                ui.label("Sampling Point Size");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut s.auto_sizing, true, "Auto Sizing");
                    ui.radio_value(&mut s.auto_sizing, false, "Custom Size");
                    ui.add_enabled(
                        !s.auto_sizing,
                        egui::DragValue::new(&mut s.sampling_point_size).range(8..=512),
                    );
                });
                ui.end_row();

                ui.label("Padding");
                ui.add(egui::DragValue::new(&mut s.padding).range(1..=64));
                ui.end_row();

                ui.label("Atlas Resolution");
                ui.horizontal(|ui| {
                    resolution(ui, "fac_w", &mut s.atlas_width);
                    ui.label("×");
                    resolution(ui, "fac_h", &mut s.atlas_height);
                });
                ui.end_row();

                ui.label("Character Set");
                egui::ComboBox::from_id_salt("fac_set")
                    .selected_text(s.character_set.label())
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for c in CreatorCharacterSet::ALL {
                            ui.selectable_value(&mut s.character_set, c, c.label());
                        }
                    });
                ui.end_row();

                match s.character_set {
                    CreatorCharacterSet::CustomRange
                    | CreatorCharacterSet::UnicodeRangeHex
                    | CreatorCharacterSet::CustomCharacters => {
                        ui.label(match s.character_set {
                            CreatorCharacterSet::CustomRange => "Character Sequence (Decimal)",
                            CreatorCharacterSet::UnicodeRangeHex => "Character Sequence (Hex)",
                            _ => "Custom Character List",
                        });
                        ui.add(
                            egui::TextEdit::multiline(&mut s.character_sequence)
                                .desired_rows(3)
                                .desired_width(260.0)
                                .hint_text(match s.character_set {
                                    CreatorCharacterSet::CustomRange => "32-126,160-255",
                                    CreatorCharacterSet::UnicodeRangeHex => "20-7E,A0-FF",
                                    _ => "Characters to include",
                                }),
                        );
                        ui.end_row();
                    }
                    CreatorCharacterSet::CharactersFromFile => {
                        ui.label("Character File");
                        let files = assets.paths_of_type("TextAsset");
                        egui::ComboBox::from_id_salt("fac_file")
                            .selected_text(if w.characters_file.is_empty() {
                                "None"
                            } else {
                                w.characters_file.as_str()
                            })
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                for f in files {
                                    ui.selectable_value(&mut w.characters_file, f.clone(), f);
                                }
                            });
                        ui.end_row();
                    }
                    _ => {}
                }
            });
        ui.add_space(6.0);
        if ui.button("Generate Font Atlas").clicked() {
            w.generate(assets);
        }
        if let Some((ok, m)) = &w.message {
            let c = if *ok {
                egui::Color32::from_rgb(120, 200, 120)
            } else {
                egui::Color32::from_rgb(230, 110, 110)
            };
            ui.colored_label(c, m);
        }
        let Some(r) = &w.result else {
            return;
        };
        ui.separator();
        ui.label(RichText::new("Font Asset Creation Report").strong());
        ui.add(
            egui::TextEdit::multiline(&mut w.report.as_str())
                .font(egui::TextStyle::Monospace)
                .desired_rows(5)
                .desired_width(f32::INFINITY),
        );
        // The atlas page: distance field as grey.
        let tex = w.preview.get_or_insert_with(|| {
            let page = &r.atlas.pages[0];
            let (aw, ah) = (
                r.atlas.settings.atlas_width as usize,
                r.atlas.settings.atlas_height as usize,
            );
            let rgba: Vec<u8> = page
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[3], p[3], p[3], 255])
                .collect();
            ui.ctx().load_texture(
                "font_asset_creator_preview",
                egui::ColorImage::from_rgba_unmultiplied([aw, ah], &rgba),
                egui::TextureOptions::LINEAR,
            )
        });
        let side = ui.available_width().min(420.0);
        ui.image((
            tex.id(),
            egui::vec2(side, side * tex.aspect_ratio().recip()),
        ));
        ui.horizontal(|ui| {
            ui.label("Save As");
            ui.add(egui::TextEdit::singleline(&mut w.save_path).desired_width(260.0));
            if ui.button("Save").clicked() {
                let path = w.save_path.trim().to_owned();
                w.message = Some(
                    if !path.starts_with("Assets/") || !path.ends_with(".fontasset") {
                        (false, "Save under Assets/ as a .fontasset file".into())
                    } else {
                        match assets.save_data(&path, Box::new(r.font_asset())) {
                            Ok(()) => (true, format!("Saved {path}")),
                            Err(e) => (false, e),
                        }
                    },
                );
            }
        });
    });
}
