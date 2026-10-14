use super::*;
use crate::theme::Theme;

fn content() -> Content {
    Content {
        name: "Nova ♪".into(),
        username: Some("Nova Exemple".into()),
        title: Some("Loups du Nord".into()),
        detail: None,
        role: Role::Resident,
        presence: Presence::Online,
        in_voice: false,
        typing: false,
        friend: false,
        too_complex: false,
        blocked: false,
    }
}

fn detail() -> Detail {
    Detail {
        full: "Complexité : 245 000".into(),
        short: "245 000".into(),
        alert: true,
    }
}

fn tag(n: u128, distance: f32, content: Content) -> Tag<'static> {
    Tag {
        id: Uuid::from_u128(n),
        anchor: Pos2::new(400.0, 300.0),
        at: glam::Vec3::new(10.0, 0.0, 2.0),
        distance,
        fade: 1.0,
        content,
        voice: None,
        loading: None,
        picture: Picture::None,
    }
}

/// One frame of plates in a bare egui context: their rectangles and shapes.
fn frame(ctx: &egui::Context, plates: &mut Plates, tags: &[Tag<'_>], now: f64) -> (Vec<(Uuid, Rect)>, Vec<egui::Shape>) {
    let p = Theme::default().palette();
    let icons = crate::ui::icons::Icons::default();
    let mut rects = Vec::new();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
            ..Default::default()
        },
        |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            plates.paint(&painter, &p, &icons, tags, now, |id, rect| rects.push((id, rect)));
        },
    );
    output.textures_delta.clear();
    (rects, output.shapes.into_iter().map(|s| s.shape).collect())
}

fn walk<'a>(shapes: &'a [egui::Shape], visit: &mut impl FnMut(&'a egui::Shape)) {
    for shape in shapes {
        match shape {
            egui::Shape::Vec(inner) => walk(inner, visit),
            other => visit(other),
        }
    }
}

/// Every color the shapes are drawn in, in order.
fn colors(shapes: &[egui::Shape]) -> Vec<Color32> {
    let mut out = Vec::new();
    walk(shapes, &mut |shape| match shape {
        egui::Shape::Rect(r) => out.extend([r.fill, r.stroke.color]),
        egui::Shape::Circle(c) => out.extend([c.fill, c.stroke.color]),
        egui::Shape::Text(t) => out.extend(t.galley.job.sections.iter().map(|s| s.format.color)),
        egui::Shape::Mesh(m) => out.extend(m.vertices.iter().map(|v| v.color)),
        _ => {}
    });
    out
}

fn text_color(shapes: &[egui::Shape], text: &str) -> Option<Color32> {
    let mut found = None;
    walk(shapes, &mut |shape| {
        if let egui::Shape::Text(t) = shape
            && t.galley.job.text == text
        {
            found = t.galley.job.sections.first().map(|s| s.format.color);
        }
    });
    found
}

/// The screen of `frame`: the rectangle of the marks that say where a plate
/// stands in the world (`aurora_render::world_label_at`).
const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));

/// The plates that blur the world: a region of their own size each.
fn blurred(shapes: &[egui::Shape]) -> usize {
    let mut n = 0;
    walk(shapes, &mut |shape| {
        n += usize::from(matches!(shape, egui::Shape::Callback(c) if c.rect != SCREEN))
    });
    n
}

/// The marks of where the plates stand, one before each plate.
fn marks(shapes: &[egui::Shape]) -> usize {
    let mut n = 0;
    walk(shapes, &mut |shape| {
        n += usize::from(matches!(shape, egui::Shape::Callback(c) if c.rect == SCREEN))
    });
    n
}

#[test]
fn a_plate_changes_level_only_past_the_margin() {
    assert_eq!(level_for(3.0, None), Level::Full);
    assert_eq!(level_for(FULL_DISTANCE + 0.1, None), Level::Compact);
    assert_eq!(level_for(COMPACT_DISTANCE + 0.1, None), Level::Far);
    // standing right at a threshold keeps the level the plate has
    for (threshold, near, far) in [
        (FULL_DISTANCE, Level::Full, Level::Compact),
        (COMPACT_DISTANCE, Level::Compact, Level::Far),
    ] {
        for d in [threshold - LEVEL_MARGIN + 0.1, threshold, threshold + LEVEL_MARGIN - 0.1] {
            assert_eq!(level_for(d, Some(near)), near, "{d} m");
            assert_eq!(level_for(d, Some(far)), far, "{d} m");
        }
        assert_eq!(level_for(threshold + LEVEL_MARGIN + 0.1, Some(near)), far);
        assert_eq!(level_for(threshold - LEVEL_MARGIN - 0.1, Some(far)), near);
    }
    // a teleport next to someone skips the middle level
    assert_eq!(level_for(2.0, Some(Level::Far)), Level::Full);
    assert_eq!(level_for(90.0, Some(Level::Full)), Level::Far);
}

