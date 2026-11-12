use super::*;
use aurora_net::search::{SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, SIM_ACCESS_PG};

fn access(max: u8, preferred: u8) -> AgentAccess {
    AgentAccess { max, preferred }
}

#[test]
fn the_maturity_asked_never_goes_above_the_account_and_its_preference() {
    assert_eq!(access_letters(access(SIM_ACCESS_ADULT, SIM_ACCESS_ADULT)), "gma");
    assert_eq!(access_letters(access(SIM_ACCESS_ADULT, SIM_ACCESS_MATURE)), "gm");
    assert_eq!(access_letters(access(SIM_ACCESS_MATURE, SIM_ACCESS_ADULT)), "gm");
    assert_eq!(access_letters(access(SIM_ACCESS_PG, SIM_ACCESS_ADULT)), "g");
    assert_eq!(access_letters(AgentAccess::default()), "g");
}

#[test]
fn the_addresses_are_those_of_the_guide_page() {
    assert_eq!(
        categories_url(),
        "https://worldaping.agni.lindenlab.com/v2/destinations/categories/bychan/viewer_dg/?format=json"
    );
    assert_eq!(
        places_url("editor", "gma"),
        "https://worldaping.agni.lindenlab.com/v2/destinations/editor/?format=json&sort_by=population&sort_dir=desc&flatten&maturity=gma"
    );
    // What's Hot Now: guide.js's limits, never above Moderate
    assert_eq!(
        places_url(HOT, "gma"),
        "https://worldaping.agni.lindenlab.com/v2/destinations/busiest/?format=json&sort_by=population&sort_dir=desc&flatten&maturity=gm&pagesize=32&maxpop=1000&minpop=20"
    );
    assert!(places_url(HOT, "g").contains("&maturity=g&"));
}

#[test]
fn categories_keep_their_order_and_only_names_safe_in_an_address() {
    let json = r#"[
        {"id": 1, "name": "Editors' Picks", "simple_name": "editor", "count": 20},
        {"id": 2, "name": "Bad", "simple_name": "../admin"},
        {"id": 3, "name": "Query", "simple_name": "a?b=c"},
        {"id": 4, "name": "Hot again", "simple_name": "busiest"},
        {"id": 5, "name": "Editors twice", "simple_name": "editor"},
        {"id": 6, "name": "", "simple_name": "nameless"},
        {"id": 7, "name": "  Music\u0007 & Clubs ", "simple_name": "floater_clubs"},
        {"id": 8, "simple_name": 42}
    ]"#;
    let cats = parse_categories(json.as_bytes()).expect("a list");
    assert_eq!(
        cats,
        vec![
            ("editor".to_owned(), "Editors' Picks".to_owned()),
            ("floater_clubs".to_owned(), "Music & Clubs".to_owned()),
        ]
    );
    assert!(parse_categories(b"{\"a\": 1}").is_err());
    assert!(parse_categories(b"not json").is_err());
    assert!(parse_categories(&vec![b' '; MAX_LIST_BYTES + 1]).is_err());
}

fn place_json(location: &str, extra: &str) -> String {
    format!(
        r#"[{{"name": "A place", "slug": "a-place", "description": "Line one\r\nLine two\u0000",
            "location": "{location}", "maturity": "M", "population": {{"current": 12}},
            "publish_date": "2026-09-08 19:00:00", "end_date": "2026-12-01 08:00:00",
            "assets": [{{"type": "fullsize", "data": "https://img.example.com/1/full.jpg"}},
                       {{"type": "midsize", "data": "https://img.example.com/1/mid.jpg"}}]{extra}}}]"#
    )
}

