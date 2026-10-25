//! The torn-off Contacts window shows what the docked tab shows (#961):
//! People's friends tab in Aurora's look, the contact list otherwise.

use super::{ContactsAction, ContactsSettings, ContactsUi, Pics, window};
use crate::ui::people::{FriendsPane, FriendsSettings, PeopleAction, PeopleUi, RadarData, RadarSettings};
use crate::ui::widgets::elide_tests::{Frames, Text};
use crate::world::World;
use std::collections::HashMap;
use uuid::Uuid;

/// The window's world and the state it shares with the docked tab.
struct State {
    world: World,
    contacts: ContactsUi,
    settings: ContactsSettings,
    people: PeopleUi,
    friends: FriendsSettings,
    /// Aurora's look of People (the friends pane), else the contact list.
    aurora: bool,
    open: bool,
    actions: Vec<ContactsAction>,
    people_actions: Vec<PeopleAction>,
}

impl State {
    fn new(aurora: bool) -> State {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for i in 0..12u128 {
            let id = Uuid::from_u128(0xF000 + i);
            world.social.friends.push(crate::world::social::Friend {
                id,
                online: true,
                rights_given: 1,
                rights_has: 1,
            });
            world.social.names.insert(id, format!("Friend {i:02}"));
        }
        State {
            world,
            contacts: Default::default(),
            settings: ContactsSettings {
                torn_off: true,
                ..Default::default()
            },
            people: Default::default(),
            friends: Default::default(),
            aurora,
            open: true,
            actions: Vec::new(),
            people_actions: Vec::new(),
        }
    }

    /// One frame of the window, as the app draws it once torn off.
    fn show(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let pics = HashMap::new();
        let radar = crate::world::radar::Radar::default();
        let mut radar_settings = RadarSettings::default();
        let voice = HashMap::new();
        let data = RadarData {
            radar: &radar,
            settings: &mut radar_settings,
            voice: &voice,
            draw_distance: 64.0,
            now: 0.0,
            pics: &pics,
        };
        let friends = self.aurora.then_some(FriendsPane {
            st: &mut self.people,
            data: &data,
            settings: &mut self.friends,
            images: &pics,
        });
        let images = Pics {
            avatars: &pics,
            images: &pics,
        };
        let (contacts, people) = window(
            ctx,
            p,
            &mut self.world,
            &mut self.contacts,
            &mut self.settings,
            &images,
            friends,
            &mut self.open,
        );
        self.actions.extend(contacts);
        self.people_actions.extend(people);
    }
}

fn settle(frames: &mut Frames, state: &mut State) -> Vec<Text> {
    let p = crate::theme::Theme::default().palette();
    let mut texts = Vec::new();
    for _ in 0..4 {
        texts = frames.run(Vec::new(), |ui| state.show(ui.ctx(), &p));
    }
    texts
}

fn drawn(texts: &[Text], source: &str) -> bool {
    texts.iter().any(|t| t.source == source)
}

/// In Aurora's look the torn-off window is People's friends tab (its
/// filter field), not the contact list and its three tabs.
#[test]
fn the_torn_off_window_keeps_aurora_s_look() {
    let mut frames = Frames::new();
    let mut state = State::new(true);
    let texts = settle(&mut frames, &mut state);
    assert!(
        drawn(&texts, aurora_i18n::tr!("people-friends-filter")),
        "the friends pane's filter"
    );
    assert!(drawn(&texts, "Friend 00"), "a friend of the list");
    assert!(!drawn(&texts, aurora_i18n::tr!("ui-contacts-contact-sets")), "no contact list tabs");
}

/// In the Legacy look it stays the contact list.
#[test]
fn the_torn_off_window_keeps_the_legacy_look() {
    let mut frames = Frames::new();
    let mut state = State::new(false);
    let texts = settle(&mut frames, &mut state);
    assert!(
        drawn(&texts, aurora_i18n::tr!("ui-contacts-contact-sets")),
        "the contact list's tabs"
    );
    assert!(!drawn(&texts, aurora_i18n::tr!("people-friends-filter")), "not the friends pane");
}

