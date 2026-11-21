use super::super::weights::{self, Body, FaceInput, Params};
use super::*;
use glam::{Mat3, Quat};
use std::f32::consts::TAU;

fn body_volumes() -> Vec<Mat4> {
    vec![
        Mat4::from_scale_rotation_translation(Vec3::new(0.15, 0.15, 0.75), Quat::IDENTITY, Vec3::new(0.0, 0.0, 0.75)),
        Mat4::from_scale_rotation_translation(Vec3::splat(0.1), Quat::IDENTITY, Vec3::new(0.0, 0.0, 1.6)),
    ]
}

fn skirt() -> ClothMesh {
    let seg = 24u32;
    let rings: Vec<(f32, f32)> = (0..=10).map(|t| (0.155 + 0.2 * (t as f32 / 10.0), 1.0 - 0.05 * t as f32)).collect();
    let mut pos = Vec::new();
    for &(r, z) in &rings {
        for i in 0..seg {
            let a = TAU * i as f32 / seg as f32;
            pos.push(Vec3::new(r * a.cos(), r * a.sin(), z));
        }
    }
    let mut idx = Vec::new();
    for ring in 0..rings.len() as u32 - 1 {
        for i in 0..seg {
            let (a, b) = (ring * seg + i, ring * seg + (i + 1) % seg);
            idx.extend_from_slice(&[a, b, b + seg, a, b + seg, a + seg]);
        }
    }
    let rigid = vec![false; pos.len()];
    let w = weights::compute(
        &[FaceInput {
            positions: &pos,
            indices: &idx,
            rigid: &rigid,
        }],
        Some(&Body::new(body_volumes())),
        &Params::default(),
    );
    let skin = vec![
        vec![
            NodeSkin {
                joints: [0; 4],
                weights: [1.0, 0.0, 0.0, 0.0],
            };
            pos.len()
        ];
        1
    ];
    // the skirt's own normals, out from the legs
    let normals = vec![pos.iter().map(|p| Vec3::new(p.x, p.y, 0.0).normalize_or_zero()).collect()];
    ClothMesh::new(
        w,
        &BindData {
            skin,
            normals,
            ..Default::default()
        },
    )
}

fn volumes(at: Mat4) -> Vec<Volume> {
    body_volumes().into_iter().filter_map(|m| Volume::new(at * m)).collect()
}

fn run(mesh: &ClothMesh, state: &mut ClothState, at: Mat4, seconds: f32) {
    let steps = (seconds * 60.0) as usize;
    for _ in 0..steps {
        state.step(mesh, &[at], &volumes(at), at, Vec3::ZERO, 1.0 / 60.0);
    }
}

/// The back half of the hem (walking +x, it is the half free to trail: the
/// front one meets the legs, which the backstop keeps it out of).
fn hem(mesh: &ClothMesh) -> Vec<usize> {
    (0..mesh.nodes.len())
        .filter(|&n| mesh.nodes[n].z < 0.51 && mesh.nodes[n].x < -0.1)
        .collect()
}

#[test]
fn a_skirt_at_rest_stays_close_to_its_shape() {
    let mesh = skirt();
    let mut s = ClothState::default();
    run(&mesh, &mut s, Mat4::IDENTITY, 3.0);
    for n in 0..mesh.nodes.len() {
        let off = (s.x[n] - mesh.nodes[n]).length();
        assert!(off < 0.12, "node {n} strayed {off}");
        if mesh.free[n] == 0.0 {
            assert_eq!(off, 0.0, "pinned node {n} moved");
        }
    }
    assert!(s.x.iter().all(|p| p.is_finite()));
}

#[test]
fn the_hem_trails_when_the_avatar_moves_and_comes_back_when_it_stops() {
    let mesh = skirt();
    let mut s = ClothState::default();
    run(&mesh, &mut s, Mat4::IDENTITY, 2.0);
    // walk +x at 1.5 m/s for half a second
    let mut at = Mat4::IDENTITY;
    for _ in 0..30 {
        at = Mat4::from_translation(at.w_axis.truncate() + Vec3::X * 0.025);
        s.step(&mesh, &[at], &volumes(at), at, Vec3::ZERO, 1.0 / 60.0);
    }
    let lag: f32 = hem(&mesh)
        .iter()
        .map(|&n| (s.x[n] - at.transform_point3(mesh.nodes[n])).x)
        .sum::<f32>()
        / hem(&mesh).len() as f32;
    assert!(lag < -0.01, "the hem trails behind: {lag}");
    run(&mesh, &mut s, at, 4.0);
    let after: f32 = hem(&mesh)
        .iter()
        .map(|&n| (s.x[n] - at.transform_point3(mesh.nodes[n])).x)
        .sum::<f32>()
        / hem(&mesh).len() as f32;
    assert!(after.abs() < 0.02, "settled back: {after}");
}

