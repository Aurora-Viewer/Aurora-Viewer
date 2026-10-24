use super::*;
use crate::ui::widgets::elide_tests::Frames;

fn demo_picker() -> GifPicker {
    GifPicker {
        demo: true,
        ..GifPicker::default()
    }
}

fn page(provider: Provider, ids: &[&str], next: Option<u32>) -> Result<Page, Error> {
    Ok(Page {
        gifs: ids
            .iter()
            .map(|id| Gif {
                provider,
                id: (*id).to_owned(),
                title: String::new(),
                thumb: format!("https://media.giphy.com/media/{id}/100w.gif"),
                thumb_size: [100, 75],
                send: format!("https://media.giphy.com/media/{id}/giphy.gif"),
                pings: gs::Pings::default(),
            })
            .collect(),
        next,
    })
}

fn ids(picker: &GifPicker) -> Vec<&str> {
    picker.gifs.iter().map(|g| g.id.as_str()).collect()
}

#[test]
fn the_search_leaves_once_the_typing_pauses() {
    let mut picker = demo_picker();
    let trending = picker.wanted(0.0).expect("trending on opening");
    assert_eq!(trending.query, Query::Trending);
    picker.begin(trending);
    assert_eq!(picker.wanted(0.1), None, "nothing changed");
    picker.query = "he".into();
    picker.typed_at = Some(1.0);
    assert_eq!(picker.wanted(1.2), None, "still typing");
    let search = picker.wanted(1.0 + DEBOUNCE + 0.01).expect("the pause");
    assert_eq!(search.query, Query::Search("he".into()));
    picker.begin(search);
    picker.query = "   ".into();
    assert_eq!(
        picker.wanted(9.0).map(|r| r.query),
        Some(Query::Trending),
        "emptied: trending again"
    );
}

#[test]
fn the_rating_and_the_tab_ask_again() {
    let mut picker = demo_picker();
    picker.begin(picker.wanted(0.0).expect("trending"));
    picker.settings.rating = Rating::Strict;
    assert_eq!(picker.wanted(0.0).map(|r| r.rating), Some(Rating::Strict));
    picker.begin(picker.wanted(0.0).expect("strict"));
    picker.tab = Some(Provider::Klipy);
    assert_eq!(picker.wanted(0.0).map(|r| r.provider), Some(Provider::Klipy));
}

#[test]
fn pages_follow_one_another_and_stale_answers_are_dropped() {
    let mut picker = demo_picker();
    let request = picker.wanted(0.0).expect("trending");
    picker.begin(request.clone());
    let generation = picker.generation;
    assert_eq!(picker.more(), None, "the first page is loading");
    picker.receive(
        None,
        Answer::Page {
            generation,
            page: 0,
            result: page(Provider::Giphy, &["a", "b", "c"], Some(24)),
        },
    );
    assert_eq!(ids(&picker), ["a", "b", "c"]);
    assert_eq!(picker.more(), Some((request.clone(), 24)));
    assert_eq!(picker.more(), None, "one page at a time");
    picker.receive(
        None,
        Answer::Page {
            generation,
            page: 24,
            result: page(Provider::Giphy, &["c", "d"], None),
        },
    );
    assert_eq!(ids(&picker), ["a", "b", "c", "d"], "appended in their order, once each");
    assert_eq!(picker.more(), None, "the last page");
    // a search begins: the trending's late answer is dropped
    picker.query = "cat".into();
    picker.typed_at = Some(0.0);
    picker.begin(picker.wanted(10.0).expect("the search"));
    picker.receive(
        None,
        Answer::Page {
            generation,
            page: 0,
            result: page(Provider::Giphy, &["late"], None),
        },
    );
    assert!(picker.gifs.is_empty() && picker.loading, "still waiting for the search");
}

#[test]
fn a_failed_first_page_says_why_and_stops_paging() {
    let mut picker = demo_picker();
    picker.begin(picker.wanted(0.0).expect("trending"));
    let generation = picker.generation;
    picker.receive(
        None,
        Answer::Page {
            generation,
            page: 0,
            result: Err(Error::RateLimited),
        },
    );
    assert_eq!(picker.error, Some(Error::RateLimited));
    assert!(!picker.loading);
    assert_eq!(picker.more(), None);
}

