//! ProBuilder tests: shapes are closed with outward faces, operations keep them that way,
//! and the component builds a mesh the renderer can draw.

use oxide_math::{Vector2, Vector3};
use oxide_probuilder::*;

/// Every edge used by exactly two faces, in opposite directions.
fn closed(m: &PbMesh) -> bool {
    let mut directed = std::collections::HashMap::new();
    for f in &m.faces {
        for (a, b) in f.edges() {
            *directed.entry((a, b)).or_insert(0) += 1;
        }
    }
    directed
        .iter()
        .all(|((a, b), n)| *n == 1 && directed.get(&(*b, *a)) == Some(&1))
}

/// Signed volume of the closed mesh (positive when faces point out).
fn volume(m: &PbMesh) -> f32 {
    let mut v = 0.0;
    for (fi, f) in m.faces.iter().enumerate() {
        let poly: Vec<Vector3> = f.indices.iter().map(|i| m.positions[*i as usize]).collect();
        for [a, b, c] in triangulate(&poly, m.face_normal(fi)) {
            v += poly[a].dot(poly[b].cross(poly[c])) / 6.0;
        }
    }
    v
}

#[test]
fn closed_shapes_are_watertight_and_outward() {
    for shape in ShapeType::ALL {
        if shape == ShapeType::Plane {
            continue;
        }
        let p = ShapeParams::new(shape);
        let m = p.build();
        let vol = volume(&m);
        if shape != ShapeType::Door {
            assert!(closed(&m), "{shape:?} is not closed");
        }
        assert!(vol > 0.0, "{shape:?} faces point inward (volume {vol})");
    }
    // The cube's volume is exactly its size.
    let m = ShapeParams {
        size: Vector3::new(2.0, 3.0, 4.0),
        ..ShapeParams::new(ShapeType::Cube)
    }
    .build();
    assert!((volume(&m) - 24.0).abs() < 1e-4);
    // Cylinder volume approaches pi r^2 h.
    let c = ShapeParams {
        sides: 64,
        ..ShapeParams::new(ShapeType::Cylinder)
    }
    .build();
    let expect = std::f32::consts::PI * 0.25 * 2.0;
    assert!(
        (volume(&c) - expect).abs() / expect < 0.01,
        "{}",
        volume(&c)
    );
}

fn top_face(m: &PbMesh) -> usize {
    (0..m.faces.len())
        .find(|f| m.face_normal(*f).y > 0.99)
        .expect("a top face")
}

#[test]
fn extrude_inset_subdivide_keep_the_mesh_closed() {
    let mut m = shapes::cube(Vector3::ONE);
    let top = top_face(&m);
    m.extrude_faces(&[top], 1.0);
    assert_eq!(m.faces.len(), 10);
    assert!(closed(&m));
    assert!(
        (volume(&m) - 2.0).abs() < 1e-4,
        "extruded volume {}",
        volume(&m)
    );
    // The extruded face moved up by one.
    assert!((m.face_center(top).y - 1.5).abs() < 1e-5);
    // Extruding two adjacent faces as a group makes no wall between them.
    let mut g = shapes::cube(Vector3::ONE);
    g.subdivide_faces(&[top_face(&g)]);
    let tops: Vec<usize> = (0..g.faces.len())
        .filter(|f| g.face_normal(*f).y > 0.99)
        .collect();
    assert_eq!(tops.len(), 4);
    assert!(
        closed(&g),
        "subdivided cube stays watertight (neighbours get midpoints)"
    );
    let before = g.faces.len();
    g.extrude_faces(&tops[..2], 0.5);
    assert!(closed(&g));
    assert_eq!(
        g.faces.len(),
        before + 6,
        "6 boundary walls around two adjacent quads"
    );
    // Inset.
    let mut i = shapes::cube(Vector3::ONE);
    let t = top_face(&i);
    i.inset_faces(&[t], 0.3);
    assert!(closed(&i));
    assert_eq!(i.faces.len(), 10);
    assert!((volume(&i) - 1.0).abs() < 1e-4);
}

