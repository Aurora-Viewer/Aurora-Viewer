//! Explicit selected-object snapshots create independent projects. Partial,
//! stale or failed responses never replace the current author's work.
use super::*;
use crate::{
    build::BuildTool,
    world::hand_ik::{ReloadStatus, Seat, object_source},
};

#[derive(Default)]
pub(super) struct State {
    pub requested: Option<Uuid>,
    pub picking: bool,
    pending: Option<(Seat, Uuid)>,
    status: Status,
}

#[derive(Default)]
enum Status {
    #[default]
    Idle,
    Waiting,
    Receiving,
    Applied {
        bindings: usize,
        nodes: usize,
        active: bool,
    },
    TimedOut,
    Cancelled,
}

impl Editor {
    pub fn request_object_data(&mut self, world: &mut World, now: Instant) {
        if self.poser.active {
            return;
        }
        if self.object_data.pending.is_none() || !self.open || self.clip.active {
            world.cancel_object_ik();
        }
        if !self.open || self.clip.active {
            self.object_data = State::default();
            return;
        }
        let Some(root) = self.object_data.requested.take() else { return };
        self.object_data.pending = world.request_object_ik(root, now);
        self.object_data.status = if self.object_data.pending.is_some() {
            Status::Waiting
        } else {
            Status::Cancelled
        };
    }

    pub(super) fn begin_object_import(&mut self) {
        self.cancel_link_pick();
        self.prim_picking(true, None);
        self.object_data = State {
            picking: true,
            ..Default::default()
        };
    }

    pub(super) fn object_import_picking(&self) -> bool {
        self.object_data.picking
    }
    pub(super) fn cancel_object_import_pick(&mut self) {
        self.object_data.picking = false;
    }
    pub(super) fn object_import_root(&self) -> Option<Uuid> {
        self.object_data
            .requested
            .or_else(|| self.object_data.pending.map(|(source, _)| source.root))
    }
    pub(super) fn picked_object_import(&mut self, world: &World, build: &BuildTool, index: usize) -> bool {
        let Some(object) = world.objects.get(index) else { return false };
        let Some(source) = object_source(world, object.full_id) else {
            return false;
        };
        if let Some(props) = build.props.get(&source.root) {
            self.prims.names.insert(source.root, props.name.clone());
        }
        self.cancel_link_pick();
        self.object_data = State {
            requested: Some(source.root),
            status: Status::Waiting,
            ..Default::default()
        };
        true
    }

    pub(super) fn poll_object_data(&mut self, world: &World, frame: &Frame) {
        let Some((source, id)) = self.object_data.pending else { return };
        self.object_data.status = match world.object_ik_reload(source, id, frame.at) {
            ReloadStatus::Waiting { receiving } => {
                return self.object_data.status = if receiving { Status::Receiving } else { Status::Waiting };
            }
            ReloadStatus::Ready(data) => match self.import_object_project(world, &data.runtime) {
                Ok(()) => Status::Applied {
                    bindings: data.runtime.definition.bindings.len(),
                    nodes: data.runtime.definition.nodes.len(),
                    active: data.enabled,
                },
                Err(e) => {
                    self.project_error(e);
                    Status::Cancelled
                }
            },
            ReloadStatus::TimedOut => Status::TimedOut,
            ReloadStatus::Cancelled => Status::Cancelled,
        };
        self.object_data.pending = None;
    }

    pub(super) fn object_data_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World) {
        if self.object_data.picking {
            ui.label(aurora_i18n::tr!("ik-object-pick-hint"));
            if let Some((id, _)) = self.prims.edited
                && ui.button(aurora_i18n::tr!("ik-project-target-edited")).clicked()
                && let Some(source) = object_source(world, id)
            {
                self.cancel_link_pick();
                self.object_data.requested = Some(source.root);
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.cancel_link_pick();
            }
            if !ui.ctx().is_pointer_over_egui() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        }
        let text: Option<egui::RichText> = match &self.object_data.status {
            Status::Idle => None,
            Status::Waiting => Some(aurora_i18n::tr!("ik-seat-waiting").into()),
            Status::Receiving => Some(aurora_i18n::tr!("ik-seat-receiving").into()),
            Status::Applied { bindings, nodes, .. } => {
                Some(aurora_i18n::tr!("ik-seat-applied", bindings = *bindings, nodes = *nodes).into())
            }
            Status::TimedOut => Some(aurora_i18n::tr!("ik-seat-timeout").into()),
            Status::Cancelled => Some(aurora_i18n::tr!("ik-object-cancelled").into()),
        };
        if let Some(text) = text {
            ui.colored_label(palette.muted, text);
        }
        if matches!(self.object_data.status, Status::Applied { active: false, .. }) {
            ui.colored_label(palette.muted, aurora_i18n::tr!("ik-seat-inactive"));
        }
        if self.object_data.pending.is_some() && widgets::icon_button(ui, palette, "x", aurora_i18n::tr!("common-cancel"), true) {
            self.object_data = State::default();
        }
    }
}

#[cfg(test)]
mod tests;
