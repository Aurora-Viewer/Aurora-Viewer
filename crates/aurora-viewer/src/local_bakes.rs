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
//! Before baking, the region is asked for the bakes it keeps for what is
//! worn (AgentCachedTexture, LLAgentWearables::queryWearableCache and
//! LLAgent::processAgentCachedTextureResponse): OpenSim keeps each face's
//! cache key with the appearance, across logins, so a login in the same
//! outfit, or a change of one layer, only composes and uploads what the
//! region lacks.

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
/// Reads of a worn wearable before the bake goes without it.
const WEARABLE_TRIES: u32 = 3;
/// Least time between two rebakes the region or the outfit sync asks for.
const REBAKE_COOLDOWN: Duration = Duration::from_secs(30);
/// How long the region may take to say which bakes it keeps, before
/// everything is baked anyway.
const QUERY_WAIT: Duration = Duration::from_secs(5);
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
    /// Failed reads of a worn wearable: past [`WEARABLE_TRIES`] it is left
    /// out of the bake, as the self preview leaves a mismatched one.
    failed: HashMap<(Uuid, Uuid), u32>,
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
    /// Bakes sent, for our own avatar to show (`take_sent`).
    sent: Option<HashMap<usize, Uuid>>,
    /// The last rebake asked for, and whether bakes went out since: a region
    /// that keeps asking gets one rebake per bake sent, 30 s apart at most.
    last_rebake: Option<Instant>,
    rebake_pending: bool,
    /// The outfit last sent: a server bake asked for it is not redone.
    sent_inputs: Option<Vec<(Uuid, Uuid)>>,
    /// Whether the region was asked for this outfit's bakes, the question
    /// still open (its serial and when it went), and what it keeps: face ->
    /// texture.
    queried: bool,
    query: Option<(i32, Instant)>,
    serial: i32,
    cached: HashMap<usize, Uuid>,
}

impl LocalBakes {
    /// The region lacks one of our bakes (RebakeAvatarTextures): bake
    /// everything again, without asking it what it keeps. Once per bake
    /// sent and [`REBAKE_COOLDOWN`] apart: a region that keeps asking, for a
    /// bake it keeps refusing, makes no endless loop of bakes and uploads.
    /// False when the request is let go.
    pub fn rebake(&mut self, now: Instant) -> bool {
        if self.rebake_pending || self.last_rebake.is_some_and(|at| now.duration_since(at) < REBAKE_COOLDOWN) {
            log::info!("local bake: rebake let go, one already asked for since the last bake sent");
            return false;
        }
        self.rebake_pending = true;
        self.last_rebake = Some(now);
        self.restart();
        self.queried = true;
        self.dirty = Some(now - SETTLE);
        true
    }

    /// The server is asked to bake (RequestServerAppearance): nothing to do
    /// when this outfit is sent or on its way, which baked it twice at
    /// each login.
    pub fn refresh(&mut self) {
        if self.dirty.is_some() || self.job.is_some() || self.sent_inputs.as_ref() == Some(&self.inputs) {
            return;
        }
        self.restart();
        self.dirty = Some(Instant::now() - SETTLE);
    }

    /// A new round: ask the region again.
    fn restart(&mut self) {
        self.tries = 0;
        self.queried = false;
        self.query = None;
        self.cached.clear();
    }

