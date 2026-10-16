//! End to end: Generate Lighting on a scene, then render it on the GPU. The cube's shadow
//! must come from the lightmap alone (the sun is Baked, so there is no realtime sun).

use oxide_assets::PrimitiveType;
use oxide_engine::{extract, lighting, objects};
use oxide_math::{Quaternion, QuaternionExt, Vector3};
use oxide_render::{
    CameraView, ClearMode, Gpu, ProjectionKind, SceneRenderer, capture_frame, wgpu,
};
use oxide_scene::{
    Light, LightProbeGroup, LightmapBakeType, MeshRenderer, RenderSettings, StaticEditorFlags,
    World,
};

#[test]
fn baked_lighting_end_to_end() {
    let dir = std::env::temp_dir().join(format!("oxide-bake-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Assets")).unwrap();
    let mut w = World::new();
    oxide_engine::init_world(&mut w, dir.clone());
    w.resource_mut::<RenderSettings>().lightmap_resolution = 6.0;
    w.resource_mut::<RenderSettings>().bake_samples = 24;

    let floor = objects::create_primitive(&mut w, PrimitiveType::Plane);
    let cube = objects::create_primitive(&mut w, PrimitiveType::Cube);
    w.set_position(cube, Vector3::new(0.0, 0.5, 0.0));
    for id in [floor, cube] {
        w.get_mut(id).unwrap().static_flags = StaticEditorFlags::EVERYTHING;
    }
    // A dynamic sphere (not static) lit by probes.
    let ball = objects::create_primitive(&mut w, PrimitiveType::Sphere);
    w.set_position(ball, Vector3::new(3.0, 1.0, 3.0));
    let probes = w.create("Light Probe Group");
    w.add_component(probes, LightProbeGroup::default());
    w.set_position(probes, Vector3::new(3.0, 1.0, 3.0));

    let sun = objects::create_directional_light(&mut w);
    // Pitched 50° down, facing -Z: the cube's shadow falls ~0.84 m toward the camera.
    w.set_rotation(sun, Quaternion::euler(50.0, 180.0, 0.0));
    w.get_component_mut::<Light>(sun)
        .unwrap()
        .lightmap_bake_type = LightmapBakeType::Baked;

    // Bake in the background, then apply.
    lighting::start(&mut w).unwrap();
    let started = std::time::Instant::now();
    let result = loop {
        if let Some(r) = lighting::poll(&mut w) {
            break r;
        }
        assert!(started.elapsed().as_secs() < 120, "bake took too long");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    result.unwrap();
    let rs = w.resource::<RenderSettings>();
    assert_eq!(rs.lightmaps.len(), 1);
    assert_eq!(rs.light_probe_positions.len(), 8);
    assert_eq!(
        w.get_component::<MeshRenderer>(floor)
            .unwrap()
            .lightmap_index,
        0
    );
    assert!(
        dir.join("Assets/Lighting/Lightmap-0_comp_light.png")
            .is_file()
    );

    let eye = Vector3::new(0.0, 6.0, -6.0);
    let cam = CameraView {
        position: eye,
        rotation: Quaternion::look_rotation((Vector3::ZERO - eye).normalize(), Vector3::Y),
        projection: ProjectionKind::Perspective { fov: 60.0 },
        near: 0.1,
        far: 100.0,
        viewport: [0.0, 0.0, 1.0, 1.0],
        clear: ClearMode::Sky,
        culling_mask: u32::MAX,
        occlusion_culling: false,
        antialiasing: Default::default(),
    };
    let frame = extract::extract(&w, Some(vec![cam.clone()]));
    assert!(
        frame.sun.is_none(),
        "a Baked sun isn't rendered in real time"
    );
    let ball_draw = frame.draws.iter().find(|d| d.id == ball.to_bits()).unwrap();
    assert!(ball_draw.light_probe.is_some() && ball_draw.lightmap.is_none());
    let floor_draw = frame
        .draws
        .iter()
        .find(|d| d.id == floor.to_bits())
        .unwrap();
    assert!(floor_draw.lightmap.is_some());

    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU; rendering part skipped");
        return;
    };
    let (width, height) = (160u32, 120u32);
    let mut r = SceneRenderer::new(&gpu);
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let assets = w.resource::<oxide_assets::AssetServer>();
    let img = capture_frame(&gpu, fmt, width, height, |view, _| {
        r.render(&gpu, assets, &frame, view, fmt, width, height);
    })
    .unwrap();
    if let Ok(d) = std::env::var("OXIDE_TEST_IMAGES") {
        std::fs::write(format!("{d}/baked.png"), img.to_png().unwrap()).unwrap();
    }
    let px = |p: Vector3| {
        let c = cam.view_projection(width as f32 / height as f32) * p.extend(1.0);
        let (x, y) = (
            ((c.x / c.w * 0.5 + 0.5) * width as f32) as u32,
            ((0.5 - c.y / c.w * 0.5) * height as f32) as u32,
        );
        let v = img.pixel(x, y);
        (v[0] as i32 + v[1] as i32 + v[2] as i32) / 3
    };
    // In the shadow in front of the cube vs open floor.
    let open = px(Vector3::new(3.5, 0.0, -2.5));
    let shadow = px(Vector3::new(0.0, 0.0, -1.0));
    assert!(
        open > 120,
        "open floor is sunlit through the lightmap: {open}"
    );
    // The sky fills the shadow and ACES compresses the bright floor, so the gap is ~45.
    assert!(shadow < open - 30, "baked shadow: {shadow} vs open {open}");
    let _ = std::fs::remove_dir_all(&dir);
}
