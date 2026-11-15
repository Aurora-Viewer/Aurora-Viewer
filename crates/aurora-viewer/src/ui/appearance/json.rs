//! Appearance XML and outfit JSON file dialogs. Native dialogs, bounded reads,
//! parsing and atomic writes run on a worker, like settings/stream exports.
use super::*;
use crate::world::outfit_backup::{self, Backup, Resolved};
use std::{
    io::Read,
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};

enum Outcome {
    Saved(PathBuf),
    Read(Backup),
    /// The file, and how many worn items were left out (no modify rights).
    AppearanceSaved(PathBuf, usize),
    Failed(String),
    Cancelled,
}

struct Job {
    appearance: bool,
    scope: (Uuid, Uuid),
    receiver: Receiver<Outcome>,
}

pub(crate) struct Pending {
    scope: (Uuid, Uuid),
    backup: Backup,
    name: String,
    resolved: Resolved,
    generation: u64,
    deadline: Instant,
    requested: bool,
    ready: bool,
    focus: bool,
}

impl Pending {
    fn new(world: &World, backup: Backup) -> Self {
        Self {
            scope: scope(world),
            name: backup.name.clone(),
            resolved: backup.resolve(&world.inventory, world.agent_id),
            generation: world.inventory.generation,
            backup,
            deadline: Instant::now() + Duration::from_secs(15),
            requested: false,
            ready: false,
            focus: true,
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    job: Option<Job>,
    pending: Option<Pending>,
}

fn scope(world: &World) -> (Uuid, Uuid) {
    (world.agent_id, world.inventory.root)
}

impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.pending.is_some()
    }
    pub fn has_confirmation(&self) -> bool {
        self.pending.is_some()
    }

    pub fn export(&mut self, ctx: &egui::Context, world: &World, backup: Backup) {
        if self.busy() {
            return;
        }
        self.spawn(ctx, world, false, move || {
            let filename = outfit_backup::file_name(&backup.name);
            let Some(path) = dialog(aurora_i18n::tr!("outfit-json-export")).set_file_name(filename).save_file() else {
                return Outcome::Cancelled;
            };
            match backup
                .bytes()
                .and_then(|bytes| crate::atomic_file::write(&path, &bytes).map_err(|e| e.to_string()))
            {
                Ok(()) => Outcome::Saved(path),
                Err(reason) => {
                    log::warn!("outfit JSON export failed: {reason}");
                    Outcome::Failed(aurora_i18n::tr!("outfit-json-failed").into())
                }
            }
        });
    }

    pub fn export_appearance(&mut self, ctx: &egui::Context, world: &World) {
        if self.busy() {
            return;
        }
        let Some(snapshot) = crate::world::appearance_xml::Snapshot::own(world) else {
            return;
        };
        let left_out = snapshot.left_out();
        self.spawn(ctx, world, true, move || {
            let Some(path) = rfd::FileDialog::new()
                .set_title(aurora_i18n::tr!("appearance-xml-export"))
                .add_filter(aurora_i18n::tr!("appearance-xml-file-type"), &["xml"])
                .set_file_name("appearance.xml")
                .save_file()
            else {
                return Outcome::Cancelled;
            };
            match snapshot.write(&path) {
                Ok(()) => Outcome::AppearanceSaved(path, left_out),
                Err(reason) => {
                    log::warn!("appearance XML export failed: {reason}");
                    Outcome::Failed(aurora_i18n::tr!("appearance-xml-export-failed").into())
                }
            }
        });
    }

    pub fn import(&mut self, ctx: &egui::Context, world: &World) {
        if self.busy() {
            return;
        }
        self.spawn(ctx, world, false, || {
            let Some(path) = dialog(aurora_i18n::tr!("outfit-json-import")).pick_file() else {
                return Outcome::Cancelled;
            };
            match read(&path) {
                Ok(backup) => Outcome::Read(backup),
                Err(reason) => Outcome::Failed(reason),
            }
        });
    }

