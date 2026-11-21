//! The LL viewer's stitcher tests (indra/newview/tests/
//! llchatservicehistorycore_test.cpp, LL viewer 26.5), case by case with
//! the same inputs and expectations; `case_NN` is the C++ `test<NN>`. The
//! randomized oracles draw from a fixed-seed generator of their own.

use super::merge::*;
use super::message::*;
use aurora_llsd::Llsd;
use aurora_net::chat_service::{created_at_seconds, format_time_uuid};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

type Hm = HistoryMessage;

const AGENT: Uuid = Uuid::from_u128(1);
const RESIDENT: Uuid = Uuid::from_u128(2);
const FIRST: &str = "00000001-0000-1000-8000-000000000000";
const SECOND: &str = "00000002-0000-1000-8000-000000000000";

/// LLDate(created).secondsSinceEpoch() truncated to S32.
fn seconds(created: &str) -> i32 {
    created_at_seconds(created).expect("fixture time parses") as i32
}

/// FIRST with its first four bytes set to `ordinal` (big-endian).
fn ordinal_id(ordinal: u32) -> String {
    format!("{ordinal:08x}-0000-1000-8000-000000000000")
}

/// Repeated incoming bodies share a plaintext minute but have distinct
/// supplied seconds.
fn repeated_delivery(second: i32) -> Hm {
    Hm {
        from_id: Some(RESIDENT),
        from: "Example Resident".to_owned(),
        message: "ok".to_owned(),
        timestamp: Some(1_789_005_900 + second),
        log_timestamp: Some(1_789_005_900 + second),
        ..Hm::default()
    }
}

fn service_copy(delivery: &Hm, id: &str) -> Hm {
    Hm {
        msg_id: Some(id.to_owned()),
        ..delivery.clone()
    }
}

fn plaintext_copy(delivery: &Hm) -> Hm {
    Hm {
        from: delivery.from.clone(),
        message: delivery.message.clone(),
        legacy_wall_time: Some(f64::from(delivery.timestamp.unwrap_or(0) / 60 * 60) - 7.0 * 3600.0),
        ..Hm::default()
    }
}

/// Stable service ids let the view retain context without guessing
/// plaintext identity.
fn context_row(ordinal: u8) -> Hm {
    Hm {
        message: ordinal.to_string(),
        ..service_copy(&repeated_delivery(i32::from(ordinal) * 10), &ordinal_id(u32::from(ordinal)))
    }
}

fn live_copy(row: &Hm) -> Hm {
    Hm {
        msg_id: None,
        is_history: false,
        ..row.clone()
    }
}

/// One outgoing body repeated under distinct service ids.
fn outgoing_repeat(ordinal: u8, timestamp: i32) -> Hm {
    Hm {
        from_id: Some(AGENT),
        message: "linden bear castle".to_owned(),
        timestamp: Some(timestamp),
        log_timestamp: None,
        ..context_row(ordinal)
    }
}

fn echo_with(service: &Hm, skew: i32, context: &LiveContext) -> Hm {
    let mut timestamp = 0;
    let now = (service.timestamp.unwrap_or(0) + skew) as u32;
    let context = capture_live_message(&service.message, &mut timestamp, context, now);
    let mut echo = live_copy(service);
    echo.timestamp = Some(timestamp as i32);
    record_live_message(&mut echo, &context, now);
    echo
}

/// The real outgoing capture path, original text and log time included.
fn local_echo(service: &Hm, skew: i32) -> Hm {
    echo_with(service, skew, &LiveContext::default())
}

/// An outgoing capture with the service boundary known before sending.
fn captured_outgoing(service: &Hm, history: &History, skew: i32) -> Hm {
    let context = LiveContext {
        after_msg_id: Some(format_time_uuid(&history.latest_service_key())),
        ..LiveContext::default()
    };
    echo_with(service, skew, &context)
}

fn source(name: &str) -> Option<TimeSource> {
    Some(match name {
        "local" => TimeSource::Local,
        "receipt" => TimeSource::Receipt,
        "sent" => TimeSource::Sent,
        _ => TimeSource::Unmarked,
    })
}

fn filter(history: &[Hm], live: &[Hm]) -> Messages {
    filter_direct_history_duplicates(history, live)
}

fn same(history: &Hm, timed: &Hm) -> bool {
    same_direct_history_occurrence(history, timed)
}

/// A fixed-seed SplitMix64 for the randomized oracles.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 32) as u32
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            items.swap(i, j);
        }
    }
}

#[test]
fn case_13_legacy_and_dotted_names_are_one_sender() {
    assert!(
        same_direct_sender_name("Bridie Linden", "bridie.linden"),
        "legacy and resident names match"
    );
    assert!(
        !same_direct_sender_name("bridie.linden", "beanie.tester"),
        "different residents stay distinct"
    );
}

#[test]
fn case_14_legacy_admission_uses_utc_minus_7_strictly() {
    let wall = 1000.0;
    let daylight_utc = wall + 7.0 * 3600.0;
    assert!(
        legacy_wall_may_precede_service(wall, daylight_utc + 1.0),
        "possible pre-boundary row retained"
    );
    assert!(!legacy_wall_may_precede_service(wall, daylight_utc), "row at boundary excluded");
    assert!(
        !legacy_wall_may_precede_service(wall, daylight_utc - 1.0),
        "post-boundary row excluded"
    );
    assert!(
        legacy_wall_may_precede_service(wall, daylight_utc + 30.0 * 60.0),
        "ambiguous DST row retained"
    );
}

#[test]
fn case_15_complete_names_compare_their_username() {
    assert!(
        same_direct_sender_name("Bridie (bridie.linden)", "Bridie Linden"),
        "complete name matches legacy resident name"
    );
    assert!(
        same_direct_sender_name("Bridie (bridie.linden)", "bridie.linden"),
        "complete name matches dotted resident name"
    );
    assert!(
        same_direct_sender_name("beanie lin alt (danyanka)", "danyanka Resident"),
        "single-name resident matches Resident suffix"
    );
    assert!(
        same_direct_sender_name("BRIDIE.LINDEN", "Bridie Linden"),
        "resident names compare without case"
    );
    assert!(
        same_direct_sender_name("Old Name (bridie.linden)", "New Name (bridie.linden)"),
        "different display names for one resident match"
    );
    assert!(
        !same_direct_sender_name("Bridie (bridie.linden)", "Bridie (danyanka)"),
        "same display name for different residents stays distinct"
    );
    assert!(
        !same_direct_sender_name("Bridie", "Bridie (bridie.linden)"),
        "display name alone does not identify the resident"
    );
    assert!(!same_direct_sender_name("", ""), "missing names do not identify a resident");
}

#[test]
fn case_16_service_rows_compare_the_original_body() {
    let history = Hm {
        from_id: Some(RESIDENT),
        message: "hello 1".to_owned(),
        timestamp: Some(1_789_005_928),
        msg_id: Some(FIRST.to_owned()),
        ..Hm::default()
    };
    let mut delivered = Hm {
        msg_id: None,
        message: "(Saved Wed Sep 09 22:05:28 2026)hello 1".to_owned(),
        original_text: Some("hello 1".to_owned()),
        ..history.clone()
    };
    assert!(same(&history, &delivered), "offline service copy matches delivered original");
    delivered.message = "(Enregistre le mercredi)hello 1".to_owned();
    assert!(same(&history, &delivered), "offline notice localization does not affect identity");
    delivered.message = "hello 1 (bonjour 1)".to_owned();
    assert!(same(&history, &delivered), "translation does not affect identity");

    // Never infer original text by stripping something the sender may have typed.
    delivered.message = "(Saved Wed Sep 09 22:05:28 2026)hello 1".to_owned();
    delivered.original_text = Some(delivered.message.clone());
    assert!(!same(&history, &delivered), "literal saved prefix remains message content");
    delivered.original_text = None;
    assert!(!same(&history, &delivered), "unidentified decorated text is not guessed");
    delivered.message = "hello 1".to_owned();
    assert!(same(&history, &delivered), "plain live delivery still matches");
    delivered.from_id = Some(AGENT);
    assert!(!same(&history, &delivered), "another sender stays distinct");
    delivered.from_id = Some(RESIDENT);
    delivered.timestamp = Some(1_789_005_988);
    assert!(!same(&history, &delivered), "another send minute stays distinct");
}

#[test]
fn case_17_plaintext_compares_the_logged_minute_and_body() {
    let delivered = Hm {
        from_id: Some(RESIDENT),
        from: "peppertest Resident".to_owned(),
        message: "(Saved Wed Sep 09 22:05:28 2026)hello 1".to_owned(),
        original_text: Some("hello 1".to_owned()),
        timestamp: Some(1_789_005_928),
        log_timestamp: Some(1_789_092_365),
        ..Hm::default()
    };
    let logged_minute = f64::from(1_789_092_365 / 60 * 60);
    let mut legacy = Hm {
        from: "peppertest".to_owned(),
        message: delivered.message.clone(),
        legacy_wall_time: Some(logged_minute - 7.0 * 3600.0),
        ..Hm::default()
    };
    assert!(same(&legacy, &delivered), "plaintext matches the delivery minute");
    legacy.legacy_wall_time = Some(logged_minute - 8.0 * 3600.0);
    assert!(same(&legacy, &delivered), "plaintext accepts the standard-time SLT offset");
    legacy.message = "hello 1".to_owned();
    assert!(!same(&legacy, &delivered), "plaintext matches its logged body exactly");
    legacy.message = delivered.message.clone();
    legacy.legacy_wall_time = None;
    assert!(!same(&legacy, &delivered), "undated plaintext is not guessed");
}

#[test]
fn case_18_interleave_by_time_keeping_live_order() {
    let row = |text: &str, timestamp: i32| Hm {
        message: text.to_owned(),
        timestamp: Some(timestamp),
        ..Hm::default()
    };
    let live = vec![
        row("hello 3", 1_789_005_932),
        row("hello 2", 1_789_005_930),
        row("hello 1", 1_789_005_928),
    ];
    let history = vec![row("hello", 1_789_005_955), row("older", 1_789_005_800)];
    let merged = interleave_direct_history(history.clone(), live.clone());
    let order: Vec<&str> = merged.iter().map(|m| m.message.as_str()).collect();
    assert_eq!(order, ["hello", "hello 3", "hello 2", "hello 1", "older"]);

    // Timestamp ties keep the existing live row later; undated history stays at the start.
    let merged = interleave_direct_history(vec![row("tie", 1_789_005_930), row("undated", 0)], live.clone());
    let order: Vec<&str> = merged.iter().map(|m| m.message.as_str()).collect();
    assert_eq!(order, ["hello 3", "hello 2", "tie", "hello 1", "undated"]);
    assert!(live.len() == 3 && history.len() == 2, "interleaving leaves source lists unchanged");
}

#[test]
fn case_19_exact_source_needs_the_same_second() {
    let history = Hm {
        from_id: Some(RESIDENT),
        message: "ok".to_owned(),
        timestamp: Some(1_789_005_905),
        msg_id: Some(FIRST.to_owned()),
        ..Hm::default()
    };
    let mut delivered = Hm {
        msg_id: None,
        timestamp: Some(1_789_005_935),
        ..history.clone()
    };
    assert!(!same(&history, &delivered), "a later repeat in the same minute stays distinct");
    delivered.timestamp = Some(1_789_005_904);
    assert!(!same(&history, &delivered), "a preceding send in the same minute stays distinct");
    delivered.timestamp = Some(1_789_005_905);
    assert!(same(&history, &delivered), "the matching service second reconciles");
    delivered.timestamp = Some(0);
    assert!(!same(&history, &delivered), "an untimed delivery stays distinct");
}

#[test]
fn case_20_local_echo_tolerates_59_seconds() {
    let mut history = Hm {
        from_id: Some(AGENT),
        message: "hello".to_owned(),
        timestamp: Some(1_789_005_905),
        msg_id: Some(FIRST.to_owned()),
        ..Hm::default()
    };
    let mut echo = Hm {
        msg_id: None,
        time_source: Some(TimeSource::Local),
        timestamp: Some(1_789_005_904),
        ..history.clone()
    };
    assert!(same(&history, &echo), "local send before persistence reconciles");
    echo.timestamp = Some(1_789_005_906);
    assert!(same(&history, &echo), "local clock ahead of the server reconciles");

    echo.message = "hello (bonjour)".to_owned();
    echo.original_text = Some("hello".to_owned());
    assert!(same(&history, &echo), "translated local echo reconciles its original body");
    echo.original_text = Some("another message".to_owned());
    assert!(!same(&history, &echo), "different local body stays distinct");
    echo.original_text = Some("hello".to_owned());
    echo.from_id = Some(RESIDENT);
    assert!(!same(&history, &echo), "another sender stays distinct for local time");
    echo.from_id = Some(AGENT);
    echo.timestamp = Some(1_789_005_965);
    assert!(!same(&history, &echo), "local time a full minute away stays distinct");

    echo.timestamp = Some(1_789_005_906);
    echo.time_source = Some(TimeSource::Sent);
    assert!(!same(&history, &echo), "supplied outgoing timestamp requires exact seconds");
    echo.timestamp = Some(1_789_005_905);
    assert!(same(&history, &echo), "supplied outgoing timestamp matches its service second");
    echo.time_source = Some(TimeSource::Local);
    history.timestamp = Some(0);
    assert!(
        !same(&history, &echo),
        "local provenance does not make an untimed service row match"
    );
}

