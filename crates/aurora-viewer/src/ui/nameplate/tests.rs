use super::*;
use crate::theme::Theme;

fn content() -> Content {
    Content {
        appearance: Default::default(),
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
        render_never: false,
        blocked: false,
        mark: None,
    }
}

fn detail() -> Detail {
    Detail {
        full: "Complexité : 245 000".into(),
        short: "245 000".into(),
        alert: true,
    }
}

#[test]
fn contact_visuals_preserve_distance_crowd_hover_and_bound_badges() {
    use crate::world::contact_sets::appearance::SetIcon;
    use crate::world::contact_sets::{Appearance, PlateSize};
    let mut c = content();
    c.friend = true;
    c.appearance = std::sync::Arc::new(Appearance {
        size: PlateSize::Reduced,
        replace_friend: true,
        icons: (0..20)
            .map(|_| SetIcon {
                set: "Example".into(),
                icon: 0,
                color: [0.3, 0.6, 0.9, 1.0],
            })
            .collect(),
        ..Default::default()
    });
    assert_eq!(preferred_level(Level::Full, &c, false), Level::Compact);
    assert_eq!(preferred_level(Level::Far, &c, false), Level::Far);
    assert_eq!(hover_level(preferred_level(Level::Full, &c, false), true, true), Level::Full);
    let b = badges(Level::Full, &c);
    assert!(!b.iter().any(|b| b == Badge::Friend));
    assert_eq!(b.len(), 9);
    assert!(b.iter().any(|b| b == Badge::More(12)));
    assert_eq!(badges(Level::Far, &c).len(), 0);
    std::sync::Arc::make_mut(&mut c.appearance).size = PlateSize::Expanded;
    assert_eq!(preferred_level(Level::Far, &c, true), Level::Far);
    assert_eq!(
        crowd_level(preferred_level(Level::Full, &c, true), MAX_FULL + 1, false),
        Level::Compact
    );
}

