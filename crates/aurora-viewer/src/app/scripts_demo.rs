//! The script editor's offline demo: the steps `AURORA_DEMO_SCRIPTS` walks,
//! the keys it sends to the source and the commands it answers, kept apart
//! from what the editor does on the grid (`app/scripts.rs`).

use super::scripts::nested_prims;
use super::scripts::{
    COLORS_DEMO_SOURCE, INLAY_DEMO_SOURCE, LAMP_DEMO_SOURCE, LINT_DEMO_SOURCE, LUA_DEMO_SOURCE, LUA_LINT_DEMO_SOURCE, NESTED_DEMO_SOURCE,
    PANEL_DEMO_SOURCE, SLUA_DEMO_SOURCE, SPARKLE_LUA_SOURCE, SYNTAX_DEMO_SOURCE, WORD_WRAP_DEMO_SOURCE,
};
use super::*;
use crate::ui::scripts::Lang;
use aurora_net::build::perm;
use aurora_net::scripts::{DEFAULT_SOURCE, ScriptSaved, ScriptTarget};
use uuid::Uuid;

impl App {
    pub(super) fn demo_script_input(&self, raw: &mut egui::RawInput) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_SCRIPTS") else { return };
        if matches!(mode.as_str(), "insert-filter" | "insert-empty") {
            raw.events
                .extend(self.script_editors.demo_insertion_events(&self.egui_ctx, self.frame_count, &mode));
        }
        if matches!(mode.as_str(), "folding" | "object-folding") {
            raw.events.extend(self.script_editors.demo_folding_events(self.frame_count));
        }
        if matches!(mode.as_str(), "root-menu" | "script-menu") {
            let root = self
                .script_editors
                .documents
                .iter()
                .rev()
                .find(|e| e.is_open() && !e.linkset.is_nil() && e.linkset != crate::ui::scripts::INVENTORY_WINDOW)
                .map(|e| e.linkset);
            // the linkset's window over the inventory's, one frame before the
            // click: egui routes an event by the layer order it knew
            if let Some(root) = root
                && self.frame_count == 390
            {
                let window = egui::LayerId::new(egui::Order::Middle, egui::Id::unique(format!("script_linkset_{root}")));
                self.egui_ctx.move_to_top(window);
            }
            // a right click on the root row, or on a script's, where the last
            // frame drew it
            if let Some(root) = root
                && matches!(self.frame_count, 400 | 401)
            {
                let item = self
                    .script_editors
                    .documents
                    .iter()
                    .find(|e| e.is_open() && e.linkset == root)
                    .map(|e| e.target.item);
                let row = if mode == "script-menu" {
                    item.and_then(|item| {
                        self.egui_ctx
                            .data(|d| d.get_temp::<(egui::Rect, egui::LayerId)>(crate::ui::scripts::script_row_id(root, item)))
                    })
                } else {
                    self.egui_ctx
                        .data(|d| d.get_temp::<(egui::Rect, egui::LayerId)>(crate::ui::scripts::root_row_id(root)))
                };
                if let Some((rect, _)) = row {
                    let at = rect.center();
                    raw.events.push(egui::Event::PointerMoved(at));
                    raw.events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Secondary,
                        pressed: self.frame_count == 400,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
        }
        if mode == "linkset" && self.frame_count == 400 {
            // Find on the script in front, as Ctrl+F does
            raw.events.push(egui::Event::Key {
                key: egui::Key::F,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            });
        }
        if self.frame_count == 321
            && matches!(
                mode.as_str(),
                "comment-lines" | "comment-block" | "multiline-indent" | "multiline-outdent"
            )
        {
            let (key, modifiers) = match mode.as_str() {
                "comment-lines" => (egui::Key::K, egui::Modifiers::COMMAND),
                "comment-block" => (egui::Key::K, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT),
                "multiline-outdent" => (egui::Key::Tab, egui::Modifiers::SHIFT),
                _ => (egui::Key::Tab, egui::Modifiers::NONE),
            };
            raw.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
        }
        if mode == "format" && self.frame_count == 420 {
            // the Code menu's Format, on the key the menu names
            raw.events.push(egui::Event::Key {
                key: egui::Key::F,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::ALT | egui::Modifiers::SHIFT,
            });
        }
        if mode == "format" {
            raw.events.extend(self.script_editors.demo_code_events(self.frame_count));
        }
        if matches!(mode.as_str(), "color-picker" | "color-accept" | "color-cancel") {
            raw.events.extend(self.script_editors.demo_color_events(self.frame_count, &mode));
        }
        if matches!(mode.as_str(), "close-brace" | "object-close-brace") {
            if matches!(self.frame_count, 321 | 323) {
                raw.events.push(egui::Event::Text("}".into()));
            }
            if self.frame_count == 322 {
                raw.events.push(egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if self.frame_count == 323 && matches!(mode.as_str(), "tabstops" | "object-tabstops") {
            for key in [egui::Key::Backspace, egui::Key::Tab] {
                raw.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            raw.events.push(egui::Event::Text("// tab stops".into()));
            raw.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if self.frame_count == 321 && matches!(mode.as_str(), "indent" | "object-indent") {
            raw.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            raw.events.push(egui::Event::Text("// indentation".into()));
        }
    }

    fn demo_say(&mut self, source: Uuid, name: &str, text: &str, debug: bool) {
        self.world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: name.to_owned(),
            source_id: source,
            owner_id: self.world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: if debug {
                aurora_net::ChatType::Debug
            } else {
                aurora_net::ChatType::OwnerSay
            },
            region: self.world.main_region,
            position: Vec3::ZERO,
            message: text.into(),
        }));
    }

    fn demo_script_linkset(&mut self) {
        let lint = false;
        for event in crate::demo::script_linkset_events() {
            self.world.apply(event);
        }
        let Some(idx) = self
            .world
            .objects
            .iter()
            .find(|(_, o)| o.key.local_id == 96_100 && Some(o.key.region) == self.world.main_region)
            .map(|(i, _)| i)
        else {
            return;
        };
        self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
        let mut opened: Vec<(ScriptTarget, String)> = Vec::new();
        for (order, prim) in crate::build::family(&self.world, idx).into_iter().enumerate() {
            let Some(object) = self.world.objects.get(prim).map(|o| o.full_id) else {
                continue;
            };
            self.build.request_contents(&self.world, prim, true);
            self.flush_build();
            let Some(listing) = self.build.contents.objects.get_mut(&object) else {
                continue;
            };
            let Some(script) = listing.items.iter_mut().find(|item| item.asset_type == 10) else {
                continue;
            };
            let name = match order {
                0 => aurora_i18n::tr!("script-demo-panel-script"),
                1 => aurora_i18n::tr!("script-demo-door-script"),
                _ => aurora_i18n::tr!("script-demo-lamp-script"),
            };
            script.name = name.to_owned();
            // the demo's own listing keeps them, should it be asked again
            self.build_demo.name_script(object, Some(script.item_id), name);
            let target = ScriptTarget {
                object,
                item: script.item_id,
            };
            // the linter's demo: the panel's script is the one it has
            // something to say about
            let source = if order == 0 {
                if lint { LINT_DEMO_SOURCE } else { PANEL_DEMO_SOURCE }
            } else {
                LAMP_DEMO_SOURCE
            };
            opened.push((target, source.to_owned()));
            // one group holds two scripts, as a prim often does
            // the group's second script is Lua (SST_LUA in its flags): the
            // tree shows both languages side by side
            if order == 2
                && let Some(item) = self
                    .build_demo
                    .name_script(object, None, aurora_i18n::tr!("script-demo-lamp-sparkle"))
            {
                let mut second = script.clone();
                second.item_id = item;
                second.name = aurora_i18n::tr!("script-demo-lamp-sparkle").into();
                second.flags = (second.flags & !0xff) | 1;
                opened.push((ScriptTarget { object, item }, SPARKLE_LUA_SOURCE.to_owned()));
                listing.items.push(second);
            }
        }
        let mut targets = Vec::new();
        for (target, source) in opened {
            targets.push(target);
            self.demo_script_sources.insert(target, source);
        }
        // the panel's script and the lamp's second one: two tabs to switch --
        // the linter's demo keeps the panel's in front, the one it has
        // something to say about
        let opening = if lint {
            vec![targets.first()]
        } else {
            vec![targets.first(), targets.last()]
        };
        for target in opening.into_iter().flatten() {
            self.open_script(*target);
        }
    }

    /// `AURORA_DEMO_SCRIPTS=nested`: a linkset whose prims go two levels deep,
    /// each with a script, the root's and the deepest prim's open in the window.
    fn demo_script_nested(&mut self) {
        for event in crate::demo::script_nested_events() {
            self.world.apply(event);
        }
        let Some(idx) = self
            .world
            .objects
            .iter()
            .find(|(_, o)| o.key.local_id == 96_200 && Some(o.key.region) == self.world.main_region)
            .map(|(i, _)| i)
        else {
            return;
        };
        self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
        let mut opened: Vec<(u32, ScriptTarget, String)> = Vec::new();
        // the family one prim after the one that holds it, each named for
        // itself (the door holds the handle, the lamp hangs from the root)
        for (prim, _depth) in nested_prims(&self.world, idx) {
            let Some(object) = self.world.objects.get(prim).map(|o| o.full_id) else {
                continue;
            };
            let local = self.world.objects.get(prim).map(|o| o.key.local_id).unwrap_or_default();
            let (name, source) = match local {
                96_200 => (aurora_i18n::tr!("script-demo-panel-script"), PANEL_DEMO_SOURCE),
                96_201 => (aurora_i18n::tr!("script-demo-door-script"), NESTED_DEMO_SOURCE),
                96_202 => (aurora_i18n::tr!("script-demo-handle-script"), NESTED_DEMO_SOURCE),
                _ => (aurora_i18n::tr!("script-demo-lamp-script"), LAMP_DEMO_SOURCE),
            };
            self.build.request_contents(&self.world, prim, true);
            self.flush_build();
            let Some(listing) = self.build.contents.objects.get_mut(&object) else {
                continue;
            };
            let Some(script) = listing.items.iter_mut().find(|item| item.asset_type == 10) else {
                continue;
            };
            script.name = name.to_owned();
            self.build_demo.name_script(object, Some(script.item_id), name);
            let target = ScriptTarget {
                object,
                item: script.item_id,
            };
            opened.push((local, target, source.to_owned()));
        }
        let mut targets = Vec::new();
        for (local, target, source) in opened {
            targets.push((local, target));
            self.demo_script_sources.insert(target, source);
        }
        // the root's script and the deepest prim's: the tree shows the levels
        // between them
        let root = targets.iter().find(|(local, _)| *local == 96_200).map(|(_, target)| *target);
        let handle = targets.iter().find(|(local, _)| *local == 96_202).map(|(_, target)| *target);
        for target in [root, handle].into_iter().flatten() {
            self.open_script(target);
        }
    }

    pub(super) fn demo_script_steps(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_SCRIPTS") else { return };
        // AURORA_DEMO_SCRIPTS=simple: the old layout, a window per script
        if mode == "simple" {
            self.settings.script_editor.simple = true;
        }
        let syntax_demo = matches!(
            mode.as_str(),
            "syntax"
                | "object-syntax"
                | "syntax-readonly"
                | "workbench"
                | "object-workbench"
                | "insert-filter"
                | "insert-empty"
                | "search"
                | "search-left"
                | "search-inline"
                | "search-overlay"
                | "search-outside"
                | "preferences"
                | "help"
                | "colors"
                | "color-picker"
                | "color-accept"
                | "color-cancel"
                | "comment-lines"
                | "comment-block"
                | "multiline-indent"
                | "multiline-outdent"
                | "folding"
                | "object-folding"
                | "word-wrap"
                | "no-word-wrap"
                | "word-wrap-preferences"
                | "shortcuts"
                | "slua"
                | "format"
                | "inlays"
        );
        if self.frame_count == 300 && matches!(mode.as_str(), "linkset" | "split") {
            // the split's demo: the two open scripts side by side, the divider
            // a little left of the middle
            if mode == "split" {
                self.script_editors.demo_split();
            }
            // what the region reports of the object's scripts, for the bar:
            // the demo's bridge has no endpoint here, so it gives the figure
            // the query would bring (four scripts, three of them running)
            if matches!(mode.as_str(), "linkset" | "split")
                && let Some(root) = self
                    .script_editors
                    .documents
                    .iter()
                    .find(|e| !e.linkset.is_nil())
                    .map(|e| e.linkset)
            {
                self.script_editors.script_stats(&aurora_net::bridge::query::ScriptInfo {
                    object: root,
                    name: aurora_i18n::tr!("script-demo-panel-script").to_string(),
                    running: 3,
                    total: 4,
                    memory: 4 * 65_536,
                    cpu_seconds: 0.0,
                    character_seconds: 0.0,
                });
            }
            // what the object's scripts say to their owner, as the running
            // scripts would say it: through the chat, so that the line takes
            // the way a grid's does into the editor's Console tab; the second
            // one comes from another prim of the linkset than the tab in front
            if let Some(target) = self.script_editors.documents.iter().find(|e| e.is_open()).map(|e| e.target) {
                let name = crate::build::replace::object_name(&self.build, &self.world, target.object);
                let other = self
                    .script_editors
                    .documents
                    .iter()
                    .find(|e| e.is_open() && e.target.object != target.object)
                    .map(|e| e.target.object)
                    .unwrap_or(target.object);
                self.demo_say(target.object, &name, "Panel ready.", false);
                self.demo_say(other, &name, "12 avatars in the plaza.", false);
                self.demo_say(
                    target.object,
                    &name,
                    "llSay: too many characters in the message (script warning)",
                    true,
                );
            }
        }
        // AURORA_DEMO_SCRIPTS=console: a log long enough to scroll, then a
        // line now and then, for the list keeping to its end as they come and
        // staying where the reader pushed it once they scroll back up
        if mode == "console"
            && (self.frame_count == 300 || (self.frame_count > 300 && self.frame_count < 700 && self.frame_count.is_multiple_of(40)))
            && let Some(target) = self.script_editors.documents.iter().find(|e| e.is_open()).map(|e| e.target)
        {
            let name = crate::build::replace::object_name(&self.build, &self.world, target.object);
            if self.frame_count == 300 {
                for n in 1..=40 {
                    self.demo_say(target.object, &name, &format!("the panel says line {n}"), false);
                }
            } else {
                let n = self.frame_count;
                self.demo_say(target.object, &name, &format!("still running, line {n}"), false);
            }
        }
        // AURORA_DEMO_SCRIPTS=inventory: two folders of the inventory get a
        // script each, so that the tree groups them as the grid's would, and
        // one of them keeps an edit: the star
        if self.frame_count == 320 && mode == "inventory" {
            use aurora_net::inventory::InvItem;
            let mut folders: Vec<(String, Uuid)> = self
                .world
                .inventory
                .folders
                .values()
                .filter(|folder| {
                    !folder.library
                        && folder.info.id != self.world.inventory.root
                        && folder.state == crate::world::inventory::FetchState::Fetched
                })
                .map(|folder| (folder.info.name.to_lowercase(), folder.info.id))
                .collect();
            folders.sort();
            let mut opened = Vec::new();
            for (n, (_, folder)) in folders.into_iter().take(2).enumerate() {
                let item = InvItem {
                    id: Uuid::new_v4(),
                    parent: folder,
                    owner: self.world.agent_id,
                    creator: self.world.agent_id,
                    name: if n == 0 {
                        aurora_i18n::tr!("script-demo-door-script").to_string()
                    } else {
                        aurora_i18n::tr!("script-demo-panel-script").to_string()
                    },
                    asset_type: 10,
                    inv_type: 10,
                    owner_mask: 0x7fff_ffff,
                    next_owner_mask: 0x7fff_ffff,
                    base_mask: 0x7fff_ffff,
                    ..Default::default()
                };
                opened.push(ScriptTarget {
                    object: Uuid::nil(),
                    item: item.id,
                });
                self.world.inventory.add_items(vec![item]);
            }
            for target in &opened {
                self.open_script(*target);
            }
            if let Some(first) = opened.first() {
                let source = DEFAULT_SOURCE.replace("Hello, Avatar!", "Bonjour, Aurora !");
                let _ = self.script_editors.demo_edit(*first, source, false);
            }
        }
        if mode == "search-linkset" {
            // Find in a bar under the linkset window's toolbar, not beside it
            self.settings.script_editor.search_inline = true;
        }
        if self.frame_count == 280 {
            self.panels.perf = false;
            if matches!(
                mode.as_str(),
                "linkset" | "quit" | "split" | "simple" | "root-menu" | "script-menu" | "new"
            ) {
                self.demo_script_linkset();
            }
            if mode == "nested" {
                self.demo_script_nested();
            }
            // AURORA_DEMO_SCRIPTS=new: the page that asks which language a new
            // script will be written in, for the linkset's root prim (Lua is
            // greyed on this demo region, which does not run SLua)
            if mode == "new"
                && let Some(root) = self
                    .script_editors
                    .documents
                    .iter()
                    .find(|e| !e.linkset.is_nil())
                    .map(|e| e.linkset)
            {
                let prim = crate::build::replace::object_name(&self.build, &self.world, root);
                let lua = self.world.main().is_some_and(|region| region.lua_scripts);
                self.script_editors
                    .demo_new_script(root, &prim, if lua { Lang::Lua } else { Lang::Lsl });
            }
            if mode == "quit" {
                // AURORA_DEMO_SCRIPTS=quit: a quit chosen while scripts ask
                // about their text is held, and goes on once the last one is
                // discarded (see the frames below).
                for e in &mut self.script_editors.documents {
                    if e.is_open() {
                        e.source.push_str("\n// an edit that is not saved yet\n");
                    }
                }
                log::info!(
                    "script demo quit: {} scripts with unsaved text",
                    self.script_editors.documents.len()
                );
            }
            if matches!(mode.as_str(), "word-wrap" | "no-word-wrap" | "word-wrap-preferences") {
                self.settings.script_editor.word_wrap = mode != "no-word-wrap";
            }
            if matches!(mode.as_str(), "colors" | "color-picker" | "color-accept" | "color-cancel") {
                self.world.notifications.list.clear();
            }
            if !matches!(
                mode.as_str(),
                "object"
                    | "object-readonly"
                    | "object-save"
                    | "object-syntax"
                    | "object-indent"
                    | "object-tabstops"
                    | "object-close-brace"
                    | "object-folding"
                    | "experience"
                    | "experience-save"
                    | "object-workbench"
                    | "object-preprocessor"
                    | "linkset"
                    | "simple"
                    | "split"
                    | "nested"
                    | "new"
            ) {
                crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
                let item = Uuid::from_u128(8102);
                if mode == "slua" {
                    // SST_LUA in the item's flags, on a region that runs SLua
                    if let Some(it) = self.world.inventory.items.get_mut(&item) {
                        it.flags |= 1;
                    }
                    if let Some(region) = self.world.main_region.and_then(|h| self.world.regions.get_mut(&h)) {
                        region.lua_scripts = true;
                    }
                }
                if mode == "readonly"
                    && let Some(it) = self.world.inventory.items.get_mut(&item)
                {
                    it.owner_mask &= !perm::MODIFY;
                }
                if mode == "syntax-readonly"
                    && let Some(mut it) = self.world.inventory.items.get(&item).cloned()
                {
                    // Copy-only library source uses the real read-only permission path.
                    it.parent = self.world.inventory.lib_root;
                    it.owner = self.world.inventory.lib_owner;
                    it.owner_mask &= !perm::MODIFY;
                    // (what the agent holds on a Library item: rules::agent_mask)
                    it.everyone_mask = it.owner_mask;
                    self.world.inventory.add_items(vec![it]);
                }
                self.panels.inventory = true;
                self.inventory_ui.show_original(&self.world.inventory, item);
                let target = ScriptTarget { object: Uuid::nil(), item };
                if matches!(mode.as_str(), "lua" | "lua-lints")
                    && let Some(it) = self.world.inventory.items.get_mut(&item)
                {
                    it.flags = (it.flags & !0xff) | 1;
                    let source = if mode == "lua-lints" {
                        LUA_LINT_DEMO_SOURCE
                    } else {
                        LUA_DEMO_SOURCE
                    };
                    self.demo_script_sources.insert(target, source.into());
                }
                if syntax_demo {
                    self.demo_script_sources.insert(
                        target,
                        if matches!(mode.as_str(), "colors" | "color-picker" | "color-accept" | "color-cancel") {
                            COLORS_DEMO_SOURCE
                        } else if matches!(mode.as_str(), "word-wrap" | "no-word-wrap" | "word-wrap-preferences") {
                            WORD_WRAP_DEMO_SOURCE
                        } else if mode == "slua" {
                            SLUA_DEMO_SOURCE
                        } else {
                            SYNTAX_DEMO_SOURCE
                        }
                        .into(),
                    );
                }
                self.open_script(target);
            }
            if matches!(
                mode.as_str(),
                "object"
                    | "object-readonly"
                    | "object-save"
                    | "object-syntax"
                    | "object-indent"
                    | "object-tabstops"
                    | "object-close-brace"
                    | "object-folding"
                    | "experience"
                    | "experience-save"
                    | "both"
                    | "object-workbench"
                    | "object-preprocessor"
            ) {
                let idx = self
                    .world
                    .objects
                    .iter()
                    // The plaza torus; DemoSim supplies its editable properties.
                    .find(|(_, o)| o.key.local_id == 117 && Some(o.key.region) == self.world.main_region)
                    .map(|(i, _)| i);
                if let Some(idx) = idx {
                    self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
                    self.build.open_build(crate::build::Tool::Edit);
                    self.build.tab = 4;
                    self.build.request_contents(&self.world, idx, true);
                    self.flush_build();
                    if let Some(object) = self.world.objects.get(idx).map(|o| o.full_id)
                        && let Some(item) = self
                            .build
                            .contents
                            .objects
                            .get(&object)
                            .and_then(|inv| inv.items.iter().find(|i| i.asset_type == 10))
                            .map(|it| it.item_id)
                    {
                        let target = ScriptTarget { object, item };
                        if mode == "object-readonly"
                            && let Some(inv) = self.build.contents.objects.get_mut(&object)
                            && let Some(script) = inv.items.iter_mut().find(|it| it.item_id == item)
                        {
                            script.owner_mask &= !perm::MODIFY;
                        }
                        if syntax_demo {
                            self.demo_script_sources.insert(target, SYNTAX_DEMO_SOURCE.into());
                        }
                        self.open_script(target);
                    }
                }
            }
        }
        if self.frame_count == 320
            && matches!(
                mode.as_str(),
                "comment-lines" | "comment-block" | "multiline-indent" | "multiline-outdent"
            )
        {
            self.script_editors.demo_selection(&self.egui_ctx, mode == "multiline-outdent");
        }
        if self.frame_count == 300 && matches!(mode.as_str(), "preprocessor" | "processed" | "object-preprocessor") {
            crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
            if let Some(mut include) = self.world.inventory.items.get(&Uuid::from_u128(8102)).cloned() {
                include.id = Uuid::from_u128(8103);
                include.name = "library.lsl".into();
                let target = ScriptTarget {
                    object: Uuid::nil(),
                    item: include.id,
                };
                self.world.inventory.add_items(vec![include]);
                self.demo_script_sources.insert(
                    target,
                    "#pragma once\ngreet(integer channel) { llSay(channel, \"Bonjour, Aurora !\"); }".into(),
                );
            }
        }
        if self.frame_count == 320
            && matches!(
                mode.as_str(),
                "search"
                    | "search-left"
                    | "search-inline"
                    | "search-overlay"
                    | "search-outside"
                    | "preferences"
                    | "word-wrap-preferences"
                    | "help"
                    | "preprocessor"
                    | "processed"
                    | "object-preprocessor"
                    | "preprocessor-error"
                    | "shortcuts"
            )
            && let Some(target) = self.script_editors.documents.first().map(|e| e.target)
        {
            self.script_editors
                .demo_tools(target, &self.egui_ctx, &mode, &mut self.settings.script_editor);
        }
        if self.frame_count == 400
            && mode == "search-linkset"
            && let Some(target) = self.script_editors.documents.iter().rev().find(|e| e.is_open()).map(|e| e.target)
        {
            // the script in front opens Find: its bar hangs under the toolbar
            self.script_editors.demo_find(target, &self.egui_ctx);
        }
        if self.frame_count == 430 && mode == "processed" {
            self.script_editors.demo_processed(&self.egui_ctx);
        }
        if self.frame_count == 330
            && mode == "linkset"
            && let Some(target) = self
                .script_editors
                .documents
                .iter()
                .find(|e| e.is_open() && !e.linkset.is_nil())
                .map(|e| e.target)
            && let Some(mut source) = self.demo_script_sources.get(&target).cloned()
        {
            // one script holds unsaved text: its row of the tree shows the star
            source.push_str("// an edit that is not saved yet\n");
            self.script_editors.demo_edit(target, source, false);
        }
        if mode == "quit" {
            // 320: a quit while two scripts have unsaved text: held, and each
            // script asks in turn; 400: both discarded, so the viewer must
            // close by itself here (its log would stop at this line)
            if self.frame_count == 320 {
                self.exit_prompt.request();
                self.confirm_exit(crate::ui::exit::Action::Quit);
                log::info!("script demo quit: held for {} script(s) to answer", self.script_editors.prompting());
            }
            if self.frame_count == 360 {
                log::info!("script demo quit: {} script(s) asking", self.script_editors.prompting());
            }
            if self.frame_count == 400 {
                let discarded = self.script_editors.demo_discard_prompts();
                log::info!("script demo quit: {discarded} discard(s), the viewer must close now");
            }
        }
        if self.frame_count == 320
            && matches!(
                mode.as_str(),
                "indent"
                    | "object-indent"
                    | "tabstops"
                    | "object-tabstops"
                    | "experience"
                    | "experience-save"
                    | "close-brace"
                    | "object-close-brace"
            )
            && let Some(target) = self.script_editors.documents.first().map(|e| e.target)
        {
            if matches!(mode.as_str(), "close-brace" | "object-close-brace") {
                self.script_editors
                    .demo_close_brace(target, &self.egui_ctx, mode == "object-close-brace");
            } else if matches!(mode.as_str(), "tabstops" | "object-tabstops") {
                self.script_editors.demo_tabstops(target, &self.egui_ctx);
            } else if matches!(mode.as_str(), "indent" | "object-indent") {
                self.script_editors.demo_indent(target, &self.egui_ctx, mode == "object-indent");
            } else if let Some(action) = self.script_editors.demo_experience(target, Uuid::from_u128(9001)) {
                self.apply_script_action(action);
            }
        }
        if self.frame_count == 300 && mode == "inlays" {
            let target = ScriptTarget {
                object: Uuid::nil(),
                item: Uuid::from_u128(8102),
            };
            if let Some(action) = self.script_editors.demo_edit(target, INLAY_DEMO_SOURCE.into(), true) {
                self.apply_script_action(action);
            }
        }
        if self.frame_count == 320 && mode == "lua" {
            let target = ScriptTarget {
                object: Uuid::nil(),
                item: Uuid::from_u128(8102),
            };
            if let Some(action) = self
                .script_editors
                .demo_edit(target, LUA_DEMO_SOURCE.replace("Avatar", "Aurora"), true)
            {
                self.apply_script_action(action);
            }
        }
        if self.frame_count == 320 && matches!(mode.as_str(), "error" | "save" | "object-save") {
            let target = if mode == "object-save" {
                let Some(target) = self
                    .script_editors
                    .documents
                    .iter()
                    .find(|e| !e.target.object.is_nil())
                    .map(|e| e.target)
                else {
                    return;
                };
                target
            } else {
                ScriptTarget {
                    object: Uuid::nil(),
                    item: Uuid::from_u128(8102),
                }
            };
            let source = if mode == "error" {
                DEFAULT_SOURCE.replace("llSay(0, \"Hello, Avatar!\");", "AURORA_DEMO_SYNTAX_ERROR")
            } else {
                DEFAULT_SOURCE.replace("Hello, Avatar!", "Bonjour, Aurora !")
            };
            if let Some(action) = self.script_editors.demo_edit(target, source, true) {
                self.apply_script_action(action);
            }
        }
        if self.frame_count == 360
            && matches!(
                mode.as_str(),
                "save" | "object-save" | "experience-save" | "preprocessor" | "processed" | "object-preprocessor"
            )
        {
            let Some(target) = self.script_editors.documents.first().map(|e| e.target) else {
                return;
            };
            if self.script_editors.demo_close_clean(target) {
                // A listing may redact the asset UUID after a successful save.
                // Reopening must still load the saved source via the item.
                if target.object.is_nil() {
                    if let Some(item) = self.world.inventory.items.get_mut(&target.item) {
                        item.asset_id = Uuid::nil();
                    }
                } else if let Some(inv) = self.build.contents.objects.get_mut(&target.object)
                    && let Some(item) = inv.items.iter_mut().find(|i| i.item_id == target.item)
                {
                    item.asset_id = Uuid::nil();
                }
                self.open_script(target);
                log::info!("demo scripts: reopened saved source with a hidden asset UUID");
            }
        }
    }

    pub(super) fn demo_editor_sync_steps(&mut self) {
        use aurora_editor_sync::demo_client::Seen;
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_EDITOR_SYNC") else {
            return;
        };
        let target = ScriptTarget {
            object: Uuid::nil(),
            item: Uuid::from_u128(8102),
        };
        if self.frame_count == 280 {
            self.panels.perf = false;
            // for the run: the demo never saves the settings
            self.settings.script_editor.websocket_sync = true;
            if let Ok(port) = mode.parse::<u16>()
                && port >= 1024
            {
                self.settings.script_editor.websocket_port = port;
            }
            crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
            self.open_script(target);
        }
        if self.frame_count == 300 {
            self.script_editors.demo_external(target);
        }
        if self.frame_count > 300
            && self.editor_sync.demo.is_none()
            && let Some(port) = self.editor_sync.port()
        {
            self.editor_sync.demo = Some(aurora_editor_sync::demo_client::spawn(self.net.runtime(), port));
        }
        let seen: Vec<Seen> = self.editor_sync.demo.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for seen in seen {
            log::info!("demo editor sync: {seen:?}");
            let title = match seen {
                Seen::Followed { success: true, .. } => {
                    // a save with an error: its result goes to the editor
                    let source = DEFAULT_SOURCE.replace("llSay(0, \"Hello, Avatar!\");", "AURORA_DEMO_SYNTAX_ERROR");
                    if let Some(action) = self.script_editors.demo_edit(target, source, true) {
                        self.apply_script_action(action);
                    }
                    aurora_i18n::tr!("script-sync-demo-followed").to_owned()
                }
                Seen::Compiled {
                    diagnostics, first_row, ..
                } => aurora_i18n::tr!(
                    "script-sync-demo-compiled",
                    count = diagnostics as i64,
                    row = first_row.unwrap_or(0)
                ),
                _ => continue,
            };
            self.world.notifications.push(
                crate::world::notifications::Kind::Info,
                title,
                "",
                crate::world::notifications::Data::None,
            );
        }
    }

    pub(super) fn demo_script_command(&mut self, cmd: &NetCommand) -> bool {
        match cmd {
            NetCommand::LoadScriptExperiences { request, target, .. } => {
                use aurora_net::scripts::experiences::{ScriptExperience, ScriptExperiences};
                let selected = self.demo_script_experiences.get(target).copied().unwrap_or_default();
                self.on_app_event(NetEvent::ScriptExperiences {
                    request: *request,
                    target: *target,
                    result: ScriptExperiences {
                        associated: Ok(selected),
                        available: Ok(vec![ScriptExperience {
                            id: Uuid::from_u128(9001),
                            name: aurora_i18n::tr!("script-experience-demo").into(),
                            selectable: true,
                        }]),
                    },
                });
            }
            NetCommand::LoadScript { request, target, .. } => {
                let source = self
                    .demo_script_sources
                    .get(target)
                    .cloned()
                    .unwrap_or_else(|| DEFAULT_SOURCE.into());
                self.on_app_event(NetEvent::ScriptLoaded {
                    request: *request,
                    target: *target,
                    result: Ok(source),
                });
            }
            NetCommand::SaveScript {
                request,
                target,
                source,
                options,
                ..
            } => {
                // A named marker simulates a server diagnostic in demo mode;
                // this is deliberately not a local LSL compiler.
                let compiled = !source.contains("AURORA_DEMO_SYNTAX_ERROR");
                log::info!("demo scripts: saved {} for target {}", target.item, options.compile.name());
                let asset = Uuid::new_v4();
                self.demo_script_sources.insert(*target, source.clone());
                self.demo_script_experiences.insert(*target, options.experience);
                self.build_demo
                    .save_script(*target, asset, compiled && options.running, options.compile);
                let result = Ok(ScriptSaved {
                    asset,
                    compiled,
                    checked: true,
                    errors: if compiled {
                        vec![]
                    } else {
                        vec!["(5, 9) : ERROR : Syntax error".into()]
                    },
                });
                self.on_app_event(NetEvent::ScriptSaved {
                    request: *request,
                    target: *target,
                    result,
                });
            }
            _ => return false,
        }
        true
    }
}
