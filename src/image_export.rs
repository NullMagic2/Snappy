//! Module: image_export. Purpose: screenshot formats and byte-exact export previews.
//! Created: 2026-10-19. Architecture: workers receive immutable pixels and return encoded bytes.
use crate::files::Result;
use gtk::gdk_pixbuf::Pixbuf;
use std::{process::Command, time::Duration};

pub fn extension(profile: u32) -> &'static str {
    match profile {
        1 | 2 => "jpg",
        3 => "bmp",
        4 => "gif",
        _ => "png",
    }
}

/// Immutable native pixels, including crop row stride, may safely cross worker threads.
#[derive(Clone)]
pub struct Snapshot {
    pixels: glib::Bytes,
    width: i32,
    height: i32,
    rowstride: i32,
    alpha: bool,
}

impl Snapshot {
    pub fn new(image: &Pixbuf) -> Self {
        Self {
            pixels: image.read_pixel_bytes(),
            width: image.width(),
            height: image.height(),
            rowstride: image.rowstride(),
            alpha: image.has_alpha(),
        }
    }

    fn image(&self) -> Pixbuf {
        Pixbuf::from_bytes(
            &self.pixels,
            gtk::gdk_pixbuf::Colorspace::Rgb,
            self.alpha,
            8,
            self.width,
            self.height,
            self.rowstride,
        )
    }
}

/// All encoders run on a worker; no full-resolution PNG is encoded on the UI thread.
/// GIF is a single-frame image, quantized with an image-specific palette.
pub fn encode(snapshot: Snapshot, profile: u32) -> Result<Vec<u8>> {
    let image = snapshot.image();
    if profile == 4 {
        let scratch = tempfile::Builder::new()
            .prefix("snappy-gif-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let input = scratch.path().join("input.png");
        let output = scratch.path().join("output.gif");
        std::fs::write(
            &input,
            image
                .save_to_bufferv("png", &[])
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let mut command = Command::new("ffmpeg");
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&input)
            .args([
                "-filter_complex_threads",
                "1",
                "-filter_complex",
                "split[a][b];[a]palettegen=stats_mode=single[p];[b][p]paletteuse=dither=sierra2_4a",
                "-frames:v",
                "1",
            ])
            .arg(&output);
        crate::capture::run_image_command(command, scratch.path(), Duration::from_secs(30))?;
        return std::fs::read(output).map_err(|e| e.to_string());
    }
    match profile {
        0 => image.save_to_bufferv("png", &[]),
        1 => image.save_to_bufferv("jpeg", &[("quality", "90")]),
        2 => image.save_to_bufferv("jpeg", &[("quality", "70")]),
        3 => image.save_to_bufferv("bmp", &[]),
        _ => return Err("Unknown screenshot format".into()),
    }
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_export_preserves_crop_pixels_and_stride() {
        for alpha in [false, true] {
            let image = Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, alpha, 8, 19, 13).unwrap();
            image.fill(0x27a3d97f);
            let crop = image.new_subpixbuf(3, 2, 7, 5);
            let snapshot = Snapshot::new(&crop);
            let expected = crop.read_pixel_bytes();
            let channels = crop.n_channels() as usize;
            let bytes = std::thread::spawn(move || encode(snapshot, 0).unwrap())
                .join()
                .unwrap();
            let decoded = Pixbuf::from_read(std::io::Cursor::new(bytes)).unwrap();
            assert_eq!(
                (decoded.width(), decoded.height(), decoded.has_alpha()),
                (7, 5, alpha)
            );
            let actual = decoded.read_pixel_bytes();
            for y in 0..5 {
                let a = y * crop.rowstride() as usize;
                let b = y * decoded.rowstride() as usize;
                assert_eq!(&expected[a..a + 7 * channels], &actual[b..b + 7 * channels]);
            }
        }
    }
}
