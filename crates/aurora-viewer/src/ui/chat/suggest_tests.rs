use super::*;
use crate::ui::chat::{ChatUi, OutgoingChat, bar};
use aurora_assets::gesture::Gesture;

fn person(n: u128, display: &str, user: &str) -> Person {
    Person {
        id: Uuid::from_u128(n),
        display: display.into(),
        user: user.into(),
    }
}

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

#[test]
fn a_mention_opens_at_the_start_of_a_word() {
    assert_eq!(mention_at(&chars("@"), 1), Some((0, String::new())));
    assert_eq!(mention_at(&chars("hi @ja"), 6), Some((3, "ja".into())));
    assert_eq!(mention_at(&chars("hi @ja there"), 6), Some((3, "ja".into())));
    // an address, a word that only contains it, the cursor elsewhere
    assert_eq!(mention_at(&chars("me@ja"), 5), None);
    assert_eq!(mention_at(&chars("hi @ja"), 2), None);
    assert_eq!(mention_at(&chars(""), 0), None);
}

#[test]
fn a_cursor_before_a_mention_does_not_open_it() {
    for text in [" @Alice", "hi @ja", "hi  @ja", "hi\n@ja", "hi\t@ja", "é 👋 @由紀"] {
        let chars = chars(text);
        let at = chars.iter().position(|c| *c == '@').expect("the sample has a mention");
        assert_eq!(mention_at(&chars, at), None, "before @ in {text:?}");
        assert_eq!(mention_at(&chars, at + 1), Some((at, String::new())), "after @ in {text:?}");
    }
}

#[test]
fn mentions_match_names_then_words_then_anywhere() {
    let people = [
        person(1, "Moon Child", "luna.star"),
        person(2, "Janet", "janet"),
        person(3, "Big Jay", "jay.walker"),
        person(4, "Ninja", "ninja"),
    ];
    let ids = |typed: &str| {
        mention_matches(&people, typed, 8)
            .iter()
            .map(|p| p.id.as_u128())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(""), vec![1, 2, 3, 4], "@ alone lists everyone, in the given order");
    assert_eq!(ids("ja"), vec![2, 3, 4]);
    assert_eq!(ids("LUNA"), vec![1], "user names too, whatever the case");
    assert_eq!(ids("star"), vec![1]);
    assert!(ids("zz").is_empty());
    assert_eq!(mention_matches(&people, "", 2).len(), 2);
}

#[test]
fn chosen_labels_are_sent_as_mention_links() {
    let mut shared = Shared::default();
    let (jane, doe) = (Uuid::from_u128(7), Uuid::from_u128(8));
    shared.remember(None, "@Jane".into(), jane);
    shared.remember(None, "@Jane Doe".into(), doe);
    let link = |id| mention_link(id);
    assert_eq!(
        shared.expand(None, "@Jane Doe hi, @Jane!"),
        format!("{} hi, {}!", link(doe), link(jane))
    );
    // inside a longer word, an address, or another conversation: as typed
    assert_eq!(shared.expand(None, "@Janet and me@Jane"), "@Janet and me@Jane");
    assert_eq!(shared.expand(Some(jane), "@Jane"), "@Jane");
    // the latest resident of a label wins
    shared.remember(None, "@Jane".into(), doe);
    assert_eq!(shared.expand(None, "@Jane"), link(doe));
}

#[test]
fn labels_are_bounded_per_conversation() {
    let mut shared = Shared::default();
    for n in 0..(LABELS as u128 + 5) {
        shared.remember(None, format!("@P{n}"), Uuid::from_u128(n));
    }
    assert_eq!(shared.labels[&None].len(), LABELS);
    assert_eq!(shared.expand(None, "@P0"), "@P0");
    let last = LABELS as u128 + 4;
    assert_eq!(shared.expand(None, &format!("@P{last}")), mention_link(Uuid::from_u128(last)));
}

