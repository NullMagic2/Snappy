//! Module: preferences. Purpose: persistent, separate image and video destinations.
//! Created: 2026-10-19. Architecture: GTK and exporters share these folder choices.
use crate::{desktop, files::Result, i18n::Language};
use std::path::{Path, PathBuf};

pub struct Preferences {
    pub images: PathBuf,
    pub videos: PathBuf,
    pub language: Language,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            images: Self::default_images(),
            videos: Self::default_videos(),
            language: Language::English,
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
        Self {
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

fn default_directory(kind: glib::UserDirectory, fallback: &str) -> PathBuf {
    glib::user_special_dir(kind)
        .unwrap_or_else(|| glib::home_dir().join(fallback))
        .join("Snappy")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remembers_independent_folders_and_language() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preferences.ini");
        let mut original = Preferences {
            images: temp.path().join("images with spaces"),
            videos: temp.path().join("vídeos"),
            language: Language::PortugueseBrazil,
        };
        original.save_to(&path).unwrap();
        original.images = temp.path().join("new image folder");
        original.save_to(&path).unwrap();
        let loaded = Preferences::load_from(&path);
        assert_eq!(loaded.images, original.images);
        assert_eq!(loaded.videos, original.videos);
        assert_eq!(loaded.language, Language::PortugueseBrazil);
    }
}
