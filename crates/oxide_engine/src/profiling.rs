//! Per-frame Profiler counters (Unity's Profiler modules): Rendering, Scripts, Physics,
//! Audio, Memory and Video.

use crate::Engine;
use oxide_assets::AssetServer;
use oxide_core::profiler::Profiler;

/// Records this frame's counters (call after rendering, before `Profiler::end_frame`).
pub fn record_counters(engine: &Engine) {
    if !Profiler::enabled() {
        return;
    }
    let s = engine.stats;
    for (name, v) in [
        ("Draw Calls", s.draw_calls as f64),
        ("Instances", s.instances as f64),
        ("Triangles", s.triangles as f64),
        ("Shadow Caster Draw Calls", s.shadow_draw_calls as f64),
        ("Cameras", s.cameras as f64),
        ("Culled", s.culled as f64),
        ("Occlusion Culled", s.occlusion_culled as f64),
        ("Reflection Probes", s.reflection_probes as f64),
        ("Probe Captures", s.probe_captures as f64),
    ] {
        Profiler::counter("Rendering", name, v);
    }
    let w = &engine.world;
    let objects = w.objects().count();
    let components: usize = w.objects().map(|(_, o)| o.component_count()).sum();
    Profiler::counter("Scripts", "GameObjects", objects as f64);
    Profiler::counter("Scripts", "Components", components as f64);
    Profiler::counter("Scripts", "FPS", engine.fps as f64);
    if let Some(p) = w.try_resource::<oxide_physics::Physics>() {
        let pw = &p.world;
        Profiler::counter("Physics", "Rigidbodies", pw.bodies.len() as f64);
        Profiler::counter(
            "Physics",
            "Active Rigidbodies",
            pw.islands.active_bodies().count() as f64,
        );
        Profiler::counter("Physics", "Colliders", pw.colliders.len() as f64);
        Profiler::counter("Physics", "Joints", pw.impulse_joints.len() as f64);
        let contacts = pw
            .narrow_phase
            .contact_pairs()
            .filter(|c| c.has_any_active_contact())
            .count();
        Profiler::counter("Physics", "Contacts", contacts as f64);
    }
    if let Some(a) = w.try_resource::<oxide_audio::Audio>() {
        Profiler::counter("Audio", "Playing Voices", a.active_voices() as f64);
        Profiler::counter("Audio", "Master Volume", a.master_volume as f64);
    }
    if let Some(assets) = w.try_resource::<AssetServer>() {
        let (mut tex_bytes, mut tex_count) = (0usize, 0usize);
        for (_, t) in assets.textures.iter() {
            tex_count += 1;
            tex_bytes += t.mips.iter().map(Vec::len).sum::<usize>();
        }
        let (mut mesh_bytes, mut mesh_count) = (0usize, 0usize);
        for (_, m) in assets.meshes.iter() {
            mesh_count += 1;
            mesh_bytes += m.vertices.len() * 64 + m.indices.len() * 4;
        }
        let audio_bytes: usize = assets.audio.iter().map(|(_, a)| a.bytes.len()).sum();
        let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
        Profiler::counter("Memory", "Texture Memory (MB)", mb(tex_bytes));
        Profiler::counter("Memory", "Textures", tex_count as f64);
        Profiler::counter("Memory", "Mesh Memory (MB)", mb(mesh_bytes));
        Profiler::counter("Memory", "Meshes", mesh_count as f64);
        Profiler::counter("Memory", "Audio Memory (MB)", mb(audio_bytes));
        Profiler::counter("Memory", "Materials", assets.materials.len() as f64);
        Profiler::counter(
            "Memory",
            "Total Asset Memory (MB)",
            mb(tex_bytes + mesh_bytes + audio_bytes),
        );
    }
    let videos = w
        .query::<oxide_video::VideoPlayer>()
        .filter(|(_, p)| p.status.is_playing)
        .count();
    Profiler::counter("Video", "Playing Videos", videos as f64);
}
