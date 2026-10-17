//! The BlenderKit window (Window → BlenderKit, docked next to the Inspector): search by kind
//! (Models, Materials, HDRIs) and price, browse thumbnails, and import. Models are added to
//! the scene, materials go on the selected objects, HDRIs become the skybox. Account holds
//! the API key that unlocks Full Plan assets.

use crate::api::{self, Asset, AssetType, PriceFilter, Profile};
use crate::import;
use crate::settings;
use oxide::EditorContext;
use oxide::EditorWindow;
use oxide::assets::deps::image;
use oxide::math::Vector3;
use oxide::network::web_request::WebRequestOperation;
use oxide::ui::egui::{self, Color32, RichText};
use std::collections::HashMap;

const PAGE_SIZE: u32 = 24;
const THUMB: f32 = 92.0;
const MAX_THUMB_REQUESTS: usize = 6;
/// BlenderKit's orange.
const ACCENT: Color32 = Color32::from_rgb(255, 141, 0);

enum Thumb {
    Loading(WebRequestOperation),
    Ready(egui::TextureHandle),
    Failed,
}

/// What a finished download turns into.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Then {
    /// Import only (into Assets/BlenderKit).
    Import,
    /// Models: add to the scene. Materials: apply to the selection. HDRIs: set the skybox.
    Use,
}

enum Stage {
    /// Asking for the signed URL.
    Link(WebRequestOperation),
    /// Downloading the file.
    File(WebRequestOperation),
}

struct Download {
    asset: Asset,
    kind: AssetType,
    resolution: String,
    extension: String,
    then: Then,
    stage: Stage,
}

pub struct BlenderKitWindow {
    kind: AssetType,
    query: String,
    price: PriceFilter,
    search: Option<WebRequestOperation>,
    /// The search is "load more" (append) rather than new.
    appending: bool,
    results: Vec<Asset>,
    count: u64,
    next: Option<String>,
    searched: bool,
    thumbs: HashMap<String, Thumb>,
    selected: Option<String>,
    resolution: String,
    downloads: Vec<Download>,
    message: Option<(bool, String)>,
    // Account.
    key_file: std::path::PathBuf,
    api_key: String,
    key_edit: String,
    show_account: bool,
    me: Option<WebRequestOperation>,
    profile: Option<Profile>,
    key_error: Option<String>,
}

impl Default for BlenderKitWindow {
    fn default() -> Self {
        let key_file = settings::user_file();
        let api_key = settings::load_api_key(&key_file);
        Self {
            kind: AssetType::Model,
            query: String::new(),
            price: PriceFilter::All,
            search: None,
            appending: false,
            results: Vec::new(),
            count: 0,
            next: None,
            searched: false,
            thumbs: HashMap::new(),
            selected: None,
            resolution: "2K".into(),
            downloads: Vec::new(),
            message: None,
            key_edit: api_key.clone(),
            me: (!api_key.is_empty()).then(|| api::me(&api_key)),
            api_key,
            key_file,
            show_account: false,
            profile: None,
            key_error: None,
        }
    }
}

impl BlenderKitWindow {
    fn start_search(&mut self) {
        let url = api::search_url(self.kind, &self.query, self.price, PAGE_SIZE);
        self.search = Some(api::search(&url, &self.api_key));
        self.appending = false;
        self.searched = true;
    }

    fn load_more(&mut self) {
        if let Some(next) = self.next.take() {
            self.search = Some(api::search(&next, &self.api_key));
            self.appending = true;
        }
    }

    fn poll_search(&mut self) {
        let Some(op) = &self.search else { return };
        if !op.is_done() {
            return;
        }
        let op = self.search.take().expect("search");
        if let Some(e) = api::error_of(&op) {
            self.message = Some((false, format!("Search failed: {e}")));
            return;
        }
        match op.json() {
            Ok(v) => {
                let page = api::parse_search(&v);
                if !self.appending {
                    self.results.clear();
                    self.selected = None;
                }
                self.count = page.count;
                self.next = page.next;
                self.results.extend(page.results);
            }
            Err(e) => self.message = Some((false, format!("Search failed: {e}"))),
        }
    }

