//! Aurora's search window driven like a user: the field searched, chips,
//! rows, rating toggles and "See all" clicked, on the demo directory's
//! answers.

use super::aurora::Kind;
use super::{SearchAction, SearchUi, WebEnv};
use crate::ui::place_details::Ctx;
use crate::world::World;
use crate::world::search::{Key, RowIcon, SearchSettings, Status, Tab};
use egui::{Pos2, Rect, Shape, Vec2};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

fn world() -> World {
    World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()))
}

/// What the app does with the window's actions, the demo answering for the
/// grid, until nothing is left to ask.
fn apply(world: &mut World, actions: Vec<SearchAction>) {
    let access = world.agent_access();
    let mut sent = Vec::new();
    for a in actions {
        match a {
            SearchAction::Find(request) => world.search.find(request, access, Instant::now()),
            SearchAction::Select(tab, key) => world.search.select(tab, key),
            SearchAction::Page(tab, forward) => world.search.turn_page(tab, forward),
            SearchAction::WantParcel(id) => sent.push(aurora_net::NetCommand::ParcelInfoRequest(id)),
            SearchAction::WantClassified(id) => sent.push(aurora_net::NetCommand::ClassifiedInfoRequest(id)),
            SearchAction::RefreshGroup(id) => sent.push(aurora_net::NetCommand::GroupProfileRequest(id)),
            _ => {}
        }
    }
    for _ in 0..8 {
        sent.extend(world.search.take_commands());
        if sent.is_empty() {
            break;
        }
        for cmd in std::mem::take(&mut sent) {
            for ev in crate::demo::demo_reply(&cmd) {
                world.apply(ev);
            }
        }
    }
}

/// The texts drawn, with their centres.
type Texts = Vec<(Pos2, String)>;

struct Window {
    st: SearchUi,
    settings: SearchSettings,
    ctx: egui::Context,
    screen_size: Vec2,
    classic: bool,
    images: HashMap<uuid::Uuid, egui::TextureHandle>,
    wanted_images: HashSet<uuid::Uuid>,
    shapes: Vec<egui::epaint::ClippedShape>,
    location_drop: Option<crate::ui::places::drag::LocationDrag>,
}

impl Window {
    fn new(text: &str) -> Window {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut st = SearchUi::default();
        st.open_all();
        st.aurora.text = text.to_owned();
        Window {
            st,
            settings: SearchSettings::default(),
            ctx,
            screen_size: Vec2::new(1400.0, 900.0),
            classic: false,
            images: HashMap::new(),
            wanted_images: HashSet::new(),
            shapes: Vec::new(),
            location_drop: None,
        }
    }

    /// One frame with these pointer events; the texts drawn and the actions.
    fn frame(&mut self, world: &World, events: Vec<egui::Event>) -> (Texts, Vec<SearchAction>) {
        let p = crate::theme::Theme::default().palette();
        let pics = HashMap::new();
        let mut emoji = crate::ui::emoji::Emoji::load();
        let (mut names, mut wanted, mut groups) = (HashSet::new(), HashSet::new(), HashSet::new());
        let browser = aurora_media::BrowserSettings::default();
        let web = WebEnv {
            paths: None,
            browser: &browser,
            cookie: None,
            home: None,
            session: None,
            anonymous: false,
        };
        let mut actions = Vec::new();
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen_size)),
                events,
                ..Default::default()
            },
            |ui| {
                let mut c = Ctx {
                    p: &p,
                    world,
                    images: &self.images,
                    emoji: &mut emoji,
                    want_names: &mut names,
                    wanted_images: &mut wanted,
                    want_groups: &mut groups,
                };
                let mut open = true;
                actions.extend(
                    self.st
                        .show(ui.ctx(), &mut c, &pics, &mut self.settings, &web, self.classic, &mut open),
                );
                self.location_drop = crate::ui::places::drag::location_world_drag(ui.ctx());
                crate::ui::places::drag::ghost(ui.ctx(), &p);
            },
        );
        output.textures_delta.clear();
        fn collect(shape: &Shape, out: &mut Texts) {
            match shape {
                // a galley laid out from the right starts left of its pos
                Shape::Text(t) => out.push((t.pos + t.galley.rect.center().to_vec2(), t.galley.text().to_owned())),
                Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let mut texts = Vec::new();
        output.shapes.iter().for_each(|s| collect(&s.shape, &mut texts));
        self.wanted_images = wanted;
        self.shapes = output.shapes;
        (texts, actions)
    }

    /// Draw twice (the window's first frame lays it out), then apply.
    fn settle(&mut self, world: &mut World) -> Texts {
        let mut texts = Vec::new();
        for _ in 0..3 {
            let (t, actions) = self.frame(world, Vec::new());
            apply(world, actions);
            texts = t;
        }
        texts
    }

    /// Click the first text drawn that reads `label`; the actions of the
    /// click, already applied.
    fn click(&mut self, world: &mut World, label: &str) -> Vec<SearchAction> {
        let texts = self.settle(world);
        let at = texts
            .iter()
            .find(|(_, t)| t == label)
            .map(|(pos, _)| *pos)
            .unwrap_or_else(|| panic!("{label:?} not drawn in {:?}", texts.iter().map(|t| &t.1).collect::<Vec<_>>()));
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let (_, mut actions) = self.frame(world, vec![egui::Event::PointerMoved(at), press(true)]);
        let (_, more) = self.frame(world, vec![press(false)]);
        actions.extend(more);
        apply(world, actions.clone());
        actions
    }
}

