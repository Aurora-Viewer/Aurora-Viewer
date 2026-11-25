//! The animations the inventory's previews play (LLPreviewAnim,
//! indra/newview/llpreviewanim.cpp, originally LGPL 2.1): each preview plays
//! and stops its own, side by side with the others.

use uuid::Uuid;

/// An animation a preview's Play buttons started, until its preview closes
/// or it plays to its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewPlay {
    pub asset: Uuid,
    /// Played on this viewer only, not for everyone.
    pub local: bool,
    /// Seen among our avatar's motions (mDidStart).
    started: bool,
    /// Started, and no longer among them: the Play buttons are back, and
    /// closing the preview still asks for the stop (LLPreviewAnim::onClose).
    pub over: bool,
}

impl PreviewPlay {
    pub fn new(asset: Uuid, local: bool) -> Self {
        Self {
            asset,
            local,
            started: false,
            over: false,
        }
    }
}

/// The play of `asset`, unless it is over.
pub fn playing(plays: &[PreviewPlay], asset: Uuid) -> Option<PreviewPlay> {
    plays.iter().find(|play| play.asset == asset && !play.over).copied()
}

/// Take the play of `asset` out, leaving the others.
pub fn take(plays: &mut Vec<PreviewPlay>, asset: Uuid) -> Option<PreviewPlay> {
    let at = plays.iter().position(|play| play.asset == asset)?;
    Some(plays.remove(at))
}

/// LLPreviewAnim::draw: note the plays that started, then stopped. One the
/// simulator lists again (a teleport's first list lacked it) plays again.
pub fn follow(plays: &mut [PreviewPlay], active: impl Fn(Uuid) -> bool) {
    for play in plays {
        let active = active(play.asset);
        play.started |= active;
        play.over = play.started && !active;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_play_side_by_side_and_one_stops_alone() {
        let [first, second] = [Uuid::from_u128(10), Uuid::from_u128(11)];
        let mut plays = vec![PreviewPlay::new(first, true), PreviewPlay::new(second, false)];
        assert_eq!(take(&mut plays, Uuid::from_u128(12)), None);
        assert_eq!(take(&mut plays, first).map(|play| play.asset), Some(first));
        assert!(playing(&plays, first).is_none());
        assert!(playing(&plays, second).is_some());
    }

    #[test]
    fn a_stopped_play_gives_its_buttons_back_and_is_still_known_at_close() {
        let asset = Uuid::from_u128(10);
        let mut plays = vec![PreviewPlay::new(asset, false)];
        follow(&mut plays, |_| false);
        assert!(playing(&plays, asset).is_some(), "asked, not started yet");
        follow(&mut plays, |_| true);
        assert!(playing(&plays, asset).is_some());
        follow(&mut plays, |_| false);
        assert!(playing(&plays, asset).is_none());
        // a list without it, then one with it again
        follow(&mut plays, |_| true);
        assert!(playing(&plays, asset).is_some());
        follow(&mut plays, |_| false);
        assert!(take(&mut plays, asset).is_some(), "the close stops it");
    }
}
