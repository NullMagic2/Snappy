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

    /// Target kilobits/second for video and audio, and the maximum height. The frame
    /// rate of the recording is kept as it is. Never upscale.
    pub fn encoding(self) -> (u32, u32, u32) {
        match self {
            Self::Discord => (1500, 96, 720),
            Self::Small => (650, 64, 480),
            Self::High => (6000, 192, 1080),
            Self::Lossless => (0, 0, 0),
        }
    }

    /// Recorded frame size for a native capture area. Compressed presets are scaled
    /// while recording, so the file never exceeds the preset height: the aspect
    /// ratio is kept, nothing is upscaled and both dimensions are rounded to the
    /// nearest even number because H.264 4:2:0 needs even sizes. Full quality
    /// keeps the native size untouched.
    pub fn output_size(self, width: i32, height: i32) -> (i32, i32) {
        let (_, _, limit) = self.encoding();
        if self == Self::Lossless || width < 2 || height < 2 {
            return (width, height);
        }
        let target_height = height.min(limit as i32);
        let target_width =
            (f64::from(width) * f64::from(target_height) / f64::from(height)).round() as i32;
        (even(target_width), even(target_height))
    }

    /// Size estimate for a recording of the given native size at frame_rate frames per second.
    pub fn estimate(
        self,
        seconds: f64,
        sound: bool,
        width: i32,
        height: i32,
        frame_rate: u32,
    ) -> String {
        let seconds = seconds.max(1.0);
        if self == Self::Lossless {
            // Content-dependent RGB lossless compression: intentionally show a broad range.
            let raw = f64::from(width) * f64::from(height) * 3.0 * f64::from(frame_rate) * seconds;
            return format!(
                "{}: {}–{} {} {:.0}s\n{}",
                tr("Estimated size"),
                size(raw * 0.03),
                size(raw * 0.65),
                tr("for"),
                seconds,
                tr("Native resolution · source frame rate · lossless")
            );
        }
        let (video, audio, limit) = self.encoding();
        let bits = video + if sound { audio } else { 0 };
        let bytes = f64::from(bits) * 1000.0 / 8.0 * seconds * 1.02;
        format!(
            "{}: ≈ {} {} {:.0}s\n{} {limit}p · {}",
            tr("Estimated size"),
            size(bytes),
            tr("for"),
            seconds,
            tr("Up to"),
            tr("source frame rate")
        )
    }
}

/// Nearest even size of at least 2 pixels.
fn even(value: i32) -> i32 {
    ((value + 1) / 2 * 2).max(2)
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
        let short = Preset::Discord.estimate(60.0, true, 1920, 1080, 60);
        assert!(short.contains("12.2 MB"));
        assert!(
            Preset::Small
                .estimate(60.0, false, 1920, 1080, 60)
                .contains("5.0 MB")
        );
        // Lossless grows with the frame rate; compressed presets do not.
        let lossless_60 = Preset::Lossless.estimate(60.0, false, 1920, 1080, 60);
        let lossless_30 = Preset::Lossless.estimate(60.0, false, 1920, 1080, 30);
        assert_ne!(lossless_60, lossless_30);
        assert_eq!(Preset::Lossless.extension(), "mkv");
    }

    #[test]
    fn output_size_limits_height_without_upscaling_and_keeps_sizes_even() {
        assert_eq!(Preset::High.output_size(3840, 2160), (1920, 1080));
        assert_eq!(Preset::Discord.output_size(3840, 2160), (1280, 720));
        // A 21:9 selection keeps its aspect ratio and lands on an even width.
        assert_eq!(Preset::Small.output_size(2560, 1080), (1138, 480));
        // Smaller captures are not enlarged; odd sizes are rounded to even.
        assert_eq!(Preset::High.output_size(701, 401), (702, 402));
        assert_eq!(Preset::Lossless.output_size(701, 401), (701, 401));
    }
}
