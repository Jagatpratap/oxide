use super::*;
use oxide_assets::PrimitiveType;
use oxide_math::{Matrix4x4Ext, Quaternion, glam::Vec3Swizzles};

fn object(
    p: PrimitiveType,
    pos: Vector3,
    scale: Vector3,
    albedo: Vector3,
    lightmapped: bool,
) -> BakeObject {
    let mut mesh = p.mesh();
    mesh.generate_lightmap_uvs();
    BakeObject {
        mesh: Arc::new(mesh),
        model: Matrix4x4::trs(pos, Quaternion::IDENTITY, scale),
        materials: vec![BakeMaterial {
            albedo,
            ..Default::default()
        }],
        receive_lightmap: lightmapped,
        scale_in_lightmap: 1.0,
    }
}

fn sun(direction: Vector3, baked: bool) -> BakeLight {
    BakeLight {
        kind: BakeLightKind::Directional {
            direction: direction.normalize(),
        },
        color: Vector3::splat(1.0),
        baked_direct: baked,
        cast_shadows: true,
        shadowmask_channel: None,
    }
}

fn input(objects: Vec<BakeObject>, lights: Vec<BakeLight>, probes: Vec<Vector3>) -> BakeInput {
    BakeInput {
        objects,
        lights,
        sky: SkyModel::new(&oxide_render::Environment::default(), None),
        ambient_intensity: 1.0,
        probes,
        settings: BakeSettings {
            texels_per_unit: 4.0,
            max_lightmap_size: 256,
            samples: 32,
            bounces: 1,
            padding: 2,
            denoise: true,
            directional: false,
        },
    }
}

/// Decoded lightmap value of object `oi` at world point `p` on its surface (searches its
/// triangles in the XZ plane — for floors).
fn floor_light(inp: &BakeInput, out: &BakeOutput, oi: usize, p: Vector3) -> Vector3 {
    let o = &inp.objects[oi];
    let m = &o.mesh;
    let (atlas, st) = out.placements[oi].expect("lightmapped");
    for t in m.indices.as_chunks::<3>().0 {
        let idx = [t[0], t[1], t[2]].map(|i| i as usize);
        let w = idx.map(|i| o.model.transform_point3(m.vertices[i]));
        // Only upward-facing triangles of the floor.
        if (w[1] - w[0]).cross(w[2] - w[0]).y.abs() < 1e-6 {
            continue;
        }
        let b = bary(Vector3::new(p.x, w[0].y, p.z), w);
        if b.min_element() < -1e-4 {
            continue;
        }
        let uv = m.uv2[idx[0]] * b.x + m.uv2[idx[1]] * b.y + m.uv2[idx[2]] * b.z;
        let a = Vector2::new(uv.x * st.x + st.z, uv.y * st.y + st.w);
        let lm = &out.lightmaps[atlas as usize];
        let x = (a.x * lm.size as f32) as u32;
        let y = ((1.0 - a.y) * lm.size as f32) as u32;
        let i = ((y * lm.size + x) * 4) as usize;
        return decode_rgbm([lm.rgbm[i], lm.rgbm[i + 1], lm.rgbm[i + 2], lm.rgbm[i + 3]]);
    }
    panic!("{p:?} is not on the floor");
}

#[test]
fn rgbm_round_trips() {
    for c in [0.0, 0.003, 0.05, 0.5, 1.0, 3.0, 7.5] {
        let v = Vector3::new(c, c * 0.5, c * 0.25);
        let back = decode_rgbm(encode_rgbm(v));
        assert!(
            (back - v).length() <= 0.02 + v.length() * 0.02,
            "{v:?} -> {back:?}"
        );
    }
}

#[test]
fn baked_sun_casts_shadows_into_the_lightmap() {
    let floor = object(
        PrimitiveType::Plane,
        Vector3::ZERO,
        Vector3::ONE,
        Vector3::splat(0.8),
        true,
    );
    let cube = object(
        PrimitiveType::Cube,
        Vector3::new(0.0, 0.5, 0.0),
        Vector3::ONE,
        Vector3::splat(0.8),
        false,
    );
    // Sun straight down: the cube's shadow is the square under it.
    let inp = input(vec![floor, cube], vec![sun(Vector3::NEG_Y, true)], vec![]);
    let out = bake(&inp, &Progress::default()).unwrap();
    assert_eq!(out.lightmaps.len(), 1);
    assert!(out.placements[1].is_none(), "non-receivers get no lightmap");
    let open = floor_light(&inp, &out, 0, Vector3::new(3.0, 0.0, 3.0));
    let under = floor_light(&inp, &out, 0, Vector3::new(0.0, 0.0, 0.0));
    // Open floor: the sun (1.0) plus sky; under the cube: no sun, little sky.
    assert!(open.x > 1.0, "open floor {open:?}");
    assert!(
        under.x < open.x * 0.3,
        "shadowed {under:?} vs open {open:?}"
    );
}

