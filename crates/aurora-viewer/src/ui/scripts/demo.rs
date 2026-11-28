//! The script editor's offline demo: the state `AURORA_DEMO_SCRIPTS` puts the
//! window in and the keys its scenarios send, kept apart from what the window
//! does as the reader uses it (`ui/scripts.rs`).

use super::Lang;

use super::*;

impl Editors {
    pub fn demo_split(&mut self) {
        // the linkset window's own scripts, not a script of another window
        let Some(root) = self
            .documents
            .iter()
            .find(|e| e.is_open() && !e.linkset.is_nil())
            .map(Self::window_key)
        else {
            return;
        };
        let open: Vec<ScriptTarget> = self
            .documents
            .iter()
            .filter(|e| e.is_open() && Self::window_key(e) == root)
            .map(|e| e.target)
            .collect();
        if open.len() >= 2 {
            self.splits.insert(root, (open[0], open[1]));
            self.split_ratio.insert(root, 0.42);
        }
    }

    pub fn demo_new_script(&mut self, root: Uuid, prim: &str, lang: Lang) {
        self.open_draft(root, root, prim.to_owned(), lang, Vec::new());
    }

    pub fn demo_discard_prompts(&mut self) -> usize {
        let mut discarded = 0;
        for e in &mut self.documents {
            if e.close_prompt && !e.closed {
                e.close_prompt = false;
                e.close_after_save = false;
                e.closed = true;
                discarded += 1;
            }
        }
        discarded
    }

    pub fn demo_edit(&mut self, target: ScriptTarget, source: String, save: bool) -> Option<Action> {
        let e = self.documents.iter_mut().find(|e| e.target == target)?;
        e.source = source;
        if save { e.save() } else { None }
    }

    pub fn demo_external(&mut self, target: ScriptTarget) {
        if let Some(e) = self.documents.iter_mut().find(|e| e.target == target && e.can_edit) {
            e.tools.external_starting = true;
        }
    }

    pub fn demo_experience(&mut self, target: ScriptTarget, experience: Uuid) -> Option<Action> {
        let e = self.documents.iter_mut().find(|e| e.target == target)?;
        e.experience.selected = Some(experience);
        e.save()
    }

    pub fn demo_indent(&mut self, target: ScriptTarget, ctx: &egui::Context, tabs: bool) {
        let Some(e) = self.documents.iter_mut().find(|e| e.target == target) else {
            return;
        };
        if tabs {
            e.source = e.source.replace("    ", "\t");
        }
        let Some(line) = e
            .source
            .find("llSay(")
            .and_then(|start| e.source[start..].find(';').map(|end| start + end + 1))
        else {
            return;
        };
        Self::focus_source(e, ctx, line);
    }

    pub fn demo_tabstops(&mut self, target: ScriptTarget, ctx: &egui::Context) {
        let Some(e) = self.documents.iter_mut().find(|e| e.target == target) else {
            return;
        };
        e.source = e.source.replacen("        llSay(", "       llSay(", 1);
        if let Some(position) = e.source.find("llSay(") {
            Self::focus_source(e, ctx, position);
        }
    }

    pub fn demo_close_brace(&mut self, target: ScriptTarget, ctx: &egui::Context, tabs: bool) {
        if let Some(e) = self.documents.iter_mut().find(|e| e.target == target) {
            e.source = "default\n{\n    state_entry()\n    {\n        llSetColor(<0.2, 0.8, 1.0>, ALL_SIDES);\n        ".into();
            if tabs {
                e.source = e.source.replace("    ", "\t");
            }
            Self::focus_source(e, ctx, e.source.len());
        }
    }

