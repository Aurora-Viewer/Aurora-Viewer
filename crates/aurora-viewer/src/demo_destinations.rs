//! Demo of the Destinations window without a grid or a network:
//! `AURORA_DEMO_DESTINATIONS=compact|wide|legacy|empty` opens the window at
//! start on a guide made up here, in the feed's own shape (read by the same
//! parser as the real one): invented places, plain pictures drawn here, two
//! of them visited in the demo's teleport history.
//!
//! - `compact`: Aurora's window as it opens, its first place opened.
//! - `wide`: the same window widened, the first place previewed.
//! - `legacy`: the Legacy style for this run (the guide's web page; it needs
//!   the network, the browser's error page shows without it).
//! - `empty`: a guide with categories but no place.
//!
//! Any other value is a web page of the tester's choice and `sl` the
//! Second Life guide's page: a grid that has only a web guide, which both
//! styles show in the web window.

use crate::world::tphistory::HistoryItem;
use serde_json::{Value, json};
use std::sync::OnceLock;

/// What the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Compact,
    Wide,
    Legacy,
    Empty,
}

/// Which made-up guide is served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixture {
    Full,
    Empty,
}

pub fn mode() -> Option<Mode> {
    static MODE: OnceLock<Option<Mode>> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_DEMO_DESTINATIONS").ok()?.trim() {
        "compact" => Some(Mode::Compact),
        "wide" => Some(Mode::Wide),
        "legacy" => Some(Mode::Legacy),
        "empty" => Some(Mode::Empty),
        _ => None,
    })
}

/// The made-up guide of a mode (none for `legacy`, a web page).
pub fn fixture(mode: Mode) -> Option<Fixture> {
    match mode {
        Mode::Compact | Mode::Wide => Some(Fixture::Full),
        Mode::Empty => Some(Fixture::Empty),
        Mode::Legacy => None,
    }
}

/// (key, name) of the categories.
const CATEGORIES: [(&str, &str); 11] = [
    ("editor", "Editors' Picks"),
    ("events", "Featured Events"),
    ("new", "Newcomer Friendly"),
    ("games", "Games & Rides"),
    ("clubs", "Music & Clubs"),
    ("nature", "Nature & Beaches"),
    ("art", "Art Installations"),
    ("roleplay", "Roleplay"),
    ("urban", "Urban"),
    ("sports", "Sports"),
    ("grid", "Grid Destinations"),
];

struct Spot {
    name: &'static str,
    region: &'static str,
    pos: (u32, u32, u32),
    rating: &'static str,
    people: u32,
    /// Its categories; "editor" too for an editors' pick.
    cats: &'static [&'static str],
    /// Days since it was listed.
    listed: i64,
    /// Days left for an event.
    ends: Option<i64>,
    text: &'static str,
}

