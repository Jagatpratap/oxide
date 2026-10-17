//! Custom render pipelines (Unity's Scriptable Render Pipeline: `RenderPipeline.Render(context,
//! cameras)`). A script type implements [`RenderPipeline`] and is registered with
//! `scripts! { ...; pipelines: MyPipeline }`; a Universal Render Pipeline Asset whose Custom
//! Pipeline names it makes it the active pipeline. Each frame its `render` records commands on a
//! [`ScriptableRenderContext`] (clear, skybox, draw renderers with filtering and an optional
//! override material, terrain, effects, sprites, gizmos); the renderer runs them for every
//! camera instead of URP's own order. Post-processing still follows.
//! Reference: <https://docs.unity3d.com/Manual/srp-creating-simple-render-loop.html>

use oxide_assets::{Handle, Material};
use oxide_render::{CameraView, RenderQueue, RenderStep};
use oxide_scene::World;
use std::sync::Mutex;

/// Unity `FilteringSettings`: which renderers a `draw_renderers` call draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilteringSettings {
    pub queue: RenderQueue,
    pub layer_mask: u32,
}

impl FilteringSettings {
    /// Unity `RenderQueueRange.opaque`, every layer.
    pub fn opaque() -> Self {
        Self {
            queue: RenderQueue::Opaque,
            layer_mask: u32::MAX,
        }
    }
    /// Unity `RenderQueueRange.transparent`, every layer.
    pub fn transparent() -> Self {
        Self {
            queue: RenderQueue::Transparent,
            layer_mask: u32::MAX,
        }
    }
    pub fn all() -> Self {
        Self {
            queue: RenderQueue::All,
            layer_mask: u32::MAX,
        }
    }
    pub fn with_layer_mask(mut self, mask: u32) -> Self {
        self.layer_mask = mask;
        self
    }
}

/// Unity `ScriptableRenderContext`: records the frame's rendering commands.
pub struct ScriptableRenderContext<'a> {
    pub world: &'a World,
    steps: Vec<RenderStep>,
}

impl ScriptableRenderContext<'_> {
    /// Unity `CommandBuffer.ClearRenderTarget`: the camera's background and/or depth.
    pub fn clear_render_target(&mut self, color: bool, depth: bool) {
        self.steps
            .push(RenderStep::ClearRenderTarget { color, depth });
    }
    /// Unity `ScriptableRenderContext.DrawSkybox`: the sky behind what's drawn so far.
    pub fn draw_skybox(&mut self) {
        self.steps.push(RenderStep::DrawSkybox);
    }
    /// Unity `ScriptableRenderContext.DrawRenderers`.
    pub fn draw_renderers(&mut self, filter: FilteringSettings) {
        self.draw_renderers_with(filter, None);
    }
    /// `DrawRenderers` with `DrawingSettings.overrideMaterial`.
    pub fn draw_renderers_with(
        &mut self,
        filter: FilteringSettings,
        override_material: Option<&Handle<Material>>,
    ) {
        self.steps.push(RenderStep::DrawRenderers {
            queue: filter.queue,
            layer_mask: filter.layer_mask,
            override_material: override_material.map(Handle::id),
        });
    }
    pub fn draw_terrain(&mut self) {
        self.steps.push(RenderStep::DrawTerrain);
    }
    /// Particle Systems and Visual Effects.
    pub fn draw_effects(&mut self) {
        self.steps.push(RenderStep::DrawEffects);
    }
    /// Sprites and 3D text.
    pub fn draw_sprites(&mut self) {
        self.steps.push(RenderStep::DrawSprites);
    }
    /// Unity `ScriptableRenderContext.DrawGizmos`.
    pub fn draw_gizmos(&mut self) {
        self.steps.push(RenderStep::DrawGizmos);
    }
}

/// Unity's `RenderPipeline`: one instance renders every frame while it's active.
pub trait RenderPipeline: Send + 'static {
    fn render(&mut self, context: &mut ScriptableRenderContext, cameras: &[CameraView]);
}

type Factory = fn() -> Box<dyn RenderPipeline>;

/// Render pipeline types registered by scripts, and the running instance.
#[derive(Default)]
pub struct RenderPipelines {
    factories: Vec<(String, Factory)>,
    active: Mutex<Option<(String, Box<dyn RenderPipeline>)>>,
}

impl RenderPipelines {
    /// Registers `T` under its type name (what a pipeline asset's Custom Pipeline names).
    pub fn register<T: RenderPipeline + Default>(&mut self) {
        let full = std::any::type_name::<T>();
        let name = full.rsplit("::").next().unwrap_or(full).to_owned();
        self.factories.retain(|(n, _)| *n != name);
        self.factories.push((name, || Box::new(T::default())));
    }

    pub fn names(&self) -> Vec<String> {
        self.factories.iter().map(|(n, _)| n.clone()).collect()
    }

    /// Drops types (a Scripts library being reloaded) and the instance if it was one.
    pub fn remove(&mut self, names: &[String]) {
        self.factories.retain(|(n, _)| !names.contains(n));
        let active = self.active.get_mut().unwrap_or_else(|e| e.into_inner());
        if active.as_ref().is_some_and(|(n, _)| names.contains(n)) {
            *active = None;
        }
    }

    /// Runs pipeline `name` for this frame; `None` if no such type is registered.
    pub fn run(
        &self,
        name: &str,
        world: &World,
        cameras: &[CameraView],
    ) -> Option<Vec<RenderStep>> {
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if active.as_ref().is_none_or(|(n, _)| n != name) {
            let (_, make) = self.factories.iter().find(|(n, _)| n == name)?;
            *active = Some((name.to_owned(), make()));
        }
        let (_, pipeline) = active.as_mut().expect("set above");
        let mut ctx = ScriptableRenderContext {
            world,
            steps: Vec::new(),
        };
        pipeline.render(&mut ctx, cameras);
        Some(ctx.steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Toon {
        frames: u32,
    }

    impl RenderPipeline for Toon {
        fn render(&mut self, ctx: &mut ScriptableRenderContext, cameras: &[CameraView]) {
            self.frames += 1;
            ctx.clear_render_target(true, true);
            ctx.draw_renderers(FilteringSettings::opaque().with_layer_mask(1));
            ctx.draw_skybox();
            if !cameras.is_empty() {
                ctx.draw_gizmos();
            }
        }
    }

    #[test]
    fn registered_pipelines_record_steps_and_keep_their_state() {
        let mut p = RenderPipelines::default();
        p.register::<Toon>();
        assert_eq!(p.names(), vec!["Toon"]);
        let w = World::new();
        let steps = p.run("Toon", &w, &[]).unwrap();
        assert_eq!(
            steps,
            vec![
                RenderStep::ClearRenderTarget {
                    color: true,
                    depth: true
                },
                RenderStep::DrawRenderers {
                    queue: RenderQueue::Opaque,
                    layer_mask: 1,
                    override_material: None
                },
                RenderStep::DrawSkybox,
            ]
        );
        p.run("Toon", &w, &[]).unwrap();
        // One instance across frames (Unity keeps the RenderPipeline object).
        let active = p.active.lock().unwrap();
        assert_eq!(active.as_ref().map(|(n, _)| n.as_str()), Some("Toon"));
        drop(active);
        assert!(p.run("Missing", &w, &[]).is_none());
        p.remove(&["Toon".into()]);
        assert!(p.run("Toon", &w, &[]).is_none());
    }
}
