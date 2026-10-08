use super::*;
use crate::params::*;
use crate::sculpt::{generate_sculpt, sculpt_calc_mesh_resolution, sculpt_placeholder};

fn vp(profile: u8, path: u8) -> VolumeParams {
    VolumeParams {
        profile: ProfileParams {
            curve_type: profile,
            ..Default::default()
        },
        path: PathParams {
            curve_type: path,
            ..Default::default()
        },
        sculpt: None,
    }
}

fn box_params() -> VolumeParams {
    vp(LL_PCODE_PROFILE_SQUARE, LL_PCODE_PATH_LINE)
}

fn cylinder_params() -> VolumeParams {
    vp(LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PATH_LINE)
}

fn sphere_params() -> VolumeParams {
    vp(LL_PCODE_PROFILE_CIRCLE_HALF, LL_PCODE_PATH_CIRCLE)
}

fn torus_params() -> VolumeParams {
    let mut p = vp(LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PATH_CIRCLE);
    p.path.scale = [1.0, 0.25];
    p
}

fn check_mesh(m: &VolumeMesh) {
    for (fi, f) in m.faces.iter().enumerate() {
        assert_eq!(f.positions.len(), f.normals.len(), "face {fi}");
        assert_eq!(f.positions.len(), f.uvs.len(), "face {fi}");
        assert!(f.positions.len() <= 65535);
        assert_eq!(f.indices.len() % 3, 0, "face {fi}");
        assert!(!f.indices.is_empty(), "face {fi} has no triangles");
        for &i in &f.indices {
            assert!((i as usize) < f.positions.len(), "face {fi} index out of range");
        }
        for n in &f.normals {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-3, "face {fi} normal not unit: {n:?}");
        }
        for p in &f.positions {
            assert!(p.iter().all(|v| v.is_finite()), "face {fi} non-finite pos");
        }
        for uv in &f.uvs {
            assert!(uv.iter().all(|v| v.is_finite()), "face {fi} non-finite uv");
        }
    }
}

/// Fraction of triangles whose CCW geometric normal agrees with the
/// averaged vertex normals.
fn winding_agreement(f: &VolumeFace) -> f32 {
    let mut good = 0;
    let mut total = 0;
    for tri in f.indices.as_chunks::<3>().0 {
        let p: Vec<Vec3> = tri.iter().map(|&i| Vec3::from(f.positions[i as usize])).collect();
        let g = (p[1] - p[0]).cross(p[2] - p[0]);
        if g.length_squared() < 1e-12 {
            continue;
        }
        let n: Vec3 = tri.iter().map(|&i| Vec3::from(f.normals[i as usize])).sum();
        total += 1;
        if g.dot(n) > 0.0 {
            good += 1;
        }
    }
    if total == 0 { 1.0 } else { good as f32 / total as f32 }
}

#[test]
fn default_box() {
    let p = box_params();
    assert_eq!(num_faces(&p), 6);
    for &d in &DETAIL_SCALES {
        let m = generate_volume(&p, d);
        assert_eq!(m.faces.len(), 6);
        check_mesh(&m);
        let mut corners = [false; 8];
        for f in &m.faces {
            for v in &f.positions {
                for c in v {
                    assert!(c.abs() <= 0.5 + 1e-5, "{v:?}");
                }
                for (ci, corner) in corners.iter_mut().enumerate() {
                    let target = [
                        if ci & 1 != 0 { 0.5 } else { -0.5 },
                        if ci & 2 != 0 { 0.5 } else { -0.5 },
                        if ci & 4 != 0 { 0.5 } else { -0.5 },
                    ];
                    if (0..3).all(|k| (v[k] - target[k]).abs() < 1e-5) {
                        *corner = true;
                    }
                }
            }
            assert!(winding_agreement(f) > 0.99);
            // Outward normals: face normals point away from the center.
            let c: Vec3 = f.positions.iter().map(|&p| Vec3::from(p)).sum::<Vec3>() / f.positions.len() as f32;
            let n = Vec3::from(f.normals[0]);
            assert!(n.dot(c) > 0.4, "face normal {n:?} centroid {c:?}");
        }
        assert!(corners.iter().all(|&c| c), "missing corners {corners:?}");
        // LL face order: top cap (PATH_BEGIN), 4 sides, bottom cap.
        assert_eq!(m.faces[0].face_id, LL_FACE_PATH_BEGIN);
        assert_eq!(m.faces[1].face_id, LL_FACE_OUTER_SIDE_0);
        assert_eq!(m.faces[4].face_id, LL_FACE_OUTER_SIDE_3);
        assert_eq!(m.faces[5].face_id, LL_FACE_PATH_END);
        // Top cap is at +Z.
        assert!(m.faces[0].positions.iter().all(|p| (p[2] - 0.5).abs() < 1e-5));
        assert!(m.faces[5].positions.iter().all(|p| (p[2] + 0.5).abs() < 1e-5));
    }
}

