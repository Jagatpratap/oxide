//! The Visual Effect component (Unity's `VisualEffect`): plays a `.vfx` asset, runs its
//! spawn contexts on the CPU and hands its systems to the renderer each frame.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@17.0/manual/VisualEffectComponent.html>

use crate::asset::VisualEffectAsset;
use crate::compile::{Compiled, compile};
use oxide_assets::{AssetId, AssetServer, Handle, MaterialProperty};
use oxide_core::Reflect;
use oxide_math::Vector4;
use oxide_render::VfxDraw;
use oxide_scene::{ComponentId, GameObjectId, World};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Per spawn block state.
#[derive(Clone, Debug, Default)]
struct SpawnState {
    acc: f32,
    fired: bool,
    next: f32,
    rate: f32,
    period_left: f32,
}

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct EffectState {
    started: bool,
    pub playing: bool,
    pub time: f32,
    /// Kill every particle on the next frame.
    reset: bool,
    seed: u32,
    rng: u32,
    spawn: HashMap<u32, SpawnState>,
    // This frame (read by extraction).
    frame_reset: bool,
    frame_dt: f32,
    frame_spawn: Vec<u32>,
    pub compiled: Option<Arc<Compiled>>,
    /// Particles spawned since the last Reinit (an upper bound of the alive count).
    pub spawned: u64,
    /// Updates so far (the renderer steps the GPU simulation once per update).
    step: u64,
}

/// Unity `VisualEffect`.
#[derive(Clone, Debug, Reflect)]
pub struct VisualEffect {
    pub asset: Option<Handle<VisualEffectAsset>>,
    /// Unity "Initial Event Name": `OnPlay` starts the effect when it appears; empty waits
    /// for `play()`.
    pub initial_event: String,
    /// Simulation speed (Unity `playRate`).
    pub play_rate: f32,
    /// Unity `pause`.
    pub pause: bool,
    pub random_seed: u32,
    /// A new seed each `play()` (Unity "Reseed On Play").
    pub reseed_on_play: bool,
    /// Exposed property overrides by reference (Unity's Properties section).
    pub properties: Vec<MaterialProperty>,
    #[reflect(skip)]
    #[doc(hidden)]
    pub state: EffectState,
}

impl Default for VisualEffect {
    fn default() -> Self {
        Self {
            asset: None,
            initial_event: "OnPlay".into(),
            play_rate: 1.0,
            pause: false,
            random_seed: 0,
            reseed_on_play: true,
            properties: Vec::new(),
            state: EffectState::default(),
        }
    }
}

impl oxide_scene::Component for VisualEffect {}

impl VisualEffect {
    pub fn new(asset: &str) -> Self {
        Self {
            asset: Some(Handle::from_path(asset.to_owned())),
            ..Default::default()
        }
    }

    /// Unity `Play` (the OnPlay event): spawners start over.
    pub fn play(&mut self) {
        self.state.started = true;
        self.state.playing = true;
        self.state.time = 0.0;
        self.state.spawn.clear();
        if self.reseed_on_play {
            self.state.rng = self
                .state
                .rng
                .wrapping_mul(1664525)
                .wrapping_add(1013904223)
                | 1;
            self.state.seed = self.state.rng;
        }
    }

    /// Unity `Stop` (the OnStop event): spawning stops, living particles finish.
    pub fn stop(&mut self) {
        self.state.started = true;
        self.state.playing = false;
    }

    /// Unity `Reinit`: kills every particle and restarts from the initial event.
    pub fn reinit(&mut self) {
        self.state.reset = true;
        self.state.spawned = 0;
        self.state.started = false;
    }

    /// Unity `SendEvent` for the built-in events.
    pub fn send_event(&mut self, name: &str) {
        match name {
            "OnPlay" => self.play(),
            "OnStop" => self.stop(),
            _ => {}
        }
    }

    pub fn is_playing(&self) -> bool {
        self.state.playing
    }

    /// Unity `SetVector4` (also SetVector2/3 and SetVector4 with colours).
    pub fn set_vector(&mut self, name: &str, v: Vector4) {
        match self.properties.iter_mut().find(|p| p.name == name) {
            Some(p) => p.value = v,
            None => self.properties.push(MaterialProperty {
                name: name.into(),
                value: v,
                texture: None,
            }),
        }
    }