#[test]
fn light_bounces_and_carries_color() {
    let floor = object(
        PrimitiveType::Plane,
        Vector3::ZERO,
        Vector3::ONE,
        Vector3::splat(0.8),
        true,
    );
    // A red wall along x = 1, lit by a Mixed sun (no baked direct, only its bounce).
    let wall = object(
        PrimitiveType::Cube,
        Vector3::new(1.25, 1.5, 0.0),
        Vector3::new(0.5, 3.0, 6.0),
        Vector3::new(0.9, 0.05, 0.05),
        false,
    );
    let inp = input(
        vec![floor.clone(), wall],
        vec![sun(Vector3::new(1.0, -1.0, 0.0), false)],
        vec![],
    );
    let out = bake(&inp, &Progress::default()).unwrap();
    let near = floor_light(&inp, &out, 0, Vector3::new(0.6, 0.0, 0.0));
    let far = floor_light(&inp, &out, 0, Vector3::new(-4.5, 0.0, 0.0));
    let redness = |c: Vector3| c.x / c.z.max(1e-3);
    assert!(
        redness(near) > redness(far) * 1.3,
        "red bounce near the wall: near {near:?} far {far:?}"
    );
    // A Mixed light's direct light is not in the lightmap: the floor isn't sun-bright.
    assert!(far.x < 1.0, "{far:?}");
}

#[test]
fn probes_see_baked_light_and_shadow() {
    let floor = object(
        PrimitiveType::Plane,
        Vector3::ZERO,
        Vector3::ONE,
        Vector3::splat(0.8),
        false,
    );
    let roof = object(
        PrimitiveType::Cube,
        Vector3::new(0.0, 3.0, 0.0),
        Vector3::new(4.0, 0.2, 4.0),
        Vector3::splat(0.8),
        false,
    );
    let probes = vec![Vector3::new(0.0, 1.0, 0.0), Vector3::new(4.5, 1.0, 4.5)];
    let inp = input(vec![floor, roof], vec![sun(Vector3::NEG_Y, true)], probes);
    let out = bake(&inp, &Progress::default()).unwrap();
    // Light from above on an upward normal: evaluate the SH at +Y.
    let up = |sh: &[Vector3; 9]| {
        let y = sh_basis(Vector3::Y);
        sh.iter().zip(y).map(|(c, y)| *c * y).sum::<Vector3>()
    };
    let (under_roof, open) = (up(&out.probes[0]), up(&out.probes[1]));
    assert!(open.x > 0.9, "open probe gets the sun: {open:?}");
    assert!(
        under_roof.x < open.x * 0.5,
        "roof shadows the probe: {under_roof:?} vs {open:?}"
    );
    let _ = open.xz();
}

#[test]
fn cancel_stops_the_bake() {
    let floor = object(
        PrimitiveType::Plane,
        Vector3::ZERO,
        Vector3::ONE,
        Vector3::splat(0.8),
        true,
    );
    let p = Progress::default();
    p.cancel.store(true, Ordering::Relaxed);
    assert!(bake(&input(vec![floor], vec![], vec![]), &p).is_err());
}

#[test]
fn area_lights_bake_soft_one_sided_light() {
    let floor = || {
        object(
            PrimitiveType::Plane,
            Vector3::ZERO,
            Vector3::ONE,
            Vector3::splat(0.8),
            true,
        )
    };
    let area = |normal: Vector3, disc: bool| BakeLight {
        kind: BakeLightKind::Area {
            position: Vector3::new(0.0, 1.0, 0.0),
            right: Vector3::X,
            up: Vector3::Z,
            normal,
            disc,
            range: 20.0,
        },
        color: Vector3::splat(4.0),
        baked_direct: true,
        cast_shadows: true,
        shadowmask_channel: None,
    };
    // A 2 m square 1 m above the floor, facing down: its form factor there is ≈ 0.55.
    let mut inp = input(vec![floor()], vec![area(Vector3::NEG_Y, false)], vec![]);
    inp.ambient_intensity = 0.0;
    inp.settings.bounces = 0;
    let out = bake(&inp, &Progress::default()).unwrap();
    let under = floor_light(&inp, &out, 0, Vector3::ZERO);
    let far = floor_light(&inp, &out, 0, Vector3::new(4.0, 0.0, 4.0));
    assert!((1.5..3.0).contains(&under.x), "under the light {under:?}");
    assert!(far.x < under.x * 0.1, "falls off: {far:?} vs {under:?}");
    // Facing up, it lights nothing below (one-sided).
    let mut inp = input(vec![floor()], vec![area(Vector3::Y, true)], vec![]);
    inp.ambient_intensity = 0.0;
    inp.settings.bounces = 0;
    let out = bake(&inp, &Progress::default()).unwrap();
    let under = floor_light(&inp, &out, 0, Vector3::ZERO);
    assert!(under.x < 0.01, "back side {under:?}");
}

