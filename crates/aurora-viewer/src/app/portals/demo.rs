//! Real inventory world-drop input, and a walk through our own portal with
//! the actual movement keys.
use super::*;

fn drop_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_PORTALS").ok().filter(|mode| mode.starts_with("drop-")))
        .as_deref()
}

impl App {
    pub(in crate::app) fn seed_portal_drop_demo(&mut self) -> bool {
        let Some(mode) = drop_mode().filter(|_| self.demo) else {
            return false;
        };
        if self.frame_count >= 420 && !self.portal_ui.demo_seeded {
            self.portal_ui.demo_seeded = true;
            self.portal_ui.open = false;
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.inventory = mode == "drop-focus";
            self.panels.places = mode.contains("places") || mode.contains("favorites") || mode.contains("history");
            self.panels.search = mode.contains("search");
            if self.panels.search {
                let tab = crate::world::search::Tab::Places;
                self.search_ui.open_tab(tab);
                self.search_ui.set_text(tab, "aurora");
                let requests = if self.settings.classic_search {
                    self.search_ui.request(tab, &self.settings.search).into_iter().collect()
                } else {
                    self.search_ui.aurora_requests(&self.settings.search)
                };
                let access = self.world.agent_access();
                for request in requests {
                    self.world.search.find(request, access, Instant::now());
                }
                ui::widgets::place_floater(&self.egui_ctx, ui::search::WINDOW, egui::pos2(20.0, 85.0), egui::vec2(720.0, 560.0));
                return true;
            }
            if self.panels.places {
                self.places_ui.open_tab(if mode.contains("history") {
                    2
                } else {
                    usize::from(!mode.contains("favorites"))
                });
                ui::widgets::place_floater(&self.egui_ctx, "places", egui::pos2(20.0, 85.0), egui::vec2(420.0, 560.0));
            }
            if self.panels.inventory {
                ui::widgets::place_floater(&self.egui_ctx, "inventory", egui::pos2(20.0, 85.0), egui::vec2(460.0, 560.0));
            }
            if mode.contains("history") {
                let region = self.world.main_region.unwrap_or_default();
                self.world.tp_storage.items.push(crate::world::tphistory::HistoryItem {
                    title: "Portal history destination".into(),
                    global: glam::DVec3::new((region >> 32) as f64 + 140.0, f64::from(region as u32) + 120.0, 25.0),
                    date: crate::world::portals::unix_time() as f64,
                    slurl: String::new(),
                });
                return true;
            }
            let (item, asset) = crate::demo::place::landmark_ids(0);
            self.world.inventory.add_items(vec![aurora_net::inventory::InvItem {
                id: item,
                parent: ui::places::system_folder(&self.world.inventory, if mode.contains("favorites") { 23 } else { 3 })
                    .unwrap_or(self.world.inventory.root),
                asset_id: if mode == "drop-invalid" {
                    uuid::Uuid::from_u128(0x0A77_0BAD)
                } else {
                    asset
                },
                asset_type: 3,
                inv_type: 3,
                owner: self.world.agent_id,
                name: aurora_i18n::tr!("common-landmark").into(),
                ..Default::default()
            }]);
        }
        if mode == "drop-focus" && self.frame_count == 562 {
            log::info!(
                "portal drop focus: inventory open {}, keyboard focus {}",
                self.panels.inventory,
                self.egui_ctx.egui_wants_keyboard_input()
            );
        }
        true
    }

    pub(in crate::app) fn portal_demo_input(&mut self, raw: &mut egui::RawInput) {
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_PORTALS")
            && mode.starts_with("walk")
        {
            self.portal_keyboard_demo_input(raw, mode == "walk-chat");
            return;
        }
        let Some(mode) = drop_mode() else { return };
        if !(480..=560).contains(&self.frame_count) {
            return;
        }
        let bounds = self.egui_ctx.content_rect();
        let point = bounds.min + egui::vec2(bounds.width() * 0.65, bounds.height() * 0.62);
        raw.focused = true;
        raw.events.retain(|event| !matches!(event, egui::Event::WindowFocused(false)));
        let modifiers = egui::Modifiers::default();
        raw.events.push(egui::Event::PointerMoved(point));
        if self.frame_count == 480 || self.frame_count == 560 && !mode.ends_with("hover") {
            raw.events.push(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: self.frame_count == 480,
                modifiers,
            });
        }
        if self.frame_count == 482 {
            let item = crate::demo::place::landmark_ids(0).0;
            if mode.contains("search") {
                if let Some(row) = self
                    .world
                    .search
                    .list(crate::world::search::Tab::Places)
                    .and_then(|list| list.rows.first())
                    && let crate::world::search::Key::Id(id) = row.key
                {
                    egui::DragAndDrop::set_payload(
                        &self.egui_ctx,
                        ui::places::drag::PlaceDrag {
                            target: ui::places::drag::Target::Parcel(id),
                            favorite: None,
                            name: row.name.clone(),
                            thumbnail: None,
                        },
                    );
                }
            } else if mode.contains("history") {
                if let Some(place) = self.world.tp_storage.items.last() {
                    egui::DragAndDrop::set_payload(
                        &self.egui_ctx,
                        ui::places::drag::PlaceDrag {
                            target: ui::places::drag::Target::History(place.global),
                            favorite: None,
                            name: place.title.clone(),
                            thumbnail: None,
                        },
                    );
                }
            } else if mode.contains("places") || mode.contains("favorites") {
                egui::DragAndDrop::set_payload(
                    &self.egui_ctx,
                    ui::places::drag::PlaceDrag {
                        target: ui::places::drag::Target::Landmark(item),
                        favorite: mode.contains("favorites").then_some(item),
                        name: self
                            .world
                            .inventory
                            .items
                            .get(&item)
                            .map_or_else(String::new, |item| item.name.clone()),
                        thumbnail: None,
                    },
                );
            } else {
                egui::DragAndDrop::set_payload(&self.egui_ctx, ui::inventory::InvDrag(item));
            }
            if mode == "drop-focus" {
                self.egui_ctx
                    .memory_mut(|memory| memory.request_focus(egui::Id::unique(("inventory_clipboard_focus", self.inventory_ui.ui_id))));
            }
        }
    }

    fn portal_keyboard_demo_input(&mut self, raw: &mut egui::RawInput, chat: bool) {
        raw.focused = true;
        raw.events.retain(|event| !matches!(event, egui::Event::WindowFocused(false)));
        if self.frame_count == 420 {
            self.panels.chat = false;
            self.chat_ui.input.clear();
            if chat {
                self.chat_ui.focus_request = true;
            } else {
                self.demo_release_keyboard();
            }
        }
        let forward = Input::key(KeyCode::ArrowUp);
        if self.frame_count == 600 {
            self.down.insert(forward.clone());
        }
        if self.frame_count == 810 {
            self.down.remove(&forward);
        }
        if (600..=840).contains(&self.frame_count) && self.frame_count.is_multiple_of(60) {
            log::info!(
                "portal keyboard demo: frame {}, chat {}, keyboard focus {}, forward {}, position {:?}, journey {}",
                self.frame_count,
                self.chat_ui.has_focus,
                self.egui_ctx.egui_wants_keyboard_input(),
                self.input.forward,
                self.world.agent.position,
                self.portal_journey.is_some()
            );
        }
    }
}