#[test]
fn case_21_plaintext_and_one_service_row_reconcile_in_any_order() {
    let earlier = repeated_delivery(5);
    let later = repeated_delivery(35);
    let legacy = plaintext_copy(&earlier);
    let service = service_copy(&later, FIRST);
    for history_reversed in [false, true] {
        for live_reversed in [false, true] {
            let mut history = vec![legacy.clone(), service.clone()];
            let mut live = vec![later.clone(), earlier.clone()];
            if history_reversed {
                history.reverse();
            }
            if live_reversed {
                live.reverse();
            }
            assert!(
                filter(&history, &live).is_empty(),
                "partial service coverage reconciles both deliveries in either order"
            );
        }
    }
}

#[test]
fn case_22_one_for_one_consumption_keeps_extras_in_place() {
    let earlier = repeated_delivery(5);
    let later = repeated_delivery(35);
    let legacy = plaintext_copy(&earlier);
    let keep_before = Hm {
        message: "before".to_owned(),
        ..legacy.clone()
    };
    let keep_after = Hm {
        message: "after".to_owned(),
        ..legacy.clone()
    };
    let history = vec![legacy.clone(), keep_before, service_copy(&later, FIRST), legacy, keep_after];
    let live = vec![later, earlier];

    let filtered = filter(&history, &live);
    assert_eq!(filtered.len(), 3, "each live occurrence is consumed once");
    assert_eq!(filtered[0].message, "before", "first unmatched row keeps its position");
    assert!(
        filtered[1].msg_id.is_none(),
        "extra repeated occurrence retains its plaintext source"
    );
    assert_eq!(filtered[1].message, "ok", "extra repeated occurrence remains");
    assert_eq!(filtered[2].message, "after", "last unmatched row keeps its position");
    assert_eq!(history.len(), 5, "history input is unchanged");
    assert_eq!(live.len(), 2, "live input is unchanged");
    assert_eq!(filter(&history, &[]).len(), history.len(), "no live rows leaves history intact");
    assert!(filter(&[], &live).is_empty(), "no history leaves an empty result");
}

#[test]
fn case_23_exact_matches_are_reserved_before_local_fallback() {
    let supplied = repeated_delivery(5);
    let local = Hm {
        time_source: Some(TimeSource::Local),
        ..repeated_delivery(34)
    };
    let service_earlier = service_copy(&supplied, FIRST);
    let service_later = service_copy(&repeated_delivery(35), SECOND);
    for history_reversed in [false, true] {
        for live_reversed in [false, true] {
            let mut history = vec![service_earlier.clone(), service_later.clone()];
            let mut live = vec![local.clone(), supplied.clone()];
            if history_reversed {
                history.reverse();
            }
            if live_reversed {
                live.reverse();
            }
            assert!(
                filter(&history, &live).is_empty(),
                "exact service matches precede local-clock fallback"
            );
        }
    }
}

#[test]
fn case_24_translated_deliveries_consume_both_representations() {
    let mut earlier = repeated_delivery(5);
    let mut later = repeated_delivery(35);
    let service_earlier = service_copy(&earlier, FIRST);
    let service_later = service_copy(&later, SECOND);
    for delivery in [&mut earlier, &mut later] {
        delivery.original_text = Some("ok".to_owned());
        delivery.message = "ok (d'accord)".to_owned();
    }
    let legacy = plaintext_copy(&earlier);
    let mut history = vec![legacy.clone(), service_later, legacy.clone(), service_earlier.clone()];
    let live = vec![later, earlier];
    assert!(
        filter(&history, &live).is_empty(),
        "decorated plaintext and raw service copies both reconcile"
    );
    history.insert(0, legacy);
    history.push(service_earlier);
    let filtered = filter(&history, &live);
    assert_eq!(filtered.len(), 2, "extra occurrences in each representation remain");
    assert!(filtered[0].msg_id.is_none(), "extra decorated plaintext retains its source");
    assert_eq!(filtered[1].msg_id.as_deref(), Some(FIRST), "extra service retains its source");
}

#[test]
fn case_25_receipt_window_admits_lead_and_delay() {
    let mut history = service_copy(&repeated_delivery(0), FIRST);
    history.message = "123".to_owned();
    history.datetime = "2026-09-10T17:17:00.904000Z".to_owned();
    history.timestamp = Some(seconds(&history.datetime));
    let mut delivered = Hm {
        msg_id: None,
        timestamp: Some(1_789_060_621),
        time_source: Some(TimeSource::Receipt),
        ..history.clone()
    };
    assert!(
        filter(std::slice::from_ref(&history), &[delivered.clone()]).is_empty(),
        "online delivery reconciles the preceding service second"
    );

    delivered.time_source = Some(TimeSource::Sent);
    assert!(!same(&history, &delivered), "offline send time stays exact");
    delivered.timestamp = Some(1_789_060_619);
    assert!(!same(&history, &delivered), "offline send time rejects later service recording");
    delivered.time_source = Some(TimeSource::Receipt);
    assert!(same(&history, &delivered), "online receipt can precede service recording");
    delivered.timestamp = Some(1_789_060_681);
    assert!(!same(&history, &delivered), "delivery outside the rolling window stays distinct");

    history.timestamp = Some(1_789_060_619);
    delivered.timestamp = Some(1_789_060_620);
    assert!(same(&history, &delivered), "delivery delay crosses a minute boundary");
    delivered.from_id = Some(AGENT);
    assert!(!same(&history, &delivered), "delivery tolerance retains sender identity");
    delivered.from_id = Some(RESIDENT);
    delivered.message = "1234".to_owned();
    assert!(!same(&history, &delivered), "delivery tolerance retains body identity");
}

#[test]
fn case_26_receipt_takes_the_newest_eligible_repeat() {
    let earlier = service_copy(&repeated_delivery(0), FIRST);
    let later = service_copy(&repeated_delivery(1), SECOND);
    let delivered = Hm {
        time_source: Some(TimeSource::Receipt),
        ..repeated_delivery(1)
    };
    for reversed in [false, true] {
        let mut history = vec![earlier.clone(), later.clone()];
        if reversed {
            history.reverse();
        }
        let filtered = filter(&history, std::slice::from_ref(&delivered));
        assert_eq!(filtered.len(), 1, "one live delivery consumes one service occurrence");
        assert_eq!(filtered[0].msg_id.as_deref(), Some(FIRST), "the earlier repeated send survives");
    }
}

#[test]
fn case_27_two_receipts_in_one_second_and_delivery_priority() {
    let earlier = service_copy(&repeated_delivery(0), FIRST);
    let later = service_copy(&repeated_delivery(1), SECOND);
    let first_delivery = Hm {
        time_source: Some(TimeSource::Receipt),
        ..repeated_delivery(1)
    };
    let second_delivery = first_delivery.clone();
    for reversed in [false, true] {
        let mut history = vec![earlier.clone(), later.clone()];
        if reversed {
            history.reverse();
        }
        assert!(
            filter(&history, &[first_delivery.clone(), second_delivery.clone()]).is_empty(),
            "both adjacent service occurrences reconcile one-for-one"
        );
    }

    let local = Hm {
        time_source: Some(TimeSource::Local),
        ..repeated_delivery(0)
    };
    let much_later = service_copy(&repeated_delivery(35), SECOND);
    assert!(
        filter(&[earlier.clone(), much_later], &[local, first_delivery.clone()]).is_empty(),
        "delivery matching precedes local-clock matching"
    );
    let legacy = plaintext_copy(&first_delivery);
    assert!(
        filter(&[legacy, earlier], &[first_delivery, repeated_delivery(35)]).is_empty(),
        "delivery matching precedes plaintext matching"
    );
}

#[test]
fn case_28_receipt_window_across_minute_hour_and_day() {
    for created in [
        "2026-09-10T12:34:01Z",
        "2026-09-10T12:59:59Z",
        "2026-09-10T13:00:01Z",
        "2026-09-10T23:59:59Z",
        "2026-09-11T00:00:01Z",
    ] {
        let timestamp = seconds(created);
        let history = Hm {
            timestamp: Some(timestamp),
            ..service_copy(&repeated_delivery(0), FIRST)
        };
        let mut delivered = Hm {
            time_source: Some(TimeSource::Receipt),
            ..repeated_delivery(0)
        };
        for delay in [-5, -4, -2, -1, 0, 2, 5, 30, 60] {
            delivered.timestamp = Some(timestamp + delay);
            assert!(
                filter(std::slice::from_ref(&history), std::slice::from_ref(&delivered)).is_empty(),
                "receipt inside the rolling window reconciles"
            );
        }
        for delay in [-6, 61] {
            delivered.timestamp = Some(timestamp + delay);
            assert_eq!(
                filter(std::slice::from_ref(&history), std::slice::from_ref(&delivered)).len(),
                1,
                "receipt outside the rolling window remains"
            );
        }
    }
}

#[test]
fn case_29_repeated_receipts_pair_independently_of_order() {
    for times in [[0, 15, 5, 30], [0, 15, 30, 75], [0, 10, 10, 65], [65, 70, 60, 65], [0, 6, -4, 2]] {
        let first = service_copy(&repeated_delivery(times[0]), FIRST);
        let second = service_copy(&repeated_delivery(times[1]), SECOND);
        let first_delivery = Hm {
            time_source: Some(TimeSource::Receipt),
            ..repeated_delivery(times[2])
        };
        let second_delivery = Hm {
            time_source: Some(TimeSource::Receipt),
            ..repeated_delivery(times[3])
        };
        for history_reversed in [false, true] {
            for live_reversed in [false, true] {
                let mut history = vec![first.clone(), second.clone()];
                let mut live = vec![first_delivery.clone(), second_delivery.clone()];
                if history_reversed {
                    history.reverse();
                }
                if live_reversed {
                    live.reverse();
                }
                assert!(
                    filter(&history, &live).is_empty(),
                    "delayed repeats reconcile independently of source order"
                );
            }
        }
    }
}

#[test]
fn case_30_preview_is_idempotent_and_loaded_values_win() {
    let first = service_copy(&repeated_delivery(5), FIRST);
    let second = service_copy(&repeated_delivery(35), SECOND);
    let legacy = plaintext_copy(&first);
    let mut history = History::default();
    history.set_loaded(merge_direct_history(
        &[legacy.clone(), legacy.clone(), legacy.clone()],
        std::slice::from_ref(&first),
    ));

    let expected = vec![legacy, first.clone(), second.clone()];
    for _ in 0..3 {
        let result = history.compose(&[second.clone(), first.clone()], &[], 10, false);
        assert_eq!(result, expected, "preview is idempotent and chronological");
    }

    let stale = Hm {
        message: "stale preview".to_owned(),
        ..first
    };
    let limited = history.compose(&[second.clone(), stale, second], &[], 2, false);
    assert_eq!(limited.len(), 2, "duplicate IDs do not evict other history");
    assert_eq!(limited[0].message, "ok", "canonical value wins");
    assert_eq!(limited[1].msg_id.as_deref(), Some(SECOND), "service tail stays in TimeUUID order");
}

#[test]
fn case_31_retained_context_survives_rollover_and_releases() {
    let initial = vec![context_row(1), context_row(2), context_row(3)];
    let bounded = vec![context_row(2), context_row(3), context_row(4)];
    let delivered = live_copy(&context_row(4));
    let mut history = History::default();
    history.set_loaded(initial.clone());
    history.compose(&[], &[], 3, true);
    history.set_loaded(bounded.clone());
    let result = history.compose(&[], std::slice::from_ref(&delivered), 3, true);
    assert_eq!(result.len(), 3, "live copy does not evict older visible context");
    assert_eq!(result[0].message, "1", "oldest context survives rollover");

    // A late delivery may replace any retained row, including the newest one.
    for ordinal in [2, 3] {
        history.clear();
        history.set_loaded(initial.clone());
        history.compose(&[], &[], 3, true);
        history.set_loaded(bounded.clone());
        history.compose(&[], std::slice::from_ref(&delivered), 3, true);
        for _ in 0..3 {
            let result = history.compose(&[], &[live_copy(&context_row(ordinal)), delivered.clone()], 3, true);
            assert_eq!(result.len(), 2, "late live delivery replaces one retained occurrence");
            assert_eq!(result[0].message, "1", "unrelated context remains");
        }
    }

    history.clear();
    history.set_loaded(initial.clone());
    history.compose(&[], &[], 3, true);
    history.set_loaded(vec![context_row(4), context_row(5), context_row(6)]);
    let live = vec![live_copy(&context_row(6)), live_copy(&context_row(5)), delivered];
    assert_eq!(
        history.compose(&[], &live, 3, true).len(),
        3,
        "an all-live source retains open history"
    );
    assert_eq!(
        history.compose(&[], &live, 3, true).len(),
        3,
        "repeated all-live publication is stable"
    );
    assert!(history.compose(&[], &live, 3, false).is_empty(), "hidden view releases old context");
    history.set_loaded(initial);
    history.compose(&[], &[], 3, true);
    history.set_loaded(Vec::new());
    assert!(history.compose(&[], &[], 3, true).is_empty(), "empty load clears retained context");

    // Revoking service consent leaves plaintext, while deletion clears both sources.
    let legacy = plaintext_copy(&repeated_delivery(5));
    history.set_loaded(vec![legacy, context_row(1)]);
    history.compose(&[], &[], 3, true);
    history.clear_service();
    assert_eq!(history.compose(&[], &[], 3, true).len(), 1, "service revocation keeps legacy");
    history.clear();
    assert!(
        history.compose(&[], &[], 3, true).is_empty(),
        "clear removes every historical source"
    );
}

