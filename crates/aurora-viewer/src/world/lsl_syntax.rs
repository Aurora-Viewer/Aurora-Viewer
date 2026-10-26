//! Which LSL keywords the script editor uses: the agent's region names its
//! syntax in SimulatorFeatures (LSLSyntaxId); a new one is fetched once
//! (cache, else the LSLSyntax capability) and then used, a region without
//! one goes back to the bundled keywords.
//!
//! Port of LLSyntaxIdLSL::handleRegionChanged / syntaxIdChanged /
//! handleFileFetched (indra/newview/llsyntaxid.cpp, originally LGPL 2.1).
//! The catalogs themselves are `crate::lsl_keywords`.

use super::World;
use crate::lsl_keywords::{self, Catalog};
use aurora_net::NetCommand;
use aurora_net::lsl_syntax::Keyword;
use uuid::Uuid;

impl World {
    /// Once a frame: follow the agent's region's syntax; the fetch to send
    /// when it names one not met yet.
    pub fn follow_lsl_syntax(&mut self) -> Option<NetCommand> {
        // its SimulatorFeatures not read yet: keep the keywords in use
        let id = self.main()?.lsl_syntax_id?;
        if id.is_nil() {
            if lsl_keywords::current_id().is_some() {
                log::info!("lsl syntax: this region names none, back to the bundled keywords");
                lsl_keywords::use_bundled();
            }
            return None;
        }
        if lsl_keywords::current_id() == Some(id) || lsl_keywords::use_known(id) {
            return None;
        }
        // asked once a session; a failure leaves the keywords in use
        if !self.lsl_syntax_asked.insert(id) {
            return None;
        }
        log::info!("lsl syntax: region syntax {id}");
        Some(NetCommand::FetchLslSyntax {
            id,
            cache_dir: crate::settings::cache_dir().join("lsl"),
        })
    }

    /// The keywords of `id` came (handleFileFetched): used if the agent's
    /// region still names it. A failure keeps the bundled ones
    /// (loadDefaultKeywordsIntoLLSD).
    pub(super) fn on_lsl_syntax(&mut self, id: Uuid, keywords: Result<Vec<Keyword>, String>) {
        let wanted = self.main().and_then(|r| r.lsl_syntax_id) == Some(id);
        match keywords {
            Ok(keywords) if wanted => lsl_keywords::install(Catalog::from_keywords(id, keywords)),
            // another region by now: ask again if it comes back
            Ok(_) => {
                self.lsl_syntax_asked.remove(&id);
            }
            Err(_) if wanted => lsl_keywords::use_bundled(),
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::NetEvent;

    /// A world whose region read its SimulatorFeatures. The keywords are
    /// never installed here: the other tests read the global catalog.
    fn world(id: Uuid) -> World {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let handle = aurora_net::origin_to_handle(256_000, 256_512);
        w.main_region = Some(handle);
        w.ensure_region(handle, (256, 256));
        w.apply(NetEvent::SimulatorFeatures {
            handle,
            voice_server_type: String::new(),
            avatar_hover_height: false,
            materials_rate: None,
            materials_max: None,
            lsl_syntax_id: id,
            grid_status_feed: String::new(),
            grid_status_page: String::new(),
        });
        w
    }

    #[test]
    fn a_region_syntax_is_asked_once() {
        let id = Uuid::from_u128(0x1515);
        let mut w = world(id);
        match w.follow_lsl_syntax() {
            Some(NetCommand::FetchLslSyntax { id: asked, cache_dir }) => {
                assert_eq!(asked, id);
                assert!(cache_dir.ends_with("lsl"));
            }
            other => panic!("{other:?}"),
        }
        assert!(w.follow_lsl_syntax().is_none(), "in flight");
        // it failed: the bundled keywords stay, and it is not asked again
        w.apply(NetEvent::LslSyntax {
            id,
            keywords: Err("status 404".into()),
        });
        assert!(w.follow_lsl_syntax().is_none());
        // the answer for a region left meanwhile is asked again later
        let other = Uuid::from_u128(0x2626);
        w.lsl_syntax_asked.insert(other);
        w.apply(NetEvent::LslSyntax {
            id: other,
            keywords: Ok(Vec::new()),
        });
        assert!(!w.lsl_syntax_asked.contains(&other));
    }

    #[test]
    fn no_syntax_id_no_request() {
        assert!(world(Uuid::nil()).follow_lsl_syntax().is_none());
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        assert!(w.follow_lsl_syntax().is_none(), "no region yet");
    }
}
