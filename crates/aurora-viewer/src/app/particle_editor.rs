//! Build › Edit particles on the app's side: the editor opened on the
//! selection, its target picker, Copy and Inject. Port of
//! LLEditParticleSource / LLEnableEditParticleSource
//! (indra/newview/llviewermenu.cpp) and ParticleEditor::onCopyButtonClicked,
//! onInjectButtonClicked, createScriptInventoryItem, callbackReturned and
//! scriptInjectReturned (indra/newview/particleeditor.cpp), originally
//! LGPL 2.1.
//!
//! Inject, as the Legacy Viewer: a "New Particle Script" item is created in
//! the viewer's own folder (#Aurora, created when missing, else the Scripts
//! folder), its source uploaded (UpdateScriptAgent, Mono), then the item put
//! into the prim, running (RezScript, LLViewerObject::saveScript). The copy
//! stays in the inventory.

use super::App;
use crate::build::particle_editor::script_name;
use crate::build::ui::particles::{Action, Notice};
use aurora_net::NetCommand;
use aurora_net::inventory::InvItem;
use aurora_net::inventory::operations::{NewItem, Operation, Reply};
use aurora_net::scripts::{ScriptOptions, ScriptSaved, ScriptTarget};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The viewer's folder at the root (ROOT_FIRESTORM_FOLDER).
const FOLDER: &str = "#Aurora";
/// FT_LSL_TEXT: the Scripts system folder, when ours cannot be made.
const FT_LSL_TEXT: i32 = 10;
/// AT_LSL_TEXT.
const AT_LSL_TEXT: i32 = 10;
/// An answer that never comes gives the window back.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Where an Inject stands.
#[derive(Default)]
pub(super) struct Inject {
    pending: Option<Pending>,
}

struct Pending {
    object: Uuid,
    source: String,
    step: Step,
    since: Instant,
}

enum Step {
    /// #Aurora is being created (createNewCategory); `asked` is the id we
    /// gave it, the server may give another.
    Folder { request: Uuid, asked: Uuid },
    /// The item is being created in `folder` (create_inventory_item).
    Item { folder: Uuid },
    /// Its source is being uploaded (LLScriptAssetUpload).
    Upload { request: Uuid, item: Box<InvItem> },
}

impl Inject {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// The inventory change or the upload is Inject's.
    pub(super) fn owns(&self, request: Uuid) -> bool {
        matches!(
            self.pending.as_ref().map(|p| &p.step),
            Some(Step::Folder { request: r, .. } | Step::Upload { request: r, .. }) if *r == request
        )
    }
}

impl App {
    /// The prim the editor opens on: the primary selected object, when an
    /// object of the selection is ours (LLEnableEditParticleSource).
    pub(super) fn particle_editor_target(&self) -> Option<usize> {
        let owned = self.build.selection.iter().any(|k| {
            self.world
                .objects
                .index_of(k)
                .and_then(|i| self.world.objects.get(i))
                .is_some_and(|o| crate::build::task_inventory::owned(&self.world, o.full_id, self.build.props.get(&o.full_id)))
        });
        if !owned {
            return None;
        }
        self.build.selection.first().and_then(|k| self.world.objects.index_of(k))
    }

    /// Build › Edit particles (LLEditParticleSource).
    pub(super) fn open_particle_editor(&mut self) {
        let Some(o) = self.particle_editor_target().and_then(|i| self.world.objects.get(i)) else {
            return;
        };
        let id = o.full_id;
        let name = self.build.props.get(&id).map(|p| p.name.clone()).unwrap_or_default();
        self.particle_editor.open_on(id, name);
        crate::ui::widgets::open_floater(&self.egui_ctx, "particle_editor", &mut self.particle_editor.open);
    }