#[test]
fn case_32_replace_history_keeps_live_rows_and_offers() {
    let mut offer = Hm {
        timestamp: Some(0),
        is_history: false,
        ..Hm::default()
    };
    offer.extra.insert("notification_id".to_owned(), Llsd::Uuid(AGENT));
    let first = Hm {
        message: "test".to_owned(),
        ..live_copy(&context_row(4))
    };
    let second = Hm {
        timestamp: Some(first.timestamp.unwrap_or(0) + 1),
        ..first.clone()
    };
    let mut current = vec![second, first, offer];
    assert!(
        replace_history(&mut current, &[context_row(1)], true),
        "initial history changes the model"
    );
    assert_eq!(current.len(), 4, "two live tests and the offer survive");
    assert_eq!(current[0].message, "test", "latest test stays live");
    assert_eq!(current[1].message, "test", "first test stays live");
    assert_eq!(
        current[2].extra.get("notification_id"),
        Some(&Llsd::Uuid(AGENT)),
        "inline offer identity survives"
    );
    assert!(
        !replace_history(&mut current, &[context_row(1)], true),
        "identical publication avoids replay"
    );
    assert!(replace_history(&mut current, &[], true), "clearing history changes the model");
    assert_eq!(current.len(), 3, "clearing history retains both live sends and offer");
    let indexes: Vec<Option<i32>> = current.iter().map(|m| m.index).collect();
    assert_eq!(indexes, [Some(2), Some(1), Some(0)], "indexes remain contiguous");
}

#[test]
fn case_33_capture_keeps_provenance_through_translation() {
    let received: u32 = 1_789_060_621;
    for supplied in [false, true] {
        let mut timestamp = if supplied { received } else { 0 };
        let captured = capture_live_message("123", &mut timestamp, &incoming_context(Some("123"), true), received);
        assert_eq!(timestamp, received, "supplied and synthesized receipt times agree");
        let captured = capture_live_message("123 (translated)", &mut timestamp, &captured, received + 90);
        let mut row = repeated_delivery(0);
        row.timestamp = Some(timestamp as i32);
        row.message = "123 (translated)".to_owned();
        record_live_message(&mut row, &captured, received + 90);
        assert_eq!(row.original_text.as_deref(), Some("123"), "translation preserves original body");
        assert_eq!(
            row.time_source,
            Some(TimeSource::Receipt),
            "translation preserves receipt provenance"
        );
        assert_eq!(
            row.log_timestamp,
            Some((received + 90) as i32),
            "logging time follows translation completion"
        );
        let mut service = service_copy(&repeated_delivery(0), FIRST);
        service.message = "123".to_owned();
        service.timestamp = Some((received - 1) as i32);
        assert!(same(&service, &row), "translation delay does not affect service matching");
        service.timestamp = Some((received + 1) as i32);
        assert!(same(&service, &row), "later service recording matches the captured receipt");
    }

    let mut timestamp = received - 86_400;
    let offline = capture_live_message(
        "(Saved yesterday)123",
        &mut timestamp,
        &incoming_context(Some("123"), false),
        received,
    );
    assert_eq!(timestamp, received - 86_400, "offline delivery keeps the send time");
    assert_eq!(offline.time_source, Some(TimeSource::Sent), "offline time is authoritative");
    let mut timestamp = 0;
    let offline = capture_live_message("(Saved)123", &mut timestamp, &incoming_context(Some("123"), false), received);
    assert_eq!(
        offline.time_source,
        Some(TimeSource::Local),
        "missing send time cannot become authoritative"
    );
    let mut timestamp = 0;
    let echo = capture_live_message("123", &mut timestamp, &LiveContext::default(), received);
    assert_eq!(echo.time_source, Some(TimeSource::Local), "outgoing echo captures its local clock");
    assert_eq!(timestamp, received, "outgoing echo captures before translation");
}

#[test]
fn case_34_rollover_keeps_plaintext_and_windows_keep_counts() {
    let old = context_row(1);
    let legacy = plaintext_copy(&old);
    let live = live_copy(&context_row(4));
    let mut history = History::default();
    history.set_loaded(vec![legacy, context_row(2), context_row(3)]);
    history.compose(&[], &[], 3, true);
    history.set_loaded(vec![context_row(2), context_row(3), context_row(4)]);
    for _ in 0..3 {
        let result = history.compose(&[], std::slice::from_ref(&live), 3, true);
        assert_eq!(result.len(), 3, "capped service rollover retains older plaintext");
        assert!(result[0].msg_id.is_none(), "retained plaintext keeps its source");
    }
    history.set_loaded(vec![old.clone(), context_row(2), context_row(3)]);
    let result = history.compose(&[], std::slice::from_ref(&live), 3, true);
    assert_eq!(result.len(), 3, "new archive row replaces retained plaintext");
    assert_eq!(result[0].msg_id, old.msg_id, "replaced plaintext acquires the canonical ID");

    // Each window may already have consumed a different member of repeated plaintext.
    let first = service_copy(&repeated_delivery(5), FIRST);
    let second = service_copy(&repeated_delivery(35), SECOND);
    let repeat = plaintext_copy(&first);
    let repeats = vec![repeat.clone(), repeat.clone(), repeat];
    let left = merge_direct_history(&repeats, &[first]);
    let right = merge_direct_history(&repeats, &[second]);
    let result = merge_direct_history(&left, &right);
    assert_eq!(result.len(), 3, "overlapping windows preserve three original occurrences");
    assert_eq!(
        merge_direct_history(&result, &right).len(),
        3,
        "repeated seam publication is stable"
    );
    assert_eq!(merge_direct_history(&right, &left).len(), 3, "source window order preserves counts");
}

#[test]
fn case_35_recreated_session_keeps_the_earlier_repeat() {
    let earlier = Hm {
        message: "hello".to_owned(),
        timestamp: Some(1_789_092_403),
        ..context_row(1)
    };
    let between = Hm {
        message: "6".to_owned(),
        timestamp: Some(1_789_092_414),
        ..context_row(2)
    };
    let later = Hm {
        message: "hello".to_owned(),
        timestamp: Some(1_789_092_446),
        ..context_row(3)
    };
    let archive = vec![earlier.clone(), between.clone(), later.clone()];

    for delay in [-5, -1, 0, 2, 15] {
        let mut history = History::default();
        history.set_loaded(vec![earlier.clone(), between.clone()]);
        let delivery = Hm {
            time_source: Some(TimeSource::Receipt),
            timestamp: Some(later.timestamp.unwrap_or(0) + delay),
            ..live_copy(&later)
        };
        let mut current = vec![delivery.clone()];

        // The archive read can complete before the latest service head arrives.
        let composed = history.compose(&[], std::slice::from_ref(&delivery), 10, true);
        replace_history(&mut current, &composed, true);
        for publication in 0..3 {
            let composed = history.compose(&archive, std::slice::from_ref(&delivery), 10, true);
            assert_eq!(composed.len(), 2, "one delivery replaces only one historical occurrence");
            assert_eq!(
                composed[0].msg_id, earlier.msg_id,
                "the earlier hello retains its identity and position"
            );
            let changed = replace_history(&mut current, &composed, true);
            assert_eq!(changed, publication == 0, "unchanged publications do not replay the transcript");
            assert_eq!(current.len(), 3, "both hellos and the intervening message remain");
        }

        // Reopening with all rows historical preserves the same message order.
        let mut reopened = Vec::new();
        replace_history(&mut reopened, &archive, true);
        let order = |rows: &Messages| rows.iter().map(|m| m.message.clone()).collect::<Vec<_>>();
        assert_eq!(order(&current), order(&reopened), "reopening preserves message order");
    }
}

#[test]
fn case_37_online_replies_without_wire_time_reconcile() {
    let mut archive = Vec::new();
    let mut live = Vec::new();
    for (ordinal, body) in [(1u8, "30"), (2, "thnk you")] {
        let received: u32 = if ordinal == 2 { 1_789_489_070 } else { 1_789_489_012 };
        let service = Hm {
            message: body.to_owned(),
            timestamp: Some((received + 1) as i32),
            ..context_row(ordinal)
        };
        archive.push(service.clone());

        let mut timestamp = 0;
        let context = capture_live_message(body, &mut timestamp, &incoming_context(Some(body), true), received);
        let mut delivery = live_copy(&service);
        delivery.timestamp = Some(timestamp as i32);
        record_live_message(&mut delivery, &context, received);
        live.insert(0, delivery);
    }

    for loaded_first in [false, true] {
        let mut history = History::default();
        if loaded_first {
            history.set_loaded(archive.clone());
        }
        let mut current = live.clone();
        for _ in 0..3 {
            let composed = history.compose(&archive, &live, 200, true);
            assert!(composed.is_empty(), "recording after receipt does not add history copies");
            assert!(
                !replace_history(&mut current, &composed, true),
                "unchanged live replies do not need replay"
            );
            assert_eq!(current.len(), 2, "both replies remain visible once");
            history.set_loaded(archive.clone());
        }

        let durable = history.compose(&[], &live, 200, true);
        assert!(durable.is_empty(), "durable read reconciles after preview expires");
        assert!(
            !replace_history(&mut current, &durable, true),
            "durable publication does not replay replies"
        );
        assert_eq!(current[0].message, "thnk you", "newest reply retains delivery order");
        assert_eq!(current[1].message, "30", "earliest reply retains delivery order");
        assert!(!current[0].is_history && !current[1].is_history, "both replies stay live");
    }
}

#[test]
fn case_38_one_receipt_consumes_one_row() {
    let earlier = Hm {
        message: "hello".to_owned(),
        ..context_row(1)
    };
    let between = Hm {
        message: "6".to_owned(),
        timestamp: Some(earlier.timestamp.unwrap_or(0) + 15),
        ..context_row(2)
    };
    let later = Hm {
        message: "hello".to_owned(),
        timestamp: Some(earlier.timestamp.unwrap_or(0) + 30),
        ..context_row(3)
    };
    let delivery = Hm {
        time_source: Some(TimeSource::Receipt),
        ..live_copy(&earlier)
    };
    let mut history = History::default();
    history.set_loaded(vec![earlier, between, later.clone()]);

    let composed = history.compose(&[], std::slice::from_ref(&delivery), 200, true);
    assert_eq!(composed.len(), 2, "one receipt consumes one occurrence");
    assert_eq!(composed[1].msg_id, later.msg_id, "later repeat keeps its service ID");
    let mut current = vec![delivery];
    assert!(
        replace_history(&mut current, &composed, true),
        "unseen service messages enter the transcript"
    );
    assert_eq!(current.len(), 3, "three real occurrences remain");
    assert_eq!(current[0].msg_id, later.msg_id, "later repeat follows intervening message");
    assert_eq!(current[1].message, "6", "intervening message keeps its position");
    assert!(!current[2].is_history, "earliest occurrence keeps its live delivery");
}

#[test]
fn case_39_local_echo_window_across_minute_hour_and_day() {
    for created in [
        "2026-10-30T17:28:59Z",
        "2026-10-30T17:29:00Z",
        "2026-10-30T17:59:59Z",
        "2026-10-30T18:00:00Z",
        "2026-10-30T23:59:59Z",
        "2026-10-31T00:00:00Z",
    ] {
        let service = outgoing_repeat(1, seconds(created));
        for skew in [-59, -1, 0, 1, 59] {
            assert!(
                filter(std::slice::from_ref(&service), &[local_echo(&service, skew)]).is_empty(),
                "local echo inside the rolling window reconciles"
            );
        }
        for skew in [-61, -60, 60, 61] {
            assert_eq!(
                filter(std::slice::from_ref(&service), &[local_echo(&service, skew)]).len(),
                1,
                "local echo outside the rolling window stays distinct"
            );
        }
    }
}