#[test]
fn contact_visuals_invalidate_surface_cache_without_a_geometry_change() {
    let ctx = egui::Context::default();
    Theme::default().apply(&ctx, 1.0);
    let mut plates = Plates::default();
    let mut c = content();
    c.appearance = std::sync::Arc::new(crate::world::contact_sets::Appearance {
        border: Some([1.0, 0.0, 0.0, 1.0]),
        ..Default::default()
    });
    let colours = |shapes: Vec<egui::Shape>| {
        shapes
            .into_iter()
            .filter_map(|s| match s {
                egui::Shape::Mesh(m) => Some(m.vertices.iter().map(|v| v.color).collect::<Vec<_>>()),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>()
    };
    let before = frame(&ctx, &mut plates, &[tag(1, 5.0, c.clone())], 1.0);
    std::sync::Arc::make_mut(&mut c.appearance).border = Some([0.0, 0.0, 1.0, 1.0]);
    let after = frame(&ctx, &mut plates, &[tag(1, 5.0, c)], 1.1);
    assert_eq!(before.0, after.0);
    assert_ne!(colours(before.1), colours(after.1));
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
            plates.paint(&painter, &p, &icons, tags, now, |id, rect, _| rects.push((id, rect)));
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
    // switched off, every name is in the theme's ink
    let off = crate::settings::ColorSettings {
        relation_names: false,
        ..Default::default()
    };
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

/// A team member's "Aurora Viewer" tag floats from the text's left: the
/// plate is as wide as it needs, and the floating badges start past it.
#[test]
fn the_staff_tag_has_its_room_over_the_top_edge() {
    for level in [Level::Full, Level::Compact] {
        let m = Metrics::of(level, SCALE);
        for (name, badges) in [(3, 0), (3, 5), (40, 2)] {
            let text = measured(&m, name, [None, None]);
            let lead = 70.0;
            let plain = layout(level, &m, &text, badges);
            let l = layout_with(level, &m, &text, badges, lead);
            assert!(l.size.x >= plain.size.x && l.size.y == plain.size.y);
            assert!(l.size.x >= l.text_x.0 + lead + m.pad_right - 0.01);
            if level == Level::Full {
                for badge in &l.badges[..badges] {
                    assert!(badge.left() >= l.text_x.0 + lead + m.gap - 0.01, "{name} {badges}");
                }
            }
        }
        // a long name already makes the room: nothing changes
        let text = measured(&m, 60, [None, None]);
        assert_eq!(layout_with(level, &m, &text, 0, 20.0), layout(level, &m, &text, 0));
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
fn text_size_preference_keeps_nameplate_lines_apart_and_inside_the_plate() {
    let ctx = egui::Context::default();
    crate::ui::fonts::apply(&ctx, crate::ui::fonts::INTER);
    for scale in [0.8, 1.0, 1.4, 2.0] {
        crate::theme::Theme::default().apply(&ctx, scale);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for level in [Level::Full, Level::Compact, Level::Far] {
                let m = Metrics::of(level, SCALE);
                let measure = |size| {
                    ui.painter()
                        .layout_no_wrap("Readable text".into(), egui::FontId::proportional(size), ui.visuals().text_color())
                        .size()
                };
                let text = TextSize {
                    name: measure(m.name_pt),
                    subs: if level == Level::Full {
                        [Some(measure(m.sub_pt)); 2]
                    } else {
                        [None; 2]
                    },
                };
                let layout = layout(level, &m, &text, 0);
                let mut bottom = layout.name.y + text.name.y;
                assert!(layout.name.y >= 0.0);
                for (pos, size) in layout.subs.iter().zip(text.subs) {
                    if let (Some(pos), Some(size)) = (pos, size) {
                        assert!(pos.y >= bottom - 0.01, "scale={scale} level={level:?}");
                        bottom = pos.y + size.y;
                    }
                }
                assert!(bottom <= layout.size.y, "scale={scale} level={level:?}");
            }
        });
        output.textures_delta.clear();
    }
}

#[test]
fn the_halo_follows_the_voice_level() {
    let mut e = VoiceEnvelope::default();
    assert_eq!(e.halo(), None, "no halo before anyone speaks");
    // in the channel but silent, even with some noise on the line: no
    // halo, so no green outline either
    for _ in 0..60 {
        e.update(0.4, false, 1.0 / 60.0);
    }
    assert_eq!(e.halo(), None);
    for _ in 0..60 {
        e.update(1.0, true, 1.0 / 60.0);
    }
    let loud = e.halo().expect("speaking");
    assert!(loud.level > 0.99 && loud.gate > 0.99, "{loud:?}");
    let mut quiet = VoiceEnvelope::default();
    for _ in 0..60 {
        quiet.update(0.0, true, 1.0 / 60.0);
    }
    let whisper = quiet.halo().expect("speaking");
    assert!(whisper.level < 0.01 && whisper.gate > 0.99, "a whisper still lights the plate");
}

/// Widest reach and strongest opacity of the voice's glow and ring.
fn voice_extent(v: &paint::VoiceLight) -> (f32, u8) {
    let glow = (v.glow.spread + v.glow.blur, v.glow.color.a());
    let ring = (v.ring.gap + v.ring.width, v.ring.color.a());
    (glow.0.max(ring.0), glow.1.max(ring.1))
}

#[test]
fn the_voice_light_grows_with_the_level_and_fades_with_the_gate() {
    let p = Theme::default().palette();
    let plain = p.ink.gamma_multiply(0.14);
    let full = Metrics::of(Level::Full, SCALE).height;
    let far = Metrics::of(Level::Far, SCALE).height;
    let look = |level: f32, gate: f32, height: f32| paint::voice_light(&p, Halo { level, gate, heat: 0.0 }, plain, height);
    let (loud, whisper) = (look(1.0, 1.0, full), look(0.0, 1.0, full));
    let ((reach_loud, alpha_loud), (reach_whisper, alpha_whisper)) = (voice_extent(&loud), voice_extent(&whisper));
    assert!(reach_loud > reach_whisper && alpha_loud > alpha_whisper);
    // a whisper still shows, and the plate lights up as soon as it speaks
    assert!(alpha_whisper > 60, "{alpha_whisper}");
    // fading out, everything goes with the gate
    let (_, alpha_fading) = voice_extent(&look(1.0, 0.1, full));
    assert!(alpha_fading < alpha_loud / 4);
    // never wider than a plate's height, tighter around a far pill
    assert!(reach_loud < full, "{reach_loud}");
    let (reach_far, _) = voice_extent(&look(1.0, 1.0, far));
    assert!(reach_far < reach_loud);
    // the plate's edge turns green with the gate
    let mesh::Border::Around(edge, _) = loud.border else {
        panic!("no outline")
    };
    assert!(edge.g() > plain.g());
    // the ring closes in on the plate as the voice starts
    let (starting, open) = (look(0.5, 0.2, full).ring, look(0.5, 1.0, full).ring);
    assert!(starting.gap > open.gap && starting.color.a() < open.color.a());
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
    let p = Theme::default().palette();
    let full = Metrics::of(Level::Full, SCALE).height;
    let alpha = |e: &VoiceEnvelope| {
        let light = e.halo().map(|h| paint::voice_light(&p, h, p.ink, full));
        light.map_or(0, |v| voice_extent(&v).1) as f32 / 255.0
    };
    let mut previous = alpha(&e);
    for frame in 0..240 {
        let speaking = frame < 20;
        e.update(if frame % 7 < 3 { 1.0 } else { 0.0 }, speaking, dt);
        let now = alpha(&e);
        assert!((now - previous).abs() < 0.2, "frame {frame}: {previous} -> {now}");
        previous = now;
    }
    // silent for good: no halo at all, not a faint one
    assert_eq!(e.halo(), None);
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
fn the_loud_ring_grows_with_the_voice_and_turns_red_only_when_overdriven() {
    let p = Theme::default().palette();
    let over = level_at_dbfs(SHOUT_DBFS);
    // from the speaking level to past the overdriven one, as the plates read it
    let raws: Vec<f32> = (0..=70).map(|i| aurora_voice::SPEAKING_AUDIO_LEVEL + i as f32 * 0.005).collect();
    let rings: Vec<_> = raws.iter().map(|&r| paint::loud_ring(&p, voice_level(r), overdrive(r))).collect();
    for (w, r) in rings.windows(2).zip(&raws[1..]) {
        let (a, b) = (w[0], w[1]);
        assert!(b.gap >= a.gap && b.width >= a.width, "{r}: {a:?} -> {b:?}");
        assert!(
            b.alpha >= a.alpha && b.glow >= a.glow && b.blur >= a.blur && b.edge >= a.edge,
            "{r}"
        );
        // the red never goes back as the voice rises
        assert!(overdrive(*r) >= overdrive(*r - 0.005), "{r}");
    }
    let green = |level: f32| paint::loud_ring(&p, level, 0.0).color;
    for (ring, &raw) in rings.iter().zip(&raws) {
        if raw <= over {
            assert_eq!(ring.color, green(voice_level(raw)), "green below the overdriven level: {raw}");
        }
    }
    // a clear range: a faint hairline on the plate, then a thick ring well out
    let (whisper, normal, shout) = (rings[2], rings[30], rings[66]);
    assert!(whisper.gap < 1.5 && whisper.width < 1.2 && whisper.alpha < 0.5 && whisper.glow < 0.06);
    assert!(normal.gap > 2.0 * whisper.gap && normal.width > 1.5 * whisper.width);
    assert!(shout.gap > 7.0 && shout.width > 3.0 && shout.glow > 0.4 && shout.blur > 20.0);
    // a shout is red, from the palette
    assert!(overdrive(raws[66]) >= 1.0);
    assert_eq!(shout.color, p.danger);
    // halfway, the amber of the palette rather than a mix of green and red
    assert_eq!(paint::loud_ring(&p, 1.0, 0.5).color, p.amber);
    assert!(overdrive(over) == 0.0 && overdrive(over + 0.015) > 0.4 && overdrive(f32::NAN) == 0.0);
}

#[test]
fn the_red_comes_and_goes_smoothly() {
    let dt = 1.0 / 60.0;
    let mut e = VoiceEnvelope::default();
    let over = level_at_dbfs(SHOUT_DBFS);
    // a normal voice: no red at all
    for _ in 0..60 {
        e.update_raw(0.46, true, dt);
    }
    assert_eq!(e.heat, 0.0);
    // a shout: red within a few frames, never in one
    e.update_raw(over + 0.05, true, dt);
    assert!(e.heat > 0.0 && e.heat < 0.5, "{}", e.heat);
    for _ in 0..10 {
        e.update_raw(over + 0.05, true, dt);
    }
    assert!(e.heat > 0.95, "{}", e.heat);
    // back to normal: it fades slower than it came
    let mut previous = e.heat;
    for _ in 0..12 {
        e.update_raw(0.46, true, dt);
        assert!(e.heat < previous && previous - e.heat < 0.1);
        previous = e.heat;
    }
    assert!(e.heat > 0.3);
    // silence for good: nothing left
    for _ in 0..120 {
        e.update_raw(0.0, false, dt);
    }
    assert_eq!(e, VoiceEnvelope::default());
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
fn a_plate_and_its_hit_rect_keep_the_world_anchor_through_pending_ui_zoom() {
    for native in [1.0, 1.5, 2.0] {
        let ctx = egui::Context::default();
        let mut plates = Plates::default();
        let p = Theme::default().palette();
        let icons = crate::ui::icons::Icons::default();
        let size = [1000.0, 600.0];
        for zoom in [1.0, 1.01, 1.384, 1.39, 1.0, 0.75, 2.0, 1.0] {
            let previous_ppp = ctx.pixels_per_point();
            ctx.set_zoom_factor(zoom);
            let mut raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(size[0], size[1]) / previous_ppp)),
                ..Default::default()
            };
            raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
            let mut output = ctx.run_ui(raw, |ui| {
                let proj = crate::ui::hud::Projector::for_ui(ui.ctx(), glam::Mat4::IDENTITY, size);
                let current = Tag {
                    anchor: proj
                        .project_name_tag(glam::Vec3::new(0.2, -0.2, 0.0), glam::Vec3::new(0.7, 0.3, 0.0))
                        .expect("visible avatar"),
                    ..tag(1, 5.0, content())
                };
                let painter = crate::ui::hud::world_painter(ui.ctx());
                let mut shown = false;
                plates.paint(&painter, &p, &icons, &[current], 1.0, |_, rect, _| {
                    shown = true;
                    let anchor = rect.center_bottom() * ui.ctx().pixels_per_point();
                    // Only the plate's intentional whole-pixel alignment may
                    // offset its anchor, never the previous frame's zoom.
                    assert!(
                        anchor.distance(Pos2::new(600.0, 210.0)) < 0.75,
                        "{native} DPI / {zoom} zoom: {anchor:?}"
                    );
                });
                assert!(shown, "the plate and its hit rectangle are drawn");
            });
            output.textures_delta.clear();
        }
    }
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

#[test]
fn a_hovered_plate_bounces_a_few_points_then_stands_still() {
    assert_eq!(bounce_grow(-0.01), 0.0);
    assert_eq!(bounce_grow(0.0), 0.0);
    // out first, a little in, then nothing
    let samples: Vec<f32> = (0..=100).map(|i| bounce_grow(BOUNCE * i as f64 / 100.0)).collect();
    let peak = samples.iter().copied().fold(f32::MIN, f32::max);
    let dip = samples.iter().copied().fold(f32::MAX, f32::min);
    assert!(peak > 1.5 && peak <= BOUNCE_GROW, "{peak}");
    assert!(dip < 0.0 && dip > -1.0, "{dip}");
    assert!(bounce_grow(BOUNCE * 0.1) > 0.0);
    assert_eq!(bounce_grow(BOUNCE), 0.0);
    assert_eq!(bounce_grow(BOUNCE + 1.0), 0.0);
}

#[test]
fn a_full_plate_lights_up_and_bounces_under_the_pointer() {
    let ctx = egui::Context::default();
    let mut plates = Plates::default();
    // complete already: the level stays, the plate still answers
    let tags = [tag(1, 4.0, content())];
    let id = Uuid::from_u128(1);
    let rect = |out: &(Vec<(Uuid, Rect)>, Vec<egui::Shape>)| out.0[0].1;
    let still = rect(&frame(&ctx, &mut plates, &tags, 1.0));
    assert_eq!(plates.entries[&id].hover, 0.0);
    plates.hovered = Some(id);
    let reached = rect(&frame(&ctx, &mut plates, &tags, 1.1));
    assert_eq!(plates.entries[&id].level, Level::Full);
    assert_eq!(plates.entries[&id].bounced, 1.1);
    // the pointer's rectangle does not move with the bounce
    assert_eq!(reached, still);
    frame(&ctx, &mut plates, &tags, 1.3);
    assert_eq!(plates.entries[&id].hover, 1.0);
    // staying on it does not bounce again; leaving fades the light
    frame(&ctx, &mut plates, &tags, 2.0);
    assert_eq!(plates.entries[&id].bounced, 1.1);
    plates.hovered = None;
    frame(&ctx, &mut plates, &tags, 3.0);
    assert_eq!(plates.entries[&id].hover, 0.0);
    // "Reduce animations": the light without the bounce
    plates.still = true;
    plates.hovered = Some(id);
    frame(&ctx, &mut plates, &tags, 4.0);
    assert_eq!(plates.entries[&id].bounced, 1.1);
    frame(&ctx, &mut plates, &tags, 4.2);
    assert_eq!(plates.entries[&id].hover, 1.0);
}

#[test]
fn a_kicked_spring_swings_then_comes_to_rest() {
    let mut s = fidget::Spring::default();
    s.kick(100.0, 280.0);
    let mut peak = 0.0f32;
    let mut crossed = false;
    for _ in 0..120 {
        s.step(1.0 / 60.0, 26.0, 0.2);
        peak = peak.max(s.x);
        crossed |= s.x < 0.0;
    }
    assert!(peak > 2.0, "{peak}");
    assert!(crossed, "it swings back past its rest");
    assert!(s.still(), "{s:?}");
    // a slow frame does not make it explode
    let mut slow = fidget::Spring::default();
    slow.kick(100.0, 280.0);
    slow.step(0.25, 26.0, 0.2);
    assert!(slow.x.abs() < 10.0 && slow.v.abs() < 280.0, "{slow:?}");
}

#[test]
fn kicks_in_a_row_add_up_within_a_bound() {
    let mut s = fidget::Spring::default();
    s.pump(95.0, 280.0);
    s.step(0.02, 26.0, 0.2);
    let once = s.v.abs();
    s.pump(95.0, 280.0);
    assert!(s.v.abs() > once, "pumped the way it goes");
    for _ in 0..20 {
        s.pump(95.0, 280.0);
    }
    assert!(s.v.abs() <= 280.0);
    // the wobble's surface stays within its stretch
    let mut w = fidget::Wobble::default();
    for _ in 0..50 {
        w.enter();
        w.step(0.005);
    }
    let grow = w.grow();
    assert!(grow.x.abs() <= 16.0 && grow.y.abs() <= 16.0, "{grow:?}");
    assert!(!w.still());
}

#[test]
fn a_growing_plate_goes_past_its_size_and_settles() {
    assert_eq!(fidget::overshoot(0.0), 0.0);
    assert!((fidget::overshoot(1.0) - 1.0).abs() < 1e-6);
    let peak = (0..=100).map(|i| fidget::overshoot(i as f32 / 100.0)).fold(0.0, f32::max);
    assert!(peak > 1.08 && peak < 1.2, "{peak}");
    assert!(grows(Level::Compact, Level::Full) && grows(Level::Far, Level::Compact));
    assert!(!grows(Level::Full, Level::Compact) && !grows(Level::Full, Level::Full));
}

#[test]
fn a_jiggly_plate_wobbles_each_time_the_pointer_comes_back() {
    let ctx = egui::Context::default();
    let mut plates = Plates {
        jiggle: true,
        ..Plates::default()
    };
    let tags = [tag(1, 4.0, content())];
    let id = Uuid::from_u128(1);
    frame(&ctx, &mut plates, &tags, 1.0);
    assert!(plates.entries[&id].wobble.still());
    // in, out and in again within the hover's linger: a kick each time in
    let mut t = 1.0;
    for _ in 0..3 {
        plates.hovered = Some(id);
        t += 0.05;
        frame(&ctx, &mut plates, &tags, t);
        assert!(!plates.entries[&id].wobble.still());
        plates.hovered = None;
        t += 0.05;
        frame(&ctx, &mut plates, &tags, t);
    }
    // no bounce of the plain plates on top of the jelly
    assert_eq!(plates.entries[&id].bounced, f64::NEG_INFINITY);
    // it settles
    for i in 1..200 {
        frame(&ctx, &mut plates, &tags, t + i as f64 * 0.02);
    }
    assert!(plates.entries[&id].wobble.still());
    // "Reduce animations" keeps it still
    plates.still = true;
    plates.hovered = Some(id);
    frame(&ctx, &mut plates, &tags, 10.0);
    assert!(plates.entries[&id].wobble.still());
}

#[test]
fn five_quick_clicks_start_a_combo_that_fades_after_a_pause() {
    let mut c = combo::Combo::default();
    for i in 0..4 {
        assert_eq!(c.click(i as f64 * 0.3, i + 1, false), combo::Hit::Counted);
        assert_eq!(c.alpha(i as f64 * 0.3), 0.0, "no counter before five");
    }
    assert_eq!(c.click(1.2, 5, false), combo::Hit::Started);
    assert_eq!(c.alpha(1.2), 1.0);
    for i in 6..=9 {
        assert_eq!(c.click(1.2 + i as f64 * 0.1, i, false), combo::Hit::Counted);
    }
    assert_eq!(c.click(2.2, 10, false), combo::Hit::Milestone);
    assert_eq!(c.streak(), 10);
    // the pause: still out, fading, then gone
    assert!(c.alive(2.2 + combo::HOLD));
    assert!(c.alpha(2.2 + combo::HOLD - 0.25) < 1.0);
    assert_eq!(c.alpha(2.2 + combo::HOLD + 0.01), 0.0);
    // a click after it starts a new run
    assert_eq!(c.click(10.0, 11, false), combo::Hit::Counted);
    assert_eq!(c.streak(), 1);
}

#[test]
fn slow_clicks_count_without_a_combo() {
    let mut c = combo::Combo::default();
    // within the hold of each other, but five take more than five seconds
    for i in 0..5 {
        assert_eq!(c.click(i as f64 * 1.4, i + 1, false), combo::Hit::Counted);
    }
    assert_eq!(c.alpha(5.6), 0.0);
    // a round hundred in all is a milestone once the counter is out
    let mut c = combo::Combo::default();
    for i in 0..4 {
        c.click(i as f64 * 0.1, 96 + i, false);
    }
    assert_eq!(c.click(0.4, 100, false), combo::Hit::Milestone);
    assert!(combo::milestone(25) && combo::milestone(300) && !combo::milestone(26) && !combo::milestone(150));
}

#[test]
fn a_click_counts_on_a_plate_on_screen_and_presses_it() {
    let ctx = egui::Context::default();
    let mut plates = Plates {
        click_combo: true,
        ..Plates::default()
    };
    let id = Uuid::from_u128(1);
    assert_eq!(plates.click(id, 0.5), None, "no plate, no click");
    frame(&ctx, &mut plates, &[tag(1, 4.0, content())], 1.0);
    assert_eq!(plates.click(id, 1.1), Some(combo::Hit::Counted));
    assert_eq!(plates.tally.get(id), 1);
    assert!(!plates.entries[&id].wobble.still(), "pressed in");
    for i in 0..4 {
        plates.click(id, 1.2 + i as f64 * 0.1);
    }
    assert_eq!(plates.tally.get(id), 5);
    // the counter is drawn while the combo lasts
    let (_, shapes) = frame(&ctx, &mut plates, &[tag(1, 4.0, content())], 1.6);
    let mut texts = Vec::new();
    walk(&shapes, &mut |s| {
        if let egui::Shape::Text(t) = s {
            texts.push(t.galley.text().to_owned());
        }
    });
    assert!(texts.iter().any(|t| t == "5"), "{texts:?}");
}

#[test]
fn the_click_counts_are_kept_by_resident() {
    let mut counts = HashMap::new();
    counts.insert(Uuid::from_u128(7), 42);
    counts.insert(Uuid::from_u128(9), 1);
    let bytes = combo::encode(&counts).expect("encoded");
    assert_eq!(combo::decode(&bytes).expect("decoded"), counts);
    // a key that is not an avatar id is left out, the others stay
    let mixed = br#"{"nope": 3, "00000000-0000-0000-0000-000000000007": 4}"#;
    assert_eq!(combo::decode(mixed).expect("decoded").get(&Uuid::from_u128(7)), Some(&4));
    assert!(combo::decode(b"[1, 2]").is_err());
}

#[test]
fn a_streak_heats_up_then_holds() {
    assert_eq!(combo::fervor(combo::START as u32), 0.0);
    assert!(combo::fervor(10) > 0.0 && combo::fervor(10) < combo::fervor(30));
    assert_eq!(combo::fervor(60), 1.0);
    assert_eq!(combo::fervor(5000), 1.0);
}

#[test]
fn a_friend_s_plate_takes_the_friends_color_unless_a_set_colors_it() {
    use crate::world::contact_sets::Appearance;
    let ctx = egui::Context::default();
    let p = Theme::default().palette();
    let icons = crate::ui::icons::Icons::default();
    let friend = Content { friend: true, ..content() };
    let in_set = |c: &Content| Content {
        appearance: std::sync::Arc::new(Appearance {
            background: Some([1.0, 0.5, 0.0, 1.0]),
            ..Default::default()
        }),
        ..c.clone()
    };
    let _ = ctx.run_ui(Default::default(), |ui| {
        let painter = ui.ctx().layer_painter(egui::LayerId::background());
        let mut env = paint::Env::new(&painter, &p, &icons, 0.0);
        assert_eq!(paint::surface_tint(&env, &friend), None, "the option off: the plain surface");
        assert_eq!(paint::surface_border(&env, &friend), None);
        let set = paint::surface_tint(&env, &in_set(&friend));
        env.friend = Some(Color32::from_rgb(0x81, 0x8C, 0xF8));
        let tint = paint::surface_tint(&env, &friend).expect("a friend is tinted");
        assert!(paint::surface_border(&env, &friend).is_some(), "and has a border of the color");
        assert_eq!(paint::surface_border(&env, &in_set(&friend)), None, "not over a set's background");
        assert_ne!(tint, env.tint);
        // another friends' color shows at once
        env.friend = Some(Color32::from_rgb(0x30, 0xC0, 0x60));
        assert_ne!(paint::surface_tint(&env, &friend), Some(tint));
        assert_eq!(paint::surface_tint(&env, &content()), None, "not a friend");
        assert_eq!(
            paint::surface_tint(
                &env,
                &Content {
                    blocked: true,
                    ..friend.clone()
                }
            ),
            None,
            "blocked"
        );
        assert_eq!(
            paint::surface_tint(&env, &in_set(&friend)),
            set,
            "the set's background alone, no mix"
        );
    });
}
