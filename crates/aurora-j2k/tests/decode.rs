//! Every decode against OpenJPEG's, sample for sample: codestreams written
//! by OpenJPEG's encoder with the settings of the format one after the
//! other (wavelets, progression orders, layers, code-block sizes and styles,
//! tiles, origins, precincts, SOP and EPH markers, sub-sampling, precisions,
//! regions of interest, progression changes), decoded whole at every reduce
//! level and cut at every byte.

mod oracle;

use std::sync::atomic::{AtomicUsize, Ordering};

use aurora_j2k::{Decoder, Error, Limits, Options, Samples, Spawn, Threads};
use oracle::{Encode, Verdict};

/// A picture with edges, gradients and noise, the same on every platform.
fn picture(x: u32, y: u32, c: u32) -> i32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    let noise = (h & 0x1F) as i32;
    let gradient = (x * 3 + y * 2 + c * 40) as i32;
    let edge = if (x / 7 + y / 5).is_multiple_of(2) { 60 } else { 0 };
    (gradient + edge + noise) & 0xFF
}

fn sample_for(e: &Encode) -> impl Fn(u32, u32, u32) -> i32 + '_ {
    move |x, y, c| {
        let v = picture(x, y, c);
        let v = if e.prec >= 8 { v << (e.prec - 8) } else { v >> (8 - e.prec) };
        if e.signed { v - (1 << (e.prec - 1)) } else { v }
    }
}

fn ours(dec: &mut Decoder, cs: &[u8], reduce: u8) -> Result<aurora_j2k::Image, Error> {
    dec.decode(
        cs,
        &Options {
            reduce,
            components: 0,
            limits: Limits::NONE,
            ..Options::default()
        },
    )
}

/// The whole codestream at every reduce level: OpenJPEG's samples.
fn check_whole(label: &str, cs: &[u8], dec: &mut Decoder) {
    let levels = aurora_j2k::read_header(cs, &Limits::NONE).expect("header").max_reduce();
    for reduce in 0..=levels {
        let a = oracle::decode(cs, u32::from(reduce));
        assert!(a.is_some(), "{label}: OpenJPEG refuses its own codestream at reduce {reduce}");
        let b = ours(dec, cs, reduce);
        assert_eq!(
            oracle::compare(cs, &a, &b),
            Verdict::Same,
            "{label}, reduce {reduce}: {:?}",
            b.as_ref().map(|i| (i.width, i.height))
        );
        let img = b.expect("decoded");
        assert!(
            !img.truncated && !img.cut_in_decoded,
            "{label}, reduce {reduce}: a whole file is not cut"
        );
    }
    // one level too many is refused, as OpenJPEG refuses it
    assert!(matches!(ours(dec, cs, levels + 1), Err(Error::Limit(_))), "{label}");
    assert!(oracle::decode(cs, u32::from(levels) + 1).is_none(), "{label}");
}

/// Prefixes of the codestream, `step` bytes apart: OpenJPEG's samples,
/// unless the decode says it kept what OpenJPEG drops; a prefix OpenJPEG
/// decodes is never refused.
fn check_cuts(label: &str, cs: &[u8], dec: &mut Decoder, step: usize) {
    let levels = aurora_j2k::read_header(cs, &Limits::NONE).expect("header").max_reduce();
    let mut kept = 0;
    for at in (1..cs.len()).step_by(step) {
        for reduce in [0, levels.min(1)] {
            let a = oracle::decode(&cs[..at], u32::from(reduce));
            let b = ours(dec, &cs[..at], reduce);
            match oracle::compare(&cs[..at], &a, &b) {
                Verdict::Same => {}
                Verdict::Differ { .. } if b.as_ref().is_ok_and(|i| i.cut_in_decoded) => kept += 1,
                // OpenJPEG gives up on some cut packet headers that this
                // decoder reads as an absent packet
                Verdict::Outcome { oracle: false, ours: true } => {}
                v => panic!(
                    "{label}, cut at {at} of {}, reduce {reduce}: {v:?} ({:?})",
                    cs.len(),
                    b.as_ref().map(|i| i.cut_in_decoded)
                ),
            }
            if let Ok(img) = &b {
                assert!(img.truncated, "{label}, cut at {at}: a cut file is truncated");
            }
        }
    }
    let _ = kept;
}