#[test]
fn case_40_outgoing_repeats_across_a_minute_each_take_one_row() {
    let boundary = seconds("2026-10-30T17:29:00Z");
    let earlier = outgoing_repeat(1, boundary - 30);
    let between = Hm {
        timestamp: Some(boundary - 15),
        ..context_row(2)
    };
    let mut archive = vec![earlier.clone(), between.clone()];
    let mut live = Vec::new();
    for ordinal in [3u8, 4] {
        let service = outgoing_repeat(ordinal, boundary - 5 + i32::from(ordinal));
        archive.push(service.clone());
        live.insert(0, local_echo(&service, 1));
    }

    for loaded_first in [false, true] {
        let mut history = History::default();
        if loaded_first {
            history.set_loaded(archive.clone());
        }
        let mut current = live.clone();
        for publication in 0..3 {
            let composed = history.compose(&archive, &live, 200, true);
            assert_eq!(composed.len(), 2, "each outgoing echo consumes one history copy");
            assert_eq!(composed[0].msg_id, earlier.msg_id, "earlier repeat retains its service ID");
            assert_eq!(composed[1].msg_id, between.msg_id, "intervening message retains its position");
            assert_eq!(
                replace_history(&mut current, &composed, true),
                publication == 0,
                "unchanged publications do not replay the transcript"
            );
            assert_eq!(current.len(), 4, "all four original messages remain");
            history.set_loaded(archive.clone());
        }
        let durable = history.compose(&[], &live, 200, true);
        assert!(
            !replace_history(&mut current, &durable, true),
            "durable publication does not replay the transcript"
        );
        assert!(!current[0].is_history, "outgoing message remains live");
    }
}

#[test]
fn case_41_plaintext_read_then_preview_across_a_minute() {
    let boundary = seconds("2026-10-30T17:29:00Z");
    let service = outgoing_repeat(1, boundary - 1);
    let echo = local_echo(&service, 1);
    let mut history = History::default();
    history.set_loaded(vec![plaintext_copy(&echo)]);
    let composed = history.compose(&[service], &[echo], 200, true);
    assert!(composed.is_empty(), "cross-minute service and plaintext copies both reconcile");
}

#[test]
fn case_42_later_repeat_in_preview_before_its_translation() {
    let boundary = seconds("2026-10-30T17:29:00Z");
    let earlier = outgoing_repeat(1, boundary - 1);
    let between = Hm {
        timestamp: Some(boundary + 5),
        ..context_row(2)
    };
    let later = outgoing_repeat(3, boundary + 20);
    let echo = local_echo(&earlier, 0);
    let mut history = History::default();
    history.set_loaded(vec![earlier, between, later.clone()]);
    let composed = history.compose(&[], &[echo], 200, true);
    assert_eq!(composed.len(), 2, "one local echo consumes one service occurrence");
    assert_eq!(composed[1].msg_id, later.msg_id, "later repeat keeps its service ID and position");
}

#[test]
fn case_43_retained_earlier_repeat_survives_rollover() {
    let boundary = seconds("2026-10-30T17:29:00Z");
    let earlier = outgoing_repeat(1, boundary + 5);
    let later = outgoing_repeat(2, boundary + 35);
    let echo = local_echo(&later, 0);
    let mut history = History::default();
    history.set_loaded(vec![earlier.clone(), later]);
    assert_eq!(
        history.compose(&[], std::slice::from_ref(&echo), 2, true).len(),
        1,
        "earlier occurrence is initially retained"
    );

    let third = Hm {
        timestamp: Some(boundary + 3600),
        ..context_row(3)
    };
    let fourth = Hm {
        timestamp: Some(boundary + 3601),
        ..context_row(4)
    };
    history.set_loaded(vec![third.clone(), fourth.clone()]);
    let composed = history.compose(&[], &[live_copy(&fourth), live_copy(&third), echo], 2, true);
    assert_eq!(composed.len(), 1, "archive rollover preserves the earlier distinct repeat");
    assert_eq!(composed[0].msg_id, earlier.msg_id, "retained repeat keeps its service ID");
}

#[test]
fn case_44_exact_second_local_is_not_reserved_first() {
    for times in [[0, 59, 59, 118], [59, 118, 0, 59], [0, 60, 59, 60], [0, 30, 29, 59]] {
        let first = service_copy(&repeated_delivery(times[0]), FIRST);
        let second = service_copy(&repeated_delivery(times[1]), SECOND);
        let first_echo = local_echo(&repeated_delivery(times[2]), 0);
        let second_echo = local_echo(&repeated_delivery(times[3]), 0);
        for history_reversed in [false, true] {
            for live_reversed in [false, true] {
                let mut history = vec![first.clone(), second.clone()];
                let mut live = vec![first_echo.clone(), second_echo.clone()];
                if history_reversed {
                    history.reverse();
                }
                if live_reversed {
                    live.reverse();
                }
                assert!(
                    filter(&history, &live).is_empty(),
                    "clock skew does not strand a repeated local echo"
                );
            }
        }
    }
}

#[test]
fn case_45_rollover_anchor_does_not_consume_extra_plaintext() {
    let boundary = seconds("2026-10-30T17:29:00Z");
    let legacy = plaintext_copy(&outgoing_repeat(1, boundary + 5));
    let later = outgoing_repeat(2, boundary + 35);
    let echo = local_echo(&later, 0);
    let mut history = History::default();
    history.set_loaded(vec![legacy.clone(), later.clone()]);
    assert_eq!(
        history.compose(&[], std::slice::from_ref(&echo), 2, true).len(),
        1,
        "distinct plaintext is initially retained"
    );

    let third = Hm {
        timestamp: Some(later.timestamp.unwrap_or(0) + 3600),
        ..context_row(3)
    };
    let fourth = Hm {
        timestamp: Some(third.timestamp.unwrap_or(0) + 1),
        ..context_row(4)
    };
    history.set_loaded(vec![third.clone(), fourth.clone()]);
    for _ in 0..3 {
        let composed = history.compose(&[], &[live_copy(&fourth), live_copy(&third), echo.clone()], 2, true);
        assert_eq!(composed.len(), 1, "rollover keeps the extra plaintext occurrence");
        assert_eq!(composed[0], legacy, "the retained plaintext is unchanged");
    }
}

#[test]
fn case_46_closest_complete_pairing_keeps_the_undelivered_repeat() {
    let boundary = seconds("2026-10-30T17:29:00Z");
    let first = outgoing_repeat(1, boundary);
    let second = outgoing_repeat(2, boundary + 2);
    let third = outgoing_repeat(3, boundary + 3);
    let first_echo = local_echo(&first, 1);
    let second_echo = local_echo(&second, 0);
    let composed = filter(&[first, second, third.clone()], &[second_echo, first_echo]);
    assert_eq!(composed.len(), 1, "two local echoes consume two service occurrences");
    assert_eq!(
        composed[0].msg_id, third.msg_id,
        "closest pairing retains the undelivered later repeat"
    );
}

#[test]
fn case_47_sender_and_body_groups_are_independent() {
    let boundary = seconds("2026-10-30T17:29:00Z");
    for different_sender in [false, true] {
        let first = outgoing_repeat(1, boundary);
        let mut second = outgoing_repeat(2, boundary + 59);
        if different_sender {
            second.from_id = Some(RESIDENT);
        } else {
            second.message = "another place".to_owned();
        }
        let first_echo = local_echo(&first, 59);
        let mut second_echo = local_echo(&second, -59);
        second_echo.message = format!("{} (translation)", second.message);
        for history_reversed in [false, true] {
            for live_reversed in [false, true] {
                let mut history = vec![first.clone(), second.clone()];
                let mut live = vec![first_echo.clone(), second_echo.clone()];
                if history_reversed {
                    history.reverse();
                }
                if live_reversed {
                    live.reverse();
                }
                assert!(
                    filter(&history, &live).is_empty(),
                    "independent identities do not compete for chronological order"
                );
            }
        }
    }
}

/// Every retained subset reaching the most pairs, then the least total
/// skew, by exhaustive search (case 48's oracle).
struct MatchingOracle<'a> {
    service_times: &'a [i32],
    local_times: &'a [i32],
    best_count: i32,
    best_distance: i64,
    optimal_retained: BTreeSet<u32>,
}

impl MatchingOracle<'_> {
    fn explore(&mut self, index: usize, used: u32, retained: u32, count: i32, distance: i64) {
        if index == self.service_times.len() {
            if count > self.best_count || (count == self.best_count && distance < self.best_distance) {
                self.best_count = count;
                self.best_distance = distance;
                self.optimal_retained.clear();
            }
            if count == self.best_count && distance == self.best_distance {
                self.optimal_retained.insert(retained);
            }
            return;
        }
        // Leave this service occurrence historical, or consume it with each
        // unused eligible live occurrence.
        self.explore(index + 1, used, retained | (1 << index), count, distance);
        for local in 0..self.local_times.len() {
            let delta = (i64::from(self.service_times[index]) - i64::from(self.local_times[local])).abs();
            if used & (1 << local) == 0 && delta <= 59 {
                self.explore(index + 1, used | (1 << local), retained, count + 1, distance + delta);
            }
        }
    }
}

#[test]
fn case_48_randomized_exhaustive_matching_oracle() {
    let mut random = Random(0x51e7c4);
    for trial in 0..4000 {
        let service_count = random.below(7) as usize;
        let local_count = random.below(7) as usize;
        let mut service_times = Vec::new();
        let mut local_times = Vec::new();
        let mut service_rows = Vec::new();
        let mut local_rows = Vec::new();
        let mut service_bits = BTreeMap::new();
        for i in 0..service_count {
            let timestamp = 1_789_005_900 + random.below(181) as i32 - 90;
            let row = Hm {
                from_id: Some(AGENT),
                message: "oracle repeat".to_owned(),
                timestamp: Some(timestamp),
                ..context_row(i as u8 + 1)
            };
            service_times.push(timestamp);
            service_bits.insert(row.msg_id.clone().unwrap_or_default(), 1u32 << i);
            service_rows.push(row);
        }
        for _ in 0..local_count {
            let timestamp = 1_789_005_900 + random.below(181) as i32 - 90;
            let mut row = Hm {
                from_id: Some(AGENT),
                message: "oracle repeat".to_owned(),
                time_source: Some(TimeSource::Local),
                timestamp: Some(timestamp),
                ..repeated_delivery(0)
            };
            local_times.push(timestamp);
            if random.below(2) == 1 {
                row.original_text = Some(row.message.clone());
                row.message = "oracle repeat (translated)".to_owned();
            }
            local_rows.push(row);
        }

        let mut oracle = MatchingOracle {
            service_times: &service_times,
            local_times: &local_times,
            best_count: -1,
            best_distance: 0,
            optimal_retained: BTreeSet::new(),
        };
        oracle.explore(0, 0, 0, 0, 0);

        // The filter sorts its input itself: varying the order also checks
        // that it keeps an optimal identity subset whatever the order.
        random.shuffle(&mut service_rows);
        random.shuffle(&mut local_rows);
        let retained = filter(&service_rows, &local_rows);
        let mask = retained
            .iter()
            .map(|row| service_bits[row.msg_id.as_deref().unwrap_or_default()])
            .fold(0, |a, b| a | b);
        assert_eq!(
            retained.len(),
            service_count - oracle.best_count as usize,
            "oracle maximum matched count at trial {trial}"
        );
        assert!(
            oracle.optimal_retained.contains(&mask),
            "oracle optimal retained identities at trial {trial}"
        );
    }
}

/// `outgoing_repeat(1, …)` with a 32-bit ordinal as its id.
fn long_chain_row(ordinal: u32, timestamp: i32) -> Hm {
    Hm {
        msg_id: Some(ordinal_id(ordinal)),
        ..outgoing_repeat(1, timestamp)
    }
}

#[test]
fn case_49_sparse_ten_thousand_row_history_keeps_the_oldest() {
    let base = 1_789_005_900;
    let count = 10_000u32;
    let earliest = long_chain_row(1, base);
    let mut archive = vec![earliest.clone()];
    let mut live = Vec::new();
    let mut previous_id = earliest.msg_id.clone().unwrap_or_default();
    for ordinal in 2..=count {
        let row = long_chain_row(ordinal, base + (ordinal as i32 - 1) * 59);
        let mut echo = local_echo(&row, 0);
        echo.after_msg_id = Some(if ordinal == 2 { String::new() } else { previous_id.clone() });
        previous_id = row.msg_id.clone().unwrap_or_default();
        archive.push(row);
        live.push(echo);
    }
    live.reverse();

    let mut history = History::default();
    history.set_loaded(archive);
    let initial = history.compose(&[], &live, 200, true);
    assert_eq!(initial.len(), 1, "sparse captures leave one historical occurrence");
    assert_eq!(initial[0], earliest, "the oldest service ID remains historical");

    let unrelated = Hm {
        message: "later unrelated traffic".to_owned(),
        ..long_chain_row(count + 1, base + count as i32 * 59 + 3600)
    };
    history.set_loaded(vec![unrelated.clone()]);
    live.insert(0, live_copy(&unrelated));
    for _ in 0..3 {
        let result = history.compose(&[], &live, 200, true);
        assert_eq!(result.len(), 1, "sparse capture anchors survive archive rollover");
        assert_eq!(result[0], earliest, "explicit publication preserves the oldest service ID");
    }
}