#[test]
fn a_place_is_read_as_its_teleport_link_is() {
    let places = parse_places(place_json("Lantern%20Isle/128/96/23", "").as_bytes()).expect("a list");
    assert_eq!(places.len(), 1);
    let p = &places[0];
    assert_eq!(p.name, "A place");
    assert_eq!(p.region, "Lantern Isle");
    assert_eq!(p.pos, glam::Vec3::new(128.0, 96.0, 23.0));
    assert_eq!(p.rating, SIM_ACCESS_MATURE);
    assert_eq!(p.population, 12);
    assert_eq!(p.description, "Line one\nLine two");
    assert_eq!(p.until, Some((2026, 12, 1)));
    assert_eq!(p.published, "2026-09-08 19:00:00");
    assert_eq!(p.card.as_deref(), Some("https://img.example.com/1/mid.jpg"));
    assert_eq!(p.photo.as_deref(), Some("https://img.example.com/1/full.jpg"));
    assert_eq!(p.teleport_link(), "secondlife:///app/teleport/Lantern%20Isle/128/96/23");
    let link = crate::slurl::place_link(&p.teleport_link()).expect("a teleport link");
    assert!(link.teleport);
    assert_eq!(link.location.region, "Lantern Isle");
}

#[test]
fn places_without_a_readable_location_or_name_are_left_out() {
    for bad in [
        "",
        "Region with space/1/2/3",
        "Region/1/2/3?x=1",
        "Region/1/2/3#a",
        "secondlife:///app/agent/x",
    ] {
        assert!(parse_places(place_json(bad, "").as_bytes()).expect("a list").is_empty(), "{bad:?}");
    }
    let long = "R".repeat(MAX_LOCATION + 1);
    assert!(parse_places(place_json(&long, "").as_bytes()).expect("a list").is_empty());
    let nameless = r#"[{"location": "Region/1/2/3"}]"#;
    assert!(parse_places(nameless.as_bytes()).expect("a list").is_empty());
}

#[test]
fn only_https_pictures_on_the_internet_are_kept() {
    for bad in [
        "http://img.example.com/a.jpg",
        "file:///c:/a.jpg",
        "https://127.0.0.1/a.jpg",
        "https://192.168.1.2/a.jpg",
        "https://localhost/a.jpg",
        "https://user:pw@img.example.com/a.jpg",
        "javascript:alert(1)",
    ] {
        assert_eq!(image_url(bad), None, "{bad}");
    }
    assert!(image_url(&format!("https://img.example.com/{}", "a".repeat(MAX_URL))).is_none());
    assert_eq!(
        image_url(" https://img.example.com/a.jpg ").as_deref(),
        Some("https://img.example.com/a.jpg")
    );
}