/// Whether OpenJPEG's encoder takes the geometry: its forward wavelet
/// stops on an assertion for a tile-component that is one sample wide or
/// high on an odd coordinate at some resolution.
fn encodable(e: &Encode) -> bool {
    let up = |a: i64, d: i64| (a + d - 1).div_euclid(d);
    let (x0, y0) = (i64::from(e.origin.0), i64::from(e.origin.1));
    let (x1, y1) = (
        x0 + i64::from(e.w - 1) * i64::from(e.sub.0) + 1,
        y0 + i64::from(e.h - 1) * i64::from(e.sub.1) + 1,
    );
    let (tw, th) = e.tile.map_or((x1, y1), |t| (i64::from(t.0), i64::from(t.1)));
    let (tx0, ty0) = if e.tile.is_some() {
        (i64::from(e.tile_origin.0), i64::from(e.tile_origin.1))
    } else {
        (0, 0)
    };
    let axis = |t0: i64, size: i64, a0: i64, a1: i64, d: i64| -> bool {
        let mut t = t0;
        while t < a1 {
            let (c0, c1) = (up(t.max(a0), d), up((t + size).min(a1), d));
            if c1 > c0 {
                for level in 0..i64::from(e.numresolution - 1) {
                    let (r0, r1) = (up(c0, 1 << level), up(c1, 1 << level));
                    if r1 - r0 == 1 && r0 % 2 == 1 {
                        return false;
                    }
                }
            }
            t += size;
        }
        true
    };
    axis(tx0, tw, x0, x1, i64::from(e.sub.0)) && axis(ty0, th, y0, y1, i64::from(e.sub.1))
}

fn check(label: &str, e: &Encode) {
    assert!(encodable(e), "{label}: a geometry OpenJPEG's encoder stops on; pick another");
    let cs = oracle::encode(e, sample_for(e)).unwrap_or_else(|| panic!("{label}: OpenJPEG refuses the settings"));
    let mut dec = Decoder::new();
    check_whole(label, &cs, &mut dec);
    // every byte of a small codestream, a spread of the larger ones
    check_cuts(label, &cs, &mut dec, (cs.len() / 150).max(1));
}

#[test]
fn wavelets_components_and_colour_transform() {
    for irreversible in [false, true] {
        for (comps, mct) in [(1, false), (2, false), (3, true), (3, false), (4, true), (5, true)] {
            let e = Encode {
                w: 48,
                h: 40,
                comps,
                mct,
                irreversible,
                numresolution: 4,
                rates: if irreversible { vec![8.0] } else { vec![0.0] },
                ..Encode::default()
            };
            check(&format!("{comps} components, mct {mct}, irreversible {irreversible}"), &e);
        }
    }
}

#[test]
fn sizes_and_resolutions() {
    for (w, h, numres) in [
        (1, 1, 1),
        (1, 64, 1),
        (64, 1, 1),
        (3, 5, 2),
        (17, 37, 4),
        (64, 64, 7),
        (100, 37, 5),
        (33, 129, 6),
        (16, 16, 5),
        (2, 2, 2),
    ] {
        for irreversible in [false, true] {
            let e = Encode {
                w,
                h,
                comps: 3,
                irreversible,
                numresolution: numres,
                ..Encode::default()
            };
            check(&format!("{w} x {h}, {numres} resolutions, irreversible {irreversible}"), &e);
        }
    }
}

#[test]
fn progression_orders_and_layers() {
    for prog in 0..5 {
        for irreversible in [false, true] {
            let e = Encode {
                w: 72,
                h: 56,
                irreversible,
                prog,
                numresolution: 5,
                rates: vec![60.0, 20.0, 6.0, if irreversible { 2.0 } else { 0.0 }],
                cblk: (16, 16),
                ..Encode::default()
            };
            check(&format!("progression {prog}, 4 layers, irreversible {irreversible}"), &e);
        }
    }
}

#[test]
fn code_block_sizes_and_styles() {
    for cblk in [(4, 4), (8, 64), (64, 8), (32, 16), (64, 64), (4, 1024), (1024, 4)] {
        let e = Encode {
            w: 80,
            h: 70,
            cblk,
            numresolution: 3,
            ..Encode::default()
        };
        check(&format!("code-blocks {cblk:?}"), &e);
    }
    // BYPASS, RESET, TERMALL, VSC, PTERM, SEGSYM, alone and together
    for mode in [1, 2, 4, 8, 16, 32, 1 | 4, 1 | 2 | 8, 63] {
        for irreversible in [false, true] {
            let e = Encode {
                w: 64,
                h: 48,
                mode,
                irreversible,
                numresolution: 4,
                rates: vec![30.0, 8.0, if irreversible { 2.0 } else { 0.0 }],
                cblk: (16, 32),
                ..Encode::default()
            };
            check(&format!("code-block style {mode:#x}, irreversible {irreversible}"), &e);
        }
    }
}