#[test]
fn merge_delete_flip_weld_detach() {
    let mut m = shapes::plane(2.0, 2.0, 2, 2);
    assert_eq!(m.faces.len(), 4);
    assert!(m.merge_faces(&[0, 1, 2, 3]));
    assert_eq!(m.faces.len(), 1);
    assert_eq!(m.faces[0].indices.len(), 8, "outer boundary of a 2x2 grid");
    // Flip reverses the normal.
    let n = m.face_normal(0);
    m.flip_faces(&[0]);
    assert!((m.face_normal(0) + n).length() < 1e-5);
    // Delete removes unused positions.
    let mut c = shapes::cube(Vector3::ONE);
    c.delete_faces(&[0]);
    assert_eq!(c.faces.len(), 5);
    assert_eq!(c.positions.len(), 8);
    // Weld: a detached copy put back welds onto the original corners.
    let mut c = shapes::cube(Vector3::ONE);
    let d = c.detach_faces(&[0]);
    assert_eq!(d.faces.len(), 1);
    assert_eq!(c.faces.len(), 5);
    let base = c.positions.len() as u32;
    c.positions.extend(d.positions.iter().copied());
    let mut f = d.faces[0].clone();
    for i in &mut f.indices {
        *i += base;
    }
    c.faces.push(f);
    c.weld_vertices(&[], 1e-4);
    assert_eq!(c.positions.len(), 8);
    assert!(closed(&c));
    // Collapse.
    let mut q = shapes::plane(1.0, 1.0, 1, 1);
    q.collapse_vertices(&[0, 1]);
    assert_eq!(q.faces[0].indices.len(), 3);
}

#[test]
fn build_makes_a_renderable_mesh() {
    let mut pb = ProBuilderMesh::from_shape(ShapeParams::new(ShapeType::Cube));
    // Two faces get a second material.
    pb.set_face_material(
        &[0, 1],
        oxide_assets::Handle::from_path("Assets/Red.mat".to_owned()),
    );
    assert!(pb.shape.is_none(), "editing drops the shape");
    let mesh = pb.mesh.build("cube", pb.slot_count());
    assert_eq!(mesh.vertices.len(), 24, "4 per face corner");
    assert_eq!(mesh.indices.len(), 36);
    assert_eq!(mesh.submeshes.len(), 2);
    assert_eq!(mesh.submeshes[1].len(), 12, "two faces, two triangles each");
    // Auto UVs tile with the face's world size.
    let max_u = mesh.uv.iter().map(|u| u.x.abs()).fold(0.0, f32::max);
    assert!((max_u - 0.5).abs() < 1e-5);
    let _ = Vector2::ZERO;
    // Smooth groups average normals across faces.
    let s = ShapeParams {
        smooth: true,
        ..ShapeParams::new(ShapeType::Cylinder)
    }
    .build();
    let sm = s.build("cyl", 1);
    let side_normals = sm.normals.iter().filter(|n| n.y.abs() < 1e-3).count();
    assert!(side_normals > 0);
}

#[test]
fn the_component_builds_its_mesh() {
    let mut w = oxide_scene::World::new();
    let dir = std::env::temp_dir().join(format!("oxide-pb-{}", std::process::id()));
    w.resources.insert(oxide_assets::AssetServer::new(&dir));
    install(&mut w);
    let id = create_shape(&mut w, ShapeParams::new(ShapeType::Stairs));
    let path = mesh_path(id);
    let filter = w.get_component::<oxide_scene::MeshFilter>(id).unwrap();
    assert_eq!(filter.mesh.as_ref().unwrap().path(), path);
    let tris = w
        .resource::<oxide_assets::AssetServer>()
        .meshes
        .get(filter.mesh.as_ref().unwrap())
        .unwrap()
        .indices
        .len();
    assert!(tris > 0);
    // Changing the shape rebuilds.
    w.get_component_mut::<ProBuilderMesh>(id)
        .unwrap()
        .shape
        .as_mut()
        .unwrap()
        .segments = 12;
    update(&mut w);
    let h = w
        .get_component::<oxide_scene::MeshFilter>(id)
        .unwrap()
        .mesh
        .clone()
        .unwrap();
    let more = w
        .resource::<oxide_assets::AssetServer>()
        .meshes
        .get(&h)
        .unwrap()
        .indices
        .len();
    assert!(more > tris, "{more} > {tris}");
    assert!(
        w.get_component::<oxide_physics::MeshCollider>(id)
            .unwrap()
            .mesh
            .is_some()
    );
}

#[test]
fn component_round_trips_through_reflection() {
    use oxide_core::reflect::Reflect;
    use oxide_probuilder::{ProBuilderMesh, ShapeParams, ShapeType};
    let mut pb = ProBuilderMesh::from_shape(ShapeParams::new(ShapeType::Stairs));
    pb.edited();
    let mut m = oxide_probuilder::PbMesh::default();
    for (name, value) in pb.mesh.fields() {
        m.set_field(name, value)
            .unwrap_or_else(|e| panic!("mesh.{name}: {e}"));
    }
    let mut f = oxide_probuilder::Face::new(vec![]);
    for (name, value) in pb.mesh.faces[0].fields() {
        f.set_field(name, value)
            .unwrap_or_else(|e| panic!("face.{name}: {e}"));
    }
    let mut back = ProBuilderMesh::from_mesh(Default::default());
    for (name, value) in pb.fields() {
        back.set_field(name, value)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    assert_eq!(back.mesh, pb.mesh);
}
