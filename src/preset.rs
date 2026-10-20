//! Module: preset. Purpose: export choices and honest size estimates.
//! Created: 2026-10-19. Architecture: one policy shared by UI and encoder.
use crate::i18n::tr;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Discord,
    Small,
    High,
    Lossless,
}

impl Preset {
    pub const ALL: [Self; 4] = [Self::Discord, Self::Small, Self::High, Self::Lossless];

    pub fn label(self) -> &'static str {
        match self {
            Self::Discord => "Discord-friendly · MP4 · balanced",
            Self::Small => "Small file · MP4 · compact",
            Self::High => "High quality · MP4 · sharp",
            Self::Lossless => "Full quality · MKV · lossless",
        }
    }

    pub fn extension(self) -> &'static str {
        if self == Self::Lossless { "mkv" } else { "mp4" }
    }

    /// Target kilobits/second, maximum height and output FPS. Never upscale.
    pub fn encoding(self) -> (u32, u32, u32, u32) {
        match self {
            Self::Discord => (1500, 96, 720, 30),
            Self::Small => (650, 64, 480, 24),
            Self::High => (6000, 192, 1080, 30),
            Self::Lossless => (0, 0, 0, 30),
        }
    }

    pub fn estimate(self, seconds: f64, sound: bool, width: i32, height: i32) -> String {
        let seconds = seconds.max(1.0);
        if self == Self::Lossless {
            // Content-dependent RGB lossless compression: intentionally show a broad range.
            let raw = f64::from(width) * f64::from(height) * 3.0 * 30.0 * seconds;
            return format!(
                "{}: {}–{} {} {:.0}s\n{}",
                tr("Estimated size"),
                size(raw * 0.03),
                size(raw * 0.65),
                tr("for"),
                seconds,
                tr("Native resolution · 30 fps · lossless")
            );
        }
        let (video, audio, limit, fps) = self.encoding();
        let bits = video + if sound { audio } else { 0 };
        let bytes = f64::from(bits) * 1000.0 / 8.0 * seconds * 1.02;
        format!(
            "{}: ≈ {} {} {:.0}s\n{} {limit}p · {fps} fps",
            tr("Estimated size"),
            size(bytes),
            tr("for"),
            seconds,
            tr("Up to")
        )
    }
}

pub fn size(bytes: f64) -> String {
    if bytes >= 1_000_000_000.0 {
        format!("{:.2} GB", bytes / 1_000_000_000.0)
    } else if bytes >= 1_000_000.0 {
        format!("{:.1} MB", bytes / 1_000_000.0)
    } else if bytes >= 1000.0 {
        format!("{:.1} KB", bytes / 1000.0)
    } else {
        format!("{bytes:.0} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn estimate_and_encoder_share_rates() {
        let short = Preset::Discord.estimate(60.0, true, 1920, 1080);
        assert!(short.contains("12.2 MB"));
        assert!(
            Preset::Small
                .estimate(60.0, false, 1920, 1080)
                .contains("5.0 MB")
        );
        assert_eq!(Preset::Lossless.extension(), "mkv");
    }
}