#[test]
fn nothing_ends_inside_the_body() {
    let mesh = skirt();
    let mut s = ClothState::default();
    // spin the avatar fast: the cloth wraps around the legs
    for k in 0..120 {
        let at = Mat4::from_rotation_z(k as f32 * 0.12);
        s.step(&mesh, &[at], &volumes(at), at, Vec3::ZERO, 1.0 / 60.0);
        for &n in &mesh.moving {
            for v in volumes(at) {
                let local = v.inv.transform_point3(s.x[n as usize]);
                assert!(local.length() >= 1.0, "node {n} inside a volume at frame {k}");
                let target = s.target[n as usize];
                assert!(
                    v.push_out(s.x[n as usize], target)
                        .is_none_or(|p| (p - s.x[n as usize]).length() < 0.02),
                    "node {n} within the skin gap at frame {k}"
                );
            }
        }
    }
}

#[test]
fn a_teleport_restarts_at_the_skinned_pose() {
    let mesh = skirt();
    let mut s = ClothState::default();
    run(&mesh, &mut s, Mat4::IDENTITY, 1.0);
    let far = Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0));
    s.step(&mesh, &[far], &volumes(far), far, Vec3::ZERO, 1.0 / 60.0);
    for n in 0..mesh.nodes.len() {
        assert!((s.x[n] - far.transform_point3(mesh.nodes[n])).length() < 1e-4);
    }
}

#[test]
fn the_gpu_rebuilds_the_simulated_place_from_the_deltas() {
    let mesh = skirt();
    let mut s = ClothState::default();
    let at = Mat4::from_scale_rotation_translation(Vec3::splat(1.1), Quat::from_rotation_z(1.0), Vec3::new(5.0, 2.0, 21.0));
    run(&mesh, &mut s, at, 0.5);
    let walk = Mat4::from_translation(Vec3::X * 0.2) * at;
    s.step(&mesh, &[walk], &volumes(walk), walk, Vec3::ZERO, 1.0 / 60.0);
    let mut out = Vec::new();
    s.deltas(&mesh, &mut out);
    assert_eq!(out.len(), mesh.vertex_count());
    let mut moved = 0;
    for (v, &node) in mesh.node_of[0].iter().enumerate() {
        let d = Vec3::new(out[v][0], out[v][1], out[v][2]);
        // the shader: skin matrix × (bind position + delta)
        let drawn = walk.transform_point3(mesh.nodes[node as usize] + d);
        let want = if mesh.free[node as usize] > 0.0 {
            s.target[node as usize] + s.view()[node as usize]
        } else {
            walk.transform_point3(mesh.nodes[node as usize])
        };
        assert!((drawn - want).length() < 1e-4, "vertex {v}: {drawn} vs {want}");
        moved += (d.length() > 1e-3) as usize;
    }
    assert!(moved > 0);
}

#[test]
#[ignore]
fn timing_30k() {
    let seg = 300u32;
    let rings: Vec<(f32, f32)> = (0..=100)
        .map(|t| (0.155 + 0.25 * (t as f32 / 100.0), 1.0 - 0.0055 * t as f32))
        .collect();
    let mut pos = Vec::new();
    for &(r, z) in &rings {
        for i in 0..seg {
            let a = TAU * i as f32 / seg as f32;
            pos.push(Vec3::new(r * a.cos(), r * a.sin(), z));
        }
    }
    let mut idx = Vec::new();
    for ring in 0..rings.len() as u32 - 1 {
        for i in 0..seg {
            let (a, b) = (ring * seg + i, ring * seg + (i + 1) % seg);
            idx.extend_from_slice(&[a, b, b + seg, a, b + seg, a + seg]);
        }
    }
    let rigid = vec![false; pos.len()];
    let t0 = std::time::Instant::now();
    let w = weights::compute(
        &[FaceInput {
            positions: &pos,
            indices: &idx,
            rigid: &rigid,
        }],
        Some(&Body::new(body_volumes())),
        &Params::default(),
    );
    let weigh = t0.elapsed();
    let skin = vec![
        vec![
            NodeSkin {
                joints: [0; 4],
                weights: [1.0, 0.0, 0.0, 0.0]
            };
            pos.len()
        ];
        1
    ];
    let mesh = ClothMesh::new(
        w,
        &BindData {
            skin,
            ..Default::default()
        },
    );
    // 26 volumes like the avatar's
    let vols: Vec<Volume> = (0..26)
        .map(|k| {
            Volume::new(Mat4::from_scale_rotation_translation(
                Vec3::splat(0.08),
                Quat::IDENTITY,
                Vec3::new(0.0, 0.0, 0.06 * k as f32),
            ))
            .unwrap()
        })
        .collect();
    let mut s = ClothState::default();
    s.step(&mesh, &[Mat4::IDENTITY], &vols, Mat4::IDENTITY, Vec3::ZERO, 1.0 / 60.0);
    let t1 = std::time::Instant::now();
    let mut out = Vec::new();
    for k in 0..60 {
        let at = Mat4::from_translation(Vec3::X * 0.02 * k as f32);
        s.step(&mesh, &[at], &vols, Mat4::IDENTITY, Vec3::ZERO, 1.0 / 60.0);
        out.clear();
        s.deltas(&mesh, &mut out);
    }
    println!(
        "nodes {} edges {} weigh {:?} step+deltas {:?}/frame",
        mesh.nodes.len(),
        mesh.edges.len(),
        weigh,
        t1.elapsed() / 60
    );
}

