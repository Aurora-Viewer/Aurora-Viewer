//! Aurora's search window driven like a user: the field searched, chips,
//! rows, rating toggles and "See all" clicked, on the demo directory's
//! answers.

use super::aurora::Kind;
use super::{SearchAction, SearchUi, WebEnv};
use crate::ui::place_details::Ctx;
use crate::world::World;
use crate::world::search::{Key, SearchSettings, Status, Tab};
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
        }
    }

    /// One frame with these pointer events; the texts drawn and the actions.
    fn frame(&mut self, world: &World, events: Vec<egui::Event>) -> (Texts, Vec<SearchAction>) {
        let p = crate::theme::Theme::default().palette();
        let (images, pics) = (HashMap::new(), HashMap::new());
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
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let mut c = Ctx {
                    p: &p,
                    world,
                    images: &images,
                    emoji: &mut emoji,
                    want_names: &mut names,
                    wanted_images: &mut wanted,
                    want_groups: &mut groups,
                };
                let mut open = true;
                actions = self.st.show(ui.ctx(), &mut c, &pics, &mut self.settings, &web, false, &mut open);
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
    assert!(actions.iter().any(|a| matches!(a, SearchAction::Select(Tab::Places, k) if *k == second.key)));
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
