//! Independent quick-import views over shared drafts and asynchronous jobs.
//! Separate type floaters follow llviewermenufile.cpp's upload_image,
//! upload_sound and upload_anim entry points.

use super::*;

pub const TYPES: [Choice; 6] = [
    Choice::Texture,
    Choice::Sound,
    Choice::Animation,
    Choice::Material,
    Choice::Mesh,
    Choice::Local,
];

pub struct Window {
    pub open: bool,
    pub active: Option<Uuid>,
    pub tab: ui::Tab,
    pub filters: [bool; 5],
    pub(super) destination: ui::destination::Picker,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            open: false,
            active: None,
            tab: Default::default(),
            filters: [true; 5],
            destination: Default::default(),
        }
    }
}

impl Choice {
    pub fn window_index(self) -> Option<usize> {
        if self == Self::Local { Some(5) } else { Imports::type_index(self) }
    }
}

impl Imports {
    pub fn open_type_window(&mut self, ctx: &egui::Context, choice: Choice) {
        let Some(index) = choice.window_index() else {
            self.open_window(ctx);
            return;
        };
        crate::ui::widgets::open_floater(ctx, choice.window_id(), &mut self.windows[index].open);
        self.reconcile_window(choice);
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(choice.window_id())));
    }

    pub fn window_open(&self, choice: Choice) -> bool {
        choice.window_index().map_or(self.open, |index| self.windows[index].open)
    }

    pub fn any_window_open(&self) -> bool {
        self.open || self.windows.iter().any(|window| window.open)
    }

    pub fn active_ids(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.open
            .then_some(self.active)
            .flatten()
            .into_iter()
            .chain(self.windows.iter().filter(|window| window.open).filter_map(|window| window.active))
    }

    pub fn entry_visible(&self, id: Uuid) -> bool {
        self.active_ids().any(|active| active == id)
    }

    pub(super) fn reconcile_window(&mut self, choice: Choice) {
        let Some(index) = choice.window_index() else { return };
        let window = &mut self.windows[index];
        if window
            .active
            .is_none_or(|id| !self.entries.iter().any(|entry| entry.id == id && choice.accepts(entry)))
        {
            window.active = self.entries.iter().find(|entry| choice.accepts(entry)).map(|entry| entry.id);
        }
    }

    /// View-local state is swapped, while edits keep the original draft/worker identity.
    pub fn with_window<R>(&mut self, choice: Choice, body: impl FnOnce(&mut Self) -> R) -> R {
        let Some(index) = choice.window_index() else { return body(self) };
        let saved = (self.open, self.active, self.tab, self.type_filters, self.choice, self.window_scope);
        let destination = std::mem::replace(&mut self.destination, std::mem::take(&mut self.windows[index].destination));
        self.open = self.windows[index].open;
        self.active = self.windows[index].active;
        self.tab = self.windows[index].tab;
        self.type_filters = self.windows[index].filters;
        self.choice = choice;
        self.window_scope = Some(choice);
        self.reconcile_active();
        let result = body(self);
        let window_destination = std::mem::replace(&mut self.destination, destination);
        self.windows[index] = Window {
            open: self.open,
            active: self.active,
            tab: self.tab,
            filters: self.type_filters,
            destination: window_destination,
        };
        (self.open, self.active, self.tab, self.type_filters, self.choice, self.window_scope) = saved;
        result
    }

    pub fn with_choice<R>(&mut self, choice: Choice, body: impl FnOnce(&mut Self) -> R) -> R {
        let saved = std::mem::replace(&mut self.choice, choice);
        let result = body(self);
        self.choice = saved;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_windows_keep_their_selection_tabs_and_geometry_without_copying_drafts() {
        let ctx = egui::Context::default();
        let first = Entry::new(
            "first.png".into(),
            Ok(Source::Texture(super::super::tests::image(4, 4))),
            Default::default(),
        );
        let second = Entry::new(
            "second.png".into(),
            Ok(Source::Texture(super::super::tests::image(4, 4))),
            Default::default(),
        );
        let mesh = Entry::new(
            "mesh.dae".into(),
            Ok(Source::Mesh(Box::new(super::super::tests::mesh_fixture()))),
            Default::default(),
        );
        let (first_id, second_id, mesh_id) = (first.id, second.id, mesh.id);
        let mut imports = Imports {
            open: true,
            active: Some(first_id),
            entries: vec![first, second, mesh],
            ..Default::default()
        };
        imports.open_type_window(&ctx, Choice::Texture);
        imports.open_type_window(&ctx, Choice::Mesh);
        imports.with_window(Choice::Texture, |imports| {
            assert_eq!(imports.visible_indices(), vec![0, 1]);
            imports.active = Some(second_id);
            imports.tab = ui::Tab::Permissions;
            imports.entries[1].name = "Edited texture".into();
        });
        imports.with_window(Choice::Mesh, |imports| {
            assert_eq!(imports.active, Some(mesh_id));
            assert_eq!(imports.tab, ui::Tab::Batch);
            assert_eq!(imports.visible_indices(), vec![2]);
            imports.open = false;
        });
        imports.with_window(Choice::Texture, |imports| {
            assert_eq!(imports.active, Some(second_id));
            assert_eq!(imports.tab, ui::Tab::Permissions);
            assert_eq!(imports.entries[1].name, "Edited texture");
            assert!(imports.open);
        });
        assert_eq!(imports.active, Some(first_id));
        assert_eq!(imports.tab, ui::Tab::Batch);
        assert!(imports.open && imports.window_open(Choice::Texture));
        assert!(!imports.window_open(Choice::Mesh));
        assert_eq!(imports.entries.len(), 3);
        assert_eq!(imports.active_ids().collect::<HashSet<_>>(), HashSet::from([first_id, second_id]));
    }

    #[test]
    fn late_file_results_and_errors_stay_with_the_type_that_requested_the_picker() {
        let ctx = egui::Context::default();
        let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let (jobs, _results) = crate::scene::jobs::Jobs::new();
        let mut imports = Imports::default();
        imports.open_type_window(&ctx, Choice::Texture);
        let request = Uuid::new_v4();
        imports.pending_files = Some(request);
        imports.pending_file_choice = Some(Choice::Texture);
        imports.open_type_window(&ctx, Choice::Sound);
        let valid = Entry::new(
            "valid.png".into(),
            Ok(Source::Texture(super::super::tests::image(4, 4))),
            Default::default(),
        );
        let texture = valid.id;
        let failed = Entry::new("broken.png".into(), Err("invalid image".into()), Default::default());
        imports.tx.send(Worker::Loaded(request, vec![valid, failed])).unwrap();
        imports.poll(&ctx, &jobs, &world, &Default::default());
        assert!(imports.pending_files.is_none() && imports.pending_file_choice.is_none());
        imports.with_window(Choice::Texture, |imports| {
            assert_eq!(imports.active, Some(texture));
            assert_eq!(imports.visible_indices(), vec![0, 1]);
        });
        imports.with_window(Choice::Sound, |imports| {
            assert!(imports.visible_indices().is_empty());
            assert!(imports.active.is_none());
        });
        assert_eq!(imports.visible_indices(), vec![0, 1], "the unified queue shares both drafts");
    }
}
