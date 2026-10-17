//! Edge loops and rings, Insert Edge Loop, element rotate/scale, manual UVs, Boolean, Poly
//! Shape and Bezier Shape.

use oxide_math::{Quaternion, QuaternionExt, Vector2, Vector3};
use oxide_probuilder::csg::volume;
use oxide_probuilder::*;

fn grid() -> PbMesh {
    // 4×4 quads on the XZ plane.
    shapes::plane(4.0, 4.0, 4, 4)
}

fn find_edge(m: &PbMesh, a: Vector3, b: Vector3) -> (u32, u32) {
    let near = |p: Vector3| {
        m.positions
            .iter()
            .position(|q| (*q - p).length() < 1e-4)
            .unwrap() as u32
    };
    edge_key(near(a), near(b))
}

#[test]
fn rings_and_loops_on_a_grid() {
    let m = grid();
    // A vertical edge (along Z) in the middle column line x = 0, first row.
    let e = find_edge(
        &m,
        Vector3::new(0.0, 0.0, -2.0),
        Vector3::new(0.0, 0.0, -1.0),
    );
    // Its loop runs the whole line x = 0 (4 edges); its ring crosses the row (5 edges).
    let lp = m.edge_loop(e);
    assert_eq!(lp.len(), 4, "{lp:?}");
    assert!(
        lp.iter().all(|e| m.positions[e.0 as usize].x.abs() < 1e-4
            && m.positions[e.1 as usize].x.abs() < 1e-4)
    );
    let ring = m.edge_ring(e);
    assert_eq!(ring.len(), 5, "{ring:?}");
    assert!(ring.iter().all(|e| {
        let (a, b) = (m.positions[e.0 as usize], m.positions[e.1 as usize]);
        (a.z + 2.0).abs() < 1e-4 && (b.z + 1.0).abs() < 1e-4
            || (a.z + 1.0).abs() < 1e-4 && (b.z + 2.0).abs() < 1e-4
    }));
}

#[test]
fn insert_edge_loop_around_a_cube() {
    let mut m = shapes::cube(Vector3::ONE);
    let before = volume(&m);
    // A vertical edge: its ring goes around the four sides.
    let e = find_edge(
        &m,
        Vector3::new(-0.5, -0.5, -0.5),
        Vector3::new(-0.5, 0.5, -0.5),
    );
    let new = m.insert_edge_loop(e);
    assert_eq!(new.len(), 4);
    assert_eq!(m.faces.len(), 10);
    assert_eq!(m.positions.len(), 12);
    // Same solid, still closed: every edge has two faces.
    assert!((volume(&m) - before).abs() < 1e-4);
    assert!(m.edge_faces().values().all(|f| f.len() == 2));
    // The new loop is at half height.
    assert!(new.iter().all(|e| m.positions[e.0 as usize].y.abs() < 1e-4));
}

#[test]
fn rotate_and_scale_elements() {
    let mut m = shapes::cube(Vector3::ONE);
    let top: Vec<u32> = (0..m.positions.len() as u32)
        .filter(|v| m.positions[*v as usize].y > 0.0)
        .collect();
    let c = m.vertices_center(&top);
    m.scale_vertices(&top, c, Vector3::new(2.0, 1.0, 2.0));
    assert!(
        top.iter()
            .all(|v| (m.positions[*v as usize].x.abs() - 1.0).abs() < 1e-5)
    );
    m.rotate_vertices(&top, c, Quaternion::euler(0.0, 90.0, 0.0));
    assert!(
        top.iter()
            .all(|v| (m.positions[*v as usize].x.abs() - 1.0).abs() < 1e-4)
    );
    assert!((m.vertices_center(&top) - c).length() < 1e-5);
}