    fn poll_account(&mut self) {
        let Some(op) = &self.me else { return };
        if !op.is_done() {
            return;
        }
        let op = self.me.take().expect("me");
        match api::error_of(&op) {
            None => {
                self.profile = op.json().ok().map(|v| api::parse_profile(&v));
                self.key_error = None;
            }
            Some(_) if op.response_code() == 401 || op.response_code() == 403 => {
                self.profile = None;
                self.key_error = Some("BlenderKit didn't accept this API key.".into());
            }
            Some(e) => {
                self.profile = None;
                self.key_error = Some(e);
            }
        }
    }

    fn poll_thumbnails(&mut self, ctx: &egui::Context) {
        for (url, t) in self.thumbs.iter_mut() {
            if let Thumb::Loading(op) = t
                && op.is_done()
            {
                *t = match image::load_from_memory(&op.data()) {
                    Ok(img) if api::error_of(op).is_none() => {
                        let rgba = img.to_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        let color = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                        Thumb::Ready(ctx.load_texture(url, color, egui::TextureOptions::LINEAR))
                    }
                    _ => Thumb::Failed,
                };
            }
        }
        let loading = self
            .thumbs
            .values()
            .filter(|t| matches!(t, Thumb::Loading(_)))
            .count();
        let wanted: Vec<String> = self
            .results
            .iter()
            .map(|a| a.thumbnail.clone())
            .filter(|u| !u.is_empty() && !self.thumbs.contains_key(u))
            .take(MAX_THUMB_REQUESTS.saturating_sub(loading))
            .collect();
        for u in wanted {
            let op = api::get(&u, "");
            self.thumbs.insert(u, Thumb::Loading(op));
        }
    }

    /// Starts a download, or uses the copy already in the project.
    fn fetch(&mut self, ctx: &mut EditorContext, asset: Asset, then: Then) {
        let kind = match asset.asset_type.as_str() {
            "material" => AssetType::Material,
            "hdr" => AssetType::Hdr,
            _ => AssetType::Model,
        };
        let res =
            asset
                .resolutions()
                .into_iter()
                .find(|r| *r == self.resolution)
                .or_else(|| {
                    asset.resolutions().into_iter().rev().find(|r| {
                        api::resolution_pixels(r) <= api::resolution_pixels(&self.resolution)
                    })
                })
                .unwrap_or_default();
        if let Some(path) = self.existing(ctx, &asset, kind, &res) {
            self.finish(ctx, &asset, kind, &path, then);
            return;
        }
        if self
            .downloads
            .iter()
            .any(|d| d.asset.id == asset.id && d.then == then)
        {
            return;
        }
        let Some(file) = asset
            .import_file((!res.is_empty()).then_some(res.as_str()))
            .cloned()
        else {
            self.message = Some((false, format!("{}: nothing Oxide can import", asset.name)));
            return;
        };
        let project = ctx
            .world
            .resource::<oxide::assets::AssetServer>()
            .root
            .clone();
        let uuid = settings::project_uuid(&project);
        let op = api::download_link(&file, &self.api_key, &uuid);
        self.downloads.push(Download {
            asset,
            kind,
            resolution: if res.is_empty() {
                "Original".into()
            } else {
                res
            },
            extension: file.extension,
            then,
            stage: Stage::Link(op),
        });
    }

    /// The asset's imported file, if this project already has it.
    fn existing(
        &self,
        ctx: &EditorContext,
        asset: &Asset,
        kind: AssetType,
        res: &str,
    ) -> Option<String> {
        let folder = import::asset_folder(asset, kind);
        let base = import::safe_name(&asset.name);
        let path = match kind {
            AssetType::Model => format!("{folder}/{base}.glb"),
            AssetType::Material => format!("{folder}/{base}.mat"),
            // One skybox material per resolution.
            AssetType::Hdr => format!("{folder}/{base}_{res}_Skybox.mat"),
        };
        ctx.world
            .resource::<oxide::assets::AssetServer>()
            .full_path(&path)
            .is_file()
            .then_some(path)
    }

