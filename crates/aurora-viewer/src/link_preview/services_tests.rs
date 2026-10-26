use super::*;

fn service(url: &str) -> Option<Service> {
    recognize(url).map(|(s, _)| s)
}

fn step(url: &str) -> Option<Step> {
    recognize(url).map(|(_, s)| s)
}

/// Every form of each service's page links, and the links that are not a
/// picture's page.
#[test]
fn each_service_s_page_links_are_recognized() {
    let gyazo = "https://gyazo.com/0a1b2c3d4e5f60718293a4b5c6d7e8f9";
    assert_eq!(service(gyazo), Some(Service::Gyazo));
    assert_eq!(
        service("http://www.gyazo.com/0A1B2C3D4E5F60718293A4B5C6D7E8F9?token=x"),
        Some(Service::Gyazo)
    );
    assert_eq!(
        step(gyazo),
        Some(Step::OEmbed(
            "https://api.gyazo.com/api/oembed?url=https%3A%2F%2Fgyazo.com%2F0a1b2c3d4e5f60718293a4b5c6d7e8f9".into()
        ))
    );
    for url in [
        "https://www.flickr.com/photos/aurora_demo/52000000001/",
        "https://flickr.com/photos/aurora_demo/52000000001/in/album-72177720300000000/",
        "https://m.flickr.com/photos/12345678@N00/52000000001",
        "https://flic.kr/p/2nDemoAb",
    ] {
        assert_eq!(service(url), Some(Service::Flickr), "{url}");
    }
    assert_eq!(
        step("https://flic.kr/p/2nDemoAb"),
        Some(Step::OEmbed(
            "https://www.flickr.com/services/oembed?format=json&maxwidth=800&maxheight=800&url=https%3A%2F%2Fflic.kr%2Fp%2F2nDemoAb".into()
        ))
    );
    for url in [
        "https://imgur.com/DemoAb1",
        "https://m.imgur.com/DemoAb1?s=sms",
        "https://imgur.com/a/DemoAlb",
        "https://imgur.com/gallery/a-new-build-on-the-plaza-DemoGal",
        "https://imgur.com/t/aurora/DemoAb1",
    ] {
        assert_eq!(service(url), Some(Service::Imgur), "{url}");
    }
    assert_eq!(
        step("https://m.imgur.com/gallery/a-new-build-DemoGal#comments"),
        Some(Step::Head("https://imgur.com/gallery/a-new-build-DemoGal".into()))
    );
    for url in ["https://prnt.sc/demo42", "https://prntscr.com/Demo42"] {
        assert_eq!(step(url), Some(Step::Head("https://prnt.sc/demo42".into())), "{url}");
    }
    let feed = Some(Step::Feed("https://www.reddit.com/comments/demo01/.rss?limit=1".into()));
    for url in [
        "https://www.reddit.com/r/AuroraDemo/comments/demo01/a_new_build/",
        "https://old.reddit.com/r/AuroraDemo/comments/demo01/a_new_build/?utm_source=share",
        "https://new.reddit.com/r/AuroraDemo/comments/demo01",
        "https://np.reddit.com/user/aurora_demo/comments/demo01/a_new_build/",
        "https://reddit.com/comments/demo01",
        "https://redd.it/demo01",
    ] {
        assert_eq!(step(url), feed, "{url}");
    }
    // Reddit's picture viewer names its picture: no request
    assert_eq!(
        step("https://www.reddit.com/media?url=https%3A%2F%2Fi.redd.it%2Fdemo01picture.jpeg"),
        Some(Step::Direct("https://i.redd.it/demo01picture.jpeg".into()))
    );
    for url in [
        // a picture elsewhere behind Reddit's viewer, a script
        "https://www.reddit.com/media?url=https%3A%2F%2Fpics.example.net%2Fa.jpg",
        "https://www.reddit.com/media?url=javascript%3Aalert(1)",
        // the services' own pages, profiles, albums of Flickr, videos
        "https://imgur.com/upload",
        "https://imgur.com/user/aurora_demo",
        "https://imgur.com/",
        "https://www.flickr.com/photos/aurora_demo/",
        "https://www.flickr.com/photos/aurora_demo/albums/72177720300000000",
        "https://gyazo.com/captures",
        "https://gyazo.com/0a1b2c3d4e5f60718293a4b5c6d7e8f",
        "https://prnt.sc/",
        "https://www.reddit.com/r/AuroraDemo/",
        "https://www.reddit.com/r/AuroraDemo/comments/DEMO01/",
        "https://www.reddit.com/r/AuroraDemo/s/AbCdEf1234",
        // look-alikes and other sites
        "https://gyazo.com.evil.example/0a1b2c3d4e5f60718293a4b5c6d7e8f9",
        "https://notimgur.com/DemoAb1",
        "https://example.com/r/x/comments/demo01/",
        "ftp://imgur.com/DemoAb1",
    ] {
        assert_eq!(recognize(url), None, "{url}");
    }
}

