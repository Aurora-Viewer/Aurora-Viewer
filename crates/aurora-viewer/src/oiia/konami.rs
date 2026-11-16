//! The Konami code: up, up, down, down, left, right, left, right, B, A.
//! The keys are only watched, never taken: they still move the avatar.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    B,
    A,
    /// Any other key: it breaks the code.
    Other,
}

const CODE: [Key; 10] = [
    Key::Up,
    Key::Up,
    Key::Down,
    Key::Down,
    Key::Left,
    Key::Right,
    Key::Left,
    Key::Right,
    Key::B,
    Key::A,
];

/// The longest pause between two keys of the code: walking around for a
/// while, then typing two letters, is not the code.
const PAUSE: Duration = Duration::from_secs(2);

/// The last keys pressed, to tell when they spell the code.
#[derive(Debug)]
pub struct Konami {
    recent: [Key; CODE.len()],
    last: Option<Instant>,
}

impl Default for Konami {
    fn default() -> Self {
        Self {
            recent: [Key::Other; CODE.len()],
            last: None,
        }
    }
}

impl Konami {
    /// A key went down (not a repeat of a held key). True when it completes
    /// the code; the keys before it are then forgotten.
    pub fn press(&mut self, key: Key, now: Instant) -> bool {
        if self.last.is_some_and(|last| now.duration_since(last) > PAUSE) {
            self.forget();
        }
        self.last = Some(now);
        self.recent.rotate_left(1);
        self.recent[CODE.len() - 1] = key;
        let done = self.recent == CODE;
        if done {
            self.forget();
        }
        done
    }

    /// Keys typed into a text field are not the code.
    pub fn forget(&mut self) {
        *self = Self::default();
    }

    /// How many keys of the code the last keys pressed spell so far.
    pub fn progress(&self) -> usize {
        (1..=CODE.len())
            .rev()
            .find(|n| self.recent[CODE.len() - n..] == CODE[..*n])
            .unwrap_or(0)
    }
}

/// The arrows the code begins with: its letters come after them.
pub const ARROWS: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(konami: &mut Konami, keys: &[Key], start: Instant) -> Vec<bool> {
        keys.iter()
            .enumerate()
            .map(|(i, key)| konami.press(*key, start + Duration::from_millis(200 * i as u64)))
            .collect()
    }

    #[test]
    fn the_code_and_only_its_last_key_completes_it() {
        let mut konami = Konami::default();
        let done = typed(&mut konami, &CODE, Instant::now());
        assert_eq!(done.iter().filter(|d| **d).count(), 1);
        assert!(done[CODE.len() - 1]);
        // Forgotten once done: the last key again does nothing.
        assert!(!konami.press(Key::A, Instant::now()));
    }

    #[test]
    fn a_third_up_before_the_code_does_not_break_it() {
        let mut konami = Konami::default();
        let mut keys = vec![Key::Up];
        keys.extend(CODE);
        assert!(typed(&mut konami, &keys, Instant::now())[keys.len() - 1]);
    }

    #[test]
    fn a_wrong_key_or_a_long_pause_breaks_it() {
        let now = Instant::now();
        let mut konami = Konami::default();
        let mut keys = CODE.to_vec();
        keys.insert(5, Key::Other);
        assert!(typed(&mut konami, &keys, now).iter().all(|d| !d));

        let mut konami = Konami::default();
        assert!(typed(&mut konami, &CODE[..8], now).iter().all(|d| !d));
        let later = now + Duration::from_secs(10);
        assert!(!konami.press(Key::B, later));
        assert!(!konami.press(Key::A, later));
    }

    #[test]
    fn the_progress_counts_the_keys_of_the_code_typed_so_far() {
        let mut konami = Konami::default();
        assert_eq!(konami.progress(), 0);
        typed(&mut konami, &CODE[..ARROWS], Instant::now());
        assert_eq!(konami.progress(), ARROWS);
        konami.press(Key::Other, Instant::now());
        assert_eq!(konami.progress(), 0);
        // Up twice again: the code may be starting over.
        konami.press(Key::Up, Instant::now());
        konami.press(Key::Up, Instant::now());
        assert_eq!(konami.progress(), 2);
    }

    #[test]
    fn keys_typed_in_a_text_field_are_forgotten() {
        let now = Instant::now();
        let mut konami = Konami::default();
        typed(&mut konami, &CODE[..9], now);
        konami.forget();
        assert!(!konami.press(Key::A, now + Duration::from_secs(2)));
    }
}
