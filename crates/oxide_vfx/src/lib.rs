//! Oxide VFX Graph: Unity's Visual Effect Graph on GPU particles.
//!
//! A `.vfx` ([`VisualEffectAsset`]) holds contexts (Spawn → Initialize Particle → Update
//! Particle → Output Particle Quad) with stacks of blocks, operators wired into block
//! inputs and exposed properties. [`compile`] turns each system into compute and render
//! hooks for `oxide_render::vfx`; the [`VisualEffect`] component plays an asset, runs its
//! spawn blocks and sends its systems to the renderer.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@17.0/manual/index.html>

pub mod asset;
pub mod blocks;
pub mod compile;
pub mod runtime;

pub use asset::*;
pub use compile::{
    Compiled, PropertySlot, SpawnBlock, System as VfxSystem, compile, context_options,
    find_operator, ports, vfx_operators,
};
pub use runtime::{VfxPrograms, VisualEffect, effect_mut, extract, update};

use oxide_assets::{AssetServer, AssetTypeInfo};
use oxide_scene::World;

/// Registers `.vfx` (Create → Visual Effects → Visual Effect Graph).
pub fn register_assets(assets: &mut AssetServer) {
    assets.register_data_asset::<VisualEffectAsset>(
        AssetTypeInfo::new("VisualEffectAsset", &["vfx"])
            .menu("Visual Effects/Visual Effect Graph", "New VFX", 76)
            .color(240, 150, 220),
    );
}

/// Runs Visual Effects in Play mode.
#[derive(Default)]
pub struct VfxRunner;

impl oxide_scene::System for VfxRunner {
    fn name(&self) -> &'static str {
        "Visual Effects"
    }

    fn update(&mut self, world: &mut World) {
        let dt = world.resource::<oxide_core::Time>().delta_time();
        update(world, dt);
    }
}

/// Installs the Visual Effect component; returns the Play mode system.
pub fn install(world: &mut World) -> Box<dyn oxide_scene::System> {
    world
        .registry
        .register::<VisualEffect>("Effects/Visual Effect");
    if !world.resources.contains::<VfxPrograms>() {
        world.resources.insert(VfxPrograms::default());
    }
    Box::new(VfxRunner)
}