#[test]
fn a_quick_wiggle_never_takes_the_skirt_through_the_legs() {
    let mesh = skirt();
    let mut s = ClothState::default();
    run(&mesh, &mut s, Mat4::IDENTITY, 1.0);
    // ±12 cm side to side, three times a second, with a twist
    for k in 0..240 {
        let t = k as f32 / 60.0;
        let at = Mat4::from_rotation_translation(
            Quat::from_rotation_z(0.5 * (t * TAU * 3.0).sin()),
            Vec3::new(0.0, 0.12 * (t * TAU * 3.0).sin(), 0.0),
        );
        s.step(&mesh, &[at], &volumes(at), at, Vec3::ZERO, 1.0 / 60.0);
        for &n in &mesh.moving {
            let n = n as usize;
            let behind = (s.x[n] - s.target[n]).dot(s.normal[n]);
            assert!(behind >= -BACKSTOP - 1e-4, "node {n} {behind} behind its surface at frame {k}");
        }
    }
}

#[test]
fn normals_of_welded_vertices_are_averaged_and_skinned() {
    let mesh = skirt();
    assert!(mesh.normals.iter().all(|n| (n.length() - 1.0).abs() < 1e-4));
    let mut s = ClothState::default();
    let at = Mat4::from_rotation_z(1.0);
    s.step(&mesh, &[at], &volumes(at), at, Vec3::ZERO, 1.0 / 60.0);
    for n in 0..mesh.nodes.len() {
        let want = Mat3::from_rotation_z(1.0) * mesh.normals[n];
        assert!((s.normal[n] - want).length() < 1e-4);
    }
}

#[test]
fn a_mesh_uploaded_in_another_space_is_weighed_where_it_is_worn() {
    // the same skirt stored at 100x scale, 50 m away, and brought onto the
    // skeleton by its inverse bind (as some uploads are)
    let mesh = skirt();
    let to_bind = Mat4::from_scale_rotation_translation(Vec3::splat(100.0), Quat::IDENTITY, Vec3::new(50.0, 0.0, 0.0));
    let inverse_bind = to_bind.inverse();
    let bind: Vec<Vec3> = mesh.nodes.iter().map(|&p| to_bind.transform_point3(p)).collect();
    let moved = ClothMesh {
        nodes: bind,
        normals: mesh.normals.clone(),
        ..ClothMesh::new(Default::default(), &BindData::default())
    };
    let moved = ClothMesh {
        free: mesh.free.clone(),
        skin: mesh.skin.clone(),
        edges: mesh.edges.clone(),
        node_of: mesh.node_of.clone(),
        moving: mesh.moving.clone(),
        ..moved
    };
    let mut a = ClothState::default();
    let mut b = ClothState::default();
    for k in 0..90 {
        let at = Mat4::from_translation(Vec3::X * 0.02 * k as f32);
        a.step(&mesh, &[at], &volumes(at), at, Vec3::ZERO, 1.0 / 60.0);
        b.step(&moved, &[at * inverse_bind], &volumes(at), at, Vec3::ZERO, 1.0 / 60.0);
    }
    for n in 0..mesh.nodes.len() {
        assert!((a.x[n] - b.x[n]).length() < 1e-3, "node {n}: {} vs {}", a.x[n], b.x[n]);
    }
}

#[test]
fn small_root_wobbles_do_not_twitch_the_hem() {
    let mesh = skirt();
    let mut s = ClothState::default();
    run(&mesh, &mut s, Mat4::IDENTITY, 1.0);
    // standing, the root nudged up to 1 cm each frame (simulator corrections)
    let mut seed = 12345u32;
    let mut rand = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
    };
    let mut last: Option<Vec<Vec3>> = None;
    let mut twitch = 0.0f32;
    for _ in 0..120 {
        let at = Mat4::from_translation(Vec3::new(rand(), rand(), rand() * 0.5) * 0.01);
        s.step(&mesh, &[at], &volumes(at), at, Vec3::ZERO, 1.0 / 60.0);
        let offsets: Vec<Vec3> = hem(&mesh).iter().map(|&n| s.x[n] - s.target[n]).collect();
        if let Some(prev) = &last {
            let change = offsets.iter().zip(prev).map(|(a, b)| (*a - *b).length()).sum::<f32>() / offsets.len() as f32;
            twitch = twitch.max(change);
        }
        last = Some(offsets);
    }
    println!("largest frame-to-frame hem twitch {twitch:.4} m");
    assert!(twitch < 0.008, "the hem twitches {twitch} m in a frame");
}