#[test]
fn no_favourite_asks_nothing() {
    let mut picker = demo_picker();
    picker.favorites = true;
    let request = picker.wanted(0.0).expect("the favourites");
    assert_eq!(request.query, Query::Favorites(Vec::new()));
    picker.begin(request);
    assert!(!picker.loading, "nothing to wait for");
    picker.settings.toggle_favorite(Provider::Giphy, "x");
    assert_eq!(
        picker.wanted(0.0).map(|r| r.query),
        Some(Query::Favorites(vec!["x".into()])),
        "a new favourite asks again"
    );
}

#[test]
fn closing_frees_everything_once() {
    let mut picker = demo_picker();
    picker.active = true;
    picker.query = "cat".into();
    picker.begin(picker.wanted(10.0).expect("the search"));
    let generation = picker.generation;
    picker.receive(
        None,
        Answer::Page {
            generation,
            page: 0,
            result: page(Provider::Giphy, &["a"], Some(24)),
        },
    );
    picker.want_thumb("https://media.giphy.com/media/a/100w.gif", Provider::Giphy, 1);
    picker.close();
    assert!(picker.gifs.is_empty() && picker.tiles.is_empty() && picker.queue.is_empty());
    assert!(picker.query.is_empty() && picker.asked.is_none() && !picker.active);
    assert_ne!(picker.generation, generation, "answers in flight are dropped");
    let after = picker.generation;
    picker.close();
    assert_eq!(picker.generation, after, "already closed: nothing to do");
}

#[test]
fn the_button_shows_only_turned_on_with_a_key() {
    let mut picker = GifPicker::default();
    assert!(!picker.usable(), "hidden by default");
    assert_eq!(picker.wanted(0.0), None, "no key: nothing is asked");
    picker.settings.show_button = true;
    assert!(!picker.usable(), "turned on, no key: no button");
    picker.settings.giphy_key = "own-key".into();
    assert!(picker.usable());
    assert_eq!(picker.tab(), Provider::Giphy);
    picker.settings.show_button = false;
    assert!(!picker.usable());
    assert!(demo_picker().usable(), "the demo's scenarios need no key");
}

#[test]
fn one_key_one_tab() {
    let mut picker = GifPicker::default();
    picker.settings.show_button = true;
    picker.settings.provider = Provider::Giphy;
    picker.settings.klipy_key = "own-key".into();
    assert_eq!(picker.providers(), [Provider::Klipy], "GIPHY has no key: no tab");
    assert_eq!(picker.tab(), Provider::Klipy);
    // a tab chosen earlier whose key was removed gives way
    picker.tab = Some(Provider::Giphy);
    assert_eq!(picker.tab(), Provider::Klipy);
    picker.settings.giphy_key = "other-key".into();
    assert_eq!(picker.providers(), [Provider::Giphy, Provider::Klipy]);
    assert_eq!(picker.tab(), Provider::Giphy);
}

/// The button's right-click menu: "Hide the GIF button" turns it off for
/// good (a key entered later no longer shows it).
#[test]
fn the_right_click_menu_hides_the_button() {
    let mut f = Frames::new();
    let p = crate::theme::Theme::default().palette();
    let mut st = crate::ui::chat::ChatUi::default();
    st.gifs.settings.giphy_key = "own-key".into();
    st.gifs.settings.key_changed();
    assert!(st.gifs.usable());
    let rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let run = |f: &mut Frames, st: &mut crate::ui::chat::ChatUi, events: Vec<egui::Event>| {
        f.run(events, |ui| {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
            rect.set(r);
            button(&resp, &p, st);
        })
    };
    run(&mut f, &mut st, Vec::new());
    let pos = rect.get().center();
    let press = |button: egui::PointerButton, pressed| egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Default::default(),
    };
    run(&mut f, &mut st, vec![egui::Event::PointerMoved(pos)]);
    run(&mut f, &mut st, vec![press(egui::PointerButton::Secondary, true)]);
    run(&mut f, &mut st, vec![press(egui::PointerButton::Secondary, false)]);
    for _ in 0..3 {
        run(&mut f, &mut st, Vec::new());
    }
    // the menu's rows: find "Hide the GIF button" among the painted texts
    let hide = aurora_i18n::tr!("ui-chat-gif-hide");
    let mut row = None;
    for t in run(&mut f, &mut st, Vec::new()) {
        if t.source == hide {
            row = Some(t.rect.center());
        }
    }
    let at = row.expect("the menu shows its Hide row");
    let click = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    run(&mut f, &mut st, vec![egui::Event::PointerMoved(at)]);
    run(&mut f, &mut st, vec![click(true)]);
    run(&mut f, &mut st, vec![click(false)]);
    assert!(!st.gifs.settings.show_button && st.gifs.settings.hidden_by_user && st.gifs.changed);
    assert!(!st.gifs.usable(), "the button is gone");
    st.gifs.settings.klipy_key = "second".into();
    st.gifs.settings.key_changed();
    assert!(!st.gifs.usable(), "hidden by the user: a new key does not bring it back");
}

