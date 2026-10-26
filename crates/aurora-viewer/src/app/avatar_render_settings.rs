//! Share display exceptions between the resident menus and their editor.
use super::*;
use crate::ui::avatar_render_settings::Row;
use uuid::Uuid;

fn apply(settings: &mut Settings, agent: Uuid, ids: &[Uuid], mode: u8) -> bool {
    if mode > 2 {
        return false;
    }
    let mut changed = false;
    for id in ids.iter().copied().filter(|id| !id.is_nil() && *id != agent) {
        if settings.render_exception(&id) != mode {
            settings.set_render_exception(id, mode);
            changed = true;
        }
    }
    changed
}

impl App {
    pub(super) fn set_avatar_render_settings(&mut self, ids: &[Uuid], mode: u8) {
        if apply(&mut self.settings, self.world.agent_id, ids, mode) {
            self.settings.save();
        }
    }

    pub(super) fn avatar_render_settings_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let mut rows = Vec::new();
        if self.panels.avatar_render_settings {
            for (id, mode) in self.settings.render_exceptions_map() {
                if id.is_nil() || id == self.world.agent_id || !(1..=2).contains(&mode) {
                    continue;
                }
                let can_name = self.world.rlva_can_name(id);
                let name = if can_name {
                    self.world.social.want_name(id);
                    self.world.social.name_of(&id)
                } else {
                    crate::world::rlva::privacy::anonymous(id).display_name.clone()
                };
                rows.push(Row { id, mode, name, can_name });
            }
        }
        if let Some(change) = self
            .avatar_render_settings_ui
            .show(ctx, p, &rows, &mut self.world, &mut self.panels.avatar_render_settings)
        {
            self.set_avatar_render_settings(&change.ids, change.mode);
        }
    }

    pub(super) fn demo_avatar_render_settings(&mut self, mode: &str) {
        self.panels.avatar_render_settings = true;
        self.panels.people = false;
        self.panels.chat = false;
        self.settings.render_exceptions.clear();
        if mode != "empty" {
            for (id, name, setting) in [
                (crate::demo::DEMO_LOUP, "Loup Demo", 2),
                (crate::demo::DEMO_NOVA, "Nova Demo", 1),
                (Uuid::from_u128(0xD664), "Distant Resident", 2),
            ] {
                self.world.social.names.insert(id, name.to_owned());
                self.settings.set_render_exception(id, setting);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_render_settings_edit_distant_residents_without_changing_other_exceptions() {
        let mut settings = Settings::default();
        let (agent, a, b, other) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        settings.set_render_exception(other, 1);
        assert!(apply(&mut settings, agent, &[a, b, a, agent, Uuid::nil()], 2));
        assert_eq!(settings.render_exceptions.len(), 3);
        assert!(!apply(&mut settings, agent, &[a, b], 2));
        assert!(!apply(&mut settings, agent, &[other], 3));
        assert!(apply(&mut settings, agent, &[a, b], 1));
        // The same serialized settings feed both the menu and the next session.
        let bytes = serde_json::to_vec(&settings).expect("settings");
        let mut restored: Settings = serde_json::from_slice(&bytes).expect("reload");
        assert_eq!(restored.render_exception(&a), 1);
        assert!(apply(&mut restored, agent, &[a, b], 0));
        assert_eq!(restored.render_exceptions_map(), std::collections::HashMap::from([(other, 1)]));
        let rules = crate::scene::complexity::Rules {
            max: 1,
            max_area: 0.0,
            mode: 0,
            exceptions: &settings.render_exceptions_map(),
            friends: &Default::default(),
        };
        assert!(rules.too_complex(&a, false, 0, 0.0));
        settings.set_render_exception(a, 2);
        let rules = crate::scene::complexity::Rules {
            exceptions: &settings.render_exceptions_map(),
            ..rules
        };
        assert!(!rules.too_complex(&a, false, u32::MAX, 100_000.0));
    }
}
