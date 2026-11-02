//! Our own bakes, where the region does not bake (OpenSim: no
//! `UpdateAvatarAppearance`, but `UploadBakedTexture`). As the Legacy
//! Viewer does without central baking (LLVOAvatarSelf,
//! LLViewerTexLayerSetBuffer, LLAgent::sendAgentSetAppearance; llvoavatarself.cpp,
//! llviewertexlayer.cpp, llagent.cpp, originally LGPL 2.1): what is worn is
//! composed into every bake (`wearable_editor::bake`, the self preview's
//! compositor), each bake is encoded and uploaded, then the appearance goes
//! out with them, the visual parameters and the avatar's size.
//!
//! The frame thread only watches the Current Outfit folder, asks for the
//! wearables and their textures, and hands the work over: composing and
//! encoding run on the scene's workers. A change of what is worn starts
//! again, after a moment for a burst of changes to settle.
//!
//! Not done: the Legacy Viewer first asks the simulator for the bakes it
//! keeps (AgentCachedTexture) and only uploads those it lacks; this bakes
//! again at each login.

use crate::scene::Scene;
use crate::scene::avatar::{IMG_DEFAULT_AVATAR, IMG_INVISIBLE};
use crate::wearable_editor::bake;
use crate::world::World;
use aurora_assets::wearable::Wearable;
use aurora_net::NetCommand;
use aurora_net::local_bakes::{AVATAR_FACES, Appearance, BAKES, UNIVERSAL, cache_id};
use aurora_render::Renderer;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A burst of outfit changes (a whole outfit worn) gives one bake.
const SETTLE: Duration = Duration::from_secs(1);
/// Wearable assets asked for at once.
const ASKED_AT_ONCE: usize = 4;
/// Bakes tried again after a failed upload (the Legacy Viewer's final bakes
/// try 7 times, with a growing delay).
const UPLOAD_TRIES: u32 = 3;
/// Wearable types, as `bake.rs` numbers them.
const SKIRT: u8 = 12;

enum Reply {
    /// One bake: encoded, or invisible (nothing of it shows).
    Bake {
        te: usize,
        j2c: Option<Vec<u8>>,
    },
    /// All bakes sent; the visual parameters and the avatar's size.
    Done {
        params: Vec<u8>,
        height: f32,
    },
    Failed(String),
}

#[derive(Default)]
pub struct LocalBakes {
    assets: HashMap<(Uuid, Uuid), Wearable>,
    requested: HashSet<(Uuid, Uuid)>,
    failed: HashSet<(Uuid, Uuid)>,
    /// What is worn, (item, asset), in the folder's layer order.
    inputs: Vec<(Uuid, Uuid)>,
    /// The worn wearables' (type, asset), for the cache keys.
    worn_kinds: Vec<(u8, Uuid)>,
    generation: Arc<AtomicU64>,
    /// When the outfit last changed (a bake waits for it to settle), `None`
    /// once the current outfit is baked or being baked.
    dirty: Option<Instant>,
    job: Option<crossbeam_channel::Receiver<Reply>>,
    /// Textures the job reads, held while it may.
    wanted: HashSet<Uuid>,
    held: HashSet<Uuid>,
    /// Uploads in flight: request -> bake face.
    uploads: HashMap<Uuid, usize>,
    /// Bake face -> its texture (uploaded, or invisible).
    baked: HashMap<usize, Uuid>,
    /// Faces the job will send, and the parameters once it is done.
    expected: HashSet<usize>,
    finished: Option<(Vec<u8>, f32)>,
    tries: u32,
}

impl LocalBakes {
    /// Bake again (RebakeAvatarTextures, or a bake the server was asked for).
    pub fn rebake(&mut self) {
        self.dirty = Some(Instant::now() - SETTLE);
        self.tries = 0;
    }

    pub fn loaded(&mut self, item: Uuid, asset: Uuid, result: &Result<Vec<u8>, String>) {
        let key = (item, asset);
        if !self.requested.remove(&key) {
            return;
        }
        match result.as_ref().ok().and_then(|bytes| Wearable::parse(bytes).ok()) {
            Some(wearable) => {
                self.assets.insert(key, wearable);
            }
            None => {
                log::warn!("local bake: a worn wearable could not be read");
                self.failed.insert(key);
            }
        }
    }

    pub fn uploaded(&mut self, request: Uuid, result: &Result<Uuid, String>) -> bool {
        let Some(te) = self.uploads.remove(&request) else {
            return false;
        };
        match result {
            Ok(id) => {
                self.baked.insert(te, *id);
            }
            Err(reason) => {
                log::warn!("local bake: face {te} not uploaded: {reason}");
                self.cancel();
                if self.tries < UPLOAD_TRIES {
                    self.tries += 1;
                    self.dirty = Some(Instant::now() + Duration::from_secs(2 << self.tries) - SETTLE);
                }
            }
        }
        true
    }

    fn cancel(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.job = None;
        self.uploads.clear();
        self.baked.clear();
        self.expected.clear();
        self.finished = None;
    }

