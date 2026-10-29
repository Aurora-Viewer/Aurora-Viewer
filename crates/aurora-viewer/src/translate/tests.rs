use super::*;
use aurora_translate::{Direction, Done, TranslateError, lang};
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

const BOB: Uuid = Uuid::from_u128(0xB0B);

fn en() -> &'static aurora_translate::Lang {
    lang("en").expect("en")
}
fn es() -> &'static aurora_translate::Lang {
    lang("es").expect("es")
}

fn key(n: u64) -> LineKey {
    LineKey {
        session: BOB,
        time: SystemTime::UNIX_EPOCH + Duration::from_secs(n),
    }
}

fn ok(id: u64, text: &str) -> Done {
    Done {
        id,
        result: Ok(text.into()),
        millis: 5,
    }
}

fn out(t: &mut Translations, n: u64, text: &str, now: Instant) -> u64 {
    let (job, fx) = t.outgoing(key(n), text, en(), es(), now);
    assert!(fx.is_empty());
    let job = job.expect("job");
    assert_eq!(job.direction, Direction::Outgoing);
    job.id
}

#[test]
fn outgoing_sends_the_translation() {
    let mut t = Translations::default();
    let id = out(&mut t, 1, "hello", Instant::now());
    assert!(matches!(t.state(&key(1)), Some(LineState::Pending)));
    let fx = t.done(ok(id, "hola"));
    assert_eq!(
        fx,
        [
            Effect::Send {
                to: BOB,
                text: "hola".into()
            },
            Effect::Replace {
                key: key(1),
                text: "hola".into()
            }
        ]
    );
    assert_eq!(t.state(&key(1)), Some(&LineState::Translated { original: "hello".into() }));
}

#[test]
fn outgoing_emote_keeps_its_prefix() {
    let mut t = Translations::default();
    let (job, _) = t.outgoing(key(1), "/me waves hello", en(), es(), Instant::now());
    let job = job.expect("job");
    assert_eq!(job.text, "waves hello");
    let fx = t.done(ok(job.id, "saluda"));
    assert_eq!(
        fx[0],
        Effect::Send {
            to: BOB,
            text: "/me saluda".into()
        }
    );
}

#[test]
fn outgoing_failure_sends_nothing() {
    let mut t = Translations::default();
    let id = out(&mut t, 1, "hello", Instant::now());
    let fx = t.done(Done {
        id,
        result: Err(TranslateError::Generate("x".into())),
        millis: 5,
    });
    assert!(fx.is_empty());
    assert_eq!(t.state(&key(1)), Some(&LineState::Failed { reason: FailReason::Error }));
    assert_eq!(
        t.send_original(key(1)),
        Some(Effect::Send {
            to: BOB,
            text: "hello".into()
        })
    );
    assert_eq!(t.state(&key(1)), None);
}

#[test]
fn outgoing_timeout() {
    let mut t = Translations::default();
    let now = Instant::now();
    let id = out(&mut t, 1, "hello", now);
    assert!(t.tick(now + Duration::from_millis(9_900)).is_empty());
    assert!(t.tick(now + Duration::from_millis(10_100)).is_empty());
    assert_eq!(
        t.state(&key(1)),
        Some(&LineState::Failed {
            reason: FailReason::Timeout
        })
    );
    assert!(t.done(ok(id, "hola")).is_empty());
    // a retry gets a new job
    let retry = t.retry(key(1), now).expect("job");
    assert_ne!(retry.id, id);
    assert!(matches!(t.state(&key(1)), Some(LineState::Pending)));
}

#[test]
fn too_long_is_held() {
    let mut t = Translations::default();
    let id = out(&mut t, 1, "hello", Instant::now());
    let long = "a".repeat(1024);
    assert!(t.done(ok(id, &long)).is_empty());
    assert_eq!(t.state(&key(1)), Some(&LineState::TooLong { translation: long }));
}

#[test]
fn sends_keep_typed_order() {
    let mut t = Translations::default();
    let now = Instant::now();
    let a = out(&mut t, 1, "first", now);
    let b = out(&mut t, 2, "second", now);
    assert!(t.done(ok(b, "segundo")).is_empty());
    let fx = t.done(ok(a, "primero"));
    let sent: Vec<&str> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::Send { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(sent, ["primero", "segundo"]);
}

#[test]
fn nothing_to_translate_waits_its_turn() {
    let mut t = Translations::default();
    let now = Instant::now();
    let a = out(&mut t, 1, "first message", now);
    let (job, fx) = t.outgoing(key(2), "😀", en(), es(), now);
    assert!(job.is_none() && fx.is_empty());
    let fx = t.done(ok(a, "primer mensaje"));
    assert_eq!(
        fx.last(),
        Some(&Effect::Send {
            to: BOB,
            text: "😀".into()
        })
    );
}

#[test]
fn results_for_a_closed_session_are_dropped() {
    let mut t = Translations::default();
    let id = out(&mut t, 1, "hello", Instant::now());
    t.close_session(BOB);
    assert!(t.done(ok(id, "hola")).is_empty());
    assert_eq!(t.state(&key(1)), None);
}

#[test]
fn incoming_is_replaced_by_its_translation() {
    let mut t = Translations::default();
    let job = t.incoming(key(1), "¿Dónde estás? Te estoy buscando.", es(), en()).expect("job");
    assert_eq!((job.from.code, job.to.code), ("es", "en"));
    let fx = t.done(ok(job.id, "Where are you? I'm looking for you."));
    assert_eq!(
        fx,
        [Effect::Replace {
            key: key(1),
            text: "Where are you? I'm looking for you.".into()
        }]
    );
    assert!(matches!(t.state(&key(1)), Some(LineState::Translated { .. })));
}

#[test]
fn incoming_in_my_language_is_not_translated() {
    let mut t = Translations::default();
    assert!(t.incoming(key(1), "I am looking for the shop near the beach", es(), en()).is_none());
    assert!(t.incoming(key(2), "https://example.com", es(), en()).is_none());
}

fn suggest_seq(texts: &[&str]) -> Vec<Option<&'static str>> {
    let mut s = Suggest::default();
    texts.iter().map(|t| s.observe(BOB, t, en()).map(|l| l.code)).collect()
}