#[test]
fn case_50_unmarked_echoes_reassign_after_the_second_delivery() {
    let timestamp = 1_789_005_900;
    let earlier = outgoing_repeat(1, timestamp);
    let later = outgoing_repeat(2, timestamp + 59);
    let old_echo = local_echo(&later, 0);
    let new_echo = local_echo(&later, 59);
    for visible in [false, true] {
        let mut history = History::default();
        history.set_loaded(vec![earlier.clone(), later.clone()]);
        let initial = history.compose(&[], std::slice::from_ref(&old_echo), 200, visible);
        assert_eq!(initial.len(), 1, "one earlier historical occurrence remains");
        assert!(
            !same(&initial[0], &new_echo),
            "new echo cannot directly match that visible occurrence"
        );
        assert!(
            history.compose(&[], &[new_echo.clone(), old_echo.clone()], 200, visible).is_empty(),
            "both occurrences become live after reassignment"
        );
    }
}

#[test]
fn case_51_late_receipt_exposes_the_distinct_plaintext() {
    let timestamp = 1_789_005_900;
    let first = outgoing_repeat(1, timestamp + 119);
    let second = outgoing_repeat(2, timestamp + 178);
    let legacy = plaintext_copy(&second);
    let old_echo = local_echo(&second, 0);
    let later_echo = local_echo(&second, 59);
    let receipt = Hm {
        time_source: Some(TimeSource::Receipt),
        log_timestamp: Some(timestamp + 237),
        ..local_echo(&first, 0)
    };
    for visible in [false, true] {
        let mut history = History::default();
        history.set_loaded(vec![legacy.clone(), first.clone(), second.clone()]);
        assert!(
            history
                .compose(&[], &[later_echo.clone(), old_echo.clone()], 200, visible)
                .is_empty(),
            "initial live echoes represent the entire source seam"
        );
        let result = history.compose(&[], &[receipt.clone(), later_echo.clone(), old_echo.clone()], 200, visible);
        assert_eq!(result.len(), 1, "receipt reassignment exposes distinct plaintext");
        assert_eq!(result[0], legacy, "the remaining occurrence is the original plaintext");
    }
}

#[test]
fn case_52_anchor_chain_keeps_the_oldest_repeat() {
    let timestamp = 1_789_005_900;
    for count in [2u8, 8] {
        let earlier = outgoing_repeat(1, timestamp);
        let mut archive = vec![earlier.clone()];
        let mut live = Vec::new();
        for ordinal in 1..=count {
            let row = outgoing_repeat(ordinal + 1, timestamp + i32::from(ordinal) * 59);
            live.insert(0, local_echo(&row, 0));
            archive.push(row);
        }
        let mut history = History::default();
        history.set_loaded(archive.clone());
        assert_eq!(
            history.compose(&[], &live, 2, true).len(),
            1,
            "all but the oldest occurrence are live"
        );

        let later = Hm {
            timestamp: Some(timestamp + 3600),
            ..context_row(100)
        };
        history.set_loaded(vec![later.clone()]);
        live.insert(0, live_copy(&later));
        for _ in 0..3 {
            let result = history.compose(&[], &live, 2, true);
            assert_eq!(result.len(), 1, "rollover preserves the oldest repeat through the chain");
            assert_eq!(result[0], earlier, "the original occurrence remains");
        }

        let next = local_echo(archive.last().expect("archive has rows"), 59);
        live.insert(0, next);
        assert!(
            history.compose(&[], &live, 2, true).is_empty(),
            "new echo consumes the final historical occurrence"
        );
    }
}

#[test]
fn case_53_untimed_offers_and_index_only_changes() {
    let mut stored = Hm {
        timestamp: Some(100),
        ..context_row(1)
    };
    let delivery = Hm {
        timestamp: Some(90),
        ..live_copy(&context_row(2))
    };
    let mut offer = Hm::default();
    offer.extra.insert("notification_id".to_owned(), Llsd::Uuid(AGENT));
    let mut current = vec![offer, delivery];
    assert!(
        replace_history(&mut current, std::slice::from_ref(&stored), true),
        "initial interleaved history changes the model"
    );
    assert!(
        current[0].timestamp.is_none(),
        "interleaving does not add a timestamp to an untimed offer"
    );

    // Index-only differences never replay notification widgets or rewrite indices.
    current[0].index = Some(1234);
    stored.index = Some(-9);
    let offer_node: *const Hm = &current[0];
    assert!(
        !replace_history(&mut current, std::slice::from_ref(&stored), true),
        "identical overlay leaves the model in place"
    );
    assert!(std::ptr::eq(&current[0], offer_node), "notification keeps its existing list node");
    assert_eq!(current[0].index, Some(1234), "notification index remains untouched");

    // Editing a live timestamp can move unchanged history relative to live rows.
    let last = current.last_mut().expect("model has rows");
    last.timestamp = Some(110);
    last.extra.insert("notification_log_id".to_owned(), Llsd::Uuid(RESIDENT));
    assert!(
        replace_history(&mut current, std::slice::from_ref(&stored), true),
        "live timestamp edits recheck interleaving"
    );
    assert_eq!(
        current[0].extra.get("notification_id"),
        Some(&Llsd::Uuid(AGENT)),
        "untimed offer remains first"
    );
    assert_eq!(
        current[1].extra.get("notification_log_id"),
        Some(&Llsd::Uuid(RESIDENT)),
        "edited live row precedes older history"
    );
    assert!(current[2].is_history, "historical row now follows the edited live row");

    // Only the top-level presentation index is ignored; nested data and key
    // presence remain meaningful even when a new value is undefined.
    let mut details = Llsd::new_map();
    details.insert("index", 7);
    stored.extra.insert("details".to_owned(), details.clone());
    assert!(
        replace_history(&mut current, std::slice::from_ref(&stored), true),
        "nested history values are compared"
    );
    details.insert("index", 8);
    stored.extra.insert("details".to_owned(), details);
    assert!(
        replace_history(&mut current, std::slice::from_ref(&stored), true),
        "nested index changes are not ignored"
    );
    stored.extra.insert("extra".to_owned(), Llsd::Undef);
    assert!(
        replace_history(&mut current, std::slice::from_ref(&stored), true),
        "an undefined-valued key is still a change"
    );
    stored.extra.remove("extra");
    assert!(
        replace_history(&mut current, std::slice::from_ref(&stored), true),
        "removing an undefined-valued key is a change"
    );
    assert!(
        replace_history(&mut current, &[], true),
        "clearing history keeps live and notification rows"
    );
    assert_eq!(current.len(), 2, "two live rows remain");
    assert!(!replace_history(&mut current, &[], true), "empty overlay is stable");
}

#[test]
fn case_54_service_anchor_covers_a_later_plaintext_load() {
    let timestamp = 1_789_005_900;
    let service = outgoing_repeat(1, timestamp + 125);
    let legacy = plaintext_copy(&service);
    let echo = Hm {
        log_timestamp: Some(timestamp + 296),
        ..local_echo(&service, 3)
    };
    let mut history = History::default();
    history.set_loaded(vec![service]);
    history.compose(&[], &[], 200, true);
    history.set_loaded(vec![legacy]);
    history.compose(&[], &[], 200, true);
    assert!(
        history.compose(&[], std::slice::from_ref(&echo), 200, true).is_empty(),
        "delayed live copy replaces the service representation"
    );
    for _ in 0..3 {
        assert!(
            history.compose(&[], std::slice::from_ref(&echo), 200, true).is_empty(),
            "empty overlay preserves the service-to-plaintext seam"
        );
    }
}

#[test]
fn case_55_randomized_publications_are_idempotent() {
    let mut random = Random(0x59A17F);
    let timestamp = 1_789_005_900;
    let sources = ["local", "receipt", "sent", ""];
    for visible in [false, true] {
        for _ in 0..250 {
            let mut archive = Vec::new();
            let mut legacy = Vec::new();
            let mut service = Vec::new();
            for ordinal in 1..=6u8 {
                let row = Hm {
                    from_id: Some(if ordinal % 2 == 1 { AGENT } else { RESIDENT }),
                    from: if ordinal % 2 == 1 { "Alpha Resident" } else { "Beta Resident" }.to_owned(),
                    message: format!("repeat{}", random.below(3)),
                    ..outgoing_repeat(ordinal, timestamp + i32::from(ordinal) * 30)
                };
                service.push(row.clone());
                archive.push(row.clone());
                if random.below(2) == 1 {
                    let mut logged = plaintext_copy(&row);
                    if random.below(2) == 1 {
                        logged.message.push_str(" translated");
                        logged.legacy_wall_time = Some(f64::from(timestamp + 240) - 7.0 * 3600.0);
                    }
                    legacy.push(logged);
                }
            }
            let archive = merge_direct_history(&legacy, &archive);
            let mut history = History::default();
            history.set_loaded(archive);
            let mut live: Messages = Vec::new();
            let mut model = Vec::new();
            let composed = history.compose(&[], &live, 5, visible);
            replace_history(&mut model, &composed, true);

            for step in 0..12 {
                let picked = random.below(service.len() as u32) as usize;
                let skew = random.below(151) as i32 - 75;
                let mut arrival = local_echo(&service[picked], skew);
                arrival.time_source = source(sources[random.below(4) as usize]);
                arrival.log_timestamp = Some(timestamp + 240 + step);
                if random.below(4) == 0 {
                    arrival.message.push_str(" translated");
                }
                if random.below(5) == 0 {
                    arrival.from_id = None;
                }
                if random.below(10) == 0 {
                    arrival.from.clear();
                }
                if random.below(4) == 0 {
                    arrival.timestamp = Some(timestamp + 600 + step);
                    arrival.message = "unrelated".to_owned();
                    arrival.original_text = Some("unrelated".to_owned());
                }

                // An identical publication before the append must be stable;
                // the new row is then the only possible cause of a change.
                let mut repeated = history.clone();
                let mut before = model.clone();
                let composed = repeated.compose(&[], &live, 5, visible);
                assert!(!replace_history(&mut before, &composed, true), "fixed-source publication is stable");
                live.insert(0, arrival.clone());
                model.insert(0, arrival);
                let composed = history.compose(&[], &live, 5, visible);
                replace_history(&mut model, &composed, true);
            }
        }
    }
}

#[test]
fn case_56_shifted_chain_of_ten_thousand_keeps_one_oldest() {
    let mut history = Vec::new();
    let mut live = Vec::new();
    for index in 0..=10_000u32 {
        let row = long_chain_row(index + 1, 1_789_005_900 + index as i32 * 10);
        if index > 0 {
            live.push(local_echo(&row, 1));
        }
        history.push(row);
    }
    live.reverse();
    let retained = filter(&history, &live);
    assert_eq!(retained.len(), 1, "a shifted anchor chain keeps one older repeat");
    assert_eq!(retained[0].msg_id.as_deref(), Some(FIRST), "the oldest repeat retains its identity");
}

#[test]
fn case_57_dense_unequal_groups_use_the_full_dp() {
    let mut history = Vec::new();
    let mut live = Vec::new();
    for index in 0..10_000u32 {
        let row = outgoing_repeat(1, 1_789_005_900);
        live.push(local_echo(&row, 1));
        if index < 200 {
            history.push(Hm {
                msg_id: Some(ordinal_id(index + 1)),
                ..row
            });
        }
    }
    assert!(filter(&history, &live).is_empty(), "dense skewed rows all reconcile");
}

#[test]
fn case_58_unrelated_component_keeps_the_tied_choice() {
    let mut history = vec![
        outgoing_repeat(1, 1_789_005_900),
        outgoing_repeat(2, 1_789_005_900),
        outgoing_repeat(3, 1_789_005_900),
    ];
    let mut live = vec![local_echo(&outgoing_repeat(20, 1_789_005_920), 0), local_echo(&history[0], 0)];
    let original = filter(&history, &live);
    assert_eq!(original.len(), 1, "the isolated component retains one occurrence");
    history.insert(0, outgoing_repeat(4, 1_789_005_980));
    history.insert(0, outgoing_repeat(5, 1_789_006_020));
    for timestamp in [1_789_005_980, 1_789_005_980, 1_789_006_019, 1_789_006_040] {
        live.insert(0, local_echo(&outgoing_repeat(20, timestamp), 0));
    }
    let extended = filter(&history, &live);
    assert_eq!(extended.len(), 1, "the disconnected component adds no retained service");
    assert_eq!(
        extended[0].msg_id, original[0].msg_id,
        "unrelated traffic cannot change the tied identity"
    );
}