#[test]
fn name_prediction_follows_the_legacy_rules() {
    let around = [(person(1, "Moonie", "Jane Dorian"), 3.0), (person(2, "Bob", "Bobby"), 8.0)];
    // the rest of the first name and a space, selected
    assert_eq!(predict("hi jan", 6, &around), Some(("hi jane ".into(), 6, 8)));
    assert_eq!(predict("hi JAN", 6, &around), Some(("hi JANe ".into(), 6, 8)));
    // two words: the rest of the last name, three letters of it typed
    assert_eq!(predict("hi jane dor", 11, &around), Some(("hi jane dorian ".into(), 11, 15)));
    assert_eq!(predict("hi jane do", 10, &around), None);
    // in the middle of the line, the rest kept after
    assert_eq!(predict("bob, hi", 3, &around), Some(("bobby , hi".into(), 3, 6)));
    // the whole line must be longer than three letters (length > 3)
    assert_eq!(predict("jan", 3, &around), None);
    assert_eq!(predict("hi ja", 5, &around), None, "two letters only");
    assert_eq!(predict("hi jane ", 8, &around), None, "after a space");
    assert_eq!(predict("hi jane", 7, &around), None, "nothing left to add");
    assert_eq!(predict("hi xyz", 6, &around), None);
}

#[test]
fn gestures_matching_a_prefix_are_sorted_once() {
    let mut gestures = Manager::default();
    for (n, trigger) in [(1, "/hug"), (2, "/Hi"), (3, "/hi"), (4, "/eject")] {
        let id = Uuid::from_u128(n);
        gestures.entries.insert(
            id,
            crate::gestures::Entry {
                asset: Uuid::nil(),
                definition: Gesture {
                    trigger: trigger.into(),
                    ..Default::default()
                },
            },
        );
        gestures.active.insert(id);
    }
    let triggers = |g: &Manager, prefix: &str| g.matching(prefix).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    assert_eq!(triggers(&gestures, "/H"), vec!["/Hi", "/hug"]);
    assert_eq!(triggers(&gestures, "/e"), vec!["/eject"]);
    assert!(triggers(&gestures, "/x").is_empty());
    gestures.enabled = false;
    assert!(triggers(&gestures, "/h").is_empty());
}

struct Fixture {
    ctx: egui::Context,
    chat: ChatUi,
    gestures: Manager,
}

impl Fixture {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique("nearby_chat_input")));
        let mut gestures = Manager::default();
        for (n, trigger, says) in [(1, "/hi", "Hello there!"), (2, "/hug", ""), (3, "/eject", "")] {
            let id = Uuid::from_u128(n);
            gestures.entries.insert(
                id,
                crate::gestures::Entry {
                    asset: Uuid::nil(),
                    definition: Gesture {
                        trigger: trigger.into(),
                        replacement: says.into(),
                        ..Default::default()
                    },
                },
            );
            gestures.active.insert(id);
        }
        let mut chat = ChatUi::default();
        chat.suggest.set_nearby(
            vec![
                (person(10, "Moonie", "Jane Doe"), 4.0),
                (person(11, "Janet", "janet"), 12.0),
                // beyond chat range: not listed after @
                (person(12, "Jasper", "jasper"), 60.0),
            ],
            0.0,
        );
        let mut fixture = Self { ctx, chat, gestures };
        for _ in 0..3 {
            fixture.frame(Vec::new());
        }
        fixture
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Option<OutgoingChat> {
        let mut outgoing = None;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                events,
                ..Default::default()
            },
            |ui| {
                ui.horizontal(|ui| {
                    outgoing = bar(ui, &crate::theme::Theme::default().palette(), &mut self.chat, &self.gestures, 400.0);
                });
            },
        );
        output.textures_delta.clear();
        outgoing
    }

    fn type_text(&mut self, s: &str) {
        for c in s.chars() {
            self.frame(vec![egui::Event::Text(c.to_string())]);
        }
    }

    fn key(&mut self, key: egui::Key) -> Option<OutgoingChat> {
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }])
    }

    fn list(&self) -> List {
        self.ctx
            .data(|d| d.get_temp(list_id(egui::Id::unique("nearby_chat_input"))))
            .unwrap_or_default()
    }

    fn shown(&self) -> Vec<String> {
        let list = self.list();
        if !list.shown() {
            return Vec::new();
        }
        list.items
            .iter()
            .map(|i| match i {
                Item::Person(p) => p.display.clone(),
                Item::Gesture(t, _) => t.clone(),
            })
            .collect()
    }
}

