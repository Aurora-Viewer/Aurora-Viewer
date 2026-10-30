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

struct Upload {
    generation: u64,
    te: usize,
    id: Uuid,
    texture: crate::imports::local::Texture,
}
enum Reply {
    Texture(Upload),
    Done(u64),
    Failed(u64, String),
}

#[derive(Default)]
pub struct Preview {
    assets: HashMap<(Uuid, Uuid), Wearable>,
    requested: HashSet<(Uuid, Uuid)>,
    failed: HashSet<(Uuid, Uuid)>,
    inputs: Vec<(Uuid, Uuid)>,
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
    pub fn invalidate(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.changed = Some(Instant::now());
        self.error = false;
        self.retired.extend(self.prepared.drain().map(|(_, value)| value));
        self.waiting.clear();
        self.completed = false;
    }
    pub fn cancel(&mut self, world: &mut World) {
        self.invalidate();
        self.receiver = None;
        self.waiting.clear();
        self.retired.extend(self.prepared.drain().map(|(_, value)| value));
        self.retired.extend(self.published.drain().map(|(_, value)| value));
        self.completed = false;
        self.built = None;
        self.active = false;
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
            self.error = true;
        }
    }
    pub fn cache(&mut self, item: Uuid, asset: Uuid, value: Wearable) {
        self.assets.insert((item, asset), value);
    }

    pub fn update(&mut self, world: &World, scene: &mut Scene, draft: &Draft) -> Vec<NetCommand> {
        self.active = true;
        let Some(cof) = crate::world::appearance::cof(&world.inventory) else {
            return Vec::new();
        };
        let mut links = crate::world::appearance::folder_links(&world.inventory, cof);
        links.sort_by_key(|link| {
            link.desc
                .strip_prefix('@')
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
        let worn: Vec<_> = links
            .iter()
            .filter(|link| !link.folder)
            .filter_map(|link| world.inventory.items.get(&link.target))
            .filter(|item| matches!(item.asset_type, 5 | 13))
            .cloned()
            .collect();
        let inputs: Vec<_> = worn.iter().map(|item| (item.id, item.asset_id)).collect();
        if self.inputs != inputs {
            self.inputs = inputs;
            self.invalidate();
        }
        let mut commands = Vec::new();
        for item in &worn {
            let key = (item.id, item.asset_id);
            if item.id != draft.item.id
                && !self.assets.contains_key(&key)
                && !self.requested.contains(&key)
                && !self.failed.contains(&key)
                && self.requested.len() < 4
            {
                self.requested.insert(key);
                commands.push(NetCommand::PreviewInventoryItem(item.clone()));
            }
        }
        if worn
            .iter()
            .any(|item| item.id != draft.item.id && !self.assets.contains_key(&(item.id, item.asset_id)))
        {
            return commands;
        }
        let mut values = Vec::new();
        for item in worn {
            let value = if item.id == draft.item.id {
                Some(draft.edited.clone())
            } else {
                self.assets.get(&(item.id, item.asset_id)).cloned()
            };
            if let Some(value) = value.filter(|value| u32::from(value.kind) == item.flags & 0xff) {
                values.push(value)
            } else {
                self.error = true;
                return commands;
            }
        }
        let mut sources = HashMap::new();
        let mut revisions = HashMap::new();
        let mut available = true;
        self.wanted.clear();
        for texture in values
            .iter()
            .flat_map(|value| {
                value.textures.iter().filter_map(|(slot, texture)| {
                    super::slot_bake(*slot)
                        .filter(|te| super::affected_bakes(draft.edited.kind).contains(te))
                        .map(|_| texture)
                })
            })
            .copied()
            .collect::<HashSet<_>>()
        {
            if texture.is_nil() || texture == IMG_DEFAULT_AVATAR || texture == IMG_INVISIBLE || texture == aurora_prim::te::BLANK_TEXTURE {
                continue;
            }
            scene.textures.note_usage(&texture, f32::MAX);
            self.wanted.insert(texture);
            let source = scene.textures.preview_source(&texture);
            if source.local.is_some() {
                revisions.insert(texture, source.revision());
            }
            if source.decoded.is_none_or(|level| level != 0) && !source.procedural {
                available = false;
            }
            sources.insert(texture, source);
        }
        // LLLocalBitmap::updateUserLayers rebakes when a watched file changes.
        // A local ID stays stable; its shared pixel revision changes on upload.
        if self.sources != revisions {
            self.sources = revisions;
            self.invalidate();
        }
        if !available
            || self.receiver.is_some()
            || self.built == Some(self.generation.load(Ordering::Relaxed))
            || self.changed.is_some_and(|changed| changed.elapsed() < Duration::from_millis(140))
        {
            return commands;
        }
        let Some(pool) = scene.textures.staging.clone() else {
            return commands;
        };
        let lib = world.avatar_lib.clone();
        let base = world.server_visual_params().to_vec();
        let kind = draft.edited.kind;
        let token = self.generation.clone();
        let generation = token.load(Ordering::Relaxed);
        let (tx, rx) = crossbeam_channel::unbounded();
        self.receiver = Some(rx);
        self.completed = false;
        self.retired.extend(self.prepared.drain().map(|(_, value)| value));
        self.built = Some(generation);
        scene.jobs.execute(move || {
            let start = Instant::now();
            let result = prepare(generation, &token, &pool, &lib, &base, kind, values, sources, &tx);
            if token.load(Ordering::Relaxed) == generation {
                if let Err(reason) = result {
                    let _ = tx.send(Reply::Failed(generation, reason));
                } else {
                    let _ = tx.send(Reply::Done(generation));
                }
                log::debug!("wearable preview: generation {generation}, {} ms", start.elapsed().as_millis());
            }
        });
        commands
    }

    pub fn apply(&mut self, world: &mut World, scene: &mut Scene, renderer: &mut Renderer) {
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
        if self.active
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
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    generation: u64,
    token: &AtomicU64,
    pool: &aurora_render::StagingPool,
    lib: &crate::scene::avatar::AvatarLibrary,
    base: &[u8],
    kind: u8,
    values: Vec<Wearable>,
    sources: HashMap<Uuid, PreviewSource>,
    tx: &crossbeam_channel::Sender<Reply>,
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
                .inspector_image(id)
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
    for region in defs.regions.iter().filter(|region| defs.affected(region, kind)) {
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
        let id = Uuid::new_v4();
        if tx
            .send(Reply::Texture(Upload {
                generation,
                te: region.te,
                id,
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
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        state.preview.update(&world, &mut scene, draft);
        assert_eq!(state.preview.sources.len(), 1, "local map participates in the real bake inputs");
        let generation = state.preview.generation.load(Ordering::Relaxed);
        state.preview.update(&world, &mut scene, draft);
        assert_eq!(
            state.preview.generation.load(Ordering::Relaxed),
            generation,
            "unchanged pixels do not rebake every frame"
        );
        scene.textures.set_local_pixels(id, image([0, 255, 0, 160]));
        state.preview.update(&world, &mut scene, draft);
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
}
