use super::*;

const HERE: RegionHandle = 1;
const NEIGHBOUR: RegionHandle = 2;

fn avatar(n: u128, region: RegionHandle, weight: u32, too_complex: bool) -> SeenAvatar {
    SeenAvatar {
        id: Uuid::from_u128(n),
        region,
        weight,
        too_complex,
    }
}

#[test]
fn the_report_lists_the_region_s_avatars_with_a_complexity() {
    let avatars = [
        avatar(1, HERE, 45_000, false),
        avatar(2, HERE, 410_000, true),
        // not computed yet: left out (avatar_complexity > 0)
        avatar(3, HERE, 0, false),
        // another region's: in that region's report
        avatar(4, NEIGHBOUR, 90_000, false),
    ];
    let (body, count) = report(&avatars, HERE);
    assert_eq!(count, 2);
    let agents = body["agents"].as_map().expect("agents map");
    assert_eq!(agents.len(), 2);
    let first = &body["agents"][Uuid::from_u128(1).to_string().as_str()];
    assert_eq!(first["weight"], Llsd::Integer(45_000));
    assert_eq!(first["tooComplex"], Llsd::Boolean(false));
    let second = &body["agents"][Uuid::from_u128(2).to_string().as_str()];
    assert_eq!(second["weight"], Llsd::Integer(410_000));
    assert_eq!(second["tooComplex"], Llsd::Boolean(true));
    // only those two keys, as the Legacy Viewer sends
    assert_eq!(second.as_map().map(|m| m.len()), Some(2));
    let (_, neighbour) = report(&avatars, NEIGHBOUR);
    assert_eq!(neighbour, 1);
}

#[test]
fn a_weight_beyond_signed_integers_is_sent_as_the_largest() {
    let (body, _) = report(&[avatar(1, HERE, u32::MAX, false)], HERE);
    assert_eq!(
        body["agents"][Uuid::from_u128(1).to_string().as_str()]["weight"],
        Llsd::Integer(i32::MAX)
    );
}

#[test]
fn an_empty_report_still_has_its_agents_map() {
    let (body, count) = report(&[], HERE);
    assert_eq!(count, 0);
    assert!(body["agents"].is_map());
    // as XML, the way it is posted
    let xml = aurora_llsd::to_xml_string(&report(&[avatar(7, HERE, 1200, true)], HERE).0);
    assert!(xml.contains("<key>agents</key>") && xml.contains("<key>tooComplex</key>") && xml.contains("<key>weight</key>"));
}

#[test]
fn reports_space_out_with_the_avatars_around() {
    assert_eq!(report_delay(0), Duration::from_secs(60));
    assert_eq!(report_delay(25), Duration::from_secs(110));
}

#[test]
fn the_answer_gives_our_visibility_only_with_both_counts() {
    let id = Uuid::from_u128(9);
    let body = aurora_llsd::llsd_map! {
        "agents" => aurora_llsd::llsd_map! { id.to_string() => aurora_llsd::llsd_map! { "weight" => 120_000 } },
        "reportinglimit" => 12,
        "overlimit" => 3,
    };
    let (visibility, agents) = parse_answer(&body);
    assert_eq!(
        visibility,
        Some(Visibility {
            reporting: 12,
            over_limit: 3
        })
    );
    assert_eq!(agents, vec![(id, 120_000)]);
    // either count missing: nothing for the notice
    let partial = aurora_llsd::llsd_map! { "reportinglimit" => 12 };
    assert_eq!(parse_answer(&partial).0, None);
    let partial = aurora_llsd::llsd_map! { "overlimit" => 3 };
    assert_eq!(parse_answer(&partial).0, None);
    // a malformed agents entry is skipped, the counts still read
    let odd = aurora_llsd::llsd_map! { "agents" => "x", "reportinglimit" => 0, "overlimit" => 0 };
    let (visibility, agents) = parse_answer(&odd);
    assert_eq!(
        visibility,
        Some(Visibility {
            reporting: 0,
            over_limit: 0
        })
    );
    assert!(agents.is_empty());
}

#[test]
fn a_post_error_is_read_for_the_log() {
    let body = aurora_llsd::llsd_map! {
        "error" => aurora_llsd::llsd_map! { "identifier" => "too_many", "message" => "Too many agents" },
    };
    assert_eq!(post_error(&body).as_deref(), Some("too_many: Too many agents"));
    assert_eq!(post_error(&Llsd::new_map()), None);
}

#[test]
fn a_new_region_is_asked_at_once_and_told_on_the_next_scan() {
    let t0 = Instant::now();
    let mut s = Schedule::new(t0);
    assert!(s.scan(t0));
    assert!(!s.scan(t0 + Duration::from_secs(4)));
    // capabilities of a region: the next frame scans
    s.new_region(HERE, t0 + Duration::from_secs(4));
    let t = t0 + Duration::from_secs(4);
    assert!(s.scan(t));
    assert!(s.request_due(HERE, t).is_some());
    assert!(!s.report_due(HERE, t), "told 5 s later");
    // the next scan, 5 s on
    let t = t + SCAN_PERIOD;
    assert!(s.scan(t));
    assert!(s.report_due(HERE, t));
    s.reported(HERE, t, 3);
    assert!(!s.report_due(HERE, t + Duration::from_secs(65)));
    assert!(s.report_due(HERE, t + Duration::from_secs(66)));
}

#[test]
fn a_request_waits_for_its_period_after_the_answer() {
    let t0 = Instant::now();
    let mut s = Schedule::new(t0);
    s.new_region(HERE, t0);
    let timer = s.request_due(HERE, t0).expect("asked at once");
    assert!(s.request_due(HERE, t0 + Duration::from_secs(10)).is_none());
    // the answer comes 4 s later: the next GET 15 s after it
    answered(&timer, t0 + Duration::from_secs(4));
    assert!(s.request_due(HERE, t0 + Duration::from_secs(15)).is_none());
    assert!(s.request_due(HERE, t0 + Duration::from_secs(19)).is_some());
}

#[test]
fn a_report_with_nobody_waits_the_plain_period() {
    let t0 = Instant::now();
    let mut s = Schedule::new(t0);
    assert!(s.report_due(NEIGHBOUR, t0), "an unknown region's timers start expired");
    s.reported(NEIGHBOUR, t0, 0);
    assert!(!s.report_due(NEIGHBOUR, t0 + Duration::from_secs(59)));
    assert!(s.report_due(NEIGHBOUR, t0 + Duration::from_secs(60)));
    s.retain(|h| h == HERE);
    assert!(s.report_due(NEIGHBOUR, t0 + Duration::from_secs(61)), "forgotten, expired again");
}