    fn poll_downloads(&mut self, ctx: &mut EditorContext) {
        let mut i = 0;
        while i < self.downloads.len() {
            let d = &mut self.downloads[i];
            let next = match &d.stage {
                Stage::Link(op) if op.is_done() => Some(match api::error_of(op) {
                    Some(e) => Err(e),
                    None => op
                        .json()
                        .ok()
                        .and_then(|v| api::parse_download_link(&v))
                        .map(|url| Stage::File(api::get(&url, "")))
                        .ok_or_else(|| "BlenderKit sent no download link".to_owned()),
                }),
                Stage::File(op) if op.is_done() => Some(match api::error_of(op) {
                    Some(e) => Err(e),
                    None => {
                        let d = self.downloads.remove(i);
                        let Stage::File(op) = d.stage else {
                            unreachable!()
                        };
                        self.import(
                            ctx,
                            d.asset,
                            d.kind,
                            &op.data(),
                            &d.extension,
                            &d.resolution,
                            d.then,
                        );
                        continue;
                    }
                }),
                _ => None,
            };
            match next {
                Some(Ok(stage)) => {
                    d.stage = stage;
                    i += 1;
                }
                Some(Err(e)) => {
                    let d = self.downloads.remove(i);
                    self.message = Some((false, format!("{}: {e}", d.asset.name)));
                }
                None => i += 1,
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn import(
        &mut self,
        ctx: &mut EditorContext,
        asset: Asset,
        kind: AssetType,
        bytes: &[u8],
        extension: &str,
        resolution: &str,
        then: Then,
    ) {
        let r = match kind {
            AssetType::Model => import::import_model(ctx.world, &asset, bytes),
            AssetType::Material => {
                import::import_material(ctx.world, &asset, bytes).map(|m| m.path)
            }
            AssetType::Hdr => import::import_hdri(ctx.world, &asset, bytes, extension, resolution),
        };
        match r {
            Ok(path) => {
                ctx.set_status(format!("BlenderKit: imported {} ({path})", asset.name));
                self.finish(ctx, &asset, kind, &path, then);
            }
            Err(e) => self.message = Some((false, format!("{}: {e}", asset.name))),
        }
    }

    /// Uses an imported asset: place the model, apply the material, set the skybox.
    fn finish(
        &mut self,
        ctx: &mut EditorContext,
        asset: &Asset,
        kind: AssetType,
        path: &str,
        then: Then,
    ) {
        if then == Then::Import {
            self.message = Some((true, format!("{} is in {path}", asset.name)));
            return;
        }
        let r = match kind {
            AssetType::Model => {
                ctx.record_undo(&format!("Add {}", asset.name));
                import::place_model(ctx.world, path, Vector3::ZERO).map(|id| {
                    ctx.select([id]);
                    format!("Added {} to the scene", asset.name)
                })
            }
            AssetType::Material => {
                let sel = ctx.selection().to_vec();
                if sel.is_empty() {
                    Ok(format!(
                        "{} is in {path}; select objects to apply it",
                        asset.name
                    ))
                } else {
                    ctx.record_undo(&format!("Apply {}", asset.name));
                    import::apply_material(ctx.world, &sel, path)
                        .map(|n| format!("Applied {} to {n} renderer(s)", asset.name))
                }
            }
            AssetType::Hdr => {
                ctx.record_undo(&format!("Skybox {}", asset.name));
                import::set_skybox(ctx.world, path)
                    .map(|()| format!("{} is the skybox", asset.name))
            }
        };
        self.message = Some(match r {
            Ok(m) => (true, m),
            Err(e) => (false, e),
        });
    }

    fn busy(&self) -> bool {
        self.search.is_some()
            || self.me.is_some()
            || !self.downloads.is_empty()
            || self.thumbs.values().any(|t| matches!(t, Thumb::Loading(_)))
    }

    fn account_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(RichText::new("BlenderKit account").strong());
            ui.weak(
                "Free assets need no account. Paste the API key from your BlenderKit \
                 profile to download Full Plan assets.",
            );
            ui.horizontal(|ui| {
                ui.label("API key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.key_edit)
                        .password(true)
                        .desired_width(ui.available_width()),
                );
            });
            ui.horizontal_wrapped(|ui| {
                let changed = self.key_edit.trim() != self.api_key;
                if ui.add_enabled(changed, egui::Button::new("Save")).clicked() {
                    match settings::save_api_key(&self.key_file, &self.key_edit) {
                        Ok(()) => {
                            self.api_key = self.key_edit.trim().to_owned();
                            self.profile = None;
                            self.key_error = None;
                            self.me = (!self.api_key.is_empty()).then(|| api::me(&self.api_key));
                        }
                        Err(e) => self.key_error = Some(e),
                    }
                }
                if ui
                    .add_enabled(!self.api_key.is_empty(), egui::Button::new("Sign out"))
                    .clicked()
                {
                    let _ = settings::save_api_key(&self.key_file, "");
                    self.api_key.clear();
                    self.key_edit.clear();
                    self.profile = None;
                    self.key_error = None;
                }
                ui.hyperlink_to("Get your API key ↗", api::PROFILE_URL);
            });
            if self.me.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("Checking the key…");
                });
            } else if let Some(e) = &self.key_error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            } else if let Some(p) = &self.profile {
                ui.label(format!("Signed in as {}", p.name));
                if p.has_full_plan() {
                    ui.colored_label(ACCENT, format!("{} Plan: every asset downloads", p.plan));
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.weak("Free Plan: Full Plan assets stay locked.");
                        ui.hyperlink_to("Plans ↗", api::PLANS_URL);
                    });
                }
            }
            ui.weak(format!(
                "Saved for this user only, in {}",
                self.key_file.display()
            ));
        });
    }

    fn card(&mut self, ui: &mut egui::Ui, i: usize) -> egui::Response {
        let a = &self.results[i];
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(THUMB, THUMB + 30.0), egui::Sense::click());
        let selected = self.selected.as_deref() == Some(a.id.as_str());
        let img_rect = egui::Rect::from_min_size(rect.min, egui::vec2(THUMB, THUMB));
        let painter = ui.painter_at(rect);
        painter.rect_filled(img_rect, 4.0, ui.visuals().extreme_bg_color);
        match self.thumbs.get(&a.thumbnail) {
            Some(Thumb::Ready(t)) => {
                egui::Image::new((t.id(), img_rect.size()))
                    .corner_radius(4.0)
                    .paint_at(ui, img_rect);
            }
            Some(Thumb::Failed) => {
                painter.text(
                    img_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "?",
                    egui::FontId::proportional(20.0),
                    ui.visuals().weak_text_color(),
                );
            }
            _ => {
                egui::Spinner::new().paint_at(
                    ui,
                    egui::Rect::from_center_size(img_rect.center(), egui::vec2(16.0, 16.0)),
                );
            }
        }
        // Price badge.
        let (badge, color) = if a.is_free {
            ("FREE", Color32::from_rgb(60, 150, 80))
        } else {
            ("FULL", ACCENT)
        };
        let g = painter.layout_no_wrap(
            badge.into(),
            egui::FontId::proportional(9.0),
            Color32::WHITE,
        );
        let br = egui::Rect::from_min_size(
            img_rect.left_top() + egui::vec2(4.0, 4.0),
            g.size() + egui::vec2(6.0, 2.0),
        );
        painter.rect_filled(br, 3.0, color);
        painter.galley(br.min + egui::vec2(3.0, 1.0), g, Color32::WHITE);
        let stroke = if selected {
            egui::Stroke::new(2.0, ACCENT)
        } else if resp.hovered() {
            egui::Stroke::new(1.0, ui.visuals().widgets.hovered.fg_stroke.color)
        } else {
            egui::Stroke::NONE
        };
        painter.rect_stroke(img_rect, 4.0, stroke, egui::StrokeKind::Inside);
        let name = painter.layout(
            a.name.clone(),
            egui::FontId::proportional(11.0),
            ui.visuals().text_color(),
            THUMB,
        );
        let name_rect = egui::Rect::from_min_size(
            img_rect.left_bottom() + egui::vec2(0.0, 2.0),
            egui::vec2(THUMB, 28.0),
        );
        ui.painter_at(name_rect)
            .galley(name_rect.min, name, ui.visuals().text_color());
        resp.on_hover_text(format!("{}\n{} · {}", a.name, a.author, a.license_label()))
    }

    fn details_ui(&mut self, ctx: &mut EditorContext, ui: &mut egui::Ui) {
        let Some(a) = self
            .selected
            .as_ref()
            .and_then(|id| self.results.iter().find(|a| &a.id == id))
            .cloned()
        else {
            return;
        };
        ui.separator();
        ui.label(RichText::new(&a.name).strong());
        ui.horizontal_wrapped(|ui| {
            ui.weak(format!("by {}", a.author));
            ui.weak("·");
            ui.weak(a.license_label());
            if !a.category.is_empty() {
                ui.weak("·");
                ui.weak(&a.category);
            }
        });
        let locked = !a.is_free && !self.profile.as_ref().is_some_and(Profile::has_full_plan);
        if locked {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(ACCENT, "🔒 Full Plan asset.");
                if ui.link("Add your API key").clicked() {
                    self.show_account = true;
                }
            });
        }
        // Models come as one glTF; materials and HDRIs come in several resolutions.
        let res = a.resolutions();
        if !res.is_empty() && a.asset_type != "model" {
            ui.horizontal(|ui| {
                ui.label("Textures");
                egui::ComboBox::from_id_salt("bk_res")
                    .selected_text(res_label(&self.resolution))
                    .show_ui(ui, |ui| {
                        for r in &res {
                            ui.selectable_value(&mut self.resolution, r.clone(), res_label(r));
                        }
                    });
            });
        }
        let (use_label, can_use) = match a.asset_type.as_str() {
            "material" => ("Apply to Selection", !ctx.selection().is_empty()),
            "hdr" => ("Set as Skybox", true),
            _ => ("Add to Scene", true),
        };
        let downloading = self.downloads.iter().any(|d| d.asset.id == a.id);
        ui.horizontal_wrapped(|ui| {
            let r = ui
                .add_enabled(
                    can_use && !downloading,
                    egui::Button::new(RichText::new(use_label).color(Color32::WHITE)).fill(ACCENT),
                )
                .on_disabled_hover_text("Select objects in the Hierarchy first");
            if r.clicked() {
                self.fetch(ctx, a.clone(), Then::Use);
            }
            if ui
                .add_enabled(!downloading, egui::Button::new("Import to Assets"))
                .clicked()
            {
                self.fetch(ctx, a.clone(), Then::Import);
            }
            ui.hyperlink_to(
                "Web ↗",
                format!("https://www.blenderkit.com/get-blenderkit/{}/", a.base_id),
            );
        });
        if !a.description.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("bk_desc")
                .max_height(60.0)
                .show(ui, |ui| {
                    ui.weak(&a.description);
                });
        }
    }
}