/// An old hidden-search preference cannot hide the field or disable its
/// filter, either before or after switching the detached window's look.
#[test]
fn contacts_search_ignores_the_saved_hidden_setting_in_both_looks() {
    let mut frames = Frames::new();
    let mut state = State::new(true);
    state.settings.show_search = false;
    state.people.filter = "Friend 03".into();
    state.contacts.friend_filter = "Friend 03".into();
    for aurora in [true, false, true] {
        state.aurora = aurora;
        let texts = settle(&mut frames, &mut state);
        assert!(drawn(&texts, "Friend 03"), "the filtered friend in either look");
        assert!(
            !drawn(&texts, "Friend 05"),
            "the field still filters when saved show_search is false"
        );
        assert!(!state.settings.show_search, "rendering preserves the old settings");
    }
}

/// A username-only Legacy table and hidden permission columns do not
/// change Aurora's displayed name or take its rights controls away.
#[test]
fn contacts_modern_rows_ignore_saved_legacy_columns() {
    let mut frames = Frames::new();
    let mut state = State::new(true);
    let id = Uuid::from_u128(0xF000);
    state.world.social.avatar_names.on_display_names(
        &[aurora_net::AvatarNameData {
            id,
            username: "stored.username".into(),
            display_name: "Visible Name".into(),
            legacy_first: "Stored".into(),
            legacy_last: "Username".into(),
            ..Default::default()
        }],
        &[],
        None,
    );
    state.settings.column_username = true;
    state.settings.column_display_name = false;
    state.settings.column_full_name = false;
    state.settings.full_name_display_first = false;
    state.people.filter = "Visible Name".into();
    let mut previous = None;
    for permissions in [true, false] {
        state.settings.column_permissions = permissions;
        let texts = settle(&mut frames, &mut state);
        assert!(drawn(&texts, "Visible Name"), "the modern name follows global name preferences");
        let cells: Vec<_> = frames.ctx.viewport(|v| {
            v.this_pass
                .widgets
                .get_layer(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("contacts")))
                .filter(|w| w.rect.size() == egui::vec2(20.0, 22.0))
                .map(|w| w.rect)
                .collect()
        });
        assert!(!cells.is_empty(), "the rights remain available");
        if let Some(previous) = &previous {
            assert_eq!(&cells, previous, "the Legacy permission switch cannot move or hide modern rights");
        }
        previous = Some(cells);
    }
    state.world.social.avatar_names.options.use_display_names = false;
    state.people.filter = "Stored Username".into();
    let texts = settle(&mut frames, &mut state);
    assert!(drawn(&texts, "Stored Username"), "global name preferences still apply");
}

/// Legacy-specific choices remain available only in the table's Options
/// menu, while search is no longer optional.
#[test]
fn contacts_legacy_options_keep_columns_and_list_choices() {
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    let mut settings = ContactsSettings::default();
    let texts = frames.run(Vec::new(), |ui| super::options_menu(ui, &p, &mut settings));
    for key in [
        "ui-contacts-show-username-column",
        "ui-contacts-show-permission-columns",
        "ui-contacts-show-profile-pictures-in-contact-set-lists",
        "ui-contacts-friend-list-color-based-on-contact-set",
    ] {
        assert!(drawn(&texts, aurora_i18n::text(key)), "Legacy option {key}");
    }
    assert!(!drawn(&texts, aurora_i18n::tr!("ui-contacts-show-search")));
}

/// The look follows the setting while the window is torn off, and the
/// pane keeps the docked tab's state (its filter here).
#[test]
fn the_torn_off_window_follows_the_look_and_keeps_the_filter() {
    let mut frames = Frames::new();
    let mut state = State::new(false);
    settle(&mut frames, &mut state);
    state.aurora = true;
    state.people.filter = "Friend 03".into();
    let texts = settle(&mut frames, &mut state);
    assert!(drawn(&texts, aurora_i18n::tr!("people-friends-filter")) || drawn(&texts, "Friend 03"));
    assert!(drawn(&texts, "Friend 03"), "the filtered friend");
    assert!(!drawn(&texts, "Friend 05"), "the filter carried over");
    state.aurora = false;
    let texts = settle(&mut frames, &mut state);
    assert!(
        drawn(&texts, aurora_i18n::tr!("ui-contacts-contact-sets")),
        "back to the contact list"
    );
}

/// The window keeps its size in both looks.
#[test]
fn the_torn_off_window_keeps_its_size() {
    for aurora in [true, false] {
        crate::ui::widgets::steady_tests::assert_window_steady(
            "contacts",
            &[egui::vec2(430.0, 390.0), egui::vec2(700.0, 520.0)],
            "Friend 00",
            || State::new(aurora),
            |state, ctx, p| state.show(ctx, p),
        );
    }
}
