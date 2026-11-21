use super::*;
use std::f32::consts::TAU;

/// The test body: legs and torso as one tall ellipsoid, the head a sphere.
fn body() -> Vec<Mat4> {
    vec![
        Mat4::from_scale_rotation_translation(Vec3::new(0.15, 0.15, 0.75), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, 0.75)),
        Mat4::from_scale_rotation_translation(Vec3::splat(0.1), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, 1.6)),
    ]
}

/// One of the two halves of a flared skirt (`half` 0 or 1), as its own
/// mesh: its columns from one side seam to the other, both seams included
/// (the other half has them too: the seams of a garment in pieces).
fn half_skirt(half: u32) -> RestMesh {
    let (seg, rings) = (24u32, 11u32);
    let cols = seg / 2 + 1;
    let mut pos = Vec::new();
    for t in 0..rings {
        let (r, z) = (0.155 + 0.2 * (t as f32 / 10.0), 1.0 - 0.05 * t as f32);
        for c in 0..cols {
            let a = TAU * (half * seg / 2 + c) as f32 / seg as f32;
            pos.push(Vec3::new(r * a.cos(), r * a.sin(), z));
        }
    }
    let mut idx = Vec::new();
    for t in 0..rings - 1 {
        for c in 0..cols - 1 {
            let (a, b) = (t * cols + c, t * cols + c + 1);
            idx.extend_from_slice(&[a, b, b + cols, a, b + cols, a + cols]);
        }
    }
    let n = pos.len();
    RestMesh {
        bind: BindData {
            skin: vec![vec![
                NodeSkin {
                    joints: [0; 4],
                    weights: [1.0, 0.0, 0.0, 0.0],
                };
                n
            ]],
            positions: vec![pos.clone()],
            normals: vec![pos.iter().map(|p| Vec3::new(p.x, p.y, 0.0).normalize_or_zero()).collect()],
        },
        positions: vec![pos],
        indices: vec![idx],
        rigid: vec![vec![false; n]],
        animated: vec![vec![false; n]],
        on_tail: vec![vec![0.0; n]],
        body_like: false,
        tail: false,
        never: false,
        sway: false,
        hair: false,
        scalp: 0.0,
        joints: 1,
    }
}

/// A skirt in two pieces is weighed as one: where the pieces meet, their
/// vertices are one node, so they cannot part however it swings; each
/// piece's offsets are its own span of the group's.
#[test]
fn a_garment_in_pieces_is_welded_at_its_seams() {
    let (a, b) = (Arc::new(half_skirt(0)), Arc::new(half_skirt(1)));
    let g = weigh_group(Kind::Clothing, &[a.clone(), b.clone()], body(), None, Vec::new(), 0).expect("cloth");
    let cloth = &g.cloth;
    assert_eq!(cloth.node_of.len(), 2, "one face per piece");
    // the seams: each vertex of the second piece at a place of the first
    let mut seams = 0;
    for (v, p) in b.positions[0].iter().enumerate() {
        if let Some(u) = a.positions[0].iter().position(|q| (*q - *p).length() < 1e-5) {
            assert_eq!(cloth.node_of[0][u], cloth.node_of[1][v], "welded across the pieces at {p}");
            seams += 1;
        }
    }
    assert_eq!(seams, 2 * 11, "two seams down the skirt");
    // the second piece is skinned to its own joint (after the first's):
    // its seam vertices are exceptions, taken back through their own skin
    assert_eq!(cloth.exceptions.len(), seams);
    assert_eq!(
        g.spans,
        vec![(0, a.positions[0].len()), (a.positions[0].len(), b.positions[0].len())]
    );
    // swung hard, its offsets are drawn exactly where the nodes are
    let mut s = ClothState::default();
    let mut at = Mat4::IDENTITY;
    for f in 0..90 {
        let x = if f % 30 < 15 { 0.06 } else { -0.06 };
        at = Mat4::from_translation(at.w_axis.truncate() + Vec3::X * x);
        s.step(cloth, &[at, at], &[], at, Vec3::ZERO, 1.0 / 60.0);
    }
    assert!(s.largest_offset() > 0.01, "it swung: {}", s.largest_offset());
    let mut out = Vec::new();
    s.deltas(cloth, &mut out);
    let piece_a = &out[g.spans[0].0..g.spans[0].0 + g.spans[0].1];
    let piece_b = &out[g.spans[1].0..g.spans[1].0 + g.spans[1].1];
    // each seam drawn at one place by both pieces: they cannot part
    let drawn = |piece: &[[f32; 4]], rest: &RestMesh, v: usize| {
        let d = piece[v];
        at.transform_point3(rest.bind.positions[0][v] + Vec3::new(d[0], d[1], d[2]))
    };
    let mut moving_seams = 0;
    for (v, p) in b.positions[0].iter().enumerate() {
        if let Some(u) = a.positions[0].iter().position(|q| (*q - *p).length() < 1e-5) {
            let (da, db) = (drawn(piece_a, &a, u), drawn(piece_b, &b, v));
            assert!((da - db).length() < 1e-4, "seam at {p} drawn at {da} and {db}");
            moving_seams += (piece_b[v] != [0.0; 4]) as usize;
        }
    }
    assert!(moving_seams > 0, "the seams swing too");
}