#[test]
fn upside_down_the_skirt_falls_and_upright_it_comes_back() {
    let mesh = skirt();
    let mut s = ClothState::default();
    run(&mesh, &mut s, Mat4::IDENTITY, 1.0);
    // a handstand: turned over about the waist, held two seconds
    let waist = Vec3::new(0.0, 0.0, 1.0);
    let over = Mat4::from_translation(waist) * Mat4::from_rotation_x(std::f32::consts::PI) * Mat4::from_translation(-waist);
    for _ in 0..120 {
        s.step(
            &mesh,
            &[over],
            &volumes(over),
            over,
            over.transform_vector3(Vec3::NEG_Z),
            1.0 / 60.0,
        );
    }
    let hem = hem(&mesh);
    // modelled, the hem would point up at the sky; it falls instead
    let fell = hem.iter().map(|&n| s.target[n].z - s.x[n].z).sum::<f32>() / hem.len() as f32;
    assert!(fell > 0.3, "the upturned hem fell {fell} m");
    assert!(s.x.iter().all(|p| p.is_finite()));
    // back on its feet, it hangs as modelled again
    run(&mesh, &mut s, Mat4::IDENTITY, 3.0);
    let back = hem.iter().map(|&n| (s.x[n] - s.target[n]).length()).sum::<f32>() / hem.len() as f32;
    assert!(back < 0.05, "upright again: {back} m off");
}

#[test]
fn standing_upright_the_tilt_changes_nothing() {
    let mesh = skirt();
    let mut s = ClothState::default();
    run(&mesh, &mut s, Mat4::IDENTITY, 2.0);
    assert!(s.tilt < 1e-4);
    let sag = hem(&mesh).iter().map(|&n| s.target[n].z - s.x[n].z).fold(0.0f32, f32::max);
    assert!(sag < 0.03, "no extra sag standing: {sag}");
}

#[test]
fn cloth_modelled_inside_a_volume_is_not_pushed_off_the_body() {
    // a volume bigger than the body the skirt was made for (an inflated
    // fit, or hair tight to the back): its waist sits 5 cm inside it
    let mesh = skirt();
    let fat = vec![
        Volume::new(Mat4::from_scale_rotation_translation(
            Vec3::new(0.24, 0.24, 0.8),
            Quat::IDENTITY,
            Vec3::new(0.0, 0.0, 0.75),
        ))
        .unwrap(),
    ];
    let mut s = ClothState::default();
    for _ in 0..120 {
        s.step(&mesh, &[Mat4::IDENTITY], &fat, Mat4::IDENTITY, Vec3::ZERO, 1.0 / 60.0);
    }
    let puffed = (0..mesh.nodes.len())
        .map(|n| (s.x[n] - s.target[n]).length())
        .fold(0.0f32, f32::max);
    assert!(puffed < 0.05, "pushed {puffed} m off its modelled shape");
}