#[test]
fn case_59_disconnected_component_keeps_the_protected_plaintext() {
    let base = 1_789_005_900;
    let service = outgoing_repeat(1, base + 120);
    let legacy = plaintext_copy(&service);
    let mut history = vec![legacy.clone(), service.clone()];
    let mut live = vec![local_echo(&service, 20), local_echo(&service, -20)];
    let isolated = filter(&history, &live);
    assert_eq!(isolated.len(), 1, "isolated extra same-minute plaintext remains");
    assert!(isolated[0].msg_id.is_none(), "the remaining occurrence is plaintext");

    let prefix = outgoing_repeat(2, base + 1100);
    history.push(prefix.clone());
    history.push(outgoing_repeat(3, base + 1100));
    live.insert(0, local_echo(&prefix, 0));
    let extended = filter(&history, &live);
    assert_eq!(extended.len(), 2, "only the independent extra service row is added");
    assert!(
        extended.contains(&legacy),
        "unrelated history leaves the protected plaintext intact"
    );
}

#[test]
fn case_60_one_live_row_covers_its_service_row_and_next_minute_log() {
    let base = 1_789_005_900;
    for translated in [false, true] {
        let service = outgoing_repeat(1, base + 59);
        let mut echo = local_echo(&service, 0);
        echo.log_timestamp = Some(base + 60);
        if translated {
            echo.message = "translated castle".to_owned();
        }
        let logged = Hm {
            timestamp: Some(base + 60),
            ..echo.clone()
        };
        let legacy = plaintext_copy(&logged);
        for remaining in [1usize, 3] {
            let mut loaded = vec![legacy.clone(); remaining + 1];
            loaded[0].time = "12:01".to_owned();
            loaded.push(service.clone());
            let mut history = History::default();
            history.set_loaded(loaded);
            let initial = history.compose(&[], std::slice::from_ref(&echo), 200, true);
            assert_eq!(initial.len(), remaining, "one live echo consumes one plaintext occurrence");
            let mut model = vec![echo.clone()];
            replace_history(&mut model, &initial, true);

            history.set_loaded(vec![service.clone()]);
            for _ in 0..3 {
                let result = history.compose(&[], std::slice::from_ref(&echo), 200, true);
                assert_eq!(result.len(), remaining, "reload preserves distinct plaintext repeats");
                assert!(result.iter().all(|row| *row == legacy), "retained plaintext stays unchanged");
                assert!(
                    !replace_history(&mut model, &result, true),
                    "unchanged reload does not replay the model"
                );
            }

            let later = Hm {
                timestamp: Some(base + 600),
                ..context_row(3)
            };
            let later_live = live_copy(&later);
            history.set_loaded(vec![later]);
            assert_eq!(
                history.compose(&[], &[later_live.clone(), echo.clone()], 200, true).len(),
                remaining,
                "complete archive rollover preserves plaintext counts"
            );
            let next = Hm {
                timestamp: Some(base + 61),
                log_timestamp: Some(base + 61),
                ..echo.clone()
            };
            assert_eq!(
                history.compose(&[], &[next, later_live, echo.clone()], 200, true).len(),
                remaining - 1,
                "a new live repeat consumes one more occurrence"
            );
        }
    }
}

#[test]
fn case_61_slt_offset_chains_keep_their_anchors() {
    let base = 1_789_005_900;
    let mut first_live = Hm {
        time_source: Some(TimeSource::Sent),
        ..local_echo(&outgoing_repeat(1, base + 8 * 3600), 0)
    };
    let second_live = local_echo(&outgoing_repeat(2, base + 9 * 3600), 0);
    let first = Hm {
        legacy_wall_time: Some(f64::from(base)),
        ..plaintext_copy(&first_live)
    };
    let second = Hm {
        legacy_wall_time: Some(f64::from(base + 3600)),
        ..first.clone()
    };
    let third = Hm {
        legacy_wall_time: Some(f64::from(base + 2 * 3600)),
        ..first.clone()
    };
    let mut history = History::default();
    history.set_loaded(vec![first.clone(), second.clone(), third.clone()]);
    assert_eq!(
        history.compose(&[], &[second_live.clone(), first_live.clone()], 200, true).len(),
        1,
        "two live messages leave one plaintext occurrence"
    );

    let later = Hm {
        timestamp: Some(base + 10 * 3600),
        ..context_row(4)
    };
    history.set_loaded(vec![later.clone()]);
    for _ in 0..3 {
        let result = history.compose(&[], &[live_copy(&later), second_live.clone(), first_live.clone()], 200, true);
        assert_eq!(result.len(), 1, "plaintext matching chains survive rollover");
        assert_eq!(result[0], third, "the third occurrence is still historical");
    }

    history.clear();
    first_live.time_source = Some(TimeSource::Receipt);
    let service = outgoing_repeat(5, base + 8 * 3600);
    let service_live = Hm {
        time_source: Some(TimeSource::Sent),
        ..local_echo(&service, 0)
    };
    history.set_loaded(vec![first, second, third.clone(), service]);
    assert_eq!(
        history
            .compose(&[], &[second_live.clone(), first_live.clone(), service_live.clone()], 200, true)
            .len(),
        1,
        "service matching preserves the extra plaintext counts"
    );
    history.set_loaded(vec![later.clone()]);
    for _ in 0..3 {
        let result = history.compose(
            &[],
            &[live_copy(&later), second_live.clone(), first_live.clone(), service_live.clone()],
            200,
            true,
        );
        assert_eq!(result.len(), 1, "service anchors still protect retained plaintext anchors");
        assert_eq!(result[0], third, "the protected occurrence remains unchanged");
    }
}

#[test]
fn case_62_receipts_revisit_rows_skipped_by_exact_sends() {
    let base = 1_789_005_900;
    let first = outgoing_repeat(1, base);
    let second = outgoing_repeat(2, base + 30);
    let third = outgoing_repeat(3, base + 60);
    for name in ["sent", ""] {
        let sent = Hm {
            time_source: source(name),
            ..local_echo(&first, 0)
        };
        let receipt = Hm {
            time_source: Some(TimeSource::Receipt),
            message: "translated castle".to_owned(),
            ..local_echo(&second, -5)
        };
        for reversed in [false, true] {
            let mut history = vec![first.clone(), second.clone(), third.clone()];
            let mut live = vec![local_echo(&third, 0), receipt.clone(), sent.clone()];
            if reversed {
                history.reverse();
                live.reverse();
            }
            assert!(
                filter(&history, &live).is_empty(),
                "receipts revisit rows skipped by exact sends before local matching"
            );
        }
    }
}

#[test]
fn case_63_receipt_window_is_the_same_with_and_without_ids() {
    let base = 1_789_005_900;
    let service = outgoing_repeat(1, base);
    for named_only in [false, true] {
        for delay in [-6, -5, 0, 60, 61] {
            let mut receipt = Hm {
                time_source: Some(TimeSource::Receipt),
                message: "translated castle".to_owned(),
                ..local_echo(&service, delay)
            };
            if named_only {
                receipt.from_id = None;
            }
            assert_eq!(
                filter(std::slice::from_ref(&service), &[receipt]).is_empty(),
                (-5..=60).contains(&delay),
                "receipt window includes only the permitted clock difference"
            );
        }
    }

    let unrelated = Hm {
        message: "another place".to_owned(),
        ..outgoing_repeat(2, base + 120)
    };
    let newer = Hm {
        time_source: Some(TimeSource::Receipt),
        ..local_echo(&service, 61)
    };
    let older = Hm {
        time_source: Some(TimeSource::Receipt),
        ..local_echo(&service, 60)
    };
    let result = filter(&[service, unrelated.clone()], &[newer, older]);
    assert_eq!(result.len(), 1, "the older receipt still consumes its occurrence");
    assert_eq!(result[0], unrelated, "unrelated history retains its identity");
}

#[test]
fn case_64_name_fallback_and_uuid_lookup_share_reservations() {
    let base = 1_789_005_900;
    for missing_service_sender in [false, true] {
        let mut history: Messages = [1u8, 2, 3, 4].iter().map(|&ordinal| outgoing_repeat(ordinal, base)).collect();
        let named = Hm {
            from_id: None,
            time_source: Some(TimeSource::Sent),
            ..local_echo(&history[0], 0)
        };
        let receipt = Hm {
            time_source: Some(TimeSource::Receipt),
            message: "translated castle".to_owned(),
            ..local_echo(&history[0], 0)
        };
        let local = local_echo(&history[0], 0);
        if missing_service_sender {
            history[0].from_id = None;
        }
        let result = filter(&history, &[named, receipt, local]);
        assert_eq!(result.len(), 1, "three deliveries consume three distinct occurrences");
        assert_eq!(result[0], history[3], "the fourth tied service occurrence remains");
    }
}

#[test]
fn case_65_chain_across_rolling_windows() {
    let base = 1_789_005_900;
    let earliest = long_chain_row(1, base);
    let mut history = History::default();
    let mut archive = vec![earliest.clone()];
    let mut live = Vec::new();
    for index in 1..=1000u32 {
        let row = long_chain_row(index + 1, base + index as i32 * 59);
        live.insert(0, local_echo(&row, 0));
        archive.push(row);
        if archive.len() > 200 {
            archive.drain(..archive.len() - 200);
        }
        if index % 199 == 0 || index == 1000 {
            history.set_loaded(archive.clone());
            let result = history.compose(&[], &live, 200, true);
            assert_eq!(result.len(), 1, "rolling windows preserve the old repeat");
            assert_eq!(result[0], earliest, "the old repeat retains its identity");
        }
    }

    // Receipts inside the retained time range but with another body.
    let mut receipts = Vec::new();
    for index in 0..5000u32 {
        let row = Hm {
            message: "unrelated received traffic".to_owned(),
            ..long_chain_row(index + 1002, base + index as i32 * 10)
        };
        receipts.push(Hm {
            time_source: Some(TimeSource::Receipt),
            ..local_echo(&row, 0)
        });
    }
    receipts.reverse();
    receipts.append(&mut live);
    let mut live = receipts;
    for _ in 0..2 {
        let result = history.compose(&[], &live, 200, true);
        assert_eq!(result.len(), 1, "received traffic preserves the distinct historical repeat");
        assert_eq!(result[0], earliest, "refresh preserves the original occurrence");
    }

    let next = local_echo(archive.last().expect("archive has rows"), 59);
    live.insert(0, next);
    assert!(
        history.compose(&[], &live, 200, true).is_empty(),
        "the additional echo consumes the final historical repeat"
    );
}

#[test]
fn case_66_randomized_grouped_matching_equals_a_full_scan() {
    let mut random = Random(0x621a557);
    let base = 1_789_005_900;
    let sources = ["sent", "receipt", ""];
    for _ in 0..2000 {
        let mut service = Vec::new();
        for ordinal in 1..=8u8 {
            let mut row = Hm {
                from_id: Some(if ordinal % 2 == 1 { AGENT } else { RESIDENT }),
                from: if ordinal % 2 == 1 { "Alpha Resident" } else { "Beta Resident" }.to_owned(),
                ..outgoing_repeat(ordinal, base + random.below(12) as i32 * 10)
            };
            row.message = format!("repeat {}", random.below(3));
            if random.below(7) == 0 {
                row.from_id = None;
            }
            service.push(row);
        }
        let mut deliveries = Vec::new();
        for _ in 0..12 {
            let picked = random.below(service.len() as u32) as usize;
            let skew = random.below(81) as i32 - 10;
            let mut delivery = local_echo(&service[picked], skew);
            delivery.time_source = source(sources[random.below(3) as usize]);
            if random.below(4) == 0 {
                delivery.from_id = None;
            }
            if random.below(4) == 0 {
                delivery.message = "translated text".to_owned();
            }
            if random.below(20) == 0 {
                delivery.timestamp = Some(0);
            }
            deliveries.push(delivery);
        }
        let live = deliveries.clone();

        // Exact timestamps first, then the newest receipts; stable order
        // decides which id survives a timestamp tie.
        let mut order: Vec<usize> = (0..service.len()).collect();
        order.sort_by_key(|&row| std::cmp::Reverse(service[row].timestamp.unwrap_or(0)));
        let receipt = |m: &Hm| m.time_source == Some(TimeSource::Receipt);
        deliveries.sort_by(|l, r| {
            receipt(l)
                .cmp(&receipt(r))
                .then(r.timestamp.unwrap_or(0).cmp(&l.timestamp.unwrap_or(0)))
        });
        let mut consumed = vec![false; service.len()];
        for delivery in &deliveries {
            if let Some(&row) = order.iter().find(|&&row| !consumed[row] && same(&service[row], delivery)) {
                consumed[row] = true;
            }
        }
        let expected: Messages = service
            .iter()
            .zip(&consumed)
            .filter(|(_, c)| !**c)
            .map(|(row, _)| row.clone())
            .collect();
        assert_eq!(
            filter(&service, &live),
            expected,
            "grouped matching preserves reference identities and order"
        );
    }
}

