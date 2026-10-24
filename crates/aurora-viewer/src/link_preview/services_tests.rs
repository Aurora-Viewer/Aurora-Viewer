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

#[test]
fn the_demo_s_links_use_the_saved_answers() {
    assert_eq!(
        demo_source("https://gyazo.com/0a1b2c3d4e5f60718293a4b5c6d7e8f9").as_deref(),
        Some("https://i.gyazo.com/0a1b2c3d4e5f60718293a4b5c6d7e8f9.png")
    );
    assert!(demo_source("https://prnt.sc/demo42").is_some());
    assert_eq!(demo_source("https://prnt.sc/removed1"), None);
    assert!(demo_source("https://redd.it/demo01").is_some());
    assert_eq!(demo_source("https://redd.it/demo02"), None);
}
