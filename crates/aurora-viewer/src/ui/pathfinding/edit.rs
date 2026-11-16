//! "Edit pathfinding attributes" of the Region objects window: the use
//! dropdown, the A–D walkability fields and Apply changes. Port of
//! LLFloaterPathfindingLinksets::updateEditFieldValues,
//! onWalkabilityCoefficientEntered, applyEdit and doApplyEdit
//! (indra/newview/llfloaterpathfindinglinksets.cpp, originally LGPL 2.1);
//! the rules themselves are `aurora_net::pathfinding::linkset`.
use aurora_net::pathfinding::Row;
use aurora_net::pathfinding::linkset::{self, Edit, Linkset, Use, Warnings};
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditPanel {
    /// The dropdown; None when nothing is selected ("Choose linkset use…").
    pub use_: Option<Use>,
    /// The A–D fields as typed.
    pub fields: [String; 4],
    /// Their last committed values (mPreviousValueA…).
    previous: [u8; 4],
}

/// What Apply changes does once confirmed: the objects' edits and the
/// terrain's, for `NetCommand::SetLinksets`.
#[derive(Debug, Clone, PartialEq)]
pub struct Apply {
    pub objects: Vec<(Uuid, Edit)>,
    pub terrain: Option<Edit>,
}

impl EditPanel {
    /// updateEditFieldValues: the first selected linkset's use and
    /// coefficients, or empty fields without a selection.
    pub fn load(&mut self, first: Option<&Linkset>) {
        match first {
            Some(linkset) => {
                self.use_ = Some(linkset.use_);
                self.previous = linkset.walkability;
                self.fields = linkset.walkability.map(|v| v.to_string());
            }
            None => *self = Self::default(),
        }
    }

    /// onWalkabilityCoefficientEntered: the field once it loses focus or
    /// takes Enter.
    pub fn commit(&mut self, field: usize) {
        let Some(text) = self.fields.get_mut(field) else { return };
        let value = linkset::coefficient_entry(text, self.previous[field]);
        self.previous[field] = value;
        *text = value.to_string();
    }

    fn values(&self) -> [i32; 4] {
        self.previous.map(i32::from)
    }

    /// The question applyEdit asks before sending, if any.
    pub fn warnings<'a>(&self, selection: impl IntoIterator<Item = &'a Row>) -> Warnings {
        Warnings::of(selection.into_iter().filter_map(Row::linkset), self.use_)
    }

    /// doApplyEdit: every selected linkset's changes (encodeObjectFields,
    /// encodeTerrainFields; the terrain only for who may read it, which the
    /// list already ensures).
    pub fn apply<'a>(&mut self, selection: impl IntoIterator<Item = &'a Row>) -> Apply {
        for field in 0..4 {
            self.commit(field);
        }
        let values = self.values();
        let mut apply = Apply {
            objects: Vec::new(),
            terrain: None,
        };
        for row in selection {
            let Some(linkset) = row.linkset() else { continue };
            let edit = linkset.edit(self.use_, values);
            if linkset.terrain {
                apply.terrain = Some(edit);
            } else {
                apply.objects.push((row.id, edit));
            }
        }
        apply
    }
}

/// The question's text (the PathfindingLinksets_* notifications), its
/// lines in the Legacy order.
pub fn question(warnings: Warnings, requested: Use) -> String {
    let requested_label = super::list::use_label(requested);
    let restricted_label = super::list::use_label(requested.with_toggled_phantom());
    let mut lines = Vec::new();
    if warnings.phantom_toggle {
        lines.push(aurora_i18n::text("pathfinding-warn-phantom").to_owned());
    }
    if warnings.restricted {
        lines.push(aurora_i18n::tr!(
            "pathfinding-warn-restricted",
            requested = requested_label,
            restricted = restricted_label
        ));
    }
    if warnings.cannot_be_volume {
        lines.push(if warnings.restricted {
            aurora_i18n::tr!("pathfinding-warn-concave-unchanged", requested = requested_label)
        } else {
            aurora_i18n::tr!("pathfinding-warn-concave", requested = requested_label)
        });
    }
    lines.push(aurora_i18n::text("pathfinding-warn-continue").to_owned());
    lines.join("\n\n")
}