/// A mermaid's tail: its top wraps the hips loosely and is rigged there to
/// the pelvis (joint 0), blending down into the tail's joint (1). Only what
/// the tail's joint carries moves: dragged fast through the air (flying),
/// its top stays on the hips instead of ballooning out behind, and its end
/// still streams.
#[test]
fn a_mer_tail_moves_only_where_rigged_to_the_tail() {
    let (seg, rings) = (24u32, 16u32);
    let mut pos = Vec::new();
    let mut share = Vec::new();
    for t in 0..rings {
        // 25 cm round at the hips (10 cm off the body), narrowing down
        let (r, z) = (0.25 - 0.01 * t as f32, 1.05 - 0.06 * t as f32);
        // the top four rings on the pelvis, then blending into the tail
        let s = ((t as f32 - 3.0) / 4.0).clamp(0.0, 1.0);
        for c in 0..seg {
            let a = TAU * c as f32 / seg as f32;
            pos.push(Vec3::new(r * a.cos(), r * a.sin(), z));
            share.push(s);
        }
    }
    let mut idx = Vec::new();
    for t in 0..rings - 1 {
        for c in 0..seg {
            let (a, b) = (t * seg + c, t * seg + (c + 1) % seg);
            idx.extend_from_slice(&[a, b, b + seg, a, b + seg, a + seg]);
        }
    }
    let n = pos.len();
    let skin: Vec<NodeSkin> = share
        .iter()
        .map(|&s| NodeSkin {
            joints: [0, 1, 0, 0],
            weights: [1.0 - s, s, 0.0, 0.0],
        })
        .collect();
    let tail = Arc::new(RestMesh {
        bind: BindData {
            skin: vec![skin],
            positions: vec![pos.clone()],
            normals: vec![pos.iter().map(|p| Vec3::new(p.x, p.y, 0.0).normalize_or_zero()).collect()],
        },
        positions: vec![pos.clone()],
        indices: vec![idx],
        rigid: vec![vec![false; n]],
        animated: vec![share.iter().map(|&s| s > 0.1).collect()],
        on_tail: vec![share.clone()],
        body_like: false,
        tail: true,
        never: false,
        sway: false,
        hair: false,
        scalp: 0.0,
        joints: 2,
    });
    let g = weigh_group(Kind::Tail, &[tail], body(), None, Vec::new(), 0).expect("cloth");
    let cloth = &g.cloth;
    let free_of = |v: usize| {
        let node = cloth.node_of[0][v];
        if node == weights::NO_NODE {
            0.0
        } else {
            cloth.freedom_of(node as usize)
        }
    };
    for v in 0..n {
        if share[v] == 0.0 {
            assert_eq!(free_of(v), 0.0, "vertex {v} on the pelvis alone, at {}, moves", pos[v]);
        }
    }
    let end = (n - seg as usize..n).map(free_of).fold(0.0f32, f32::max);
    assert!(end > 0.1, "the end of the tail swings: {end}");
    // flown forward at 10 m/s: the hips' ring stays put, the end trails
    let mut s = ClothState::default();
    let mut at = Mat4::IDENTITY;
    for _ in 0..120 {
        at = Mat4::from_translation(at.w_axis.truncate() + Vec3::X * (10.0 / 60.0));
        s.step(cloth, &[at, at], &[], at, Vec3::ZERO, 1.0 / 60.0);
    }
    s.deltas(cloth, &mut Vec::new());
    let offset = |v: usize| {
        let node = cloth.node_of[0][v];
        if node == weights::NO_NODE {
            0.0
        } else {
            s.offset_of(cloth, node as usize).length()
        }
    };
    let hips = (0..4 * seg as usize).map(offset).fold(0.0f32, f32::max);
    assert!(hips < 1e-4, "the top of the tail bulges {hips} m off the hips");
    let trail = (n - seg as usize..n).map(offset).fold(0.0f32, f32::max);
    assert!(trail > 0.01, "the end trails: {trail} m");
}