#[test]
fn tiles_fill_without_stretching() {
    let wide = cover_uv(egui::vec2(200.0, 100.0), egui::vec2(100.0, 100.0));
    assert!((wide.width() - 0.5).abs() < 1e-4 && (wide.height() - 1.0).abs() < 1e-4);
    assert!((wide.center().x - 0.5).abs() < 1e-4, "its middle");
    let tall = cover_uv(egui::vec2(100.0, 300.0), egui::vec2(120.0, 90.0));
    assert!((tall.width() - 1.0).abs() < 1e-4);
    assert!(tall.height() < 0.3);
}

/// A demo picker drawn for real: its tiles animate, a click on one sends
/// it, and the popup keeps one size whatever the grid shows.
#[test]
fn a_clicked_tile_is_sent_and_the_picker_keeps_its_size() {
    let mut f = Frames::new();
    let icons = crate::ui::icons::Icons::load(&f.ctx, None);
    let p = crate::theme::Theme::default().palette();
    let mut picker = demo_picker();
    let mut out = Out::default();
    let mut size = None;
    let draw = |f: &mut Frames, picker: &mut GifPicker, events: Vec<egui::Event>, out: &mut Out, size: &mut Option<egui::Vec2>| {
        f.run(events, |ui| {
            let r = ui.scope(|ui| picker.show(ui, &p, &icons));
            *out = r.inner;
            *size = Some(r.response.rect.size());
        });
    };
    draw(&mut f, &mut picker, Vec::new(), &mut out, &mut size);
    let empty = size.expect("drawn");
    for _ in 0..4 {
        draw(&mut f, &mut picker, Vec::new(), &mut out, &mut size);
    }
    assert_eq!(picker.gifs.len(), gs::PER_PAGE as usize);
    assert!(
        picker.tiles.values().any(|t| matches!(t.thumb, Thumb::Ready(_))),
        "the shown tiles are loaded"
    );
    assert!(picker.tiles.len() < picker.gifs.len(), "only those in view");
    assert_eq!(size, Some(empty), "the grid filled without moving the popup");
    // nothing found: the same size
    picker.query = "none".into();
    picker.typed_at = None;
    draw(&mut f, &mut picker, Vec::new(), &mut out, &mut size);
    draw(&mut f, &mut picker, Vec::new(), &mut out, &mut size);
    assert!(picker.gifs.is_empty());
    assert_eq!(size, Some(empty), "an empty grid keeps its room");
    // back to the trending, then a click on the second tile
    picker.query.clear();
    for _ in 0..3 {
        draw(&mut f, &mut picker, Vec::new(), &mut out, &mut size);
    }
    let (rect, id) = picker.drawn[1].clone();
    let pos = rect.left_bottom() + egui::vec2(10.0, -10.0);
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    draw(&mut f, &mut picker, vec![egui::Event::PointerMoved(pos)], &mut out, &mut size);
    draw(&mut f, &mut picker, vec![button(true)], &mut out, &mut size);
    draw(&mut f, &mut picker, vec![button(false)], &mut out, &mut size);
    assert_eq!(out.send.as_ref().map(|g| g.id.as_str()), Some(id.as_str()));
    // its star keeps it as a favourite instead
    let (rect, id) = picker.drawn[2].clone();
    let star = rect.right_top() + egui::vec2(-14.0, 14.0);
    let press = |pressed| egui::Event::PointerButton {
        pos: star,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    draw(&mut f, &mut picker, vec![egui::Event::PointerMoved(star)], &mut out, &mut size);
    draw(&mut f, &mut picker, vec![press(true)], &mut out, &mut size);
    draw(&mut f, &mut picker, vec![press(false)], &mut out, &mut size);
    assert!(out.send.is_none(), "the star does not send");
    assert!(picker.settings.is_favorite(Provider::Giphy, &id) && picker.changed);
}
