//! The `.terraindata` file (Unity's TerrainData `.asset`): a YAML header with the settings,
//! layers, tree and detail prototypes, followed by binary blocks — 16-bit heights (like
//! Unity's RAW heightmaps), RGBA control maps, holes, tree instances and detail densities.

use crate::handle::Handle;
use crate::server::AssetServer;
use crate::terrain::{
    DetailPrototype, DetailRenderMode, TerrainData, TerrainLayer, TreeInstance, TreePrototype,
    WindSettings,
};
use oxide_math::{Color, Vector2, Vector3};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const MAGIC: &[u8; 8] = b"OXTERRA1";

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct LayerFile {
    /// The `.terrainlayer` asset (preferred), else the inline layer below.
    asset: Option<String>,
    name: String,
    diffuse: Option<String>,
    normal_map: Option<String>,
    normal_scale: f32,
    tile_size: [f32; 2],
    tile_offset: [f32; 2],
    tint: [f32; 4],
    smoothness: f32,
    metallic: f32,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct TreeFile {
    source: Option<String>,
    mesh: String,
    materials: Vec<String>,
    bend_factor: f32,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct DetailFile {
    texture: Option<String>,
    mesh: Option<String>,
    materials: Vec<String>,
    source: Option<String>,
    render_mode: String,
    width: [f32; 2],
    height: [f32; 2],
    noise_spread: f32,
    healthy_color: [f32; 4],
    dry_color: [f32; 4],
    density: f32,
    align_to_ground: f32,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Header {
    name: String,
    size: [f32; 3],
    heightmap_resolution: u32,
    alphamap_resolution: u32,
    alphamap_count: u32,
    detail_resolution: u32,
    holes: bool,
    tree_count: u32,
    layers: Vec<LayerFile>,
    tree_prototypes: Vec<TreeFile>,
    detail_prototypes: Vec<DetailFile>,
    wind: [f32; 3],
    grass_tint: [f32; 4],
}

fn c4(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

fn color(a: [f32; 4]) -> Color {
    Color::new(a[0], a[1], a[2], a[3])
}

/// Encodes a TerrainData (as Unity writes a TerrainData asset).
pub fn encode(d: &TerrainData) -> Vec<u8> {
    let header = Header {
        name: d.name.clone(),
        size: d.size.to_array(),
        heightmap_resolution: d.resolution,
        alphamap_resolution: d.splat_resolution,
        alphamap_count: d.alphamaps.len() as u32,
        detail_resolution: d.detail_resolution,
        holes: !d.holes.is_empty(),
        tree_count: d.trees.len() as u32,
        layers: d
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| LayerFile {
                asset: d.layer_assets.get(i).cloned().flatten(),
                name: l.name.clone(),
                diffuse: l.diffuse.as_ref().map(|h| h.path().to_owned()),
                normal_map: l.normal_map.as_ref().map(|h| h.path().to_owned()),
                normal_scale: l.normal_scale,
                tile_size: l.tile_size.to_array(),
                tile_offset: l.tile_offset.to_array(),
                tint: c4(l.tint),
                smoothness: l.smoothness,
                metallic: l.metallic,
            })
            .collect(),
        tree_prototypes: d
            .tree_prototypes
            .iter()
            .map(|t| TreeFile {
                source: t.source.clone(),
                mesh: t.mesh.path().to_owned(),
                materials: t.materials.iter().map(|m| m.path().to_owned()).collect(),
                bend_factor: t.bend_factor,
            })
            .collect(),
        detail_prototypes: d
            .detail_prototypes
            .iter()
            .map(|p| DetailFile {
                texture: p.texture.as_ref().map(|h| h.path().to_owned()),
                mesh: p.mesh.as_ref().map(|h| h.path().to_owned()),
                materials: p.materials.iter().map(|m| m.path().to_owned()).collect(),
                source: p.source.clone(),
                render_mode: format!("{:?}", p.render_mode),
                width: [p.min_width, p.max_width],
                height: [p.min_height, p.max_height],
                noise_spread: p.noise_spread,
                healthy_color: c4(p.healthy_color),
                dry_color: c4(p.dry_color),
                density: p.density,
                align_to_ground: p.align_to_ground,
            })
            .collect(),
        wind: [d.wind.speed, d.wind.size, d.wind.bending],
        grass_tint: c4(d.wind.grass_tint),
    };
    let yaml = serde_yaml_ng::to_string(&header).unwrap_or_default();
    let mut out = Vec::with_capacity(yaml.len() + d.heights.len() * 2 + 64);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(yaml.len() as u32).to_le_bytes());
    out.extend_from_slice(yaml.as_bytes());
    for h in d.heights.iter() {
        out.extend_from_slice(&((h.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes());
    }
    for m in &d.alphamaps {
        for px in m.iter() {
            out.extend_from_slice(px);
        }
    }
    if !d.holes.is_empty() {
        out.extend_from_slice(&d.holes);
    }
    for t in &d.trees {
        for f in [
            t.position.x,
            t.position.y,
            t.position.z,
            t.width_scale,
            t.height_scale,
            t.rotation,
            t.color.r,
            t.color.g,
            t.color.b,
        ] {
            out.extend_from_slice(&f.to_le_bytes());
        }
        out.extend_from_slice(&t.prototype.to_le_bytes());
    }
    for layer in &d.details {
        out.extend_from_slice(layer);
    }
    out
}

/// Decodes a `.terraindata` file.
pub fn decode(bytes: &[u8]) -> Result<TerrainData, String> {
    if bytes.len() < 12 || &bytes[..8] != MAGIC {
        return Err("not a .terraindata file".into());
    }
    let ylen = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes")) as usize;
    let yaml = std::str::from_utf8(bytes.get(12..12 + ylen).ok_or("truncated header")?)
        .map_err(|e| e.to_string())?;
    let h: Header = serde_yaml_ng::from_str(yaml).map_err(|e| e.to_string())?;
    let mut at = 12 + ylen;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let s = bytes.get(at..at + n).ok_or("truncated terrain data")?;
        at += n;
        Ok(s)
    };
    let n = h.heightmap_resolution.max(2);
    let heights: Vec<f32> = take((n * n * 2) as usize)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b) as f32 / 65535.0)
        .collect();
    let s = h.alphamap_resolution.max(1);
    let mut alphamaps = vec![];
    for _ in 0..h.alphamap_count {
        alphamaps.push(Arc::new(
            take((s * s * 4) as usize)?.as_chunks::<4>().0.to_vec(),
        ));
    }
    let holes = if h.holes {
        take(((n - 1) * (n - 1)) as usize)?.to_vec()
    } else {
        vec![]
    };
    let mut trees = Vec::with_capacity(h.tree_count as usize);
    for _ in 0..h.tree_count {
        let b = take(40)?;
        let f = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
        trees.push(TreeInstance {
            position: Vector3::new(f(0), f(1), f(2)),
            width_scale: f(3),
            height_scale: f(4),
            rotation: f(5),
            color: Color::rgb(f(6), f(7), f(8)),
            prototype: u32::from_le_bytes(b[36..40].try_into().expect("4 bytes")),
        });
    }
    let dres = h.detail_resolution.max(1);
    let mut details = vec![];
    for _ in 0..h.detail_prototypes.len() {
        details.push(Arc::new(take((dres * dres) as usize)?.to_vec()));
    }
    let mut d = TerrainData::flat(Vector3::from_array(h.size), n);
    d.name = h.name;
    d.heights = Arc::new(heights);
    d.splat_resolution = s;
    d.alphamaps = alphamaps;
    d.holes = Arc::new(holes);
    d.layer_assets = h.layers.iter().map(|l| l.asset.clone()).collect();
    d.layers = h
        .layers
        .into_iter()
        .map(|l| TerrainLayer {
            name: l.name,
            diffuse: l.diffuse.map(Handle::from_path),
            normal_map: l.normal_map.map(Handle::from_path),
            normal_scale: l.normal_scale,
            tile_size: Vector2::from_array(l.tile_size),
            tile_offset: Vector2::from_array(l.tile_offset),
            tint: color(l.tint),
            smoothness: l.smoothness,
            metallic: l.metallic,
        })
        .collect();
    d.tree_prototypes = h
        .tree_prototypes
        .into_iter()
        .map(|t| TreePrototype {
            source: t.source,
            mesh: Handle::from_path(t.mesh),
            materials: t.materials.into_iter().map(Handle::from_path).collect(),
            bend_factor: t.bend_factor,
        })
        .collect();
    d.trees = trees;
    d.detail_resolution = dres;
    d.detail_prototypes = h
        .detail_prototypes
        .into_iter()
        .map(|p| DetailPrototype {
            texture: p.texture.map(Handle::from_path),
            mesh: p.mesh.map(Handle::from_path),
            materials: p.materials.into_iter().map(Handle::from_path).collect(),
            source: p.source,
            render_mode: match p.render_mode.as_str() {
                "GrassBillboard" => DetailRenderMode::GrassBillboard,
                "VertexLit" => DetailRenderMode::VertexLit,
                _ => DetailRenderMode::Grass,
            },
            min_width: p.width[0],
            max_width: p.width[1],
            min_height: p.height[0],
            max_height: p.height[1],
            noise_spread: p.noise_spread,
            healthy_color: color(p.healthy_color),
            dry_color: color(p.dry_color),
            density: p.density,
            align_to_ground: p.align_to_ground,
        })
        .collect();
    d.details = details;
    d.wind = WindSettings {
        speed: h.wind[0],
        size: h.wind[1],
        bending: h.wind[2],
        grass_tint: color(h.grass_tint),
    };
    d.fit_alphamaps();
    Ok(d)
}

impl AssetServer {
    /// Loads (or reloads) a `.terraindata` file; layers saved as `.terrainlayer` references
    /// are read from those assets.
    pub fn load_terrain_data(&mut self, path: &str) -> Result<Handle<TerrainData>, String> {
        let bytes = self.read_file(path)?;
        let mut d = decode(&bytes).map_err(|e| format!("{path}: {e}"))?;
        for (i, asset) in d.layer_assets.clone().iter().enumerate() {
            if let Some(a) = asset {
                let h: Handle<TerrainLayer> = Handle::from_path(a.clone());
                if let Some(l) = self.data_asset(&h).cloned() {
                    d.layers[i] = l;
                }
            }
        }
        for l in &d.layers {
            for t in [&l.diffuse, &l.normal_map].into_iter().flatten() {
                if let Err(e) = self.ensure_loaded(t.path()) {
                    oxide_core::debug_log_warning!("{path}: {e}");
                }
            }
        }
        // Reloading keeps the handle (and bumps every version so everything refreshes).
        let old = self
            .terrains
            .handle_of(path)
            .and_then(|h| self.terrains.get(&h))
            .map(|o| o.version);
        if let Some(v) = old {
            d.version = v + 1;
            d.versions.heights = v + 1;
            d.versions.alphamaps = v + 1;
            d.versions.layers = v + 1;
            d.versions.holes = v + 1;
            d.versions.trees = v + 1;
            d.versions.details = v + 1;
        }
        Ok(self.terrains.insert(path.to_owned(), d))
    }

    /// Writes a TerrainData to a `.terraindata` file (and stores it under that path).
    pub fn save_terrain_data(
        &mut self,
        path: &str,
        d: &TerrainData,
    ) -> Result<Handle<TerrainData>, String> {
        self.write_bytes(path, &encode(d))?;
        match self.terrains.handle_of(path) {
            Some(h) => {
                if let Some(cur) = self.terrains.get_mut(&h)
                    && !std::ptr::eq(cur, d)
                {
                    *cur = d.clone();
                }
                Ok(h)
            }
            None => Ok(self.terrains.insert(path.to_owned(), d.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{Brush, HeightOp};

    #[test]
    fn round_trip() {
        let mut d = TerrainData::flat(Vector3::new(200.0, 60.0, 200.0), 65);
        d.name = "Island".into();
        d.paint_height(0.5, 0.5, &Brush::default(), HeightOp::Raise(10.0));
        for i in 0..5 {
            d.add_layer(
                TerrainLayer {
                    name: format!("L{i}"),
                    tile_size: Vector2::splat(4.0 + i as f32),
                    ..Default::default()
                },
                (i == 2).then(|| "Assets/Rock.terrainlayer".to_owned()),
            );
        }
        d.paint_texture(0.3, 0.3, &Brush::default(), 4, 1.0);
        d.paint_holes(
            0.8,
            0.8,
            &Brush {
                size: 10.0,
                ..Default::default()
            },
            false,
        );
        d.tree_prototypes.push(TreePrototype::new(
            Handle::from_path("builtin:tree/simple"),
            vec![Handle::from_path("Assets/Bark.mat")],
        ));
        d.mass_place_trees(50, 3, false);
        d.detail_prototypes.push(DetailPrototype {
            texture: Some(Handle::from_path("Assets/Grass.png")),
            ..Default::default()
        });
        d.paint_details(0.5, 0.5, &Brush::default(), 0, 12.0, false);
        let back = decode(&encode(&d)).unwrap();
        assert_eq!(back.name, "Island");
        assert_eq!(back.resolution, 65);
        let err = back
            .heights
            .iter()
            .zip(d.heights.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(err < 1.0 / 65535.0, "16-bit heights: {err}");
        assert_eq!(back.layers.len(), 5);
        assert_eq!(
            back.layer_assets[2].as_deref(),
            Some("Assets/Rock.terrainlayer")
        );
        assert_eq!(back.alphamaps, d.alphamaps);
        assert_eq!(back.holes, d.holes);
        assert_eq!(back.trees, d.trees);
        assert_eq!(back.tree_prototypes, d.tree_prototypes);
        assert_eq!(back.details, d.details);
        assert_eq!(
            back.detail_prototypes[0].texture,
            d.detail_prototypes[0].texture
        );
    }
}