#[test]
fn case_67_minute_boundary_send_has_no_duplicate() {
    let boundary = seconds("2026-10-30T17:29:00Z");
    let castle = Hm {
        message: "one of my fav places if ya'll need ideas for next time, is the linden bear castle".to_owned(),
        ..outgoing_repeat(1, boundary - 1)
    };
    let bears = Hm {
        message: "which has 1000+ linden bears from over the years".to_owned(),
        ..outgoing_repeat(2, boundary + 8)
    };
    let reply = Hm {
        timestamp: Some(boundary + 14),
        message: "Oh, that is a fun spot".to_owned(),
        ..context_row(3)
    };
    let archive = vec![castle.clone(), bears.clone(), reply.clone()];

    for loaded_first in [false, true] {
        let mut history = History::default();
        let castle_echo = captured_outgoing(&castle, &history, 1);
        let bears_echo = captured_outgoing(&bears, &history, 1);
        let received = Hm {
            time_source: Some(TimeSource::Receipt),
            ..local_echo(&reply, 0)
        };
        let live = vec![received, bears_echo, castle_echo];
        let mut current = live.clone();
        if loaded_first {
            history.set_loaded(archive.clone());
        }
        for _ in 0..3 {
            let composed = history.compose(&archive, &live, 200, true);
            assert!(composed.is_empty(), "the minute-boundary send has no historical duplicate");
            assert!(
                !replace_history(&mut current, &composed, true),
                "unchanged screenshot messages are not replayed"
            );
            assert_eq!(current.len(), 3, "all three screenshot messages remain");
            assert_eq!(current[0].message, reply.message, "the reply remains newest");
            assert_eq!(current[1].message, bears.message, "the bear description keeps its position");
            assert_eq!(current[2].message, castle.message, "the castle suggestion appears once");
            history.set_loaded(archive.clone());
        }
        assert!(
            history.compose(&[], &live, 200, true).is_empty(),
            "expiring the preview does not restore the duplicate"
        );
    }
}

#[test]
fn case_68_fresh_identical_send_keeps_the_old_id() {
    let boundary = seconds("2026-10-30T17:29:00Z");
    let old = Hm {
        message: "one of my fav places if ya'll need ideas for next time, is the linden bear castle".to_owned(),
        ..outgoing_repeat(1, boundary - 1)
    };
    for visible in [false, true] {
        let mut history = History::default();
        history.set_loaded(vec![old.clone()]);
        let mut current = Vec::new();
        let composed = history.compose(&[], &[], 200, visible);
        replace_history(&mut current, &composed, true);
        let fresh = captured_outgoing(&old, &history, 2);
        current.insert(0, fresh.clone());

        for _ in 0..3 {
            let composed = history.compose(&[], std::slice::from_ref(&fresh), 200, visible);
            assert_eq!(composed.len(), 1, "a fresh repeat keeps the old occurrence");
            assert_eq!(composed[0].msg_id, old.msg_id, "the old service ID survives");
            assert!(
                !replace_history(&mut current, &composed, true),
                "stale history does not rewrite the transcript"
            );
            assert_eq!(current.len(), 2, "both real messages remain displayed");
        }

        let own = Hm {
            message: old.message.clone(),
            ..outgoing_repeat(2, boundary + 1)
        };
        history.set_loaded(vec![old.clone(), own.clone()]);
        let composed = history.compose(&[own], std::slice::from_ref(&fresh), 200, visible);
        assert_eq!(composed.len(), 1, "the fresh service copy is consumed once");
        assert_eq!(composed[0].msg_id, old.msg_id, "new history still preserves the old ID");
        assert!(
            !replace_history(&mut current, &composed, true),
            "the fresh service copy does not replay either message"
        );
    }
}

#[test]
fn case_69_empty_boundary_is_captured_evidence() {
    let base = 1_789_005_900;
    let before = context_row(1);
    let service = outgoing_repeat(2, base + 59);
    for has_before in [false, true] {
        let mut history = History::default();
        if has_before {
            history.set_loaded(vec![before.clone()]);
        }
        history.compose(&[], &[], 200, true);
        let boundary = format_time_uuid(&history.latest_service_key());
        let context = LiveContext {
            after_msg_id: Some(boundary.clone()),
            ..LiveContext::default()
        };
        let mut timestamp = 0;
        let context = capture_live_message(&service.message, &mut timestamp, &context, (base + 60) as u32);

        // The service row becomes visible while the original echo is translating.
        let mut archive = vec![service.clone()];
        if has_before {
            archive.insert(0, before.clone());
        }
        history.set_loaded(archive);
        history.compose(&[], &[], 200, true);
        let context = capture_live_message("translated castle", &mut timestamp, &context, (base + 180) as u32);
        assert_eq!(
            context.after_msg_id.as_deref(),
            Some(boundary.as_str()),
            "a second capture preserves the original boundary"
        );
        assert_eq!(timestamp, (base + 60) as u32, "translation preserves the original send clock");

        let mut echo = Hm {
            message: "translated castle".to_owned(),
            timestamp: Some(timestamp as i32),
            ..live_copy(&service)
        };
        record_live_message(&mut echo, &context, (base + 180) as u32);
        assert_eq!(
            echo.after_msg_id.as_deref(),
            Some(boundary.as_str()),
            "recording preserves the boundary value"
        );
        for _ in 0..3 {
            let composed = history.compose(&[], std::slice::from_ref(&echo), 200, true);
            assert_eq!(composed.len(), usize::from(has_before), "only unrelated earlier history remains");
            if has_before {
                assert_eq!(composed[0], before, "the earlier row keeps its identity");
            }
        }
    }
}

#[test]
fn case_70_delayed_precaptured_echo_may_complete_the_chain() {
    let base = 1_789_005_900;
    let first = outgoing_repeat(1, base);
    let second = outgoing_repeat(2, base + 59);
    let third = outgoing_repeat(3, base + 118);
    let mut history = History::default();
    let old_live = vec![captured_outgoing(&third, &history, 0), captured_outgoing(&second, &history, 0)];
    let delayed = captured_outgoing(&third, &history, 59);
    history.set_loaded(vec![first.clone(), second, third.clone()]);
    assert_eq!(
        history.compose(&[], &old_live, 200, true).len(),
        1,
        "the old echoes leave one historical repeat"
    );

    let mut delayed_history = history.clone();
    let mut delayed_live = old_live.clone();
    delayed_live.insert(0, delayed);
    assert!(
        delayed_history.compose(&[], &delayed_live, 200, true).is_empty(),
        "a delayed echo can complete the old occurrence chain"
    );
    let mut fresh_live = old_live;
    fresh_live.insert(0, captured_outgoing(&third, &history, 59));
    let result = history.compose(&[], &fresh_live, 200, true);
    assert_eq!(result.len(), 1, "a fresh echo cannot consume the old chain");
    assert_eq!(result[0], first, "the first occurrence keeps its identity");

    let unrelated = Hm {
        timestamp: Some(base + 3600),
        ..context_row(4)
    };
    history.set_loaded(vec![unrelated.clone()]);
    fresh_live.insert(0, live_copy(&unrelated));
    for _ in 0..3 {
        let result = history.compose(&[], &fresh_live, 200, true);
        assert_eq!(result.len(), 1, "rollover retains the distinct old occurrence");
        assert_eq!(result[0], first, "rollover retains the original ID");
    }
}

#[test]
fn case_71_two_sends_in_one_second_with_reversed_callbacks() {
    let base = 1_789_005_900;
    let first = outgoing_repeat(1, base);
    let second = outgoing_repeat(2, base);
    let mut history = History::default();
    let first_echo = captured_outgoing(&first, &history, 0);
    history.set_loaded(vec![first.clone()]);
    history.compose(&[], &[], 200, true);
    let second_echo = captured_outgoing(&second, &history, 0);
    history.set_loaded(vec![first.clone(), second]);
    let waiting = history.compose(&[], std::slice::from_ref(&second_echo), 200, true);
    assert_eq!(waiting.len(), 1, "the later callback consumes only its own row");
    assert_eq!(waiting[0], first, "the earlier callback's row remains");
    let live = vec![first_echo, second_echo];
    assert!(
        history.compose(&[], &live, 200, true).is_empty(),
        "both same-second sends reconcile after reversed callbacks"
    );
    assert!(
        history.compose(&[], &live, 200, true).is_empty(),
        "the completed publication is stable"
    );
}

#[test]
fn case_72_causal_bounds_allow_crossed_clock_assignment() {
    let base = 1_789_005_900;
    let first = outgoing_repeat(1, base + 50);
    let second = outgoing_repeat(2, base + 100);
    let mut history = History::default();
    let earlier_echo = captured_outgoing(&second, &history, 0);
    history.set_loaded(vec![first.clone()]);
    history.compose(&[], &[], 200, true);
    let later_echo = captured_outgoing(&second, &history, -10);
    for history_reversed in [false, true] {
        for live_reversed in [false, true] {
            let mut archive = vec![first.clone(), second.clone()];
            let mut live = vec![earlier_echo.clone(), later_echo.clone()];
            if history_reversed {
                archive.reverse();
            }
            if live_reversed {
                live.reverse();
            }
            assert!(filter(&archive, &live).is_empty(), "causal bounds allow crossed clock assignments");
        }
    }
}

#[test]
fn case_73_fresh_send_reserves_its_only_eligible_row() {
    let base = 1_789_005_900;
    let first = outgoing_repeat(1, base);
    let second = outgoing_repeat(2, base + 1);
    let third = outgoing_repeat(3, base + 5);
    let mut history = History::default();
    let older_echo = captured_outgoing(&second, &history, 0);
    history.set_loaded(vec![first.clone(), second.clone()]);
    history.compose(&[], &[], 200, true);
    let fresh_echo = captured_outgoing(&second, &history, 1);
    for reversed in [false, true] {
        let mut archive = vec![first.clone(), second.clone(), third.clone()];
        let mut live = vec![fresh_echo.clone(), older_echo.clone()];
        if reversed {
            archive.reverse();
            live.reverse();
        }
        let result = filter(&archive, &live);
        assert_eq!(result.len(), 1, "both eligible echoes consume an occurrence");
        assert_eq!(result[0], first, "the closest legal complete pairing retains ID 1");
    }
}

/// Every retained set of a legal assignment that no unused echo could still
/// extend directly (case 74's oracle).
struct CausalOracle<'a> {
    eligible: &'a dyn Fn(usize, usize) -> bool,
    service_count: usize,
    live_count: usize,
    retained_sets: BTreeSet<u32>,
}

impl CausalOracle<'_> {
    fn explore(&mut self, row: usize, used: u32, retained: u32) {
        if row == self.service_count {
            for remaining in (0..self.service_count).filter(|r| retained & (1 << r) != 0) {
                if (0..self.live_count).any(|echo| used & (1 << echo) == 0 && (self.eligible)(remaining, echo)) {
                    return;
                }
            }
            self.retained_sets.insert(retained);
            return;
        }
        self.explore(row + 1, used, retained | (1 << row));
        for echo in 0..self.live_count {
            if used & (1 << echo) == 0 && (self.eligible)(row, echo) {
                self.explore(row + 1, used | (1 << echo), retained);
            }
        }
    }
}

#[test]
fn case_74_randomized_causal_oracle() {
    let mut random = Random(0xc4a57e11);
    let base = 1_789_005_900;
    for trial in 0..2000 {
        let service_count = random.below(7) as usize;
        let live_count = random.below(7) as usize;
        let mut ranks = [1u8, 2, 3, 4, 5, 6];
        random.shuffle(&mut ranks);
        let mut service_times = Vec::new();
        let mut live_times = Vec::new();
        let mut floors = Vec::new();
        let mut service_rows = Vec::new();
        let mut live_rows = Vec::new();
        let mut bits = BTreeMap::new();

        for (index, &rank) in ranks.iter().enumerate().take(service_count) {
            let timestamp = base + random.below(181) as i32 - 90;
            let row = outgoing_repeat(rank, timestamp);
            service_times.push(timestamp);
            bits.insert(row.msg_id.clone().unwrap_or_default(), 1u32 << index);
            service_rows.push(row);
        }
        for _ in 0..live_count {
            let timestamp = base + random.below(181) as i32 - 90;
            let floor = random.below(8) as u8;
            let mut row = local_echo(&outgoing_repeat(20, timestamp), 0);
            row.after_msg_id = Some(if floor > 0 {
                context_row(floor).msg_id.unwrap_or_default()
            } else {
                String::new()
            });
            if random.below(2) == 1 {
                row.message = "translated castle".to_owned();
            }
            live_times.push(timestamp);
            floors.push(floor);
            live_rows.push(row);
        }

        let eligible = |row: usize, echo: usize| {
            ranks[row] > floors[echo] && (i64::from(service_times[row]) - i64::from(live_times[echo])).abs() <= 59
        };
        let mut oracle = CausalOracle {
            eligible: &eligible,
            service_count,
            live_count,
            retained_sets: BTreeSet::new(),
        };
        oracle.explore(0, 0, 0);

        random.shuffle(&mut service_rows);
        random.shuffle(&mut live_rows);
        let actual = filter(&service_rows, &live_rows);
        let retained = actual
            .iter()
            .map(|row| bits[row.msg_id.as_deref().unwrap_or_default()])
            .fold(0, |a, b| a | b);
        assert!(
            oracle.retained_sets.contains(&retained),
            "causal oracle legal and maximal matching at trial {trial}"
        );
        let mut previous = 0;
        for row in &actual {
            let found = service_rows[previous..].iter().position(|source| source == row);
            assert!(found.is_some(), "retained rows preserve source order at trial {trial}");
            previous += found.unwrap_or(0) + 1;
        }
    }
}