/// Texel of an extra atlas (shadowmask / directions) at world point `p` on the floor.
fn floor_texel(inp: &BakeInput, out: &BakeOutput, data: &[Vec<u8>], p: Vector3) -> [u8; 4] {
    let o = &inp.objects[0];
    let m = &o.mesh;
    let (atlas, st) = out.placements[0].expect("lightmapped");
    for t in m.indices.as_chunks::<3>().0 {
        let idx = [t[0], t[1], t[2]].map(|i| i as usize);
        let w = idx.map(|i| o.model.transform_point3(m.vertices[i]));
        if (w[1] - w[0]).cross(w[2] - w[0]).y.abs() < 1e-6 {
            continue;
        }
        let b = bary(Vector3::new(p.x, w[0].y, p.z), w);
        if b.min_element() < -1e-4 {
            continue;
        }
        let uv = m.uv2[idx[0]] * b.x + m.uv2[idx[1]] * b.y + m.uv2[idx[2]] * b.z;
        let size = out.lightmaps[atlas as usize].size;
        let a = Vector2::new(uv.x * st.x + st.z, uv.y * st.y + st.w);
        let (x, y) = (
            (a.x * size as f32) as u32,
            ((1.0 - a.y) * size as f32) as u32,
        );
        let i = ((y * size + x) * 4) as usize;
        let d = &data[atlas as usize];
        return [d[i], d[i + 1], d[i + 2], d[i + 3]];
    }
    panic!("{p:?} is not on the floor");
}

#[test]
fn shadowmask_and_directional_lightmaps() {
    let floor = object(
        PrimitiveType::Plane,
        Vector3::ZERO,
        Vector3::ONE,
        Vector3::splat(0.8),
        true,
    );
    let cube = object(
        PrimitiveType::Cube,
        Vector3::new(0.0, 0.5, 0.0),
        Vector3::ONE,
        Vector3::splat(0.8),
        false,
    );
    // A Mixed sun from +X and above: realtime direct, static shadows in channel 0.
    let mut mixed = sun(Vector3::new(-1.0, -2.0, 0.0), false);
    mixed.shadowmask_channel = Some(0);
    let mut inp = input(vec![floor, cube], vec![mixed], vec![]);
    inp.settings.directional = true;
    let out = bake(&inp, &Progress::default()).unwrap();
    assert_eq!(out.shadowmasks.len(), out.lightmaps.len());
    assert_eq!(out.directions.len(), out.lightmaps.len());
    // The cube's shadow falls toward -X.
    let shadowed = floor_texel(&inp, &out, &out.shadowmasks, Vector3::new(-1.0, 0.0, 0.0));
    let open = floor_texel(&inp, &out, &out.shadowmasks, Vector3::new(3.0, 0.0, 3.0));
    assert!(shadowed[0] < 30, "shadowed {shadowed:?}");
    assert!(open[0] > 225, "open {open:?}");
    // Unused channels stay unshadowed.
    assert!(shadowed[1] > 250 && shadowed[3] > 250);
    // Mixed lights aren't baked as direct light: the open floor gets only bounce and sky.
    let lit = floor_light(&inp, &out, 0, Vector3::new(3.0, 0.0, 3.0));
    assert!(lit.x < 1.0, "no direct light baked: {lit:?}");
    // Directions: mostly up (sky and bounce) on the open floor.
    let d = floor_texel(&inp, &out, &out.directions, Vector3::new(3.0, 0.0, 3.0));
    assert!(d[1] > 160, "dominant light from above: {d:?}");
    // Subtractive bakes the Mixed sun's direct light too.
    let mut inp2 = input(
        vec![inp.objects[0].clone(), inp.objects[1].clone()],
        vec![sun(Vector3::new(-1.0, -2.0, 0.0), true)],
        vec![],
    );
    inp2.settings.directional = false;
    let out2 = bake(&inp2, &Progress::default()).unwrap();
    let lit2 = floor_light(&inp2, &out2, 0, Vector3::new(3.0, 0.0, 3.0));
    assert!(lit2.x > lit.x + 0.5, "direct baked: {lit2:?} vs {lit:?}");
    assert!(out2.directions.is_empty() && out2.shadowmasks.is_empty());
}