    /// AgentCachedTextureResponse: the region's bakes for the faces asked,
    /// a nil texture for those it lacks.
    pub fn cached_answer(&mut self, serial: i32, faces: &[(u8, Uuid)]) {
        if self.query.is_none_or(|(asked, _)| asked != serial) {
            return;
        }
        self.query = None;
        self.cached = faces
            .iter()
            .filter(|(_, texture)| !texture.is_nil())
            .map(|(te, texture)| (usize::from(*te), *texture))
            .collect();
        log::info!("local bake: the region keeps {} of {} bakes", self.cached.len(), faces.len());
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
                let tries = self.failed.entry(key).or_default();
                *tries += 1;
                log::warn!("local bake: a worn wearable could not be read ({tries} of {WEARABLE_TRIES})");
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
                } else {
                    // given up: a later RebakeAvatarTextures may try again,
                    // past the cooldown, instead of waiting for an outfit change
                    self.rebake_pending = false;
                }
            }
        }
        true
    }

    /// Logout: nothing of this session's outfit is kept. The next login
    /// wears the same items, which would otherwise look unchanged and
    /// never be baked (#1985). A job still composing stops, and the
    /// textures it held are let go: the scene keeps its textures across
    /// sessions.
    pub fn end_session(&mut self, scene: &mut Scene) {
        for id in self.reset() {
            scene.textures.release(&id);
        }
    }

    /// The state of a new session; the textures held until now.
    fn reset(&mut self) -> HashSet<Uuid> {
        self.cancel();
        std::mem::take(self).held
    }

    /// A worn wearable the bake still waits for: not read yet, and not
    /// given up on.
    fn still_reading(&self, key: (Uuid, Uuid)) -> bool {
        !self.assets.contains_key(&key) && self.failed.get(&key).copied().unwrap_or(0) < WEARABLE_TRIES
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
            // nothing to read for a bake: `apply` lets the textures go
            self.wanted.clear();
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
            self.restart();
            self.dirty = Some(Instant::now());
        }
        self.receive(&mut commands);
        let Some(changed) = self.dirty else {
            return commands;
        };
        for item in &worn {
            let key = (item.id, item.asset_id);
            if !self.assets.contains_key(&key)
                && !self.requested.contains(&key)
                && self.failed.get(&key).copied().unwrap_or(0) < WEARABLE_TRIES
                && self.requested.len() < ASKED_AT_ONCE
            {
                self.requested.insert(key);
                commands.push(NetCommand::PreviewInventoryItem(item.clone()));
            }
        }
        // waiting for the wearables still read; one that failed every try is
        // left out rather than holding the bake for good
        if worn.iter().any(|item| self.still_reading((item.id, item.asset_id))) {
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
        // Legacy rules: no skirt bake without a skirt; the bakes of bakes on
        // mesh only with a universal (LLAgent::sendAgentSetAppearance)
        let skirt = values.iter().any(|value| value.kind == SKIRT);
        let universal = values.iter().any(|value| value.kind == UNIVERSAL);
        let expected: HashSet<usize> = BAKES
            .iter()
            .map(|b| usize::from(b.te))
            .filter(|te| (*te != 19 || skirt) && (*te < 40 || universal))
            .collect();
        if !self.queried {
            self.queried = true;
            let keys: Vec<(Uuid, u8)> = BAKES
                .iter()
                .filter(|b| expected.contains(&usize::from(b.te)))
                .filter_map(|b| cache_id(b, |kind| self.worn(kind)).map(|id| (id, b.te)))
                .collect();
            if !keys.is_empty() {
                self.serial = self.serial.wrapping_add(1);
                self.query = Some((self.serial, Instant::now()));
                commands.push(NetCommand::QueryCachedBakes { serial: self.serial, keys });
            }
        }
        if let Some((_, asked)) = self.query {
            if asked.elapsed() < QUERY_WAIT {
                return commands;
            }
            log::warn!("local bake: the region did not say which bakes it keeps");
            self.query = None;
        }
        let compose: HashSet<usize> = expected.iter().filter(|te| !self.cached.contains_key(te)).copied().collect();
        // the textures the bakes read, held and ready; none when the region
        // keeps every bake
        let mut sources = HashMap::new();
        let mut ready = true;
        self.wanted.clear();
        for texture in values
            .iter()
            .flat_map(|value| value.textures.values())
            .copied()
            .filter(|_| !compose.is_empty())
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
        self.baked = self
            .cached
            .iter()
            .filter(|(te, _)| expected.contains(te))
            .map(|(te, texture)| (*te, *texture))
            .collect();
        self.expected = expected;
        let lib = world.avatar_lib.clone();
        let token = self.generation.clone();
        let generation = token.load(Ordering::Relaxed);
        let (tx, rx) = crossbeam_channel::unbounded();
        self.job = Some(rx);
        self.dirty = None;
        log::info!(
            "local bake: {} wearables into {} bakes, {} kept by the region",
            values.len(),
            self.expected.len(),
            self.baked.len()
        );
        scene.jobs.execute(move || {
            if let Err(reason) = compose_all(&token, generation, &lib, values, sources, &compose, &tx) {
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
            self.sent = Some(self.baked.clone());
            self.rebake_pending = false;
            // the bakes are out: their textures need not stay at top priority
            self.wanted.clear();
            self.sent_inputs = Some(self.inputs.clone());
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
        let cache = BAKES
            .iter()
            .filter(|b| self.baked.contains_key(&usize::from(b.te)))
            .filter_map(|b| cache_id(b, |kind| self.worn(kind)).map(|id| (id, b.te)))
            .collect();
        Appearance {
            // DEFAULT_AGENT_DEPTH, DEFAULT_AGENT_WIDTH (indra_constants.h)
            size: glam::Vec3::new(0.45, 0.60, height),
            cache,
            texture_entry: aurora_prim::te::pack_texture_entry(&te, AVATAR_FACES),
            params,
        }
    }

    /// The worn assets of one wearable type, in layer order.
    fn worn(&self, kind: u8) -> Vec<Uuid> {
        self.worn_kinds
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, asset)| *asset)
            .collect()
    }

    /// The bakes just sent, once: our own avatar shows them.
    pub fn take_sent(&mut self) -> Option<HashMap<usize, Uuid>> {
        self.sent.take()
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

/// The job: every bake the region lacks composed and encoded, then the
/// visual parameters and the body's height from the same wearables.
fn compose_all(
    token: &AtomicU64,
    generation: u64,
    lib: &crate::scene::avatar::AvatarLibrary,
    values: Vec<Wearable>,
    sources: HashMap<Uuid, crate::scene::textures::PreviewSource>,
    compose: &HashSet<usize>,
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
    for region in defs.regions.iter().filter(|region| compose.contains(&region.te)) {
        if token.load(Ordering::Relaxed) != generation {
            return Ok(());
        }
        let rgba = bake::compose(defs, region, region.size, &worn, &global, &images)?;
        // LLViewerTexLayerSet::isVisible: nothing of the layer set shows
        let j2c = if rgba.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0) {
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
    fn a_new_session_bakes_the_same_outfit_again() {
        let worn = vec![(Uuid::from_u128(1), Uuid::from_u128(2))];
        let mut bakes = LocalBakes {
            inputs: worn.clone(),
            worn_kinds: vec![(1, Uuid::from_u128(2))],
            baked: [(8, Uuid::from_u128(8))].into_iter().collect(),
            sent: Some(HashMap::new()),
            sent_inputs: Some(worn.clone()),
            queried: true,
            serial: 4,
            tries: 2,
            ..Default::default()
        };
        bakes.held.insert(Uuid::from_u128(5));
        let running = bakes.generation.clone();
        let released = bakes.reset();
        assert_eq!(
            released.into_iter().collect::<Vec<_>>(),
            [Uuid::from_u128(5)],
            "the textures held are let go"
        );
        assert_ne!(running.load(Ordering::Relaxed), 0, "the job of the session left stops");
        assert!(bakes.inputs.is_empty(), "the same items count as a change at the next login");
        assert!(bakes.baked.is_empty() && bakes.sent.is_none() && bakes.dirty.is_none());
        assert_eq!(bakes.tries, 0);
        assert_ne!(bakes.inputs, worn);
        assert!(bakes.sent_inputs.is_none(), "a server bake asked at the next login is not skipped");
        assert!(!bakes.queried && bakes.query.is_none(), "the next region is asked what it keeps");
    }

    #[test]
    fn only_a_full_answer_from_a_region_that_does_not_bake_bakes_locally() {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let region = world.main_region.expect("region");
        let mut with = |caps: &[&str]| {
            world.regions.get_mut(&region).expect("region").caps =
                Arc::new(caps.iter().map(|c| ((*c).to_owned(), "https://cap.invalid/".to_owned())).collect());
            world.bakes_locally()
        };
        assert!(with(&["UploadBakedTexture", "FetchInventoryDescendents2"]), "OpenSim");
        assert!(
            !with(&["UploadBakedTexture", "FetchInventoryDescendents2", "UpdateAvatarAppearance"]),
            "a region that bakes"
        );
        assert!(!with(&["UploadBakedTexture"]), "a neighbour's partial seed answer");
    }

    #[test]
    fn a_region_that_keeps_asking_gets_one_rebake_per_bake_sent() {
        let mut bakes = LocalBakes::default();
        let t0 = Instant::now();
        assert!(bakes.rebake(t0));
        assert!(!bakes.rebake(t0 + Duration::from_secs(1)), "one already asked for");
        // the bakes went out (`receive`)
        bakes.rebake_pending = false;
        assert!(!bakes.rebake(t0 + Duration::from_secs(10)), "within the cooldown");
        assert!(bakes.rebake(t0 + REBAKE_COOLDOWN + Duration::from_secs(1)));
    }

    #[test]
    fn a_bake_given_up_on_lets_a_later_rebake_through() {
        let mut bakes = LocalBakes::default();
        let t0 = Instant::now();
        assert!(bakes.rebake(t0));
        bakes.tries = UPLOAD_TRIES;
        let request = Uuid::from_u128(1);
        bakes.uploads.insert(request, 0);
        assert!(bakes.uploaded(request, &Err("refused".into())));
        assert!(!bakes.rebake(t0 + Duration::from_secs(1)), "within the cooldown");
        assert!(
            bakes.rebake(t0 + REBAKE_COOLDOWN + Duration::from_secs(1)),
            "not stuck until the outfit changes"
        );
    }

    #[test]
    fn a_wearable_that_never_reads_is_left_out_instead_of_holding_the_bake() {
        let mut bakes = LocalBakes::default();
        let key = (Uuid::from_u128(1), Uuid::from_u128(2));
        for _ in 0..WEARABLE_TRIES {
            assert!(bakes.still_reading(key));
            bakes.requested.insert(key);
            bakes.loaded(key.0, key.1, &Err("unreadable".into()));
        }
        assert!(!bakes.still_reading(key), "given up on after {WEARABLE_TRIES} reads");
        bakes.requested.insert(key);
        bakes.loaded(key.0, key.1, &Ok(b"not a wearable".to_vec()));
        assert!(!bakes.still_reading(key));
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
    fn only_the_answer_to_the_open_question_counts() {
        let mut bakes = LocalBakes {
            query: Some((2, Instant::now())),
            ..Default::default()
        };
        bakes.cached_answer(1, &[(8, Uuid::from_u128(80))]);
        assert!(bakes.cached.is_empty() && bakes.query.is_some(), "an older question's answer");
        bakes.cached_answer(2, &[(8, Uuid::from_u128(80)), (9, Uuid::nil())]);
        assert_eq!(bakes.cached, HashMap::from([(8, Uuid::from_u128(80))]), "nil: the region lacks it");
        assert!(bakes.query.is_none());
    }

    #[test]
    fn a_server_bake_of_the_outfit_sent_is_not_redone() {
        let outfit = vec![(Uuid::from_u128(1), Uuid::from_u128(2))];
        let mut bakes = LocalBakes {
            inputs: outfit.clone(),
            sent_inputs: Some(outfit),
            ..Default::default()
        };
        bakes.refresh();
        assert!(bakes.dirty.is_none());
        // another outfit than the one sent is baked, asking the region first
        bakes.inputs.push((Uuid::from_u128(3), Uuid::from_u128(4)));
        bakes.refresh();
        assert!(bakes.dirty.is_some() && !bakes.queried);
    }

    #[test]
    fn a_bake_the_region_lacks_is_redone_without_asking() {
        let mut bakes = LocalBakes {
            cached: HashMap::from([(8, Uuid::from_u128(80))]),
            ..Default::default()
        };
        assert!(bakes.rebake(Instant::now()));
        assert!(bakes.dirty.is_some() && bakes.queried && bakes.cached.is_empty());
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
