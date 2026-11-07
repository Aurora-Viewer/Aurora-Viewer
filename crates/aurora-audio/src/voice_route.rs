/// Where to play voice relative to the main audio output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceStream {
    /// Use the main mix and output stream.
    Same,
    /// Open a separate output for voice only.
    Separate(String),
}

/// Chooses a separate voice output only when a named voice device differs
/// from the device currently used for the main mix. An empty voice name means
/// that voice follows the main output.
pub fn voice_stream_for(output: &str, voice: &str) -> VoiceStream {
    if voice.is_empty() || voice == output {
        VoiceStream::Same
    } else {
        VoiceStream::Separate(voice.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_voice_device_uses_the_main_output() {
        assert_eq!(voice_stream_for("Speakers", ""), VoiceStream::Same);
    }

    #[test]
    fn same_named_voice_device_uses_the_main_output() {
        assert_eq!(voice_stream_for("Headphones", "Headphones"), VoiceStream::Same);
    }

    #[test]
    fn a_different_voice_device_gets_a_separate_stream() {
        assert_eq!(
            voice_stream_for("Speakers", "Headphones"),
            VoiceStream::Separate("Headphones".into())
        );
    }
}