    fn spawn(&mut self, ctx: &egui::Context, world: &World, appearance: bool, work: impl FnOnce() -> Outcome + Send + 'static) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        self.job = Some(Job {
            appearance,
            scope: scope(world),
            receiver,
        });
        if let Err(e) = std::thread::Builder::new().name("aurora-appearance-file".into()).spawn(move || {
            let _ = sender.send(work());
            ctx.request_repaint();
        }) {
            log::warn!("cannot start appearance file worker: {e}");
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context, world: &mut World) -> Vec<aurora_net::NetCommand> {
        if self.job.as_ref().is_some_and(|job| job.scope != scope(world)) || self.pending.as_ref().is_some_and(|p| p.scope != scope(world))
        {
            *self = Self::default();
            return Vec::new();
        }
        if let Some(job) = &self.job {
            let appearance = job.appearance;
            let outcome = match job.receiver.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Outcome::Failed(
                    if appearance {
                        aurora_i18n::tr!("appearance-xml-export-failed")
                    } else {
                        aurora_i18n::tr!("outfit-json-failed")
                    }
                    .into(),
                )),
            };
            if let Some(outcome) = outcome {
                self.job = None;
                match outcome {
                    Outcome::AppearanceSaved(path, left_out) => {
                        world.notifications.push(
                            crate::world::notifications::Kind::Info,
                            aurora_i18n::tr!("appearance-xml-export"),
                            saved_message(&path, left_out),
                            crate::world::notifications::Data::None,
                        );
                    }
                    Outcome::Read(backup) => self.pending = Some(Pending::new(world, backup)),
                    Outcome::Saved(path) => {
                        world.notifications.push(
                            crate::world::notifications::Kind::Info,
                            aurora_i18n::tr!("outfit-json-export"),
                            aurora_i18n::tr!("outfit-json-saved", file = path.display().to_string()),
                            crate::world::notifications::Data::None,
                        );
                    }
                    Outcome::Failed(reason) => {
                        world.notifications.push(
                            crate::world::notifications::Kind::Alert,
                            if appearance {
                                aurora_i18n::tr!("appearance-xml-export")
                            } else {
                                aurora_i18n::tr!("outfit-json-title")
                            },
                            reason,
                            crate::world::notifications::Data::None,
                        );
                    }
                    Outcome::Cancelled => {}
                }
            }
        }
        let Some(pending) = &mut self.pending else {
            return Vec::new();
        };
        if pending.generation != world.inventory.generation {
            pending.resolved = pending.backup.resolve(&world.inventory, world.agent_id);
            pending.generation = world.inventory.generation;
        }
        if pending.ready {
            return Vec::new();
        }
        let unresolved = pending.backup.unresolved(&world.inventory);
        if unresolved.is_empty() || Instant::now() >= pending.deadline {
            pending.ready = true;
            return Vec::new();
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        if pending.requested {
            return Vec::new();
        }
        pending.requested = true;
        unresolved
            .chunks(100)
            .map(|items| aurora_net::NetCommand::FetchItems {
                items: items.to_vec(),
                owner: world.agent_id,
            })
            .collect()
    }
}

fn dialog(title: &str) -> rfd::FileDialog {
    rfd::FileDialog::new()
        .set_title(title)
        .add_filter(aurora_i18n::tr!("outfit-json-file-type"), &["json"])
}

fn read(path: &std::path::Path) -> Result<Backup, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(outfit_backup::MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| {
            log::warn!("outfit JSON read failed: {e}");
            aurora_i18n::tr!("outfit-json-failed").to_owned()
        })?;
    Backup::parse(&bytes)
}

