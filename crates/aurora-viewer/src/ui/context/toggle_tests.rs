//! Real scene popups, including nested/customized groups: checkable actions
//! update live without dismissing any level; ordinary commands still close.
use super::*;
use crate::ui::widgets::elide_tests::{Frames, Text};

struct Harness {
    frames: Frames,
    p: Palette,
    menu: Option<ContextMenu>,
    facts: Facts,
    prefs: Preferences,
    editor: Customization,
    texts: Vec<Text>,
}

impl Harness {
    fn new() -> Self {
        let frames = Frames::new();
        let theme = crate::theme::Theme::default();
        theme.apply(&frames.ctx, 1.0);
        Self {
            frames,
            p: theme.palette(),
            menu: Some(ContextMenu {
                target: Target::Avatar {
                    id: Uuid::from_u128(7),
                    own: false,
                    attachment: None,
                },
                pos: egui::pos2(40.0, 40.0),
                point: Vec3::ZERO,
                pie_path: Vec::new(),
                pie_input_frame: None,
            }),
            facts: Facts::default(),
            prefs: Preferences::default(),
            editor: Customization::default(),
            texts: Vec::new(),
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<CtxAction> {
        self.texts = self.frames.run(events, |ui| {
            show(
                ui.ctx(),
                &self.p,
                &mut self.menu,
                &self.facts,
                false,
                &mut self.prefs,
                &mut self.editor,
            );
        });
        take_requests(&self.frames.ctx)
    }

    fn warm(&mut self) {
        for _ in 0..5 {
            assert!(self.frame(Vec::new()).is_empty());
        }
    }

    fn at(&self, key: &'static str) -> egui::Pos2 {
        self.texts
            .iter()
            .find(|t| t.source == aurora_i18n::text(key))
            .unwrap_or_else(|| panic!("{key} missing"))
            .rect
            .center()
    }

    fn hover(&mut self, key: &'static str) {
        let at = self.at(key);
        self.frame(vec![egui::Event::PointerMoved(at)]);
        // Follow egui's hover-open delay, including sizing passes.
        for _ in 0..35 {
            self.frame(Vec::new());
        }
    }

    fn click_at(&mut self, at: egui::Pos2) -> Vec<CtxAction> {
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(self.frame(vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]));
        }
        actions
    }

    fn click(&mut self, key: &'static str) -> Vec<CtxAction> {
        self.click_at(self.at(key))
    }

    fn check(&self, key: &'static str) -> Option<bool> {
        fn find(rows: &[entries::Entry], label: &str) -> Option<bool> {
            rows.iter().find_map(|row| {
                if row.label == label {
                    return row.checked;
                }
                if let entries::Kind::Submenu(children) = &row.kind {
                    return find(children, label);
                }
                None
            })
        }
        let menu = self.menu.as_ref().expect("still open");
        find(&entries::world_menu(&menu.target, &self.facts, menu.point), aurora_i18n::text(key))
    }
}

#[test]
fn scene_render_choices_keep_the_popup_and_submenu_open_with_live_selection() {
    let mut h = Harness::new();
    h.warm();
    h.hover("ui-context-display-options");
    let keys = [
        "ui-context-always-show-full-detail",
        "ui-context-never-fully-render",
        "ui-context-display-according-to-your-preferences",
    ];
    for (key, mode) in keys.into_iter().zip([2, 1, 0]) {
        let actions = h.click(key);
        assert!(
            matches!(&actions[..], [CtxAction::SetRender(id, value)] if *id == Uuid::from_u128(7) && *value == mode),
            "{actions:?}"
        );
        h.facts.render = mode;
        h.warm();
        for (option, value) in keys.into_iter().zip([2, 1, 0]) {
            h.at(option); // The child stays visible without reopening it.
            assert_eq!(h.check(option), Some(value == mode));
        }
    }
    // A normal command in that same submenu still closes the whole menu.
    assert!(matches!(
        &h.click("ui-context-show-textures")[..],
        [CtxAction::ShowAvatarTextures(_)]
    ));
    h.frame(Vec::new());
    assert!(h.menu.is_none());
}