#[test]
fn tiles_and_origins() {
    // (image origin, tile size, tile origin): odd origins put a resolution's
    // first sample on an odd coordinate, the wavelet's other parity
    for (origin, tile, tile_origin) in [
        ((0, 0), Some((32, 32)), (0, 0)),
        ((5, 3), None, (0, 0)),
        ((5, 3), Some((24, 40)), (2, 1)),
        ((1, 1), Some((17, 13)), (0, 0)),
        ((61, 47), Some((64, 64)), (0, 0)),
    ] {
        for irreversible in [false, true] {
            for prog in [0, 2, 3, 4] {
                // as many resolutions as OpenJPEG's encoder takes of the geometry
                let e = (2..=4)
                    .rev()
                    .map(|numresolution| Encode {
                        w: 75,
                        h: 66,
                        irreversible,
                        prog,
                        origin,
                        tile,
                        tile_origin,
                        numresolution,
                        cblk: (16, 16),
                        ..Encode::default()
                    })
                    .find(encodable)
                    .expect("two resolutions of this geometry encode");
                check(
                    &format!(
                        "origin {origin:?}, tiles {tile:?} from {tile_origin:?}, progression {prog}, irreversible {irreversible}, {} resolutions",
                        e.numresolution
                    ),
                    &e,
                );
            }
        }
    }
}

#[test]
fn precincts_and_packet_markers() {
    for prog in 0..5 {
        let e = Encode {
            w: 96,
            h: 80,
            prog,
            numresolution: 4,
            precincts: vec![(32, 32), (16, 32), (16, 16), (8, 8)],
            cblk: (16, 16),
            rates: vec![20.0, 0.0],
            ..Encode::default()
        };
        check(&format!("precincts, progression {prog}"), &e);
    }
    for csty in [2, 4, 6] {
        for irreversible in [false, true] {
            let e = Encode {
                w: 64,
                h: 64,
                csty,
                irreversible,
                numresolution: 4,
                rates: vec![20.0, if irreversible { 3.0 } else { 0.0 }],
                cblk: (16, 16),
                ..Encode::default()
            };
            check(&format!("packet markers {csty}, irreversible {irreversible}"), &e);
        }
    }
}

#[test]
fn precisions_signs_and_sub_sampling() {
    for (prec, signed) in [(8, true), (12, false), (16, false), (16, true), (5, false), (1, false)] {
        for irreversible in [false, true] {
            let e = Encode {
                w: 40,
                h: 33,
                comps: 3,
                prec,
                signed,
                irreversible,
                numresolution: 4,
                ..Encode::default()
            };
            check(&format!("{prec} bits, signed {signed}, irreversible {irreversible}"), &e);
        }
    }
    for sub in [(2, 2), (2, 1), (3, 2)] {
        let e = Encode {
            w: 31,
            h: 29,
            comps: 3,
            mct: false,
            sub,
            origin: (4, 6),
            numresolution: 3,
            ..Encode::default()
        };
        check(&format!("sub-sampling {sub:?}"), &e);
    }
}

#[test]
fn region_of_interest_and_progression_changes() {
    for irreversible in [false, true] {
        let e = Encode {
            w: 48,
            h: 48,
            irreversible,
            roi: Some((0, 5)),
            numresolution: 4,
            ..Encode::default()
        };
        check(&format!("region of interest, irreversible {irreversible}"), &e);
    }
    let e = Encode {
        w: 64,
        h: 64,
        numresolution: 4,
        rates: vec![30.0, 10.0, 0.0],
        cblk: (16, 16),
        // the first two resolutions layer by layer, then everything by resolution
        pocs: vec![(0, 0, 3, 2, 3, 0), (0, 0, 3, 4, 3, 1)],
        ..Encode::default()
    };
    check("progression order changes", &e);
}