#[test]
fn box_uvs_are_unit_square() {
    let m = generate_volume(&box_params(), 1.0);
    for f in &m.faces {
        for uv in &f.uvs {
            assert!(uv[0] >= -1e-5 && uv[0] <= 1.0 + 1e-5, "{uv:?}");
            assert!(uv[1] >= -1e-5 && uv[1] <= 1.0 + 1e-5, "{uv:?}");
        }
    }
}

#[test]
fn cylinder_faces() {
    let p = cylinder_params();
    assert_eq!(num_faces(&p), 3);
    for &d in &DETAIL_SCALES {
        let m = generate_volume(&p, d);
        assert_eq!(m.faces.len(), 3);
        check_mesh(&m);
        for f in &m.faces {
            assert!(winding_agreement(f) > 0.99);
            for v in &f.positions {
                // LL scales low-side-count n-gons up (tableScale: 6 sides -> 0.525).
                assert!((v[0] * v[0] + v[1] * v[1]).sqrt() <= 0.525 + 1e-4);
            }
        }
        // Side face normals are radial. (LL quirk: with a 2-point path the seam
        // is not S-wrapped, so seam vertices keep the facet normal, cos 30deg.)
        let side = &m.faces[1];
        for (p, n) in side.positions.iter().zip(&side.normals) {
            let r = Vec3::new(p[0], p[1], 0.0).normalize();
            assert!(r.dot(Vec3::from(*n)) > 0.86, "p {p:?} n {n:?}");
        }
    }
    // 24 sides at the highest LOD -> 25 profile points; split = 2 -> 4 path points.
    let m = generate_volume(&p, 4.0);
    assert_eq!(m.faces[1].positions.len(), 25 * 4);
}

#[test]
fn sphere_one_face_outward_normals() {
    let p = sphere_params();
    assert_eq!(num_faces(&p), 1);
    for &d in &DETAIL_SCALES {
        let m = generate_volume(&p, d);
        assert_eq!(m.faces.len(), 1);
        check_mesh(&m);
        let f = &m.faces[0];
        assert!(winding_agreement(f) > 0.95, "{}", winding_agreement(f));
        let mut outward = 0;
        for (p, n) in f.positions.iter().zip(&f.normals) {
            let pv = Vec3::from(*p);
            assert!(pv.length() <= 0.55);
            if pv.dot(Vec3::from(*n)) > 0.0 {
                outward += 1;
            }
        }
        assert_eq!(outward, f.positions.len(), "all sphere normals must point outward");
    }
}

#[test]
fn torus_one_face() {
    let p = torus_params();
    assert_eq!(num_faces(&p), 1);
    let m = generate_volume(&p, 2.5);
    assert_eq!(m.faces.len(), 1);
    check_mesh(&m);
    assert!(winding_agreement(&m.faces[0]) > 0.95);
}

#[test]
fn hollow_and_cut_face_counts() {
    let mut p = box_params();
    p.profile.hollow = 0.5;
    assert_eq!(num_faces(&p), 7);
    let m = generate_volume(&p, 1.0);
    assert_eq!(m.faces.len(), 7);
    check_mesh(&m);

    // Profile cut box: 3 sides + 2 caps + 2 cut faces.
    let mut p = box_params();
    p.profile.end = 0.75;
    assert_eq!(num_faces(&p), 7);
    let m = generate_volume(&p, 1.0);
    assert_eq!(m.faces.len(), 7);
    check_mesh(&m);

    // Cut + hollow box: 3 sides + 2 caps + inner + 2 cut faces.
    let mut p = box_params();
    p.profile.begin = 0.125;
    p.profile.end = 0.875;
    p.profile.hollow = 0.3;
    assert_eq!(num_faces(&p), 9);
    let m = generate_volume(&p, 1.0);
    assert_eq!(m.faces.len(), 9);
    check_mesh(&m);

    // Hollow cylinder: caps + outer + inner.
    let mut p = cylinder_params();
    p.profile.hollow = 0.5;
    assert_eq!(num_faces(&p), 4);

    // Prism (equilateral triangle): 2 caps + 3 sides.
    let p = vp(LL_PCODE_PROFILE_EQUALTRI, LL_PCODE_PATH_LINE);
    assert_eq!(num_faces(&p), 5);

    // Path-cut sphere gets caps.
    let mut p = sphere_params();
    p.path.end = 0.5;
    assert!(num_faces(&p) > 1);
}