    /// Once a frame: watch the outfit, ask for what is missing, start the
    /// job when everything is there, and send what it gave.
    pub fn update(&mut self, world: &World, scene: &mut Scene) -> Vec<NetCommand> {
        let mut commands = Vec::new();
        if !world.bakes_locally() {
            return commands;
        }
        let Some(cof) = crate::world::appearance::cof(&world.inventory) else {
            return commands;
        };
        let mut links = crate::world::appearance::folder_links(&world.inventory, cof);
        // the folder's layer order (LLAppearanceMgr's "@n" descriptions)
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
            self.cancel();
            self.dirty = Some(Instant::now());
            self.tries = 0;
        }
        self.receive(&mut commands);
        let Some(changed) = self.dirty else {
            return commands;
        };
        for item in &worn {
            let key = (item.id, item.asset_id);
            if !self.assets.contains_key(&key)
                && !self.requested.contains(&key)
                && !self.failed.contains(&key)
                && self.requested.len() < ASKED_AT_ONCE
            {
                self.requested.insert(key);
                commands.push(NetCommand::PreviewInventoryItem(item.clone()));
            }
        }
        if worn.iter().any(|item| !self.assets.contains_key(&(item.id, item.asset_id))) {
            return commands;
        }
        // a wearable whose asset is not of its item's type is left out, as
        // the self preview does
        let pairs: Vec<(Uuid, Wearable)> = worn
            .iter()
            .filter_map(|item| {
                let value = self.assets.get(&(item.id, item.asset_id))?;
                (u32::from(value.kind) == item.flags & 0xff).then(|| (item.asset_id, value.clone()))
            })
            .collect();
        self.worn_kinds = pairs.iter().map(|(asset, value)| (value.kind, *asset)).collect();
        let values: Vec<Wearable> = pairs.into_iter().map(|(_, value)| value).collect();
        // the textures the bakes read, held and ready
        let mut sources = HashMap::new();
        let mut ready = true;
        self.wanted.clear();
        for texture in values
            .iter()
            .flat_map(|value| value.textures.values())
            .copied()
            .collect::<HashSet<_>>()
        {
            if texture.is_nil() || texture == IMG_DEFAULT_AVATAR || texture == IMG_INVISIBLE || texture == aurora_prim::te::BLANK_TEXTURE {
                continue;
            }
            scene.textures.note_usage(&texture, f32::MAX);
            self.wanted.insert(texture);
            let source = scene.textures.preview_source(&texture);
            ready &= source.bake_ready(texture);
            sources.insert(texture, source);
        }
        if !ready || self.job.is_some() || changed.elapsed() < SETTLE {
            return commands;
        }
        // Legacy rules: no skirt bake without a skirt; the bakes of bakes on
        // mesh only with a universal (LLAgent::sendAgentSetAppearance)
        let skirt = values.iter().any(|value| value.kind == SKIRT);
        let universal = values.iter().any(|value| value.kind == UNIVERSAL);
        self.expected = BAKES
            .iter()
            .map(|b| usize::from(b.te))
            .filter(|te| (*te != 19 || skirt) && (*te < 40 || universal))
            .collect();
        let expected = self.expected.clone();
        let lib = world.avatar_lib.clone();
        let token = self.generation.clone();
        let generation = token.load(Ordering::Relaxed);
        let (tx, rx) = crossbeam_channel::unbounded();
        self.job = Some(rx);
        self.dirty = None;
        log::info!("local bake: {} wearables into {} bakes", values.len(), expected.len());
        scene.jobs.execute(move || {
            if let Err(reason) = compose_all(&token, generation, &lib, values, sources, &expected, &tx) {
                let _ = tx.send(Reply::Failed(reason));
            }
        });
        commands
    }

    fn receive(&mut self, commands: &mut Vec<NetCommand>) {
        let Some(job) = &self.job else {
            return;
        };
        let mut gone = false;
        loop {
            match job.try_recv() {
                Ok(Reply::Bake { te, j2c: Some(j2c) }) => {
                    let request = Uuid::new_v4();
                    self.uploads.insert(request, te);
                    commands.push(NetCommand::UploadBake { request, j2c });
                }
                Ok(Reply::Bake { te, j2c: None }) => {
                    self.baked.insert(te, IMG_INVISIBLE);
                }
                Ok(Reply::Done { params, height }) => self.finished = Some((params, height)),
                Ok(Reply::Failed(reason)) => {
                    log::warn!("local bake: {reason}");
                    gone = true;
                    break;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    gone = self.finished.is_none();
                    break;
                }
            }
        }
        if gone {
            self.cancel();
            return;
        }
        if self.uploads.is_empty()
            && let Some((params, height)) = &self.finished
            && self.expected.iter().all(|te| self.baked.contains_key(te))
        {
            commands.push(NetCommand::SetAppearance(Box::new(self.appearance(params.clone(), *height))));
            log::info!("local bake: appearance sent with {} bakes", self.baked.len());
            self.job = None;
            self.finished = None;
            self.uploads.clear();
            self.expected.clear();
        }
    }

    /// The appearance of the bakes made (LLAgent::sendAgentSetAppearance).
    fn appearance(&self, params: Vec<u8>, height: f32) -> Appearance {
        let mut te = aurora_prim::te::TextureEntry::default();
        for (i, face) in te.faces.iter_mut().enumerate().take(AVATAR_FACES) {
            face.texture = self.baked.get(&i).copied().unwrap_or(IMG_DEFAULT_AVATAR);
        }
        let worn = |kind: u8| -> Vec<Uuid> {
            self.worn_kinds
                .iter()
                .filter(|(k, _)| *k == kind)
                .map(|(_, asset)| *asset)
                .collect()
        };
        let cache = BAKES
            .iter()
            .filter(|b| self.baked.contains_key(&usize::from(b.te)))
            .filter_map(|b| cache_id(b, worn).map(|id| (id, b.te)))
            .collect();
        Appearance {
            // DEFAULT_AGENT_DEPTH, DEFAULT_AGENT_WIDTH (indra_constants.h)
            size: glam::Vec3::new(0.45, 0.60, height),
            cache,
            texture_entry: aurora_prim::te::pack_texture_entry(&te, AVATAR_FACES),
            params,
        }
    }

    /// Hold the textures the job reads, as the self preview does.
    pub fn apply(&mut self, scene: &mut Scene, renderer: &mut Renderer) {
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
    }
}

