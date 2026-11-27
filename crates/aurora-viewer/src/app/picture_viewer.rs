//! The Picture viewer driven (`ui/picture_viewer.rs`; Aurora's own): the
//! picture opened and its edits played on worker threads, never on the
//! interface's; the screen works on a copy of at most `SCREEN_SIDE` pixels
//! and the save plays the same edits on the full picture. The looks are
//! drawn by the GPU (`aurora_render::picture_look`): on screen, on the 3D
//! card (a copy read back), and on the picture saved.

use super::*;
use crate::picture::{self, Edit, History, Picture, Recipe};
use crate::snapshot::looks::{self, Layer, Placement};
use crate::ui::picture_viewer::{Action, PictureViewerUi};
use aurora_render::picture_look::{self, GradedImage, Read};
use aurora_render::preview::{Image, Model};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

/// The side of the copy the window works on live.
const SCREEN_SIDE: u32 = 2048;

/// What a read of the looks back is for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ReadFor {
    /// The 3D card's picture, of the screen's copy at `generation`.
    Card(u64),
    /// The picture being saved.
    Save,
}

#[derive(Default)]
pub(crate) struct PictureViewer {
    pub ui: PictureViewerUi,
    path: Option<PathBuf>,
    name: String,
    /// The picture as opened, full size, and its copy for the screen.
    original: Option<Arc<Picture>>,
    screen: Option<Arc<Picture>>,
    recipe: Recipe,
    history: History,
    /// The recipe when a slider's drag began (its step of the history).
    adjust_from: Option<Recipe>,
    /// The screen's copy with the recipe played, its texture, and which
    /// recipe it shows (`generation`).
    shown: Option<Arc<Picture>>,
    shown_image: Option<Arc<Image>>,
    texture: Option<egui::TextureHandle>,
    generation: u64,
    working: Option<Receiver<(u64, Picture)>>,
    /// The looks of this window (not the photo window's), their settings.
    pub looks: Vec<Layer>,
    pub look_settings: looks::Settings,
    /// A LUT imported from this window goes into its looks.
    pub importing: bool,
    /// The 3D card, the relief and picture it was made with.
    model: Option<Arc<Model>>,
    model_key: Option<(u64, u32, bool)>,
    model_rx: Option<Receiver<((u64, u32, bool), Model)>>,
    /// The screen's copy through the looks, for the card.
    card_picture: Option<(u64, Arc<Picture>)>,
    relief: f32,
    /// A read of the looks to ask for in the next paint, and the answers.
    read: Option<Arc<Read>>,
    reads: Vec<(u64, ReadFor)>,
    read_tag: u64,
    read_tx: Option<Sender<Result<GradedImage, String>>>,
    read_rx: Option<Receiver<Result<GradedImage, String>>>,
    /// The file dialogs' answers.
    open_rx: Option<Receiver<Option<PathBuf>>>,
    save_rx: Option<Receiver<Option<PathBuf>>>,
    loading: Option<Receiver<Result<(PathBuf, Picture), String>>>,
    /// Where the picture is being saved, once chosen.
    save_to: Option<PathBuf>,
    exporting: Option<Receiver<Picture>>,
    saving: Option<crate::snapshot::save::Saving>,
    status: Option<(String, bool)>,
}

impl PictureViewer {
    fn reader(&mut self) -> Sender<Result<GradedImage, String>> {
        if self.read_tx.is_none() {
            let (tx, rx) = channel();
            self.read_tx = Some(tx);
            self.read_rx = Some(rx);
        }
        self.read_tx.clone().expect("made above")
    }

    /// Asks for `image` drawn through the looks, read back for `what`.
    fn ask_read(&mut self, image: Arc<Image>, what: ReadFor) {
        self.read_tag += 1;
        let tag = self.read_tag;
        let reply = self.reader();
        self.reads.push((tag, what));
        self.read = Some(Arc::new(Read { tag, image, reply }));
    }

    fn has_looks(&self) -> bool {
        self.looks.iter().any(|l| l.on)
    }

