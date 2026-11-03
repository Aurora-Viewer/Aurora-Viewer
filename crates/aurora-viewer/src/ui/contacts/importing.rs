//! Shared import action for People and Conversations. Native file selection
//! and XML parsing run on a worker; only its originating account may apply it.
use super::*;
use crate::world::contact_sets::importing::ImportData;
use std::io::Read;
use std::sync::mpsc::{Receiver, TryRecvError};

pub(super) struct Job {
    account: Uuid,
    receiver: Receiver<Result<Option<ImportData>, String>>,
}

impl ContactsUi {
    pub(crate) fn can_import_contact_sets(&self) -> bool {
        self.import_job.is_none()
    }

    pub(crate) fn import_contact_sets(&mut self, ctx: &egui::Context, account: Uuid) {
        if self.import_job.is_some() {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let title = aurora_i18n::tr!("contacts-import").to_owned();
        let filter = aurora_i18n::tr!("contacts-import-file-type").to_owned();
        let _ = std::thread::Builder::new().name("aurora-contact-import".into()).spawn(move || {
            let result = (|| {
                let Some(path) = rfd::FileDialog::new()
                    .set_title(title)
                    .set_file_name(contact_sets::CONTACT_SETS_FILE)
                    .add_filter(filter, &["xml"])
                    .pick_file()
                else {
                    return Ok(None);
                };
                read_import(&path).map(Some)
            })();
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.import_job = Some(Job { account, receiver });
    }
}

fn read_import(path: &std::path::Path) -> Result<ImportData, String> {
    // Large enough for extensive resident lists, bounded before XML parsing.
    const MAX_BYTES: u64 = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("cannot read contact sets: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("contact sets file exceeds 16 MiB".into());
    }
    ImportData::from_xml(&bytes).map_err(str::to_owned)
}

pub(super) fn poll(world: &mut World, st: &mut ContactsUi) {
    let Some(job) = &st.import_job else {
        return;
    };
    if job.account != world.agent_id {
        st.import_job = None;
        return;
    }
    let result = match job.receiver.try_recv() {
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("contact import worker stopped".into()),
        Ok(result) => result,
    };
    st.import_job = None;
    let result = match result {
        Ok(Some(data)) => world
            .contact_sets
            .merge_import(data, |id| world.social.is_friend(id))
            .map_err(|e| e.to_string()),
        Ok(None) => return,
        Err(e) => Err(e),
    };
    let (kind, body) = match result {
        Ok(count) => {
            st.set_choice = Some(contact_sets::ALL_SETS.to_owned());
            st.set_filter.clear();
            st.set_sel.clear();
            (
                crate::world::notifications::Kind::Info,
                aurora_i18n::tr!("contacts-import-success", count = count),
            )
        }
        Err(e) => {
            log::warn!("contact sets import failed: {e}");
            (
                crate::world::notifications::Kind::Alert,
                aurora_i18n::tr!("contacts-import-failed").to_owned(),
            )
        }
    };
    world.notifications.push(
        kind,
        aurora_i18n::tr!("ui-contacts-contact-sets"),
        body,
        crate::world::notifications::Data::None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()))
    }

    fn parsed() -> ImportData {
        ImportData::from_xml(
            br#"<llsd><map><key>groupname</key><string>Imported</string><key>notices</key><boolean>1</boolean></map></llsd>"#,
        )
        .expect("individual export")
    }

    #[test]
    fn contact_set_import_completion_updates_shared_state_once() {
        let mut world = world();
        let mut st = ContactsUi {
            set_filter: "old filter".into(),
            ..Default::default()
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        st.import_job = Some(Job {
            account: world.agent_id,
            receiver,
        });
        assert!(sender.send(Ok(Some(parsed()))).is_ok());
        poll(&mut world, &mut st);
        assert!(world.contact_sets.is_valid("Imported"));
        assert_eq!(st.set_choice.as_deref(), Some(contact_sets::ALL_SETS));
        assert!(st.set_filter.is_empty());
        assert!(st.can_import_contact_sets());
        let generation = world.contact_sets.generation;
        poll(&mut world, &mut st);
        assert_eq!(world.contact_sets.generation, generation);
    }

    #[test]
    fn contact_set_import_cancel_failure_and_account_change_preserve_contacts() {
        let mut world = world();
        world.contact_sets.add_set("Existing");
        for result in [Ok(None), Err("invalid file".into()), Ok(Some(parsed()))] {
            let account = if result.as_ref().is_ok_and(|v| v.is_some()) {
                Uuid::from_u128(42)
            } else {
                world.agent_id
            };
            let mut st = ContactsUi::default();
            let (sender, receiver) = std::sync::mpsc::channel();
            st.import_job = Some(Job { account, receiver });
            assert!(sender.send(result).is_ok());
            let generation = world.contact_sets.generation;
            poll(&mut world, &mut st);
            assert_eq!(world.contact_sets.set_names(), ["Existing"]);
            assert_eq!(world.contact_sets.generation, generation);
            assert!(st.can_import_contact_sets());
        }
    }
}