    pub fn demo_color_events(&self, frame: u64, mode: &str) -> Vec<egui::Event> {
        use super::super::color_picker::Control;
        let Some(e) = self.documents.first() else { return Vec::new() };
        let rect = match frame {
            323 | 324 => e.colors.swatches.first().copied(),
            343 | 344 => e.colors.picker.controls.get(&Control::Slot(20)).copied(),
            363 | 364 if mode == "color-cancel" => e.colors.picker.controls.get(&Control::Cancel).copied(),
            363 | 364 if mode == "color-accept" => e.colors.picker.controls.get(&Control::Accept).copied(),
            _ => None,
        };
        let Some(rect) = rect else { return Vec::new() };
        let pos = rect.center();
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: frame % 2 == 1,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    pub fn demo_selection(&mut self, ctx: &egui::Context, outdent: bool) {
        if let Some(e) = self.documents.first_mut() {
            let Some(start) = e.source.find("integer count") else { return };
            let Some(end) = e.source.find("string greet") else { return };
            let id = format!("script_editor_{}_{}", e.target.object, e.target.item);
            let id = egui::Id::unique((&id, "source_text"));
            editing::select(ctx, id, e.source[..start].chars().count()..e.source[..end].chars().count(), true);
            if outdent {
                editing::selection_command(ctx, id, &mut e.source, editing::SelectionCommand::Indent);
            }
        }
    }

    pub fn demo_insertion_events(&self, ctx: &egui::Context, frame: u64, mode: &str) -> Vec<egui::Event> {
        let Some(e) = self.documents.first() else { return Vec::new() };
        if matches!(frame, 323 | 324) {
            let Some(rect) = e.tools.insert_button else { return Vec::new() };
            let pos = rect.center();
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 323,
                    modifiers: egui::Modifiers::NONE,
                },
            ];
        }
        if (frame == 400 && mode == "insert-filter") || (frame == 326 && mode == "insert-empty") {
            return vec![egui::Event::Text(
                if mode == "insert-empty" { "no_matching_function" } else { "color" }.into(),
            )];
        }
        if frame == 350 && mode == "insert-filter" {
            let id = format!("script_editor_{}_{}", e.target.object, e.target.item);
            let id = egui::Id::unique((&id, "source_text")).with("insert_filter");
            if let Some(field) = ctx.read_response(id) {
                return vec![
                    egui::Event::PointerMoved(field.rect.center_bottom() + Vec2::new(0.0, 90.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0.0, -240.0),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Unknown,
                    },
                ];
            }
        }
        Vec::new()
    }

    pub fn demo_code_events(&self, frame: u64) -> Vec<egui::Event> {
        if !matches!(frame, 640 | 641) {
            return Vec::new();
        }
        let Some(e) = self.documents.first() else { return Vec::new() };
        let Some(rect) = e.tools.code_button else { return Vec::new() };
        let pos = rect.center();
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: frame == 640,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    pub fn demo_folding_events(&self, frame: u64) -> Vec<egui::Event> {
        if !matches!(frame, 323 | 324) {
            return Vec::new();
        }
        let Some(e) = self.documents.first() else { return Vec::new() };
        let Some((_, rect)) = e.folding.controls.get(1) else {
            return Vec::new();
        };
        let pos = rect.center();
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: frame == 323,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    pub fn demo_tools(&mut self, target: ScriptTarget, ctx: &egui::Context, mode: &str, settings: &mut settings::ScriptSettings) {
        let Some(e) = self.documents.iter_mut().find(|e| e.target == target) else {
            return;
        };
        let id = format!("script_editor_{}_{}", target.object, target.item);
        let text_id = egui::Id::unique((&id, "source_text"));
        match mode {
            "search" | "search-left" | "search-inline" | "search-overlay" | "search-outside" => {
                settings.search_prefer_left = mode == "search-left";
                settings.search_inline = mode == "search-inline";
                let geometry = match mode {
                    "search-left" => Some((egui::pos2(450.0, 130.0), Vec2::new(660.0, 650.0))),
                    "search-inline" => Some((egui::pos2(120.0, 130.0), Vec2::new(960.0, 700.0))),
                    "search-overlay" => Some((egui::pos2(140.0, 130.0), Vec2::new(1300.0, 700.0))),
                    "search-outside" => Some((egui::pos2(-380.0, 130.0), Vec2::new(660.0, 650.0))),
                    _ => None,
                };
                if let Some((pos, size)) = geometry {
                    super::super::widgets::place_floater(ctx, &id, pos, size);
                }
                self.panels.search.begin(ctx, text_id, text_id, &e.source);
                self.panels.search.query = "llSay".into();
                self.panels.search.replacement = "llOwnerSay".into();
                self.panels.search.find(ctx, text_id, &e.source);
            }
            "preferences" | "word-wrap-preferences" => {
                self.panels.preferences.open = true;
                // the editor and the preferences side by side: the preferences
                // are two panes wide, and the editor would cover their middle
                super::super::widgets::place_floater(ctx, dialogs::Preferences::WINDOW, egui::pos2(8.0, 70.0), Vec2::new(748.0, 700.0));
                super::super::widgets::place_floater(ctx, &id, egui::pos2(768.0, 70.0), Vec2::new(584.0, 700.0));
            }
            "shortcuts" => self.panels.shortcuts = true,
            "help" => {}
            "preprocessor" | "processed" | "object-preprocessor" | "preprocessor-error" => {
                settings.preprocessor = true;
                e.processor_settings.clone_from(settings);
                e.source = if mode == "preprocessor-error" {
                    "#if 1\n#error Demo preprocessing error\n#endif\n".into()
                } else {
                    "#define CHANNEL 0\n#include \"library.lsl\"\nlist values = [1, 2];\nstring label = \"\";\ninteger counter = 0;\nunused_helper(string text)\n{\n    return \"unused \" + text;\n}\ndefault\n{\n    state_entry()\n    {\n        values[1] = 42;\n        counter = counter + 1;\n        llSetText((string)label + (string)counter, <1, 1, 1>, 1.0);\n        switch ((integer)values[1])\n        {\n            case 42: greet(CHANNEL); break;\n            default: llOwnerSay(\"Unexpected value\");\n        }\n    }\n}\n".into()
                };
                e.save();
            }
            _ => {}
        }
    }

    pub fn demo_find(&mut self, target: ScriptTarget, ctx: &egui::Context) {
        let Some(e) = self.documents.iter_mut().find(|e| e.target == target) else {
            return;
        };
        let text_id = egui::Id::unique((&Self::document_id(e), "source_text"));
        self.panels.search.begin(ctx, text_id, text_id, &e.source);
        self.panels.search.query = "ll".into();
        self.panels.search.find(ctx, text_id, &e.source);
    }

    pub fn demo_processed(&mut self, ctx: &egui::Context) {
        if let Some(e) = self.documents.first_mut() {
            e.tools.processed = true;
            let id = format!("script_editor_{}_{}", e.target.object, e.target.item);
            let id = egui::Id::unique((&id, "source_text")).with("processed");
            let at = e
                .tools
                .processed_source
                .find("\n//nfo_preprocessor_version 0\n")
                .map(|i| i + "\n//nfo_preprocessor_version 0\n".len())
                .unwrap_or(0);
            let at = e.tools.processed_source[..at].chars().count();
            editing::select(ctx, id, at..at, false);
            e.tools.scroll_to_cursor = true;
        }
    }

    pub fn demo_close_clean(&mut self, target: ScriptTarget) -> bool {
        let Some(index) = self
            .documents
            .iter()
            .position(|e| e.target == target && e.loaded && !e.dirty() && e.saving.is_none())
        else {
            return false;
        };
        self.documents.remove(index);
        true
    }
}
