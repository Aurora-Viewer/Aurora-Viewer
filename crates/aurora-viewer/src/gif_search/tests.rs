use super::*;

const GIPHY_SEARCH: &[u8] = include_bytes!("fixtures/giphy_search.json");
const KLIPY_SEARCH: &[u8] = include_bytes!("fixtures/klipy_search.json");
const GIPHY_CATEGORIES: &[u8] = include_bytes!("fixtures/giphy_categories.json");
const KLIPY_CATEGORIES: &[u8] = include_bytes!("fixtures/klipy_categories.json");

fn query_of(url: &str) -> Vec<(String, String)> {
    reqwest::Url::parse(url)
        .expect("a valid address")
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn get(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

#[test]
fn giphy_search_addresses_carry_the_key_rating_language_and_offset() {
    let url = page_url(
        Provider::Giphy,
        "demo-key",
        &Query::Search("chat & café #1".into()),
        48,
        Rating::Moderate,
        "fr",
        "abc",
    );
    assert!(url.starts_with("https://api.giphy.com/v1/gifs/search?"), "{url}");
    let q = query_of(&url);
    assert_eq!(get(&q, "api_key").as_deref(), Some("demo-key"));
    // escaped, then read back as typed
    assert_eq!(get(&q, "q").as_deref(), Some("chat & café #1"));
    assert_eq!(get(&q, "offset").as_deref(), Some("48"));
    assert_eq!(get(&q, "limit").as_deref(), Some("24"));
    assert_eq!(get(&q, "rating").as_deref(), Some("pg-13"));
    assert_eq!(get(&q, "lang").as_deref(), Some("fr"));
    assert_eq!(get(&q, "customer_id").as_deref(), Some("abc"));
    // no raw space nor '&' of the text in the address
    assert!(!url.contains(' ') && !url.contains("& "), "{url}");
}

#[test]
fn giphy_search_text_is_cut_at_fifty_characters_and_offsets_stop_at_its_end() {
    let long = "a".repeat(80);
    let url = page_url(Provider::Giphy, "k", &Query::Search(long), 9_000, Rating::Strict, "pt-BR", "");
    let q = query_of(&url);
    assert_eq!(get(&q, "q").map(|s| s.len()), Some(50));
    assert_eq!(get(&q, "offset").as_deref(), Some("4999"));
    assert_eq!(get(&q, "rating").as_deref(), Some("g"));
    assert_eq!(get(&q, "lang").as_deref(), Some("pt"));
    assert_eq!(get(&q, "customer_id"), None);
    let trending = page_url(Provider::Giphy, "k", &Query::Trending, 900, Rating::Relaxed, "en", "");
    let q = query_of(&trending);
    assert!(trending.starts_with("https://api.giphy.com/v1/gifs/trending?"), "{trending}");
    assert_eq!(get(&q, "offset").as_deref(), Some("499"));
    assert_eq!(get(&q, "rating").as_deref(), Some("r"));
    assert_eq!(get(&q, "lang"), None, "trending takes no language");
}

#[test]
fn favourites_are_asked_by_id() {
    let ids = Query::Favorites(vec!["AbC".into(), "DeF".into()]);
    let giphy = page_url(Provider::Giphy, "k", &ids, 0, Rating::Moderate, "en", "");
    assert!(giphy.starts_with("https://api.giphy.com/v1/gifs?"), "{giphy}");
    assert_eq!(get(&query_of(&giphy), "ids").as_deref(), Some("AbC,DeF"));
    let klipy = page_url(Provider::Klipy, "k", &ids, 0, Rating::Moderate, "en", "");
    assert!(klipy.starts_with("https://api.klipy.com/api/v1/k/gifs/items?"), "{klipy}");
    assert_eq!(get(&query_of(&klipy), "slugs").as_deref(), Some("AbC,DeF"));
}

#[test]
fn klipy_addresses_put_the_key_in_the_path_and_page_from_one() {
    let url = page_url(
        Provider::Klipy,
        "demo-key",
        &Query::Search("good night".into()),
        0,
        Rating::Strict,
        "fr",
        "abc",
    );
    assert!(url.starts_with("https://api.klipy.com/api/v1/demo-key/gifs/search?"), "{url}");
    let q = query_of(&url);
    assert_eq!(get(&q, "q").as_deref(), Some("good night"));
    assert_eq!(get(&q, "page").as_deref(), Some("1"));
    assert_eq!(get(&q, "per_page").as_deref(), Some("24"));
    assert_eq!(get(&q, "content_filter").as_deref(), Some("high"));
    assert_eq!(get(&q, "format_filter").as_deref(), Some("gif"));
    assert_eq!(get(&q, "customer_id").as_deref(), Some("abc"));
    let trending = page_url(Provider::Klipy, "demo-key", &Query::Trending, 3, Rating::Moderate, "fr", "");
    assert!(
        trending.starts_with("https://api.klipy.com/api/v1/demo-key/gifs/trending?"),
        "{trending}"
    );
    assert_eq!(get(&query_of(&trending), "page").as_deref(), Some("3"));
    assert_eq!(get(&query_of(&trending), "content_filter").as_deref(), Some("medium"));
}

#[test]
fn a_key_never_reaches_the_log() {
    let giphy = page_url(Provider::Giphy, "secret-one", &Query::Trending, 0, Rating::Moderate, "en", "");
    let klipy = page_url(Provider::Klipy, "secret-two", &Query::Trending, 1, Rating::Moderate, "en", "");
    assert!(!without_key(&giphy).contains("secret-one"), "{}", without_key(&giphy));
    assert!(!without_key(&klipy).contains("secret-two"), "{}", without_key(&klipy));
    assert!(without_key(&klipy).contains("/gifs/trending"));
}

#[test]
fn giphy_answers_give_their_tiles_links_and_next_offset() {
    let page = parse_giphy(GIPHY_SEARCH).expect("a page");
    // the off-site and the plain http ones are left out
    assert_eq!(page.gifs.len(), 2);
    let first = &page.gifs[0];
    assert_eq!(first.id, "AbCdEf0001");
    assert_eq!(first.thumb_size, [100, 75]);
    assert!(first.thumb.contains("/100w.gif?cid="), "the address is kept as given");
    assert!(first.send.contains("giphy-downsized.gif"), "{}", first.send);
    assert!(first.pings.onsent.as_deref().is_some_and(|u| u.contains("SENT")));
    // without a downsized rendition, the original
    assert!(page.gifs[1].send.ends_with("/AbCdEf0002/giphy.gif"));
    assert_eq!(page.gifs[1].thumb_size, [100, 100]);
    assert_eq!(page.next, Some(48));
}

#[test]
fn giphy_stops_paging_at_its_total() {
    let last = br#"{"data":[],"pagination":{"total_count":30,"count":6,"offset":24}}"#;
    assert_eq!(parse_giphy(last).expect("a page").next, None);
    let by_ids = br#"{"data":[]}"#;
    assert_eq!(parse_giphy(by_ids).expect("a page").next, None);
    assert_eq!(parse_giphy(b"<html>").err(), Some(Error::Unavailable));
}

#[test]
fn klipy_answers_give_their_tiles_links_and_next_page() {
    let page = parse_klipy(KLIPY_SEARCH).expect("a page");
    assert_eq!(page.gifs.len(), 2, "the one sending off KLIPY's servers is left out");
    let first = &page.gifs[0];
    assert_eq!(first.id, "made-up-hello-1");
    assert!(first.thumb.ends_with("/xs1.gif"));
    assert_eq!(first.thumb_size, [90, 90]);
    assert!(first.send.ends_with("/md1.gif"));
    // no xs: the small one; no md: the large one
    assert!(page.gifs[1].thumb.ends_with("/sm2.gif"));
    assert!(page.gifs[1].send.ends_with("/hd2.gif"));
    assert_eq!(page.next, Some(2));
    let last = br#"{"result":true,"data":{"data":[],"current_page":3,"has_next":false}}"#;
    assert_eq!(parse_klipy(last).expect("a page").next, None);
    assert_eq!(parse_klipy(br#"{"result":false}"#).err(), Some(Error::Unavailable));
}

#[test]
fn categories_of_both_services() {
    let giphy = parse_categories(Provider::Giphy, GIPHY_CATEGORIES);
    assert_eq!(
        giphy.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
        ["Actions", "Animals"],
        "a blank name is left out"
    );
    let klipy = parse_categories(Provider::Klipy, KLIPY_CATEGORIES);
    assert_eq!(klipy[1].label, "Good night");
    assert_eq!(klipy[1].query, "good night");
    assert!(parse_categories(Provider::Klipy, b"nope").is_empty());
}

#[test]
fn the_button_is_hidden_until_a_key_is_entered() {
    let mut s = Settings::default();
    assert!(!s.show_button && !s.usable(), "hidden by default");
    assert!(s.providers().is_empty());
    // turned on with no key: still no button
    s.show_button = true;
    assert!(!s.usable(), "no key: no button even when turned on");
    s.show_button = false;
    // a key typed shows it
    s.klipy_key = "  my-own_Key1  ".into();
    s.key_changed();
    assert_eq!(s.key(Provider::Klipy).as_deref(), Some("my-own_Key1"));
    assert!(s.show_button && s.usable(), "a key shows the button");
    assert_eq!(s.providers(), [Provider::Klipy], "one key: one service");
    // a key that would change the address is no key
    s.giphy_key = "not a key/../".into();
    assert_eq!(s.key(Provider::Giphy), None);
}

#[test]
fn a_button_hidden_by_the_user_stays_hidden() {
    let mut s = Settings {
        giphy_key: "k1".into(),
        ..Settings::default()
    };
    s.key_changed();
    assert!(s.usable());
    // the right-click menu's "Hide the GIF button"
    s.hide();
    assert!(!s.show_button && !s.usable() && s.hidden_by_user);
    s.klipy_key = "k2".into();
    s.key_changed();
    assert!(!s.show_button, "a key entered after an explicit hide does not show it again");
    assert_eq!(s.providers(), [Provider::Giphy, Provider::Klipy]);
    // turned on again in Preferences
    s.show_button = true;
    assert!(s.usable());
    // a key typed before any hide, then emptied: the toggle stays as it was
    let mut fresh = Settings::default();
    fresh.key_changed();
    assert!(!fresh.show_button, "no key, nothing to show");
}

#[test]
fn favourites_are_kept_newest_first_per_service() {
    let mut s = Settings::default();
    s.toggle_favorite(Provider::Giphy, "a");
    s.toggle_favorite(Provider::Klipy, "k");
    s.toggle_favorite(Provider::Giphy, "b");
    assert_eq!(s.favorite_ids(Provider::Giphy), ["b", "a"]);
    assert!(s.is_favorite(Provider::Klipy, "k") && !s.is_favorite(Provider::Giphy, "k"));
    s.toggle_favorite(Provider::Giphy, "a");
    assert_eq!(s.favorite_ids(Provider::Giphy), ["b"]);
    for i in 0..60 {
        s.toggle_favorite(Provider::Giphy, &format!("g{i}"));
    }
    assert_eq!(s.favorite_ids(Provider::Giphy).len(), MAX_FAVORITES);
    assert_eq!(s.favorite_ids(Provider::Giphy)[0], "g59");
    assert_eq!(s.favorite_ids(Provider::Klipy), ["k"], "the other service keeps its own");
}

#[test]
fn the_random_id_is_made_once() {
    let mut s = Settings::default();
    assert!(s.ensure_customer_id());
    let id = s.customer_id.clone();
    assert_eq!(id.len(), 32);
    assert!(!s.ensure_customer_id());
    assert_eq!(s.customer_id, id);
}

#[test]
fn analytics_and_share_requests() {
    let ping = ping_url(
        "https://giphy-analytics.giphy.com/v2/pingback_simple?analytics_response_payload=AAAA&action_type=SENT",
        "abc",
        1_700_000_000_000,
    )
    .expect("a link");
    let q = query_of(&ping);
    assert_eq!(get(&q, "analytics_response_payload").as_deref(), Some("AAAA"), "kept as given");
    assert_eq!(get(&q, "ts").as_deref(), Some("1700000000000"));
    assert_eq!(get(&q, "customer_id").as_deref(), Some("abc"));
    assert_eq!(ping_url("http://example.com/x", "abc", 1), None);
    let (url, body) = klipy_share("k", "made-up-hello-1", "abc", Some(" hello "));
    assert_eq!(url, "https://api.klipy.com/api/v1/k/gifs/share/made-up-hello-1");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["customer_id"], "abc");
    assert_eq!(v["q"], "hello");
    let (_, trending) = klipy_share("k", "s", "abc", None);
    assert!(!trending.contains("\"q\""), "no search: no q");
}

#[test]
fn demo_pages_page_once_and_draw_their_tiles() {
    let first = demo::page(Provider::Giphy, &Query::Trending, 0).expect("a page");
    assert_eq!(first.gifs.len(), PER_PAGE as usize);
    assert_eq!(first.next, Some(PER_PAGE));
    let second = demo::page(Provider::Giphy, &Query::Trending, PER_PAGE).expect("a page");
    assert_eq!(second.next, None);
    assert_ne!(first.gifs[0].id, second.gifs[0].id);
    assert!(
        demo::page(Provider::Klipy, &Query::Search("none".into()), 1)
            .expect("a page")
            .gifs
            .is_empty()
    );
    let tile = demo::thumb(&first.gifs[3].thumb);
    assert!(tile.frames.len() > 1, "animated");
    assert!(media_ok(Provider::Giphy, &first.gifs[0].send));
    assert!(media_ok(
        Provider::Klipy,
        &demo::page(Provider::Klipy, &Query::Trending, 1).expect("a page").gifs[0].send
    ));
}

#[test]
fn sent_links_are_trusted_pictures_for_the_previews() {
    let page = parse_giphy(GIPHY_SEARCH).expect("a page");
    let klipy = parse_klipy(KLIPY_SEARCH).expect("a page");
    for gif in page.gifs.iter().chain(&klipy.gifs) {
        assert_eq!(
            crate::link_trust::classify(&gif.send),
            crate::link_trust::Trust::Trusted,
            "{}",
            gif.send
        );
        assert_eq!(
            crate::link_preview::target(&gif.send),
            Some(crate::link_preview::Target::Picture),
            "{}",
            gif.send
        );
    }
}