fn has(texts: &Texts, text: &str) -> bool {
    texts.iter().any(|(_, t)| t == text)
}

#[test]
fn search_places_drag_to_the_world_before_details_load_in_both_styles_and_all() {
    use crate::ui::places::drag::{PlaceDrag, Target};
    for (classic, tab, all) in [
        (false, Tab::Places, false),
        (true, Tab::Places, false),
        (false, Tab::Places, true),
        (false, Tab::Land, false),
        (true, Tab::Land, false),
        (false, Tab::People, false),
        (true, Tab::People, false),
        (false, Tab::Groups, false),
        (true, Tab::Groups, false),
    ] {
        let mut world = world();
        let mut w = Window::new("aurora");
        w.classic = classic;
        if !all {
            w.st.open_tab(tab);
        }
        w.st.set_text(tab, "aurora");
        let requests = if classic {
            w.st.request(tab, &w.settings).into_iter().collect()
        } else {
            w.st.aurora_requests(&w.settings)
        };
        apply(&mut world, requests.into_iter().map(SearchAction::Find).collect());
        world.profiles.parcels.clear();
        let list = world.search.list(tab).expect("directory reply");
        let row = list.sorted()[0].clone();
        let selected = list.selected;
        let mut texts = Vec::new();
        // Deliberately withhold ParcelInfoReply throughout the gesture.
        for _ in 0..4 {
            texts = w.frame(&world, Vec::new()).0;
        }
        let start = texts.iter().find(|(_, t)| *t == row.name).expect("visible row").0;
        let button = |pos, pressed, modifiers| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        let (_, mut actions) = w.frame(
            &world,
            vec![egui::Event::PointerMoved(start), button(start, true, Default::default())],
        );
        actions.extend(w.frame(&world, vec![egui::Event::PointerMoved(start + egui::vec2(16.0, 0.0))]).1);
        if !matches!(tab, Tab::Places | Tab::Land) {
            assert!(egui::DragAndDrop::payload::<PlaceDrag>(&w.ctx).is_none(), "non-place row: {tab:?}");
            continue;
        }
        let Key::Id(id) = row.key else { panic!("parcel identity") };
        let cargo = egui::DragAndDrop::payload::<PlaceDrag>(&w.ctx).expect("drag from result without parcel details");
        assert_eq!(cargo.target, Target::Parcel(id));
        assert_eq!(cargo.name, row.name);
        assert!(cargo.favorite.is_none() && cargo.thumbnail.is_none());
        assert_eq!(cargo.target.item(), None, "directory places are not inventory cargo");
        let end = egui::pos2(1370.0, 850.0);
        w.frame(&world, vec![egui::Event::PointerMoved(end)]);
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        actions.extend(
            w.frame(&world, vec![egui::Event::ModifiersChanged(shift), button(end, false, shift)])
                .1,
        );
        let drop = w.location_drop.take().expect("world receives the release");
        assert_eq!(drop.target, Target::Parcel(id));
        assert!(drop.released && drop.shift);
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, SearchAction::Select(..) | SearchAction::Teleport(..)))
        );
        assert_eq!(world.search.list(tab).expect("list").selected, selected);
        assert!(world.inventory.items.is_empty() && world.profiles.parcels.is_empty());
    }
}