#[test]
fn at_lists_the_residents_in_chat_range_and_enter_writes_the_mention() {
    let mut f = Fixture::new();
    f.type_text("hi @");
    assert_eq!(f.shown(), vec!["Moonie", "Janet"], "nearest first, within chat range");
    f.type_text("ja");
    assert_eq!(f.shown(), vec!["Moonie", "Janet"], "Jane Doe's user name matches too");
    f.key(egui::Key::ArrowDown);
    assert_eq!(f.list().selected, Some(1));
    assert!(f.key(egui::Key::Enter).is_none(), "Enter picked from the list must not send");
    assert_eq!(f.chat.input, "hi @Janet ");
    assert!(f.shown().is_empty());
    f.type_text("o/");
    let sent = f.key(egui::Key::Enter).expect("sent");
    assert_eq!(
        sent.message,
        format!("hi secondlife:///app/agent/{}/mention o/", Uuid::from_u128(11))
    );
}

#[test]
fn pasting_a_mention_and_moving_before_it_keeps_the_field_usable() {
    let mut f = Fixture::new();
    f.frame(vec![egui::Event::Paste("hi @ja".into())]);
    assert_eq!(f.shown(), vec!["Moonie", "Janet"]);
    for _ in 0..3 {
        assert!(f.key(egui::Key::ArrowLeft).is_none());
    }
    assert_eq!(f.chat.input, "hi @ja");
    assert!(f.shown().is_empty(), "the cursor is before @");
    f.key(egui::Key::ArrowRight);
    assert_eq!(f.shown(), vec!["Moonie", "Janet"], "the cursor is after @ again");
}

#[test]
fn escape_closes_the_residents_until_the_letters_change() {
    let mut f = Fixture::new();
    f.type_text("@m");
    assert_eq!(f.shown(), vec!["Moonie"]);
    f.key(egui::Key::Escape);
    assert!(f.shown().is_empty());
    assert_eq!(f.chat.input, "@m");
    f.type_text("o");
    assert_eq!(f.shown(), vec!["Moonie"]);
    // without the option, nothing is listed
    let mut f = Fixture::new();
    f.chat.suggest.options.mentions = false;
    f.type_text("@m");
    assert!(f.shown().is_empty());
}

#[test]
fn slash_lists_the_gestures_and_the_arrows_write_them() {
    let mut f = Fixture::new();
    f.type_text("/h");
    assert_eq!(f.shown(), vec!["/hi", "/hug"]);
    assert_eq!(f.chat.input, "/h", "no common rest to complete inline");
    assert_eq!(f.list().selected, None);
    f.key(egui::Key::ArrowDown);
    assert_eq!(f.chat.input, "/hi");
    f.key(egui::Key::ArrowDown);
    assert_eq!(f.chat.input, "/hug");
    assert_eq!(f.list().selected, Some(1));
    // Enter is not the list's: it says the gesture written
    let sent = f.key(egui::Key::Enter).expect("gesture sent");
    assert_eq!(sent.message, "/hug");
}

#[test]
fn tab_keeps_the_gesture_and_closes_the_list() {
    let mut f = Fixture::new();
    f.type_text("/e");
    assert_eq!(f.chat.input, "/eject", "inline completion as before");
    assert_eq!(f.shown(), vec!["/eject"]);
    assert_eq!(f.list().selected, Some(0));
    f.key(egui::Key::Tab);
    assert_eq!(f.chat.input, "/eject");
    assert!(f.shown().is_empty());
    f.type_text(" now");
    assert_eq!(f.chat.input, "/eject now");
    // without the option, neither the list nor the inline completion
    let mut f = Fixture::new();
    f.chat.suggest.options.gestures = false;
    f.type_text("/e");
    assert_eq!(f.chat.input, "/e");
    assert!(f.shown().is_empty());
}

#[test]
fn names_around_are_predicted_unless_the_option_is_off() {
    let mut f = Fixture::new();
    f.chat.suggest.options.names = false;
    f.type_text("hi jan");
    assert_eq!(f.chat.input, "hi jan");
    // on by default
    let mut f = Fixture::new();
    f.type_text("hi jan");
    assert_eq!(f.chat.input, "hi jane ");
    // the next letter replaces the prediction
    f.type_text("x");
    assert_eq!(f.chat.input, "hi janx");
}
