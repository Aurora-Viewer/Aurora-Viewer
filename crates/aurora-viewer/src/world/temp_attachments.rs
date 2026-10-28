//! Our temporary attachments: what an experience or llAttachToAvatarTemp put
//! on our avatar, with no inventory item of ours. Port of
//! LLAgentWearables::getTempAttachments (indra/newview/llagentwearables.cpp)
//! and of the names of LLPanelWearing::requestAttachmentDetails /
//! setAttachmentDetails (indra/newview/llpanelwearing.cpp), originally
//! LGPL 2.1.
use super::{World, objects::ObjKey};
use aurora_net::land::ScriptAttachment;
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use uuid::Uuid;

/// The least time between two AttachmentResources requests for names: a
/// burst of temporary attachments asks once.
const REQUEST_GAP: Duration = Duration::from_secs(5);
/// At most this many names are remembered (objects detached long ago go).
const NAMES_LIMIT: usize = 512;

/// One temporary attachment, by its root prim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempAttachment {
    pub key: ObjKey,
    pub id: Uuid,
    pub point: u8,
    /// The root prim handles touches (enable_attachment_touch).
    pub touch: bool,
}

/// The names AttachmentResources gave, and what was asked already.
#[derive(Default)]
pub struct TempAttachmentNames {
    names: HashMap<Uuid, String>,
    asked: HashSet<Uuid>,
    last: Option<Instant>,
}

impl TempAttachmentNames {
    pub fn get(&self, id: &Uuid) -> Option<&str> {
        self.names.get(id).map(String::as_str)
    }

    /// setAttachmentDetails: every object of the answer, by its task id.
    /// Any answer is used, whoever asked for it (the Scripts window too).
    pub fn learn(&mut self, attachments: &[ScriptAttachment]) {
        if self.names.len() > NAMES_LIMIT {
            self.names.clear();
        }
        for o in attachments.iter().flat_map(|a| &a.objects) {
            if !o.id.is_nil() && !o.name.is_empty() {
                self.names.insert(o.id, o.name.clone());
            }
        }
    }

    /// Whether to ask AttachmentResources now: a temporary attachment has
    /// no name and was not asked for yet. Each object is asked for once, so
    /// an object the answer leaves out does not ask again at every frame.
    fn ask(&mut self, temporary: &[TempAttachment], now: Instant) -> bool {
        let unnamed: Vec<Uuid> = temporary
            .iter()
            .map(|t| t.id)
            .filter(|id| !self.names.contains_key(id) && !self.asked.contains(id))
            .collect();
        if unnamed.is_empty() || self.last.is_some_and(|at| now.saturating_duration_since(at) < REQUEST_GAP) {
            return false;
        }
        if self.asked.len() > NAMES_LIMIT {
            self.asked.clear();
        }
        self.asked.extend(unnamed);
        self.last = Some(now);
        true
    }
}

/// LLViewerObject::isTempAttachment: the simulator gives a temporary
/// attachment its own id as AttachItemID.
pub fn is_temp_attachment(id: Uuid, attach_item: Option<Uuid>) -> bool {
    !id.is_nil() && attach_item == Some(id)
}

impl World {
    /// LLAgentWearables::getTempAttachments, by attachment point.
    pub fn temp_attachments(&self) -> Vec<TempAttachment> {
        let Some(me) = self.objects.index_of_uuid(&self.agent_id).and_then(|i| self.objects.get(i)) else {
            return Vec::new();
        };
        let mut list: Vec<_> = self
            .objects
            .children_of(&me.key)
            .iter()
            .filter_map(|i| self.objects.get(*i))
            // an item of ours by that id is worn from inventory, as
            // rlva_temporary_attachment and the HUD scales decide
            .filter(|o| {
                o.attachment_point() != 0
                    && is_temp_attachment(o.full_id, o.attachment_item_id())
                    && !self.inventory.items.contains_key(&o.full_id)
            })
            .map(|o| TempAttachment {
                key: o.key,
                id: o.full_id,
                point: o.attachment_point(),
                touch: o.update_flags & crate::ui::context::flags::HANDLE_TOUCH != 0,
            })
            .collect();
        list.sort_by_key(|t| (t.point, t.id));
        list
    }

    /// LLPanelWearing::updateAttachmentsList: ask AttachmentResources for
    /// the names of temporary attachments that have none yet.
    pub fn request_temp_attachment_names(&mut self, temporary: &[TempAttachment], now: Instant) {
        if self.temp_attachment_names.ask(temporary, now) {
            self.land.request_attachment_names();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::land::ScriptObject;

    fn temp(n: u128) -> TempAttachment {
        TempAttachment {
            key: ObjKey {
                region: 1,
                local_id: n as u32,
            },
            id: Uuid::from_u128(n),
            point: 2,
            touch: false,
        }
    }

    fn answer(n: u128, name: &str) -> Vec<ScriptAttachment> {
        vec![ScriptAttachment {
            location: "Skull".into(),
            objects: vec![ScriptObject {
                id: Uuid::from_u128(n),
                name: name.into(),
                owner: Uuid::nil(),
                group_owned: false,
                memory: 0,
                urls: 0,
                location: None,
            }],
        }]
    }

    #[test]
    fn a_temporary_attachment_is_attached_by_its_own_id() {
        let id = Uuid::from_u128(7);
        assert!(is_temp_attachment(id, Some(id)));
        // an inventory item, none, or nil: an ordinary attachment
        assert!(!is_temp_attachment(id, Some(Uuid::from_u128(8))));
        assert!(!is_temp_attachment(id, None));
        assert!(!is_temp_attachment(Uuid::nil(), Some(Uuid::nil())));
    }

    #[test]
    fn names_are_asked_once_per_object_and_not_in_a_burst() {
        let mut names = TempAttachmentNames::default();
        let now = Instant::now();
        assert!(!names.ask(&[], now));
        assert!(names.ask(&[temp(1)], now));
        // the same object, unanswered: not asked again
        assert!(!names.ask(&[temp(1)], now + Duration::from_secs(60)));
        // a new one right after: waits for the gap
        assert!(!names.ask(&[temp(1), temp(2)], now + Duration::from_secs(1)));
        assert!(names.ask(&[temp(1), temp(2)], now + REQUEST_GAP));
        // a named one is never asked for
        names.learn(&answer(3, "Experience HUD"));
        assert_eq!(names.get(&Uuid::from_u128(3)), Some("Experience HUD"));
        assert!(!names.ask(&[temp(3)], now + REQUEST_GAP * 3));
    }

    #[test]
    fn empty_names_and_nil_ids_are_not_learnt() {
        let mut names = TempAttachmentNames::default();
        names.learn(&answer(4, ""));
        names.learn(&answer(0, "Nothing"));
        assert_eq!(names.get(&Uuid::from_u128(4)), None);
        assert_eq!(names.get(&Uuid::nil()), None);
    }
}