#[test]
fn a_crowd_gets_a_few_complete_plates() {
    // the nearest ones are complete, the next ones compact pills
    assert_eq!(crowd_level(Level::Full, 0, false), Level::Full);
    assert_eq!(crowd_level(Level::Full, MAX_FULL - 1, false), Level::Full);
    assert_eq!(crowd_level(Level::Full, MAX_FULL, false), Level::Compact);
    // a complete plate is not taken away by the first avatar that steps nearer
    assert_eq!(crowd_level(Level::Full, MAX_FULL + FULL_RANK_MARGIN - 1, true), Level::Full);
    assert_eq!(crowd_level(Level::Full, MAX_FULL + FULL_RANK_MARGIN, true), Level::Compact);
    // the other levels are those of the distance
    assert_eq!(crowd_level(Level::Compact, 0, false), Level::Compact);
    assert_eq!(crowd_level(Level::Far, 99, true), Level::Far);

    let ctx = egui::Context::default();
    let mut plates = Plates::default();
    let height = |level| Metrics::of(level, SCALE).height;
    // twenty avatars within a few meters, far first; `moved` puts some elsewhere
    let crowd = |moved: &[(u128, f32)]| -> Vec<Tag<'static>> {
        let distance = |n: u128| moved.iter().find(|(m, _)| *m == n).map_or(2.0 + n as f32 * 0.5, |(_, d)| *d);
        let mut tags: Vec<Tag<'static>> = (0..20u128).map(|n| tag(n, distance(n), content())).collect();
        tags.sort_by(|a, b| b.distance.total_cmp(&a.distance));
        tags
    };
    // (two frames: a plate that changes level takes a moment to get its size)
    let mut complete = |tags: &[Tag<'static>], now: f64| -> Vec<u128> {
        frame(&ctx, &mut plates, tags, now);
        let (rects, _) = frame(&ctx, &mut plates, tags, now + 1.0);
        assert!(rects.iter().all(|(_, r)| r.height() >= height(Level::Compact)));
        let full = rects.iter().filter(|(_, r)| r.height() == height(Level::Full));
        let mut ids: Vec<u128> = full.map(|(id, _)| id.as_u128()).collect();
        ids.sort_unstable();
        ids
    };
    assert_eq!(complete(&crowd(&[]), 1.0), (0..MAX_FULL as u128).collect::<Vec<_>>());
    // the ninth steps just in front of the eighth: it gets a complete plate,
    // and the one it passed keeps its own
    assert_eq!(complete(&crowd(&[(8, 5.25)]), 5.0), (0..=MAX_FULL as u128).collect::<Vec<_>>());
    // two more come to the front: now the farthest of them loses its plate
    let front = crowd(&[(8, 0.3), (9, 0.1), (10, 0.2)]);
    assert_eq!(complete(&front, 9.0), [0, 1, 2, 3, 4, 5, 6, 8, 9, 10]);
}

#[test]
fn the_lines_under_the_name_follow_the_options() {
    let all = content();
    assert_eq!(
        sub_lines(Level::Full, &all),
        [Some(SubLine::Username("Nova Exemple", None)), Some(SubLine::Title("Loups du Nord"))]
    );
    // no username (the option, or no display name chosen): no empty line
    let no_user = Content {
        username: None,
        ..content()
    };
    assert_eq!(sub_lines(Level::Full, &no_user), [Some(SubLine::Title("Loups du Nord")), None]);
    let no_title = Content { title: None, ..content() };
    assert_eq!(
        sub_lines(Level::Full, &no_title),
        [Some(SubLine::Username("Nova Exemple", None)), None]
    );
    let bare = Content {
        username: None,
        title: None,
        ..content()
    };
    assert_eq!(sub_lines(Level::Full, &bare), [None, None]);
    // the complexity takes a free line, or follows the username
    let d = detail();
    let with = |username: bool, title: bool| Content {
        username: username.then(|| "Nova Exemple".to_owned()),
        title: title.then(|| "Loups du Nord".to_owned()),
        detail: Some(detail()),
        ..content()
    };
    assert_eq!(sub_lines(Level::Full, &with(false, false)), [Some(SubLine::Detail(&d)), None]);
    assert_eq!(
        sub_lines(Level::Full, &with(true, false)),
        [Some(SubLine::Username("Nova Exemple", None)), Some(SubLine::Detail(&d))]
    );
    assert_eq!(
        sub_lines(Level::Full, &with(false, true)),
        [Some(SubLine::Title("Loups du Nord")), Some(SubLine::Detail(&d))]
    );
    assert_eq!(
        sub_lines(Level::Full, &with(true, true)),
        [
            Some(SubLine::Username("Nova Exemple", Some(&d))),
            Some(SubLine::Title("Loups du Nord"))
        ]
    );
    // farther, the name alone
    for level in [Level::Compact, Level::Far] {
        assert_eq!(sub_lines(level, &with(true, true)), [None, None]);
    }
}