    pub(super) fn particle_editor_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.particle_inject.pending.as_ref().is_some_and(|p| p.since.elapsed() > TIMEOUT) {
            log::warn!("particle editor: inject got no answer");
            self.particle_inject_failed(aurora_i18n::tr!("particle-editor-inject-timeout").to_owned());
        }
        self.particle_editor.busy = self.particle_inject.busy();
        if self.particle_editor.picking && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.particle_editor.picking = false;
        }
        let action = self.particle_editor.show(ctx, p, &self.ui_images);
        if let Some(texture) = self.particle_editor.picker.show_live(ctx, p, &mut self.world, &self.ui_images) {
            self.particle_editor.params.texture = texture;
        }
        match action {
            Some(Action::Copy) => {
                let editor = &mut self.particle_editor;
                ctx.copy_text(editor.params.script(editor.object));
                editor.notice = Some(Notice {
                    text: aurora_i18n::tr!("particle-editor-copied").into(),
                    error: false,
                });
            }
            Some(Action::Inject) => self.particle_inject_start(),
            Some(Action::Pick) => self.particle_editor.picking = true,
            Some(Action::CancelPick) => self.particle_editor.picking = false,
            None => {}
        }
    }

    /// The target picker's click (onTargetPicked). Nothing yet while the
    /// depth under the cursor is asked for (the press is run again with it).
    pub(super) fn particle_target_pick(&mut self) {
        let picked = self.menu_pick();
        if picked.is_none() && self.clicks.waiting() {
            return;
        }
        self.particle_editor.picking = false;
        if let Some((Some(idx), _)) = picked
            && let Some(id) = self.world.objects.get(idx).map(|o| o.full_id)
        {
            self.particle_editor.set_target(id);
        }
    }

    /// onInjectButtonClicked: our folder, made when missing.
    fn particle_inject_start(&mut self) {
        if self.particle_inject.busy() {
            return;
        }
        let object = self.particle_editor.object;
        let pending = |step| Pending {
            object,
            source: self.particle_editor.params.script(object),
            step,
            since: Instant::now(),
        };
        let inv = &self.world.inventory;
        if let Some(folder) = crate::ao::inventory::child(inv, inv.root, FOLDER) {
            self.particle_inject.pending = Some(pending(Step::Item { folder }));
            self.particle_create_script(folder);
            return;
        }
        let change = if inv.root.is_nil() {
            Err(String::new())
        } else {
            crate::world::inventory::actions::create_folder(inv.root, FOLDER)
        };
        match change {
            Ok(change) => {
                let Some(Operation::Folder { id: asked, .. }) = change.operations.first() else {
                    return;
                };
                let request = Uuid::new_v4();
                self.particle_inject.pending = Some(pending(Step::Folder { request, asked: *asked }));
                self.send(NetCommand::EditInventory { request, change });
            }
            Err(reason) => {
                log::warn!("particle editor: no folder for the script: {reason}");
                self.particle_inject.pending = Some(pending(Step::Item { folder: Uuid::nil() }));
                self.particle_create_script(Uuid::nil());
            }
        }
    }

    /// The answer to the folder's creation.
    pub(super) fn particle_folder_created(&mut self, request: Uuid, result: Result<Reply, String>) {
        let Some(Step::Folder { asked, .. }) = self.particle_inject.pending.as_ref().map(|p| &p.step) else {
            return;
        };
        let asked = *asked;
        log::debug!("particle editor: folder {request} answered");
        let folder = match result {
            Ok(reply) => {
                let folder = reply.created.get(&asked).copied().unwrap_or(asked);
                self.world.inventory.remove(&reply.removed);
                self.world.inventory.apply(reply.contents);
                self.world.inventory.created_folders(reply.folders);
                folder
            }
            Err(reason) => {
                log::warn!("particle editor: {FOLDER} not created: {reason}");
                Uuid::nil()
            }
        };
        self.particle_create_script(folder);
    }

    /// createScriptInventoryItem: without our folder, the Scripts folder;
    /// without either, Inject stops (ParticleScriptFindFolderFailed).
    fn particle_create_script(&mut self, folder: Uuid) {
        let folder = Some(folder)
            .filter(|f| !f.is_nil())
            .or_else(|| crate::world::appearance::system_folder(&self.world.inventory, FT_LSL_TEXT));
        let Some(folder) = folder else {
            self.particle_inject_failed(aurora_i18n::tr!("particle-editor-folder-failed").to_owned());
            return;
        };
        if let Some(p) = &mut self.particle_inject.pending {
            p.step = Step::Item { folder };
        }
        self.send(NetCommand::CreateInventoryItem {
            parent: folder,
            kind: NewItem::Script,
            name: script_name().into(),
            permissions: None,
        });
    }

    /// callbackReturned: the item is there, its source goes up. Returns the
    /// item when it is not Inject's.
    pub(super) fn particle_script_created(&mut self, item: InvItem) -> Option<InvItem> {
        let ours = matches!(
            self.particle_inject.pending.as_ref().map(|p| &p.step),
            Some(Step::Item { folder }) if *folder == item.parent && item.asset_type == AT_LSL_TEXT && item.name == script_name()
        );
        if !ours {
            return Some(item);
        }
        self.world.inventory.add_items(vec![item.clone()]);
        let request = Uuid::new_v4();
        let Some(p) = &mut self.particle_inject.pending else {
            return None;
        };
        let source = p.source.clone();
        p.step = Step::Upload {
            request,
            item: Box::new(item.clone()),
        };
        self.send(NetCommand::SaveScript {
            request,
            target: ScriptTarget {
                object: Uuid::nil(),
                item: item.id,
            },
            handle: self.world.main_region.unwrap_or_default(),
            options: ScriptOptions {
                mono: true,
                running: true,
                experience: Uuid::nil(),
            },
            source,
        });
        None
    }

    /// scriptInjectReturned: the item goes into the prim, running.
    pub(super) fn particle_script_uploaded(&mut self, request: Uuid, result: Result<ScriptSaved, String>) {
        let Some(p) = self.particle_inject.pending.take() else { return };
        let Step::Upload { item, .. } = p.step else { return };
        let saved = match result {
            Ok(saved) => saved,
            Err(reason) => {
                log::warn!("particle editor: upload {request} failed: {reason}");
                self.particle_inject_failed(aurora_i18n::tr!("particle-editor-upload-failed", reason = reason));
                return;
            }
        };
        let mut item = *item;
        item.asset_id = saved.asset;
        self.world.inventory.add_items(vec![item.clone()]);
        // play it safe, because some time may have passed
        let drop = self
            .world
            .objects
            .index_of_uuid(&p.object)
            .and_then(|idx| crate::build::contents_drop::item_drop(&self.world, idx, &item, true));
        let Some(drop) = drop else {
            log::warn!("particle editor: the object went away before the script went in");
            self.particle_inject_failed(aurora_i18n::tr!("particle-editor-object-gone").to_owned());
            return;
        };
        self.build.send(drop.command);
        self.flush_build();
        self.build.contents_changed(p.object);
        self.particle_editor.busy = false;
        self.particle_editor.notice = Some(Notice {
            text: aurora_i18n::tr!("particle-editor-injected").into(),
            error: false,
        });
    }

    fn particle_inject_failed(&mut self, text: String) {
        self.particle_inject.pending = None;
        self.particle_editor.busy = false;
        self.particle_editor.notice = Some(Notice { text, error: true });
    }
}

impl App {
    /// AURORA_DEMO_PARTICLE_EDITOR=1|fountain|inject, with an object of ours
    /// selected by AURORA_DEMO_BUILD (`move,object:<local id>`): the editor
    /// opens on it at frame 700; `fountain` tunes a coloured cone, `inject`
    /// also injects it at frame 760.
    pub(super) fn particle_editor_demo(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_PARTICLE_EDITOR") else {
            return;
        };
        match self.frame_count {
            700 => {
                self.open_particle_editor();
                if mode != "1" {
                    self.particle_editor.params = crate::build::particle_editor::demo_fountain();
                }
            }
            760 if mode == "inject" && self.particle_editor.open => self.particle_inject_start(),
            _ => {}
        }
    }
}