#[test]
fn case_76_boundary_applies_to_local_only_in_timeuuid_order() {
    let first = outgoing_repeat(1, 1_789_005_900);
    let second = outgoing_repeat(2, 1_789_005_900);
    for named_only in [false, true] {
        let mut echo = Hm {
            after_msg_id: first.msg_id.clone(),
            ..local_echo(&first, 0)
        };
        if named_only {
            echo.from_id = None;
        }
        assert!(!same(&first, &echo), "a local echo cannot consume its captured boundary");
        assert!(same(&second, &echo), "a later ID remains eligible");
        let local = filter(&[first.clone(), second.clone()], std::slice::from_ref(&echo));
        assert_eq!(local.len(), 1, "the boundary reserves the earlier occurrence");
        assert_eq!(local[0], first, "UUID and name lookup preserve the same old ID");
        for name in ["sent", "receipt", ""] {
            echo.time_source = source(name);
            assert!(same(&first, &echo), "nonlocal provenance ignores an outgoing boundary");
            assert!(
                filter(std::slice::from_ref(&first), std::slice::from_ref(&echo)).is_empty(),
                "nonlocal matching still consumes its eligible copy"
            );
        }
    }

    for ids in [
        ["ffffffff-0000-1000-8000-000000000000", "00000000-0001-1000-8000-000000000000"],
        ["00000001-0000-1000-8080-000000000000", "00000001-0000-1000-8000-000000000000"],
    ] {
        let older = Hm {
            msg_id: Some(ids[0].to_owned()),
            ..first.clone()
        };
        let newer = Hm {
            msg_id: Some(ids[1].to_owned()),
            ..second.clone()
        };
        let echo = Hm {
            after_msg_id: Some(ids[0].to_owned()),
            ..local_echo(&older, 0)
        };
        assert!(same(&newer, &echo), "a later TimeUUID remains eligible despite text order");
        let result = filter(&[older.clone(), newer], &[echo]);
        assert_eq!(result.len(), 1, "a TimeUUID boundary consumes one later occurrence");
        assert_eq!(result[0], older, "the earlier TimeUUID stays historical");
    }
}

#[test]
fn case_77_latest_service_key_covers_every_source() {
    let second = outgoing_repeat(2, 1_789_005_960);
    let third = outgoing_repeat(3, 1_789_005_930);
    let latest = |history: &History| format_time_uuid(&history.latest_service_key());
    let mut history = History::default();
    assert_eq!(latest(&history), "", "empty history has no service boundary");
    history.set_loaded(vec![second.clone()]);
    assert_eq!(
        Some(latest(&history)),
        second.msg_id,
        "loaded history contributes before publication"
    );
    let echo = local_echo(&third, 0);
    let initial = history.compose(std::slice::from_ref(&third), std::slice::from_ref(&echo), 200, true);
    assert_eq!(initial.len(), 1, "the newest ID is already consumed");
    assert_eq!(
        Some(latest(&history)),
        third.msg_id,
        "consumed preview candidates contribute their ID"
    );

    history.set_loaded(vec![plaintext_copy(&second)]);
    history.compose(&[], std::slice::from_ref(&echo), 200, true);
    assert_eq!(
        Some(latest(&history)),
        third.msg_id,
        "retained anchors contribute after source rollover"
    );
    history.clear_service();
    assert_eq!(latest(&history), "", "service revocation releases the boundary");
    history.set_loaded(vec![second]);
    history.clear();
    assert_eq!(latest(&history), "", "deletion releases the boundary");
}

#[test]
fn case_78_ineligible_fresh_sends_keep_the_older_tie() {
    let base = 1_789_005_900;
    let first = outgoing_repeat(1, base + 59);
    let second = outgoing_repeat(2, base + 61);
    let mut history = History::default();
    let mut live = vec![captured_outgoing(&first, &history, 1)];
    history.set_loaded(vec![first.clone(), second]);
    let initial = history.compose(&[], &live, 200, true);
    assert_eq!(initial.len(), 1, "one tied historical occurrence remains");
    assert_eq!(initial[0], first, "the initial pairing retains the first ID");
    let mut current = live.clone();
    replace_history(&mut current, &initial, true);

    for _ in 0..2 {
        let fresh = captured_outgoing(&first, &history, 1);
        live.insert(0, fresh.clone());
        current.insert(0, fresh);
        let result = history.compose(&[], &live, 200, true);
        assert_eq!(result.len(), 1, "ineligible fresh sends retain one old occurrence");
        assert_eq!(result[0], first, "ineligible fresh sends preserve its original ID");
        assert!(
            !replace_history(&mut current, &result, true),
            "the history publication leaves the model unchanged"
        );
    }
}

#[test]
fn case_79_same_second_echoes_each_take_their_copies() {
    let base = 1_789_005_900;
    let first = outgoing_repeat(1, base + 59);
    let second = outgoing_repeat(2, base + 59);
    let mut history = History::default();
    let first_echo = captured_outgoing(&first, &history, 1);
    history.set_loaded(vec![first.clone()]);
    history.compose(&[], &[], 200, true);
    let second_echo = Hm {
        log_timestamp: Some(base + 120),
        ..captured_outgoing(&second, &history, 1)
    };
    let first_log = plaintext_copy(&first_echo);
    let logged_second = Hm {
        timestamp: Some(base + 120),
        ..second_echo.clone()
    };
    let second_log = plaintext_copy(&logged_second);

    let archive = vec![first_log.clone(), first_log.clone(), second_log, first, second];
    for reversed in [false, true] {
        let mut live = vec![first_echo.clone(), second_echo.clone()];
        if reversed {
            live.reverse();
        }
        let result = filter(&archive, &live);
        assert_eq!(result.len(), 1, "each echo consumes exactly one logged occurrence");
        assert_eq!(result[0], first_log, "the extra plaintext occurrence remains");
    }
}

#[test]
fn case_80_independent_later_component_keeps_an_older_tie() {
    let base = 1_789_005_900;
    let first = outgoing_repeat(1, base + 59);
    let second = outgoing_repeat(2, base + 61);
    let later = outgoing_repeat(3, base + 178);
    let mut history = History::default();
    let mut live = vec![captured_outgoing(&first, &history, 1)];
    history.set_loaded(vec![first.clone(), second.clone()]);
    let initial = history.compose(&[], &live, 200, true);
    assert_eq!(initial[0], first, "the earlier tied occurrence starts historical");

    for _ in 0..3 {
        live.insert(0, captured_outgoing(&first, &history, 60));
    }
    history.set_loaded(vec![first.clone(), second, later]);
    let result = history.compose(&[], &live, 200, true);
    assert_eq!(result.len(), 1, "the later repeats replace only their own service occurrence");
    assert_eq!(result[0], first, "the unrelated occurrence keeps its original service ID");
}

#[test]
fn case_81_randomized_separate_component_keeps_counts_and_ids() {
    let mut random = Random(0x51ab1e);
    let base = 1_789_005_900;
    for trial in 0..250 {
        let mut archive = Vec::new();
        for id in 1..=3u8 {
            let offset = if trial > 0 {
                8 + random.below(3) as i32
            } else if id == 1 {
                8
            } else {
                9
            };
            archive.push(outgoing_repeat(id, base + offset));
        }
        let mut live = Vec::new();
        let first = if trial > 0 { random.below(3) as i32 } else { 0 };
        live.push(local_echo(&outgoing_repeat(1, base + first), 0));
        let second = if trial > 0 { random.below(3) as i32 } else { 2 };
        live.push(local_echo(&outgoing_repeat(2, base + second), 0));
        let before = filter(&archive, &live);

        // Old captures cannot reach time 126; new captures exclude ids 1-3.
        archive.push(outgoing_repeat(4, base + 126));
        for _ in 0..3 {
            let echo = Hm {
                after_msg_id: context_row(3).msg_id,
                ..local_echo(&outgoing_repeat(4, base + 67), 0)
            };
            live.insert(0, echo);
        }
        let after = filter(&archive, &live);
        assert_eq!(after, before, "a separate eligible component preserves every historical identity");
    }
}

#[test]
fn case_82_capture_priority_can_leave_one_duplicate() {
    let base = 1_789_005_900;

    // Newer captures get first claim on their nearest eligible record.
    let first = outgoing_repeat(2, base + 59);
    let second = outgoing_repeat(3, base + 118);
    let mut history = History::default();
    let earlier_echo = captured_outgoing(&first, &history, -59);
    history.set_loaded(vec![context_row(1)]);
    let fresh_echo = captured_outgoing(&first, &history, 0);
    assert!(same(&first, &earlier_echo), "a wider reassignment could cover the earlier echo");
    assert!(same(&second, &fresh_echo), "a wider reassignment could cover the fresh echo");
    for reversed in [false, true] {
        let mut archive = vec![first.clone(), second.clone()];
        let mut live = vec![fresh_echo.clone(), earlier_echo.clone()];
        if reversed {
            archive.reverse();
            live.reverse();
        }
        let result = filter(&archive, &live);
        assert_eq!(result.len(), 1, "capture priority deliberately leaves one ambiguous duplicate");
        assert_eq!(result[0], second, "the unmatched service occurrence stays historical");
    }

    // With three nearby repeats, the same priority can keep another id than
    // a global minimum-skew assignment, still consuming each row once.
    let oldest = outgoing_repeat(1, base - 4);
    let middle = outgoing_repeat(2, base);
    let newest = outgoing_repeat(3, base + 5);
    history.clear();
    let older = captured_outgoing(&middle, &history, 0);
    history.set_loaded(vec![oldest.clone()]);
    let fresh = captured_outgoing(&middle, &history, 2);
    assert!(!same(&oldest, &fresh), "known history stays ineligible for the fresh send");
    for reversed in [false, true] {
        let mut archive = vec![oldest.clone(), middle.clone(), newest.clone()];
        let mut live = vec![fresh.clone(), older.clone()];
        if reversed {
            archive.reverse();
            live.reverse();
        }
        let result = filter(&archive, &live);
        assert_eq!(result.len(), 1, "capture priority preserves the repeated-message count");
        assert_eq!(result[0], newest, "capture priority keeps its deterministic historical choice");
    }
}

/// #4233: a computer 133 s behind the grid. The live rows are stamped on
/// the grid's clock (time_corrected, as the LL viewer's capture and its
/// transcript writer do): the service copies of a receipt and of an echo,
/// and the plaintext line logged with them, are consumed. Stamped on the
/// computer's own clock instead, the receipt falls outside its window (5 s
/// ahead, 60 s behind) and the echo outside its 59 s, and both service
/// rows show again as history.
#[test]
fn live_rows_on_the_grids_clock_consume_their_service_copies() {
    let behind = 133;
    let grid = 1_795_068_780; // 2026-11-19 06:13:00 UTC
    let received = Hm {
        from_id: Some(RESIDENT),
        from: "Example Resident".to_owned(),
        message: "sorry I missed you".to_owned(),
        timestamp: Some(grid + 2),
        ..Hm::default()
    };
    let sent = Hm {
        from_id: Some(AGENT),
        from: "Me".to_owned(),
        message: "welp.. see you around".to_owned(),
        timestamp: Some(grid + 40),
        ..Hm::default()
    };
    let archive = vec![service_copy(&received, FIRST), service_copy(&sent, SECOND)];
    let live_at = |now: i32| {
        let mut timestamp = 0;
        let context = capture_live_message(&received.message, &mut timestamp, &incoming_context(None, true), now as u32);
        let mut receipt = live_copy(&received);
        receipt.timestamp = Some(timestamp as i32);
        record_live_message(&mut receipt, &context, now as u32);
        let mut timestamp = 0;
        let context = capture_live_message(&sent.message, &mut timestamp, &LiveContext::default(), (now + 38) as u32);
        let mut echo = live_copy(&sent);
        echo.timestamp = Some(timestamp as i32);
        record_live_message(&mut echo, &context, (now + 38) as u32);
        vec![echo, receipt]
    };

    // the computer's clock: both service rows come back
    let computer = grid + 2 - behind;
    assert_eq!(filter(&archive, &live_at(computer)), archive);
    // the grid's clock: none does, nor the logged plaintext line
    let corrected = live_at(computer + behind);
    assert!(filter(&archive, &corrected).is_empty());
    let mut logged = plaintext_copy(&corrected[1]);
    logged.legacy_wall_time = Some(f64::from(corrected[1].log_timestamp.unwrap_or(0) / 60 * 60) - 8.0 * 3600.0);
    assert!(filter(&[logged], &corrected).is_empty());
}
