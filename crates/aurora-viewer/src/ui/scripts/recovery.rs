//! The offer of the scripts left with unsaved changes by an earlier session
//! (`backup.rs`). Port of LLFloaterScriptRecover and
//! `LLScriptRecoverQueue::recoverIfNeeded` (indra/newview/
//! llfloaterscriptrecover.cpp, floater_script_recover.xml, originally LGPL
//! 2.1): at every login the account's copies are listed, each one ticked;
//! Recover puts the ticked ones in Lost and Found as new scripts and
//! forgets the others, Cancel forgets them all. The Legacy Viewer's window
//! has no close button; here its X keeps every copy for the next login, so
//! that closing it by mistake loses nothing.

use crate::theme::Palette;
use crate::ui::widgets;
use std::path::{Path, PathBuf};

pub const WINDOW: &str = "script_recovery";

/// A copy found at login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub source: String,
    pub saved: i64,
    pub chosen: bool,
}

#[derive(Debug, Default)]
pub struct Offer {
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Recover,
    Cancel,
    /// The window's X: the copies stay for the next login.
    Later,
}

/// The copies of `dir`, read and opened on a thread of their own; the
/// newest first. A file that does not open (damaged, sealed under another
/// key) is left where it is and not offered.
pub fn scan(dir: PathBuf) -> crossbeam_channel::Receiver<Vec<Row>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("aurora-script-recovery".into())
        .spawn(move || {
            let _ = tx.send(read(&dir));
        });
    if let Err(e) = spawned {
        log::warn!("script recovery: not checked ({e})");
    }
    rx
}

pub fn read(dir: &Path) -> Vec<Row> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rows: Vec<Row> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|path| super::backup::is_copy(path))
        .filter_map(|path| {
            let copy = std::fs::read(&path).ok().and_then(super::backup::decode);
            if copy.is_none() {
                log::warn!("script recovery: a copy could not be read; left in place");
            }
            let copy = copy?;
            Some(Row {
                path,
                name: copy.name,
                source: copy.source,
                saved: copy.saved,
                chosen: true,
            })
        })
        .collect();
    rows.sort_by(|a, b| b.saved.cmp(&a.saved).then_with(|| a.name.cmp(&b.name)));
    log::info!("script recovery: {} recoverable script(s) found", rows.len());
    rows
}

/// The name of the recovered script: the one it had, or "(Unknown script)"
/// (LLScriptRecoverQueue::recoverIfNeeded's `unknown_script`).
pub fn display_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        aurora_i18n::tr!("script-recovery-unknown").to_owned()
    } else {
        name.to_owned()
    }
}

/// The window; `None` while unanswered.
pub fn window(ctx: &egui::Context, p: &Palette, offer: &mut Offer) -> Option<Answer> {
    let mut open = true;
    let mut answer = None;
    let size = egui::vec2(420.0, 340.0);
    widgets::Floater::new(
        WINDOW,
        aurora_i18n::tr!("script-recovery-title"),
        ctx.content_rect().center() - size / 2.0,
        size,
    )
    .steady_size()
    .show(ctx, p, &mut open, |ui| {
        let (mut above, footer) = widgets::above_footer(ui);
        body(&mut above, p, offer);
        footer.show(ui, |ui| {
            let chosen = offer.rows.iter().any(|r| r.chosen);
            let buttons = [
                widgets::DialogButton {
                    icon: "x",
                    label: aurora_i18n::tr!("common-cancel"),
                    kind: widgets::ActionKind::Flat,
                    enabled: true,
                },
                widgets::DialogButton {
                    icon: "restore",
                    label: aurora_i18n::tr!("script-recovery-recover"),
                    kind: widgets::ActionKind::Primary,
                    enabled: chosen,
                },
            ];
            answer = match widgets::dialog_buttons(ui, p, &buttons, 32.0) {
                Some(0) => Some(Answer::Cancel),
                Some(_) => Some(Answer::Recover),
                None => None,
            };
        });
    });
    if !open && answer.is_none() {
        answer = Some(Answer::Later);
    }
    answer
}

fn body(ui: &mut egui::Ui, p: &Palette, offer: &mut Offer) {
    ui.spacing_mut().item_spacing.y = 10.0;
    ui.add(egui::Label::new(egui::RichText::new(aurora_i18n::tr!("script-recovery-intro")).color(p.ink)).wrap());
    egui::ScrollArea::vertical()
        .id_salt("script_recovery_rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for row in &mut offer.rows {
                widgets::checkbox(ui, p, &mut row.chosen, display_name(&row.name));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::scripts::backup;
    use aurora_net::scripts::ScriptTarget;
    use uuid::Uuid;

    fn rows(n: usize) -> Vec<Row> {
        (0..n)
            .map(|i| Row {
                path: PathBuf::from(format!("{i}.bak")),
                name: format!("A script with a rather long name, number {i}"),
                source: String::new(),
                saved: i as i64,
                chosen: i % 2 == 0,
            })
            .collect()
    }

    #[test]
    fn the_copies_of_the_folder_are_listed_newest_first_and_damaged_ones_left_alone() {
        let dir = crate::settings::account_dir(&Uuid::new_v4()).join(backup::FOLDER);
        std::fs::create_dir_all(&dir).expect("folder");
        for (item, name, saved) in [(1, "Old", 10), (2, "New", 20), (3, "", 15)] {
            let target = ScriptTarget {
                object: Uuid::nil(),
                item: Uuid::from_u128(item),
            };
            let copy = backup::Copy {
                name: name.into(),
                source: format!("// {name}"),
                saved,
            };
            backup::apply(backup::Job::Write(backup::file(&dir, target), copy));
        }
        std::fs::write(dir.join("damaged.bak"), b"not a copy").expect("damaged");
        std::fs::write(dir.join("notes.txt"), b"other").expect("other");
        let found = read(&dir);
        let names: Vec<_> = found.iter().map(|r| display_name(&r.name)).collect();
        assert_eq!(names, ["New", aurora_i18n::tr!("script-recovery-unknown"), "Old"]);
        assert!(found.iter().all(|r| r.chosen), "every copy is ticked, as in the Legacy Viewer");
        assert_eq!(found[0].source, "// New");
        assert!(dir.join("damaged.bak").exists(), "a copy that does not open stays");
        let _ = std::fs::remove_dir_all(dir.parent().expect("account folder"));
        assert!(read(&dir).is_empty(), "no folder: nothing to offer");
    }

    #[test]
    fn the_window_keeps_its_size() {
        for count in [1, 12] {
            widgets::steady_tests::assert_window_steady(
                WINDOW,
                &[egui::vec2(420.0, 340.0), egui::vec2(360.0, 260.0)],
                aurora_i18n::tr!("script-recovery-recover"),
                || Offer { rows: rows(count) },
                |offer, ctx, p| {
                    window(ctx, p, offer);
                },
            );
        }
    }
}
