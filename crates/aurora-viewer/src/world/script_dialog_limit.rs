//! How many script dialogs an object keeps open: the Legacy Viewer's
//! `ScriptDialogLimitations` (app_settings/settings.xml, default 1 "one per
//! channel"; Preferences › User Interface › 2D Overlay, "Script Dialogs per
//! Object"), applied by LLScriptFloaterManager::onAddNotification
//! (indra/newview/llscriptfloater.cpp).
//!
//! A new llDialog or llTextBox (both are `ScriptDialog` notifications)
//! replaces the older one it matches: that one is closed without any
//! `ScriptDialogReply`, as LLNotificationsUtil::cancel sends nothing. An
//! llLoadURL, a permission question or an object's gift is never limited.
//!
//! Deliberate differences, both for the user:
//! - The Legacy Viewer's per-object search (findUsingObjectId) also finds a
//!   URL request or a gift of the same object, which the new dialog would
//!   then close; here only an older dialog or text box can be replaced, so
//!   an offered item is never lost to a menu.
//! - The Legacy Viewer tells a worn object by the prim that sent the dialog
//!   (getAttachmentItemID, isHUDAttachment); here by its linkset
//!   (`build::select_type`), so a HUD button in a child prim counts as the
//!   HUD it belongs to.

use uuid::Uuid;

/// The six choices of the Legacy Viewer's list, in its order (its numbers
/// 0 to 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Limit {
    /// One per object (0).
    PerObject,
    /// One per object and channel (1, the Legacy Viewer's default).
    #[default]
    PerChannel,
    /// One per channel for a worn object (a HUD included), one per object
    /// for the others (2).
    PerChannelForAttachments,
    /// One per channel for a HUD, one per object for the others (3).
    PerChannelForHuds,
    /// No limit for a HUD, one per object for the others (4).
    UnlimitedForHuds,
    /// No limit at all (5).
    Unlimited,
}

impl Limit {
    pub const ALL: [Limit; 6] = [
        Limit::PerObject,
        Limit::PerChannel,
        Limit::PerChannelForAttachments,
        Limit::PerChannelForHuds,
        Limit::UnlimitedForHuds,
        Limit::Unlimited,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Limit::PerObject => aurora_i18n::tr!("ui-options-script-dialog-limit-per-object"),
            Limit::PerChannel => aurora_i18n::tr!("ui-options-script-dialog-limit-per-channel"),
            Limit::PerChannelForAttachments => aurora_i18n::tr!("ui-options-script-dialog-limit-per-channel-attachments"),
            Limit::PerChannelForHuds => aurora_i18n::tr!("ui-options-script-dialog-limit-per-channel-huds"),
            Limit::UnlimitedForHuds => aurora_i18n::tr!("ui-options-script-dialog-limit-unlimited-huds"),
            Limit::Unlimited => aurora_i18n::tr!("ui-options-script-dialog-limit-unlimited"),
        }
    }
}

/// Where the object that sent the dialog is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Rezzed in world, or not known to the viewer (the Legacy Viewer's
    /// object not found).
    InWorld,
    /// Worn on an avatar's body.
    Attachment,
    /// Worn on a HUD point (31 to 38).
    Hud,
}

/// Which older dialog of the same object a new one replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replaces {
    /// None: they all stay.
    Nothing,
    /// Any (findUsingObjectId).
    SameObject,
    /// The one on the same channel (findUsingObjectIdAndChannel).
    SameChannel,
}

/// The switch of LLScriptFloaterManager::onAddNotification.
pub fn replaces(limit: Limit, source: Source) -> Replaces {
    use {Limit::*, Replaces::*, Source::*};
    match (limit, source) {
        (PerObject, _) => SameObject,
        (PerChannel, _) => SameChannel,
        (PerChannelForAttachments, Attachment | Hud) => SameChannel,
        (PerChannelForHuds, Hud) => SameChannel,
        (UnlimitedForHuds, Hud) | (Unlimited, _) => Nothing,
        (PerChannelForAttachments | PerChannelForHuds | UnlimitedForHuds, _) => SameObject,
    }
}