    /// Unity `SetFloat`.
    pub fn set_float(&mut self, name: &str, v: f32) {
        self.set_vector(name, Vector4::new(v, 0.0, 0.0, 0.0));
    }

    /// Unity `SetBool`.
    pub fn set_bool(&mut self, name: &str, v: bool) {
        self.set_float(name, v as u32 as f32);
    }

    /// Unity `GetFloat` (the override, else the asset's default).
    pub fn get_vector(&self, name: &str) -> Option<Vector4> {
        if let Some(p) = self.properties.iter().find(|p| p.name == name) {
            return Some(p.value);
        }
        self.state
            .compiled
            .as_ref()?
            .properties
            .iter()
            .find(|p| p.reference == name)
            .map(|p| p.default)
    }

    /// Unity `ResetOverride`.
    pub fn reset_override(&mut self, name: &str) {
        self.properties.retain(|p| p.name != name);
    }
}

/// Compiled `.vfx` assets by path (recompiled when the asset changes).
#[derive(Default)]
pub struct VfxPrograms {
    cache: HashMap<String, (Option<u64>, Arc<Compiled>)>,
}

impl VfxPrograms {
    /// The compiled asset at `path`, loading and compiling it if needed.
    pub fn get(&mut self, assets: &mut AssetServer, path: &str) -> Option<Arc<Compiled>> {
        if !assets.data.contains(path)
            && let Err(e) = assets.ensure_loaded(path)
        {
            oxide_core::debug_log_warning!("{e}");
            return None;
        }
        let version = assets.data.version(path);
        if let Some((v, c)) = self.cache.get(path)
            && *v == version
        {
            return Some(c.clone());
        }
        let asset = assets
            .data
            .get(&Handle::<VisualEffectAsset>::from_path(path.to_owned()))?
            .clone();
        let name = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Visual Effect");
        let c = Arc::new(compile(&asset, name));
        for (n, e) in &c.errors {
            oxide_core::debug_log_warning!("Visual Effect {name} (node {n}): {e}");
        }
        for s in &c.systems {
            if let Some(t) = &s.texture
                && let Err(e) = assets.ensure_loaded(t)
            {
                oxide_core::debug_log_warning!("{e}");
            }
        }
        self.cache.insert(path.to_owned(), (version, c.clone()));
        Some(c)
    }
}

fn next_random(rng: &mut u32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    (*rng >> 8) as f32 / 16_777_216.0
}