#[test]
fn many_shapes_are_sane() {
    let profiles = [
        LL_PCODE_PROFILE_CIRCLE,
        LL_PCODE_PROFILE_SQUARE,
        LL_PCODE_PROFILE_ISOTRI,
        LL_PCODE_PROFILE_EQUALTRI,
        LL_PCODE_PROFILE_RIGHTTRI,
        LL_PCODE_PROFILE_CIRCLE_HALF,
    ];
    let holes = [
        LL_PCODE_HOLE_SAME,
        LL_PCODE_HOLE_CIRCLE,
        LL_PCODE_HOLE_SQUARE,
        LL_PCODE_HOLE_TRIANGLE,
    ];
    let paths = [
        LL_PCODE_PATH_LINE,
        LL_PCODE_PATH_CIRCLE,
        LL_PCODE_PATH_CIRCLE2,
        LL_PCODE_PATH_TEST,
        LL_PCODE_PATH_FLEXIBLE,
    ];
    for &pr in &profiles {
        for &h in &holes {
            for &pa in &paths {
                for variant in 0..4 {
                    let mut p = vp(pr | h, pa);
                    match variant {
                        0 => {}
                        1 => {
                            p.profile.hollow = 0.6;
                            p.profile.begin = 0.2;
                            p.profile.end = 0.8;
                        }
                        2 => {
                            p.path.twist_begin = -0.5;
                            p.path.twist_end = 1.0;
                            p.path.taper = [0.5, -0.3];
                            p.path.shear = [0.2, -0.1];
                            p.path.scale = [0.6, 0.3];
                            p.path.revolutions = 2.5;
                            p.path.skew = 0.3;
                            p.path.radius_offset = 0.4;
                        }
                        _ => {
                            p.path.begin = 0.1;
                            p.path.end = 0.7;
                            p.profile.hollow = 0.95;
                        }
                    }
                    for &d in &DETAIL_SCALES {
                        let m = generate_volume(&p, d);
                        assert_eq!(m.faces.len(), num_faces(&p), "{p:?}");
                        assert!(!m.faces.is_empty());
                        check_mesh(&m);
                    }
                }
            }
        }
    }
}

#[test]
fn degenerate_inputs_do_not_panic() {
    let mut p = box_params();
    p.profile.begin = f32::NAN;
    p.profile.end = f32::INFINITY;
    p.path.scale = [f32::NAN, -5.0];
    p.path.twist_end = f32::NAN;
    p.path.revolutions = 1e30;
    let _ = generate_volume(&p, f32::NAN);
    let _ = generate_volume(&p, 1e9);
    let _ = generate_volume(&p, -3.0);
    let _ = num_faces(&p);
    let mut p = box_params();
    p.profile.curve_type = 0x0f;
    p.path.curve_type = 0x00;
    let _ = generate_volume(&p, 1.0);
    let mut p = sphere_params();
    p.path.scale = [0.0, 0.0];
    p.profile.begin = 0.99;
    p.profile.end = 0.2;
    let m = generate_volume(&p, 4.0);
    check_mesh(&m);
}

fn sphere_map(size: u32, comps: u8) -> Vec<u8> {
    let mut px = Vec::with_capacity((size * size * comps as u32) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32;
            let v = y as f32 / (size - 1) as f32;
            let theta = v * std::f32::consts::PI;
            let phi = u * 2.0 * std::f32::consts::PI;
            let p = Vec3::new(theta.sin() * phi.cos(), theta.sin() * phi.sin(), -theta.cos()) * 0.5;
            let c = ((p + Vec3::splat(0.5)) * 255.0).round();
            px.push(c.x as u8);
            px.push(c.y as u8);
            px.push(c.z as u8);
            if comps == 4 {
                px.push(255);
            }
        }
    }
    px
}

