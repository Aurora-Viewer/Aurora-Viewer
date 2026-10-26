use super::*;
use aurora_net::profile::flags;

/// The texts of one frame of the window's body `width` wide, top to bottom
/// then left to right.
fn body_texts(world: &World, w: &mut Window, width: f32) -> Vec<String> {
    body_texts_with(world, w, width, true)
}

/// `body_texts`, with the friendship rights bar shown or hidden.
fn body_texts_with(world: &World, w: &mut Window, width: f32, friend_rights: bool) -> Vec<String> {
    let p = crate::theme::Theme::default().palette();
    let ctx = egui::Context::default();
    let mut emoji = Emoji::load();
    let browser = crate::media::browser_settings(std::path::Path::new("."), "fr");
    let web = WebContext {
        paths: None,
        browser: &browser,
        cookie: None,
        profile_base: None,
    };
    let (pics, images, banners) = (HashMap::new(), HashMap::new(), HashMap::new());
    let (mut want_names, mut wanted_pics, mut wanted_images) = (HashSet::new(), HashSet::new(), HashSet::new());
    let (mut actions, mut pick_picture) = (Vec::new(), None);
    let mut shapes = Vec::new();
    // the texts follow the server's, as `ProfileUi::show` makes them
    if let Some(pr) = world.profiles.get(&w.id) {
        w.about.sync(&pr.sl_about);
        if let Some(n) = &pr.notes {
            w.notes.sync(n);
        }
    }
    // a second frame: the first one measures
    for _ in 0..2 {
        let mut env = Env {
            p: &p,
            emoji: &mut emoji,
            pics: &pics,
            images: &images,
            banners: &banners,
            world,
            want_names: &mut want_names,
            web: &web,
            wanted_pics: &mut wanted_pics,
            wanted_images: &mut wanted_images,
            actions: &mut actions,
            pick_picture: &mut pick_picture,
            friend_rights,
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 1400.0))),
                ..Default::default()
            },
            |ui| body(ui, &mut env, w),
        );
        output.textures_delta.clear();
        shapes = output.shapes;
    }
    fn collect(shape: &egui::Shape, out: &mut Vec<(f32, f32, String)>) {
        match shape {
            egui::Shape::Text(t) => out.push((t.pos.y.round(), t.pos.x, t.galley.text().to_owned())),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    shapes.iter().for_each(|s| collect(&s.shape, &mut texts));
    // a row's texts sit within a few points of each other
    texts.sort_by(|a, b| {
        ((a.0 / 6.0).round(), a.1)
            .partial_cmp(&((b.0 / 6.0).round(), b.1))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    texts.into_iter().map(|(_, _, t)| t).collect()
}

/// `want` appears in `texts` in this order.
fn assert_in_order(texts: &[String], want: &[&str]) {
    let mut at = 0;
    for w in want {
        let found = texts[at..].iter().position(|t| t.contains(w));
        assert!(found.is_some(), "{w:?} after the previous one in {texts:?}");
        at += found.unwrap_or(0) + 1;
    }
}

fn friend_world() -> (World, Uuid) {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let id = Uuid::from_u128(0xF00D);
    world.social.friends.push(Friend {
        id,
        online: true,
        rights_given: rights::ONLINE_STATUS | rights::MAP_LOCATION,
        rights_has: rights::ONLINE_STATUS | rights::MAP_LOCATION | rights::MODIFY_OBJECTS,
    });
    world.profiles.insert(AvatarProfile {
        id,
        sl_about: "Musicienne du dimanche".into(),
        born: Some((aurora_net::profile::days_from_civil(2019, 4, 2) * 86_400) as f64),
        customer_type: "Premium".into(),
        flags: flags::IDENTIFIED,
        groups: vec![aurora_net::ProfileGroup {
            id: Uuid::from_u128(0x6E),
            name: "Aurora Builders".into(),
            insignia: Uuid::nil(),
        }],
        notes: Some("Rencontrée à la plage".into()),
        ..Default::default()
    });
    (world, id)
}

#[test]
fn a_friends_profile_reads_in_the_order_of_the_mockup() {
    let (world, id) = friend_world();
    let mut w = Window::new(id);
    let texts = body_texts(&world, &mut w, 960.0);
    let tr = aurora_i18n::text;
    // the date in the order of the language the tests run in
    let birth = world
        .profiles
        .get(&id)
        .and_then(|pr| profiles::birth_line(pr, false, now()))
        .unwrap_or_default();
    // who it is, then the tabs along the banner's bottom; the buttons sit
    // on the line of the picture, beside who it is
    assert!(texts.iter().any(|t| t == tr("ui-profile-aurora-conversation")));
    assert_in_order(
        &texts,
        &[
            &birth,
            tr("ui-profile-aurora-friend"),
            tr("world-profiles-premium"),
            tr("ui-profile-aurora-view-in-people"),
            tr("ui-profile-aurora-tab-profile"),
            tr("ui-profile-aurora-tab-picks"),
            tr("ui-profile-aurora-tab-classifieds"),
            tr("ui-profile-aurora-tab-first-life"),
            tr("ui-profile-aurora-tab-web"),
            tr("common-notes"),
            &tr("ui-profile-aurora-about").to_uppercase(),
            "Musicienne du dimanche",
        ],
    );
    // the right column
    assert_in_order(
        &texts,
        &[
            tr("common-notes"),
            &tr("ui-profile-aurora-infos").to_uppercase(),
            tr("ui-profile-aurora-account"),
            tr("ui-profile-aurora-partner"),
            tr("ui-profile-aurora-permissions"),
            tr("ui-profile-aurora-key"),
            &aurora_i18n::tr!("ui-profile-aurora-groups", count = 1).to_uppercase(),
            "Aurora Builders",
            &tr("ui-profile-aurora-private-notes").to_uppercase(),
            "Rencontrée à la plage",
        ],
    );
    assert!(texts.iter().all(|t| !t.contains(tr("ui-profile-aurora-add-friend"))));
    assert!(texts.iter().all(|t| t != tr("ui-profile-aurora-friendship")));
    // ending the friendship is in the "…" menu, giving a drop anywhere on
    // the window (#1089)
    assert!(texts.iter().all(|t| !t.contains(tr("ui-profile-aurora-remove-friend"))));
}

#[test]
fn a_friends_rights_can_be_hidden() {
    let (world, id) = friend_world();
    let tr = aurora_i18n::text;
    for width in [960.0, 380.0] {
        let mut w = Window::new(id);
        let texts = body_texts_with(&world, &mut w, width, false);
        assert!(texts.iter().all(|t| t != tr("ui-profile-aurora-permissions")), "{width}");
        assert!(
            texts.iter().any(|t| t.contains("Musicienne du dimanche")),
            "{width}: the cards stay"
        );
    }
    // a stranger's "Add as friend" is not a right: it stays, on its line
    // of Info
    let mut stranger = world;
    stranger.social.friends.clear();
    let mut w = Window::new(id);
    let texts = body_texts_with(&stranger, &mut w, 960.0, false);
    assert_in_order(
        &texts,
        &[
            tr("ui-profile-aurora-partner"),
            tr("ui-profile-aurora-friendship"),
            tr("ui-profile-aurora-add-friend"),
            tr("ui-profile-aurora-key"),
        ],
    );
}

#[test]
fn the_compact_window_stacks_its_cards() {
    let (world, id) = friend_world();
    let mut w = Window::new(id);
    let texts = body_texts(&world, &mut w, 380.0);
    let tr = aurora_i18n::text;
    // the tabs in a drop-down list, About and Info without their heads
    assert!(
        texts
            .iter()
            .any(|t| t == &aurora_i18n::tr!("ui-profile-aurora-sections", count = 6))
    );
    assert!(texts.iter().all(|t| t != tr("ui-profile-aurora-tab-picks")));
    assert!(texts.iter().all(|t| *t != tr("ui-profile-aurora-about").to_uppercase()));
    assert!(texts.iter().all(|t| t != tr("ui-profile-aurora-view-in-people")));
    assert_in_order(
        &texts,
        &[
            tr("ui-profile-aurora-tab-profile"),
            "Musicienne du dimanche",
            tr("ui-profile-aurora-account"),
            tr("ui-profile-aurora-permissions"),
            "Aurora Builders",
            &tr("ui-profile-aurora-private-notes").to_uppercase(),
        ],
    );
}

#[test]
fn our_own_profile_in_edit_mode() {
    let (mut world, id) = friend_world();
    world.agent_id = id;
    world.social.friends.clear();
    let mut w = Window::new(id);
    let texts = body_texts(&world, &mut w, 960.0);
    let tr = aurora_i18n::text;
    assert!(texts.iter().any(|t| t == tr("ui-profile-aurora-edit")));
    assert!(
        texts
            .iter()
            .all(|t| t != tr("common-notes") && t != tr("ui-profile-aurora-permissions") && t != tr("ui-profile-aurora-friendship"))
    );
    w.editing = true;
    w.about.text.push_str(" !");
    let texts = body_texts(&world, &mut w, 960.0);
    assert_in_order(
        &texts,
        &[
            tr("ui-profile-aurora-view-as-others"),
            tr("ui-profile-aurora-done"),
            &tr("ui-profile-aurora-visibility").to_uppercase(),
            tr("common-show-in-search"),
            &aurora_i18n::tr!("ui-profile-aurora-unsaved", parts = tr("ui-profile-aurora-part-bio")),
        ],
    );
}

fn facts(friend: bool, common: usize, blocked: bool) -> Facts {
    Facts {
        friend,
        common,
        blocked,
        ..Facts::default()
    }
}

#[test]
fn a_friends_badges_follow_the_mockup() {
    let p = crate::theme::Theme::default().palette();
    let f = Facts {
        account: Some(("Premium", false)),
        sets: vec![("Famille".into(), p.rose)],
        ..facts(true, 1, false)
    };
    assert_eq!(
        badges(&f, false),
        [
            Badge::Friend,
            Badge::Common(1),
            Badge::Account("Premium", false),
            Badge::Set("Famille".into(), p.rose),
            Badge::People,
        ]
    );
}

#[test]
fn a_blocked_stranger_and_our_own_profile() {
    // compact: the link to People moves to "…"
    assert_eq!(badges(&facts(false, 2, true), true), [Badge::Common(2), Badge::Blocked]);
    assert_eq!(badges(&facts(false, 0, false), false), [Badge::People]);
    // ourselves: no groups "in common", no link to ourselves in People
    let own = Facts {
        own: true,
        common: 2,
        account: Some(("Premium", false)),
        ..Facts::default()
    };
    assert_eq!(badges(&own, false), [Badge::Account("Premium", false)]);
}

#[test]
fn the_header_buttons() {
    use Act::*;
    let full = [Conversation, Call, OfferTeleport, Pay, Block, More];
    assert_eq!(header_acts(false, false, true, false), full);
    assert_eq!(
        header_acts(false, false, false, false),
        full,
        "the add button replaces the rights bar"
    );
    assert_eq!(header_acts(false, false, true, true), full);
    assert_eq!(
        header_acts(false, false, false, true),
        [Conversation, AddFriend, OfferTeleport, Pay, Block, More],
        "a stranger's compact row offers the friendship"
    );
    assert_eq!(header_acts(true, false, false, false), [Edit]);
    assert_eq!(header_acts(true, true, false, true), [ViewAsOthers, Done]);
}

#[test]
fn a_click_on_a_right_we_grant() {
    let all = rights::ONLINE_STATUS | rights::MAP_LOCATION | rights::MODIFY_OBJECTS;
    // hiding our online status hides where we are too
    assert_eq!(
        right_change(rights::ONLINE_STATUS | rights::MAP_LOCATION, rights::ONLINE_STATUS),
        RightChange::Send(0)
    );
    // showing where we are shows that we are online
    assert_eq!(
        right_change(0, rights::MAP_LOCATION),
        RightChange::Send(rights::ONLINE_STATUS | rights::MAP_LOCATION)
    );
    // our objects: asked first, either way
    assert_eq!(
        right_change(rights::ONLINE_STATUS, rights::MODIFY_OBJECTS),
        RightChange::Ask {
            rights: rights::ONLINE_STATUS | rights::MODIFY_OBJECTS,
            grant: true
        }
    );
    assert_eq!(
        right_change(all, rights::MODIFY_OBJECTS),
        RightChange::Ask {
            rights: rights::ONLINE_STATUS | rights::MAP_LOCATION,
            grant: false
        }
    );
}

#[test]
fn no_notes_tab_on_our_own_profile() {
    assert_eq!(tabs_of(false), [TAB_SL, TAB_PICKS, TAB_CLASSIFIEDS, TAB_RL, TAB_FEED, TAB_NOTES]);
    assert_eq!(tabs_of(true), [TAB_SL, TAB_PICKS, TAB_CLASSIFIEDS, TAB_RL, TAB_FEED]);
}

#[test]
fn the_unsaved_bar_names_what_is_edited() {
    let mut w = Window::new(Uuid::from_u128(7));
    w.about = Edit::with("Bonjour");
    assert!(unsaved_parts(&w).is_empty());
    assert!(!has_unsaved(&w));
    w.about.text.push('!');
    w.notes = Edit::with("a");
    w.notes.text = "b".into();
    assert_eq!(
        unsaved_parts(&w),
        [
            aurora_i18n::tr!("ui-profile-aurora-part-bio"),
            aurora_i18n::tr!("ui-profile-aurora-part-notes")
        ]
    );
    assert!(has_unsaved(&w));
    // Cancel puts the server's texts back
    revert(&mut w);
    assert!(!has_unsaved(&w));
    assert_eq!(w.about.text, "Bonjour");
}

#[test]
fn the_banner_is_a_few_soft_pixels_of_the_picture() {
    // 8 x 4: red on the left half, blue on the right
    let mut rgba = Vec::new();
    for _ in 0..4 {
        for x in 0..8 {
            rgba.extend_from_slice(if x < 4 { &[200, 40, 40, 255] } else { &[40, 40, 200, 255] });
        }
    }
    let img = banner_image(8, 4, &rgba);
    assert_eq!(img.size, [12, 6], "twelve pixels wide, the picture's proportions");
    let left = img.pixels[0].to_srgba_unmultiplied();
    let right = img.pixels[11].to_srgba_unmultiplied();
    assert!(left[0] > 200 && left[2] < 40, "a little more saturated: {left:?}");
    assert!(right[2] > 200 && right[0] < 40, "{right:?}");
    assert_eq!(left[3], 230, "slightly see-through");
    // a broken buffer gives a dark banner, not a panic
    let img = banner_image(8, 4, &[]);
    assert_eq!(img.size, [12, 6]);
}

struct PhotoFixture {
    ctx: egui::Context,
    world: World,
    profiles: super::super::ProfileUi,
    images: HashMap<Uuid, egui::TextureHandle>,
    pics: HashMap<Uuid, egui::TextureHandle>,
    emoji: Emoji,
    id: Uuid,
    classic: bool,
    time: f64,
}

impl PhotoFixture {
    fn new(classic: bool, compact: bool) -> Self {
        let (mut world, id) = friend_world();
        let ctx = egui::Context::default();
        let main = Uuid::from_u128(7001);
        let first = Uuid::from_u128(7002);
        let texture = |name| ctx.load_texture(name, egui::ColorImage::filled([128, 64], Color32::GRAY), Default::default());
        let (main_texture, first_texture) = (texture("main photo"), texture("first life photo"));
        world.profiles.insert(aurora_net::AvatarProfile {
            id,
            sl_image: main,
            fl_image: first,
            fl_about: "First life biography".into(),
            ..Default::default()
        });
        let mut profiles = super::super::ProfileUi::default();
        let mut window = Window::new(id);
        window.tab = TAB_RL;
        profiles.windows.push(window);
        if compact {
            profiles.demo(&ctx, id, "compact:4");
        }
        Self {
            ctx,
            world,
            profiles,
            images: HashMap::from([(main, main_texture.clone()), (first, first_texture)]),
            pics: HashMap::from([(id, main_texture)]),
            emoji: Emoji::load(),
            id,
            classic,
            time: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 1.0 / 60.0;
        let browser = crate::media::browser_settings(std::path::Path::new("."), "en");
        let web = WebContext {
            paths: None,
            browser: &browser,
            cookie: None,
            profile_base: None,
        };
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |host| {
                let actions = self.profiles.show(
                    host.ctx(),
                    &crate::theme::Theme::default().palette(),
                    &mut self.emoji,
                    &self.pics,
                    &self.images,
                    &mut self.world,
                    &mut HashSet::new(),
                    &web,
                    self.classic,
                    true,
                );
                assert!(actions.is_empty(), "previewing a picture never changes the profile");
            },
        );
        output.textures_delta.clear();
        output
    }

    fn settle(&mut self) -> egui::FullOutput {
        for _ in 0..7 {
            self.frame(Vec::new());
        }
        self.frame(Vec::new())
    }

    fn click(&mut self, point: Pos2) {
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
    }

    fn image_rect(&self) -> Rect {
        self.ctx
            .data(|data| data.get_temp(egui::Id::unique(("profile_photo_preview", self.id))))
            .expect("preview image")
    }
}

fn photo_shape(output: &egui::FullOutput, find: impl Fn(&egui::Shape) -> Option<Rect>) -> Rect {
    fn visit(shape: &egui::Shape, find: &impl Fn(&egui::Shape) -> Option<Rect>) -> Option<Rect> {
        find(shape).or_else(|| match shape {
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| visit(shape, find)),
            _ => None,
        })
    }
    output
        .shapes
        .iter()
        .find_map(|shape| visit(&shape.shape, &find))
        .expect("visible photo control")
}

#[test]
fn profile_photo_click_opens_the_right_asset_in_both_profile_styles_and_compact_layout() {
    for (classic, compact) in [(false, false), (false, true), (true, false)] {
        for first_life in [false, true] {
            let mut f = PhotoFixture::new(classic, compact);
            f.profiles.windows[0].tab = if first_life { TAB_RL } else { TAB_SL };
            let output = f.settle();
            let asset = Uuid::from_u128(if first_life { 7002 } else { 7001 });
            let texture = f.images[&asset].id();
            let rect = photo_shape(&output, |shape| match shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.calc_bounds()),
                egui::Shape::Rect(rect) if rect.brush.as_ref().is_some_and(|brush| brush.fill_texture_id == texture) => Some(rect.rect),
                _ => None,
            });
            f.click(rect.center());
            let photo = f.profiles.windows[0].photo.as_ref().expect("photo opens on click");
            assert_eq!(matches!(photo.slot, PictureSlot::FirstLife), first_life);
            assert_eq!(photo.asset, asset);
            f.settle();
            assert!((f.image_rect().aspect_ratio() - 2.0).abs() < 0.01);
        }
    }
}