/// The saved answers lead to their picture; a video, a removed screenshot, a
/// text post, a picture elsewhere lead to none.
#[test]
fn the_services_answers_lead_to_their_picture() {
    assert_eq!(
        from_oembed(Service::Gyazo, GYAZO.as_bytes()).as_deref(),
        Some("https://i.gyazo.com/0a1b2c3d4e5f60718293a4b5c6d7e8f9.png")
    );
    assert_eq!(from_oembed(Service::Gyazo, include_bytes!("fixtures/gyazo_video.json")), None);
    assert_eq!(
        from_oembed(Service::Flickr, FLICKR.as_bytes()).as_deref(),
        Some("https://live.staticflickr.com/65535/52000000001_0a1b2c3d4e_c.jpg")
    );
    // a photo said to be on another site
    assert_eq!(from_oembed(Service::Flickr, GYAZO.as_bytes()), None);
    assert_eq!(from_oembed(Service::Flickr, b"<html>"), None);
    // og:image wins over twitter:image, without the copy for social cards
    assert_eq!(
        from_head(Service::Imgur, IMGUR_ALBUM).as_deref(),
        Some("https://i.imgur.com/DemoAb1.jpeg")
    );
    assert_eq!(from_head(Service::Imgur, include_str!("fixtures/imgur_home.html")), None);
    assert_eq!(
        from_head(Service::Lightshot, LIGHTSHOT).as_deref(),
        Some("https://image.prntscr.com/image/Dm0Aur0raExample.png")
    );
    assert_eq!(from_head(Service::Lightshot, LIGHTSHOT_REMOVED), None);
    // the post's own picture, not a reply's
    assert_eq!(
        from_feed(REDDIT_POST).as_deref(),
        Some("https://preview.redd.it/demo01picture.jpg?width=640&crop=smart&auto=webp&s=0a1b2c")
    );
    assert_eq!(from_feed(REDDIT_TEXT), None);
    assert_eq!(from_feed("<feed></feed>"), None);
}