#[test]
fn suggests_after_two_spanish_messages() {
    let r = suggest_seq(&["Hola, ¿cómo estás hoy amigo?", "¿Quieres ir a la playa conmigo?"]);
    assert_eq!(r, [None, Some("es")]);
}

#[test]
fn no_suggestion_after_short_or_mixed_messages() {
    assert_eq!(suggest_seq(&["Hola amigo", "¿Quieres ir a la playa conmigo?"]), [None, None]);
    assert_eq!(
        suggest_seq(&[
            "Hola, ¿cómo estás hoy amigo?",
            "I am looking for the shop",
            "¿Quieres ir a la playa conmigo?"
        ]),
        [None, None, None]
    );
}

#[test]
fn settings_round_trip() {
    let mut s = TranslateSettings::default();
    assert_eq!(s.model, aurora_translate::DEFAULT_MODEL);
    s.sessions.insert(BOB, "es".into());
    let json = serde_json::to_string(&s).expect("json");
    let back: TranslateSettings = serde_json::from_str(&json).expect("parse");
    assert_eq!(back.sessions.get(&BOB).map(String::as_str), Some("es"));
    let old: TranslateSettings = serde_json::from_str("{}").expect("old");
    assert!(old.sessions.is_empty() && !old.preview);
}

#[test]
fn parse_tr_forms() {
    let code = |t: &str| match parse_tr(t) {
        Some(TrCommand::Set(l)) => Some(l.code.to_owned()),
        Some(TrCommand::Off) => Some("off".into()),
        Some(TrCommand::Status) => Some("status".into()),
        Some(TrCommand::Unknown(w)) => Some(format!("?{w}")),
        None => None,
    };
    assert_eq!(code("/tr es").as_deref(), Some("es"));
    assert_eq!(code("/tr Spanish").as_deref(), Some("es"));
    assert_eq!(code("/TR  español ").as_deref(), Some("es"));
    assert_eq!(code("/tr off").as_deref(), Some("off"));
    assert_eq!(code("/tr").as_deref(), Some("status"));
    assert_eq!(code("/tr klingon").as_deref(), Some("?klingon"));
    assert_eq!(code("/try this"), None);
    assert_eq!(code("hello /tr es"), None);
}

fn settings_with(session_lang: Option<&str>) -> TranslateSettings {
    let mut s = TranslateSettings {
        my_language: "en".into(),
        ..Default::default()
    };
    if let Some(l) = session_lang {
        s.sessions.insert(BOB, l.into());
    }
    s
}

fn poll_until(svc: &mut TranslateService, s: &mut TranslateSettings, n: usize) -> Vec<Effect> {
    let end = Instant::now() + Duration::from_secs(3);
    let mut fx = Vec::new();
    while fx.len() < n && Instant::now() < end {
        fx.extend(svc.poll(s, Instant::now()).effects);
        std::thread::sleep(Duration::from_millis(5));
    }
    fx
}

#[test]
fn service_translates_both_ways_with_the_fake_backend() {
    let mut svc = TranslateService::with_backend(|| Box::new(aurora_translate::FakeBackend::new()));
    let mut s = settings_with(Some("es"));
    let fx = svc.submit_outgoing(&s, BOB, key(1).time, "hello there", Instant::now());
    assert!(fx.is_empty());
    let fx = poll_until(&mut svc, &mut s, 2);
    assert_eq!(
        fx[0],
        Effect::Send {
            to: BOB,
            text: "[es] hello there".into()
        }
    );
    svc.on_incoming(&s, BOB, key(2).time, "¿Dónde estás? Te estoy buscando.");
    let fx = poll_until(&mut svc, &mut s, 1);
    assert_eq!(
        fx,
        [Effect::Replace {
            key: key(2),
            text: "[en] ¿Dónde estás? Te estoy buscando.".into()
        }]
    );
}

#[test]
fn service_suggests_in_an_untranslated_conversation() {
    let mut svc = TranslateService::with_backend(|| Box::new(aurora_translate::FakeBackend::new()));
    let mut s = settings_with(None);
    svc.on_incoming(&s, BOB, key(1).time, "Hola, ¿cómo estás hoy amigo?");
    svc.on_incoming(&s, BOB, key(2).time, "¿Quieres ir a la playa conmigo?");
    assert_eq!(svc.suggestion(BOB, &s).map(|l| l.code), Some("es"));
    s.dismissed.insert(BOB);
    assert_eq!(svc.suggestion(BOB, &s), None);
}

#[test]
fn set_partner_without_a_model_asks_for_the_download() {
    let mut svc = TranslateService::default();
    let mut s = settings_with(None);
    let r = svc.set_partner(&mut s, BOB, Some(es()), std::path::Path::new("/nonexistent"));
    if aurora_translate::BUILT {
        assert_eq!(r, SetResult::NeedsDownload);
        assert!(s.sessions.is_empty());
    } else {
        assert_eq!(r, SetResult::NotBuilt);
    }
    assert_eq!(
        svc.set_partner(&mut s, BOB, None, std::path::Path::new("/nonexistent")),
        SetResult::Off
    );
}