/// Rigid only where skinned mostly to joints that never swing (joint 0
/// here): a cuff a third on the hand swings with the sleeve, a glove does
/// not.
#[test]
fn rigid_only_when_mostly_on_joints_that_never_swing() {
    let fixed = [true, false];
    let skin = |on_hand: f32| NodeSkin {
        joints: [0, 1, 0, 0],
        weights: [on_hand, 1.0 - on_hand, 0.0, 0.0],
    };
    assert!(!mostly_on(&fixed, &skin(0.3)), "a cuff a third on the hand");
    assert!(mostly_on(&fixed, &skin(0.5)));
    assert!(mostly_on(&fixed, &skin(0.9)), "a glove");
    assert!(!mostly_on(&fixed, &skin(0.0)));
}

/// The budget: ours always swings; the others nearest first, within their
/// own share and the total; an avatar whose dress does not fit keeps its
/// hair swinging; one swinging already is not dropped for one a little
/// nearer (no flicker walking past).
#[test]
fn the_budget_gives_the_nearest_first_and_never_cuts_ours() {
    let who = |n: u128| Uuid::from_u128(n);
    let c = |owner: u128, kind: Kind, distance: f32, points: usize, active: bool| Candidate {
        slot: (who(owner), kind, usize::MAX),
        own: owner == 0,
        distance,
        points,
        active,
    };
    let candidates = [
        // ours: far over every share
        c(0, Kind::Clothing, 0.0, 50_000, true),
        c(0, Kind::Hair, 0.0, 20_000, true),
        // a near avatar whose dress is over its share; its hair fits
        c(1, Kind::Clothing, 2.0, AVATAR_POINTS + 1, false),
        c(1, Kind::Hair, 2.0, 4_000, false),
        // three more, nearest first, until the total is spent
        c(2, Kind::Clothing, 4.0, 10_000, false),
        c(3, Kind::Clothing, 6.0, 10_000, false),
        c(4, Kind::Clothing, 8.0, 10_000, false),
    ];
    let allowed = budget(&candidates);
    let has = |owner: u128, kind: Kind| allowed.contains(&(who(owner), kind, usize::MAX));
    assert!(has(0, Kind::Clothing) && has(0, Kind::Hair), "ours always swings");
    assert!(!has(1, Kind::Clothing), "a dress over its avatar's share");
    assert!(has(1, Kind::Hair), "its hair still swings");
    assert!(has(2, Kind::Clothing) && has(3, Kind::Clothing), "the nearest next");
    assert!(!has(4, Kind::Clothing), "past the total");
    // the fourth one swinging already, the third one 1 m nearer: it stays
    let mut walked = candidates;
    walked[5].active = false;
    walked[6] = c(4, Kind::Clothing, 6.5, 10_000, true);
    walked[5].distance = 5.5;
    let allowed = budget(&walked);
    assert!(allowed.contains(&(who(4), Kind::Clothing, usize::MAX)), "kept while swinging");
    assert!(
        !allowed.contains(&(who(3), Kind::Clothing, usize::MAX)),
        "not swapped for one a little nearer"
    );
}

/// Never cloth: a mesh mostly on joints that never swing (wings, a pet on a
/// Bento joint, gloves), or too small to be a garment (a deformer). Hair,
/// pinned by its own rules, is not judged so.
#[test]
fn wings_props_and_deformers_are_never_cloth() {
    let mesh = |n: usize, fixed: usize| vec![(0..n).map(|v| v < fixed).collect::<Vec<bool>>()];
    assert!(never_cloth(&mesh(500, 400)), "mostly on the wing joints");
    assert!(!never_cloth(&mesh(500, 100)), "a sleeve with its cuff on the hand");
    assert!(never_cloth(&mesh(12, 0)), "a deformer");
    assert!(!never_cloth(&mesh(500, 0)), "a skirt");
    let mut rest = half_skirt(0);
    rest.never = true;
    assert_eq!(Kind::of(&rest), None, "never cloth: never simulated");
}
