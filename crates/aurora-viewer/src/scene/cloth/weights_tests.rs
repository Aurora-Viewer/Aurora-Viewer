use super::*;
use std::f32::consts::TAU;

/// A body of collision volumes: legs and torso as one tall ellipsoid, the
/// head a sphere (the shapes the Blender prototype was checked on).
fn body() -> Body {
    Body::new([
        Mat4::from_scale_rotation_translation(Vec3::new(0.15, 0.15, 0.75), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, 0.75)),
        Mat4::from_scale_rotation_translation(Vec3::splat(0.1), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, 1.6)),
    ])
}

/// An open tube through rings (radius, height), top to bottom.
fn lathe(rings: &[(f32, f32)], seg: u32) -> (Vec<Vec3>, Vec<u32>) {
    let mut pos = Vec::new();
    for &(r, z) in rings {
        for i in 0..seg {
            let a = TAU * i as f32 / seg as f32;
            pos.push(Vec3::new(r * a.cos(), r * a.sin(), z));
        }
    }
    let mut idx = Vec::new();
    for ring in 0..rings.len() as u32 - 1 {
        for i in 0..seg {
            let (a, b) = (ring * seg + i, ring * seg + (i + 1) % seg);
            let (c, d) = (a + seg, b + seg);
            idx.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    (pos, idx)
}

/// The same tube with its vertices duplicated along every fourth seam, as a
/// mesh split by UVs reaches the GPU.
fn split_seams(pos: &[Vec3], idx: &[u32], seg: u32) -> (Vec<Vec3>, Vec<u32>) {
    let mut out_pos = pos.to_vec();
    let mut out_idx = Vec::new();
    for t in idx.as_chunks::<3>().0 {
        let seam = t.iter().all(|&i| i % seg == 0 || i % seg == seg - 1) || t.iter().any(|&i| i % seg == 0);
        for &i in t {
            if seam && i % seg == 0 && (i / seg).is_multiple_of(4) {
                out_idx.push(out_pos.len() as u32);
                out_pos.push(pos[i as usize]);
            } else {
                out_idx.push(i);
            }
        }
    }
    (out_pos, out_idx)
}

fn ring_means(pos: &[Vec3], free: &[f32]) -> Vec<(f32, f32)> {
    let mut rings: Vec<(f32, f32, u32)> = Vec::new();
    for (p, &f) in pos.iter().zip(free) {
        match rings.iter_mut().find(|r| (r.0 - p.z).abs() < 1e-4) {
            Some(r) => {
                r.1 += f;
                r.2 += 1;
            }
            None => rings.push((p.z, f, 1)),
        }
    }
    rings.sort_by(|a, b| b.0.total_cmp(&a.0));
    rings.into_iter().map(|r| (r.0, r.1 / r.2 as f32)).collect()
}

fn run(pos: &[Vec3], idx: &[u32], body: Option<&Body>) -> Weights {
    let rigid = vec![false; pos.len()];
    compute(
        &[FaceInput {
            positions: pos,
            indices: idx,
            rigid: &rigid,
        }],
        body,
        &Params::default(),
    )
}

#[test]
fn a_flared_skirt_is_pinned_at_the_waist_and_free_at_the_hem() {
    let rings: Vec<_> = (0..=10)
        .map(|t| (0.155 + 0.25 * (t as f32 / 10.0).powf(1.5), 1.0 - 0.055 * t as f32))
        .collect();
    let (pos, idx) = lathe(&rings, 32);
    let w = run(&pos, &idx, Some(&body()));
    let m = ring_means(&pos, &w.faces[0]);
    assert_eq!(m.first().unwrap().1, 0.0, "waistband pinned: {m:?}");
    assert!(m.last().unwrap().1 > 0.99, "hem free: {m:?}");
    assert!(m.windows(2).all(|p| p[1].1 >= p[0].1 - 1e-6), "monotonic down the skirt: {m:?}");
    assert!(!w.stats.hugging);
}

#[test]
fn seams_split_on_the_gpu_do_not_change_the_result() {
    let rings: Vec<_> = (0..=10)
        .map(|t| (0.155 + 0.25 * (t as f32 / 10.0).powf(1.5), 1.0 - 0.055 * t as f32))
        .collect();
    let (pos, idx) = lathe(&rings, 32);
    let (spos, sidx) = split_seams(&pos, &idx, 32);
    assert!(spos.len() > pos.len());
    let whole = ring_means(&pos, &run(&pos, &idx, Some(&body())).faces[0]);
    let split = run(&spos, &sidx, Some(&body()));
    assert_eq!(split.stats.nodes, pos.len(), "duplicates welded back");
    let split = ring_means(&spos, &split.faces[0]);
    for (a, b) in whole.iter().zip(&split) {
        assert!((a.1 - b.1).abs() < 1e-5, "{whole:?} vs {split:?}");
    }
}

#[test]
fn a_hem_brushing_the_legs_is_a_contact_not_a_pin() {
    // tulip skirt: wide in the middle, its hem back against the legs
    let rings: Vec<_> = (0..=10)
        .map(|t| {
            (
                0.155 + 0.14 * (std::f32::consts::PI * t as f32 / 10.0).sin(),
                1.0 - 0.045 * t as f32,
            )
        })
        .collect();
    let (pos, idx) = lathe(&rings, 32);
    let w = run(&pos, &idx, Some(&body()));
    let m = ring_means(&pos, &w.faces[0]);
    assert!(w.stats.dropped_clusters >= 1, "{:?}", w.stats);
    assert!(m.last().unwrap().1 > 0.9, "hem free: {m:?}");
}

#[test]
fn hair_hangs_from_the_scalp_even_where_it_rests_on_the_shoulders() {
    // a ribbon from the back of the head down over the shoulders
    let mut pos = Vec::new();
    let mut idx = Vec::new();
    for k in 0..12u32 {
        let t = k as f32 / 11.0;
        let (z, y) = (1.62 - 0.4 * t, -(0.101 + 0.06 * t));
        pos.push(Vec3::new(-0.03, y, z));
        pos.push(Vec3::new(0.03, y, z));
        if k > 0 {
            let (a, b, c, d) = (2 * k - 2, 2 * k - 1, 2 * k, 2 * k + 1);
            idx.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    let w = run(&pos, &idx, Some(&body()));
    let f = &w.faces[0];
    assert_eq!(f[0], 0.0, "root pinned");
    assert!(f[23] > 0.99, "tip free: {f:?}");
}

#[test]
fn a_mesh_hugging_the_body_stays_rigid() {
    // a body-tight tube from the ankles to the chest: a mesh body or leggings
    let rings: Vec<_> = (0..=16).map(|t| (0.13, 1.35 - 0.065 * t as f32)).collect();
    let (pos, idx) = lathe(&rings, 24);
    let w = run(&pos, &idx, Some(&body()));
    assert!(w.stats.hugging, "{:?}", w.stats);
    assert!(w.is_rigid());
}

#[test]
fn vertices_on_joints_that_never_swing_are_pinned() {
    let rings: Vec<_> = (0..=10)
        .map(|t| (0.155 + 0.25 * (t as f32 / 10.0), 1.0 - 0.055 * t as f32))
        .collect();
    let (pos, idx) = lathe(&rings, 16);
    // the hem's ring is skinned to, say, fingers
    let rigid: Vec<bool> = (0..pos.len()).map(|i| i >= 160).collect();
    let w = compute(
        &[FaceInput {
            positions: &pos,
            indices: &idx,
            rigid: &rigid,
        }],
        Some(&body()),
        &Params::default(),
    );
    assert!(w.faces[0][160..].iter().all(|&f| f == 0.0));
    assert!(w.faces[0][100..140].iter().any(|&f| f > 0.3), "the rest still moves");
}

#[test]
fn cards_resting_on_the_back_hang_off_the_cap() {
    // a cap on the scalp and three cards below it, each its own island, their
    // tops clear of the head, their ends leaning on the back
    let (mut pos, mut idx) = lathe(&[(0.105, 1.68), (0.105, 1.6)], 16);
    for c in 0..3u32 {
        let x = -0.04 + 0.04 * c as f32;
        let base = pos.len() as u32;
        for k in 0..8u32 {
            let z = 1.595 - 0.05 * k as f32;
            let y = -0.16 + 0.07 * k as f32 / 7.0;
            pos.push(Vec3::new(x, y, z));
            pos.push(Vec3::new(x + 0.03, y, z));
            if k > 0 {
                let (a, b, cc, d) = (base + 2 * k - 2, base + 2 * k - 1, base + 2 * k, base + 2 * k + 1);
                idx.extend_from_slice(&[a, b, d, a, d, cc]);
            }
        }
    }
    let w = run(&pos, &idx, Some(&body()));
    assert_eq!(w.stats.islands, 4);
    assert_eq!(w.stats.floating_islands, 3, "{:?}", w.stats);
    let f = &w.faces[0];
    let card_tip = 32 + 15;
    assert!(f[..32].iter().all(|&v| v == 0.0), "cap pinned");
    assert!(f[card_tip] > 0.9, "card tips free: {}", f[card_tip]);
    assert!(f[32] < f[card_tip], "card tops less free than their tips");
}

#[test]
fn without_a_body_the_top_is_pinned() {
    let rings: Vec<_> = (0..=10).map(|t| (0.2, 1.0 - 0.05 * t as f32)).collect();
    let (pos, idx) = lathe(&rings, 16);
    let m = ring_means(&pos, &run(&pos, &idx, None).faces[0]);
    assert_eq!(m.first().unwrap().1, 0.0);
    assert!(m.last().unwrap().1 > 0.99);
}

#[test]
fn ellipsoid_distance_is_signed_and_scaled() {
    let b = body();
    assert!((b.distance(Vec3::new(0.0, 0.0, 1.8)) - 0.1).abs() < 1e-4, "above the head");
    assert!(b.distance(Vec3::new(0.0, 0.0, 0.75)) < 0.0, "inside the torso");
    assert!((b.distance(Vec3::new(0.25, 0.0, 0.75)) - 0.10).abs() < 1e-4, "beside the waist");
}

/// A sheet of long hair from the crown, lying 1.5 cm off the head and the
/// upper back, then hanging away from the back below the shoulder blades.
fn hair_on_the_back() -> (Vec<Vec3>, Vec<u32>) {
    let (rows, cols) = (24u32, 6u32);
    let mut pos = Vec::new();
    for r in 0..=rows {
        let t = r as f32 / rows as f32;
        let z = 1.70 - 0.6 * t;
        for c in 0..=cols {
            let y = (c as f32 / cols as f32 - 0.5) * 0.12;
            // on the head sphere above the neck, on the torso below it
            let x = if z > 1.52 {
                let dz = z - 1.6;
                -((0.1f32 * 0.1 - dz * dz - y * y).max(0.0).sqrt() + 0.015)
            } else {
                let rr = 0.15 * (1.0 - ((z - 0.75) / 0.75).powi(2)).max(0.0).sqrt();
                let away = ((1.38 - z) * 0.6).max(0.0);
                -((rr * rr - y * y).max(0.0).sqrt() + 0.015 + away)
            };
            pos.push(Vec3::new(x, y, z));
        }
    }
    let mut idx = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let a = r * (cols + 1) + c;
            let (b, cc, d) = (a + 1, a + cols + 1, a + cols + 2);
            idx.extend_from_slice(&[a, b, d, a, d, cc]);
        }
    }
    (pos, idx)
}

#[test]
fn long_hair_on_the_back_is_pinned_at_the_scalp_only() {
    let (pos, idx) = hair_on_the_back();
    let w = run(&pos, &idx, Some(&body()));
    assert!(!w.stats.hugging, "{:?}", w.stats);
    assert!(w.stats.wrap > 0.5, "one-sided {:?}", w.stats);
    let f = &w.faces[0];
    let top = pos.iter().map(|p| p.z).fold(f32::MIN, f32::max);
    for (p, &v) in pos.iter().zip(f) {
        if p.z > top - 0.05 {
            assert_eq!(v, 0.0, "crown pinned at {p}");
        }
        if p.z < 1.2 {
            assert!(v > 0.6, "free at {p}: {v}");
        }
        if (1.4..1.5).contains(&p.z) {
            assert!(v > 0.1, "resting on the upper back, not pinned, at {p}: {v}");
        }
    }
}

#[test]
fn the_volumes_fit_the_body_worn() {
    // a volume of radius 0.1, and a body whose surface there is 0.15 round
    // (a bigger bottom) or 0.07 (a slim one)
    let volume = Mat4::from_scale_rotation_translation(Vec3::splat(0.1), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, 1.0));
    for radius in [0.13f32, 0.085] {
        let mut points = Vec::new();
        for i in 0..40 {
            for j in 1..20 {
                let (a, b) = (TAU * i as f32 / 40.0, std::f32::consts::PI * j as f32 / 20.0);
                points.push(Vec3::new(b.sin() * a.cos(), b.sin() * a.sin(), b.cos()) * radius + Vec3::Z);
            }
        }
        let s = fit_scales(&[volume], &points)[0];
        let want = radius / 0.1;
        assert!((s - Vec3::splat(want)).abs().max_element() < 0.03, "{s} for {want}");
    }
    // a whole size off: held within the range
    let far: Vec<Vec3> = (0..100)
        .map(|i| Vec3::new((i as f32 * 0.3).cos() * 0.19, (i as f32 * 0.3).sin() * 0.19, 1.0))
        .collect();
    assert_eq!(fit_scales(&[volume], &far)[0], Vec3::splat(FIT_RANGE.1));
    // too few points: unchanged
    assert_eq!(fit_scales(&[volume], &[Vec3::new(0.2, 0.0, 1.0)]), vec![Vec3::ONE]);
}

#[test]
fn a_segment_of_a_mesh_body_stays_rigid_though_one_sided() {
    // the back of the hips, alone (bodies are cut into such segments)
    let mut pos = Vec::new();
    for r in 0..=8 {
        for c in 0..=8 {
            let a = std::f32::consts::PI * (0.6 + 0.8 * c as f32 / 8.0);
            let z = 0.85 + 0.3 * r as f32 / 8.0;
            let rr = 0.15 * (1.0 - ((z - 0.75) / 0.75).powi(2)).max(0.0).sqrt() + 0.01;
            pos.push(Vec3::new(rr * a.cos(), rr * a.sin(), z));
        }
    }
    let mut idx = Vec::new();
    for r in 0..8u32 {
        for c in 0..8u32 {
            let a = r * 9 + c;
            idx.extend_from_slice(&[a, a + 1, a + 10, a, a + 10, a + 9]);
        }
    }
    let w = run(&pos, &idx, Some(&body()));
    assert!(w.stats.wrap > 0.8, "one-sided {:?}", w.stats);
    assert!(w.stats.hugging && w.is_rigid(), "{:?}", w.stats);
}

/// Long hair lying flat down the back: as many of its vertices touch the
/// body as a fitted top's, but hair swings (pinned at the scalp); the same
/// shape that is no hair (a top, a body segment) stays rigid.
#[test]
fn hair_lying_down_the_back_swings_where_a_top_would_not() {
    let (rows, cols) = (24u32, 6u32);
    let mut pos = Vec::new();
    for r in 0..=rows {
        let z = 1.70 - 0.6 * r as f32 / rows as f32;
        for c in 0..=cols {
            let y = (c as f32 / cols as f32 - 0.5) * 0.12;
            let x = if z > 1.52 {
                let dz = z - 1.6;
                -((0.1f32 * 0.1 - dz * dz - y * y).max(0.0).sqrt() + 0.015)
            } else {
                let rr = 0.15 * (1.0 - ((z - 0.75) / 0.75).powi(2)).max(0.0).sqrt();
                -((rr * rr - y * y).max(0.0).sqrt() + 0.015)
            };
            pos.push(Vec3::new(x, y, z));
        }
    }
    let mut idx = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let a = r * (cols + 1) + c;
            idx.extend_from_slice(&[a, a + 1, a + cols + 2, a, a + cols + 2, a + cols + 1]);
        }
    }
    let rigid = vec![false; pos.len()];
    let face = [FaceInput {
        positions: &pos,
        indices: &idx,
        rigid: &rigid,
    }];
    let top = run(&pos, &idx, Some(&body()));
    assert!(top.stats.hugging && top.is_rigid(), "{:?}", top.stats);
    let hair = compute(
        &face,
        Some(&body()),
        &Params {
            drapes: true,
            ..Default::default()
        },
    );
    assert!(!hair.is_rigid(), "{:?}", hair.stats);
    let crown = pos.iter().map(|p| p.z).fold(f32::MIN, f32::max);
    for (p, &v) in pos.iter().zip(&hair.faces[0]) {
        if p.z > crown - 0.05 {
            assert_eq!(v, 0.0, "crown pinned at {p}");
        }
        if p.z < 1.3 {
            assert!(v > 0.6, "free down the back at {p}: {v}");
        }
    }
}

