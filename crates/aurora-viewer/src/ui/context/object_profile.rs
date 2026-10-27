//! Editable Object.Inspect / task_properties, following LLSidepanelTaskInfo
//! (indra/newview/llsidepaneltaskinfo.cpp, originally LGPL 2.1). General and
//! this window share fields, permission rules, native commands and dialogs.
//! Its private selection and drafts stay pinned to the requested UUID.
use crate::{
    build::{BuildTool, ui::general},
    theme::Palette,
    ui::{self, widgets},
    world::{World, objects::ObjKey},
};
use aurora_net::{NetCommand, build::ObjectProps};
use uuid::Uuid;

#[derive(Default)]
pub struct ObjectProfileUi {
    pub(crate) object: Option<Uuid>,
    // A second full editor must not enlarge App's Windows startup stack.
    editor: Box<BuildTool>,
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
    /// Keep the profile's live simulator subscription when a menu, inspector
    /// or build selection releases the same root. Closing releases our hold.
    pub(crate) fn retain_selection(&self, world: &World, command: &mut NetCommand) -> bool {
        if let NetCommand::Build(aurora_net::build::BuildCmd::Deselect { handle, local_ids }) = command
            && let Some(object) = self
                .object
                .and_then(|id| world.objects.index_of_uuid(&id))
                .and_then(|index| world.objects.get(index))
            && object.key.region == *handle
        {
            local_ids.retain(|local| *local != object.key.local_id);
            return !local_ids.is_empty();
        }
        true
    }

    pub(crate) fn open(&mut self, id: Uuid, key: ObjKey, props: Option<ObjectProps>) {
        if self.object != Some(id) {
            *self = Self::default();
        }
        self.object = Some(id);
        self.editor.selection = vec![key];
        if let Some(props) = props {
            self.on_properties(&[props]);
        }
    }

    pub(crate) fn on_properties(&mut self, props: &[ObjectProps]) {
        let received = props.iter().filter(|props| Some(props.object_id) == self.object).cloned().collect();
        self.editor.on_properties(received);
    }

    pub(crate) fn take_output(&mut self) -> (Vec<NetCommand>, Vec<crate::build::ui::Request>, Vec<ObjectProps>) {
        let commands = std::mem::take(&mut self.editor.out);
        let properties = if commands.is_empty() {
            Vec::new()
        } else {
            self.editor.props.values().cloned().collect()
        };
        (commands, std::mem::take(&mut self.editor.ui.requests), properties)
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World, facts: &super::ObjectFacts) {
        let Some(id) = self.object else { return };
        let index = world.objects.index_of_uuid(&id);
        if let Some(key) = index.and_then(|index| world.objects.get(index)).map(|object| object.key) {
            self.editor.selection = vec![key];
        }
        let mut open = true;
        let mut floater = widgets::Floater::new(
            "object_profile",
            aurora_i18n::tr!("ui-context-object-profile"),
            egui::pos2(260.0, 100.0),
            egui::vec2(490.0, 760.0),
        );
        floater.min_size = egui::vec2(380.0, 420.0);
        floater.show(ctx, p, &mut open, |ui| {
            ui.push_id(id, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if index.is_some() {
                        general::show(ui, p, &mut self.editor, world, false);
                    } else {
                        if let Some(props) = self.editor.props.get(&id) {
                            ui.label(&props.name);
                        }
                        ui.label(aurora_i18n::tr!("scene-object-no-longer-visible"));
                    }
                    if let Some(props) = self.editor.props.get(&id).filter(|props| props.full) {
                        ui.label(aurora_i18n::tr!(
                            "scene-object-created-at",
                            date = ui::inspect::slt_datetime(props.creation_date / 1_000_000)
                        ));
                    }
                    if !self.editor.status.is_empty() {
                        ui.label(&self.editor.status);
                    }
                    widgets::divider(ui, p);
                    ui.horizontal_wrapped(|ui| {
                        // Explicit interactions use the existing object actions;
                        // the combo above alone edits the default click action.
                        for (label, enabled, action) in [
                            (aurora_i18n::tr!("common-open"), facts.open, super::CtxAction::Open(id)),
                            (aurora_i18n::tr!("common-pay"), facts.pay, super::CtxAction::Pay(id)),
                            (aurora_i18n::tr!("common-buy"), facts.buy, super::CtxAction::Buy(id)),
                            (aurora_i18n::tr!("scene-object-details"), true, super::CtxAction::Inspect(id)),
                        ] {
                            if ui.add_enabled(index.is_some() && enabled, egui::Button::new(label)).clicked() {
                                super::request(ctx, action);
                            }
                        }
                        if ui.button(aurora_i18n::tr!("build-ui-general-copy-uuid")).clicked() {
                            ctx.copy_text(id.to_string());
                        }
                    });
                });
            });
        });
        if index.is_some() {
            general::property_dialogs(ctx, p, &mut self.editor, world);
        }
        if !open {
            self.object = None;
            self.editor.ui.edits.clear();
            self.editor.ui.sale_draft = None;
            self.editor.ui.group_picker_open = false;
            self.editor.ui.confirm = None;
        }
    }
}

#[cfg(test)]
#[path = "object_profile_tests.rs"]
mod tests;