/// Tags written any way a page writes them; only a whole attribute name.
#[test]
fn a_page_s_tags_are_read_quoted_or_not() {
    let page = |tag: &str| format!("<html><head>{tag}</head></html>");
    for tag in [
        r#"<meta property="og:image" content="https://i.imgur.com/DemoAb1.png">"#,
        "<meta property='og:image' content='https://i.imgur.com/DemoAb1.png' />",
        "<meta property=og:image content=https://i.imgur.com/DemoAb1.png />",
        "<META PROPERTY=\"OG:IMAGE\" CONTENT=\"https://i.imgur.com/DemoAb1.png\">",
        "<meta content=\"https://i.imgur.com/DemoAb1.png\" property=\"og:image\">",
        "<meta name=\"og:image\" content=\"https://i.imgur.com/DemoAb1.png?fb&amp;x=1\">",
    ] {
        assert_eq!(
            from_head(Service::Imgur, &page(tag)).as_deref(),
            Some("https://i.imgur.com/DemoAb1.png"),
            "{tag}"
        );
    }
    // "data-content" is not "content", a link is not a meta
    assert_eq!(
        from_head(
            Service::Imgur,
            &page(r#"<meta property="og:image" data-content="https://i.imgur.com/DemoAb1.png">"#)
        ),
        None
    );
    assert_eq!(
        from_head(
            Service::Imgur,
            &page(r#"<link rel="og:image" content="https://i.imgur.com/DemoAb1.png">"#)
        ),
        None
    );
}

/// A picture found must be an https file on the service's own servers.
#[test]
fn a_page_cannot_send_the_viewer_elsewhere() {
    assert!(picture_host_ok(Service::Flickr, "https://live.staticflickr.com/1/2_3_c.jpg"));
    assert!(picture_host_ok(Service::Flickr, "https://farm4.staticflickr.com/1/2_3_c.jpg"));
    assert!(picture_host_ok(Service::Lightshot, "https://i.imgur.com/DemoAb1.png"));
    for url in [
        "http://i.gyazo.com/a.png",
        "https://i.gyazo.com.evil.example/a.png",
        "https://evili.gyazo.com/a.png",
        "https://user@i.gyazo.com/a.png",
        "https://i.gyazo.com:8443/a.png",
        "https://gyazo.com/a.png",
        "//i.gyazo.com/a.png",
        "https://192.168.1.2/a.png",
    ] {
        assert!(!picture_host_ok(Service::Gyazo, url), "{url}");
    }
    assert!(!picture_host_ok(Service::Imgur, "https://s.imgur.com/images/logo-1200-630.png"));
    assert!(!picture_host_ok(Service::Reddit, "https://b.thumbs.redditmedia.com/a.png"));
}

#[test]
fn reddit_pictures_are_blurred_until_clicked() {
    for url in [
        "https://i.redd.it/demo01picture.jpeg",
        "https://preview.redd.it/demo01picture.jpg?width=640",
        "https://www.reddit.com/r/AuroraDemo/comments/demo01/",
        "https://redd.it/demo01",
        "https://old.reddit.com/media?url=x",
    ] {
        assert!(blurred_until_clicked(url), "{url}");
    }
    for url in ["https://i.imgur.com/DemoAb1.png", "https://notreddit.com/x", "https://gyazo.com/x"] {
        assert!(!blurred_until_clicked(url), "{url}");
    }
}

/// Every form of a Bluesky or X post's link; profiles, other pages and
/// look-alikes are not posts.
#[test]
fn bluesky_and_x_posts_are_recognized() {
    let thread = |actor: &str, key: &str| {
        Some(Step::Post(format!(
            "https://public.api.bsky.app/xrpc/app.bsky.feed.getPostThread?uri=at%3A%2F%2F{actor}%2Fapp.bsky.feed.post%2F{key}&depth=0&parentHeight=0"
        )))
    };
    assert_eq!(
        step("https://bsky.app/profile/aurora-demo.bsky.social/post/3demopost2abc"),
        thread("aurora-demo.bsky.social", "3demopost2abc")
    );
    assert_eq!(
        step("https://bsky.app/profile/did:plc:auroraexampledemo2345abcd/post/3demopost2abc?ref=share"),
        thread("did%3Aplc%3Aauroraexampledemo2345abcd", "3demopost2abc")
    );
    let post = Some(Step::Post(
        "https://cdn.syndication.twimg.com/tweet-result?id=1850000000000000001&lang=en&token=4hfy2jnxph".into(),
    ));
    for url in [
        "https://x.com/AuroraDemo/status/1850000000000000001",
        "https://www.x.com/AuroraDemo/status/1850000000000000001?s=20",
        "https://twitter.com/AuroraDemo/status/1850000000000000001/photo/1",
        "https://mobile.twitter.com/AuroraDemo/status/1850000000000000001",
        "https://x.com/i/status/1850000000000000001",
        "https://x.com/i/web/status/1850000000000000001",
        "https://twitter.com/aurora_demo/statuses/1850000000000000001",
    ] {
        assert_eq!(step(url), post, "{url}");
    }
    for url in [
        "https://bsky.app/profile/aurora-demo.bsky.social",
        "https://bsky.app/profile/aurora-demo.bsky.social/post/",
        "https://bsky.app/profile/aurora-demo.bsky.social/feed/whats-hot",
        "https://bsky.app/profile/not a handle/post/3demopost2abc",
        "https://bsky.app/profile/nodot/post/3demopost2abc",
        "https://bsky.app/profile/aurora-demo.bsky.social/post/..",
        "https://x.com/AuroraDemo",
        "https://x.com/AuroraDemo/status/",
        "https://x.com/AuroraDemo/status/12a4",
        "https://x.com/AuroraDemo/status/99999999999999999999999",
        "https://x.com/a_name_far_too_long/status/1850000000000000001",
        "https://x.com.evil.example/AuroraDemo/status/1850000000000000001",
        "https://fxtwitter.com/AuroraDemo/status/1850000000000000001",
    ] {
        assert_eq!(recognize(url), None, "{url}");
    }
}

/// The token is the one X's embed widget computes, digit for digit as
/// JavaScript writes the number (values from Node).
#[test]
fn x_tokens_match_the_widget_s() {
    for (id, token) in [
        (20, "6dq1a2xwd93"),
        (999, "8uqo9xs4uzm"),
        (1_000_000_000_000_000, "353i5ab8p5f"),
        (1_585_341_984_679_469_056, "3uchycv2wqc"),
        (1_850_000_000_000_000_001, "4hfy2jnxph"),
        (1_234_567_890_123_456_789, "2zqic77uqyk"),
        (9_223_372_036_854_775_807, "mcw2svce2ao"),
    ] {
        assert_eq!(x_token(id), token, "{id}");
    }
    assert_eq!(js_base36(1.5), "1.i");
    assert_eq!(js_base36(36.0), "10");
}

/// A post's first picture or its video's thumbnail; the marks of its
/// network; nothing for a text post or a picture elsewhere.
#[test]
fn posts_lead_to_their_picture_and_its_mark() {
    assert_eq!(
        from_post(Service::Bluesky, BLUESKY_POST.as_bytes()),
        Some((
            "https://cdn.bsky.app/img/feed_fullsize/plain/did:plc:auroraexampledemo2345abcd/bafkreiauroraexampledemopicture01@jpeg".into(),
            false
        ))
    );
    assert_eq!(
        from_post(Service::Bluesky, BLUESKY_LABELLED.as_bytes()),
        Some((
            "https://video.bsky.app/watch/did%3Aplc%3Aauroraexampledemo2345abcd/bafkreiauroraexampledemovideo01/thumbnail.jpg".into(),
            true
        ))
    );
    // an account's label marks its posts; a removed label ("neg") does not
    let account = BLUESKY_POST.replacen(r#""labels": []"#, r#""labels": [{"val": "porn"}]"#, 1);
    assert_eq!(from_post(Service::Bluesky, account.as_bytes()).map(|p| p.1), Some(true));
    let removed = BLUESKY_POST.replacen(r#""labels": []"#, r#""labels": [{"val": "porn", "neg": true}]"#, 1);
    assert_eq!(from_post(Service::Bluesky, removed.as_bytes()).map(|p| p.1), Some(false));
    // a text post, a link card, a post not found
    let text = r#"{"thread": {"post": {"labels": [], "author": {}, "record": {"text": "hi"}}}}"#;
    assert_eq!(from_post(Service::Bluesky, text.as_bytes()), None);
    let card = r#"{"thread": {"post": {"embed": {"$type": "app.bsky.embed.external#view", "external": {"thumb": "https://cdn.bsky.app/img/a@jpeg"}}}}}"#;
    assert_eq!(from_post(Service::Bluesky, card.as_bytes()), None);
    assert_eq!(from_post(Service::Bluesky, br#"{"error": "NotFound"}"#), None);

    assert_eq!(
        from_post(Service::X, X_POST.as_bytes()),
        Some(("https://pbs.twimg.com/media/AuroraDemoPic01.jpg".into(), false))
    );
    let sensitive = X_POST.replace(r#""possibly_sensitive": false"#, r#""possibly_sensitive": true"#);
    assert_eq!(from_post(Service::X, sensitive.as_bytes()).map(|p| p.1), Some(true));
    // a removed post, a picture said to be elsewhere, X's answer to Bluesky
    assert_eq!(from_post(Service::X, br#"{"__typename": "TweetTombstone"}"#), None);
    let elsewhere = X_POST.replace("https://pbs.twimg.com/", "https://pics.example.net/");
    assert_eq!(from_post(Service::X, elsewhere.as_bytes()), None);
    assert_eq!(from_post(Service::Bluesky, X_POST.as_bytes()), None);
    assert_eq!(from_post(Service::X, b"<!DOCTYPE html>"), None);
}

#[test]
fn the_demo_s_links_use_the_saved_answers() {
    assert_eq!(
        demo_source("https://gyazo.com/0a1b2c3d4e5f60718293a4b5c6d7e8f9"),
        Some(("https://i.gyazo.com/0a1b2c3d4e5f60718293a4b5c6d7e8f9.png".into(), false))
    );
    assert_eq!(
        demo_source("https://bsky.app/profile/aurora-demo.bsky.social/post/3demolabelled").map(|p| p.1),
        Some(true)
    );
    assert_eq!(
        demo_source("https://x.com/AuroraDemo/status/1850000000000000001").map(|p| p.1),
        Some(false)
    );
    assert!(demo_source("https://prnt.sc/demo42").is_some());
    assert_eq!(demo_source("https://prnt.sc/removed1"), None);
    assert!(demo_source("https://redd.it/demo01").is_some());
    assert_eq!(demo_source("https://redd.it/demo02"), None);
}

/// The picture sites of these tests: the list is shared by every test, so
/// each sets the same one.
fn with_sites() {
    set_sites(vec!["cloud.example.org".into()]);
}

/// A page of a listed site (or of a subdomain) is read for its `og:image`;
/// its root, a plain http link, a hidden user and the other sites are not.
#[test]
fn the_pages_of_the_user_s_picture_sites_are_recognized() {
    with_sites();
    assert_eq!(
        recognize("https://cloud.example.org/s/DemoShare7xKq2#top"),
        Some((Service::Site, Step::Head("https://cloud.example.org/s/DemoShare7xKq2".into())))
    );
    assert_eq!(service("https://photos.cloud.example.org/share/abc"), Some(Service::Site));
    assert_eq!(
        service("https://CLOUD.example.org./index.php/s/DemoShare7xKq2"),
        Some(Service::Site)
    );
    for url in [
        "https://cloud.example.org/",
        "http://cloud.example.org/s/DemoShare7xKq2",
        "https://me@cloud.example.org/s/DemoShare7xKq2",
        "https://example.org/s/DemoShare7xKq2",
        "https://cloud.example.org.evil.example/s/DemoShare7xKq2",
        "https://evilcloud.example.org/s/DemoShare7xKq2",
    ] {
        assert_eq!(service(url), None, "{url}");
    }
    // a built-in service keeps its own reading
    assert_eq!(service("https://gyazo.com/0a1b2c3d4e5f60718293a4b5c6d7e8f9"), Some(Service::Gyazo));
}

/// A Nextcloud share names its picture; a folder's share names the site's
/// icon, which is no picture; a picture elsewhere is refused.
#[test]
fn a_picture_site_s_page_leads_to_its_own_picture_only() {
    with_sites();
    assert_eq!(
        from_head(Service::Site, NEXTCLOUD_PICTURE).as_deref(),
        Some("https://cloud.example.org/s/DemoShare7xKq2/preview")
    );
    assert_eq!(from_head(Service::Site, NEXTCLOUD_FOLDER), None);
    let elsewhere = NEXTCLOUD_PICTURE.replace("https://cloud.example.org/s/", "https://pics.example.net/s/");
    assert_eq!(from_head(Service::Site, &elsewhere), None);
    // a port is allowed on the user's own server, not http
    assert!(picture_host_ok(Service::Site, "https://cloud.example.org:8443/s/x/preview"));
    assert!(!picture_host_ok(Service::Site, "http://cloud.example.org/s/x/preview"));
    assert!(!picture_host_ok(Service::Gyazo, "https://cloud.example.org/s/x/preview"));
    assert_eq!(
        demo_source("https://cloud.example.org/s/DemoShare7xKq2"),
        Some(("https://cloud.example.org/s/DemoShare7xKq2/preview".into(), false))
    );
    assert_eq!(demo_source("https://cloud.example.org/s/DemoFolder3Pq9"), None);
}
