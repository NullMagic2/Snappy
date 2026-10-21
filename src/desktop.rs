//! Module: desktop. Purpose: one application identity for launchers, portals and dock icons.
//! Created: 2026-10-19. Architecture: shared by first startup and explicit installation.
use crate::{APP_ID, files::Result};
use gtk::gdk_pixbuf::{InterpType, Pixbuf};
use std::{
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

pub const ICON: &[u8] = include_bytes!("../assets/snappy.png");

/// A portable launch must expose its desktop ID before making any portal request.
pub fn ensure_identity() -> Result<()> {
    install_icons(false)?;
    let filename = format!("{APP_ID}.desktop");
    let installed = std::iter::once(glib::user_data_dir())
        .chain(glib::system_data_dirs())
        .any(|directory| directory.join("applications").join(&filename).is_file());
    if !installed {
        write_launcher(
            &std::env::current_exe().map_err(|e| e.to_string())?,
            false,
            false,
        )?;
    }
    Ok(())
}

pub fn install(autostart: bool) -> Result<()> {
    crate::gnome::install()?;
    install_icons(true)?;
    write_launcher(
        &std::env::current_exe().map_err(|e| e.to_string())?,
        autostart,
        true,
    )?;
    Ok(())
}

fn entry(binary: &Path) -> String {
    // Desktop Entry escaping is applied twice: Exec argument quoting, then value escaping.
    let quoted = binary
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%")
        .replace('\\', "\\\\");
    include_str!("../packaging/io.github.snappy.Capture.desktop").replace(
        "Exec=/usr/lib/snappy/launch",
        &format!("Exec=\"{quoted}\" --capture"),
    )
}

fn write_launcher(binary: &Path, autostart: bool, replace: bool) -> Result<()> {
    let text = entry(binary);
    let launcher = glib::user_data_dir()
        .join("applications")
        .join(format!("{APP_ID}.desktop"));
    write_file(&launcher, text.as_bytes(), replace)?;
    if autostart {
        let path = glib::user_config_dir()
            .join("autostart")
            .join(format!("{APP_ID}.desktop"));
        write_file(&path, text.as_bytes(), true)?;
    }
    Ok(())
}

pub(crate) fn write_file(path: &Path, data: &[u8], replace: bool) -> Result<()> {
    if !replace && path.exists() {
        return Ok(());
    }
    let directory = path
        .parent()
        .ok_or("Missing desktop integration directory")?;
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    file.write_all(data).map_err(|e| e.to_string())?;
    if replace {
        file.persist(path).map_err(|e| e.to_string())?;
    } else if let Err(error) = file.persist_noclobber(path) {
        if error.error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(error.to_string());
        }
    }
    Ok(())
}

fn install_icons(replace: bool) -> Result<()> {
    let mut image = None;
    for size in [32, 48, 64, 128, 256, 512] {
        let path: PathBuf =
            glib::user_data_dir().join(format!("icons/hicolor/{size}x{size}/apps/{APP_ID}.png"));
        if !replace && path.exists() {
            continue;
        }
        let image = match &image {
            Some(image) => image,
            None => image.insert(Pixbuf::from_read(Cursor::new(ICON)).map_err(|e| e.to_string())?),
        };
        let scaled = image
            .scale_simple(size, size, InterpType::Bilinear)
            .ok_or("Cannot resize app icon")?;
        let bytes = scaled
            .save_to_bufferv("png", &[])
            .map_err(|e| e.to_string())?;
        write_file(&path, &bytes, replace)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launcher_uses_one_id_for_dock_and_portal() {
        let text = entry(Path::new("/tmp/app folder/snappy"));
        assert!(text.contains(&format!("Icon={APP_ID}\nStartupWMClass={APP_ID}")));
        let key = glib::KeyFile::new();
        key.load_from_data(&text, glib::KeyFileFlags::NONE).unwrap();
        assert_eq!(
            key.string("Desktop Entry", "Exec").unwrap(),
            "\"/tmp/app folder/snappy\" --capture"
        );
    }
}
