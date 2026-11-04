use super::*;

/// An Atom feed shaped like the status page's history: the content is
/// escaped HTML, one paragraph per update, newest first.
const ATOM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xml:lang="en-US" xmlns="http://www.w3.org/2005/Atom">
  <id>tag:status.example,2005:/history</id>
  <title>Example Status - Incident History</title>
  <updated>2026-10-26T18:00:00Z</updated>
  <entry>
    <id>tag:status.example,2005:Incident/1</id>
    <published>2026-10-20T16:00:00Z</published>
    <updated>2026-10-20T19:30:00Z</updated>
    <link rel="alternate" type="text/html" href="https://status.example/incidents/one"/>
    <title>Delays in inventory &amp; delivery</title>
    <content type="html">&lt;p&gt;&lt;small&gt;Oct &lt;var data-var='date'&gt;20&lt;/var&gt;, &lt;var data-var='time'&gt;12:30&lt;/var&gt; PDT&lt;/small&gt;&lt;br&gt;&lt;strong&gt;Resolved&lt;/strong&gt; - Deliveries are back to normal &amp;amp; nothing was lost.&lt;/p&gt;&lt;p&gt;&lt;small&gt;Oct &lt;var data-var='date'&gt;20&lt;/var&gt;, &lt;var data-var='time'&gt;09:00&lt;/var&gt; PDT&lt;/small&gt;&lt;br&gt;&lt;strong&gt;Investigating&lt;/strong&gt; - Some deliveries&lt;br&gt;arrive late.&lt;/p&gt;</content>
  </entry>
  <entry>
    <id>tag:status.example,2005:Incident/2</id>
    <published>2026-10-26T15:00:00Z</published>
    <updated>2026-10-26T17:45:00Z</updated>
    <link rel="alternate" type="text/html" href="https://status.example/incidents/two"/>
    <title>Scheduled maintenance</title>
    <content type="html">&lt;p&gt;&lt;strong&gt;THIS IS A SCHEDULED EVENT Oct 28, 07:00 - 10:00 PDT&lt;/strong&gt;&lt;/p&gt;&lt;p&gt;&lt;small&gt;Oct &lt;var data-var='date'&gt;26&lt;/var&gt;, &lt;var data-var='time'&gt;10:45&lt;/var&gt; PDT&lt;/small&gt;&lt;br&gt;&lt;strong&gt;Scheduled&lt;/strong&gt; - Regions restart&amp;nbsp;in turn.&lt;/p&gt;</content>
  </entry>
</feed>"#;

#[test]
fn the_status_page_feed_reads_as_entries_and_updates() {
    let entries = parse(ATOM.as_bytes()).expect("feed");
    assert_eq!(entries.len(), 2);
    // the most recently updated first
    let maintenance = &entries[0];
    assert_eq!(maintenance.title, "Scheduled maintenance");
    assert_eq!(maintenance.updated_raw, "2026-10-26T17:45:00Z");
    assert_eq!(maintenance.updated, 1_793_036_700);
    assert_eq!(maintenance.phase(), Phase::Maintenance);
    assert_eq!(
        maintenance.updates[0],
        Update {
            when: String::new(),
            status: String::new(),
            text: "THIS IS A SCHEDULED EVENT Oct 28, 07:00 - 10:00 PDT".into(),
        },
        "a bold line without the dash is a note, not a status"
    );
    assert_eq!(maintenance.updates[1].status, "Scheduled");
    assert_eq!(maintenance.updates[1].text, "Regions restart in turn.");

    let incident = &entries[1];
    assert_eq!(incident.id, "tag:status.example,2005:Incident/1");
    assert_eq!(incident.title, "Delays in inventory & delivery");
    assert_eq!(incident.link.as_deref(), Some("https://status.example/incidents/one"));
    assert_eq!(incident.phase(), Phase::Over);
    assert_eq!(
        incident.updates,
        [
            Update {
                when: "Oct 20, 12:30 PDT".into(),
                status: "Resolved".into(),
                text: "Deliveries are back to normal & nothing was lost.".into(),
            },
            Update {
                when: "Oct 20, 09:00 PDT".into(),
                status: "Investigating".into(),
                text: "Some deliveries\narrive late.".into(),
            },
        ]
    );
}

#[test]
fn an_rss_feed_of_an_opensim_grid_is_read_too() {
    let rss = br#"<rss version="2.0"><channel><title>Grid</title>
        <item><title>Login problems</title><link>https://grid.example/news/7</link>
        <guid>news-7</guid><pubDate>Mon, 26 Oct 2026 10:00:00 +0000</pubDate>
        <description><![CDATA[<p>Logins fail for some users.</p>]]></description></item>
        <item><title>No link</title><link>javascript:alert(1)</link><description>Plain text</description></item>
        </channel></rss>"#;
    let entries = parse(rss).expect("rss");
    assert_eq!(entries[0].id, "news-7");
    assert_eq!(entries[0].updated, 1_793_008_800);
    assert_eq!(entries[0].updates[0].text, "Logins fail for some users.");
    assert_eq!(entries[0].phase(), Phase::Unknown);
    // only web pages are opened; the title stands in for a missing id
    assert_eq!(entries[1].link, None);
    assert_eq!(entries[1].id, "No link");
    assert_eq!(entries[1].updates[0].text, "Plain text");
}

