use super::*;
use crate::demo::destinations::{Fixture, history};
use crate::destination_guide::Source;

const ADULT: AgentAccess = AgentAccess { max: 42, preferred: 42 };
const GENERAL: AgentAccess = AgentAccess { max: 13, preferred: 13 };

/// The demo's guide, read as the window reads it.
fn demo_guide(access: AgentAccess) -> DestinationGuide {
    let ctx = egui::Context::default();
    let mut g = DestinationGuide::default();
    for _ in 0..3 {
        g.poll(&ctx, None, Some(Source::Demo(Fixture::Full)), access, true, Instant::now());
    }
    assert_eq!(g.load, Load::Ready);
    g
}

fn key(g: &DestinationGuide, category: &str, ratings: [bool; 3], access: AgentAccess) -> Key {
    Key {
        generation: g.generation,
        category: g.categories.iter().position(|c| c.key == category).expect("a demo category"),
        ratings: effective(ratings, access),
        sort: Sort::Population,
        hide_visited: false,
        visited: (0, 0),
        language: "en",
    }
}

fn names(g: &DestinationGuide, cache: &Cache) -> Vec<String> {
    cache.items.iter().map(|i| g.categories[i.cat].places[i.idx].name.clone()).collect()
}

#[test]
fn whats_hot_lists_by_population_within_the_ratings() {
    let g = demo_guide(ADULT);
    let none = HashSet::new();
    let cache = build(&g.categories, key(&g, guide::HOT, [true, true, false], ADULT), "", &none);
    let listed = names(&g, &cache);
    assert_eq!(listed.len(), 7);
    assert_eq!(listed[0], "Blue Neon Club", "the most people first");
    let pops: Vec<u32> = cache.items.iter().map(|i| g.categories[i.cat].places[i.idx].population).collect();
    assert!(pops.windows(2).all(|w| w[0] >= w[1]));
    // a place of What's Hot Now is shown under its own category
    let clubs = g.categories.iter().position(|c| c.key == "clubs").unwrap();
    assert_eq!(cache.items[0].home, clubs);
    assert!(cache.count_line.contains("7 places"), "{}", cache.count_line);
    // General only
    let cache = build(&g.categories, key(&g, guide::HOT, [true, false, false], ADULT), "", &none);
    assert_eq!(cache.items.len(), 4);
    assert!(cache.items.iter().all(|i| g.categories[i.cat].places[i.idx].rating == 13));
}

#[test]
fn a_rating_above_the_account_never_counts() {
    assert_eq!(effective([true, true, true], GENERAL), [true, false, false]);
    assert_eq!(effective([true, true, true], ADULT), [true, true, true]);
    let mature = AgentAccess { max: 21, preferred: 21 };
    assert_eq!(effective([true, true, true], mature), [true, true, false]);
    // the demo's adult club stays out of a General account's list, even
    // with every toggle on
    let g = demo_guide(ADULT);
    let none = HashSet::new();
    let cache = build(&g.categories, key(&g, "clubs", [true, true, true], GENERAL), "", &none);
    assert!(cache.items.is_empty(), "{:?}", names(&g, &cache));
    let cache = build(&g.categories, key(&g, "clubs", [true, true, true], ADULT), "", &none);
    assert_eq!(names(&g, &cache), ["Blue Neon Club", "Velvet Lounge"]);
}

#[test]
fn the_search_covers_the_whole_guide_once() {
    let g = demo_guide(ADULT);
    let none = HashSet::new();
    let cache = build(&g.categories, key(&g, "games", [true, true, false], ADULT), "  SAIKO ", &none);
    // Saiko Gardens is in two categories, Garden Concert's region matches
    let mut listed = names(&g, &cache);
    listed.sort();
    assert_eq!(listed, ["Garden Concert", "Saiko Gardens"]);
    assert!(cache.count_line.starts_with("Results"), "{}", cache.count_line);
}

#[test]
fn visited_places_are_marked_and_can_be_hidden() {
    let regions = visited_regions(&history());
    assert!(regions.contains("lantern isle") && regions.contains("pixelreach"), "{regions:?}");
    let g = demo_guide(ADULT);
    let mut k = key(&g, guide::HOT, [true, true, false], ADULT);
    let cache = build(&g.categories, k, "", &regions);
    let visited: Vec<String> = cache
        .items
        .iter()
        .filter(|i| i.visited)
        .map(|i| g.categories[i.cat].places[i.idx].name.clone())
        .collect();
    assert_eq!(visited, ["Lantern Isle", "Pixel Arcade"]);
    assert!(cache.items.iter().filter(|i| i.visited).all(|i| i.third.is_some()));
    k.hide_visited = true;
    let cache = build(&g.categories, k, "", &regions);
    assert_eq!(cache.items.len(), 5);
    assert!(cache.items.iter().all(|i| !i.visited));
}

#[test]
fn the_orders_and_an_event_end() {
    let g = demo_guide(ADULT);
    let none = HashSet::new();
    let mut k = key(&g, guide::HOT, [true, true, false], ADULT);
    k.sort = Sort::Name;
    let cache = build(&g.categories, k, "", &none);
    let listed = names(&g, &cache);
    let mut sorted = listed.clone();
    sorted.sort_by_key(|n| n.to_lowercase());
    assert_eq!(listed, sorted);
    k.sort = Sort::Newest;
    let cache = build(&g.categories, k, "", &none);
    let dates: Vec<&str> = cache
        .items
        .iter()
        .map(|i| g.categories[i.cat].places[i.idx].published.as_str())
        .collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]), "{dates:?}");
    // the demo's concert ends in a few days: its date shows
    let events = build(&g.categories, key(&g, "events", [true, true, false], ADULT), "", &none);
    assert!(events.items[0].until.as_deref().is_some_and(|u| u.starts_with("Until ")));
    assert!(events.items[0].location.starts_with("Saikomori · "));
}

#[test]
fn a_teleport_goes_the_way_of_the_guides_links() {
    let g = demo_guide(ADULT);
    let place = &g.categories.iter().find(|c| c.key == "nature").unwrap().places[0];
    let ctx = egui::Context::default();
    teleport(&ctx, place);
    show_on_map(&ctx, place);
    about_land(&ctx, place);
    let asked = crate::ui::context::take_requests(&ctx);
    assert!(
        matches!(&asked[..], [
            CtxAction::TeleportToPlace(r1, p1),
            CtxAction::ShowPlace(r2, _),
            CtxAction::ShowPlaceInfo(r3, _),
        ] if r1 == &place.region && r2 == &place.region && r3 == &place.region && p1.distance(place.pos.round()) < 0.01),
        "{asked:?}"
    );
}

#[test]
fn a_landmark_waits_for_the_places_region() {
    let g = demo_guide(ADULT);
    let place = &g.categories.iter().find(|c| c.key == "nature").unwrap().places[0];
    let history = history();
    let env = |here| Env {
        history: &history,
        access: ADULT,
        here,
    };
    let upper = place.region.to_uppercase();
    assert!(here(&env(Some(&upper)), place));
    assert!(!here(&env(Some("Elsewhere")), place));
    assert!(!here(&env(None), place));
}