/// Whether an open dialog of `old_object` on `old_channel` is replaced by a
/// new one of `object` on `channel` under `rule`.
pub fn replaced(rule: Replaces, (old_object, old_channel): (Uuid, i32), (object, channel): (Uuid, i32)) -> bool {
    old_object == object
        && match rule {
            Replaces::Nothing => false,
            Replaces::SameObject => true,
            Replaces::SameChannel => old_channel == channel,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Uuid = Uuid::from_u128(0xA);
    const B: Uuid = Uuid::from_u128(0xB);

    /// What stays of `open` (object, channel) once a dialog of `new` comes.
    fn after(limit: Limit, source: Source, open: &[(Uuid, i32)], new: (Uuid, i32)) -> Vec<(Uuid, i32)> {
        let rule = replaces(limit, source);
        let mut kept: Vec<_> = open.iter().copied().filter(|&old| !replaced(rule, old, new)).collect();
        kept.push(new);
        kept
    }

    #[test]
    fn the_default_is_one_per_channel_like_the_legacy_viewer() {
        assert_eq!(Limit::default(), Limit::PerChannel);
        // the Legacy Viewer's numbers are the list's order
        assert_eq!(Limit::ALL[1], Limit::PerChannel);
        assert_eq!(Limit::ALL[5], Limit::Unlimited);
    }

    #[test]
    fn one_per_object_replaces_any_channel_but_keeps_other_objects() {
        for source in [Source::InWorld, Source::Attachment, Source::Hud] {
            let open = [(A, 1), (B, 1)];
            assert_eq!(after(Limit::PerObject, source, &open, (A, 2)), vec![(B, 1), (A, 2)]);
        }
    }

    #[test]
    fn one_per_channel_keeps_other_channels_and_objects() {
        for source in [Source::InWorld, Source::Attachment, Source::Hud] {
            let open = [(A, 1), (A, 2), (B, 1)];
            assert_eq!(after(Limit::PerChannel, source, &open, (A, 1)), vec![(A, 2), (B, 1), (A, 1)]);
        }
    }

    #[test]
    fn per_channel_for_attachments_counts_huds_as_attachments() {
        let open = [(A, 1), (A, 2), (B, 1)];
        for source in [Source::Attachment, Source::Hud] {
            assert_eq!(
                after(Limit::PerChannelForAttachments, source, &open, (A, 1)),
                vec![(A, 2), (B, 1), (A, 1)]
            );
        }
        // an object in world: one per object
        assert_eq!(
            after(Limit::PerChannelForAttachments, Source::InWorld, &open, (A, 1)),
            vec![(B, 1), (A, 1)]
        );
    }

    #[test]
    fn per_channel_for_huds_limits_body_attachments_per_object() {
        let open = [(A, 1), (A, 2), (B, 1)];
        assert_eq!(
            after(Limit::PerChannelForHuds, Source::Hud, &open, (A, 1)),
            vec![(A, 2), (B, 1), (A, 1)]
        );
        for source in [Source::Attachment, Source::InWorld] {
            assert_eq!(after(Limit::PerChannelForHuds, source, &open, (A, 1)), vec![(B, 1), (A, 1)]);
        }
    }

    #[test]
    fn unlimited_for_huds_stacks_hud_dialogs_only() {
        let open = [(A, 1), (B, 1)];
        assert_eq!(
            after(Limit::UnlimitedForHuds, Source::Hud, &open, (A, 1)),
            vec![(A, 1), (B, 1), (A, 1)]
        );
        for source in [Source::Attachment, Source::InWorld] {
            assert_eq!(after(Limit::UnlimitedForHuds, source, &open, (A, 3)), vec![(B, 1), (A, 3)]);
        }
    }

    #[test]
    fn unlimited_stacks_everything() {
        for source in [Source::InWorld, Source::Attachment, Source::Hud] {
            let open = [(A, 1), (A, 1), (B, 1)];
            assert_eq!(after(Limit::Unlimited, source, &open, (A, 1)).len(), 4);
        }
    }

    #[test]
    fn every_choice_has_its_label() {
        for limit in Limit::ALL {
            assert!(!limit.label().is_empty());
        }
    }
}
