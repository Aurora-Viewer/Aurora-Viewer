use super::{ChatUi, compose::Volume, earshot_count};
use crate::world::radar::ChatRanges;

#[test]
fn earshot_count_follows_whisper_say_and_shout_ranges() {
    let distances = [Some(5.0), Some(15.0), Some(50.0), Some(150.0)];
    let ranges = ChatRanges::default();
    let mut chat = ChatUi::default();

    for (volume, expected) in [(Volume::Whisper, 1), (Volume::Say, 2), (Volume::Shout, 3)] {
        chat.volume = volume;
        let count = earshot_count(distances, chat.earshot_range(ranges));
        assert_eq!(count, expected, "{volume:?}");
    }
}

#[test]
fn earshot_count_uses_opensim_chat_ranges_and_skips_unknown_distances() {
    let distances = [Some(5.0), Some(15.0), Some(50.0), Some(150.0), None];
    let ranges = ChatRanges {
        whisper: 8.0,
        say: 30.0,
        shout: 60.0,
    };
    let mut chat = ChatUi::default();

    for (volume, expected) in [(Volume::Whisper, 1), (Volume::Say, 2), (Volume::Shout, 3)] {
        chat.volume = volume;
        let count = earshot_count(distances, chat.earshot_range(ranges));
        assert_eq!(count, expected, "{volume:?}");
    }
}