#[test]
fn badges_and_status_dot_by_state_and_level() {
    // out of the voice channel: no microphone at all (the human's choice)
    let plain = content();
    assert_eq!(badges(Level::Full, &plain).len(), 0);
    assert_eq!(badges(Level::Far, &plain).len(), 0);
    assert_eq!(status_dot(Level::Full, &plain), Some(Presence::Online));
    let busy = Content {
        presence: Presence::Busy,
        ..content()
    };
    assert_eq!(status_dot(Level::Full, &busy), Some(Presence::Busy));
    // no picture, no dot
    assert_eq!(status_dot(Level::Compact, &busy), None);
    let everything = Content {
        in_voice: true,
        typing: true,
        friend: true,
        too_complex: true,
        blocked: true,
        presence: Presence::Away,
        ..content()
    };
    let order = [Badge::Voice, Badge::Typing, Badge::Friend, Badge::TooComplex, Badge::Blocked];
    for level in [Level::Full, Level::Compact] {
        assert_eq!(badges(level, &everything).iter().collect::<Vec<_>>(), order);
    }
    assert_eq!(badges(Level::Far, &everything).len(), 0);
    // a blocked resident's status is hidden
    assert_eq!(status_dot(Level::Full, &everything), None);
    // a friend's heart, after the microphone, at the levels that have badges
    let friend = Content { friend: true, ..content() };
    assert_eq!(badges(Level::Full, &friend).iter().collect::<Vec<_>>(), [Badge::Friend]);
    let in_voice = Content {
        in_voice: true,
        ..friend.clone()
    };
    assert_eq!(
        badges(Level::Compact, &in_voice).iter().collect::<Vec<_>>(),
        [Badge::Voice, Badge::Friend]
    );
    assert_eq!(badges(Level::Far, &in_voice).len(), 0);
    // the status dot stays the status, friend or not
    let away = Content {
        presence: Presence::Away,
        ..friend
    };
    assert_eq!(status_dot(Level::Full, &away), Some(Presence::Away));
}

#[test]
fn full_plates_for_friends_only_cap_the_others_at_the_compact_pill() {
    let stranger = content();
    let friend = Content { friend: true, ..content() };
    let own = Content {
        role: Role::Own,
        ..content()
    };
    // off (the default): the distance alone decides
    for c in [&stranger, &friend, &own] {
        for level in [Level::Full, Level::Compact, Level::Far] {
            assert_eq!(friends_level(level, c, false), level);
        }
    }
    // on: a stranger stops at the compact pill, a friend and we stay complete
    assert_eq!(friends_level(Level::Full, &stranger, true), Level::Compact);
    assert_eq!(friends_level(Level::Full, &friend, true), Level::Full);
    assert_eq!(friends_level(Level::Full, &own, true), Level::Full);
    // the far pill still follows the distance
    assert_eq!(friends_level(Level::Far, &stranger, true), Level::Far);
    assert_eq!(friends_level(Level::Compact, &stranger, true), Level::Compact);
}

#[test]
fn a_stranger_s_plate_stays_compact_close_up_with_friends_only() {
    let ctx = egui::Context::default();
    let mut plates = Plates {
        friends_only_full: true,
        ..Plates::default()
    };
    let friend = Content { friend: true, ..content() };
    let tags = [tag(1, 3.0, content()), tag(2, 4.0, friend)];
    frame(&ctx, &mut plates, &tags, 1.0);
    let level = |id: u128| plates.entries.get(&Uuid::from_u128(id)).map(|e| e.level);
    assert_eq!(level(1), Some(Level::Compact));
    assert_eq!(level(2), Some(Level::Full));
}

