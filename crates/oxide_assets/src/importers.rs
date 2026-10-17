//! Unity's Model and Audio importer settings, kept in the asset's `.meta` file next to its
//! GUID (like the Texture Importer). Changing them re-imports the asset.
//! References: <https://docs.unity3d.com/Manual/FBXImporter-Model.html>,
//! <https://docs.unity3d.com/Manual/class-AudioClip.html>

use crate::handle::Handle;
use crate::model::Model;
use crate::server::AssetServer;
use oxide_core::Reflect;

/// Unity `ModelImporterNormals`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelImporterNormals {
    /// Normals from the file (calculated if it has none).
    #[default]
    Import,
    /// Recalculated, smoothing faces closer than Smoothing Angle.
    Calculate,
}

/// Unity `ModelImporterAnimationType` (the Rig tab).
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelImporterAnimationType {
    /// No animation: no Animator and no clips.
    None,
    /// The legacy Animation component plays the first clip.
    Legacy,
    /// An Animator with a controller made from the clips.
    #[default]
    Generic,
    /// Generic plus humanoid bone mapping (retargetable clips).
    Humanoid,
}

/// Unity Model Import Settings (Model, Rig, Animation and Materials tabs).
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(default)]
pub struct ModelImporter {
    // Model
    /// Multiplies every position (Unity "Scale Factor").
    pub scale_factor: f32,
    /// FBX: convert the file's unit to metres (Unity "Convert Units").
    pub convert_units: bool,
    /// Add a Mesh Collider to every mesh when the model is placed in a scene.
    pub generate_colliders: bool,
    pub normals: ModelImporterNormals,
    #[reflect(range(0, 180))]
    pub smoothing_angle: f32,
    /// Swap the first and second UV sets.
    pub swap_uvs: bool,
    /// Lightmap UVs for static (non-skinned) meshes that have none.
    pub generate_lightmap_uvs: bool,
    // Rig
    pub animation_type: ModelImporterAnimationType,
    // Animation
    pub import_animation: bool,
    /// Clips loop (Unity's per-clip "Loop Time", for every clip).
    pub loop_time: bool,
    // Materials
    /// Off: every mesh gets the default material.
    pub import_materials: bool,
}

impl Default for ModelImporter {
    fn default() -> Self {
        Self {
            scale_factor: 1.0,
            convert_units: true,
            generate_colliders: false,
            normals: ModelImporterNormals::Import,
            smoothing_angle: 60.0,
            swap_uvs: false,
            generate_lightmap_uvs: true,
            animation_type: ModelImporterAnimationType::Generic,
            import_animation: true,
            loop_time: true,
            import_materials: true,
        }
    }
}

/// Unity `AudioSampleRateSetting`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioSampleRateSetting {
    #[default]
    PreserveSampleRate,
    /// The lowest rate that keeps the clip's content (Unity analyses it; here: 22050 Hz for
    /// mono clips, 44100 Hz otherwise, never above the original).
    OptimizeSampleRate,
    OverrideSampleRate,
}

/// Unity Audio Clip import settings.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(default)]
pub struct AudioImporter {
    /// Mix stereo down to mono.
    pub force_to_mono: bool,
    /// With Force To Mono: scale the mix so its peak is at full level.
    pub normalize: bool,
    pub sample_rate_setting: AudioSampleRateSetting,
    /// Hz, for Override Sample Rate.
    pub sample_rate: u32,
    /// Decode the clip when it loads instead of on first play (Unity "Preload Audio Data").
    pub preload_audio_data: bool,
}

impl Default for AudioImporter {
    fn default() -> Self {
        Self {
            force_to_mono: false,
            normalize: true,
            sample_rate_setting: AudioSampleRateSetting::PreserveSampleRate,
            sample_rate: 44100,
            preload_audio_data: true,
        }
    }
}

impl AudioImporter {
    /// The sample rate to resample to, if any.
    pub fn target_rate(&self, original: u32, mono: bool) -> Option<u32> {
        let r = match self.sample_rate_setting {
            AudioSampleRateSetting::PreserveSampleRate => return None,
            AudioSampleRateSetting::OverrideSampleRate => self.sample_rate.clamp(8000, 192_000),
            AudioSampleRateSetting::OptimizeSampleRate => {
                if mono {
                    22050
                } else {
                    44100
                }
            }
        };
        (r < original || self.sample_rate_setting == AudioSampleRateSetting::OverrideSampleRate)
            .then_some(r)
            .filter(|&r| r != original)
    }
}

