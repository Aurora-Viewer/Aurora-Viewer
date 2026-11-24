//! Cancellable, bounded self-bake preview. The frame thread only requests
//! missing worn assets, shares immutable image sources, and records staged
//! GPU copies. A complete generation is published together, then retired.
use super::{Draft, bake};
use crate::{
    scene::{
        Scene,
        avatar::{IMG_DEFAULT_AVATAR, IMG_INVISIBLE},
        textures::PreviewSource,
    },
    world::World,
};
use aurora_assets::wearable::Wearable;
use aurora_net::NetCommand;
use aurora_render::{Renderer, StagedTexture};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

const DEBOUNCE: Duration = Duration::from_millis(140);

struct Upload {
    generation: u64,
    te: usize,
    id: Uuid,
    texture: crate::imports::local::Texture,
    hint: Arc<aurora_render::preview::Image>,
}
enum Reply {
    Texture(Upload),
    Done(u64),
    Failed(u64, String),
}

pub enum Target<'a> {
    Layer(&'a Draft),
    Shape(Option<&'a Draft>),
}

#[derive(Default)]
pub struct Preview {
    assets: HashMap<(Uuid, Uuid), Wearable>,
    requested: HashSet<(Uuid, Uuid)>,
    failed: HashSet<(Uuid, Uuid)>,
    removed_local: HashSet<Uuid>,
    inputs: Vec<(Uuid, Uuid)>,
    regions: Vec<usize>,
    generation: Arc<AtomicU64>,
    changed: Option<Instant>,
    receiver: Option<crossbeam_channel::Receiver<Reply>>,
    waiting: Vec<Upload>,
    prepared: HashMap<usize, (Uuid, u32)>,
    published: HashMap<usize, (Uuid, u32)>,
    retired: Vec<(Uuid, u32)>,
    wanted: HashSet<Uuid>,
    sources: HashMap<Uuid, crate::scene::textures::PreviewRevision>,
    held: HashSet<Uuid>,
    original: HashSet<Uuid>,
    active: bool,
    completed: bool,
    built: Option<u64>,
    pub error: bool,
}
impl Preview {
    #[cfg(test)]
    pub(crate) fn test_generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn invalidate(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.changed = Some(Instant::now());
        self.error = false;
        self.retired.extend(self.prepared.drain().map(|(_, value)| value));
        self.waiting.clear();
        self.completed = false;
    }
    /// After a GPU recovery (app/gpu_recovery.rs): the preview's textures
    /// were slots of the lost renderer, forgotten and never freed (the new
    /// renderer numbers its own), and its uploads under way were staged in
    /// the lost pool. The preview is made again.
    pub fn gpu_reset(&mut self) {
        self.invalidate();
        self.retired.clear();
        self.published.clear();
    }

    pub fn cancel(&mut self, world: &mut World) {
        if !self.active {
            return;
        }
        self.invalidate();
        self.receiver = None;
        self.waiting.clear();
        self.retired.extend(self.prepared.drain().map(|(_, value)| value));
        self.retired.extend(self.published.drain().map(|(_, value)| value));
        self.completed = false;
        self.built = None;
        self.active = false;
        self.regions.clear();
        self.wanted.clear();
        self.sources.clear();
        self.requested.clear();
        self.failed.clear();
        world.preview_bakes(HashMap::new());
    }
    pub fn loaded(&mut self, item: Uuid, asset: Uuid, result: &Result<Vec<u8>, String>) {
        let key = (item, asset);
        if !self.requested.remove(&key) {
            return;
        }
        if let Ok(bytes) = result
            && let Ok(wearable) = Wearable::parse(bytes)
        {
            self.assets.insert(key, wearable);
        } else {
            self.failed.insert(key);
        }
    }
    pub fn cache(&mut self, item: Uuid, asset: Uuid, value: Wearable) {
        self.assets.insert((item, asset), value);
    }

    pub fn forget_local_texture(&mut self, id: Uuid) {
        self.removed_local.insert(id);
    }

    /// In shape mode the saved, worn layers supply the local maps, even if
    /// no layer editor has been opened this session (LLVOAvatarSelf::onCustomizeStart).
    pub fn update(&mut self, world: &World, scene: &mut Scene, target: Target<'_>, ctx: Option<&egui::Context>) -> Vec<NetCommand> {
        let (draft, layer_regions) = match target {
            Target::Layer(draft) => (Some(draft), Some(super::affected_bakes(draft.edited.kind))),
            Target::Shape(draft) => (draft, None),
        };
        self.active = true;
        let mut links = crate::world::appearance::cof(&world.inventory)
            .map(|cof| crate::world::appearance::folder_links(&world.inventory, cof))
            .unwrap_or_default();
        links.sort_by_key(|link| {
            link.desc
                .strip_prefix('@')
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
        let mut worn: Vec<_> = links
            .iter()
            .filter(|link| !link.folder)
            .filter_map(|link| world.inventory.items.get(&link.target))
            .filter(|item| super::supported(item))
            .filter(|item| {
                layer_regions.is_none_or(|regions| super::affected_bakes(item.flags as u8).iter().any(|te| regions.contains(te)))
            })
            .collect();
        let mut commands = Vec::new();
        for item in &worn {
            let key = (item.id, item.asset_id);
            if draft.is_none_or(|d| item.id != d.item.id)
                && !self.assets.contains_key(&key)
                && !self.requested.contains(&key)
                && !self.failed.contains(&key)
                && self.requested.len() < 4
            {
                self.requested.insert(key);
                commands.push(NetCommand::PreviewInventoryItem((*item).clone()));
            }
        }
        let regions = if let Some(regions) = layer_regions {
            regions.to_vec()
        } else {
            let mut local = Vec::new();
            for value in worn.iter().filter_map(|item| {
                draft
                    .filter(|d| d.item.id == item.id)
                    .map(|d| &d.edited)
                    .or_else(|| self.assets.get(&(item.id, item.asset_id)))
            }) {
                for (slot, texture) in &value.textures {
                    if world.local_assets.get(texture).is_some_and(|(_, material)| !material)
                        && let Some(te) = super::slot_bake(*slot)
                    {
                        local.push(te);
                    }
                }
            }
            local.sort_unstable();
            local.dedup();
            worn.retain(|item| super::affected_bakes(item.flags as u8).iter().any(|te| local.contains(te)));
            local
        };
        let inputs: Vec<_> = worn.iter().map(|item| (item.id, item.asset_id)).collect();
        if self.inputs != inputs || self.regions != regions {
            self.inputs = inputs;
            self.regions = regions;
            self.wanted.clear();
            self.sources.clear();
            self.invalidate();
        }
        if self.regions.is_empty() {
            return commands;
        }
        if worn
            .iter()
            .any(|item| draft.is_none_or(|d| item.id != d.item.id) && !self.assets.contains_key(&(item.id, item.asset_id)))
        {
            self.error = self
                .inputs
                .iter()
                .any(|key| self.failed.contains(key) && draft.is_none_or(|d| d.item.id != key.0));
            return commands;
        }
        let mut values = Vec::new();
        let mut draft_linked = false;
        for item in worn {
            let value = if let Some(draft) = draft.filter(|d| item.id == d.item.id) {
                draft_linked = true;
                Some(&draft.edited)
            } else {
                self.assets.get(&(item.id, item.asset_id))
            };
            if let Some(value) = value.filter(|value| u32::from(value.kind) == item.flags & 0xff) {
                values.push(value);
            } else {
                self.error = true;
                return commands;
            }
        }
        if !draft_linked && let Some(draft) = draft {
            values.push(&draft.edited);
        }
        // A second local map can keep this region active after a file is
        // removed. Never wait for the removed file's private ID on the grid.
        let resolve = |kind, texture: Uuid| {
            if self.removed_local.contains(&texture) && !world.local_assets.contains_key(&texture) {
                crate::world::inventory::wearable::default_texture(kind)
            } else {
                texture
            }
        };
        let mut sources = HashMap::new();
        let mut revisions = HashMap::new();
        let mut available = true;
        self.wanted.clear();
        for value in &values {
            for (&slot, &id) in &value.textures {
                if super::slot_bake(slot).is_none_or(|te| !self.regions.contains(&te)) {
                    continue;
                }
                let texture = resolve(value.kind, id);
                if texture.is_nil() || matches!(texture, IMG_DEFAULT_AVATAR | IMG_INVISIBLE | aurora_prim::te::BLANK_TEXTURE)
                    || !self.wanted.insert(texture) {
                    continue;
                }
                scene.textures.note_usage_boosted(&texture, f32::MAX);
                let source = scene.textures.preview_source(&texture);
                revisions.insert(texture, source.bake_revision());
                available &= source.bake_ready(texture);
                sources.insert(texture, source);
            }
        }
        // LLLocalBitmap::updateUserLayers rebakes when a watched file changes.
        // A local ID stays stable; its shared pixel revision changes on upload.
        if self.sources != revisions {
            self.sources = revisions;
            self.invalidate();
            if let Some(ctx) = ctx {
                ctx.request_repaint_after(DEBOUNCE);
            }
            return commands;
        }
        if !available || self.receiver.is_some() || self.built == Some(self.generation.load(Ordering::Relaxed)) {
            return commands;
        }
        if let Some(changed) = self.changed {
            let elapsed = changed.elapsed();
            if elapsed < DEBOUNCE {
                if let Some(ctx) = ctx {
                    ctx.request_repaint_after(DEBOUNCE - elapsed);
                }
                return commands;
            }
        }
        let Some(pool) = scene.textures.staging.clone() else {
            return commands;
        };
        // Copy complete wearable assets only when a bake is ready to start.
        let values = values.into_iter().cloned().map(|mut value| {
            for texture in value.textures.values_mut() {
                *texture = resolve(value.kind, *texture);
            }
            value
        }).collect();
        let lib = world.avatar_lib.clone();
        let base = world.server_visual_params().to_vec();
        let regions = self.regions.clone();
        let token = self.generation.clone();
        let generation = token.load(Ordering::Relaxed);
        let (tx, rx) = crossbeam_channel::unbounded();
        self.receiver = Some(rx);
        self.completed = false;
        self.retired.extend(self.prepared.drain().map(|(_, value)| value));
        self.built = Some(generation);
        let wake = ctx.cloned();
        scene.jobs.execute(move || {
            let start = Instant::now();
            let result = prepare(
                generation,
                &token,
                &pool,
                &lib,
                &base,
                &regions,
                values,
                sources,
                &tx,
                wake.as_ref(),
            );
            if token.load(Ordering::Relaxed) == generation {
                if let Err(reason) = result {
                    let _ = tx.send(Reply::Failed(generation, reason));
                } else {
                    let _ = tx.send(Reply::Done(generation));
                }
                log::debug!("wearable preview: generation {generation}, {} ms", start.elapsed().as_millis());
            }
            drop(tx);
            if let Some(wake) = wake {
                wake.request_repaint();
            }
        });
        commands
    }

    pub fn apply(&mut self, world: &mut World, scene: &mut Scene, renderer: &mut Renderer, ctx: Option<&egui::Context>) {
        if self.published.keys().any(|te| !self.regions.contains(te)) {
            self.published.retain(|te, texture| {
                if self.regions.contains(te) {
                    return true;
                }
                self.retired.push(*texture);
                false
            });
            world.preview_bakes(self.published.iter().map(|(te, (id, _))| (*te, *id)).collect());
        }
        for id in self.held.difference(&self.wanted) {
            scene.textures.release(id);
        }
        for id in self.wanted.difference(&self.held) {
            scene.textures.acquire(renderer, *id, crate::scene::textures::TexSource::Asset);
        }
        self.held.clone_from(&self.wanted);
        for id in &self.held {
            scene.textures.note_usage_fixed(id, f32::MAX);
        }
        let mut original = HashMap::new();
        if !self.regions.is_empty()
            && let Some(index) = world.objects.index_of_uuid(&world.agent_id)
        {
            for (te, name, _) in crate::scene::avatar::BAKES {
                if let Some(id) = world.server_bake_texture(index, te) {
                    original.insert(id, name);
                }
            }
        }
        for id in self.original.iter().filter(|id| !original.contains_key(id)) {
            scene.textures.release(id);
        }
        for (id, name) in original.iter().filter(|(id, _)| !self.original.contains(id)) {
            scene.textures.acquire(renderer, *id, world.bake_source(world.agent_id, name, *id));
        }
        self.original = original.into_keys().collect();
        for (id, slot) in self.retired.drain(..) {
            scene.textures.unregister_local(&id);
            renderer.free_texture(slot);
        }
        let generation = self.generation.load(Ordering::Relaxed);
        let mut disconnected = false;
        if let Some(receiver) = &self.receiver {
            loop {
                let reply = match receiver.try_recv() {
                    Ok(reply) => reply,
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                };
                match reply {
                    Reply::Texture(upload) if upload.generation == generation => self.waiting.push(upload),
                    Reply::Done(value) if value == generation => self.completed = true,
                    Reply::Failed(value, reason) if value == generation => {
                        log::warn!("wearable preview: {reason}");
                        self.error = true;
                        self.completed = false;
                    }
                    _ => {}
                }
            }
        }
        if disconnected {
            self.receiver = None;
        }
        let mut waiting = Vec::new();
        for upload in self.waiting.drain(..) {
            if upload.generation != generation {
                continue;
            }
            if !upload.texture.staged.data.ready() {
                waiting.push(upload);
                continue;
            }
            let Some(slot) = renderer.textures.create_placeholder() else {
                self.error = true;
                continue;
            };
            if renderer.replace_texture_staged(slot, &upload.texture.staged) {
                scene
                    .textures
                    .register_local_image(upload.id, slot, upload.texture.alpha, upload.texture.has_alpha);
                scene.textures.set_local_pixels(upload.id, upload.hint);
                self.prepared.insert(upload.te, (upload.id, slot));
            } else {
                renderer.free_texture(slot);
                self.error = true;
            }
        }
        self.waiting = waiting;
        if self.error {
            self.retired.extend(self.prepared.drain().map(|(_, value)| value));
            self.completed = false;
        } else if self.completed && self.waiting.is_empty() {
            world.preview_bakes(self.prepared.iter().map(|(te, (id, _))| (*te, *id)).collect());
            self.retired.extend(self.published.drain().map(|(_, value)| value));
            self.published = std::mem::take(&mut self.prepared);
            self.completed = false;
        }
        if !self.waiting.is_empty()
            && let Some(ctx) = ctx
        {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    generation: u64,
    token: &AtomicU64,
    pool: &aurora_render::StagingPool,
    lib: &crate::scene::avatar::AvatarLibrary,
    base: &[u8],
    regions: &[usize],
    values: Vec<Wearable>,
    sources: HashMap<Uuid, PreviewSource>,
    tx: &crossbeam_channel::Sender<Reply>,
    wake: Option<&egui::Context>,
) -> Result<(), String> {
    use rayon::prelude::*;
    let defs = bake::definitions()?;
    let mut edited = std::collections::BTreeMap::new();
    for value in &values {
        edited.extend(defs.owned(value));
    }
    let global = lib.shape_params.edited_weights(base, &edited);
    let worn: Vec<_> = values
        .into_iter()
        .map(|value| {
            let weights = lib.shape_params.edited_weights(base, &defs.owned(&value));
            bake::Worn { value, weights }
        })
        .collect();
    let mut images: HashMap<_, _> = sources
        .into_par_iter()
        .map(|(id, source)| {
            source
                .bake_image(id)
                .map(|image| (id, image))
                .ok_or_else(|| format!("texture {id} is unavailable"))
        })
        .collect::<Result<_, _>>()?;
    images.insert(
        aurora_prim::te::BLANK_TEXTURE,
        Arc::new(aurora_render::preview::Image {
            size: [1, 1],
            rgba: vec![255; 4].into(),
        }),
    );
    images.insert(
        IMG_INVISIBLE,
        Arc::new(aurora_render::preview::Image {
            size: [1, 1],
            rgba: vec![0; 4].into(),
        }),
    );
    for region in defs.regions.iter().filter(|region| regions.contains(&region.te)) {
        if token.load(Ordering::Relaxed) != generation {
            return Ok(());
        }
        // A low-memory staging pool holds a 2048² base level but not its
        // RGBA mip chain. Compose at 1024 there; no large frame-thread copy.
        let size = region.size.min(if pool.max_region_bytes() < 24 << 20 { 1024 } else { 2048 });
        let rgba = bake::compose(defs, region, size, &worn, &global, &images)?;
        let alpha = crate::scene::jobs::classify_alpha(&rgba);
        let mips = aurora_render::build_mips(size, size, rgba, 12);
        let refs: Vec<_> = mips.iter().map(|(w, h, data)| (*w, *h, data.as_slice())).collect();
        let mut staged = None;
        for _ in 0..6 {
            if token.load(Ordering::Relaxed) != generation {
                return Ok(());
            }
            staged = StagedTexture::new(pool, &refs);
            if staged.is_some() {
                break;
            }
        }
        let staged = staged.ok_or("texture staging is busy")?;
        // The shape slider thumbnails sample CPU pixels. One small, already
        // filtered mip suffices for all thumbnails that share this bake.
        let (w, h, pixels) = mips.into_iter().find(|(w, _, _)| *w <= 256).ok_or("missing preview mip")?;
        let hint = Arc::new(aurora_render::preview::Image {
            size: [w, h],
            rgba: pixels.into(),
        });
        let id = Uuid::new_v4();
        if tx
            .send(Reply::Texture(Upload {
                generation,
                te: region.te,
                id,
                hint,
                texture: crate::imports::local::Texture {
                    id,
                    staged,
                    alpha,
                    has_alpha: true,
                },
            }))
            .is_err()
        {
            return Ok(());
        }
        if let Some(wake) = wake {
            wake.request_repaint();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_unrelated_bodypart_cannot_block_a_live_clothing_texture() {
        let (mut world, mut state, _) = super::super::tests::fixture(4);
        let texture = Uuid::from_u128(1378003);
        world.local_assets.insert(texture, ("shirt.png".into(), false));
        assert!(state.set_texture(&world, 1, texture));
        let mut missing_eyes = false;
        for item in world.inventory.items.values().filter(|item| matches!(item.asset_type, 5 | 13)) {
            if item.flags & 0xff == 3 {
                missing_eyes = true;
                continue;
            }
            let bytes = crate::world::inventory::wearable::create((item.flags & 0xff) as u8, &item.name, world.agent_id).expect("wearable");
            state.preview.cache(item.id, item.asset_id, Wearable::parse(&bytes).expect("asset"));
        }
        assert!(missing_eyes);
        let mut scene = Scene::new(
            std::env::temp_dir().join(format!("aurora-shirt-preview-{}", Uuid::new_v4())),
            world.avatar_lib.clone(),
        );
        scene.textures.register_local(texture, 0);
        scene.textures.set_local_pixels(
            texture,
            Arc::new(aurora_render::preview::Image {
                size: [1, 1],
                rgba: vec![255, 0, 0, 255].into(),
            }),
        );
        let commands = state
            .preview
            .update(&world, &mut scene, Target::Layer(state.draft.as_ref().expect("draft")), None);
        assert!(
            commands.is_empty(),
            "an unrelated eye asset must not be requested by the shirt preview"
        );
        assert!(
            state.preview.wanted.contains(&texture),
            "the shirt reaches texture preparation despite missing eyes"
        );
    }

    #[test]
    fn shape_preview_discovers_local_maps_on_worn_layers_and_releases_their_regions() {
        let (mut world, _, tattoo) = super::super::tests::fixture(14);
        let head = Uuid::new_v4();
        let body = Uuid::new_v4();
        world.local_assets.insert(head, ("head.png".into(), false));
        world.local_assets.insert(body, ("body.png".into(), false));
        let mut preview = Preview::default();
        let mut scene = Scene::new(
            std::env::temp_dir().join(format!("aurora-shape-layers-{head}")),
            world.avatar_lib.clone(),
        );
        // Open Shape directly: discover the worn assets through the normal
        // requests, with an unrelated eye asset that cannot be read.
        loop {
            let requests = preview.update(&world, &mut scene, Target::Shape(None), None);
            if requests.is_empty() {
                break;
            }
            assert!(requests.len() <= 4);
            for request in requests {
                let NetCommand::PreviewInventoryItem(item) = request else {
                    panic!("wearable request")
                };
                let kind = item.flags as u8;
                if kind == 3 {
                    preview.loaded(item.id, item.asset_id, &Err("missing eyes".into()));
                    continue;
                }
                let data = crate::world::inventory::wearable::create(kind, &item.name, world.agent_id).expect("wearable");
                let mut value = Wearable::parse(&data).expect("asset");
                if item.id == tattoo {
                    value.textures.insert(26, head);
                } else if kind == 1 {
                    value.textures.insert(5, body);
                }
                preview.loaded(item.id, item.asset_id, &Ok(value.encode(&item.name).expect("encoded")));
            }
        }
        assert_eq!(preview.regions, [8, 9]);
        assert!(preview.wanted.contains(&head) && preview.wanted.contains(&body));
        assert!(!preview.error);
        world.local_assets.remove(&head);
        preview.forget_local_texture(head);
        preview.update(&world, &mut scene, Target::Shape(None), None);
        assert_eq!(preview.regions, [9]);
        assert!(!preview.wanted.contains(&head));
        let cof = crate::world::appearance::cof(&world.inventory).expect("current outfit");
        let skin_link = world
            .inventory
            .items
            .values()
            .find(|i| {
                i.parent == cof
                    && i.asset_type == 24
                    && world
                        .inventory
                        .items
                        .get(&i.asset_id)
                        .is_some_and(|w| w.asset_type == 13 && w.flags as u8 == 1)
            })
            .expect("skin link")
            .id;
        world.inventory.remove(&[skin_link]);
        preview.update(&world, &mut scene, Target::Shape(None), None);
        assert!(preview.regions.is_empty());
        assert!(preview.wanted.is_empty());
    }

    #[test]
    fn shape_preserves_an_unsaved_local_layer_without_overwriting_the_shape_sliders() {
        let (mut world, mut state, _) = super::super::tests::fixture(14);
        let local = Uuid::new_v4();
        world.local_assets.insert(local, ("tattoo.png".into(), false));
        state.open_picker(&world, 26);
        assert!(state.preview_picker(&world, local));
        assert!(state.suspend_for_shape(&world));
        assert!(!state.open && state.picker_slot.is_none());
        world.preview_shape(Some(vec![19, 88]));
        state.update(&mut world);
        let draft = state.draft.as_ref().expect("suspended draft");
        let mut scene = Scene::new(
            std::env::temp_dir().join(format!("aurora-layer-shape-{local}")),
            world.avatar_lib.clone(),
        );
        state.preview.update(&world, &mut scene, Target::Shape(Some(draft)), None);
        assert_eq!(state.preview.regions, [8]);
        assert_eq!(world.visual_params(&world.agent_id), Some([19, 88].as_slice()));
        state.resume_layer(&mut world);
        assert!(state.open && !state.suspended_for_shape);
        let draft = state.draft.as_ref().expect("resumed draft");
        assert!(draft.dirty());
        assert_eq!(draft.edited.textures[&26], local);
        assert_ne!(draft.original.textures[&26], local);
    }

    #[test]
    fn local_tattoo_reload_invalidates_the_bake_and_cancel_restores_server_appearance() {
        let (mut world, mut state, _) = super::super::tests::fixture(14);
        let id = Uuid::new_v4();
        world.local_assets.insert(id, ("tattoo.png".into(), false));
        assert!(state.set_texture(&world, 26, id));
        for item in world.inventory.items.values().filter(|item| matches!(item.asset_type, 5 | 13)) {
            let bytes = crate::world::inventory::wearable::create((item.flags & 0xff) as u8, &item.name, world.agent_id);
            if let Ok(value) = bytes.and_then(|bytes| Wearable::parse(&bytes).map_err(|error| error.to_string())) {
                state.preview.cache(item.id, item.asset_id, value);
            }
        }
        let mut scene = Scene::new(
            std::env::temp_dir().join(format!("aurora-local-tattoo-{}", Uuid::new_v4())),
            world.avatar_lib.clone(),
        );
        scene.textures.register_local(id, 0);
        let image = |color: [u8; 4]| {
            Arc::new(aurora_render::preview::Image {
                size: [1, 1],
                rgba: color.to_vec().into(),
            })
        };
        scene.textures.set_local_pixels(id, image([255, 0, 0, 160]));
        let draft = state.draft.as_ref().expect("draft");
        state.preview.update(&world, &mut scene, Target::Layer(draft), None);
        assert_eq!(state.preview.sources.len(), 1, "local map participates in the real bake inputs");
        let generation = state.preview.generation.load(Ordering::Relaxed);
        state.preview.update(&world, &mut scene, Target::Layer(draft), None);
        assert_eq!(
            state.preview.generation.load(Ordering::Relaxed),
            generation,
            "unchanged pixels do not rebake every frame"
        );
        scene.textures.set_local_pixels(id, image([0, 255, 0, 160]));
        state.preview.update(&world, &mut scene, Target::Layer(draft), None);
        assert!(
            state.preview.generation.load(Ordering::Relaxed) > generation,
            "a file reload invalidates the bake without changing its ID"
        );
        state.close(&mut world);
        assert!(state.preview.sources.is_empty());
        assert!(state.preview.wanted.is_empty());
        assert!(!state.preview.active);
        assert!(state.draft.is_none());
    }

    #[test]
    fn a_pending_tattoo_still_previews_beside_another_inventory_item_of_the_same_kind() {
        let (mut world, mut state, id) = super::super::tests::fixture(14);
        let draft_texture = Uuid::new_v4();
        world.local_assets.insert(draft_texture, ("edited-tattoo.png".into(), false));
        assert!(state.set_texture(&world, 26, draft_texture));
        assert!(state.save(&world, None).is_some());

        let cof = crate::world::appearance::cof(&world.inventory).expect("current outfit");
        let link = world
            .inventory
            .items
            .values()
            .find(|item| item.parent == cof && item.asset_type == 24 && item.asset_id == id)
            .expect("edited tattoo link")
            .id;
        world.inventory.remove(&[link]);
        state.update(&mut world);
        let draft = state.draft.as_ref().expect("pending save keeps the edited tattoo open");
        assert!(!super::super::worn(&world, id));

        let other_texture = Uuid::new_v4();
        for item in world.inventory.items.values().filter(|item| matches!(item.asset_type, 5 | 13)) {
            let bytes = crate::world::inventory::wearable::create((item.flags & 0xff) as u8, &item.name, world.agent_id).expect("wearable");
            let mut value = Wearable::parse(&bytes).expect("asset");
            if item.flags & 0xff == 14 && item.id != id {
                value.textures.insert(26, other_texture);
            }
            state.preview.cache(item.id, item.asset_id, value);
        }
        let mut scene = Scene::new(
            std::env::temp_dir().join(format!("aurora-pending-tattoo-{}", Uuid::new_v4())),
            world.avatar_lib.clone(),
        );
        state.preview.update(&world, &mut scene, Target::Layer(draft), None);
        assert!(state.preview.wanted.contains(&draft_texture));
        assert!(state.preview.wanted.contains(&other_texture));
    }

    #[test]
    fn local_skin_and_universal_layer_reload_rebakes_and_requests_repaint() {
        // Issue #3173: local texture reload for Skin (kind 1) and Universal (kind 16).
        for (kind, slot) in [(1u8, 0u32), (1u8, 5u32), (16u8, 29u32), (16u8, 30u32)] {
            let (mut world, mut state, _) = super::super::tests::fixture(kind);
            let id = Uuid::new_v4();
            world.local_assets.insert(id, ("texture.png".into(), false));
            assert!(state.set_texture(&world, slot, id));
            for item in world.inventory.items.values().filter(|item| matches!(item.asset_type, 5 | 13)) {
                let bytes = crate::world::inventory::wearable::create((item.flags & 0xff) as u8, &item.name, world.agent_id);
                if let Ok(value) = bytes.and_then(|bytes| Wearable::parse(&bytes).map_err(|error| error.to_string())) {
                    state.preview.cache(item.id, item.asset_id, value);
                }
            }
            let mut scene = Scene::new(
                std::env::temp_dir().join(format!("aurora-local-layer-{}-{}", kind, Uuid::new_v4())),
                world.avatar_lib.clone(),
            );
            scene.textures.register_local(id, 0);
            let image = |color: [u8; 4]| {
                Arc::new(aurora_render::preview::Image {
                    size: [1, 1],
                    rgba: color.to_vec().into(),
                })
            };
            scene.textures.set_local_pixels(id, image([200, 100, 50, 255]));
            let ctx = egui::Context::default();
            let draft = state.draft.as_ref().expect("draft");
            state.preview.update(&world, &mut scene, Target::Layer(draft), Some(&ctx));
            assert!(
                state.preview.wanted.contains(&id),
                "kind {kind} slot {slot} registers local texture as wanted"
            );
            assert!(
                state.preview.sources.contains_key(&id),
                "local texture is included in bake input sources"
            );
            // The demo outfit's other worn maps must also be ready for the debounce to run.
            for &texture in state.preview.wanted.iter().filter(|texture| **texture != id) {
                scene.textures.set_local_pixels(texture, image([255; 4]));
            }
            state.preview.update(&world, &mut scene, Target::Layer(draft), None);
            let (tx, rx) = crossbeam_channel::unbounded();
            ctx.set_repaint_observer(move |info| {
                tx.send(info.delay).expect("repaint observer");
            });
            let initial_gen = state.preview.generation.load(Ordering::Relaxed);

            // Simulate local file update (new pixels with same UUID)
            scene.textures.set_local_pixels(id, image([50, 100, 200, 255]));
            state.preview.update(&world, &mut scene, Target::Layer(draft), Some(&ctx));
            let new_gen = state.preview.generation.load(Ordering::Relaxed);
            assert!(
                new_gen > initial_gen,
                "local texture reload triggers invalidation and increments generation for kind {kind}"
            );
            let delay = rx.try_recv().expect("the reload schedules its own repaint");
            assert!(!delay.is_zero() && delay <= Duration::from_millis(140));

            state.close(&mut world);
        }
    }
}