pub(super) fn confirmation(ctx: &egui::Context, p: &Palette, world: &World, st: &mut AppearanceUi, actions: &mut Vec<Action>) {
    let Some(mut pending) = st.json.pending.take() else {
        return;
    };
    if pending.scope != scope(world) {
        return;
    }
    let mut close = false;
    let response = widgets::Modal::new(egui::Id::unique("outfit_json_import")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("outfit-json-import"))
                .size(17.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(6.0);
        ui.label(aurora_i18n::tr!("outfit-json-confirm", name = pending.backup.name.as_str()));
        ui.label(aurora_i18n::tr!("outfit-json-name"));
        let response = ui.add(
            egui::TextEdit::singleline(&mut pending.name)
                .desired_width(f32::INFINITY)
                .char_limit(63),
        );
        if pending.focus {
            response.request_focus();
            pending.focus = false;
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("outfit-json-references-only"))
                .color(p.muted)
                .size(11.0),
        );
        let missing = &pending.resolved.missing;
        if !pending.ready {
            ui.label(aurora_i18n::tr!("outfit-json-checking"));
        } else if !missing.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "outfit-json-missing",
                    count = missing.len(),
                    attachments = missing.iter().filter(|it| it.asset_type == 6).count()
                ))
                .color(p.warn),
            );
            egui::ScrollArea::vertical()
                .id_salt("outfit_json_missing")
                .max_height(130.0)
                .show(ui, |ui| {
                    for item in missing {
                        ui.label(&item.name);
                    }
                });
            ui.label(aurora_i18n::tr!("outfit-json-partial-help"));
            if pending.resolved.links.is_empty() {
                ui.label(aurora_i18n::tr!("outfit-json-none-found"));
            }
        } else if pending.resolved.links.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(aurora_i18n::tr!("outfit-json-empty")).color(p.warn));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let valid =
                pending.ready && outfit_backup::valid_name(&pending.name) && st.pending.is_none() && !pending.resolved.links.is_empty();
            let label = if missing.is_empty() {
                aurora_i18n::tr!("outfit-json-create-wear")
            } else {
                aurora_i18n::tr!("outfit-json-import-partial")
            };
            if widgets::action_button_if(
                ui,
                p,
                "shirt-filled",
                label,
                widgets::ActionKind::Primary,
                Vec2::new(0.0, 28.0),
                valid,
            )
            .clicked()
            {
                actions.push(Action::ImportJson {
                    backup: Box::new(pending.backup.clone()),
                    name: pending.name.trim().into(),
                    partial: !missing.is_empty(),
                });
                close = true;
            }
            if widgets::action_button(
                ui,
                p,
                "x",
                aurora_i18n::tr!("common-cancel"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 28.0),
            )
            .clicked()
            {
                close = true;
            }
        });
    });
    if !close && !response.should_close() {
        st.json.pending = Some(pending);
    }
}

pub(crate) fn demo(world: &World, st: &mut AppearanceUi, partial: bool) {
    let Some(cof) = model::cof(&world.inventory) else {
        return;
    };
    if let Ok(mut backup) = Backup::capture(&world.inventory, cof, &world.outfit_attachment_points()) {
        if partial {
            let mut missing = backup.items.iter().find(|it| it.asset_type == 6).cloned();
            if let Some(item) = &mut missing {
                item.item_id = Uuid::from_u128(9_876_543);
            }
            backup.items.extend(missing);
        }
        let mut pending = Pending::new(world, backup);
        pending.ready = true;
        st.json.pending = Some(pending);
    }
}