/// Applies Model Import Settings to a freshly imported model and its sub-assets.
pub fn apply_model_settings(server: &mut AssetServer, h: &Handle<Model>, imp: &ModelImporter) {
    let Some(mut model) = server.models.get(h).cloned() else {
        return;
    };
    let scale = if imp.scale_factor.is_finite() && imp.scale_factor > 0.0 {
        imp.scale_factor
    } else {
        1.0
    };
    let mut meshes: Vec<Handle<crate::mesh::Mesh>> = Vec::new();
    for n in &mut model.nodes {
        n.position *= scale;
        if let Some(m) = &n.mesh
            && !meshes.contains(m)
        {
            meshes.push(m.clone());
        }
        if !imp.import_materials {
            for mat in &mut n.materials {
                *mat = Handle::from_path(crate::server::DEFAULT_MATERIAL);
            }
        }
    }
    for mh in &meshes {
        let Some(mesh) = server.meshes.get_mut(mh) else {
            continue;
        };
        if scale != 1.0 {
            for v in &mut mesh.vertices {
                *v *= scale;
            }
            for b in &mut mesh.bindposes {
                b.w_axis.x *= scale;
                b.w_axis.y *= scale;
                b.w_axis.z *= scale;
            }
            mesh.recalculate_bounds();
        }
        if imp.swap_uvs && !mesh.uv2.is_empty() {
            std::mem::swap(&mut mesh.uv, &mut mesh.uv2);
        }
        if imp.normals == ModelImporterNormals::Calculate {
            mesh.recalculate_normals_with_angle(imp.smoothing_angle);
        }
        if imp.generate_lightmap_uvs && mesh.bindposes.is_empty() && !mesh.has_lightmap_uvs() {
            mesh.generate_lightmap_uvs();
        }
    }
    if !imp.import_animation || imp.animation_type == ModelImporterAnimationType::None {
        model.animations.clear();
    }
    for a in &model.animations {
        if let Some(clip) = server.animations.get_mut(a) {
            clip.looping = imp.loop_time;
            if scale != 1.0 {
                for c in &mut clip.curves {
                    if let Some(t) = c.position.as_mut() {
                        for v in &mut t.values {
                            *v *= scale;
                        }
                    }
                }
            }
        }
    }
    let path = h.path().to_owned();
    server.models.insert(path, model);
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_core::reflect::Reflect as _;

    #[test]
    fn model_settings_scale_and_strip() {
        let dir = std::env::temp_dir().join(format!("oxide_imp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Assets")).unwrap();
        std::fs::write(
            dir.join("Assets/q.obj"),
            "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n",
        )
        .unwrap();
        let mut s = AssetServer::new(&dir);
        let imp = ModelImporter {
            scale_factor: 2.0,
            import_materials: false,
            ..Default::default()
        };
        s.set_importer("Assets/q.obj", &imp).unwrap();
        // Kept in the .meta next to the GUID-less defaults, and read back.
        let back: ModelImporter = s.importer("Assets/q.obj");
        assert_eq!(back, imp);
        let h = s.load_model("Assets/q.obj").unwrap();
        let m = s.models.get(&h).unwrap();
        let mesh = s.meshes.get(m.nodes[0].mesh.as_ref().unwrap()).unwrap();
        assert!(
            mesh.vertices.iter().any(|v| (v.x + 2.0).abs() < 1e-5),
            "{:?}",
            mesh.vertices
        );
        assert_eq!(
            m.nodes[0].materials[0].path(),
            crate::server::DEFAULT_MATERIAL
        );
        // Defaults aren't written: only changed fields are.
        let meta = std::fs::read_to_string(dir.join("Assets/q.obj.meta")).unwrap();
        assert!(
            meta.contains("scale_factor") && !meta.contains("smoothing_angle"),
            "{meta}"
        );
        let _ = ModelImporter::default().fields();
    }

    #[test]
    fn audio_target_rate() {
        let mut a = AudioImporter::default();
        assert_eq!(a.target_rate(48000, false), None);
        a.sample_rate_setting = AudioSampleRateSetting::OptimizeSampleRate;
        assert_eq!(a.target_rate(48000, true), Some(22050));
        assert_eq!(a.target_rate(22050, true), None);
        a.sample_rate_setting = AudioSampleRateSetting::OverrideSampleRate;
        a.sample_rate = 11025;
        assert_eq!(a.target_rate(44100, false), Some(11025));
    }
}