/// The job: every expected bake composed and encoded, then the visual
/// parameters and the body's height from the same wearables.
fn compose_all(
    token: &AtomicU64,
    generation: u64,
    lib: &crate::scene::avatar::AvatarLibrary,
    values: Vec<Wearable>,
    sources: HashMap<Uuid, crate::scene::textures::PreviewSource>,
    expected: &HashSet<usize>,
    tx: &crossbeam_channel::Sender<Reply>,
) -> Result<(), String> {
    let defs = bake::definitions()?;
    let mut owned = std::collections::BTreeMap::new();
    for value in &values {
        owned.extend(defs.owned(value));
    }
    let params = lib.shape_params.transmit_values(&owned);
    let global = lib.shape_params.weights(&params);
    let worn: Vec<_> = values
        .into_iter()
        .map(|value| {
            let weights = lib.shape_params.weights_with(&params, &defs.owned(&value));
            bake::Worn { value, weights }
        })
        .collect();
    let mut images: HashMap<_, _> = sources
        .into_iter()
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
    for region in defs.regions.iter().filter(|region| expected.contains(&region.te)) {
        if token.load(Ordering::Relaxed) != generation {
            return Ok(());
        }
        let rgba = bake::compose(defs, region, region.size, &worn, &global, &images)?;
        // LLViewerTexLayerSet::isVisible: nothing of the layer set shows
        let j2c = if rgba.chunks_exact(4).all(|pixel| pixel[3] == 0) {
            None
        } else {
            // lossy, as LLImageJ2C's default for bakes
            Some(aurora_assets::j2k::encode_texture_j2k(region.size, region.size, 4, &rgba, false).map_err(|e| e.to_string())?)
        };
        if tx.send(Reply::Bake { te: region.te, j2c }).is_err() {
            return Ok(());
        }
    }
    let shape = lib.shape_params.shape(&params, &lib.rig);
    let (_, height) = crate::scene::shape::body_size(&lib.rig, &shape.local_pos, &shape.scale);
    let _ = tx.send(Reply::Done { params, height });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baked(faces: &[(usize, Uuid)], worn: &[(u8, Uuid)]) -> Appearance {
        let bakes = LocalBakes {
            baked: faces.iter().copied().collect(),
            worn_kinds: worn.to_vec(),
            ..Default::default()
        };
        bakes.appearance(vec![1, 2, 3], 1.8)
    }

    #[test]
    fn faces_without_a_bake_say_default_avatar() {
        let head = Uuid::from_u128(8);
        let a = baked(&[(8, head), (11, IMG_INVISIBLE)], &[(1, Uuid::from_u128(100))]);
        let te = aurora_prim::te::parse_texture_entry(&a.texture_entry).expect("texture entry");
        assert_eq!(te.face(8).texture, head);
        assert_eq!(te.face(11).texture, IMG_INVISIBLE);
        for face in [0, 7, 9, 19, 20, 40, 44] {
            assert_eq!(te.face(face).texture, IMG_DEFAULT_AVATAR, "face {face}");
        }
        assert_eq!((a.size.x, a.size.y, a.size.z), (0.45, 0.60, 1.8));
        assert_eq!(a.params, [1, 2, 3]);
    }

    #[test]
    fn only_bakes_made_of_something_worn_carry_a_cache_key() {
        // a skin (type 1) feeds the head; nothing feeds the eyes' bake here
        let a = baked(&[(8, Uuid::from_u128(8)), (11, Uuid::from_u128(11))], &[(1, Uuid::from_u128(100))]);
        assert_eq!(a.cache.iter().map(|(_, te)| *te).collect::<Vec<_>>(), [8]);
        let expected = cache_id(&BAKES[0], |kind| if kind == 1 { vec![Uuid::from_u128(100)] } else { Vec::new() });
        assert_eq!(Some(a.cache[0].0), expected);
    }
}