#[test]
fn roles_and_their_name_colors() {
    assert_eq!(Role::of(true, true, true, true), Role::Own);
    assert_eq!(Role::of(false, true, true, true), Role::Blocked);
    assert_eq!(Role::of(false, false, true, true), Role::Linden);
    assert_eq!(Role::of(false, false, false, true), Role::Friend);
    assert_eq!(Role::of(false, false, false, false), Role::Resident);
    let p = Theme::default().palette();
    let roles = [Role::Resident, Role::Own, Role::Friend, Role::Linden, Role::Blocked];
    // by default every name is in the theme's ink
    let off = crate::settings::ColorSettings::default();
    assert!(!off.relation_names);
    for role in roles {
        assert_eq!(role.name_color(&p, &off), p.ink);
    }
    // switched on: the colors of the preferences, shared with the classic tags
    let on = crate::settings::ColorSettings {
        relation_names: true,
        ..off
    };
    let c = crate::ui::colors::c;
    assert_eq!(Role::Resident.name_color(&p, &on), p.ink);
    assert_eq!(Role::Own.name_color(&p, &on), c(on.tag_me));
    assert_eq!(Role::Friend.name_color(&p, &on), c(on.tag_friend));
    assert_eq!(Role::Linden.name_color(&p, &on), c(on.tag_linden));
    assert_eq!(Role::Blocked.name_color(&p, &on), c(on.tag_muted));
    // a Linden and a friend are told apart, and both from a resident
    assert_ne!(Role::Linden.name_color(&p, &on), Role::Friend.name_color(&p, &on));
    assert_ne!(Role::Friend.name_color(&p, &on), Role::Resident.name_color(&p, &on));
}

#[test]
fn metrics_keep_the_mockup_proportions() {
    let full = Metrics::of(Level::Full, 1.0);
    assert_eq!((full.height, full.photo, full.name_pt, full.sub_pt), (66.0, 54.0, 17.0, 11.5));
    assert_eq!((full.badge, full.badge_icon, full.badge_gap, full.gap), (25.0, 14.0, 5.0, 12.0));
    let compact = Metrics::of(Level::Compact, 1.0);
    assert_eq!(
        (compact.height, compact.name_pt, compact.badge, compact.photo),
        (40.0, 14.5, 21.0, 0.0)
    );
    let far = Metrics::of(Level::Far, 1.0);
    assert_eq!((far.height, far.name_pt), (26.0, 12.5));
    // in the viewer the name keeps the size of the classic tags, and the
    // pills end in exact half circles at any scale
    assert!((Metrics::of(Level::Full, SCALE).name_pt - NAME_PT).abs() < 0.001);
    for scale in [SCALE, 1.0, 1.3] {
        for level in [Level::Full, Level::Compact, Level::Far] {
            let m = Metrics::of(level, scale);
            assert_eq!(m.height % 2.0, 0.0, "{level:?} at {scale}");
            assert!((m.height - [66.0, 40.0, 26.0][level as usize] * scale).abs() <= 1.0);
        }
    }
}

/// Sizes of lines of text of `chars` characters, roughly as a font gives them.
fn measured(m: &Metrics, name: usize, subs: [Option<usize>; 2]) -> TextSize {
    let size = |chars: usize, pt: f32| Vec2::new(chars as f32 * pt * 0.55, pt * 1.36);
    TextSize {
        name: size(name, m.name_pt),
        subs: subs.map(|s| s.map(|chars| size(chars, m.sub_pt))),
    }
}

#[test]
fn a_full_plate_keeps_its_text_clear_of_the_picture_and_the_badges() {
    for scale in [SCALE, 1.0] {
        let m = Metrics::of(Level::Full, scale);
        for (name, subs) in [
            (11, [Some(11), Some(15)]),
            (3, [None, None]),
            (2, [Some(2), None]),
            (40, [Some(60), Some(20)]),
        ] {
            for badges in 0..=5 {
                let text = measured(&m, name, subs);
                let l = layout(Level::Full, &m, &text, badges);
                assert_eq!(l.size.y, m.height);
                let photo = l.photo.expect("a picture");
                // the picture sits inside the pill's left end
                assert!((photo.center.y - m.height * 0.5).abs() < 0.01);
                assert!(photo.center.x - photo.radius >= 0.0 && photo.radius * 2.0 < m.height);
                // every line starts right of it and ends before the right edge
                let right_of_photo = photo.center.x + photo.radius;
                let lines = [Some((l.name, text.name))]
                    .into_iter()
                    .chain(l.subs.iter().zip(text.subs).map(|(p, s)| p.zip(s)))
                    .flatten();
                let mut bottom = 0.0;
                for (pos, size) in lines {
                    assert!(pos.x >= right_of_photo + m.gap - 0.01);
                    assert!(pos.x + size.x <= l.size.x - m.pad_right + 0.01);
                    // stacked, never one over the other
                    assert!(pos.y + size.y * 0.15 >= bottom - size.y * 0.15, "{name} {subs:?}");
                    bottom = pos.y + size.y;
                    assert!(pos.y > 0.0 && bottom < m.height);
                }
                // the status dot stays on the picture's rim
                let dot = l.dot.expect("a dot");
                assert!(((dot.center - photo.center).length() - photo.radius).abs() < dot.radius);
                // the badges float over the top edge, clear of the picture,
                // inside the plate's width, and end above the capitals of
                // the name (0.36 em above the middle of its line)
                let capitals = l.name.y + text.name.y * 0.5 - 0.36 * m.name_pt;
                for (i, badge) in l.badges[..badges].iter().enumerate() {
                    assert!(badge.left() >= right_of_photo && badge.right() <= l.size.x);
                    assert!(badge.top() < 0.0 && badge.bottom() > 0.0);
                    if subs[0].is_some() && subs[1].is_some() {
                        assert!(badge.bottom() <= capitals + 0.5, "{} > {capitals}", badge.bottom());
                    }
                    if i > 0 {
                        assert!((badge.left() - l.badges[i - 1].right() - m.badge_gap).abs() < 0.01);
                    }
                }
                if badges > 0 {
                    assert!((l.size.x - l.badges[badges - 1].right() - 21.0 * scale).abs() < 0.01);
                }
            }
        }
    }
}