const SPOTS: [Spot; 13] = [
    Spot {
        name: "Welcome Harbour",
        region: "Harbourmoor",
        pos: (128, 128, 25),
        rating: "G",
        people: 55,
        cats: &["new", "editor"],
        listed: 120,
        ends: None,
        text: "Tutorials, volunteers who answer questions and free boxes to get started.",
    },
    Spot {
        name: "Blue Neon Club",
        region: "Neonvale",
        pos: (96, 140, 22),
        rating: "M",
        people: 67,
        cats: &["clubs"],
        listed: 40,
        ends: None,
        text: "Live DJs every night from 8 pm to midnight, a large dance floor and a quiet lounge upstairs.",
    },
    Spot {
        name: "Lantern Isle",
        region: "Lantern Isle",
        pos: (128, 96, 23),
        rating: "G",
        people: 42,
        cats: &["nature", "editor"],
        listed: 12,
        ends: None,
        text: "A floating village lit up every evening: boat rides, a craft market and photo spots.",
    },
    Spot {
        name: "Amber Kingdom",
        region: "Amberhold",
        pos: (60, 200, 31),
        rating: "M",
        people: 38,
        cats: &["roleplay"],
        listed: 300,
        ends: None,
        text: "Medieval fantasy roleplay with guilds, quests and a combat system. An arrival kit is offered.",
    },
    Spot {
        name: "Pixel Arcade",
        region: "Pixelreach",
        pos: (140, 60, 22),
        rating: "G",
        people: 31,
        cats: &["games"],
        listed: 60,
        ends: None,
        text: "Retro games, roller coasters and a Ferris wheel. Everything is free.",
    },
    Spot {
        name: "Garden Concert",
        region: "Saikomori",
        pos: (110, 90, 24),
        rating: "G",
        people: 27,
        cats: &["events"],
        listed: 2,
        ends: Some(3),
        text: "An open-air acoustic concert by the pond, with a live singer.",
    },
    Spot {
        name: "Lower City",
        region: "Lowerbrook",
        pos: (200, 30, 40),
        rating: "M",
        people: 24,
        cats: &["urban", "roleplay"],
        listed: 90,
        ends: None,
        text: "A rainy city at night, open roleplay and flats to rent.",
    },
    Spot {
        name: "Velvet Lounge",
        region: "Velvetmere",
        pos: (80, 80, 300),
        rating: "A",
        people: 22,
        cats: &["clubs"],
        listed: 30,
        ends: None,
        text: "A late-night lounge with live music, for adults only.",
    },
    Spot {
        name: "Saiko Gardens",
        region: "Saikomori",
        pos: (128, 128, 24),
        rating: "G",
        people: 18,
        cats: &["nature", "editor"],
        listed: 200,
        ends: None,
        text: "Zen gardens, waterfalls and a tea house. A calm place to sit down.",
    },
    Spot {
        name: "Surfers' Bay",
        region: "Surfcrest",
        pos: (30, 220, 21),
        rating: "G",
        people: 15,
        cats: &["sports"],
        listed: 7,
        ends: None,
        text: "Boards to rent, animated waves and contests at the weekend.",
    },
    Spot {
        name: "Wave Gallery",
        region: "Wavelight",
        pos: (128, 64, 1200),
        rating: "G",
        people: 12,
        cats: &["art", "editor"],
        listed: 20,
        ends: None,
        text: "Interactive installations that change every month, up in the sky.",
    },
    Spot {
        name: "World's End Lighthouse",
        region: "Farlight Point",
        pos: (220, 200, 30),
        rating: "G",
        people: 9,
        cats: &["nature"],
        listed: 400,
        ends: None,
        text: "Cliffs, a lighthouse to visit and a pebble beach, ideal for photos at sunset.",
    },
    Spot {
        name: "Founders' Park",
        region: "Foundersfield",
        pos: (128, 128, 30),
        rating: "G",
        people: 6,
        cats: &["grid"],
        listed: 700,
        ends: None,
        text: "A park from the grid's early days, kept by its team.",
    },
];

/// The made-up pictures' addresses: a reserved domain, never asked (the
/// demo draws them, `picture`).
const PICTURES: &str = "https://destinations.demo.invalid/";

fn date(days_from_now: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::days(days_from_now))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn entry(i: usize, s: &Spot) -> Value {
    let location = format!("{}/{}/{}/{}", crate::slurl::encode(s.region), s.pos.0, s.pos.1, s.pos.2);
    json!({
        "id": 9000 + i,
        "locale": "en",
        "name": s.name,
        "slug": s.name.to_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "-"),
        "description": s.text,
        "slurl": format!("{PICTURES}map/{location}"),
        "publish_date": date(-s.listed),
        "end_date": s.ends.map_or_else(|| "2099-12-31 08:00:00".to_owned(), date),
        "location": location,
        "maturity": s.rating,
        "categories": s.cats.iter().map(|c| json!({ "simple_name": c })).collect::<Vec<_>>(),
        "assets": [
            { "type": "fullsize", "data": format!("{PICTURES}{i}/fullsize.jpg") },
            { "type": "midsize", "data": format!("{PICTURES}{i}/midsize.jpg") },
            { "type": "thumbnail", "data": format!("{PICTURES}{i}/thumbnail.jpg") },
        ],
        "population": { "current": s.people },
    })
}