fn groups_window(world: &mut World, classic: bool) -> Window {
    let mut w = Window::new("aurora");
    w.classic = classic;
    w.st.open_tab(Tab::Groups);
    apply(
        world,
        w.st.aurora_requests(&w.settings).into_iter().map(SearchAction::Find).collect(),
    );
    w
}

#[test]
fn search_groups_join_uses_the_selected_group_in_both_styles() {
    for classic in [false, true] {
        let mut world = world();
        let mut w = groups_window(&mut world, classic);
        let group = crate::demo::search::boreal();
        apply(&mut world, vec![SearchAction::Select(Tab::Groups, Key::Id(group))]);
        w.settle(&mut world);
        let label = aurora_i18n::tr!("search-group-join");
        let actions = w.click(&mut world, label);
        assert!(actions.iter().any(|a| matches!(a, SearchAction::JoinGroup(id) if *id == group)));
        world.search.groups.get_mut(&group).expect("profile").open_enrollment = false;
        let actions = w.click(&mut world, label);
        assert!(
            !actions.iter().any(|a| matches!(a, SearchAction::JoinGroup(_))),
            "invite-only group"
        );
    }
}

#[test]
fn search_groups_visible_rows_request_and_draw_insignias_once() {
    let mut world = world();
    let mut w = groups_window(&mut world, false);
    let mut requests = 0;
    for _ in 0..4 {
        let (_, actions) = w.frame(&world, Vec::new());
        let count = actions.iter().filter(|a| matches!(a, SearchAction::RefreshGroup(_))).count();
        assert!(count <= 6, "visible detail requests remain bounded");
        requests += count;
        apply(&mut world, actions);
    }
    assert!(requests > 0, "visible rows request their group profiles after the initial layout");
    w.settle(&mut world);
    let image = crate::demo::TEX_SNAPSHOT;
    assert!(w.wanted_images.contains(&image));
    let texture = w.ctx.load_texture(
        "group-insignia",
        egui::ColorImage::filled([8, 8], egui::Color32::WHITE),
        Default::default(),
    );
    let texture_id = texture.id();
    w.images.insert(image, texture);
    let texts = w.settle(&mut world);
    fn images(shape: &Shape, id: egui::TextureId) -> usize {
        match shape {
            Shape::Mesh(mesh) => usize::from(mesh.texture_id == id),
            Shape::Rect(rect) => usize::from(rect.brush.as_ref().is_some_and(|brush| brush.fill_texture_id == id)),
            Shape::Vec(shapes) => shapes.iter().map(|s| images(s, id)).sum(),
            _ => 0,
        }
    }
    let drawn: usize = w.shapes.iter().map(|s| images(&s.shape, texture_id)).sum();
    assert!(drawn >= 5, "the five groups with insignias draw their received image: {drawn}");
    assert!(has(&texts, "AB"), "the group without an insignia retains its initials");
    let (_, actions) = w.frame(&world, Vec::new());
    assert!(
        !actions.iter().any(|a| matches!(a, SearchAction::RefreshGroup(_))),
        "no repeated requests"
    );
}

