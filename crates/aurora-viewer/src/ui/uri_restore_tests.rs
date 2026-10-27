//! Exercise explicit openings with the actual destination windows (#795).

use super::*;
use crate::demo::{DEMO_GROUP1, DEMO_LOUP};
use crate::world::{World, place_details::Source};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
enum Destination {
    Profile,
    Group,
    Places,
    StandalonePlace,
    Experience,
    Pay,
    Report,
    Marketplace,
}

#[test]
fn uri_destinations_restore_on_explicit_open_and_stay_minimized_otherwise() {
    let library = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    for classic in [false, true] {
        for destination in [
            Destination::Profile,
            Destination::Group,
            Destination::Places,
            Destination::StandalonePlace,
            Destination::Experience,
            Destination::Pay,
            Destination::Report,
            Destination::Marketplace,
        ] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let mut world = World::new(library.clone());
            let mut emoji = emoji::Emoji::load();
            let images = HashMap::new();
            let (mut names, mut wanted, mut groups) = (HashSet::new(), HashSet::new(), HashSet::new());
            let mut profile = profile::ProfileUi::default();
            let mut group = group_profile::GroupProfileUi::default();
            let mut places = places::PlacesUi::default();
            let mut details = place_details::PlaceDetailsUi::default();
            let mut pay = pay::PayUi::default();
            let mut report = report_abuse::ReportAbuseUi::default();
            let mut marketplace = marketplace::MarketplaceUi::default();
            let browser = aurora_media::BrowserSettings::default();
            let source = Source::Parcel(uuid::Uuid::from_u128(795));
            let mut frame = 0;
            let window = match destination {
                Destination::Profile if classic => format!("profile-{DEMO_LOUP}"),
                Destination::Profile => format!("profile-card-{DEMO_LOUP}"),
                Destination::Group => "group_profile".to_owned(),
                Destination::Places => "places".into(),
                Destination::StandalonePlace => {
                    let serial = world
                        .place_details
                        .open_window(source.clone(), &world.map, &mut world.landmarks, Instant::now());
                    format!("place-details-{serial}")
                }
                Destination::Experience => "experience_profile".into(),
                Destination::Pay if classic => "pay_classic".into(),
                Destination::Pay => "pay".into(),
                Destination::Report => "report_abuse".into(),
                Destination::Marketplace => marketplace::WINDOW.into(),
            };
            // Reopen the same target, as a repeated URI click does. The
            // rendering in between must leave an intentionally minimized
            // window alone, including while its data is still loading.
            for reopening in [false, true] {
                match destination {
                    Destination::Profile => profile.open(&mut world, DEMO_LOUP),
                    Destination::Group => group.open(&mut world, DEMO_GROUP1),
                    Destination::Places => {
                        world
                            .place_details
                            .open_panel(source.clone(), &world.map, &mut world.landmarks, Instant::now());
                        places.raise();
                    }
                    Destination::StandalonePlace => {
                        let serial = world
                            .place_details
                            .open_window(source.clone(), &world.map, &mut world.landmarks, Instant::now());
                        details.raise(serial);
                    }
                    Destination::Experience => world.experiences.open_profile(crate::demo::experience::EXPERIENCE),
                    Destination::Pay => pay.open(DEMO_LOUP, false),
                    Destination::Report => report.begin("Aurora Demo".into(), glam::Vec3::ZERO, String::new()),
                    Destination::Marketplace => marketplace.open(Some("https://marketplace.secondlife.com/p/example".into())),
                }
                let mut draw = || {
                    frame += 1;
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1200.0))),
                            time: Some(frame as f64 / 60.0),
                            ..Default::default()
                        },
                        |ui| {
                            let ctx = ui.ctx();
                            match destination {
                                Destination::Profile => {
                                    profile.show(
                                        ctx,
                                        &p,
                                        &mut emoji,
                                        &images,
                                        &images,
                                        &mut world,
                                        &mut names,
                                        &profile::WebContext {
                                            paths: None,
                                            browser: &browser,
                                            cookie: None,
                                            profile_base: None,
                                        },
                                        classic,
                                        true,
                                    );
                                }
                                Destination::Group | Destination::Places | Destination::StandalonePlace => {
                                    let mut c = place_details::Ctx {
                                        p: &p,
                                        world: &world,
                                        images: &images,
                                        emoji: &mut emoji,
                                        want_names: &mut names,
                                        wanted_images: &mut wanted,
                                        want_groups: &mut groups,
                                    };
                                    match destination {
                                        Destination::Group => {
                                            group.show(
                                                ctx,
                                                &mut c,
                                                &group_profile::Env {
                                                    avatar_pics: &images,
                                                    classic,
                                                    now: 0.0,
                                                    group_calls: false,
                                                },
                                            );
                                        }
                                        Destination::Places => {
                                            places.show(ctx, &mut c, &mut true, false, false, None);
                                        }
                                        _ => {
                                            details.show(ctx, &mut c);
                                        }
                                    }
                                }
                                Destination::Experience => {
                                    experience_profile::show(ctx, &p, &mut world);
                                }
                                Destination::Pay => {
                                    pay.show(ctx, &p, &world, (&HashMap::new(), &HashMap::new()), classic, true, 0);
                                }
                                Destination::Report => {
                                    report.show(ctx, &p, &mut world, "Second Life");
                                }
                                Destination::Marketplace => marketplace.show(
                                    ctx,
                                    &p,
                                    &marketplace::MarketWeb {
                                        paths: None,
                                        browser: &browser,
                                        cookie: None,
                                        home: None,
                                    },
                                ),
                            }
                        },
                    );
                    output.textures_delta.clear();
                };
                for _ in 0..8 {
                    draw();
                }
                assert!(
                    !widgets::floater_minimized(&ctx, &window),
                    "{destination:?}, classic={classic}, reopening={reopening}"
                );
                let rect = ctx
                    .memory(|m| m.area_rect(egui::Id::unique(&window)))
                    .expect("destination is drawn");
                assert!(rect.height() > 100.0, "expanded body: {destination:?} {rect:?}");
                widgets::minimize_floater(&ctx, &window);
                for _ in 0..4 {
                    draw();
                }
                assert!(
                    widgets::floater_minimized(&ctx, &window),
                    "ordinary frames must not restore {destination:?}"
                );
            }
        }
    }
}
