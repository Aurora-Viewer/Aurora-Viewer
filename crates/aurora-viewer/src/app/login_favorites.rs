//! The session writes its favourite landmarks down for the login screen
//! (`login_favorites.rs`): LLFavoritesBarCtrl saves them a second after the
//! Favorites folder changes and LLFavoritesOrderStorage::destroyClass at
//! the end (indra/newview/llfavoritesbar.cpp, originally LGPL 2.1). Here a
//! one-second check compares the favourites with the stored ones, and the
//! end of the session writes them once more. Their places come from the
//! landmarks and the map (LLFavoritesOrderStorage::getSLURL), asked only
//! for the favourites not stored with their landmark yet.

use super::App;
use crate::login_favorites::{self as stored, Current, Favorite, Key};
use std::time::{Duration, Instant};

/// LLFavoritesBarCtrl's mItemsChangedTimer.
const EVERY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(super) struct Session {
    /// The session's grid and account, when the login screen remembers it
    /// (an account the catalogue does not keep stores nothing).
    key: Option<Key>,
    next: Option<Instant>,
}

impl Session {
    pub(super) fn session_started(&mut self, settings: &crate::settings::Settings, demo: bool) {
        let key = stored::key(&settings.login_uri(), &settings.username);
        self.key = ((demo || settings.remember_username) && !key.1.is_empty()).then_some(key);
        self.next = None;
    }

    pub(super) fn session_ended(&mut self) {
        *self = Session::default();
    }

    pub(super) fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }
}

impl App {
    /// The favourite chosen in the start menu of the account about to log
    /// in, while that account still has it.
    pub(super) fn chosen_start_favorite(&self) -> Option<&Favorite> {
        let key = stored::key(&self.settings.login_uri(), &self.settings.username);
        let (chosen_for, item) = self.login_form.start_favorite.as_ref()?;
        if *chosen_for != key {
            return None;
        }
        self.login_form.favorites.favorites(&key).iter().find(|f| f.item == *item)
    }

    /// saveFavoritesRecord, once a second while logged in, or `now` at the
    /// end of the session (no new request then).
    pub(super) fn keep_login_favorites(&mut self, now: bool) {
        use crate::world::inventory::FetchState;
        use crate::world::landmarks::Resolved;
        let Some(key) = self.login_favorites.key().cloned() else {
            return;
        };
        let at = Instant::now();
        if !now && self.login_favorites.next.is_some_and(|next| at < next) {
            return;
        }
        self.login_favorites.next = Some(at + EVERY);
        if !self.login_form.favorites.shown(&key) {
            return;
        }
        // isInventoryUsable: the Favorites folder read, else nothing changes
        let folder = crate::world::appearance::system_folder(&self.world.inventory, crate::world::favorites::FOLDER_FAVORITES);
        match folder.and_then(|f| self.world.inventory.folders.get(&f)).map(|f| f.state) {
            Some(FetchState::Fetched) => {}
            Some(FetchState::Unknown | FetchState::Failed) if !now => {
                if let Some(folder) = folder {
                    self.world.inventory.request(folder);
                }
                return;
            }
            _ => return,
        }
        let ids = self.world.favorites.list(&self.world.inventory).to_vec();
        let mut current = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(item) = self.world.inventory.items.get(&id) else {
                continue;
            };
            let (asset, name) = (item.asset_id, item.name.clone());
            let known = self.login_form.favorites.knows(&key, id, asset);
            let resolved = if now || known {
                self.world.landmarks.peek(asset)
            } else {
                self.world.landmarks.resolve(asset)
            };
            let place = match resolved {
                Resolved::Ready { global, .. } => match self.world.map.sim_at_global(global.x, global.y) {
                    Some(((gx, gy), sim)) => Some((
                        sim.name.clone(),
                        [
                            (global.x - gx as f64 * 256.0) as f32,
                            (global.y - gy as f64 * 256.0) as f32,
                            global.z as f32,
                        ],
                    )),
                    None => {
                        if !now && !known {
                            let (x, y) = ((global.x / 256.0) as i64, (global.y / 256.0) as i64);
                            self.world.map.want_regions(x, y, x, y);
                        }
                        None
                    }
                },
                _ => None,
            };
            current.push(Current {
                item: id,
                asset,
                name,
                place,
            });
        }
        let list = stored::merge(self.login_form.favorites.favorites(&key), current);
        let count = list.len();
        if self.login_form.favorites.update(&key, list) {
            log::info!("login favourites: {count} stored for this account");
        }
    }
}
