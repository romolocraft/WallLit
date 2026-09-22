#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Playback {
    Playing,
    Paused,
    Sleeping,
}

impl Playback {
    pub fn decide(dormant: bool, pause_when_fullscreen: bool, covered: bool) -> Self {
        if dormant {
            Self::Sleeping
        } else if pause_when_fullscreen && covered {
            Self::Paused
        } else {
            Self::Playing
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Playing => "playing",
            Self::Paused => "paused (covered)",
            Self::Sleeping => "sleeping",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullscreen_preference_never_overrides_session_or_manual_pause() {
        for enabled in [false, true] {
            for covered in [false, true] {
                assert_eq!(Playback::decide(true, enabled, covered), Playback::Sleeping);
            }
        }
    }

    #[test]
    fn covered_monitor_only_pauses_when_enabled() {
        assert_eq!(Playback::decide(false, false, true), Playback::Playing);
        assert_eq!(Playback::decide(false, true, true), Playback::Paused);
        assert_eq!(Playback::decide(false, true, false), Playback::Playing);
    }
}