fn res_label(r: &str) -> String {
    if r == "0_5K" {
        "512".into()
    } else {
        r.to_owned()
    }
}

impl EditorWindow for BlenderKitWindow {
    fn on_enable(&mut self, _ctx: &mut EditorContext) {
        if !self.searched {
            self.start_search();
        }
    }

    fn on_gui(&mut self, ctx: &mut EditorContext, ui: &mut egui::Ui) {
        self.poll_search();
        self.poll_account();
        self.poll_thumbnails(ui.ctx());
        self.poll_downloads(ctx);
        if self.busy() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }

        // Header: kind tabs and the account button.
        ui.horizontal(|ui| {
            let mut changed = false;
            for k in AssetType::ALL {
                changed |= ui.selectable_value(&mut self.kind, k, k.label()).changed();
            }
            if changed {
                self.start_search();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = match &self.profile {
                    Some(p) if p.has_full_plan() => format!("👤 {}", p.plan),
                    Some(_) => "👤 Free".into(),
                    None if self.api_key.is_empty() => "👤 Sign in".into(),
                    None => "👤".into(),
                };
                if ui
                    .selectable_label(self.show_account, label)
                    .on_hover_text("BlenderKit account and API key")
                    .clicked()
                {
                    self.show_account = !self.show_account;
                }
            });
        });
        if self.show_account {
            self.account_ui(ui);
        }
        // Search row.
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(format!("Search {}", self.kind.label().to_lowercase()))
                    .desired_width(ui.available_width() - 90.0),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.start_search();
            }
            let before = self.price;
            egui::ComboBox::from_id_salt("bk_price")
                .width(70.0)
                .selected_text(match self.price {
                    PriceFilter::All => "All",
                    PriceFilter::Free => "Free",
                    PriceFilter::Full => "Full Plan",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.price, PriceFilter::All, "All");
                    ui.selectable_value(&mut self.price, PriceFilter::Free, "Free");
                    ui.selectable_value(&mut self.price, PriceFilter::Full, "Full Plan");
                });
            if self.price != before {
                self.start_search();
            }
        });
        ui.horizontal(|ui| {
            if self.search.is_some() && !self.appending {
                ui.spinner();
                ui.weak("Searching…");
            } else {
                ui.weak(format!("{} results", self.count));
            }
        });
        if let Some((ok, m)) = &self.message {
            let color = if *ok {
                ui.visuals().text_color()
            } else {
                ui.visuals().error_fg_color
            };
            let mut close = false;
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(color, m);
                close = ui.small_button("✕").clicked();
            });
            if close {
                self.message = None;
            }
        }
        for d in &self.downloads {
            let (label, p) = match &d.stage {
                Stage::Link(_) => ("Requesting", 0.0),
                Stage::File(op) => ("Downloading", op.progress()),
            };
            ui.add(
                egui::ProgressBar::new(p)
                    .text(format!("{label} {} ({})", d.asset.name, d.resolution))
                    .animate(true),
            );
        }

        // Details of the selected asset at the bottom; the grid fills the rest.
        egui::Panel::bottom(egui::Id::new("bk_details"))
            .resizable(false)
            .show_separator_line(false)
            .show(ui, |ui| self.details_ui(ctx, ui));
        egui::ScrollArea::vertical()
            .id_salt("bk_grid")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.results.is_empty() && self.search.is_none() && self.searched {
                    ui.weak("Nothing found.");
                }
                let mut clicked = None;
                let mut activated = None;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    for i in 0..self.results.len() {
                        let r = self.card(ui, i);
                        if r.clicked() {
                            clicked = Some(i);
                        }
                        if r.double_clicked() {
                            activated = Some(i);
                        }
                    }
                });
                if let Some(i) = clicked {
                    self.selected = Some(self.results[i].id.clone());
                }
                if let Some(i) = activated {
                    let a = self.results[i].clone();
                    self.fetch(ctx, a, Then::Use);
                }
                if self.next.is_some() || (self.appending && self.search.is_some()) {
                    ui.add_space(6.0);
                    ui.vertical_centered(|ui| {
                        if self.search.is_some() {
                            ui.spinner();
                        } else if ui.button("Load more").clicked() {
                            self.load_more();
                        }
                    });
                }
            });
    }
}
