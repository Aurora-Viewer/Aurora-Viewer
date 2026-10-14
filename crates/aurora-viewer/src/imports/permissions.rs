//! Per-type next-owner defaults. LLFloaterPermsDefault::onCommitCopy
//! (indra/newview/llfloaterperms.cpp) requires transfer whenever copy is off.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextOwner {
    pub modify: bool,
    pub copy: bool,
    pub transfer: bool,
    /// Missing values inherit the legacy common sharing preferences.
    #[serde(default)]
    pub share_group: Option<bool>,
    #[serde(default)]
    pub everyone_copy: Option<bool>,
}
impl NextOwner {
    pub fn mask(self) -> u32 {
        use aurora_net::build::perm;
        perm::MOVE
            | if self.modify { perm::MODIFY } else { 0 }
            | if self.copy { perm::COPY } else { 0 }
            | if self.transfer || !self.copy { perm::TRANSFER } else { 0 }
    }
}

// Missing rows inherit the old common preferences, including saved custom
// values. Editing one row cannot reset another type during migration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ByType {
    pub texture: Option<NextOwner>,
    pub sound: Option<NextOwner>,
    pub animation: Option<NextOwner>,
    pub mesh: Option<NextOwner>,
    pub material: Option<NextOwner>,
}
impl ByType {
    pub fn get(&self, kind: Kind) -> Option<NextOwner> {
        match kind {
            Kind::Texture => self.texture,
            Kind::Sound => self.sound,
            Kind::Animation => self.animation,
            Kind::Mesh => self.mesh,
            Kind::Material => self.material,
            // a photo gives every right to the next owner (saveTexture)
            Kind::Snapshot => None,
        }
    }
    pub fn set(&mut self, kind: Kind, row: NextOwner) {
        let slot = match kind {
            Kind::Texture => &mut self.texture,
            Kind::Sound => &mut self.sound,
            Kind::Animation => &mut self.animation,
            Kind::Mesh => &mut self.mesh,
            Kind::Material => &mut self.material,
            Kind::Snapshot => return,
        };
        *slot = Some(row);
    }
}
impl Preferences {
    pub fn next_owner_for(&self, kind: Kind) -> NextOwner {
        self.permissions_by_type.get(kind).unwrap_or(NextOwner {
            modify: self.next_modify,
            copy: self.next_copy,
            transfer: self.next_transfer,
            share_group: None,
            everyone_copy: None,
        })
    }
    pub fn permissions_for(&self, kind: Kind) -> Permissions {
        use aurora_net::build::perm;
        let row = self.next_owner_for(kind);
        Permissions {
            next_owner: row.mask(),
            group: if row.share_group.unwrap_or(self.share_group) {
                perm::COPY | perm::MOVE | perm::MODIFY
            } else {
                0
            },
            everyone: if row.everyone_copy.unwrap_or(self.everyone_copy) {
                perm::COPY
            } else {
                0
            },
        }
    }
}