/// "Appearance saved to …", and how many worn items were left out.
fn saved_message(path: &std::path::Path, left_out: usize) -> String {
    let saved = aurora_i18n::tr!("appearance-xml-saved", file = path.display().to_string());
    if left_out == 0 {
        saved
    } else {
        format!("{saved} {}", aurora_i18n::tr!("appearance-xml-left-out", count = left_out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (egui::Context, World, AppearanceUi) {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        world.inventory.owner = world.agent_id;
        model::seed_demo(&mut world.inventory, world.agent_id);
        let mut st = AppearanceUi::default();
        demo(&world, &mut st, true);
        (ctx, world, st)
    }

    fn frame(
        ctx: &egui::Context,
        world: &World,
        st: &mut AppearanceUi,
        events: Vec<egui::Event>,
    ) -> (Vec<Action>, Vec<egui::epaint::ClippedShape>) {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 800.0))),
                events,
                ..Default::default()
            },
            |ui| confirmation(ui.ctx(), &crate::theme::Theme::default().palette(), world, st, &mut actions),
        );
        output.textures_delta.clear();
        (actions, output.shapes)
    }

    fn click(ctx: &egui::Context, world: &World, st: &mut AppearanceUi, label: &str) -> Vec<Action> {
        let (_, shapes) = frame(ctx, world, st, vec![]);
        let pos = shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos + Vec2::new(4.0, 5.0)),
                _ => None,
            })
            .expect("button text");
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(
                frame(
                    ctx,
                    world,
                    st,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                )
                .0,
            );
        }
        actions
    }

    #[test]
    fn appearance_xml_jobs_handle_cancel_failure_success_and_account_changes_without_inventory_commands() {
        let (ctx, mut world, _) = fixture();
        for outcome in [
            Outcome::Cancelled,
            Outcome::Failed("failure".into()),
            Outcome::AppearanceSaved("appearance.xml".into(), 0),
        ] {
            let (sender, receiver) = std::sync::mpsc::channel();
            assert!(sender.send(outcome).is_ok());
            let mut state = State {
                job: Some(Job {
                    appearance: true,
                    scope: scope(&world),
                    receiver,
                }),
                pending: None,
            };
            let before = world.notifications.list.len();
            assert!(state.poll(&ctx, &mut world).is_empty());
            assert!(!state.busy());
            if world.notifications.list.len() > before {
                assert_eq!(
                    world.notifications.list.last().unwrap().title,
                    aurora_i18n::tr!("appearance-xml-export")
                );
            }
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut state = State {
            job: Some(Job {
                appearance: true,
                scope: scope(&world),
                receiver,
            }),
            pending: None,
        };
        world.agent_id = Uuid::from_u128(2);
        assert!(sender.send(Outcome::AppearanceSaved("appearance.xml".into(), 0)).is_ok());
        let count = world.notifications.list.len();
        assert!(state.poll(&ctx, &mut world).is_empty());
        assert_eq!(world.notifications.list.len(), count);
        assert!(!state.busy());
    }

    #[test]
    fn outfit_json_confirmation_names_missing_attachment_and_requires_explicit_partial_choice() {
        let (ctx, world, mut st) = fixture();
        let missing_name = st.json.pending.as_ref().expect("confirmation").resolved.missing[0].name.clone();
        st.json.pending.as_mut().expect("confirmation").name = "Renamed import".into();
        // egui sizes a new centered modal before painting its content.
        for _ in 0..3 {
            frame(&ctx, &world, &mut st, vec![]);
        }
        let (_, shapes) = frame(&ctx, &world, &mut st, vec![]);
        assert!(
            shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == missing_name))
        );
        let actions = click(&ctx, &world, &mut st, aurora_i18n::tr!("outfit-json-import-partial"));
        assert!(matches!(&actions[..], [Action::ImportJson { name, partial: true, .. }] if name == "Renamed import"));
        assert!(!st.json.has_confirmation());
    }

    #[test]
    fn outfit_json_confirmation_never_offers_to_wear_an_empty_backup() {
        let (ctx, world, mut st) = fixture();
        let pending = st.json.pending.as_mut().expect("confirmation");
        pending.backup.items.clear();
        pending.resolved = pending.backup.resolve(&world.inventory, world.agent_id);
        for _ in 0..3 {
            frame(&ctx, &world, &mut st, vec![]);
        }
        let (_, shapes) = frame(&ctx, &world, &mut st, vec![]);
        let empty = aurora_i18n::tr!("outfit-json-empty");
        assert!(
            shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == empty))
        );
        assert!(click(&ctx, &world, &mut st, aurora_i18n::tr!("outfit-json-create-wear")).is_empty());
        assert!(st.json.has_confirmation());
    }

    #[test]
    fn outfit_json_cancel_and_account_changes_never_modify_inventory() {
        let (ctx, mut world, mut st) = fixture();
        let count = world.inventory.items.len();
        frame(&ctx, &world, &mut st, vec![]);
        assert!(click(&ctx, &world, &mut st, aurora_i18n::tr!("common-cancel")).is_empty());
        assert!(!st.json.busy());
        demo(&world, &mut st, true);
        world.agent_id = Uuid::from_u128(2);
        assert!(st.json.poll(&ctx, &mut world).is_empty());
        assert!(!st.json.busy());
        assert_eq!(world.inventory.items.len(), count);
    }

    #[test]
    fn outfit_json_checks_uncached_originals_before_reporting_missing_and_bounds_wait() {
        let (ctx, mut world, mut st) = fixture();
        let pending = st.json.pending.as_mut().expect("confirmation");
        pending.ready = false;
        let commands = st.json.poll(&ctx, &mut world);
        assert!(
            matches!(&commands[..], [aurora_net::NetCommand::FetchItems { items, owner }] if items == &[Uuid::from_u128(9_876_543)] && *owner == world.agent_id)
        );
        assert!(!st.json.pending.as_ref().expect("pending").ready);
        assert!(st.json.poll(&ctx, &mut world).is_empty(), "no repeated fetch flood");
        st.json.pending.as_mut().expect("pending").deadline = Instant::now();
        assert!(st.json.poll(&ctx, &mut world).is_empty());
        assert!(st.json.pending.as_ref().expect("pending").ready);
        assert_eq!(st.json.pending.as_ref().expect("pending").resolved.missing.len(), 1);
    }
}