#[test]
fn what_is_not_a_feed_is_an_error() {
    assert!(parse(b"<html><body>Down</body></html>").is_err());
    assert!(parse(b"not xml").is_err());
    assert_eq!(parse(b"<feed xmlns='http://www.w3.org/2005/Atom'/>"), Ok(Vec::new()));
}

#[test]
fn entities_and_long_texts_are_bounded() {
    assert_eq!(decode_entities("a &amp; b &#233;&#xE9; &bogus; & c"), "a & b éé &bogus; & c");
    let long = format!("<p>{}</p>", "word ".repeat(2000));
    let text = &updates(&long)[0].text;
    assert!(text.chars().count() <= MAX_TEXT + 1 && text.ends_with('…'));
}

#[test]
fn entities_next_to_unicode_text_decode_in_atom_and_rss_feeds() {
    let html = "<p>&amp;éééé &amp;日本語 &amp;🙂🙂 &bogus;日本語</p>";
    let mut escaped = String::new();
    aurora_llsd::xml::escape(html, &mut escaped);
    for document in [
        format!("<feed><entry><id>test</id><content type=\"html\">{escaped}</content></entry></feed>"),
        format!("<feed><entry><id>test</id><content type=\"html\"><![CDATA[{html}]]></content></entry></feed>"),
        format!("<rss><channel><item><guid>test</guid><description><![CDATA[{html}]]></description></item></channel></rss>"),
    ] {
        let entries = parse(document.as_bytes()).expect("valid feed");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].updates[0].text, "&éééé &日本語 &🙂🙂 &bogus;日本語");
    }
}

fn entry(id: &str, updated: &str) -> Entry {
    Entry {
        id: id.into(),
        updated_raw: updated.into(),
        ..Default::default()
    }
}

/// getGridStatusRSSCoro: the first answer only fills sItemsMap; after it,
/// a new id or a changed `updated` flashes the button.
#[test]
fn only_a_new_or_changed_entry_after_the_first_answer_is_news() {
    let mut s = GridStatus::default();
    assert!(!s.apply(vec![entry("a", "1"), entry("b", "1")]), "the first answer is no news");
    assert!(!s.apply(vec![entry("a", "1"), entry("b", "1")]));
    assert!(s.apply(vec![entry("a", "1"), entry("b", "2")]), "an update of b");
    assert!(s.apply(vec![entry("c", "1"), entry("a", "1")]), "a new entry");
    assert_eq!(s.fresh, HashSet::from(["b".to_owned(), "c".to_owned()]));
    assert_eq!(s.load, Load::Loaded);
}

#[test]
fn the_feed_follows_the_grid() {
    let rt = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
    let mut s = GridStatus::default();
    let now = Instant::now();
    // a grid without a feed fetches nothing
    assert!(!s.poll(rt.handle(), None, None, true, now));
    assert_eq!(s.load, Load::NoFeed);
    s.feed = Some("https://a.example/feed".into());
    s.apply(vec![entry("a", "1")]);
    // another feed: what was seen of the first one is forgotten
    s.poll(rt.handle(), Some("https://b.example/feed"), Some("https://b.example/"), false, now);
    assert!(s.entries.is_empty() && s.seen.is_empty() && s.first);
    assert_eq!(s.load, Load::Loading);
    assert_eq!(s.page(), Some("https://b.example/"));
    // an answer for the old feed, arriving late, is dropped
    s.tx.send(("https://a.example/feed".into(), Ok(vec![entry("a", "2")])))
        .expect("send");
    assert!(!s.poll(rt.handle(), Some("https://b.example/feed"), None, false, now));
    assert!(s.entries.is_empty());
}

#[test]
fn second_life_has_its_feed_and_an_opensim_grid_names_its_own() {
    assert_eq!(
        sources(true, None),
        (Some(SECOND_LIFE_FEED.to_owned()), Some(SECOND_LIFE_PAGE.to_owned()))
    );
    assert_eq!(sources(false, None), (None, None));
    assert_eq!(
        sources(false, Some(("https://grid.example/status.rss", "https://grid.example/status"))),
        (
            Some("https://grid.example/status.rss".to_owned()),
            Some("https://grid.example/status".to_owned())
        )
    );
    assert_eq!(sources(false, Some(("file:///C:/feed.xml", ""))), (None, None));
}
