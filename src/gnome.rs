//! Module: gnome. Purpose: authenticated GNOME Shell capture integration.
//! Created: 2026-10-19. Architecture: native capture sources reuse the common exporters.
use crate::{desktop, files::Result, portal};
use gtk::{gio, prelude::*};
use std::{path::Path, sync::OnceLock};

pub const UUID: &str = "snappy@snappy.local";
const PATH: &str = "/io/github/snappy/Capture";
const IFACE: &str = "io.github.snappy.Capture.Shell";

fn gnome_session() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase()
        .split(':')
        .any(|part| part == "gnome" || part == "ubuntu")
}

fn installed() -> bool {
    std::iter::once(glib::user_data_dir())
        .chain(glib::system_data_dirs())
        .any(|root| {
            root.join(format!("gnome-shell/extensions/{UUID}/extension.js"))
                .is_file()
        })
}

async fn connection() -> Result<zbus::Connection> {
    static CONNECTION: OnceLock<tokio::sync::OnceCell<zbus::Connection>> = OnceLock::new();
    CONNECTION
        .get_or_init(tokio::sync::OnceCell::new)
        .get_or_try_init(|| async { zbus::Connection::session().await.map_err(|e| e.to_string()) })
        .await
        .cloned()
}

async fn proxy(connection: &zbus::Connection) -> Result<zbus::Proxy<'_>> {
    zbus::Proxy::new(connection, "org.gnome.Shell", PATH, IFACE)
        .await
        .map_err(|e| e.to_string())
}

/// An installed bridge must be enabled instead of unexpectedly falling back to sharing prompts.
pub async fn available() -> Result<bool> {
    if !gnome_session() {
        return Ok(false);
    }
    let connection = connection().await?;
    let native = proxy(&connection).await?;
    match native.call::<_, _, u32>("Ping", &()).await {
        Ok(1 | 2) => Ok(true),
        Ok(_) => Err("Unsupported Snappy GNOME Shell integration version".into()),
        Err(error) if installed() => Err(format!(
            "{}: {error}",
            crate::i18n::tr(
                "Snappy GNOME Shell integration is not ready. Sign out and back in once after installation, then enable Snappy Capture Integration in Extensions."
            )
        )),
        Err(_) => Ok(false),
    }
}

pub async fn screenshot(path: &Path, show_pointer: bool) -> Result<()> {
    let connection = connection().await?;
    let native = proxy(&connection).await?;
    let bytes: Vec<u8> = if show_pointer {
        require_current(&native).await?;
        native.call("ScreenshotWithPointer", &(show_pointer,)).await
    } else {
        native.call("Screenshot", &()).await
    }
    .map_err(|e| e.to_string())?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Shell returned an invalid screenshot".into());
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

/// A cached Shell module can survive an app/package restart. Refuse recording with
/// an older integration that cannot exclude the recording controls.
async fn require_current(native: &zbus::Proxy<'_>) -> Result<()> {
    let version: u32 = native.call("Ping", &()).await.map_err(|e| e.to_string())?;
    if version != 2 {
        return Err("GNOME is still running an older Snappy integration. Install the latest Snappy package, then sign out and back in to load it.".into());
    }
    Ok(())
}

pub struct Stream {
    connection: zbus::Connection,
    path: zbus::zvariant::OwnedObjectPath,
    pub node: u32,
}

impl Stream {
    pub async fn request(show_pointer: bool) -> Result<Self> {
        let connection = connection().await?;
        let native = proxy(&connection).await?;
        require_current(&native).await?;
        let (path, node) = native
            .call("StartStreamWithPointer", &(show_pointer,))
            .await
            .map_err(|e| e.to_string())?;
        Ok(Self {
            connection,
            path,
            node,
        })
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let connection = self.connection.clone();
        let path = self.path.clone();
        portal::runtime().spawn(async move {
            if let Ok(proxy) = proxy(&connection).await {
                let _ = proxy.call::<_, _, ()>("StopStream", &(path,)).await;
            }
        });
    }
}

/// Explicit installation only: install/enable the bridge without changing Shell security policy.
pub fn install() -> Result<()> {
    if !gnome_session() {
        return Ok(());
    }
    let relative = format!("gnome-shell/extensions/{UUID}");
    let local = glib::user_data_dir().join(&relative);
    let system = glib::system_data_dirs()
        .iter()
        .any(|root| root.join(&relative).join("extension.js").is_file());
    if !system || local.exists() {
        desktop::write_file(
            &local.join("metadata.json"),
            include_bytes!("../integration/snappy@snappy.local/metadata.json"),
            true,
        )?;
        desktop::write_file(
            &local.join("extension.js"),
            include_bytes!("../integration/snappy@snappy.local/extension.js"),
            true,
        )?;
    }
    let source =
        gio::SettingsSchemaSource::default().and_then(|s| s.lookup("org.gnome.shell", true));
    if let Some(schema) = source {
        let settings = gio::Settings::new_full(&schema, gio::SettingsBackend::NONE, None);
        let mut enabled: Vec<String> = settings
            .strv("enabled-extensions")
            .iter()
            .map(|s| s.to_string())
            .collect();
        if !enabled.iter().any(|id| id == UUID) {
            enabled.push(UUID.into());
            settings
                .set_strv("enabled-extensions", &enabled[..])
                .map_err(|e| e.to_string())?;
            gio::Settings::sync();
        }
    }
    match std::process::Command::new("gnome-extensions")
        .args(["enable", UUID])
        .output()
    {
        Ok(output) if output.status.success() => (),
        _ => eprintln!("Snappy: sign out and back in once to load the GNOME Shell integration."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run only against an isolated Shell/Mutter test service and virtual display.
    #[test]
    #[ignore = "Requires private D-Bus Shell fixture, virtual display, and executable named snappy"]
    fn native_bridge_authentication_and_lifecycle() {
        assert_eq!(std::env::var("SNAPPY_BRIDGE_TEST").unwrap(), "isolated");
        gtk::init().unwrap();
        let app = gtk::Application::new(Some(crate::APP_ID), gio::ApplicationFlags::FLAGS_NONE);
        app.register(gio::Cancellable::NONE).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("snapshot.png");
        portal::runtime().block_on(async {
            assert!(available().await.unwrap());
            screenshot(&image, false).await.unwrap();
            assert!(
                std::fs::read(&image)
                    .unwrap()
                    .starts_with(b"\x89PNG\r\n\x1a\n")
            );
            let stream = Stream::request(false).await.unwrap();
            assert_eq!(stream.node, 777);
            drop(stream);
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        });
        let denied = std::process::Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.gnome.Shell",
                "--object-path",
                PATH,
                "--method",
                "io.github.snappy.Capture.Shell.Screenshot",
            ])
            .output()
            .unwrap();
        assert!(!denied.status.success());
        assert!(String::from_utf8_lossy(&denied.stderr).contains("Only the active Snappy"));
        let stopped = std::env::var("SNAPPY_BRIDGE_STOP_LOG").unwrap();
        assert!(
            std::fs::read_to_string(stopped)
                .unwrap()
                .contains("stopped")
        );
    }
}