#[test]
fn sculpt_sphere_map() {
    let mut p = sphere_params();
    p.sculpt = Some(SculptParams {
        texture: uuid::Uuid::nil(),
        sculpt_type: LL_SCULPT_TYPE_SPHERE,
    });
    for comps in [3u8, 4] {
        let px = sphere_map(64, comps);
        for &d in &DETAIL_SCALES {
            let m = generate_sculpt(&p, d, 64, 64, comps, &px);
            assert_eq!(m.faces.len(), 1);
            check_mesh(&m);
            let f = &m.faces[0];
            let mut maxr = 0.0f32;
            for v in &f.positions {
                let r = Vec3::from(*v).length();
                maxr = maxr.max(r);
                assert!(v.iter().all(|c| c.abs() <= 0.5 + 1e-5));
            }
            // Real map data (radius ~0.5), not the 0.3 placeholder.
            assert!(maxr > 0.45, "maxr {maxr}");
        }
    }
    // Invert / mirror flags still produce sane geometry.
    for flags in [
        LL_SCULPT_FLAG_INVERT,
        LL_SCULPT_FLAG_MIRROR,
        LL_SCULPT_FLAG_INVERT | LL_SCULPT_FLAG_MIRROR,
    ] {
        for kind in [
            LL_SCULPT_TYPE_SPHERE,
            LL_SCULPT_TYPE_TORUS,
            LL_SCULPT_TYPE_PLANE,
            LL_SCULPT_TYPE_CYLINDER,
        ] {
            p.sculpt = Some(SculptParams {
                texture: uuid::Uuid::nil(),
                sculpt_type: kind | flags,
            });
            let m = generate_sculpt(&p, 2.5, 64, 64, 3, &sphere_map(64, 3));
            assert_eq!(m.faces.len(), 1);
            check_mesh(&m);
        }
    }
}

#[test]
fn sculpt_resolution_matches_ll() {
    assert_eq!(sculpt_calc_mesh_resolution(64, 64, 4.0), (32, 32));
    assert_eq!(sculpt_calc_mesh_resolution(64, 64, 1.0), (6, 6));
    assert_eq!(sculpt_calc_mesh_resolution(0, 0, 2.5), (16, 16));
    // 128x32 map (ratio 4) at the highest LOD.
    let (s, t) = sculpt_calc_mesh_resolution(128, 32, 4.0);
    assert!(s * t <= 1024 && s >= 4 && t >= 4);
}

#[test]
fn sculpt_placeholder_and_bad_data() {
    let mut p = sphere_params();
    p.sculpt = Some(SculptParams {
        texture: uuid::Uuid::nil(),
        sculpt_type: LL_SCULPT_TYPE_SPHERE,
    });
    for &d in &DETAIL_SCALES {
        let m = sculpt_placeholder(&p, d);
        assert_eq!(m.faces.len(), 1);
        check_mesh(&m);
        for v in &m.faces[0].positions {
            assert!((Vec3::from(*v).length() - 0.3).abs() < 1e-4);
        }
    }
    // Truncated data, too few components, flat (degenerate) maps.
    let m = generate_sculpt(&p, 4.0, 64, 64, 3, &[1, 2, 3]);
    assert_eq!(m.faces.len(), 1);
    check_mesh(&m);
    let m = generate_sculpt(&p, 4.0, 64, 64, 1, &vec![0; 64 * 64]);
    assert_eq!(m.faces.len(), 1);
    let flat = vec![128u8; 64 * 64 * 3];
    let m = generate_sculpt(&p, 4.0, 64, 64, 3, &flat);
    assert_eq!(m.faces.len(), 1);
    for v in &m.faces[0].positions {
        assert!(
            (Vec3::from(*v).length() - 0.3).abs() < 1e-4,
            "flat map must fall back to placeholder"
        );
    }
    let m = generate_sculpt(&p, 1.0, 1, 1, 3, &[0, 0, 0]);
    check_mesh(&m);
}

#[test]
fn lod_triangle_counts_grow_with_detail() {
    use crate::params::*;
    // sphere: path circle, profile half circle
    let mut p = VolumeParams::default();
    p.path.curve_type = LL_PCODE_PATH_CIRCLE;
    p.profile.curve_type = LL_PCODE_PROFILE_CIRCLE_HALF;
    p.path.end = 1.0;
    p.profile.end = 1.0;
    p.path.scale = [1.0, 1.0];
    p.path.revolutions = 1.0;
    let c = super::lod_triangle_counts(&p);
    assert!(c[0] > 0 && c[0] < c[1] && c[1] < c[2] && c[2] < c[3], "{c:?}");
}