#[test]
fn scene_disturbance_toggles_keep_submenus_open_and_labels_stable() {
    let mut h = Harness::new();
    h.warm();
    h.hover("ui-context-disturbance");
    for blocked in [true, false] {
        assert!(matches!(&h.click("common-ignore")[..], [CtxAction::ToggleBlock(_)]));
        h.facts.blocked = blocked;
        assert!(matches!(
            &h.click("common-block-particles")[..],
            [CtxAction::ToggleOwnerParticleFilter(_)]
        ));
        h.facts.particles_blocked = blocked;
        for (key, flag) in [
            ("ui-context-block-worn-sounds", 1),
            ("ui-context-block-rezzed-sounds", 2),
            ("ui-context-block-gesture-sounds", 4),
        ] {
            let actions = h.click(key);
            assert!(
                matches!(&actions[..], [CtxAction::SetOwnerSoundBlock { owner, flag: value, blocked: value_on }] if *owner == Uuid::from_u128(7) && *value == flag && *value_on == blocked),
                "{actions:?}"
            );
            if blocked {
                h.facts.sound_blocks |= flag;
            } else {
                h.facts.sound_blocks &= !flag;
            }
            h.warm();
            h.at(key);
            assert_eq!(h.check(key), Some(blocked));
        }
        h.warm();
        assert_eq!(h.check("common-ignore"), Some(blocked));
        assert_eq!(h.check("common-block-particles"), Some(blocked));
    }
    h.click_at(egui::pos2(1200.0, 850.0));
    h.frame(Vec::new());
    assert!(h.menu.is_none(), "outside clicks still dismiss the hierarchy");
}

#[test]
fn scene_toggles_in_nested_custom_groups_keep_every_level_open() {
    let mut h = Harness::new();
    // Nest the ordinary Disturbance group inside another scene group using
    // the actual inventory arrangement resolver rather than a fake popup.
    let m = h.menu.as_ref().expect("menu");
    let available = entries::world_menu(&m.target, &h.facts, m.point);
    let catalog = customize::Catalog::new(&available);
    let profile = h
        .prefs
        .profiles
        .entry("scene:avatar".into())
        .or_insert_with(|| crate::ui::inventory::menu_layout::Profile {
            layout: Default::default(),
            open: Default::default(),
            locked: true,
        });
    assert!(profile.layout.create_group(
        &catalog.entries,
        "Nested options",
        crate::ui::inventory::menu_layout::Direction::Horizontal,
        None
    ));
    let group = profile.layout.groups.keys().next().expect("custom group").clone();
    let disturbance = catalog
        .entries
        .iter()
        .find(|e| e.label == aurora_i18n::tr!("ui-context-disturbance"))
        .expect("disturbance");
    profile
        .layout
        .placements
        .insert(disturbance.id.clone(), crate::ui::inventory::menu_layout::children(&group));
    profile.locked = true;
    h.warm();
    let at = h
        .texts
        .iter()
        .find(|t| t.source == "Nested options")
        .expect("custom group")
        .rect
        .center();
    h.frame(vec![egui::Event::PointerMoved(at)]);
    for _ in 0..35 {
        h.frame(Vec::new());
    }
    h.hover("ui-context-disturbance");
    for _ in 0..2 {
        assert!(matches!(
            &h.click("common-block-particles")[..],
            [CtxAction::ToggleOwnerParticleFilter(_)]
        ));
        h.facts.particles_blocked = !h.facts.particles_blocked;
        h.warm();
        h.at("common-block-particles");
    }
    // Escape still closes the parent and all its children.
    h.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    assert!(h.menu.is_none());
}

#[test]
fn named_particle_emitters_target_their_own_object_and_owner_without_closing() {
    let mut h = Harness::new();
    h.facts.particle_sources = vec![ParticleMenuSource {
        object: Uuid::from_u128(21),
        owner: Uuid::from_u128(31),
        name: "Smoke source".into(),
        clicked: false,
        object_blocked: false,
        owner_blocked: false,
    }];
    h.warm();
    h.hover("ui-context-disturbance");
    h.hover("scene-particles-menu");
    let at = h
        .texts
        .iter()
        .find(|t| t.source == "Smoke source")
        .expect("source name")
        .rect
        .center();
    h.frame(vec![egui::Event::PointerMoved(at)]);
    for _ in 0..35 {
        h.frame(Vec::new());
    }
    for blocked in [true, false] {
        let actions = h.click("scene-particles-block-object");
        assert!(
            matches!(&actions[..], [CtxAction::SetParticleBlock { id, owner: false, blocked: value }] if *id == Uuid::from_u128(21) && *value == blocked),
            "{actions:?}"
        );
        h.facts.particle_sources[0].object_blocked = blocked;
        h.warm();
        let actions = h.click("scene-particles-block-owner");
        assert!(
            matches!(&actions[..], [CtxAction::SetParticleBlock { id, owner: true, blocked: value }] if *id == Uuid::from_u128(31) && *value == blocked),
            "{actions:?}"
        );
        h.facts.particle_sources[0].owner_blocked = blocked;
        h.warm();
        h.at("scene-particles-block-object");
        h.at("scene-particles-block-owner");
    }
}
