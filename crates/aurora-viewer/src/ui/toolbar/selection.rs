//! Preset identity is independent of its edited layout and survives restarts.
//! Only a matching successful upload changes a personal preset's clean snapshot.
use super::{ToolbarSettings, presets::Preset, profile};
use crate::ui::skin::{Layout, ToolButton};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Source {
    Builtin(Preset),
    Personal { item: Uuid, name: String, owner: Uuid },
}

impl Source {
    pub fn label(&self) -> &str {
        match self {
            Self::Builtin(preset) => preset.label(),
            Self::Personal { name, .. } => name,
        }
    }

    pub fn personal(&self, owner: Uuid) -> Option<Uuid> {
        match self {
            Self::Personal { item, owner: account, .. } if *account == owner => Some(*item),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Selection {
    pub source: Source,
    pub baseline: profile::Profile,
}

impl Selection {
    pub fn builtin(preset: Preset, layout: &Layout) -> Self {
        Self {
            source: Source::Builtin(preset),
            baseline: profile::snapshot(&preset.layout(layout), layout, preset != Preset::Legacy),
        }
    }
}

#[derive(Default)]
pub(super) struct View {
    input: Option<ToolbarSettings>,
    skin_buttons: Vec<ToolButton>,
    snapshot: Option<profile::Profile>,
    inferred: Option<Preset>,
}

impl View {
    pub fn refresh(&mut self, toolbar: &ToolbarSettings, layout: &Layout, islands: bool) {
        if self.input.as_ref() == Some(toolbar)
            && self.skin_buttons == toolbar.buttons(layout)
            && self
                .snapshot
                .as_ref()
                .is_some_and(|profile| profile.base == profile::Base::from_islands(islands))
        {
            return;
        }
        let current = profile::snapshot(toolbar, layout, islands);
        self.inferred = Preset::ALL
            .into_iter()
            .find(|preset| Selection::builtin(*preset, layout).baseline == current);
        self.input = Some(toolbar.clone());
        self.skin_buttons = toolbar.buttons(layout).to_vec();
        self.snapshot = Some(current);
    }

    pub fn source(&self, selected: Option<&Selection>) -> Option<Source> {
        selected.map(|s| s.source.clone()).or_else(|| self.inferred.map(Source::Builtin))
    }

    pub fn dirty(&self, selected: Option<&Selection>) -> bool {
        match selected {
            Some(selection) => self.snapshot.as_ref() != Some(&selection.baseline),
            None => self.inferred.is_none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_selection_keeps_identity_and_detects_edits_reverts_and_base_changes() {
        let layout = Layout::default();
        for preset in Preset::ALL {
            let mut settings = crate::settings::Settings::default();
            assert!(preset.apply(&mut settings, &layout));
            let selection = settings.toolbar_selection.as_ref().expect("selection").clone();
            let mut view = View::default();
            let original = settings.toolbar.clone().expect("layout");
            view.refresh(&original, &layout, !settings.classic_bars);
            assert!(!view.dirty(Some(&selection)));
            assert_eq!(view.source(Some(&selection)), Some(Source::Builtin(preset)));
            let toolbar = settings.toolbar.as_mut().expect("layout");
            toolbar.address_visible = Some(!toolbar.address_visible.unwrap_or(true));
            view.refresh(toolbar, &layout, !settings.classic_bars);
            assert!(view.dirty(Some(&selection)));
            assert_eq!(view.source(Some(&selection)), Some(Source::Builtin(preset)));
            let restored: crate::settings::Settings =
                serde_json::from_str(&serde_json::to_string(&settings).expect("settings")).expect("restart");
            assert_eq!(restored.toolbar_selection.as_ref(), Some(&selection));
            view.refresh(&original, &layout, !settings.classic_bars);
            assert!(!view.dirty(Some(&selection)), "reverting clears unsaved state");
            view.refresh(&original, &layout, settings.classic_bars);
            assert!(view.dirty(Some(&selection)), "the Aurora/Legacy base belongs to the snapshot");
        }
    }

    #[test]
    fn toolbar_selection_infers_existing_builtin_and_does_not_export_preset_identity() {
        let layout = Layout::default();
        let mut view = View::default();
        let toolbar = Preset::OldSchool.layout(&layout);
        view.refresh(&toolbar, &layout, true);
        assert_eq!(view.source(None), Some(Source::Builtin(Preset::OldSchool)));
        assert!(!view.dirty(None));
        let text = profile::write(&toolbar, &layout, true).expect("portable profile");
        assert!(!text.contains("baseline") && !text.contains("source") && !text.contains("toolbar_selection"));
        let mut edited = toolbar;
        edited.address_visible = Some(!edited.address_visible.unwrap_or(true));
        view.refresh(&edited, &layout, true);
        assert!(view.source(None).is_none());
        assert!(view.dirty(None));
    }
    #[test]
    fn toolbar_editor_adopts_an_existing_preset_before_the_first_edit_and_keeps_personal_identity() {
        let layout = Layout::default();
        let mut toolbar = Preset::OldSchool.layout(&layout);
        let mut state = super::super::ToolbarUi {
            open: true,
            islands: true,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))),
                ..Default::default()
            },
            |ui| {
                state.show(ui.ctx(), &p, &layout, &toolbar, None);
            },
        );
        let inferred = state.inferred_preset.take().expect("persist the initial identity");
        assert_eq!(inferred, Preset::OldSchool);
        let mut selected = Selection::builtin(inferred, &layout);
        toolbar.address_visible = Some(false);
        state.preset_view.refresh(&toolbar, &layout, true);
        assert!(state.preset_view.dirty(Some(&selected)));
        assert_eq!(state.preset_view.source(Some(&selected)), Some(Source::Builtin(inferred)));
        selected.source = Source::Personal {
            item: Uuid::new_v4(),
            owner: Uuid::new_v4(),
            name: "My old school".into(),
        };
        assert_eq!(
            state.preset_view.source(Some(&selected)),
            Some(selected.source.clone()),
            "same layout does not replace a personal identity with the built-in"
        );
    }
}