#[test]
fn search_groups_counts_follow_profile_then_complete_members_in_both_styles() {
    use aurora_net::groups::{GroupMember, LastSeen, MembersPage};
    for classic in [false, true] {
        let mut world = world();
        let mut w = groups_window(&mut world, classic);
        let group = crate::demo::search::boreal();
        apply(&mut world, vec![SearchAction::Select(Tab::Groups, Key::Id(group))]);
        w.settle(&mut world);
        let mut profile = world.search.groups[&group].clone();
        profile.members = 3;
        world.apply(aurora_net::NetEvent::GroupProfile(Box::new(profile)));
        let texts = w.settle(&mut world);
        assert!(has(&texts, &aurora_i18n::tr!("search-group-members", count = 3_i64)));
        if !classic {
            assert_eq!(
                texts
                    .iter()
                    .filter(|(_, t)| t == &aurora_i18n::tr!("search-group-members", count = 3_i64))
                    .count(),
                2
            );
        } else {
            assert!(has(&texts, "3"), "the list column also uses the profile");
        }
        for total in [2, 0] {
            world.group_data.refresh(group, false);
            let request = world
                .group_data
                .take_commands()
                .into_iter()
                .find_map(|cmd| match cmd {
                    aurora_net::NetCommand::GroupMembersRequest { request, .. } => Some(request),
                    _ => None,
                })
                .expect("members request");
            if total > 0 {
                world.apply(aurora_net::NetEvent::GroupMembers(Box::new(MembersPage {
                    group,
                    request,
                    total,
                    members: Vec::new(),
                    whole: true,
                })));
                assert_eq!(world.group_member_count(group), Some(3), "a partial list is not the count");
            }
            let members = (0..total)
                .map(|i| GroupMember {
                    id: uuid::Uuid::from_u128(i as u128 + 1),
                    contribution: 0,
                    powers: 0,
                    title: String::new(),
                    last_seen: LastSeen::Online,
                    is_owner: false,
                })
                .collect();
            world.apply(aurora_net::NetEvent::GroupMembers(Box::new(MembersPage {
                group,
                request,
                total,
                members,
                whole: true,
            })));
            let texts = w.settle(&mut world);
            assert!(has(&texts, &aurora_i18n::tr!("search-group-members", count = i64::from(total))));
            let list = world.search.list_mut(Tab::Groups).expect("groups");
            list.sort = Some((1, false));
            assert_eq!(list.sorted()[0].key, Key::Id(group), "the current count is used for sorting too");
            if !classic {
                assert_eq!(
                    texts
                        .iter()
                        .filter(|(_, t)| t == &aurora_i18n::tr!("search-group-members", count = i64::from(total)))
                        .count(),
                    2
                );
            } else {
                assert!(has(&texts, &total.to_string()));
            }
        }
        apply(
            &mut world,
            w.st.aurora_requests(&w.settings).into_iter().map(SearchAction::Find).collect(),
        );
        let list = world.search.list(Tab::Groups).expect("groups");
        let row = list.rows.iter().find(|r| r.key == Key::Id(group)).expect("group row");
        assert!(
            matches!(row.values.first(), Some(crate::world::search::Value::Int(0))),
            "a new directory reply preserves the current count"
        );
    }
}

#[test]
fn search_groups_navigation_wraps_without_overlap_at_the_minimum_width() {
    let mut world = world();
    let mut w = Window::new("page");
    w.screen_size = Vec2::new(750.0, 700.0);
    apply(
        &mut world,
        w.st.aurora_requests(&w.settings).into_iter().map(SearchAction::Find).collect(),
    );
    let texts = w.settle(&mut world);
    let labels = [
        "search-aurora-all",
        "common-people",
        "common-places",
        "search-tab-events",
        "common-groups",
        "search-tab-classifieds",
        "search-tab-land",
        "search-tab-web",
    ];
    let mut rects = Vec::new();
    for key in labels {
        let label = aurora_i18n::text(key);
        assert!(has(&texts, label));
        let shape = w
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == label => Some((s.clip_rect, t.galley.rect.translate(t.pos.to_vec2()))),
                _ => None,
            })
            .unwrap_or_else(|| panic!("chip {label}"));
        assert!(shape.0.contains_rect(shape.1), "chip {label} is fully visible: {shape:?}");
        assert!(
            rects.iter().all(|r: &Rect| !r.intersects(shape.1)),
            "chip {label} overlaps another chip"
        );
        rects.push(shape.1);
    }
    w.click(&mut world, aurora_i18n::tr!("search-tab-web"));
    assert_eq!(w.st.aurora_kind(), Kind::Web, "the last chip remains clickable");
}

