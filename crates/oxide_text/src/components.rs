//! The TextMeshPro component (3D text) and the material settings text components share.
//! Unity keeps the material settings in a material preset; here they sit on the component.

use crate::font::{FontAsset, FontLibrary};
use crate::layout::{
    HorizontalAlignment, LayoutSettings, TextMesh, TextOverflowModes, VerticalAlignment,
};
use crate::rich::{Case, Style};
use oxide_assets::{AssetServer, Handle};
use oxide_core::Reflect;
use oxide_math::{Color, Vector2, Vector3};
use oxide_scene::{Component, GameObjectId, World};
use std::collections::HashMap;

/// The TextMeshPro "Distance Field" shader settings.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct TextMaterial {
    /// Multiplies the vertex colour.
    pub face_color: Color,
    /// Thickens (+) or thins (-) the letters.
    #[reflect(range(-1, 1))]
    pub face_dilate: f32,
    /// Blurs the edges.
    #[reflect(range(0, 1))]
    pub softness: f32,
    pub outline_color: Color,
    /// Outline thickness, centred on the letter edge.
    #[reflect(range(0, 1))]
    pub outline_width: f32,
    /// Drop shadow (TMP "Underlay").
    pub underlay: bool,
    pub underlay_color: Color,
    /// Offset in fractions of the padding (+x right, +y up).
    pub underlay_offset: Vector2,
    #[reflect(range(-1, 1))]
    pub underlay_dilate: f32,
    #[reflect(range(0, 1))]
    pub underlay_softness: f32,
}

impl Default for TextMaterial {
    fn default() -> Self {
        Self {
            face_color: Color::WHITE,
            face_dilate: 0.0,
            softness: 0.0,
            outline_color: Color::BLACK,
            outline_width: 0.0,
            underlay: false,
            underlay_color: Color::new(0.0, 0.0, 0.0, 0.5),
            underlay_offset: Vector2::new(0.5, -0.5),
            underlay_dilate: 0.0,
            underlay_softness: 0.3,
        }
    }
}

impl TextMaterial {
    /// Linear colours for the renderer.
    pub fn to_sdf(&self) -> oxide_render::SdfMaterial {
        let lin = |c: Color| {
            let l = c.linear();
            [l.r, l.g, l.b, c.a]
        };
        oxide_render::SdfMaterial {
            face: lin(self.face_color),
            outline: lin(self.outline_color),
            underlay: lin(self.underlay_color),
            dilate: self.face_dilate,
            softness: self.softness,
            outline_width: self.outline_width,
            underlay_on: self.underlay,
            underlay_offset: self.underlay_offset.to_array(),
            underlay_dilate: self.underlay_dilate,
            underlay_softness: self.underlay_softness,
        }
    }
}

/// TMP Font Style buttons.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontStyles {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub lowercase: bool,
    pub uppercase: bool,
    pub smallcaps: bool,
}

impl FontStyles {
    pub fn to_style(self) -> Style {
        Style {
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            strikethrough: self.strikethrough,
            case: if self.uppercase {
                Case::Upper
            } else if self.lowercase {
                Case::Lower
            } else if self.smallcaps {
                Case::SmallCaps
            } else {
                Case::Normal
            },
            ..Default::default()
        }
    }
}

/// The text settings TextMeshPro and TextMeshProUGUI share (everything but placement).
#[derive(Clone, Debug, PartialEq)]
pub struct TextInput<'a> {
    pub text: &'a str,
    pub font_asset: Option<&'a Handle<FontAsset>>,
    pub rich_text: bool,
    pub layout: LayoutSettings,
    pub auto_size: Option<(f32, f32)>,
}