#[test]
fn a_long_strand_keeps_its_length_when_it_lags() {
    // a tail: a thin ribbon pinned at the top, the body bouncing up and
    // down (along its length: a lagging tail is squashed, not bent)
    let mut pos = Vec::new();
    let mut idx = Vec::new();
    for k in 0..30u32 {
        let z = 1.0 - 0.025 * k as f32;
        pos.push(Vec3::new(-0.3, -0.01, z));
        pos.push(Vec3::new(-0.3, 0.01, z));
        if k > 0 {
            let (a, b, c, d) = (2 * k - 2, 2 * k - 1, 2 * k, 2 * k + 1);
            idx.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    let rigid = vec![false; pos.len()];
    let w = weights::compute(
        &[FaceInput {
            positions: &pos,
            indices: &idx,
            rigid: &rigid,
        }],
        None,
        &Params::default(),
    );
    let mesh = ClothMesh::new(
        w,
        &BindData {
            skin: vec![vec![
                NodeSkin {
                    joints: [0; 4],
                    weights: [1.0, 0.0, 0.0, 0.0]
                };
                pos.len()
            ]],
            ..Default::default()
        },
    );
    let length = |x: &[Vec3]| {
        (0..29)
            .map(|k| (x[mesh.node_of[0][2 * k + 2] as usize] - x[mesh.node_of[0][2 * k] as usize]).length())
            .sum::<f32>()
    };
    let rest = length(&mesh.nodes);
    let mut s = ClothState::default();
    let mut shortest = f32::MAX;
    for k in 0..180 {
        let at = Mat4::from_translation(Vec3::new(0.0, 0.0, 0.25 * (k as f32 * 0.35).sin()));
        s.step(&mesh, &[at], &[], at, Vec3::ZERO, 1.0 / 60.0);
        shortest = shortest.min(length(&s.x));
    }
    assert!(shortest > 0.85 * rest, "squashed to {shortest} of {rest}");
}

#[test]
fn an_animation_turning_the_skin_is_not_a_body_upside_down() {
    // the skin matrices turned over (a tail curled up by its animation), the
    // body upright: no extra gravity, the spring holds the animated shape
    let mesh = skirt();
    let mut s = ClothState::default();
    let curled = Mat4::from_rotation_x(2.5);
    for _ in 0..120 {
        s.step(&mesh, &[curled], &[], Mat4::IDENTITY, Vec3::NEG_Z, 1.0 / 60.0);
    }
    let off = (0..mesh.nodes.len())
        .map(|n| (s.x[n] - s.target[n]).length())
        .fold(0.0f32, f32::max);
    assert!(off < 0.05, "dragged {off} m off its animated shape");
}

#[test]
fn uneven_and_empty_frames_do_not_stutter() {
    // swinging, then frames of 0, 2 and 16 ms in turn (a high frame rate's
    // pacing). A stutter is the cloth jumping on the body: no frame of those
    // may move its offsets much further than a steady 60 fps frame does, nor
    // its path run much longer (back and forth) over the same time.
    let mesh = skirt();
    let swing = |t: f32| Mat4::from_translation(Vec3::new(0.15 * (t * 4.0).sin(), 0.0, 0.0));
    let measure = |frames: &[f32]| {
        let mut s = ClothState::default();
        let (mut t, mut length, mut jump) = (0.0f32, 0.0f32, 0.0f32);
        let mut last: Option<Vec<Vec3>> = None;
        while t < 6.0 {
            for &dt in frames {
                t += dt;
                let at = swing(t);
                s.step(&mesh, &[at], &volumes(at), at, Vec3::ZERO, dt);
                if t > 2.0
                    && let Some(prev) = &last
                {
                    for (a, b) in s.view().iter().zip(prev) {
                        length += (*a - *b).length();
                        jump = jump.max((*a - *b).length());
                    }
                }
                last = Some(s.view().to_vec());
            }
        }
        (length, jump)
    };
    let (steady, steady_jump) = measure(&[1.0 / 60.0]);
    let (uneven, uneven_jump) = measure(&[0.0, 0.002, 0.016]);
    // (the cloth steps at a fixed 60 Hz: a frame moves at most about one
    // step's offset, a little more where the uneven sampling of the body
    // swings it a little harder)
    assert!(
        uneven_jump <= steady_jump * 2.5,
        "a frame jumped {uneven_jump} m, steady {steady_jump} m"
    );
    assert!(uneven < steady * 1.6, "uneven frames: {uneven} m of offset path, steady {steady} m");
}

/// A wearer whose skeleton skins the mesh bigger than the default avatar the
/// weights were worked out on (a tail's joint offsets, a tall shape): the
/// cloth keeps the shape it is drawn with, not the default one's lengths
/// (the edges held to those squashed a tail's cards sideways).
#[test]
fn a_mesh_skinned_bigger_than_the_default_keeps_its_drawn_shape() {
    let mesh = skirt();
    let mut s = ClothState::default();
    let at = Mat4::from_scale(Vec3::new(1.4, 1.4, 1.0));
    for _ in 0..180 {
        s.step(&mesh, &[at], &[], Mat4::IDENTITY, Vec3::ZERO, 1.0 / 60.0);
    }
    let worst = (0..mesh.nodes.len())
        .map(|n| (s.x[n] - at.transform_point3(mesh.nodes[n])).length())
        .fold(0.0f32, f32::max);
    assert!(worst < 0.03, "the cloth drifted {worst} m from its drawn shape");
}

/// Flying fast, a long strand streams behind but keeps its length (held by
/// its edges alone it stretched to thrice it).
#[test]
fn a_long_strand_keeps_its_length_flying_fast() {
    // a strand of 30 cards' worth: 60 cm down from a pinned top
    let n = 31;
    let pos: Vec<Vec3> = (0..n).map(|k| Vec3::new(-0.2, 0.0, 1.6 - 0.02 * k as f32)).collect();
    let mut faces = Vec::new();
    let mut idx = Vec::new();
    for k in 0..n - 1 {
        faces.push(pos[k]);
        faces.push(pos[k + 1]);
        faces.push(pos[k] + Vec3::Y * 0.01);
        let b = (faces.len() - 3) as u32;
        idx.extend_from_slice(&[b, b + 1, b + 2]);
    }
    let rigid = vec![false; faces.len()];
    let w = weights::compute(
        &[FaceInput {
            positions: &faces,
            indices: &idx,
            rigid: &rigid,
        }],
        None,
        &Params::default(),
    );
    let skin = vec![
        vec![
            NodeSkin {
                joints: [0; 4],
                weights: [1.0, 0.0, 0.0, 0.0],
            };
            faces.len()
        ];
        1
    ];
    let mesh = ClothMesh::new(
        w,
        &BindData {
            skin,
            ..Default::default()
        },
    );
    let mut s = ClothState::default();
    let top = (0..mesh.nodes.len())
        .max_by(|&a, &b| mesh.nodes[a].z.total_cmp(&mesh.nodes[b].z))
        .unwrap();
    let tip = (0..mesh.nodes.len())
        .min_by(|&a, &b| mesh.nodes[a].z.total_cmp(&mesh.nodes[b].z))
        .unwrap();
    let rest = (mesh.nodes[tip] - mesh.nodes[top]).length();
    // flying +x at 20 m/s, turning now and then
    let mut at = Mat4::IDENTITY;
    let mut longest = 0.0f32;
    for f in 0..240 {
        let turn = Mat4::from_rotation_z(if f % 60 < 30 { 0.03 } else { -0.03 });
        at = Mat4::from_translation(Vec3::X * (20.0 / 60.0)) * at * turn;
        s.step(&mesh, &[at], &[], at, Vec3::ZERO, 1.0 / 60.0);
        longest = longest.max((s.x[tip] - s.x[top]).length());
    }
    assert!(longest < rest * 1.1, "the strand stretched to {longest} from {rest}");
}

/// The weather's wind blows the hem downwind and lets it flutter; the
/// waistband stays.
#[test]
fn the_wind_blows_the_hem_downwind() {
    let mesh = skirt();
    let mut calm = ClothState::default();
    run(&mesh, &mut calm, Mat4::IDENTITY, 3.0);
    let mut s = ClothState {
        wind: Vec3::new(0.0, 6.0, 0.0),
        ..Default::default()
    };
    let mut path = Vec::new();
    for _ in 0..240 {
        s.step(
            &mesh,
            &[Mat4::IDENTITY],
            &volumes(Mat4::IDENTITY),
            Mat4::IDENTITY,
            Vec3::ZERO,
            1.0 / 60.0,
        );
        let hem: Vec<usize> = hem(&mesh);
        path.push(hem.iter().map(|&n| s.x[n].y - calm.x[n].y).sum::<f32>() / hem.len() as f32);
    }
    let late = &path[120..];
    let mean = late.iter().sum::<f32>() / late.len() as f32;
    assert!(mean > 0.04, "the hem went {mean} m downwind");
    let spread = late.iter().copied().fold(f32::MIN, f32::max) - late.iter().copied().fold(f32::MAX, f32::min);
    assert!(spread > 0.002, "it flutters: {spread}");
    for n in 0..mesh.nodes.len() {
        if mesh.free[n] == 0.0 {
            assert_eq!(s.x[n], mesh.nodes[n], "pinned node {n} moved");
        }
        assert!(s.x[n].is_finite());
    }
}

/// At a low frame rate (8 fps, and the avatar turning and walking) the
/// cloth swings as it does at 60, not flung about by the body's jumps.
#[test]
fn a_low_frame_rate_does_not_fling_the_cloth() {
    let mesh = skirt();
    let mut worst = 0.0f32;
    let mut s = ClothState::default();
    let mut at = Mat4::IDENTITY;
    let mut last: Option<Vec<Vec3>> = None;
    for f in 0..40 {
        let dt = 1.0 / 8.0;
        let turn = if f % 10 < 5 { 2.5 } else { -2.5 };
        at = Mat4::from_translation(at.w_axis.truncate() + Vec3::X * 3.0 * dt)
            * Mat4::from_rotation_z(turn * dt)
            * Mat4::from_translation(-at.w_axis.truncate())
            * at;
        s.step(&mesh, &[at], &volumes(at), at, Vec3::ZERO, dt);
        let off: Vec<Vec3> = s.view().to_vec();
        if let Some(l) = &last {
            for (a, b) in off.iter().zip(l) {
                worst = worst.max((*a - *b).length());
            }
        }
        last = Some(off);
    }
    assert!(worst < 0.2, "the cloth jumped {worst} m from its skinned place between frames");
}

/// Flying face down, fast: hair lying along the back is held out behind by
/// the air rather than sagging under the tilt's gravity (without the drag
/// it sagged 10 cm).
#[test]
fn flying_fast_face_down_the_hair_is_held_out_behind() {
    let n = 31;
    let pos: Vec<Vec3> = (0..n).map(|k| Vec3::new(-0.2, 0.0, 1.6 - 0.02 * k as f32)).collect();
    let mut faces = Vec::new();
    let mut idx = Vec::new();
    for k in 0..n - 1 {
        faces.push(pos[k]);
        faces.push(pos[k + 1]);
        faces.push(pos[k] + Vec3::Y * 0.01);
        let b = (faces.len() - 3) as u32;
        idx.extend_from_slice(&[b, b + 1, b + 2]);
    }
    let rigid = vec![false; faces.len()];
    let w = weights::compute(
        &[FaceInput {
            positions: &faces,
            indices: &idx,
            rigid: &rigid,
        }],
        None,
        &Params::default(),
    );
    let skin = vec![
        vec![
            NodeSkin {
                joints: [0; 4],
                weights: [1.0, 0.0, 0.0, 0.0],
            };
            faces.len()
        ];
        1
    ];
    let mesh = ClothMesh::new(
        w,
        &BindData {
            skin,
            ..Default::default()
        },
    );
    let tip = (0..mesh.nodes.len())
        .min_by(|&a, &b| mesh.nodes[a].z.total_cmp(&mesh.nodes[b].z))
        .unwrap();
    // face down (the body turned over on its front: the hair lies along the
    // back, toward the feet, -x), flying +x at 12 m/s
    let mut s = ClothState::default();
    let pose = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let mut at = pose;
    for f in 0..180 {
        at = Mat4::from_translation(Vec3::X * (12.0 / 60.0) * f as f32) * pose;
        s.step(&mesh, &[at], &[], at, Vec3::NEG_X, 1.0 / 60.0);
    }
    let off = s.x[tip] - at.transform_point3(mesh.nodes[tip]);
    assert!(-off.z < 0.075, "sagged {} m (behind {} m)", -off.z, -off.x);
    assert!(off.x < 0.0, "pushed forward {} m", off.x);
}

/// A tail held to its capped freedom while animated swings further when
/// still (no animation to fight).
#[test]
fn a_still_tail_swings_further_than_an_animated_one() {
    let mut mesh = skirt();
    mesh.free_still = mesh.free.clone();
    for f in mesh.free.iter_mut() {
        *f = f.min(0.3);
    }
    let lag = |held: bool| {
        let mut s = ClothState {
            held,
            ..Default::default()
        };
        run(&mesh, &mut s, Mat4::IDENTITY, 1.0);
        let mut at = Mat4::IDENTITY;
        for _ in 0..30 {
            at = Mat4::from_translation(at.w_axis.truncate() + Vec3::X * 0.04);
            s.held = held;
            s.step(&mesh, &[at], &volumes(at), at, Vec3::ZERO, 1.0 / 60.0);
        }
        hem(&mesh)
            .iter()
            .map(|&n| (at.transform_point3(mesh.nodes[n]) - s.x[n]).x)
            .sum::<f32>()
            / hem(&mesh).len() as f32
    };
    let (held, still) = (lag(true), lag(false));
    assert!(still > held * 1.5, "still {still} m, held {held} m");
}

/// Under water the hem floats up and spreads, slowly; with the water
/// below it, nothing changes.
#[test]
fn under_water_the_cloth_floats() {
    let mesh = skirt();
    let hem_z = |s: &ClothState| hem(&mesh).iter().map(|&n| s.x[n].z - mesh.nodes[n].z).sum::<f32>() / hem(&mesh).len() as f32;
    let settle = |water: Option<f32>| {
        let mut s = ClothState {
            water,
            ..Default::default()
        };
        run(&mesh, &mut s, Mat4::IDENTITY, 4.0);
        s
    };
    let dry = settle(None);
    let shallow = settle(Some(0.0));
    let deep = settle(Some(3.0));
    assert!((hem_z(&shallow) - hem_z(&dry)).abs() < 1e-4, "water below changes nothing");
    assert!(hem_z(&deep) > hem_z(&dry) + 0.04, "floats up: {} vs {}", hem_z(&deep), hem_z(&dry));
    // slow: no hem node moves fast once floating
    let mut s = deep;
    let before = s.x.clone();
    s.step(
        &mesh,
        &[Mat4::IDENTITY],
        &volumes(Mat4::IDENTITY),
        Mat4::IDENTITY,
        Vec3::ZERO,
        1.0 / 60.0,
    );
    let fastest =
        s.x.iter()
            .zip(&before)
            .map(|(a, b)| (*a - *b).length() * 60.0)
            .fold(0.0f32, f32::max);
    assert!(fastest < 0.5, "drifts slowly: {fastest} m/s");
    assert!(s.x.iter().all(|p| p.is_finite()));
}

/// Under water the cloth near its pins floats too, not only its free ends.
#[test]
fn under_water_the_less_free_part_floats_too() {
    let mesh = skirt();
    // the nodes a little free on land, half way up the ramp at most
    let mid: Vec<usize> = (0..mesh.nodes.len())
        .filter(|&n| mesh.free[n] > 0.05 && mesh.free[n] < 0.5)
        .collect();
    assert!(!mid.is_empty());
    let rise = |water: Option<f32>| {
        let mut s = ClothState {
            water,
            ..Default::default()
        };
        run(&mesh, &mut s, Mat4::IDENTITY, 4.0);
        mid.iter().map(|&n| s.x[n].z - mesh.nodes[n].z).sum::<f32>() / mid.len() as f32
    };
    let (dry, wet) = (rise(None), rise(Some(3.0)));
    assert!(wet > dry + 0.02, "floats {wet} m against {dry} m on land");
}

/// A strand of 60 cm hanging from a pinned top (cards along it).
fn strand() -> ClothMesh {
    let n = 31;
    let pos: Vec<Vec3> = (0..n).map(|k| Vec3::new(-0.2, 0.0, 1.6 - 0.02 * k as f32)).collect();
    let mut faces = Vec::new();
    let mut idx = Vec::new();
    for k in 0..n - 1 {
        faces.push(pos[k]);
        faces.push(pos[k + 1]);
        faces.push(pos[k] + Vec3::Y * 0.01);
        let b = (faces.len() - 3) as u32;
        idx.extend_from_slice(&[b, b + 1, b + 2]);
    }
    let rigid = vec![false; faces.len()];
    let w = weights::compute(
        &[FaceInput {
            positions: &faces,
            indices: &idx,
            rigid: &rigid,
        }],
        None,
        &Params::default(),
    );
    let skin = vec![
        vec![
            NodeSkin {
                joints: [0; 4],
                weights: [1.0, 0.0, 0.0, 0.0],
            };
            faces.len()
        ];
        1
    ];
    ClothMesh::new(
        w,
        &BindData {
            skin,
            ..Default::default()
        },
    )
}

/// Under water a strand floats up without crumpling: once it has risen
/// (its first quick flip aside) it may curve, even over into a U as hair
/// rises above the head, but does not settle folded sharply back on
/// itself (a zigzag: the bunched hair seen in-world).
#[test]
fn under_water_a_strand_floats_without_crumpling() {
    let mesh = strand();
    // the strand's own line, top to tip (the cards' other corners aside)
    let mut chain: Vec<usize> = (0..mesh.nodes.len()).filter(|&n| mesh.nodes[n].y.abs() < 1e-4).collect();
    chain.sort_by(|&a, &b| mesh.nodes[b].z.total_cmp(&mesh.nodes[a].z));
    let mut s = ClothState {
        water: Some(3.0),
        ..Default::default()
    };
    let mut sharpest = 1.0f32;
    for f in 0..600 {
        let at = Mat4::from_translation(Vec3::X * (0.05 * (f as f32 * 0.03).sin()));
        s.step(&mesh, &[at], &[], at, Vec3::ZERO, 1.0 / 60.0);
        if f < 300 {
            continue;
        }
        for w in chain.windows(3) {
            let a = (s.x[w[1]] - s.x[w[0]]).normalize_or_zero();
            let b = (s.x[w[2]] - s.x[w[1]]).normalize_or_zero();
            sharpest = sharpest.min(a.dot(b));
        }
    }
    // (a cosine of the turn at one node: -0.5 is 120 degrees; settled
    // without the bends it folded right back, 172; with them the sharpest
    // is the turn up from the scalp, about 97)
    assert!(sharpest > -0.5, "folded at one node to an angle of cosine {sharpest}");
    assert!(s.x.iter().all(|p| p.is_finite()));
}

/// A tail does not float: under water it moves as on land.
#[test]
fn a_tail_does_not_float() {
    let mut mesh = skirt();
    mesh.floats = false;
    let settle = |water: Option<f32>| {
        let mut s = ClothState {
            water,
            ..Default::default()
        };
        run(&mesh, &mut s, Mat4::IDENTITY, 3.0);
        s.x
    };
    assert_eq!(settle(None), settle(Some(3.0)));
}

/// Under water the cloth fans out from the body, and stays out a while
/// once the body stops (no snap back into its dry shape).
#[test]
fn under_water_the_hem_fans_out_and_settles_slowly() {
    let mesh = skirt();
    let radius =
        |s: &ClothState| hem(&mesh).iter().map(|&n| Vec3::new(s.x[n].x, s.x[n].y, 0.0).length()).sum::<f32>() / hem(&mesh).len() as f32;
    let mut dry = ClothState::default();
    run(&mesh, &mut dry, Mat4::IDENTITY, 4.0);
    let mut wet = ClothState {
        water: Some(3.0),
        ..Default::default()
    };
    run(&mesh, &mut wet, Mat4::IDENTITY, 4.0);
    assert!(radius(&wet) > radius(&dry) + 0.02, "fans out: {} vs {}", radius(&wet), radius(&dry));
    // swim +x half a second, stop: the hem trails, then comes back slowly
    let mut at = Mat4::IDENTITY;
    for _ in 0..30 {
        at = Mat4::from_translation(at.w_axis.truncate() + Vec3::X * 0.02);
        wet.step(&mesh, &[at], &volumes(at), at, Vec3::ZERO, 1.0 / 60.0);
    }
    let lag = |s: &ClothState| {
        hem(&mesh)
            .iter()
            .map(|&n| (at.transform_point3(mesh.nodes[n]) - s.x[n]).x)
            .sum::<f32>()
            / hem(&mesh).len() as f32
    };
    let stopped = lag(&wet);
    for _ in 0..20 {
        wet.step(&mesh, &[at], &volumes(at), at, Vec3::ZERO, 1.0 / 60.0);
    }
    assert!(
        lag(&wet) > stopped * 0.5,
        "a third of a second later still {} of {}",
        lag(&wet),
        stopped
    );
    assert!(wet.x.iter().all(|p| p.is_finite()));
}