#[test]
fn a_compact_pill_puts_its_badges_after_the_name() {
    let m = Metrics::of(Level::Compact, SCALE);
    let text = measured(&m, 11, [None, None]);
    let bare = layout(Level::Compact, &m, &text, 0);
    assert!(bare.photo.is_none() && bare.dot.is_none());
    assert!((bare.size.x - (m.pad_left + text.name.x + m.pad_right).ceil()).abs() < 0.01);
    let two = layout(Level::Compact, &m, &text, 2);
    assert!(two.size.x > bare.size.x + 2.0 * m.badge);
    for badge in &two.badges[..2] {
        assert!(badge.left() >= two.name.x + text.name.x + m.gap - 0.01);
        assert!(badge.right() <= two.size.x - m.pad_right + 0.01);
        assert!(badge.top() >= 0.0 && badge.bottom() <= m.height);
        assert!((badge.center().y - m.height * 0.5).abs() < 0.01);
    }
    // the far pill: the name between its two ends
    let far = Metrics::of(Level::Far, SCALE);
    let name = measured(&far, 11, [None, None]);
    let l = layout(Level::Far, &far, &name, 0);
    assert!((l.size.x - (2.0 * far.pad_left + name.name.x).ceil()).abs() < 0.01);
    assert!(l.size.y < m.height);
}

#[test]
fn the_halo_follows_the_voice_level() {
    let mut e = VoiceEnvelope::default();
    assert_eq!(e.halo(1.0), None, "no halo before anyone speaks");
    // in the channel but silent, even with some noise on the line: no
    // halo, so no green outline either
    for _ in 0..60 {
        e.update(0.4, false, 1.0 / 60.0);
    }
    assert_eq!(e.halo(1.0), None);
    // loud: the widest halo of the mockup's rule
    for _ in 0..60 {
        e.update(1.0, true, 1.0 / 60.0);
    }
    let loud = e.halo(1.0).expect("speaking");
    assert!((loud.blur - 14.0).abs() < 0.1 && (loud.spread - 2.0).abs() < 0.05 && (loud.alpha - 0.85).abs() < 0.01);
    assert!(loud.gate > 0.99, "the outline is fully green: {}", loud.gate);
    // a whisper: the tightest one, still visible
    let mut quiet = VoiceEnvelope::default();
    for _ in 0..60 {
        quiet.update(0.0, true, 1.0 / 60.0);
    }
    let whisper = quiet.halo(1.0).expect("speaking");
    assert!((whisper.blur - 3.0).abs() < 0.01 && whisper.spread.abs() < 0.01 && (whisper.alpha - 0.30).abs() < 0.01);
    // in points: everything follows the plate's scale but the opacity
    let scaled = e.halo(0.5).expect("speaking");
    assert!((scaled.blur - loud.blur * 0.5).abs() < 0.01 && (scaled.alpha - loud.alpha).abs() < 0.001);
}

