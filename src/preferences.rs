//! Module: preferences. Purpose: persistent, separate image and video destinations.
//! Created: 2026-10-19. Architecture: GTK and exporters share these folder choices.
use crate::{capture::Area, desktop, files::Result, i18n::Language};
use std::path::{Path, PathBuf};

pub struct Preferences {
    pub images: PathBuf,
    pub videos: PathBuf,
    pub language: Language,
    pub show_pointer: bool,
    pub selection: Option<RememberedArea>,
    pub select_region: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            images: Self::default_images(),
            videos: Self::default_videos(),
            language: Language::English,
            show_pointer: false,
            selection: None,
            select_region: false,
        }
    }
}

impl Preferences {
    pub fn default_images() -> PathBuf {
        default_directory(glib::UserDirectory::Pictures, "Pictures")
    }
    pub fn default_videos() -> PathBuf {
        default_directory(glib::UserDirectory::Videos, "Videos")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::config_path())
    }

    fn load_from(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        let key = glib::KeyFile::new();
        if let Err(error) = key.load_from_file(path, glib::KeyFileFlags::NONE) {
            eprintln!("Snappy: cannot read folder preferences: {error}");
            return Self::default();
        }
        let read = |name, default| {
            key.string("folders", name)
                .ok()
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or(default)
        };
        let selection = (|| {
            let read = |name| key.integer("selection", name).ok();
            RememberedArea::new(
                Area {
                    x: read("x")?,
                    y: read("y")?,
                    width: read("width")?,
                    height: read("height")?,
                },
                read("screen-width")?,
                read("screen-height")?,
            )
        })();
        Self {
            selection,
            select_region: key.boolean("selection", "active").unwrap_or(false),
            show_pointer: key.boolean("capture", "show-pointer").unwrap_or(false),
            images: read("images", Self::default_images()),
            videos: read("videos", Self::default_videos()),
            language: Language::from_code(
                key.string("interface", "language")
                    .as_deref()
                    .unwrap_or("en"),
            ),
        }
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::config_path())
    }

    fn save_to(&self, path: &Path) -> Result<()> {
        let key = glib::KeyFile::new();
        for (name, path) in [("images", &self.images), ("videos", &self.videos)] {
            if !path.is_absolute() {
                return Err("Capture folder must be an absolute path".into());
            }
            key.set_string(
                "folders",
                name,
                path.to_str().ok_or("Capture folder must be UTF-8")?,
            );
        }
        key.set_string("interface", "language", self.language.code());
        key.set_boolean("capture", "show-pointer", self.show_pointer);
        key.set_boolean("selection", "active", self.select_region);
        if let Some(selection) = self.selection {
            for (name, value) in [
                ("x", selection.area.x),
                ("y", selection.area.y),
                ("width", selection.area.width),
                ("height", selection.area.height),
                ("screen-width", selection.width),
                ("screen-height", selection.height),
            ] {
                key.set_integer("selection", name, value);
            }
        }
        desktop::write_file(path, key.to_data().as_bytes(), true)
    }

    pub fn remember_image(&mut self, path: &Path) -> Result<()> {
        self.images = path
            .parent()
            .ok_or("Capture has no parent folder")?
            .to_owned();
        self.save()
    }

    fn config_path() -> PathBuf {
        glib::user_config_dir().join("snappy/preferences.ini")
    }
}

/// Native pixel geometry, independent of GTK scale. Invalid saved values are ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RememberedArea {
    pub area: Area,
    width: i32,
    height: i32,
}

impl RememberedArea {
    pub fn new(area: Area, width: i32, height: i32) -> Option<Self> {
        (width >= 2
            && height >= 2
            && area.x >= 0
            && area.y >= 0
            && area.width >= 2
            && area.height >= 2
            && i64::from(area.x) + i64::from(area.width) <= i64::from(width)
            && i64::from(area.y) + i64::from(area.height) <= i64::from(height))
        .then_some(Self {
            area,
            width,
            height,
        })
    }

    pub fn restore(self, width: i32, height: i32) -> Option<Area> {
        if width < 2 || height < 2 {
            return None;
        }
        let sx = f64::from(width) / f64::from(self.width);
        let sy = f64::from(height) / f64::from(self.height);
        Area::from_points(
            (f64::from(self.area.x) * sx, f64::from(self.area.y) * sy),
            (
                f64::from(self.area.x + self.area.width) * sx,
                f64::from(self.area.y + self.area.height) * sy,
            ),
            width,
            height,
        )
    }
}

fn default_directory(kind: glib::UserDirectory, fallback: &str) -> PathBuf {
    glib::user_special_dir(kind)
        .unwrap_or_else(|| glib::home_dir().join(fallback))
        .join("Snappy")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_scales_with_capture_bounds_and_rejects_invalid_geometry() {
        let area = Area {
            x: 100,
            y: 200,
            width: 400,
            height: 300,
        };
        let saved = RememberedArea::new(area, 1920, 1080).unwrap();
        assert_eq!(saved.restore(1920, 1080), Some(area));
        assert_eq!(
            saved.restore(3840, 2160),
            Some(Area {
                x: 200,
                y: 400,
                width: 800,
                height: 600
            })
        );
        assert!(
            RememberedArea::new(
                Area {
                    x: i32::MAX,
                    ..area
                },
                1920,
                1080
            )
            .is_none()
        );
        assert!(saved.restore(0, 0).is_none());
    }

    #[test]
    fn remembers_independent_folders_and_language() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preferences.ini");
        let mut original = Preferences {
            images: temp.path().join("images with spaces"),
            videos: temp.path().join("vídeos"),
            language: Language::PortugueseBrazil,
            show_pointer: true,
            selection: RememberedArea::new(
                Area {
                    x: 100,
                    y: 200,
                    width: 400,
                    height: 300,
                },
                1920,
                1080,
            ),
            select_region: true,
        };
        original.save_to(&path).unwrap();
        original.images = temp.path().join("new image folder");
        original.save_to(&path).unwrap();
        let loaded = Preferences::load_from(&path);
        assert_eq!(loaded.images, original.images);
        assert_eq!(loaded.videos, original.videos);
        assert_eq!(loaded.language, Language::PortugueseBrazil);
        assert!(loaded.show_pointer);
        assert_eq!(loaded.selection, original.selection);
        assert!(loaded.select_region);
    }
}