    /// Plays the recipe on the screen's copy on a worker; the latest asked
    /// wins.
    fn rework(&mut self) {
        let Some(screen) = self.screen.clone() else { return };
        self.generation += 1;
        let generation = self.generation;
        let recipe = self.screen_recipe();
        let (tx, rx) = channel();
        self.working = Some(rx);
        let spawned = std::thread::Builder::new().name("aurora-picture-edit".into()).spawn(move || {
            let _ = tx.send((generation, picture::render(&screen, &recipe)));
        });
        if let Err(e) = spawned {
            log::warn!("picture viewer: no worker thread ({e})");
        }
    }

    /// The recipe for the screen's copy: the same edits, the resizes made
    /// relative so that the copy keeps working at the screen's size.
    fn screen_recipe(&self) -> Recipe {
        let mut recipe = self.recipe.clone();
        // (a resize of the full picture would make the screen's copy blurry
        // or huge: the screen shows it at its own size, the save makes it)
        recipe.edits.retain(|e| !matches!(e, Edit::Resize { .. }));
        recipe
    }

    /// A change of the recipe, remembered for undo.
    fn change(&mut self, f: impl FnOnce(&mut Recipe)) {
        let before = self.recipe.clone();
        f(&mut self.recipe);
        if self.recipe != before {
            self.history.push(before);
            self.rework();
        }
    }

    fn set_recipe(&mut self, recipe: Recipe) {
        self.adjust_from = None;
        self.ui.adjust = recipe.adjust;
        self.recipe = recipe;
        self.rework();
    }

    /// The size the picture is saved at.
    fn full_size(&self) -> (u32, u32) {
        self.original
            .as_ref()
            .map_or((0, 0), |o| picture::geometry::size_after(o.w, o.h, &self.recipe.edits))
    }

    fn opened(&mut self, path: PathBuf, pic: Picture) {
        let pic = Arc::new(pic);
        self.screen = Some(Arc::new(picture::io::working_copy(&pic, SCREEN_SIDE)));
        self.original = Some(pic);
        self.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.path = Some(path);
        self.recipe = Recipe::default();
        self.history = History::default();
        self.ui.adjust = Default::default();
        self.ui.view = Default::default();
        self.ui.crop = None;
        self.ui.view3d = None;
        self.shown = None;
        self.texture = None;
        self.model = None;
        self.model_key = None;
        self.card_picture = None;
        self.status = None;
        self.rework();
    }
}