#[test]
fn the_voice_level_rises_fast_falls_slowly_and_ends_dark() {
    let dt = 1.0 / 60.0;
    let mut e = VoiceEnvelope::default();
    // three frames in, the halo is mostly there
    for _ in 0..3 {
        e.update(0.9, true, dt);
    }
    assert!(e.level > 0.6 && e.gate > 0.6, "{e:?}");
    for _ in 0..30 {
        e.update(0.9, true, dt);
    }
    let top = e.level;
    // a gap between two syllables: three frames later it has barely moved
    for _ in 0..3 {
        e.update(0.1, true, dt);
    }
    assert!(e.level > top * 0.8, "{} after {top}", e.level);
    // never a jump from one frame to the next
    let mut previous = e.halo(1.0).expect("speaking").alpha;
    for frame in 0..240 {
        let speaking = frame < 20;
        e.update(if frame % 7 < 3 { 1.0 } else { 0.0 }, speaking, dt);
        let alpha = e.halo(1.0).map_or(0.0, |h| h.alpha);
        assert!((alpha - previous).abs() < 0.2, "frame {frame}: {previous} -> {alpha}");
        previous = alpha;
    }
    // silent for good: no halo at all, not a faint one
    assert_eq!(e.halo(1.0), None);
    assert_eq!(e, VoiceEnvelope::default());
}

#[test]
fn the_voice_envelope_does_not_depend_on_the_frame_rate() {
    let run = |fps: u32| {
        let mut e = VoiceEnvelope::default();
        for _ in 0..fps / 2 {
            e.update(0.7, true, 1.0 / fps as f32);
        }
        for _ in 0..fps / 10 {
            e.update(0.0, false, 1.0 / fps as f32);
        }
        e
    };
    let (slow, fast) = (run(30), run(240));
    assert!((slow.level - fast.level).abs() < 0.01 && (slow.gate - fast.gate).abs() < 0.01);
}

#[test]
fn raw_voice_levels_cover_the_range_of_a_voice() {
    assert_eq!(voice_level(0.0), 0.0);
    assert_eq!(voice_level(f32::NAN), 0.0);
    assert_eq!(voice_level(0.9), 1.0);
    let middle = voice_level(0.4);
    assert!(middle > 0.2 && middle < 0.8);
}

#[test]
fn typing_dots_rise_one_after_the_other() {
    // at rest: dim, on the line
    assert_eq!(typing_dot(0.0, 0), (0.3, 0.0));
    // 30 % into its 1.1 s: lit and lifted
    let (opacity, lift) = typing_dot(0.33, 0);
    assert!((opacity - 1.0).abs() < 0.001 && (lift + 2.5).abs() < 0.001);
    // the next dot is 0.18 s behind
    let (next, first) = (typing_dot(0.33 + 0.18, 1), typing_dot(0.33, 0));
    assert!((next.0 - first.0).abs() < 0.001 && (next.1 - first.1).abs() < 0.001);
    assert!(typing_dot(0.33, 2).0 < typing_dot(0.33, 1).0);
    // back at rest for the last 40 %, and again every 1.1 s
    assert_eq!(typing_dot(0.9, 0), (0.3, 0.0));
    let (a, b) = (typing_dot(0.2, 0), typing_dot(0.2 + 1.1 * 7.0, 0));
    assert!((a.0 - b.0).abs() < 0.001 && (a.1 - b.1).abs() < 0.001);
}

#[test]
fn pictures_are_asked_a_few_a_second() {
    let mut budget = PictureBudget::default();
    let granted = |budget: &mut PictureBudget, now: f64| (0..10).filter(|_| budget.take(now)).count() as u32;
    assert_eq!(granted(&mut budget, 10.0), PICTURES_PER_SECOND);
    assert_eq!(granted(&mut budget, 10.5), 0);
    assert_eq!(granted(&mut budget, 11.0), PICTURES_PER_SECOND);
}

#[test]
fn only_complete_plates_ask_for_their_picture_nearest_first() {
    let ctx = egui::Context::default();
    let mut plates = Plates::default();
    let wanted = |n: u128, distance: f32| Tag {
        picture: Picture::Wanted,
        ..tag(n, distance, content())
    };
    // far first, as the plates are drawn: ten complete plates, two farther
    let mut tags: Vec<Tag<'_>> = vec![wanted(100, 60.0), wanted(101, 30.0)];
    tags.extend((0..10).rev().map(|n| wanted(n, 3.0 + n as f32)));
    frame(&ctx, &mut plates, &tags, 5.0);
    let first: HashSet<Uuid> = (0..PICTURES_PER_SECOND as u128).map(Uuid::from_u128).collect();
    assert_eq!(plates.wanted_pics, first);
    // the same ones stay wanted until their picture is there; the next
    // ones wait for the next second
    plates.wanted_pics.clear();
    frame(&ctx, &mut plates, &tags, 5.4);
    assert_eq!(plates.wanted_pics, first);
    plates.wanted_pics.clear();
    frame(&ctx, &mut plates, &tags, 6.1);
    assert_eq!(plates.wanted_pics.len(), 2 * PICTURES_PER_SECOND as usize);
    // a resident without a picture, or a compact plate, asks for nothing
    assert!(!plates.wanted_pics.contains(&Uuid::from_u128(100)));
    assert!(!plates.wanted_pics.contains(&Uuid::from_u128(101)));
    let mut none = Plates::default();
    frame(&ctx, &mut none, &[tag(1, 3.0, content())], 5.0);
    assert!(none.wanted_pics.is_empty());
}