#[test]
fn texts_and_numbers_are_bounded() {
    let name = "N".repeat(500);
    let json = format!(
        r#"[{{"name": "{name}", "location": "Region/1/2/3", "maturity": "X", "population": {{"current": 99999999}},
             "end_date": "2099-12-31 08:00:00", "publish_date": "garbage"}}]"#
    );
    let places = parse_places(json.as_bytes()).expect("a list");
    let p = &places[0];
    assert_eq!(p.name.chars().count(), MAX_NAME);
    assert!(p.name.ends_with('…'));
    assert_eq!(p.rating, SIM_ACCESS_PG, "an unknown rating is read as General");
    assert_eq!(p.population, 100_000);
    assert_eq!(p.until, None, "2099 is the feed's never");
    assert_eq!(p.published, "");
    assert_eq!(p.card, None);
    // a slug seen twice is one place
    let twice = format!("[{0},{0}]", r#"{"name": "A", "slug": "a", "location": "Region/1/2/3"}"#);
    assert_eq!(parse_places(twice.as_bytes()).expect("a list").len(), 1);
    let many: Vec<String> = (0..MAX_PLACES + 20)
        .map(|i| format!(r#"{{"name": "P{i}", "slug": "p{i}", "location": "Region/1/2/3"}}"#))
        .collect();
    let many = format!("[{}]", many.join(","));
    assert_eq!(parse_places(many.as_bytes()).expect("a list").len(), MAX_PLACES);
    assert_eq!(
        parse_places(br#"[{"name": "A", "location": "Region/1/2/3", "maturity": "A"}]"#).expect("a list")[0].rating,
        SIM_ACCESS_ADULT
    );
}

#[test]
fn pictures_are_decoded_within_their_side_and_only_as_pictures() {
    let img = image::RgbaImage::from_pixel(640, 384, image::Rgba([10, 20, 30, 255]));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encoded");
    let card = decode(&png, CARD_SIDE).expect("decoded");
    assert_eq!(card.size, [320, 192]);
    let photo = decode(&png, PHOTO_SIDE).expect("decoded");
    assert_eq!(photo.size, [640, 384], "a smaller picture is kept whole");
    assert!(decode(b"<html>not a picture</html>", CARD_SIDE).is_none());
    let mut bmp = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(4, 4))
        .write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp)
        .expect("encoded");
    assert!(decode(&bmp, CARD_SIDE).is_none(), "only the feed's formats");
}

#[test]
fn the_demo_guide_is_read_by_the_same_parser() {
    use crate::demo::destinations::{Fixture, categories_json, picture, places_json};
    let cats = parse_categories(categories_json(Fixture::Full).as_bytes()).expect("categories");
    assert_eq!(cats.len(), 11);
    let hot = parse_places(places_json(Fixture::Full, HOT).as_bytes()).expect("places");
    assert!(hot.len() >= 5 && hot.windows(2).all(|w| w[0].population >= w[1].population));
    assert!(hot.iter().all(|p| p.population >= 20 && p.rating != SIM_ACCESS_ADULT));
    let events = parse_places(places_json(Fixture::Full, "events").as_bytes()).expect("places");
    assert!(events[0].until.is_some(), "the event has its last day");
    let url = events[0].card.clone().expect("a picture");
    let img = picture(&url, CARD_SIDE).expect("drawn");
    assert_eq!(img.size, [250, 150]);
    assert!(
        parse_places(places_json(Fixture::Empty, "editor").as_bytes())
            .expect("places")
            .is_empty()
    );
}

#[test]
fn nothing_is_asked_while_the_window_is_closed_and_the_demo_fills_it_when_open() {
    let ctx = egui::Context::default();
    let mut guide = DestinationGuide::default();
    let now = Instant::now();
    let source = Some(Source::Demo(crate::demo::destinations::Fixture::Full));
    let adult = access(SIM_ACCESS_ADULT, SIM_ACCESS_ADULT);
    guide.poll(&ctx, None, source, adult, false, now);
    assert_eq!(guide.load, Load::Idle);
    assert!(guide.categories.is_empty());
    // open: the categories, then every category's places
    for _ in 0..3 {
        guide.poll(&ctx, None, source, adult, true, now);
    }
    assert_eq!(guide.load, Load::Ready);
    assert_eq!(guide.categories[0].key, HOT);
    assert_eq!(guide.categories.len(), 12);
    assert!(guide.categories.iter().all(|c| c.feed == Feed::Ready));
    assert!(guide.updated.is_some());
    // asked again after REFRESH only, or by the button once idle
    let generation = guide.generation;
    guide.poll(&ctx, None, source, adult, true, now + Duration::from_secs(5));
    assert_eq!(guide.generation, generation);
    guide.refresh(now + Duration::from_secs(5));
    guide.poll(&ctx, None, source, adult, true, now + Duration::from_secs(5));
    assert_eq!(guide.generation, generation, "the button waits a few seconds after an answer");
    guide.refresh(now + MIN_MANUAL);
    guide.poll(&ctx, None, source, adult, true, now + MIN_MANUAL);
    assert!(guide.generation > generation);
    // another content rating: read again from scratch
    guide.poll(&ctx, None, source, access(SIM_ACCESS_ADULT, SIM_ACCESS_PG), true, now + MIN_MANUAL);
    assert_eq!(guide.letters, "g");
    // no feed on this grid: nothing
    let mut none = DestinationGuide::default();
    none.poll(&ctx, None, None, adult, true, now);
    assert_eq!(none.load, Load::Idle);
}

#[test]
fn a_failed_answer_keeps_what_was_shown() {
    let mut guide = DestinationGuide::default();
    let now = Instant::now();
    guide.asked = 1;
    guide.pending = 1;
    assert!(guide.apply(Msg::Categories(1, Ok(vec![("editor".into(), "Editors".into())])), None, now));
    // no network in the test: each category's answer comes as an error
    while let Ok(msg) = guide.rx.try_recv() {
        guide.apply(msg, None, now);
    }
    assert_eq!(guide.load, Load::Failed);
    assert!(guide.categories.iter().all(|c| c.feed == Feed::Failed));
    // an answer to an older request is dropped
    assert!(!guide.apply(Msg::Places(0, "editor".into(), Ok(Vec::new())), None, now));
}