impl App {
    /// The Picture viewer, while it is open.
    pub(super) fn picture_viewer_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.panels.picture_viewer {
            return;
        }
        self.picture_viewer_poll(ctx);
        let resolved = if self.picture_viewer.has_looks() {
            let (stack, settings) = (self.picture_viewer.looks.clone(), self.picture_viewer.look_settings.clone());
            looks::resolve_stack(&stack, &settings, self.photo_luts())
        } else {
            Vec::new()
        };
        let pv = &mut self.picture_viewer;
        let graded = pv.shown_image.clone().filter(|_| !resolved.is_empty()).and_then(|image| {
            let place = Placement {
                origin: [0.0, 0.0],
                photo: (image.size[0], image.size[1]),
                scale: 1.0,
            };
            looks::frame_look(&resolved, place, None, false, None).map(|look| picture_look::Graded {
                id: egui::Id::unique("picture_viewer_graded"),
                image,
                look,
                read: pv.read.take(),
            })
        });
        // a read asked while no look is on (they were all switched off):
        // nothing will answer it
        if graded.is_none()
            && let Some(read) = pv.read.take()
        {
            let _ = read.reply.send(Err("no look".into()));
        }
        let busy = if let Some(saving) = &pv.saving {
            Some((saving.fraction(), aurora_i18n::tr!("picture-viewer-saving").to_string()))
        } else if pv.exporting.is_some() || pv.save_to.is_some() {
            Some((0.0, aurora_i18n::tr!("picture-viewer-saving").to_string()))
        } else if pv.loading.is_some() {
            Some((0.0, aurora_i18n::tr!("picture-viewer-loading").to_string()))
        } else {
            None
        };
        let model = if pv.ui.mode3d { pv.model.clone() } else { None };
        let full = pv.full_size();
        let size = pv.shown.as_ref().map_or((1, 1), |s| (s.w, s.h));
        let mut open = true;
        let mut actions = Vec::new();
        let shown = crate::ui::picture_viewer::Shown {
            name: pv.path.as_ref().map(|_| pv.name.as_str()),
            texture: pv.texture.as_ref(),
            size,
            full,
            model,
            graded,
            can_undo: !pv.history.done.is_empty(),
            can_redo: !pv.history.undone.is_empty(),
            busy,
            status: pv.status.as_ref().map(|(s, e)| (s.as_str(), *e)),
            luts: &self.snapshots.ui.lut_names,
            thumbs: &self.snapshots.ui.thumbs,
        };
        let stack = crate::ui::looks_editor::Stack {
            looks: &mut pv.looks,
            look_settings: &mut pv.look_settings,
            favorites: &mut self.settings.snapshot.look_favorites,
        };
        crate::ui::picture_viewer::show(ctx, p, &mut open, &mut pv.ui, shown, stack, &mut actions);
        self.panels.picture_viewer = open;
        for action in actions {
            self.picture_viewer_action(action);
        }
        if self.picture_viewer.saving.is_some() || self.picture_viewer.working.is_some() || self.picture_viewer.loading.is_some() {
            ctx.request_repaint();
        }
    }

    fn picture_viewer_action(&mut self, action: Action) {
        let pv = &mut self.picture_viewer;
        match action {
            Action::Open => {
                if pv.open_rx.is_some() {
                    return;
                }
                let (tx, rx) = channel();
                pv.open_rx = Some(rx);
                std::thread::spawn(move || {
                    let picked = rfd::FileDialog::new()
                        .set_title(aurora_i18n::tr!("picture-viewer-open"))
                        .add_filter(aurora_i18n::tr!("picture-viewer-files"), picture::io::OPEN_EXTENSIONS)
                        .pick_file();
                    let _ = tx.send(picked);
                });
            }
            Action::Save => {
                if pv.save_rx.is_some() || pv.original.is_none() {
                    return;
                }
                let (tx, rx) = channel();
                pv.save_rx = Some(rx);
                let name = pv
                    .path
                    .as_deref()
                    .map_or_else(|| "picture-edited.png".into(), picture::io::suggested_name);
                let dir = pv.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
                std::thread::spawn(move || {
                    let mut dialog = rfd::FileDialog::new()
                        .set_title(aurora_i18n::tr!("picture-viewer-save"))
                        .set_file_name(name)
                        .add_filter("PNG", &["png"])
                        .add_filter("JPEG", &["jpg", "jpeg"])
                        .add_filter("BMP", &["bmp"]);
                    if let Some(dir) = dir {
                        dialog = dialog.set_directory(dir);
                    }
                    let _ = tx.send(dialog.save_file());
                });
            }
            Action::Overwrite(yes) => {
                if let Some(path) = pv.save_to.take().filter(|_| yes) {
                    self.picture_viewer_export(path);
                }
            }
            Action::Undo => {
                if let Some(previous) = pv.history.undo(&pv.recipe) {
                    pv.set_recipe(previous);
                }
            }
            Action::Redo => {
                if let Some(next) = pv.history.redo(&pv.recipe) {
                    pv.set_recipe(next);
                }
            }
            Action::Reset => {
                pv.change(|r| *r = Recipe::default());
                pv.ui.adjust = Default::default();
            }
            Action::Edit(edit) => {
                let (w, h) = pv.full_size();
                if let Edit::Resize { sx, sy } = edit
                    && picture::geometry::resized(w, h, sx, sy) == (w, h)
                {
                    return;
                }
                pv.change(|r| r.edits.push(edit));
                if matches!(edit, Edit::Crop(_) | Edit::RotateLeft | Edit::RotateRight) {
                    pv.ui.view = Default::default();
                }
            }
            Action::Adjust { adjust, done } => {
                // one step of the history for a whole drag, from where it began
                let before = pv.adjust_from.take().unwrap_or_else(|| pv.recipe.clone());
                pv.recipe.adjust = adjust.clamped();
                if done {
                    if pv.recipe != before {
                        pv.history.push(before);
                    }
                } else {
                    pv.adjust_from = Some(before);
                }
                pv.rework();
            }
            Action::Relief { relief, done } => {
                pv.ui.relief = relief;
                // the card is made again once the slider is let go
                if done {
                    pv.relief = relief;
                }
            }
            Action::Looks => {
                pv.card_picture = None;
                pv.model_key = None;
            }
            Action::LooksEditor(editor_action) => {
                if matches!(editor_action, crate::ui::snapshot::Action::ImportLut) && self.snapshots.lut_import.is_none() {
                    self.picture_viewer.importing = true;
                }
                self.photo_lut_action(editor_action, false);
            }
        }
    }

    /// The pictures, edits, reads and saves that came back.
    fn picture_viewer_poll(&mut self, ctx: &egui::Context) {
        let pv = &mut self.picture_viewer;
        if let Some(rx) = &pv.open_rx
            && let Ok(picked) = rx.try_recv()
        {
            pv.open_rx = None;
            if let Some(path) = picked {
                let (tx, rx) = channel();
                pv.loading = Some(rx);
                std::thread::spawn(move || {
                    let _ = tx.send(picture::io::load(&path).map(|pic| (path, pic)));
                });
            }
        }
        if let Some(rx) = &pv.loading
            && let Ok(result) = rx.try_recv()
        {
            pv.loading = None;
            match result {
                Ok((path, pic)) => {
                    log::info!("picture viewer: opened {} ({}x{})", path.display(), pic.w, pic.h);
                    pv.opened(path, pic);
                }
                Err(e) => {
                    log::warn!("picture viewer: cannot open the picture: {e}");
                    pv.status = Some((aurora_i18n::tr!("picture-viewer-error-open", error = e).to_string(), true));
                }
            }
        }
        if let Some(rx) = &pv.working {
            // the latest answer only
            let mut latest = None;
            while let Ok(answer) = rx.try_recv() {
                latest = Some(answer);
            }
            if let Some((generation, pic)) = latest
                && generation == pv.generation
            {
                pv.working = None;
                let pic = Arc::new(pic);
                let image = egui::ColorImage::from_rgba_unmultiplied([pic.w as usize, pic.h as usize], &pic.rgba);
                match &mut pv.texture {
                    Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                    None => pv.texture = Some(ctx.load_texture("picture_viewer", image, egui::TextureOptions::LINEAR)),
                }
                pv.shown_image = Some(Arc::new(Image {
                    size: [pic.w, pic.h],
                    rgba: pic.rgba.clone(),
                }));
                pv.shown = Some(pic);
                pv.card_picture = None;
            }
        }
        // the 3D card: made again when the picture, its looks or the relief
        // changed (the relief once let go)
        if pv.ui.mode3d
            && let Some(shown) = pv.shown.clone()
        {
            let looks = pv.has_looks();
            let source = if looks {
                match &pv.card_picture {
                    Some((g, pic)) if *g == pv.generation => Some(pic.clone()),
                    _ => {
                        if !pv.reads.iter().any(|(_, w)| *w == ReadFor::Card(pv.generation))
                            && let Some(image) = pv.shown_image.clone()
                        {
                            pv.ask_read(image, ReadFor::Card(pv.generation));
                        }
                        None
                    }
                }
            } else {
                Some(shown)
            };
            let key = (pv.generation, pv.relief.to_bits(), looks);
            if let Some(source) = source
                && pv.model_key != Some(key)
                && pv.model_rx.is_none()
            {
                pv.model_key = Some(key);
                let relief = pv.relief;
                let (tx, rx) = channel();
                pv.model_rx = Some(rx);
                std::thread::spawn(move || {
                    let _ = tx.send((key, picture::relief::card(&source, relief)));
                });
            }
        }
        if let Some(rx) = &pv.model_rx
            && let Ok((key, model)) = rx.try_recv()
        {
            pv.model_rx = None;
            if pv.model_key == Some(key) {
                pv.model = Some(Arc::new(model));
            } else {
                // made for an older picture or relief: the next one is asked
                pv.model_key = None;
            }
            ctx.request_repaint();
        }
        // the looks read back
        let answers: Vec<_> = pv.read_rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for answer in answers {
            match answer {
                Ok(graded) => {
                    let Some(index) = pv.reads.iter().position(|(tag, _)| *tag == graded.tag) else {
                        continue;
                    };
                    let (_, what) = pv.reads.remove(index);
                    let pic = Picture::new(graded.width, graded.height, graded.rgba);
                    match what {
                        ReadFor::Card(generation) => pv.card_picture = Some((generation, Arc::new(pic))),
                        ReadFor::Save => {
                            if let Some(path) = pv.save_to.take() {
                                picture_viewer_save(pv, path, pic);
                            }
                        }
                    }
                }
                Err(e) => {
                    // the oldest read in flight failed
                    if let Some((_, what)) = (!pv.reads.is_empty()).then(|| pv.reads.remove(0))
                        && what == ReadFor::Save
                    {
                        pv.save_to = None;
                        pv.status = Some((aurora_i18n::tr!("picture-viewer-error-save", error = e).to_string(), true));
                    }
                }
            }
        }
        // the save chosen
        if let Some(rx) = &pv.save_rx
            && let Ok(picked) = rx.try_recv()
        {
            pv.save_rx = None;
            if let Some(path) = picked {
                let (path, _) = picture::io::save_target(&path);
                let original = pv.path.as_deref().is_some_and(|o| picture::io::same_file(o, &path));
                if original {
                    pv.ui.confirm_overwrite = Some(pv.name.clone());
                    pv.save_to = Some(path);
                } else {
                    self.picture_viewer_export(path);
                }
            }
        }
        let pv = &mut self.picture_viewer;
        if let Some(rx) = &pv.exporting
            && let Ok(pic) = rx.try_recv()
        {
            pv.exporting = None;
            if let Some(path) = pv.save_to.clone() {
                if pv.has_looks() {
                    // drawn through the looks on the GPU, at most its size
                    let pic = picture::io::working_copy(&pic, picture_look::MAX_SIDE);
                    let image = Arc::new(Image {
                        size: [pic.w, pic.h],
                        rgba: pic.rgba.clone(),
                    });
                    pv.ask_read(image, ReadFor::Save);
                } else {
                    pv.save_to = None;
                    picture_viewer_save(pv, path, pic);
                }
            }
        }
        if let Some(result) = pv.saving.as_ref().and_then(|s| s.poll()) {
            pv.saving = None;
            pv.status = Some(match result {
                Ok(saved) => {
                    let name = saved.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    log::info!("picture viewer: saved {} ({} bytes)", saved.path.display(), saved.bytes);
                    (aurora_i18n::tr!("picture-viewer-saved", name = name).to_string(), false)
                }
                Err(e) => {
                    log::warn!("picture viewer: not saved: {e}");
                    (aurora_i18n::tr!("picture-viewer-error-save", error = e).to_string(), true)
                }
            });
        }
    }

    /// Plays the edits on the full picture (a worker), then saves it to
    /// `path` (through the looks first when there are some).
    fn picture_viewer_export(&mut self, path: PathBuf) {
        let pv = &mut self.picture_viewer;
        let Some(original) = pv.original.clone() else { return };
        pv.save_to = Some(path);
        pv.status = None;
        let recipe = pv.recipe.clone();
        let (tx, rx) = channel();
        pv.exporting = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(picture::render(&original, &recipe));
        });
    }

    /// AURORA_DEMO_PICTURE_VIEWER=1[:<option>,…]: at frame 600 the Picture
    /// viewer opens with the demo's test picture (made by the viewer), or a
    /// picture of the computer (`file=<path>`). Options: `3d` (as a card),
    /// `relief=<0..1>`, `look=<id>[+<id>…]` (the photo looks' ids),
    /// `rotate`, `flip`, `crop=<w>:<h>` (the middle at that ratio),
    /// `bright=<-1..1>`, `warm=<-1..1>`. Nothing is saved.
    pub(super) fn picture_viewer_demo(&mut self) {
        if !self.demo || self.frame_count != 600 {
            return;
        }
        let Ok(value) = std::env::var("AURORA_DEMO_PICTURE_VIEWER") else {
            return;
        };
        let options = value.split_once(':').map_or("", |(_, o)| o);
        let pv = &mut self.picture_viewer;
        let file = options.split(',').find_map(|o| o.trim().strip_prefix("file="));
        let (path, pic) = match file.map(|f| (PathBuf::from(f), picture::io::load(std::path::Path::new(f)))) {
            Some((path, Ok(pic))) => (path, pic),
            Some((path, Err(e))) => {
                log::warn!("picture viewer demo: {} not opened: {e}", path.display());
                (PathBuf::from("demo-picture.png"), picture::demo_image(1024, 768))
            }
            None => (PathBuf::from("demo-picture.png"), picture::demo_image(1024, 768)),
        };
        pv.opened(path, pic);
        let mut recipe = Recipe::default();
        for option in options.split(',').map(str::trim).filter(|o| !o.is_empty()) {
            let (key, value) = option.split_once('=').unwrap_or((option, ""));
            let number = value.parse::<f32>().ok();
            match key {
                "3d" => pv.ui.mode3d = true,
                "relief" => {
                    pv.relief = number.unwrap_or(0.6).clamp(0.0, 1.0);
                    pv.ui.relief = pv.relief;
                }
                "look" => {
                    pv.looks = value.split('+').filter(|id| looks::name(id).is_some()).map(Layer::new).collect();
                    looks::sanitize(&mut pv.looks);
                    pv.ui.looks.selected = pv.looks.len().saturating_sub(1);
                }
                "rotate" => recipe.edits.push(Edit::RotateRight),
                "flip" => recipe.edits.push(Edit::FlipHorizontal),
                "crop" => {
                    let (w, h) = value.split_once(':').unwrap_or(("16", "9"));
                    if let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>()) {
                        let (pw, ph) = picture::geometry::size_after(
                            pv.original.as_ref().map_or(1, |o| o.w),
                            pv.original.as_ref().map_or(1, |o| o.h),
                            &recipe.edits,
                        );
                        let aspect = w / h * ph as f32 / pw as f32;
                        let (cw, ch) = if aspect <= 1.0 { (aspect, 1.0) } else { (1.0, 1.0 / aspect) };
                        recipe.edits.push(Edit::Crop(
                            picture::Crop {
                                x: (1.0 - cw) / 2.0,
                                y: (1.0 - ch) / 2.0,
                                w: cw,
                                h: ch,
                            }
                            .clamped(),
                        ));
                    }
                }
                "bright" => recipe.adjust.brightness = number.unwrap_or(0.2),
                "warm" => recipe.adjust.warmth = number.unwrap_or(0.4),
                "file" => {}
                other => log::warn!("picture viewer demo: unknown option {other}"),
            }
        }
        recipe.adjust = recipe.adjust.clamped();
        if recipe != Recipe::default() {
            pv.history.push(Recipe::default());
            pv.set_recipe(recipe);
        }
        self.panels.picture_viewer = true;
        log::info!(
            "picture viewer demo: {} 3d={} relief={} looks={}",
            pv.name,
            pv.ui.mode3d,
            pv.relief,
            pv.looks.len()
        );
    }
}

/// Starts writing `pic` to `path`.
fn picture_viewer_save(pv: &mut PictureViewer, path: PathBuf, pic: Picture) {
    let (path, format) = picture::io::save_target(&path);
    let alpha = format == crate::snapshot::FileFormat::Png && pic.rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 255);
    pv.saving = Some(crate::snapshot::save::Saving::start(crate::snapshot::save::Job {
        width: pic.w,
        height: pic.h,
        rgba: pic.rgba.to_vec(),
        format,
        jpeg_quality: 92,
        path,
        alpha,
    }));
}