#[test]
fn a_long_name_is_elided_on_the_plate_only() {
    let ctx = egui::Context::default();
    let long = "Maximilienne ♪ the Wandering Northern Aurora Wolf of the Seven Seas and Beyond".repeat(2);
    let tags = [tag(
        1,
        3.0,
        Content {
            name: long.clone(),
            username: Some(long.clone()),
            detail: Some(detail()),
            ..content()
        },
    )];
    let (rects, shapes) = frame(&ctx, &mut Plates::default(), &tags, 1.0);
    let m = Metrics::of(Level::Full, SCALE);
    let widest = m.pad_left + m.photo + m.gap + m.max_text + m.pad_right;
    assert!(rects[0].1.width() <= widest + 1.0, "{} > {widest}", rects[0].1.width());
    let mut elided = 0;
    let mut figure = false;
    walk(&shapes, &mut |shape| {
        if let egui::Shape::Text(t) = shape {
            elided += usize::from(t.galley.elided);
            figure |= t.galley.job.text.contains("245 000") && !t.galley.elided;
        }
    });
    assert_eq!(elided, 2, "the name and the username");
    assert!(figure, "the complexity is never the elided part");
    // what the plate was given is untouched
    assert_eq!(tags[0].content.name, long);
}

#[test]
fn a_plate_glides_from_one_level_to_the_next() {
    let ctx = egui::Context::default();
    let mut plates = Plates::default();
    let height = |level| Metrics::of(level, SCALE).height;
    let at = |plates: &mut Plates, distance: f32, now: f64| frame(&ctx, plates, &[tag(1, distance, content())], now).0[0].1;
    // a new plate is at its level at once
    let full = at(&mut plates, 5.0, 1.0);
    assert_eq!(full.height(), height(Level::Full));
    assert_eq!(full.center_bottom().y.round(), 300.0);
    // walking away: between the two sizes for a moment, then compact
    assert_eq!(at(&mut plates, 25.0, 1.0).height(), height(Level::Full));
    let moving = at(&mut plates, 25.0, 1.0 + LEVEL_EASE * 0.5);
    assert!(moving.height() < height(Level::Full) && moving.height() > height(Level::Compact));
    assert!(moving.width() < full.width());
    assert_eq!(moving.center_bottom().y.round(), 300.0, "it stays on its anchor");
    let compact = at(&mut plates, 25.0, 2.0);
    assert_eq!(compact.height(), height(Level::Compact));
    // and it does not come back until well inside the threshold
    assert_eq!(at(&mut plates, FULL_DISTANCE - 0.5, 3.0).height(), height(Level::Compact));
    assert_eq!(at(&mut plates, FULL_DISTANCE - 0.5, 9.0).height(), height(Level::Compact));
    at(&mut plates, FULL_DISTANCE - LEVEL_MARGIN - 0.5, 9.5);
    assert_eq!(
        at(&mut plates, FULL_DISTANCE - LEVEL_MARGIN - 0.5, 10.5).height(),
        height(Level::Full)
    );
}

#[test]
fn only_the_nearest_plates_blur_the_world() {
    let ctx = egui::Context::default();
    let mut plates = Plates::default();
    // far pills never do
    let mut tags: Vec<Tag<'_>> = (0..5).map(|n| tag(500 + n, 60.0, content())).collect();
    let (_, shapes) = frame(&ctx, &mut plates, &tags, 1.0);
    assert_eq!(blurred(&shapes), 0);
    tags.extend((0..MAX_GLASS as u128 + 12).rev().map(|n| tag(n, 2.0 + n as f32 * 0.4, content())));
    let (rects, shapes) = frame(&ctx, &mut Plates::default(), &tags, 1.0);
    assert_eq!(rects.len(), tags.len());
    assert_eq!(blurred(&shapes), MAX_GLASS);
    // each plate says where it stands: the scene hides what is behind it
    assert_eq!(marks(&shapes), tags.len());
}