#[test]
fn changing_or_removing_a_profile_photo_drops_old_pixels_and_retains_display_options() {
    let mut f = PhotoFixture::new(false, false);
    f.profiles.windows[0].open_photo(PictureSlot::FirstLife, Uuid::from_u128(7002));
    let view = &mut f.profiles.windows[0].photo.as_mut().expect("preview").view;
    view.display.ratio = 6;
    view.dimensions = Some((1024, 512));
    view.image = Some(f.images[&Uuid::from_u128(7002)].clone());
    let mut scene = crate::scene::Scene::new(std::env::temp_dir(), f.world.avatar_lib.clone());
    let replacement = Uuid::from_u128(7003);
    f.world.profiles.insert(aurora_net::AvatarProfile {
        id: f.id,
        fl_image: replacement,
        ..Default::default()
    });
    f.profiles.prepare_photos(&f.ctx, &mut scene, &f.world);
    let photo = f.profiles.windows[0].photo.as_ref().expect("replacement preview");
    assert_eq!(photo.asset, replacement);
    assert_eq!(photo.view.display.ratio, 6);
    assert!(photo.view.dimensions.is_none());
    assert!(photo.view.image.is_none());
    assert!(f.profiles.wanted_images.contains(&replacement));
    f.world.profiles.insert(aurora_net::AvatarProfile {
        id: f.id,
        ..Default::default()
    });
    f.profiles.prepare_photos(&f.ctx, &mut scene, &f.world);
    assert!(f.profiles.windows[0].photo.is_none());
}

