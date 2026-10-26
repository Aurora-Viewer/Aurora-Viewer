//! Object.Inspect / task_properties (LLSidepanelTaskInfo): the selected
//! linkset's actual simulator properties, with editing through the shared
//! native General inspector rather than a second permission editor.
use crate::{
    theme::Palette,
    ui::{self, widgets},
    world::World,
};
use aurora_net::build::{ObjectProps, perm};
use uuid::Uuid;

#[derive(Default)]
pub struct ObjectProfileUi {
    pub(crate) object: Option<Uuid>,
}

pub(super) fn person(ui: &mut egui::Ui, p: &Palette, world: &mut World, id: Uuid) {
    if id.is_nil() {
        ui.label(aurora_i18n::tr!("status-nobody"));
    } else if !world.rlva_can_name(id) {
        ui.label(aurora_i18n::tr!("scene-object-hidden-person"));
    } else {
        world.social.want_name(id);
        let name = world.person_name(&id).unwrap_or_else(|| aurora_i18n::tr!("common-loading").into());
        if ui
            .add(egui::Label::new(egui::RichText::new(name).color(p.violet_light)).sense(egui::Sense::click()))
            .clicked()
        {
            super::request(ui.ctx(), super::CtxAction::Profile(id));
        }
    }
}

impl ObjectProfileUi {
    pub(crate) fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World, props: Option<&ObjectProps>) -> Option<Uuid> {
        let id = self.object?;
        let mut open = true;
        let mut edit = None;
        widgets::Floater::new(
            "object_profile",
            aurora_i18n::tr!("ui-context-object-profile"),
            egui::pos2(260.0, 160.0),
            egui::vec2(450.0, 560.0),
        )
        .show(ctx, p, &mut open, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let exists = world.objects.index_of_uuid(&id).is_some();
                if let Some(props) = props.filter(|props| props.object_id == id) {
                    ui.label(egui::RichText::new(&props.name).strong().color(p.violet_light));
                    if !props.description.is_empty() {
                        ui.label(&props.description);
                    }
                    widgets::divider(ui, p);
                    egui::Grid::new("object_profile_identity")
                        .num_columns(2)
                        .spacing(egui::vec2(14.0, 7.0))
                        .show(ui, |ui| {
                            for (label, person_id, owner) in [
                                (aurora_i18n::tr!("common-creator-2"), props.creator_id, false),
                                (aurora_i18n::tr!("common-owner-2"), props.owner_id, true),
                                (aurora_i18n::tr!("scene-object-last-owner"), props.last_owner_id, false),
                            ] {
                                ui.label(label);
                                if !props.group_id.is_nil() && (person_id == props.group_id || owner && person_id.is_nil()) {
                                    ui.label(aurora_i18n::tr!("common-group"));
                                } else {
                                    person(ui, p, world, person_id);
                                }
                                ui.end_row();
                            }
                            ui.label(aurora_i18n::tr!("common-group-2"));
                            ui.label(if props.group_id.is_nil() {
                                aurora_i18n::tr!("common-none").into()
                            } else {
                                world
                                    .groups
                                    .group(&props.group_id)
                                    .map_or_else(|| props.group_id.to_string(), |group| group.name.clone())
                            });
                            ui.end_row();
                            ui.label(aurora_i18n::tr!("scene-object-created"));
                            ui.label(ui::inspect::slt_datetime(props.creation_date / 1_000_000));
                            ui.end_row();
                        });
                    widgets::divider(ui, p);
                    if props.full {
                        ui.label(egui::RichText::new(aurora_i18n::tr!("scene-object-permissions")).strong());
                        egui::Grid::new("object_profile_permissions")
                            .num_columns(4)
                            .spacing(egui::vec2(10.0, 6.0))
                            .show(ui, |ui| {
                                ui.label("");
                                for label in [
                                    aurora_i18n::tr!("common-copy"),
                                    aurora_i18n::tr!("scene-object-modify"),
                                    aurora_i18n::tr!("common-transfer"),
                                ] {
                                    ui.label(label);
                                }
                                ui.end_row();
                                for (label, mask) in [
                                    (aurora_i18n::tr!("common-owner"), props.owner_mask),
                                    (aurora_i18n::tr!("common-group"), props.group_mask),
                                    (aurora_i18n::tr!("common-everyone"), props.everyone_mask),
                                    (aurora_i18n::tr!("build-ui-general-next-owner"), props.next_owner_mask),
                                ] {
                                    ui.label(label);
                                    for bit in [perm::COPY, perm::MODIFY, perm::TRANSFER] {
                                        let mut granted = mask & bit != 0;
                                        ui.add_enabled_ui(false, |ui| {
                                            widgets::checkbox(ui, p, &mut granted, "");
                                        });
                                    }
                                    ui.end_row();
                                }
                            });
                        if props.sale_type != 0 {
                            ui.label(aurora_i18n::tr!("scene-object-sale-price", price = props.sale_price));
                        }
                    } else {
                        ui.label(aurora_i18n::tr!("build-ui-general-waiting-for-simulator-properties"));
                    }
                } else {
                    ui.label(aurora_i18n::tr!("build-ui-general-waiting-for-simulator-properties"));
                }
                if !exists {
                    ui.label(aurora_i18n::tr!("scene-object-no-longer-visible"));
                }
                widgets::divider(ui, p);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(exists, egui::Button::new(aurora_i18n::tr!("scene-object-edit-properties")))
                        .clicked()
                    {
                        edit = Some(id);
                    }
                    if ui.button(aurora_i18n::tr!("build-ui-general-copy-uuid")).clicked() {
                        ui.ctx().copy_text(id.to_string());
                    }
                });
            });
        });
        if !open {
            self.object = None;
        }
        edit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_never_displays_properties_from_another_object_and_handles_a_departed_object() {
        use crate::ui::widgets::elide_tests::Frames;
        let mut frames = Frames::new();
        let theme = crate::theme::Theme::default();
        theme.apply(&frames.ctx, 1.0);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let id = Uuid::from_u128(1);
        let mut profile = ObjectProfileUi { object: Some(id) };
        let mut props = ObjectProps {
            object_id: Uuid::from_u128(2),
            full: true,
            name: "Unrelated confidential name".into(),
            ..Default::default()
        };
        for _ in 0..3 {
            let texts = frames.run(Vec::new(), |ui| {
                assert!(profile.show(ui.ctx(), &theme.palette(), &mut world, Some(&props)).is_none());
            });
            assert!(!texts.iter().any(|text| text.source.contains("Unrelated confidential")));
        }
        props.object_id = id;
        props.name = "Selected object".into();
        let texts = frames.run(Vec::new(), |ui| {
            profile.show(ui.ctx(), &theme.palette(), &mut world, Some(&props));
        });
        assert!(texts.iter().any(|text| text.source == "Selected object"));
        assert!(
            texts
                .iter()
                .any(|text| text.source == aurora_i18n::tr!("scene-object-no-longer-visible"))
        );
        assert!(super::super::take_requests(&frames.ctx).is_empty());
    }
}