/// A sign floating beside a skirt (a demo logo) is no card of it.
#[test]
fn a_loose_piece_far_from_the_mesh_stays_rigid() {
    let rings: Vec<_> = (0..=10).map(|t| (0.155 + 0.2 * (t as f32 / 10.0), 1.0 - 0.05 * t as f32)).collect();
    let (mut pos, mut idx) = lathe(&rings, 24);
    // a card 1 cm off the hem, and a sign 20 cm out from it
    for x in [0.325f32, 0.52] {
        let base = pos.len() as u32;
        pos.extend([
            Vec3::new(x, -0.05, 0.65),
            Vec3::new(x, 0.05, 0.65),
            Vec3::new(x, 0.05, 0.55),
            Vec3::new(x, -0.05, 0.55),
        ]);
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let w = run(&pos, &idx, Some(&body()));
    assert_eq!(w.stats.detached, 1, "{:?}", w.stats);
    let n = pos.len();
    let f = &w.faces[0];
    assert!(f[n - 8..n - 4].iter().all(|&v| v > 0.5), "the card swings: {:?}", &f[n - 8..n - 4]);
    assert!(f[n - 4..].iter().all(|&v| v == 0.0), "the sign stays: {:?}", &f[n - 4..]);
}

/// Hair made of locks, each its own piece: one from the scalp, one lying on
/// the back from the shoulders down. The lower lock hangs from the scalp's
/// hair, not pinned where it starts on the back.
#[test]
fn a_lock_lying_on_the_back_hangs_from_the_scalp() {
    let (cols, rows) = (6u32, 24u32);
    let mut pos = Vec::new();
    let mut idx = Vec::new();
    // (two strips: rows 0..=10 from the crown, rows 11..=24 from the shoulders)
    for (from, to) in [(0u32, 10u32), (11, rows)] {
        let base = pos.len() as u32;
        for r in from..=to {
            let z = 1.70 - 0.6 * r as f32 / rows as f32;
            for c in 0..=cols {
                let y = (c as f32 / cols as f32 - 0.5) * 0.12;
                let x = if z > 1.52 {
                    let dz = z - 1.6;
                    -((0.1f32 * 0.1 - dz * dz - y * y).max(0.0).sqrt() + 0.015)
                } else {
                    let rr = 0.15 * (1.0 - ((z - 0.75) / 0.75).powi(2)).max(0.0).sqrt();
                    -((rr * rr - y * y).max(0.0).sqrt() + 0.015)
                };
                pos.push(Vec3::new(x, y, z));
            }
        }
        for r in 0..to - from {
            for c in 0..cols {
                let a = base + r * (cols + 1) + c;
                idx.extend_from_slice(&[a, a + 1, a + cols + 2, a, a + cols + 2, a + cols + 1]);
            }
        }
    }
    let rigid = vec![false; pos.len()];
    let face = [FaceInput {
        positions: &pos,
        indices: &idx,
        rigid: &rigid,
    }];
    let weigh = |pin_above| {
        compute(
            &face,
            Some(&body()),
            &Params {
                drapes: true,
                pin_above,
                ..Default::default()
            },
        )
    };
    let lock_top = 1.70 - 0.6 * 11.0 / rows as f32;
    let top_of_lock = |w: &Weights| {
        pos.iter()
            .zip(&w.faces[0])
            .filter(|(p, _)| (p.z - lock_top).abs() < 1e-4)
            .map(|(_, &v)| v)
            .fold(f32::MAX, f32::min)
    };
    assert_eq!(top_of_lock(&weigh(None)), 0.0, "pinned where it touches, as before");
    let w = weigh(Some(1.66));
    assert!(top_of_lock(&w) > 0.3, "hangs from the scalp: {}", top_of_lock(&w));
    let crown = pos.iter().map(|p| p.z).fold(f32::MIN, f32::max);
    for (p, &v) in pos.iter().zip(&w.faces[0]) {
        if p.z > crown - 0.05 {
            assert_eq!(v, 0.0, "crown pinned at {p}");
        }
    }
}

/// A dress fitted from the chest to the hips, its skirt flaring off the
/// legs: most of it hugs, as a body does, but its skirt stands well off the
/// body. Held where it hugs, free at the hem.
#[test]
fn a_fitted_dress_with_a_loose_skirt_swings_below_the_hips() {
    // a dense fitted bodice (1.0 m down to the hips at 0.8 m), then the
    // skirt flaring out to 0.45 m round at 0.4 m
    let mut rings: Vec<(f32, f32)> = (0..=40).map(|t| (0.17, 1.0 - 0.005 * t as f32)).collect();
    rings.extend((1..=8).map(|t| (0.17 + 0.035 * t as f32, 0.8 - 0.05 * t as f32)));
    let (pos, idx) = lathe(&rings, 24);
    let w = run(&pos, &idx, Some(&body()));
    assert!(!w.stats.hugging, "{:?}", w.stats);
    assert!(!hugs(&body(), &pos, &Params::default()).0, "not body-like");
    let means = ring_means(&pos, &w.faces[0]);
    let bodice = means.iter().filter(|r| r.0 > 0.81).map(|r| r.1).fold(0.0f32, f32::max);
    assert_eq!(bodice, 0.0, "the bodice holds: {means:?}");
    let hem = means.last().unwrap().1;
    assert!(hem > 0.6, "the hem swings: {means:?}");
}

/// A skirt lying on hips far bigger than the volumes (a Reborn's, past the
/// fit's range): against the volumes alone it stood well off the body and
/// was taken for a loose skirt; against the body worn it hugs it.
#[test]
fn a_skirt_on_hips_bigger_than_the_volumes_hugs_the_body_worn() {
    // hips 0.26 m round where the volume is 0.15: the skirt 1 cm on them
    let hips = |z: f32| 0.26 * (1.0 - ((z - 0.95) / 0.3).powi(2)).max(0.0).sqrt().max(0.12);
    let rings: Vec<(f32, f32)> = (0..=12).map(|t| 1.1 - 0.025 * t as f32).map(|z| (hips(z) + 0.01, z)).collect();
    let (pos, idx) = lathe(&rings, 24);
    let mut surface = Vec::new();
    for t in 0..=60 {
        let z = 1.2 - 0.01 * t as f32;
        for i in 0..48 {
            let a = TAU * i as f32 / 48.0;
            surface.push(Vec3::new(hips(z) * a.cos(), hips(z) * a.sin(), z));
        }
    }
    let alone = run(&pos, &idx, Some(&body()));
    assert!(!alone.is_rigid(), "against the volumes alone it seemed loose: {:?}", alone.stats);
    let worn = body().with_surface(Some(std::sync::Arc::new(Surface::new(&surface, &[]))));
    let w = run(&pos, &idx, Some(&worn));
    assert!(w.stats.hugging && w.is_rigid(), "{:?}", w.stats);
}

/// A top worn close from the chest to the waist, its hem flaring 4 cm off
/// the body: pinned where it touches the body worn, its flare free.
#[test]
fn a_close_top_moves_only_at_its_flared_hem() {
    // the body worn: a tube of 0.15 m round; the top 1 cm on it, then
    // flaring at its last rings
    let mut rings: Vec<(f32, f32)> = (0..=20).map(|t| (0.16, 1.2 - 0.01 * t as f32)).collect();
    rings.extend([(0.175, 0.99), (0.19, 0.98), (0.20, 0.97)]);
    let (pos, idx) = lathe(&rings, 24);
    let mut surface = Vec::new();
    for t in 0..=40 {
        let z = 1.25 - 0.01 * t as f32;
        for i in 0..48 {
            let a = TAU * i as f32 / 48.0;
            surface.push(Vec3::new(0.15 * a.cos(), 0.15 * a.sin(), z));
        }
    }
    let worn = body().with_surface(Some(std::sync::Arc::new(Surface::new(&surface, &[]))));
    let w = run(&pos, &idx, Some(&worn));
    assert!(w.stats.hugging, "{:?}", w.stats);
    assert!(!w.is_rigid(), "the hem sways: {:?}", w.stats);
    let means = ring_means(&pos, &w.faces[0]);
    let top = means.iter().filter(|r| r.0 > 1.05).map(|r| r.1).fold(0.0f32, f32::max);
    assert_eq!(top, 0.0, "worn close above: {means:?}");
    let hem = means.last().unwrap().1;
    assert!(hem > 0.05, "the hem moves: {means:?}");
}

/// A body piece among others: measured against the others, its open
/// edges lie on them (the next segment): it stays rigid, however far the
/// rough volumes say it stands.
#[test]
fn a_body_piece_is_measured_against_the_others() {
    let rings: Vec<(f32, f32)> = (0..=10).map(|t| (0.26, 1.0 - 0.02 * t as f32)).collect();
    let (pos, idx) = lathe(&rings, 24);
    // this piece (owner 0) and the next one below it (owner 1)
    let mut surface = pos.clone();
    let mut owners = vec![0u32; pos.len()];
    let (below, _) = lathe(&(0..=10).map(|t| (0.26, 0.8 - 0.02 * t as f32)).collect::<Vec<_>>(), 24);
    owners.extend(std::iter::repeat_n(1, below.len()));
    surface.extend(below);
    let worn = |exclude| {
        body()
            .with_surface(Some(std::sync::Arc::new(Surface::new(&surface, &owners))))
            .excluding(exclude)
    };
    let p = Params {
        sway: true,
        ..Default::default()
    };
    let rigid = vec![false; pos.len()];
    let face = [FaceInput {
        positions: &pos,
        indices: &idx,
        rigid: &rigid,
    }];
    let w = compute(&face, Some(&worn(vec![0])), &p);
    // its lower edge on the next piece; its upper one has none above it
    let means = ring_means(&pos, &w.faces[0]);
    assert_eq!(means.last().unwrap().1, 0.0, "{means:?}");
}

/// Two meshes weighed together, each judged on its own: a bodice worn
/// close is held, the skirt below it swings (judged as one, the pair
/// hugged the body and both stayed rigid).
#[test]
fn a_close_bodice_and_its_skirt_are_judged_each_on_its_own() {
    // the bodice: 1 cm on the torso; the skirt: a flare from the hips
    let bodice_rings: Vec<(f32, f32)> = (0..=10).map(|t| (0.16, 1.0 - 0.01 * t as f32)).collect();
    let skirt_rings: Vec<(f32, f32)> = (0..=10).map(|t| (0.2 + 0.03 * t as f32, 0.85 - 0.04 * t as f32)).collect();
    let (bodice, bi) = lathe(&bodice_rings, 24);
    let (skirt, si) = lathe(&skirt_rings, 24);
    let rb = vec![false; bodice.len()];
    let rs = vec![false; skirt.len()];
    let faces = [
        FaceInput {
            positions: &bodice,
            indices: &bi,
            rigid: &rb,
        },
        FaceInput {
            positions: &skirt,
            indices: &si,
            rigid: &rs,
        },
    ];
    let w = compute_members(&faces, &[0, 1], Some(&body()), &Params::default());
    assert!(w.faces[0].iter().all(|&f| f == 0.0), "the bodice held");
    let hem = w.faces[1].iter().copied().fold(0.0f32, f32::max);
    assert!(hem > 0.5, "the skirt swings: {hem}");
}

/// Two hairs weighed together: the second's base a little further from the
/// head than the first's is still pinned at its own scalp contact, not left
/// to hang off the first one's ends.
#[test]
fn a_second_hair_is_pinned_at_its_own_scalp() {
    let (first, fi) = hair_on_the_back();
    // the same hair 3 cm further out
    let second: Vec<Vec3> = first.iter().map(|p| *p + Vec3::new(-0.03, 0.0, 0.0)).collect();
    let r = vec![false; first.len()];
    let faces = [
        FaceInput {
            positions: &first,
            indices: &fi,
            rigid: &r,
        },
        FaceInput {
            positions: &second,
            indices: &fi,
            rigid: &r,
        },
    ];
    let p = Params {
        drapes: true,
        ..Default::default()
    };
    let w = compute_members(&faces, &[0, 1], Some(&body()), &p);
    let top = first.iter().map(|p| p.z).fold(f32::MIN, f32::max);
    for (k, face) in w.faces.iter().enumerate() {
        let crown: Vec<f32> = first.iter().zip(face).filter(|(p, _)| p.z > top - 0.03).map(|(_, &f)| f).collect();
        assert!(crown.iter().all(|&f| f == 0.0), "hair {k} pinned at its crown: {crown:?}");
    }
}

/// A fin beside a tail, not sharing a vertex with it (its own piece): with
/// stitches it is one piece with the tail, hanging from where the tail is
/// pinned, held at its distance from it; without, it floats on its own.
#[test]
fn a_fin_near_its_tail_is_stitched_to_it() {
    // the tail: a strip hanging down the back; the fin: a strip 1.5 cm off it
    let strip = |x: f32| -> (Vec<Vec3>, Vec<u32>) {
        let mut pos = Vec::new();
        for r in 0..=10 {
            let z = 0.9 - 0.04 * r as f32;
            pos.push(Vec3::new(x, -0.05, z));
            pos.push(Vec3::new(x, 0.05, z));
        }
        let mut idx = Vec::new();
        for r in 0..10u32 {
            let a = 2 * r;
            idx.extend_from_slice(&[a, a + 1, a + 3, a, a + 3, a + 2]);
        }
        (pos, idx)
    };
    let (tail, ti) = strip(-0.17);
    let (fin, fi) = strip(-0.185);
    let r = vec![false; tail.len()];
    let faces = [
        FaceInput {
            positions: &tail,
            indices: &ti,
            rigid: &r,
        },
        FaceInput {
            positions: &fin,
            indices: &fi,
            rigid: &r,
        },
    ];
    let apart = compute_members(&faces, &[0, 1], Some(&body()), &Params::default());
    assert_eq!(apart.stats.islands, 2);
    let p = Params {
        stitch: 0.03,
        ..Default::default()
    };
    let w = compute_members(&faces, &[0, 1], Some(&body()), &p);
    assert_eq!(w.stats.islands, 1, "{:?}", w.stats);
    assert!(w.stats.stitches >= 20, "{:?}", w.stats);
    // stitched edges between the pieces, at their distance
    let across = w
        .edges
        .iter()
        .filter(|&&(a, b, _)| {
            (w.node_of[0].contains(&a) && w.node_of[1].contains(&b)) || (w.node_of[1].contains(&a) && w.node_of[0].contains(&b))
        })
        .count();
    assert!(across >= 20, "{across} stitches in the edges");
}

/// A top worn close, flaring at its hem, weighed with a loose skirt (a
/// separate mesh of the same group): its hem moves as it does alone, not
/// held whole, nor ramped over the skirt's length.
#[test]
fn a_close_top_among_other_clothes_moves_as_it_does_alone() {
    let mut rings: Vec<(f32, f32)> = (0..=20).map(|t| (0.16, 1.2 - 0.01 * t as f32)).collect();
    rings.extend([(0.175, 0.99), (0.19, 0.98), (0.20, 0.97)]);
    let (top, ti) = lathe(&rings, 24);
    let skirt_rings: Vec<(f32, f32)> = (0..=10).map(|t| (0.2 + 0.03 * t as f32, 0.85 - 0.04 * t as f32)).collect();
    let (skirt, si) = lathe(&skirt_rings, 24);
    let mut surface = Vec::new();
    for t in 0..=60 {
        let z = 1.25 - 0.01 * t as f32;
        for i in 0..48 {
            let a = TAU * i as f32 / 48.0;
            surface.push(Vec3::new(0.15 * a.cos(), 0.15 * a.sin(), z));
        }
    }
    let worn = body().with_surface(Some(std::sync::Arc::new(Surface::new(&surface, &[]))));
    let rt = vec![false; top.len()];
    let rs = vec![false; skirt.len()];
    let faces = [
        FaceInput {
            positions: &top,
            indices: &ti,
            rigid: &rt,
        },
        FaceInput {
            positions: &skirt,
            indices: &si,
            rigid: &rs,
        },
    ];
    let w = compute_members(&faces, &[0, 1], Some(&worn), &Params::default());
    let means = ring_means(&top, &w.faces[0]);
    let above = means.iter().filter(|r| r.0 > 1.05).map(|r| r.1).fold(0.0f32, f32::max);
    assert_eq!(above, 0.0, "worn close above: {means:?}");
    let hem = means.last().unwrap().1;
    let alone = compute_members(&faces[..1], &[0], Some(&worn), &Params::default());
    let alone_hem = ring_means(&top, &alone.faces[0]).last().unwrap().1;
    assert!(
        hem > 0.05 && (hem - alone_hem).abs() < 1e-4,
        "the hem moves as alone ({alone_hem}): {means:?}"
    );
    let skirt_hem = w.faces[1].iter().copied().fold(0.0f32, f32::max);
    assert!(skirt_hem > 0.5, "the skirt swings: {skirt_hem}");
}

/// Pruned, only the moving nodes and the pinned ones tied to them are left;
/// every vertex keeps its freedom, the others are `NO_NODE`, and no
/// constraint is left between two pinned nodes.
#[test]
fn pruning_keeps_what_moves_and_what_holds_it() {
    // a tall waistband on the body, then a flared skirt
    let rings: Vec<(f32, f32)> = (0..=20)
        .map(|t| {
            let flare = (t as f32 - 10.0).max(0.0) / 10.0;
            (0.155 + 0.25 * flare.powf(1.5), 1.2 - 0.03 * t as f32)
        })
        .collect();
    let (pos, idx) = lathe(&rings, 32);
    // its top skinned to joints that never swing
    let rigid: Vec<bool> = pos.iter().map(|p| p.z > 1.0).collect();
    let face = [FaceInput {
        positions: &pos,
        indices: &idx,
        rigid: &rigid,
    }];
    let mut w = compute(&face, Some(&body()), &Params::default());
    let before = w.faces.clone();
    let n = w.nodes.len();
    assert!(!w.is_rigid());
    let kept = w.prune();
    assert_eq!(kept.len(), w.nodes.len());
    assert!(w.nodes.len() < n, "pinned nodes dropped: {} of {n}", w.nodes.len());
    for (v, &node) in w.node_of[0].iter().enumerate() {
        let f = if node == NO_NODE { 0.0 } else { w.node_free[node as usize] };
        assert_eq!(f, before[0][v], "vertex {v}");
    }
    for &(a, b, _) in w.edges.iter().chain(&w.bends) {
        assert!((a as usize) < w.nodes.len() && (b as usize) < w.nodes.len());
        assert!(w.node_free[a as usize] > 0.0 || w.node_free[b as usize] > 0.0, "a pinned pair kept");
    }
    for (u, &r) in w.node_root.iter().enumerate() {
        assert!((r as usize) < w.nodes.len(), "node {u}'s root {r} dropped");
    }
}

/// A top worn close with a short flared skirt modelled with a thickness:
/// down the outside, round its hem and back up the inside, so the hem has
/// no open edge (as most clothes are made). Among other clothes, it is
/// pinned where it touches the body worn and its skirt swings from there,
/// most at the hem; swaying only at open edges, it never moved.
#[test]
fn a_close_top_with_a_thick_hem_swings_below_the_hips() {
    // outside: 1 cm on the body from 1.2 m to 1.0 m, then flaring out to
    // 10 cm off it at 0.85 m; inside: 5 mm in, back up to the top
    let mut rings: Vec<(f32, f32)> = (0..=20).map(|t| (0.16, 1.2 - 0.01 * t as f32)).collect();
    let skirt: Vec<(f32, f32)> = (1..=10).map(|t| (0.16 + 0.01 * t as f32, 1.0 - 0.015 * t as f32)).collect();
    rings.extend(skirt.iter().copied());
    rings.extend(skirt.iter().rev().map(|&(r, z)| (r - 0.005, z + 0.002)));
    rings.extend((0..=20).rev().map(|t| (0.155, 1.2 - 0.01 * t as f32 + 0.002)));
    let (top, ti) = lathe(&rings, 32);
    // a separate loose scarf hanging far off the body: the top's group
    let scarf_rings: Vec<(f32, f32)> = (0..=10).map(|t| (0.3, 1.3 - 0.03 * t as f32)).collect();
    let (scarf, si) = lathe(&scarf_rings, 16);
    let mut surface = Vec::new();
    for t in 0..=80 {
        let z = 1.3 - 0.01 * t as f32;
        for i in 0..64 {
            let a = TAU * i as f32 / 64.0;
            surface.push(Vec3::new(0.15 * a.cos(), 0.15 * a.sin(), z));
        }
    }
    let worn = body().with_surface(Some(std::sync::Arc::new(Surface::new(&surface, &[]))));
    let rt = vec![false; top.len()];
    let rs = vec![false; scarf.len()];
    let faces = [
        FaceInput {
            positions: &top,
            indices: &ti,
            rigid: &rt,
        },
        FaceInput {
            positions: &scarf,
            indices: &si,
            rigid: &rs,
        },
    ];
    for (label, w) in [
        ("alone", compute_members(&faces[..1], &[0], Some(&worn), &Params::default())),
        ("among others", compute_members(&faces, &[0, 1], Some(&worn), &Params::default())),
    ] {
        let f = &w.faces[0];
        let body_part = top
            .iter()
            .zip(f)
            .filter(|(p, _)| p.z > 1.02)
            .map(|(_, &f)| f)
            .fold(0.0f32, f32::max);
        assert_eq!(body_part, 0.0, "{label}: held where it touches");
        let hem = top
            .iter()
            .zip(f)
            .filter(|(p, _)| p.z < 0.86)
            .map(|(_, &f)| f)
            .fold(0.0f32, f32::max);
        assert!(hem > 0.2, "{label}: the skirt's hem swings: {hem}");
        let mid = top
            .iter()
            .zip(f)
            .filter(|(p, _)| (p.z - 0.93).abs() < 0.01)
            .map(|(_, &f)| f)
            .fold(0.0f32, f32::max);
        assert!(mid < hem, "{label}: freer toward the hem ({mid} mid-skirt, {hem} at the hem)");
    }
}

/// A skirt with a frill tier of its own below its hem (a separate piece,
/// stitched to it), far from the legs: the frill hangs from the skirt, free
/// from its top down. Pinned at its own nearest point to the body, as a
/// second hair is, it was nailed in mid-air and only its edge swung.
#[test]
fn a_frill_far_from_the_body_hangs_from_the_skirt_it_is_stitched_to() {
    let skirt_rings: Vec<(f32, f32)> = (0..=10).map(|t| (0.16 + 0.03 * t as f32, 0.85 - 0.04 * t as f32)).collect();
    let frill_rings: Vec<(f32, f32)> = (0..=5).map(|t| (0.47 + 0.01 * t as f32, 0.44 - 0.02 * t as f32)).collect();
    let (skirt, si) = lathe(&skirt_rings, 32);
    let (frill, fi) = lathe(&frill_rings, 32);
    let rs = vec![false; skirt.len()];
    let rf = vec![false; frill.len()];
    let faces = [
        FaceInput {
            positions: &skirt,
            indices: &si,
            rigid: &rs,
        },
        FaceInput {
            positions: &frill,
            indices: &fi,
            rigid: &rf,
        },
    ];
    let p = Params {
        stitch: 0.03,
        ..Default::default()
    };
    let w = compute_members(&faces, &[0, 1], Some(&body()), &p);
    assert_eq!(w.stats.islands, 1, "stitched: {:?}", w.stats);
    assert_eq!(w.stats.members[1].anchors, 0, "the frill pinned itself: {:?}", w.stats.members[1]);
    let means = ring_means(&frill, &w.faces[1]);
    assert!(means[0].1 > 0.9, "the frill's top swings with the hem: {means:?}");
    let waist = ring_means(&skirt, &w.faces[0])[0].1;
    assert_eq!(waist, 0.0, "the skirt pinned at the waist");
}

/// Clothes worn close are as free as they are slack on the body worn: a
/// snug part holds, a loose sleeve swings (more the looser), and a cuff
/// skinned to the hand swings too while it stands off it. Pinned wherever
/// they touched, a sweater's sleeves never moved.
#[test]
fn clothes_worn_close_are_as_free_as_they_are_slack() {
    // round the body worn (15 cm): snug (1 cm off) from 1.2 m to 1.0 m,
    // then 3 cm off, then a cuff 5 cm off skinned to a joint that never
    // swings
    let mut rings: Vec<(f32, f32)> = (0..=20).map(|t| (0.16, 1.2 - 0.01 * t as f32)).collect();
    rings.extend((1..=10).map(|t| (0.18, 1.0 - 0.01 * t as f32)));
    rings.extend((1..=5).map(|t| (0.20, 0.9 - 0.01 * t as f32)));
    let (pos, idx) = lathe(&rings, 32);
    let rigid: Vec<bool> = pos.iter().map(|p| p.z < 0.895).collect();
    let mut surface = Vec::new();
    for t in 0..=60 {
        let z = 1.3 - 0.01 * t as f32;
        for i in 0..64 {
            let a = TAU * i as f32 / 64.0;
            surface.push(Vec3::new(0.15 * a.cos(), 0.15 * a.sin(), z));
        }
    }
    let worn = body().with_surface(Some(std::sync::Arc::new(Surface::new(&surface, &[]))));
    let face = [FaceInput {
        positions: &pos,
        indices: &idx,
        rigid: &rigid,
    }];
    let w = compute(&face, Some(&worn), &Params::default());
    assert!(w.stats.members[0].tight, "worn close: {:?}", w.stats.members[0]);
    let band = |lo: f32, hi: f32| {
        let f: Vec<f32> = pos
            .iter()
            .zip(&w.faces[0])
            .filter(|(p, _)| p.z > lo && p.z < hi)
            .map(|(_, &f)| f)
            .collect();
        f.iter().sum::<f32>() / f.len() as f32
    };
    let (snug, loose, cuff) = (band(1.01, 1.21), band(0.905, 0.995), band(0.84, 0.895));
    assert_eq!(snug, 0.0, "snug, it holds");
    assert!(loose > 0.05, "3 cm off, it swings: {loose}");
    assert!(
        cuff > loose,
        "the cuff, further off, swings more ({cuff} against {loose}), hand or not"
    );
}

/// Small pieces of their own are found solid (a 2 cm buckle), bigger ones
/// are not (a skirt, a frill), and pruning keeps them by their new nodes.
#[test]
fn small_pieces_of_their_own_are_solid() {
    let rings: Vec<(f32, f32)> = (0..=10).map(|t| (0.16 + 0.03 * t as f32, 0.85 - 0.04 * t as f32)).collect();
    let (skirt, si) = lathe(&rings, 32);
    // a 2 cm buckle 1 cm outside the skirt's third ring
    let r = 0.22 + 0.01;
    let buckle = vec![
        Vec3::new(r, -0.01, 0.78),
        Vec3::new(r, 0.01, 0.78),
        Vec3::new(r, 0.01, 0.76),
        Vec3::new(r, -0.01, 0.76),
    ];
    let bi = vec![0, 1, 2, 0, 2, 3];
    let (rs, rb) = (vec![false; skirt.len()], vec![false; buckle.len()]);
    let faces = [
        FaceInput {
            positions: &skirt,
            indices: &si,
            rigid: &rs,
        },
        FaceInput {
            positions: &buckle,
            indices: &bi,
            rigid: &rb,
        },
    ];
    let p = Params {
        stitch: 0.03,
        ..Default::default()
    };
    let mut w = compute_members(&faces, &[0, 1], Some(&body()), &p);
    assert_eq!(w.pieces.len(), 1, "the buckle alone: {:?}", w.pieces);
    let buckle_nodes: Vec<u32> = w.node_of[1].clone();
    assert_eq!(w.pieces[0].len(), 4);
    assert!(w.pieces[0].iter().all(|u| buckle_nodes.contains(u)));
    w.prune();
    let after: Vec<u32> = w.node_of[1].clone();
    assert_eq!(w.pieces.len(), 1);
    assert!(w.pieces[0].iter().all(|u| after.contains(u)), "kept by its new nodes");
}
