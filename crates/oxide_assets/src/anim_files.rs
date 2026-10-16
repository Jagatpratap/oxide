//! Animation asset files: `.anim` clips, `.controller` Animator Controllers, `.mask`
//! Avatar Masks and `.overrideController` Animator Override Controllers, as YAML.

use crate::animation::AnimationClip;
use crate::animator_controller::{AnimatorController, AnimatorOverrideController, AvatarMask};
use crate::handle::Handle;
use crate::server::AssetServer;
use std::path::Path;

fn stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Asset")
        .to_owned()
}

impl AssetServer {
    fn read_yaml<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let text = String::from_utf8(self.read_file(path)?).map_err(|e| format!("{path}: {e}"))?;
        serde_yaml_ng::from_str(&text).map_err(|e| format!("{path}: {e}"))
    }

    fn write_yaml<T: serde::Serialize>(&self, path: &str, value: &T) -> Result<(), String> {
        let text = serde_yaml_ng::to_string(value).map_err(|e| e.to_string())?;
        self.write_file(path, &text)
    }

    /// Loads (or reloads) an `.anim` clip.
    pub fn load_animation_clip(&mut self, path: &str) -> Result<Handle<AnimationClip>, String> {
        let mut clip: AnimationClip = self.read_yaml(path)?;
        if clip.name.is_empty() {
            clip.name = stem(path);
        }
        Ok(self.animations.insert(path.to_owned(), clip))
    }

    /// Writes `clip` to an `.anim` file and stores it there.
    pub fn save_animation_clip(
        &mut self,
        path: &str,
        clip: &AnimationClip,
    ) -> Result<Handle<AnimationClip>, String> {
        self.write_yaml(path, clip)?;
        Ok(self.animations.insert(path.to_owned(), clip.clone()))
    }

    /// Loads (or reloads) an Avatar Mask.
    pub fn load_avatar_mask(&mut self, path: &str) -> Result<Handle<AvatarMask>, String> {
        let mut m: AvatarMask = self.read_yaml(path)?;
        if m.name.is_empty() {
            m.name = stem(path);
        }
        Ok(self.masks.insert(path.to_owned(), m))
    }

    pub fn save_avatar_mask(
        &mut self,
        path: &str,
        m: &AvatarMask,
    ) -> Result<Handle<AvatarMask>, String> {
        self.write_yaml(path, m)?;
        let h = self.masks.insert(path.to_owned(), m.clone());
        // Controllers using the mask see the edit straight away.
        let users: Vec<String> = self
            .controllers
            .iter()
            .filter(|(_, c)| {
                c.layers.iter().any(|l| {
                    l.avatar_mask_asset
                        .as_ref()
                        .is_some_and(|a| a.path() == path)
                })
            })
            .map(|(p, _)| p.to_owned())
            .collect();
        for p in users {
            let ch: Handle<AnimatorController> = Handle::from_path(p);
            if let Some(c) = self.controllers.get_mut(&ch) {
                for l in &mut c.layers {
                    if l.avatar_mask_asset
                        .as_ref()
                        .is_some_and(|a| a.path() == path)
                    {
                        l.avatar_mask = Some(m.clone());
                    }
                }
            }
        }
        Ok(h)
    }

    /// Loads (or reloads) an Animator Controller, with the clips and masks it uses.
    pub fn load_animator_controller(
        &mut self,
        path: &str,
    ) -> Result<Handle<AnimatorController>, String> {
        let mut c: AnimatorController = self.read_yaml(path)?;
        if c.name.is_empty() {
            c.name = stem(path);
        }
        c.upgrade();
        self.load_controller_dependencies(path, &mut c);
        let h = self.controllers.insert(path.to_owned(), c);
        // Override controllers built on this one pick up the change.
        self.refresh_override_controllers(path);
        Ok(h)
    }

    fn load_controller_dependencies(&mut self, path: &str, c: &mut AnimatorController) {
        for clip in c.clips() {
            if let Err(e) = self.ensure_loaded(clip.path()) {
                oxide_core::debug_log_warning!("{path}: {e}");
            }
        }
        for l in &mut c.layers {
            if let Some(m) = l.avatar_mask_asset.clone() {
                if self.masks.handle_of(m.path()).is_none()
                    && let Err(e) = self.load_avatar_mask(m.path())
                {
                    oxide_core::debug_log_warning!("{path}: {e}");
                }
                l.avatar_mask = self.masks.get(&m).cloned();
            }
        }
    }

    /// Writes a controller (layers using a `.mask` asset keep the reference, not a copy).
    pub fn save_animator_controller(
        &mut self,
        path: &str,
        c: &AnimatorController,
    ) -> Result<Handle<AnimatorController>, String> {
        let mut file = c.clone();
        for l in &mut file.layers {
            if l.avatar_mask_asset.is_some() {
                l.avatar_mask = None;
            }
        }
        self.write_yaml(path, &file)?;
        // Keep the in-memory copy's masks resolved (the window may have just picked one).
        let mut c = c.clone();
        self.load_controller_dependencies(path, &mut c);
        let h = self.controllers.insert(path.to_owned(), c);
        self.refresh_override_controllers(path);
        Ok(h)
    }

    /// Loads an Animator Override Controller and stores the resolved controller (base
    /// with clips swapped) under its path in `controllers`.
    pub fn load_override_controller(
        &mut self,
        path: &str,
    ) -> Result<Handle<AnimatorController>, String> {
        let oc: AnimatorOverrideController = self.read_yaml(path)?;
        self.store_override_controller(path, oc)
    }

    pub fn save_override_controller(
        &mut self,
        path: &str,
        oc: &AnimatorOverrideController,
    ) -> Result<Handle<AnimatorController>, String> {
        self.write_yaml(path, oc)?;
        self.store_override_controller(path, oc.clone())
    }

    fn store_override_controller(
        &mut self,
        path: &str,
        oc: AnimatorOverrideController,
    ) -> Result<Handle<AnimatorController>, String> {
        let base = match &oc.controller {
            Some(b) => {
                if self.controllers.handle_of(b.path()).is_none() {
                    self.ensure_loaded(b.path())?;
                }
                self.controllers.get(b).cloned().unwrap_or_default()
            }
            None => AnimatorController::default(),
        };
        for o in &oc.overrides {
            if let Some(r) = &o.replacement
                && let Err(e) = self.ensure_loaded(r.path())
            {
                oxide_core::debug_log_warning!("{path}: {e}");
            }
        }
        let resolved = oc.resolve(&base, &stem(path));
        self.override_controllers.insert(path.to_owned(), oc);
        Ok(self.controllers.insert(path.to_owned(), resolved))
    }

    /// Re-resolves loaded override controllers whose base is `base_path`.
    fn refresh_override_controllers(&mut self, base_path: &str) {
        let dependants: Vec<(String, AnimatorOverrideController)> = self
            .override_controllers
            .iter()
            .filter(|(_, oc)| {
                oc.controller
                    .as_ref()
                    .is_some_and(|c| c.path() == base_path)
            })
            .map(|(p, oc)| (p.to_owned(), oc.clone()))
            .collect();
        for (p, oc) in dependants {
            let _ = self.store_override_controller(&p, oc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Interpolation, PropertyCurve, Track, TransformCurves};
    use crate::animator_controller::{ClipOverride, Motion};
    use oxide_math::Vector3;

    #[test]
    fn clips_controllers_masks_and_overrides_round_trip() {
        let dir = std::env::temp_dir().join(format!("oxide-anim-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = AssetServer::new(&dir);
        let mut walk = AnimationClip {
            name: "Walk".into(),
            looping: true,
            curves: vec![TransformCurves {
                path: "Hips".into(),
                position: Some(Track {
                    times: vec![0.0, 1.0],
                    values: vec![Vector3::ZERO, Vector3::Z],
                    interpolation: Interpolation::Linear,
                }),
                ..Default::default()
            }],
            properties: vec![PropertyCurve {
                path: "Lamp".into(),
                component: "Light".into(),
                property: "intensity".into(),
                curve: oxide_core::AnimationCurve::linear(0.0, 0.0, 2.0, 5.0),
            }],
            ..Default::default()
        };
        walk.recalculate_length();
        assert_eq!(walk.length, 2.0, "property curves count towards the length");
        a.save_animation_clip("Assets/Walk.anim", &walk).unwrap();
        a.save_animation_clip(
            "Assets/Run.anim",
            &AnimationClip {
                name: "Run".into(),
                ..Default::default()
            },
        )
        .unwrap();
        a.save_avatar_mask(
            "Assets/Upper.mask",
            &AvatarMask {
                name: "Upper".into(),
                transform_paths: vec!["Hips/Spine".into()],
                ..Default::default()
            },
        )
        .unwrap();

        let mut c = AnimatorController::from_clips(
            "Hero",
            &[("Walk".into(), Handle::from_path("Assets/Walk.anim"))],
        );
        let l = c.add_layer("Arms");
        l.avatar_mask_asset = Some(Handle::from_path("Assets/Upper.mask"));
        a.save_animator_controller("Assets/Hero.controller", &c)
            .unwrap();
        let text = std::fs::read_to_string(dir.join("Assets/Hero.controller")).unwrap();
        assert!(
            text.contains("Assets/Upper.mask") && !text.contains("Hips/Spine"),
            "mask saved by reference"
        );

        // A fresh server loads it all back, mask resolved.
        let mut b = AssetServer::new(&dir);
        b.ensure_loaded("Assets/Hero.controller").unwrap();
        let h: Handle<AnimatorController> = Handle::from_path("Assets/Hero.controller");
        let back = b.controllers.get(&h).unwrap().clone();
        assert_eq!(
            back.layers[1].avatar_mask.as_ref().unwrap().transform_paths,
            vec!["Hips/Spine"]
        );
        let wh: Handle<AnimationClip> = Handle::from_path("Assets/Walk.anim");
        assert_eq!(
            b.animations.get(&wh).unwrap().properties[0]
                .curve
                .evaluate(1.0),
            2.5
        );

        // Override controller: Walk → Run.
        let oc = AnimatorOverrideController {
            controller: Some(h.clone()),
            overrides: vec![ClipOverride {
                original: wh.clone(),
                replacement: Some(Handle::from_path("Assets/Run.anim")),
            }],
        };
        let oh = b
            .save_override_controller("Assets/Hero Pistol.overrideController", &oc)
            .unwrap();
        let resolved = b.controllers.get(&oh).unwrap();
        assert!(
            matches!(&resolved.layers[0].state_machine.states[0].motion, Motion::Clip(c) if c.path() == "Assets/Run.anim")
        );
        // Editing the base re-resolves the override.
        let mut edited = back.clone();
        edited.base().states[0].speed = 2.0;
        b.save_animator_controller("Assets/Hero.controller", &edited)
            .unwrap();
        assert_eq!(
            b.controllers.get(&oh).unwrap().layers[0]
                .state_machine
                .states[0]
                .speed,
            2.0
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