#[test]
fn profile_photo_controls_change_ratio_and_use_full_dimensions_for_native_pixels() {
    let mut f = PhotoFixture::new(false, false);
    f.profiles.windows[0].open_photo(PictureSlot::FirstLife, Uuid::from_u128(7002));
    let full = f.ctx.load_texture(
        "full photo",
        egui::ColorImage::filled([1024, 512], Color32::GRAY),
        Default::default(),
    );
    let view = &mut f.profiles.windows[0].photo.as_mut().expect("preview").view;
    view.image = Some(full);
    view.dimensions = Some((1024, 512));
    let output = f.settle();
    let text = |output: &egui::FullOutput, label: &str| {
        photo_shape(output, |shape| match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                assert_eq!(
                    text.galley.rows.len(),
                    1,
                    "each preview control, including image dimensions, stays in one piece"
                );
                Some(Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
    };
    text(
        &output,
        &aurora_i18n::tr!("resource-preview-dimensions", width = "1024", height = "512"),
    );
    f.click(text(&output, aurora_i18n::tr!("resource-preview-original-ratio")).center());
    let output = f.settle();
    f.click(text(&output, "16:9").center());
    let output = f.settle();
    assert!((f.image_rect().aspect_ratio() - 16.0 / 9.0).abs() < 0.01);
    f.click(text(&output, aurora_i18n::tr!("resource-preview-actual")).center());
    f.settle();
    assert_eq!(f.image_rect().size(), Vec2::new(1024.0, 512.0));
    f.ctx.set_pixels_per_point(2.0);
    f.settle();
    assert_eq!(f.image_rect().size(), Vec2::new(512.0, 256.0));
    assert!(
        egui::DragAndDrop::payload::<crate::ui::inventory::InvDrag>(&f.ctx).is_none(),
        "photos are not inventory cargo"
    );
}