#[test]
fn the_plate_names_follow_the_relation_switch() {
    let ctx = egui::Context::default();
    let p = Theme::default().palette();
    let friend = "Orion le Grand";
    let tags = [
        tag(1, 3.0, content()),
        tag(
            2,
            4.0,
            Content {
                name: friend.into(),
                role: Role::Friend,
                friend: true,
                ..content()
            },
        ),
        tag(
            3,
            5.0,
            Content {
                role: Role::Blocked,
                blocked: true,
                ..content()
            },
        ),
        tag(
            4,
            6.0,
            Content {
                role: Role::Linden,
                ..content()
            },
        ),
        tag(
            5,
            30.0,
            Content {
                role: Role::Own,
                ..content()
            },
        ),
    ];
    let saved = crate::ui::colors::get();
    let odd = [255, 0, 255, 255];
    let changed = crate::settings::ColorSettings {
        tag_me: odd,
        tag_friend: odd,
        tag_muted: odd,
        tag_linden: odd,
        tag_match: odd,
        tag_mismatch: odd,
        ..saved
    };
    // the colors by relation off (the default): every name in ink, whatever
    // the colors of the preferences
    crate::ui::colors::set(&crate::settings::ColorSettings {
        relation_names: false,
        ..changed
    });
    let (_, off) = frame(&ctx, &mut Plates::default(), &tags, 1.0);
    // switched on: the colors of the preferences
    crate::ui::colors::set(&crate::settings::ColorSettings {
        relation_names: true,
        ..changed
    });
    let (_, on) = frame(&ctx, &mut Plates::default(), &tags, 1.0);
    crate::ui::colors::set(&saved);
    assert!(!colors(&off).contains(&Color32::from_rgb(255, 0, 255)));
    assert_eq!(text_color(&off, friend), Some(p.ink));
    assert_eq!(text_color(&on, friend), Some(Color32::from_rgb(255, 0, 255)));
    // the other lines keep the theme's colors either way
    for shapes in [&off, &on] {
        assert_eq!(text_color(shapes, "Nova Exemple"), Some(p.muted));
        assert_eq!(text_color(shapes, "Loups du Nord"), Some(p.teal));
    }
}

#[test]
fn a_picture_fills_its_disc_without_being_stretched() {
    let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    assert_eq!(paint::cover([256, 256]), whole);
    let wide = paint::cover([512, 256]);
    assert_eq!((wide.left(), wide.right(), wide.top(), wide.bottom()), (0.25, 0.75, 0.0, 1.0));
    let tall = paint::cover([128, 512]);
    assert_eq!((tall.top(), tall.bottom(), tall.width()), (0.375, 0.625, 1.0));
    // a broken size does not divide by zero
    assert!(paint::cover([0, 0]).is_finite());
}

#[test]
fn the_plate_under_the_pointer_shows_full_while_the_option_is_on() {
    for level in [Level::Full, Level::Compact, Level::Far] {
        assert_eq!(hover_level(level, true, true), Level::Full);
        // off, or another plate: the level the plate had
        assert_eq!(hover_level(level, true, false), level);
        assert_eq!(hover_level(level, false, true), level);
    }
    // the pointer just gone: the plate stays full a moment
    assert!(hover_holds(10.0, 10.0));
    assert!(hover_holds(10.0, 10.0 + HOVER_LINGER * 0.99));
    assert!(!hover_holds(10.0, 10.0 + HOVER_LINGER + 0.01));
}

#[test]
fn a_hovered_pill_grows_even_with_full_plates_for_friends_only() {
    let ctx = egui::Context::default();
    let mut plates = Plates {
        friends_only_full: true,
        expand_on_hover: true,
        ..Plates::default()
    };
    // a stranger close up (capped by friends only) and one far away
    let tags = [tag(1, 30.0, content()), tag(2, 4.0, content())];
    let level = |plates: &Plates, id: u128| plates.entries.get(&Uuid::from_u128(id)).map(|e| e.level);
    frame(&ctx, &mut plates, &tags, 1.0);
    assert_eq!(level(&plates, 1), Some(Level::Compact));
    assert_eq!(level(&plates, 2), Some(Level::Compact));
    for id in [1, 2] {
        plates.hovered = Some(Uuid::from_u128(id));
        frame(&ctx, &mut plates, &tags, 2.0);
        assert_eq!(level(&plates, id), Some(Level::Full));
        // gone: still full within the linger, back after it
        plates.hovered = None;
        frame(&ctx, &mut plates, &tags, 2.0 + HOVER_LINGER * 0.5);
        assert_eq!(level(&plates, id), Some(Level::Full));
        frame(&ctx, &mut plates, &tags, 3.0);
        assert_eq!(level(&plates, id), Some(Level::Compact));
    }
    // the option off: hovering changes nothing
    plates.expand_on_hover = false;
    plates.hovered = Some(Uuid::from_u128(1));
    frame(&ctx, &mut plates, &tags, 4.0);
    assert_eq!(level(&plates, 1), Some(Level::Compact));
}