/// Lays out `input`, loading its font and shrinking/growing within the auto-size range.
pub fn build(lib: &mut FontLibrary, assets: &mut AssetServer, input: &TextInput) -> TextMesh {
    let key = lib.load(assets, input.font_asset);
    let chain = lib.chain(assets, &key);
    let run = |lib: &mut FontLibrary, size: f32| {
        let s = LayoutSettings {
            font_size: size,
            ..input.layout.clone()
        };
        let chars = crate::rich::parse(input.text, input.rich_text, size);
        crate::layout::layout(&chars, lib, &chain, &s)
    };
    let mesh = match input.auto_size {
        None => run(lib, input.layout.font_size),
        Some((min, max)) => {
            // The largest size in range that fits (TMP's auto size).
            let (mut lo, mut hi) = (min.max(0.5), max.max(min.max(0.5)));
            let mut best = run(lib, lo);
            if !best.overflowed {
                let top = run(lib, hi);
                if !top.overflowed {
                    best = top;
                } else {
                    for _ in 0..10 {
                        let mid = (lo + hi) * 0.5;
                        let m = run(lib, mid);
                        if m.overflowed {
                            hi = mid;
                        } else {
                            lo = mid;
                            best = m;
                        }
                    }
                }
            }
            best
        }
    };
    lib.upload(assets);
    mesh
}

/// Unity `TextMeshPro`: text in the 3D world, inside a rectangle centred on the object
/// (its local X/Y plane, readable from -Z like a Quad).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct TextMeshPro {
    pub text: String,
    /// Empty uses the default Font Asset.
    pub font_asset: Option<Handle<FontAsset>>,
    pub font_style: FontStyles,
    /// Points; 10 points = 1 world unit per em.
    pub font_size: f32,
    pub auto_size: bool,
    pub font_size_min: f32,
    pub font_size_max: f32,
    /// Vertex colour.
    pub color: Color,
    /// Spacing in em / 100.
    pub character_spacing: f32,
    pub word_spacing: f32,
    pub line_spacing: f32,
    pub paragraph_spacing: f32,
    pub horizontal_alignment: HorizontalAlignment,
    pub vertical_alignment: VerticalAlignment,
    pub wrapping: bool,
    pub overflow: TextOverflowModes,
    pub rich_text: bool,
    /// The text rectangle in world units (Unity's RectTransform size; 0 = unlimited).
    pub rect_size: Vector2,
    pub material: TextMaterial,
}

impl Default for TextMeshPro {
    fn default() -> Self {
        Self {
            text: "Sample text".into(),
            font_asset: None,
            font_style: FontStyles::default(),
            font_size: 36.0,
            auto_size: false,
            font_size_min: 18.0,
            font_size_max: 72.0,
            color: Color::WHITE,
            character_spacing: 0.0,
            word_spacing: 0.0,
            line_spacing: 0.0,
            paragraph_spacing: 0.0,
            horizontal_alignment: HorizontalAlignment::Left,
            vertical_alignment: VerticalAlignment::Top,
            wrapping: true,
            overflow: TextOverflowModes::Overflow,
            rich_text: true,
            rect_size: Vector2::new(20.0, 5.0),
            material: TextMaterial::default(),
        }
    }
}

impl Component for TextMeshPro {}

impl TextMeshPro {
    /// World units per point.
    pub const UNIT_SCALE: f32 = 0.1;

    pub fn input(&self) -> TextInput<'_> {
        TextInput {
            text: &self.text,
            font_asset: self.font_asset.as_ref(),
            rich_text: self.rich_text,
            layout: LayoutSettings {
                font_size: self.font_size,
                unit_scale: Self::UNIT_SCALE,
                width: self.rect_size.x.max(0.0),
                height: self.rect_size.y.max(0.0),
                wrap: self.wrapping,
                overflow: self.overflow,
                horizontal: self.horizontal_alignment,
                vertical: self.vertical_alignment,
                character_spacing: self.character_spacing,
                word_spacing: self.word_spacing,
                line_spacing: self.line_spacing,
                paragraph_spacing: self.paragraph_spacing,
                color: self.color,
                style: self.font_style.to_style(),
            },
            auto_size: self
                .auto_size
                .then_some((self.font_size_min, self.font_size_max)),
        }
    }
}

/// Laid out text of every TextMeshPro, rebuilt when its settings or font change.
#[derive(Default)]
pub struct TextMeshes {
    pub meshes: HashMap<GameObjectId, (TextMeshPro, u64, TextMesh)>,
}

