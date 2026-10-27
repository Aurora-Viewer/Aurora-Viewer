//! The title the parcel's radio plays now and the ones it played before,
//! for the stream title window (`ui/stream_title.rs`). Port of
//! FSStreamTitleManager (indra/newview/fsfloaterstreamtitle.cpp, originally
//! LGPL 2.1): a title joins the history when it differs from the last one
//! kept, the history keeps the newest `FSStreamtitleHistoryLength` titles,
//! and clearing it keeps the title playing now.

/// One title of the history; `id` stays with it while older ones go, so
/// that the window's selection follows its titles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Played {
    pub id: u64,
    pub title: String,
}

#[derive(Debug, Default)]
pub struct StreamTitles {
    current: String,
    /// Oldest first, as the Legacy Viewer keeps it.
    history: Vec<Played>,
    next_id: u64,
}

impl StreamTitles {
    /// The title playing now, empty when no stream is.
    pub fn current(&self) -> &str {
        &self.current
    }

    /// The titles played, oldest first.
    pub fn history(&self) -> &[Played] {
        &self.history
    }

    /// processMetadataUpdate: the station's title now (empty when nothing
    /// plays), with at most `keep` titles in the history.
    pub fn update(&mut self, title: &str, keep: usize) {
        if title == self.current {
            return;
        }
        self.current = title.to_owned();
        if !title.is_empty() && self.history.last().is_none_or(|last| last.title != title) {
            self.push(title.to_owned());
            self.trim(keep);
        }
    }

    /// clearHistory: only the title playing now stays, when there is one
    /// and the history keeps any.
    pub fn clear(&mut self, keep: usize) {
        self.history.clear();
        if !self.current.is_empty() && keep > 0 {
            self.push(self.current.clone());
        }
    }

    /// The oldest titles dropped down to `keep`. The Legacy Viewer drops
    /// one title per new title, so a smaller maximum only shows after
    /// several titles; here it applies at once.
    pub fn trim(&mut self, keep: usize) {
        let extra = self.history.len().saturating_sub(keep);
        self.history.drain(..extra);
    }

    fn push(&mut self, title: String) {
        self.next_id += 1;
        self.history.push(Played { id: self.next_id, title });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(t: &StreamTitles) -> Vec<&str> {
        t.history().iter().map(|p| p.title.as_str()).collect()
    }

    #[test]
    fn keeps_new_titles_once_and_the_newest_ones() {
        let mut t = StreamTitles::default();
        t.update("A - One", 3);
        t.update("A - One", 3);
        t.update("B - Two", 3);
        assert_eq!(t.current(), "B - Two");
        assert_eq!(titles(&t), ["A - One", "B - Two"]);
        t.update("C - Three", 3);
        t.update("D - Four", 3);
        assert_eq!(titles(&t), ["B - Two", "C - Three", "D - Four"]);
    }

    #[test]
    fn a_stop_keeps_the_history_and_the_same_title_after_it_is_not_repeated() {
        let mut t = StreamTitles::default();
        t.update("A - One", 10);
        t.update("", 10);
        assert_eq!(t.current(), "");
        assert_eq!(titles(&t), ["A - One"]);
        t.update("A - One", 10);
        assert_eq!(titles(&t), ["A - One"]);
        t.update("B - Two", 10);
        t.update("A - One", 10);
        assert_eq!(titles(&t), ["A - One", "B - Two", "A - One"]);
    }

    #[test]
    fn clear_keeps_the_title_playing_now() {
        let mut t = StreamTitles::default();
        t.update("A - One", 10);
        t.update("B - Two", 10);
        t.clear(10);
        assert_eq!(titles(&t), ["B - Two"]);
        t.clear(0);
        assert!(t.history().is_empty());
        t.update("", 10);
        t.update("C - Three", 10);
        t.update("", 10);
        t.clear(10);
        assert!(t.history().is_empty());
    }

    #[test]
    fn ids_follow_their_titles_and_a_smaller_maximum_applies_at_once() {
        let mut t = StreamTitles::default();
        for title in ["1", "2", "3", "4"] {
            t.update(title, 10);
        }
        let id_of_3 = t.history()[2].id;
        t.trim(2);
        assert_eq!(titles(&t), ["3", "4"]);
        assert_eq!(t.history()[0].id, id_of_3);
        t.update("5", 0);
        assert!(t.history().is_empty());
        assert_eq!(t.current(), "5");
    }
}