#[test]
fn manual_uvs() {
    let mut m = shapes::cube(Vector3::ONE);
    let auto = m.face_uvs(0);
    m.uv_transform(
        &[0],
        Vector2::new(0.5, 0.0),
        0.0,
        Vector2::ONE,
        Vector2::ZERO,
    );
    assert!(m.faces[0].has_manual_uv());
    for (a, b) in auto.iter().zip(m.face_uvs(0)) {
        assert!((b - *a - Vector2::new(0.5, 0.0)).length() < 1e-5);
    }
    // Rotating 90° about the center keeps the center.
    let c = m.uv_center(&[0]);
    m.uv_transform(&[0], Vector2::ZERO, 90.0, Vector2::ONE, c);
    assert!((m.uv_center(&[0]) - c).length() < 1e-5);
    // Fit fills 0–1; the built mesh uses the manual UVs.
    m.uv_fit(&[0]);
    let uvs = m.face_uvs(0);
    let max = uvs.iter().copied().fold(Vector2::ZERO, Vector2::max);
    assert!((max - Vector2::ONE).length() < 1e-5);
    let built = m.build("t", 1);
    assert!(built.uv[..4].iter().all(|uv| uvs.contains(uv)));
    // Geometry edits that change the corners fall back to Auto UVs.
    m.subdivide_faces(&[0]);
    assert!(
        m.faces
            .iter()
            .all(|f| !f.has_manual_uv() || f.manual_uv.len() == f.indices.len())
    );
    m.uv_reset_auto(&[1]);
    assert!(!m.faces[1].has_manual_uv());
    // Planar projection of the top and a side: one island in 0–1.
    m.uv_planar_project(&[2, 3]);
    let all: Vec<Vector2> = [2, 3].iter().flat_map(|f| m.face_uvs(*f)).collect();
    assert!(
        all.iter()
            .all(|uv| uv.x > -1e-4 && uv.x < 1.0001 && uv.y > -1e-4 && uv.y < 1.0001)
    );
}

#[test]
fn boolean_volumes() {
    let a = shapes::cube(Vector3::splat(2.0));
    let mut b = shapes::cube(Vector3::splat(2.0));
    for p in &mut b.positions {
        *p += Vector3::new(1.0, 0.0, 0.0);
    }
    // Two 2×2×2 cubes overlapping by 1×2×2.
    let union = boolean(&a, &b, BooleanOp::Union, 0);
    let sub = boolean(&a, &b, BooleanOp::Subtract, 0);
    let int = boolean(&a, &b, BooleanOp::Intersect, 1);
    assert!((volume(&union) - 12.0).abs() < 1e-3, "{}", volume(&union));
    assert!((volume(&sub) - 4.0).abs() < 1e-3, "{}", volume(&sub));
    assert!((volume(&int) - 4.0).abs() < 1e-3, "{}", volume(&int));
    // The intersection's faces come from both inputs; b's slots were shifted by 1.
    assert!(int.faces.iter().any(|f| f.material == 1) && int.faces.iter().any(|f| f.material == 0));
    // Results render.
    assert!(!union.build("u", 1).indices.is_empty());
}

#[test]
fn poly_and_bezier_shapes() {
    // An L-shaped outline (concave), 3 m high, drawn clockwise.
    let mut p = ShapeParams::new(ShapeType::PolyShape);
    p.points = vec![
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 2.0),
        Vector3::new(1.0, 0.0, 2.0),
        Vector3::new(1.0, 0.0, 1.0),
        Vector3::new(2.0, 0.0, 1.0),
        Vector3::new(2.0, 0.0, 0.0),
    ];
    p.size.y = 3.0;
    let m = p.build();
    assert!((volume(&m) - 9.0).abs() < 1e-4, "{}", volume(&m));
    // The other winding gives the same solid.
    p.points.reverse();
    assert!((volume(&p.build()) - 9.0).abs() < 1e-4);
    p.flip_normals = true;
    assert!((volume(&p.build()) + 9.0).abs() < 1e-4);

    // A straight Bezier tube approaches a cylinder's volume.
    let mut b = ShapeParams::new(ShapeType::BezierShape);
    b.points = vec![Vector3::ZERO, Vector3::new(4.0, 0.0, 0.0)];
    b.size = Vector3::splat(1.0);
    b.sides = 32;
    let m = b.build();
    let cyl = std::f32::consts::PI * 0.25 * 4.0;
    assert!(
        (volume(&m) - cyl).abs() / cyl < 0.02,
        "{} vs {cyl}",
        volume(&m)
    );
    assert!(m.edge_faces().values().all(|f| f.len() == 2), "closed");
    // A closed loop has no caps.
    let mut c = ShapeParams::new(ShapeType::BezierShape);
    c.closed = true;
    let m = c.build();
    assert!(m.edge_faces().values().all(|f| f.len() == 2));
    assert!(volume(&m) > 0.0);
}