#[test]
fn all_then_a_chip_rows_toggles_and_the_web() {
    let mut world = world();
    let mut w = Window::new("aurora");
    let requests = w.st.aurora_requests(&w.settings);
    assert_eq!(requests.len(), 5);
    apply(&mut world, requests.into_iter().map(SearchAction::Find).collect());
    let texts = w.settle(&mut world);
    // the sections of All, their first rows, "See all" for the long ones
    let places = aurora_i18n::text("common-places").to_uppercase();
    let people = aurora_i18n::text("common-people").to_uppercase();
    assert!(has(&texts, &places) && has(&texts, &people), "{texts:?}");
    assert!(has(&texts, aurora_i18n::tr!("search-aurora-see-all")));
    // a chip shows the list All searched, without a new search
    let actions = w.click(&mut world, aurora_i18n::tr!("common-places"));
    assert!(actions.iter().all(|a| !matches!(a, SearchAction::Find(_))), "no second search");
    assert_eq!(w.st.aurora_kind(), Kind::Places);
    let texts = w.settle(&mut world);
    let list = world.search.list(Tab::Places).expect("places");
    assert!(list.rows.len() >= 3);
    for row in &list.rows {
        assert!(has(&texts, &row.name), "{} is a row", row.name);
        // the rows asked for their parcel: snapshot, region and rating
        let Key::Id(id) = row.key else { panic!("a parcel") };
        assert!(world.profiles.parcels.contains_key(&id), "{} asked for its parcel", row.name);
    }
    // a click on the second row selects it and shows its details
    let second = list.sorted()[1].clone();
    let actions = w.click(&mut world, &second.name);
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, SearchAction::Select(Tab::Places, k) if *k == second.key))
    );
    assert_eq!(world.search.list(Tab::Places).and_then(|l| l.selected), Some(second.key));
    let texts = w.settle(&mut world);
    assert!(has(&texts, aurora_i18n::tr!("common-teleport-3")), "the place's actions");
    // a rating toggle is saved and searches again
    let actions = w.click(&mut world, aurora_i18n::tr!("common-moderate"));
    assert!(w.settings.aurora_ratings.mature || !world.agent_access().can_access_mature());
    if world.agent_access().can_access_mature() {
        assert!(actions.iter().any(|a| matches!(a, SearchAction::SettingsChanged)));
        assert!(actions.iter().any(|a| matches!(a, SearchAction::Find(r) if r.tab() == Tab::Places)));
    }
    // back to All, "See all" opens the section's chip
    w.click(&mut world, aurora_i18n::tr!("search-aurora-all"));
    assert_eq!(w.st.aurora_kind(), Kind::All);
    w.click(&mut world, aurora_i18n::tr!("search-aurora-see-all"));
    assert_ne!(w.st.aurora_kind(), Kind::All);
    // land sales are searched by their filters when their chip is chosen
    let actions = w.click(&mut world, aurora_i18n::tr!("search-tab-land"));
    assert!(actions.iter().any(|a| matches!(a, SearchAction::Find(r) if r.tab() == Tab::Land)));
    assert_eq!(world.search.list(Tab::Land).map(|l| &l.status), Some(&Status::Found));
    // the web page is asked with the field's text
    w.click(&mut world, aurora_i18n::tr!("search-tab-web"));
    assert_eq!(w.st.aurora_kind(), Kind::Web);
    assert_eq!(w.st.web_query(false), "aurora");
    assert_eq!(w.st.web_query(true), "", "the Legacy tab opens the page without a query");
}

#[test]
fn a_place_is_for_sale_by_its_row_not_by_its_parcel_price() {
    let mut world = world();
    let mut w = Window::new("aurora");
    apply(
        &mut world,
        w.st.aurora_requests(&w.settings).into_iter().map(SearchAction::Find).collect(),
    );
    w.click(&mut world, aurora_i18n::tr!("common-places"));
    w.settle(&mut world);
    let list = world.search.list(Tab::Places).expect("places").clone();
    let selected = list.sorted()[0].clone();
    w.click(&mut world, &selected.name);
    // ParcelInfoReply keeps the last asking price of parcels no longer for
    // sale (L$10000 for one never sold)
    for row in &list.rows {
        let Key::Id(id) = row.key else { panic!("a parcel") };
        world.profiles.parcels.get_mut(&id).expect("asked").sale_price = 10_000;
    }
    let texts = w.settle(&mut world);
    let for_sale = |r: &crate::world::search::Row| r.icon == RowIcon::ForSale;
    let expected = list.rows.iter().filter(|r| for_sale(r)).count() + usize::from(for_sale(&selected));
    let drawn = texts
        .iter()
        .filter(|(_, t)| t == aurora_i18n::tr!("search-aurora-for-sale"))
        .count();
    assert_eq!(drawn, expected, "only the rows the search reply marked for sale, and their details");
}

#[test]
fn enter_searches_and_an_empty_field_says_what_to_do() {
    let mut world = world();
    let mut w = Window::new("");
    let texts = w.settle(&mut world);
    assert!(has(&texts, aurora_i18n::tr!("search-aurora-start")), "nothing searched yet");
    w.st.aurora.text = "nova".into();
    w.st.focus = true;
    w.settle(&mut world);
    let (_, actions) = w.frame(
        &world,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    let finds: Vec<Tab> = actions
        .iter()
        .filter_map(|a| match a {
            SearchAction::Find(r) => Some(r.tab()),
            _ => None,
        })
        .collect();
    assert_eq!(finds.len(), 5, "Enter searches every directory of All");
    apply(&mut world, actions);
    let texts = w.settle(&mut world);
    assert!(has(&texts, "Nova Exemple") || world.search.list(Tab::People).is_some_and(|l| !l.rows.is_empty()));
}