/// Lays out changed TextMeshPro components (once per frame, before extraction).
pub fn update(world: &mut World) {
    if world.try_resource::<FontLibrary>().is_none() {
        world.resources.insert(FontLibrary::default());
    }
    if world.try_resource::<TextMeshes>().is_none() {
        world.resources.insert(TextMeshes::default());
    }
    let comps: Vec<(GameObjectId, TextMeshPro)> = world
        .query::<TextMeshPro>()
        .map(|(id, t)| (id, t.clone()))
        .collect();
    let mut lib = world
        .resources
        .remove::<FontLibrary>()
        .expect("inserted above");
    let mut meshes = world
        .resources
        .remove::<TextMeshes>()
        .expect("inserted above");
    {
        let assets = world.resource_mut::<AssetServer>();
        meshes
            .meshes
            .retain(|id, _| comps.iter().any(|(c, _)| c == id));
        for (id, t) in comps {
            let version = t
                .font_asset
                .as_ref()
                .and_then(|h| assets.data.version(h.path()))
                .unwrap_or(0);
            if meshes
                .meshes
                .get(&id)
                .is_some_and(|(old, v, _)| *old == t && *v == version)
            {
                continue;
            }
            let mesh = build(&mut lib, assets, &t.input());
            meshes.meshes.insert(id, (t, version, mesh));
        }
    }
    world.resources.insert(lib);
    world.resources.insert(meshes);
}

/// TextMeshPro components as render draws (one per atlas page they use).
pub fn extract(world: &World, out: &mut Vec<oxide_render::TextDraw>) {
    let (Some(meshes), Some(lib)) = (
        world.try_resource::<TextMeshes>(),
        world.try_resource::<FontLibrary>(),
    ) else {
        return;
    };
    for (&id, (t, _, mesh)) in &meshes.meshes {
        if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        if world.get_component::<TextMeshPro>(id).is_none() {
            continue;
        }
        let model = world.local_to_world(id);
        // The rectangle is centred on the object.
        let off = Vector2::new(-t.rect_size.x * 0.5, t.rect_size.y * 0.5);
        let m = t.material.to_sdf();
        let mut by_page: HashMap<(u32, u32), Vec<oxide_render::TextInstance>> = HashMap::new();
        for q in &mesh.quads {
            let Some(atlas) = mesh.fonts.get(q.font as usize).and_then(|k| lib.get(k)) else {
                continue;
            };
            let corners = q
                .corners
                .map(|c| model.transform_point3(Vector3::new(c[0] + off.x, c[1] + off.y, 0.0)));
            let c = q.color.linear();
            let bold = if q.bold {
                atlas.settings.bold_style
            } else {
                0.0
            };
            by_page
                .entry((q.font, q.page))
                .or_default()
                .push(oxide_render::TextInstance::new(
                    corners,
                    q.uv,
                    [c.r, c.g, c.b, q.color.a],
                    bold,
                    &m,
                    atlas.spread(),
                    [
                        atlas.settings.atlas_width as f32,
                        atlas.settings.atlas_height as f32,
                    ],
                ));
        }
        let center = model.transform_point3(Vector3::ZERO);
        let mut pages: Vec<_> = by_page.into_iter().collect();
        pages.sort_by_key(|(k, _)| *k);
        let texture = |font: u32, page: u32| {
            mesh.fonts
                .get(font as usize)
                .and_then(|k| lib.get(k))
                .map(|a| oxide_assets::AssetId::from_path(&a.texture_path(page)))
        };
        // Shadows of the whole text first, then the letters.
        if m.underlay_on {
            for ((font, page), instances) in &pages {
                if let Some(texture) = texture(*font, *page) {
                    out.push(oxide_render::TextDraw {
                        texture,
                        center,
                        instances: instances.iter().map(|i| i.underlay_only()).collect(),
                    });
                }
            }
        }
        for ((font, page), instances) in pages {
            if let Some(texture) = texture(font, page) {
                out.push(oxide_render::TextDraw {
                    texture,
                    center,
                    instances: if m.underlay_on {
                        instances.into_iter().map(|i| i.face_only()).collect()
                    } else {
                        instances
                    },
                });
            }
        }
    }
}