#[test]
fn the_viewer_s_requests() {
    // a texture as the viewer asks for it: its first four components, with
    // the limits of the Second Life profile
    let e = Encode {
        w: 128,
        h: 128,
        comps: 5,
        irreversible: true,
        numresolution: 6,
        rates: vec![40.0, 16.0, 8.0],
        ..Encode::default()
    };
    let cs = oracle::encode(&e, sample_for(&e)).expect("encode");
    let mut dec = Decoder::new();
    for reduce in 0..=5u8 {
        let all = oracle::decode(&cs, u32::from(reduce)).expect("oracle");
        let img = dec
            .decode(
                &cs,
                &Options {
                    reduce,
                    components: 4,
                    limits: Limits::SL_TEXTURE,
                    ..Options::default()
                },
            )
            .expect("decode");
        assert_eq!((img.width, img.height, img.components), (128 >> reduce, 128 >> reduce, 4));
        let Samples::Packed(data) = &img.samples else {
            panic!("8-bit components pack")
        };
        for (c, plane) in all.iter().take(4).enumerate() {
            let ours: Vec<i32> = data.iter().skip(c).step_by(4).map(|&v| i32::from(v)).collect();
            assert_eq!(ours, plane.data, "component {c} at reduce {reduce}");
        }
    }
    // two components asked of a colour image: its three decode together
    let img = dec
        .decode(
            &cs,
            &Options {
                reduce: 0,
                components: 2,
                limits: Limits::NONE,
                ..Options::default()
            },
        )
        .expect("decode");
    assert_eq!(img.components, 3);
}

/// A lent thread that runs where it is queued, before the calling thread
/// starts on the work: every code-block and every component is then a lent
/// thread's, whatever else the machine does.
fn inline(task: Box<dyn FnOnce() + Send + 'static>) {
    task();
}