pub fn categories_json(fixture: Fixture) -> String {
    let n = if fixture == Fixture::Empty { 3 } else { CATEGORIES.len() };
    let list: Vec<Value> = CATEGORIES
        .iter()
        .take(n)
        .enumerate()
        .map(|(i, (key, name))| {
            json!({
                "id": 100 + i,
                "name": name,
                "simple_name": key,
                "description": "",
                "image": format!("{PICTURES}category/{key}.jpg"),
                "count": SPOTS.iter().filter(|s| s.cats.contains(key)).count(),
            })
        })
        .collect();
    Value::Array(list).to_string()
}

/// A category's places as the feed sorts them (by population); What's Hot
/// Now as the feed limits it (20 people and more, General and Moderate).
pub fn places_json(fixture: Fixture, key: &str) -> String {
    if fixture == Fixture::Empty {
        return "[]".into();
    }
    let hot = key == crate::destination_guide::HOT;
    let mut list: Vec<(usize, &Spot)> = SPOTS
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            if hot {
                s.people >= 20 && s.rating != "A"
            } else {
                s.cats.contains(&key)
            }
        })
        .collect();
    list.sort_by(|a, b| b.1.people.cmp(&a.1.people));
    Value::Array(list.into_iter().map(|(i, s)| entry(i, s)).collect()).to_string()
}

/// Two visits of the demo's teleport history to places of the guide.
pub fn history() -> Vec<HistoryItem> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    [("Lantern Pier, Lantern Isle", 2.0), ("Arcade Hall, Pixelreach", 9.0)]
        .into_iter()
        .map(|(title, days)| HistoryItem {
            title: title.into(),
            global: glam::DVec3::new(1_002.0 * 256.0 + 128.0, 1_003.0 * 256.0 + 96.0, 23.0),
            date: now - days * 86_400.0,
            slurl: String::new(),
        })
        .collect()
}

/// A plain picture for a made-up address: a sky, a hill and a sun in the
/// tones of its place, as the mockup draws them.
pub fn picture(url: &str, side: u32) -> Option<egui::ColorImage> {
    let rest = url.strip_prefix(PICTURES)?;
    let seed: usize = rest.split('/').next()?.parse().ok()?;
    let (w, h) = if rest.contains("fullsize") { (640u32, 384u32) } else { (250, 150) };
    let hue = (seed as f32 * 47.0 + 210.0) % 360.0;
    let sky_top = hsl(hue, 0.35, 0.16);
    let sky_low = hsl(hue, 0.32, 0.26);
    let hill = hsl((hue + 20.0) % 360.0, 0.55, 0.62);
    let far = hsl((hue + 330.0) % 360.0, 0.40, 0.42);
    let mut img = image::RgbaImage::new(w, h);
    let (fw, fh) = (w as f32, h as f32);
    let sun = (fw * 0.76, fh * 0.26, fh * 0.12);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let (u, v) = (x as f32 / fw, y as f32 / fh);
        let mut c = mix(sky_top, sky_low, v);
        let (dx, dy) = (x as f32 - sun.0, y as f32 - sun.1);
        if dx * dx + dy * dy < sun.2 * sun.2 {
            c = hill;
        }
        // a far ridge, then the hill of the mockup
        let ridge = 0.62 - 0.10 * (u * 7.0 + seed as f32).sin() * (u * 3.0).cos();
        if v > ridge {
            c = mix(c, far, 0.85);
        }
        let near = if u < 0.32 { 1.0 - u / 0.32 * 0.65 } else { 0.35 + (u - 0.32) * 0.9 };
        if v > near.clamp(0.3, 1.0) {
            c = mix(c, hill, 0.85);
        }
        *px = image::Rgba([c[0], c[1], c[2], 255]);
    }
    Some(crate::destination_guide::fit(&image::DynamicImage::ImageRgba8(img), side))
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

fn hsl(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [to(r), to(g), to(b)]
}