/// Advances every Visual Effect by `dt` seconds (Play mode, and Edit mode in the editor,
/// where Unity plays effects in the Scene view too).
pub fn update(world: &mut World, dt: f32) {
    if !world.resources.contains::<VfxPrograms>() {
        world.resources.insert(VfxPrograms::default());
    }
    for (id, cid) in world.query_ids::<VisualEffect>() {
        let path = world
            .component::<VisualEffect>(id, cid)
            .and_then(|v| v.asset.as_ref().map(|h| h.path().to_owned()));
        let compiled = path.and_then(|p| {
            let mut programs = world.resources.remove::<VfxPrograms>().unwrap_or_default();
            let c = programs.get(world.resource_mut::<AssetServer>(), &p);
            world.resources.insert(programs);
            c
        });
        let Some(v) = world.component_mut::<VisualEffect>(id, cid) else {
            continue;
        };
        let st = &mut v.state;
        if st.rng == 0 {
            st.rng = (id.to_bits() as u32).wrapping_mul(2654435761) | 1;
            st.seed = if v.random_seed != 0 {
                v.random_seed
            } else {
                st.rng
            };
        }
        let changed = match (&st.compiled, &compiled) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, Some(_)) => true,
            _ => false,
        };
        st.compiled = compiled.clone();
        st.frame_reset = std::mem::take(&mut st.reset) || changed;
        if !st.started {
            st.started = true;
            let on = v.initial_event == "OnPlay";
            if on {
                v.play();
            }
        }
        let v = world
            .component_mut::<VisualEffect>(id, cid)
            .expect("still there");
        let dt = if v.pause {
            0.0
        } else {
            dt * v.play_rate.max(0.0)
        };
        let overrides: HashMap<String, f32> = v
            .properties
            .iter()
            .map(|p| (p.name.clone(), p.value.x))
            .collect();
        let st = &mut v.state;
        st.frame_dt = dt;
        st.frame_spawn.clear();
        st.step += 1;
        let Some(c) = compiled else { continue };
        let defaults: HashMap<String, f32> = c
            .properties
            .iter()
            .map(|p| (p.reference.clone(), p.default.x))
            .collect();
        let props = |name: &str| overrides.get(name).or(defaults.get(name)).copied();
        if st.playing {
            st.time += dt;
        }
        for sys in &c.systems {
            let mut n = 0u32;
            if st.playing && dt > 0.0 {
                for b in &sys.spawn {
                    let s = st.spawn.entry(b.id).or_default();
                    match b.kind.as_str() {
                        "Constant Spawn Rate" => {
                            s.acc += b.get("Rate", &props).max(0.0) * dt;
                        }
                        "Single Burst" => {
                            if !s.fired && st.time >= b.get("Delay", &props) {
                                s.fired = true;
                                s.acc += b.get("Count", &props).max(0.0);
                            }
                        }
                        "Periodic Burst" => {
                            let period = b.get("Delay", &props).max(0.01);
                            while st.time >= s.next {
                                s.acc += b.get("Count", &props).max(0.0);
                                s.next += period;
                            }
                        }
                        "Variable Spawn Rate" => {
                            s.period_left -= dt;
                            if s.period_left <= 0.0 {
                                let (lo, hi) =
                                    (b.get("Rate Min", &props), b.get("Rate Max", &props));
                                s.rate = lo + (hi - lo) * next_random(&mut st.rng);
                                let (pl, ph) =
                                    (b.get("Period Min", &props), b.get("Period Max", &props));
                                s.period_left =
                                    (pl + (ph - pl) * next_random(&mut st.rng)).max(0.01);
                            }
                            s.acc += s.rate.max(0.0) * dt;
                        }
                        _ => {}
                    }
                    let whole = s.acc.floor();
                    s.acc -= whole;
                    n = n.saturating_add(whole as u32);
                }
            }
            st.spawned += n as u64;
            st.frame_spawn.push(n);
        }
    }
}

/// Adds every Visual Effect's systems to the frame.
pub fn extract(world: &World, out: &mut Vec<VfxDraw>) {
    for (id, cid) in world.query_ids::<VisualEffect>() {
        let Some(v) = world.component::<VisualEffect>(id, cid) else {
            continue;
        };
        let Some(c) = &v.state.compiled else { continue };
        let model = world.local_to_world(id);
        // Property values: overrides, else the asset's defaults (colours linear).
        let mut values = vec![[0.0f32; 4]; c.properties.len()];
        for p in &c.properties {
            let raw = v
                .properties
                .iter()
                .find(|o| o.name == p.reference)
                .map_or(p.default, |o| o.value);
            let mut x = raw.to_array();
            if p.kind == oxide_shadergraph::PropertyType::Color {
                let l = oxide_math::Color::new(x[0], x[1], x[2], x[3]).linear();
                x = [l.r, l.g, l.b, x[3]];
            }
            if let Some(slot) = values.get_mut(p.slot as usize) {
                *slot = x;
            }
        }
        for (k, sys) in c.systems.iter().enumerate() {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            id.to_bits().hash(&mut h);
            cid.0.hash(&mut h);
            k.hash(&mut h);
            out.push(VfxDraw {
                key: h.finish(),
                program: sys.program.clone(),
                spawn: v.state.frame_spawn.get(k).copied().unwrap_or(0),
                reset: v.state.frame_reset,
                simulate: v.state.frame_dt > 0.0 || v.state.frame_reset,
                step: v.state.step,
                visible: true,
                dt: v.state.frame_dt,
                time: v.state.time,
                seed: v.state.seed,
                object_to_world: model,
                properties: values.clone(),
                texture: sys.texture.as_ref().map(|t| AssetId::from_path(t)),
            });
        }
    }
}

/// The effect a component id refers to (for tools).
pub fn effect_mut(world: &mut World, id: GameObjectId) -> Option<(&mut VisualEffect, ComponentId)> {
    let (_, cid) = world
        .query_ids::<VisualEffect>()
        .into_iter()
        .find(|(o, _)| *o == id)?;
    Some((world.component_mut::<VisualEffect>(id, cid)?, cid))
}