/// A pool with no idle thread: its tasks never run.
fn never(_: Box<dyn FnOnce() + Send + 'static>) {}

fn lent(reduce: u8, count: usize, spawn: Spawn) -> Options {
    Options {
        reduce,
        threads: Threads { count, spawn: Some(spawn) },
        ..Options::default()
    }
}

/// The same image whatever the threads lent, and OpenJPEG's, at each of
/// `reduces`.
fn check_threads(e: &Encode, reduces: &[u8]) {
    assert!(encodable(e), "{e:?}");
    let cs = oracle::encode(e, sample_for(e)).expect("encode");
    let mut dec = Decoder::new();
    for &reduce in reduces {
        let alone = ours(&mut dec, &cs, reduce);
        assert_eq!(
            oracle::compare(&cs, &oracle::decode(&cs, u32::from(reduce)), &alone),
            Verdict::Same,
            "{e:?}, reduce {reduce}"
        );
        for count in [2, 3, 8, 64] {
            assert_eq!(
                dec.decode(&cs, &lent(reduce, count, aurora_j2k::spawn_thread)),
                alone,
                "{count} threads, reduce {reduce}, {e:?}"
            );
        }
        // every code-block a lent thread's, then none
        assert_eq!(
            dec.decode(&cs, &lent(reduce, 4, inline)),
            alone,
            "lent threads alone, reduce {reduce}, {e:?}"
        );
        assert_eq!(
            dec.decode(&cs, &lent(reduce, 8, never)),
            alone,
            "no thread came, reduce {reduce}, {e:?}"
        );
        // cut: the same too
        let cut = &cs[..cs.len() * 2 / 3];
        let cut_alone = ours(&mut dec, cut, reduce);
        assert_eq!(
            dec.decode(cut, &lent(reduce, 5, aurora_j2k::spawn_thread)),
            cut_alone,
            "cut, reduce {reduce}"
        );
        assert_eq!(
            dec.decode(cut, &lent(reduce, 3, inline)),
            cut_alone,
            "cut, lent threads alone, reduce {reduce}"
        );
    }
}

#[test]
fn lent_threads_change_no_sample() {
    // tiles large enough to be shared: the same image whatever the threads,
    // and OpenJPEG's
    for (irreversible, comps, origin) in [(true, 4, (0, 0)), (false, 3, (0, 0)), (true, 3, (3, 5))] {
        let e = Encode {
            w: 600,
            h: 540,
            comps,
            irreversible,
            origin,
            numresolution: 6,
            rates: if irreversible { vec![20.0, 6.0] } else { vec![0.0] },
            ..Encode::default()
        };
        check_threads(&e, &[0]);
    }
    // one level down, still shared
    let e = Encode {
        w: 1100,
        h: 1000,
        comps: 3,
        irreversible: true,
        numresolution: 6,
        rates: vec![20.0, 6.0],
        ..Encode::default()
    };
    check_threads(&e, &[0, 1]);
    // two tiles, each shared: the second with what the first left
    let e = Encode {
        w: 1100,
        h: 600,
        comps: 3,
        irreversible: false,
        numresolution: 5,
        rates: vec![0.0],
        tile: Some((560, 600)),
        ..Encode::default()
    };
    check_threads(&e, &[0]);
}

#[test]
fn a_memory_limit_without_room_for_lent_threads_keeps_one() {
    static OFFERS: AtomicUsize = AtomicUsize::new(0);
    fn counted(task: Box<dyn FnOnce() + Send + 'static>) {
        OFFERS.fetch_add(1, Ordering::Relaxed);
        task();
    }
    let e = Encode {
        w: 600,
        h: 540,
        comps: 4,
        irreversible: true,
        numresolution: 6,
        rates: vec![20.0, 6.0],
        ..Encode::default()
    };
    let cs = oracle::encode(&e, sample_for(&e)).expect("encode");
    let mut dec = Decoder::new();
    let alone = ours(&mut dec, &cs, 0);
    assert!(alone.is_ok());
    let with = |dec: &mut Decoder, max_memory: u64| {
        dec.decode(
            &cs,
            &Options {
                limits: Limits {
                    max_memory,
                    ..Limits::NONE
                },
                ..lent(0, 4, counted)
            },
        )
    };
    // the least memory the decode is allowed with: what the limit counts
    let (mut refused, mut need) = (1u64, 256u64 << 20);
    while need - refused > 1 {
        let mid = refused + (need - refused) / 2;
        match with(&mut dec, mid) {
            Err(Error::Limit(_)) => refused = mid,
            _ => need = mid,
        }
    }
    // with exactly that, no room for what a shared decode holds besides:
    // nothing is offered, and the image is the same
    OFFERS.store(0, Ordering::Relaxed);
    assert_eq!(with(&mut dec, need), alone);
    assert_eq!(OFFERS.load(Ordering::Relaxed), 0, "no room, no lent thread");
    // with room for the lent threads' samples and the tile's bytes: shared
    let samples = 600 * 540 * 4 * 4;
    assert_eq!(with(&mut dec, need + samples + cs.len() as u64), alone);
    assert!(OFFERS.load(Ordering::Relaxed) > 0, "room for the lent threads");
}

#[test]
fn damaged_bytes_decode_alike_with_lent_threads() {
    // a damaged codestream large enough to be shared: one thread's image or
    // one thread's error, and decoders that go on
    let e = Encode {
        w: 600,
        h: 540,
        comps: 3,
        irreversible: true,
        numresolution: 6,
        rates: vec![30.0, 10.0],
        ..Encode::default()
    };
    let cs = oracle::encode(&e, sample_for(&e)).expect("encode");
    let (mut one, mut shared) = (Decoder::new(), Decoder::new());
    let limits = Limits {
        max_memory: 256 << 20,
        ..Limits::SL_TEXTURE
    };
    for i in 0..96 {
        let at = i * cs.len() / 96;
        let mut bad = cs.clone();
        bad[at] ^= if i % 2 == 0 { 0xFF } else { 0x10 };
        let alone = one.decode(
            &bad,
            &Options {
                limits,
                ..Options::default()
            },
        );
        let with_lent = shared.decode(
            &bad,
            &Options {
                limits,
                ..lent(0, 4, inline)
            },
        );
        assert_eq!(with_lent, alone, "byte {at}");
    }
    assert_eq!(shared.decode(&cs, &lent(0, 4, inline)), ours(&mut one, &cs, 0));
}

#[test]
fn limits_refuse_before_allocating() {
    let e = Encode {
        w: 96,
        h: 96,
        comps: 3,
        tile: Some((32, 32)),
        rates: vec![20.0, 0.0],
        ..Encode::default()
    };
    let cs = oracle::encode(&e, sample_for(&e)).expect("encode");
    let mut dec = Decoder::new();
    let with = |limits: Limits| Options {
        reduce: 0,
        components: 0,
        limits,
        ..Options::default()
    };
    assert!(dec.decode(&cs, &with(Limits::SL_TEXTURE)).is_ok());
    assert!(matches!(
        dec.decode(
            &cs,
            &with(Limits {
                max_tiles: 8,
                ..Limits::NONE
            })
        ),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        dec.decode(
            &cs,
            &with(Limits {
                max_components: 2,
                ..Limits::NONE
            })
        ),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        dec.decode(
            &cs,
            &with(Limits {
                max_layers: 1,
                ..Limits::NONE
            })
        ),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        dec.decode(
            &cs,
            &with(Limits {
                max_decoded_side: 64,
                ..Limits::NONE
            })
        ),
        Err(Error::Limit(_))
    ));
    assert!(
        dec.decode(
            &cs,
            &Options {
                reduce: 1,
                components: 0,
                limits: Limits {
                    max_decoded_side: 64,
                    ..Limits::NONE
                },
                ..Options::default()
            }
        )
        .is_ok()
    );
    assert!(matches!(
        dec.decode(
            &cs,
            &with(Limits {
                max_memory: 64 * 1024,
                ..Limits::NONE
            })
        ),
        Err(Error::Limit(_))
    ));
    // a header that claims a huge image is refused from its header alone
    // (Xsiz, Ysiz, XTsiz and YTsiz of the SIZ marker: one tile of 2^30 a side)
    let mut huge = cs.clone();
    for at in [8, 12, 24, 28] {
        huge[at..at + 4].copy_from_slice(&0x4000_0000u32.to_be_bytes());
    }
    assert!(matches!(dec.decode(&huge, &with(Limits::SL_TEXTURE)), Err(Error::Limit(_))));
    assert!(matches!(
        dec.decode(
            &huge,
            &with(Limits {
                max_memory: 256 << 20,
                ..Limits::NONE
            })
        ),
        Err(Error::Limit(_))
    ));
}

#[test]
fn damaged_bytes_never_panic() {
    // every byte of a codestream flipped in turn: an error or an image,
    // whatever OpenJPEG makes of it
    let e = Encode {
        w: 32,
        h: 32,
        comps: 3,
        irreversible: true,
        numresolution: 4,
        rates: vec![20.0, 4.0],
        cblk: (16, 16),
        csty: 6,
        ..Encode::default()
    };
    let cs = oracle::encode(&e, sample_for(&e)).expect("encode");
    let mut dec = Decoder::new();
    let limits = Limits {
        max_memory: 64 << 20,
        ..Limits::SL_TEXTURE
    };
    for at in 0..cs.len() {
        for flip in [0x01u8, 0x80, 0xFF] {
            let mut bad = cs.clone();
            bad[at] ^= flip;
            let _ = dec.decode(
                &bad,
                &Options {
                    reduce: 0,
                    components: 0,
                    limits,
                    ..Options::default()
                },
            );
            let _ = dec.decode(
                &bad[..bad.len() * 2 / 3],
                &Options {
                    reduce: 1,
                    components: 4,
                    limits,
                    ..Options::default()
                },
            );
        }
    }
}

/// A lossy codestream written once by OpenJPEG's encoder on x86-64: 40 x 33,
/// three components with the colour transform, the 9/7 wavelet, four
/// resolutions.
#[rustfmt::skip]
const LOSSY: [u8; 508] = [
    0xff, 0x4f, 0xff, 0x51, 0x00, 0x2f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00, 0x21,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00, 0x21,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01,
    0x07, 0x01, 0x01, 0xff, 0x52, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x01, 0x01, 0x03, 0x04, 0x04, 0x00,
    0x00, 0xff, 0x5c, 0x00, 0x17, 0x42, 0x67, 0x38, 0x67, 0x50, 0x67, 0x50, 0x67, 0x68, 0x50, 0x05,
    0x50, 0x05, 0x50, 0x47, 0x57, 0xd3, 0x57, 0xd3, 0x57, 0x62, 0xff, 0x64, 0x00, 0x25, 0x00, 0x01,
    0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4f, 0x70, 0x65, 0x6e, 0x4a,
    0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x20, 0x32, 0x2e, 0x35, 0x2e,
    0x33, 0xff, 0x90, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x01, 0x79, 0x00, 0x01, 0xff, 0x93, 0xc3,
    0xe2, 0x1e, 0x0d, 0x38, 0xf7, 0x57, 0x0a, 0x18, 0x08, 0x2d, 0x6f, 0xdd, 0xd8, 0xd8, 0xa3, 0xa9,
    0xaa, 0xc3, 0xe0, 0x30, 0x24, 0xc6, 0x3d, 0xd3, 0x4e, 0x71, 0xc8, 0xa2, 0x00, 0xb4, 0x92, 0xa3,
    0xc3, 0xc4, 0x00, 0x40, 0xe4, 0x60, 0xfd, 0x24, 0x8a, 0xc4, 0x3f, 0xc1, 0xd1, 0xe1, 0xf1, 0x09,
    0x87, 0xc4, 0x28, 0x26, 0x94, 0x22, 0xaa, 0x8f, 0xe7, 0x06, 0x0e, 0xca, 0xce, 0x04, 0x89, 0x11,
    0x28, 0xad, 0xca, 0x29, 0xc6, 0xc4, 0xf7, 0x39, 0xba, 0x3b, 0xdd, 0x19, 0xf0, 0xc3, 0xe2, 0x13,
    0x0f, 0x88, 0x44, 0x7c, 0xa2, 0x80, 0x27, 0x76, 0x1b, 0xdb, 0xd5, 0xed, 0xc1, 0xa9, 0x16, 0x24,
    0x7c, 0x75, 0xd5, 0xdc, 0x10, 0xbc, 0x38, 0x26, 0x0b, 0x6c, 0x11, 0xa5, 0x05, 0xd2, 0x8e, 0xad,
    0xac, 0xc1, 0xf0, 0x0b, 0x07, 0xc0, 0x24, 0x3e, 0x11, 0x40, 0x28, 0xc4, 0xc8, 0x92, 0xf9, 0x23,
    0xdf, 0xf1, 0x6b, 0x29, 0x6f, 0x6e, 0x09, 0x5b, 0xc7, 0x8d, 0x87, 0x55, 0x00, 0x6d, 0x32, 0xf8,
    0x4a, 0x05, 0x95, 0xe3, 0x10, 0x15, 0x8b, 0x57, 0xb6, 0xaa, 0x66, 0x41, 0xa2, 0xe9, 0xd6, 0xa6,
    0x95, 0x08, 0x7a, 0xda, 0xa5, 0x2e, 0x1a, 0x74, 0xfc, 0x6f, 0x49, 0xdf, 0x88, 0x73, 0x74, 0xc7,
    0x93, 0x8f, 0x85, 0x18, 0x4e, 0x62, 0x6e, 0x50, 0x1b, 0xcb, 0x19, 0x11, 0x26, 0x3a, 0xcb, 0xa2,
    0x3e, 0xc5, 0x75, 0xa8, 0xee, 0x68, 0x0a, 0xbb, 0x64, 0x21, 0x84, 0x79, 0x67, 0x5a, 0x88, 0x67,
    0x3a, 0x71, 0x93, 0xb0, 0xd1, 0x4c, 0xd5, 0x66, 0xed, 0x3b, 0x56, 0xdd, 0x5e, 0xb6, 0xa7, 0xa8,
    0xc2, 0x40, 0x6d, 0x9b, 0x92, 0xc2, 0xc2, 0xaa, 0x15, 0x70, 0xaa, 0xa6, 0x53, 0x45, 0x18, 0x4e,
    0xe3, 0x1c, 0x0a, 0xa1, 0xf9, 0x94, 0x03, 0x1b, 0x8d, 0x10, 0x30, 0xe0, 0xcf, 0x38, 0x52, 0x1b,
    0x91, 0xdf, 0x98, 0xab, 0xae, 0x00, 0x07, 0xf8, 0xb8, 0x75, 0xc2, 0xd6, 0x8e, 0xb9, 0x0a, 0xd0,
    0x5b, 0x3d, 0x1a, 0xc2, 0x04, 0xdc, 0x32, 0xde, 0x51, 0x79, 0xf0, 0x69, 0xf6, 0x35, 0x1b, 0x8c,
    0xc4, 0x80, 0x30, 0x77, 0x35, 0xc6, 0x8b, 0x82, 0x15, 0x02, 0x12, 0xea, 0x31, 0x8b, 0xe3, 0xf6,
    0x2f, 0xe6, 0x90, 0x88, 0x96, 0xc2, 0x17, 0xb8, 0x76, 0x50, 0x9b, 0xaa, 0x87, 0x56, 0xca, 0x4d,
    0xbb, 0x82, 0x53, 0x05, 0xbf, 0xe2, 0xa7, 0xbd, 0x5d, 0xbf, 0xa7, 0x11, 0x5b, 0xbe, 0x1f, 0xc3,
    0x8c, 0x86, 0x58, 0xa6, 0xb9, 0x03, 0x0e, 0xb2, 0x9b, 0x9a, 0x39, 0x6a, 0x6b, 0x9b, 0xef, 0x94,
    0x1e, 0xad, 0xf8, 0x51, 0x73, 0xaf, 0x8b, 0xfe, 0x2f, 0x70, 0xff, 0xd9,
];

/// FNV-1a of an image's planes: their sizes, then their samples.
fn digest(image: &aurora_j2k::Image) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for plane in oracle::planes_of(image) {
        for value in [plane.w as i32, plane.h as i32].iter().chain(&plane.data) {
            for byte in value.to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    hash
}

#[test]
fn a_lossy_codestream_decodes_alike_on_every_platform() {
    // The digests are those of x86-64, where every sample is OpenJPEG's
    // (the other tests). This decoder never fuses a product and a sum, so
    // its floats, and its samples, are the same wherever it runs; OpenJPEG's
    // are not (`oracle::compare`), which is why its samples cannot say so.
    let mut dec = Decoder::new();
    for (reduce, expected) in [
        (0, 0xf438_5554_8022_35e8_u64),
        (1, 0x49a6_e04e_316e_fc74),
        (3, 0xf034_c3f9_d5de_7f89),
    ] {
        let image = ours(&mut dec, &LOSSY, reduce).expect("decoded");
        assert_eq!(digest(&image), expected, "reduce {reduce}");
    }
    // cut inside a packet: what arrived is kept, the same way everywhere
    let image = ours(&mut dec, &LOSSY[..338], 0).expect("decoded");
    assert!(image.cut_in_decoded);
    assert_eq!(digest(&image), 0xebab_25eb_c2e6_d2d5);
    // and with lent threads that take everything (too small to be shared: one path)
    assert_eq!(
        dec.decode(&LOSSY, &lent(0, 4, inline)).map(|image| digest(&image)),
        Ok(0xf438_5554_8022_35e8)
    );
}

/// Found by the Fuzz workflow: 8 x 8, three components, the third
/// sub-sampled 47 times across (a plane 1 sample wide), in tiles 1 sample
/// wide. Every component of the first tile column reads the same region, so
/// the tile takes the one-pass write, which wrote the third plane with the
/// first plane's stride, past its end.
#[rustfmt::skip]
const NARROW_THIRD_PLANE: [u8; 195] = [
    0xff, 0x4f, 0xff, 0x51, 0x00, 0x2f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x15, 0xd5, 0x08,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01,
    0x07, 0x2f, 0x01, 0xff, 0x52, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x02, 0x01, 0x03, 0x04, 0x04, 0x00,
    0x00, 0xff, 0x5c, 0x00, 0x17, 0x42, 0x67, 0x38, 0x67, 0x50, 0x67, 0x50, 0x67, 0x68, 0x50, 0x05,
    0x50, 0x05, 0x50, 0x47, 0x57, 0xd3, 0x57, 0xd3, 0x57, 0x62, 0xff, 0x64, 0x00, 0x25, 0x00, 0x01,
    0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4f, 0x70, 0x65, 0x6e, 0x4a,
    0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x20, 0x32, 0x2e, 0x35, 0x2e,
    0x33, 0xff, 0x90, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3f, 0x00, 0x01, 0xff, 0x93, 0x80,
    0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x83, 0x91, 0xd1, 0x80, 0x0b, 0x95, 0xe2, 0x01, 0xe8,
    0x84, 0x91, 0x2c, 0x1b, 0x04, 0xa9, 0x84, 0x80, 0x80, 0x80, 0x80, 0x80, 0xc1, 0xf7, 0x81, 0x00,
    0x07, 0x37, 0x80, 0x80, 0x3b, 0x80, 0x80, 0x80, 0x80, 0x91, 0xd1, 0x40, 0x1e, 0x37, 0xae, 0xeb,
    0x95, 0xff, 0xd9,
];

#[test]
fn a_sub_sampled_plane_keeps_its_own_stride() {
    let mut dec = Decoder::new();
    let a = oracle::decode(&NARROW_THIRD_PLANE, 0);
    let b = ours(&mut dec, &NARROW_THIRD_PLANE, 0);
    assert_eq!(oracle::compare(&NARROW_THIRD_PLANE, &a, &b), Verdict::Same);
    // with the four components the viewer asks for: the same refusal
    let viewer = dec.decode(
        &NARROW_THIRD_PLANE,
        &Options {
            reduce: 0,
            components: 4,
            limits: Limits::SL_TEXTURE,
            ..Options::default()
        },
    );
    assert!(matches!(viewer, Err(Error::Data(_))));
}
