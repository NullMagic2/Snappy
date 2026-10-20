//! Module: portal. Purpose: Wayland permission sessions and PipeWire connections.
//! Created: 2026-10-19. Architecture: no GTK objects cross into portal workers.
use crate::files::Result;
use ashpd::desktop::{
    PersistMode, Session,
    screencast::{CursorMode, Screencast, SourceType},
};
use std::{os::fd::OwnedFd, path::PathBuf, sync::OnceLock};

pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("create portal runtime"))
}

/// Register the shared D-Bus connection before any portal operation.
pub async fn initialize() -> Result<()> {
    static READY: tokio::sync::OnceCell<Result<()>> = tokio::sync::OnceCell::const_new();
    READY
        .get_or_init(|| async {
            crate::desktop::ensure_identity()?;
            let app_id = crate::APP_ID
                .parse()
                .map_err(|e| format!("Invalid application ID: {e}"))?;
            match ashpd::register_host_app(app_id).await {
                Ok(()) => Ok(()),
                Err(ashpd::Error::PortalNotFound(_)) => Ok(()),
                Err(ashpd::Error::Zbus(zbus::Error::MethodError(name, _, _)))
                    if matches!(
                        name.as_str(),
                        "org.freedesktop.DBus.Error.UnknownMethod"
                            | "org.freedesktop.DBus.Error.UnknownInterface"
                    ) =>
                {
                    Ok(())
                }
                Err(error) => Err(format!(
                    "Cannot register Snappy with the desktop portal: {error}"
                )),
            }
        })
        .await
        .clone()
}

pub struct Stream {
    proxy: Screencast<'static>,
    session: Option<Session<'static, Screencast<'static>>>,
    pub node: u32,
    pub position: Option<(i32, i32)>,
}

impl Stream {
    /// Requests one monitor. The compositor owns screen choice and consent.
    pub async fn request() -> Result<Self> {
        initialize().await?;
        let proxy = Screencast::new().await.map_err(portal_error)?;
        let session = proxy.create_session().await.map_err(portal_error)?;
        let token_path = glib::user_config_dir().join("snappy/screencast-restore-token");
        let restore_token = std::fs::read_to_string(&token_path).ok();
        let setup = async {
            proxy
                .select_sources(
                    &session,
                    // Keep the frozen overlay free of a second, captured pointer.
                    CursorMode::Hidden,
                    SourceType::Monitor.into(),
                    false,
                    restore_token.as_deref(),
                    PersistMode::ExplicitlyRevoked,
                )
                .await
                .map_err(portal_error)?
                .response()
                .map_err(portal_error)?;
            let response = proxy
                .start(&session, None)
                .await
                .map_err(portal_error)?
                .response()
                .map_err(portal_error)?;
            // Tokens are single-use: persist the replacement returned by every Start.
            if let Some(token) = response.restore_token() {
                if let Err(error) = crate::desktop::write_file(&token_path, token.as_bytes(), true)
                {
                    eprintln!("Snappy: could not remember screen permission: {error}");
                }
            } else if restore_token.is_some() {
                let _ = std::fs::remove_file(&token_path);
            }
            response
                .streams()
                .first()
                .map(|s| (s.pipe_wire_node_id(), s.position()))
                .ok_or_else(|| "The desktop did not provide a screen stream".to_string())
        }
        .await;
        match setup {
            Ok((node, position)) => Ok(Self {
                proxy,
                session: Some(session),
                node,
                position,
            }),
            Err(error) => {
                let _ = session.close().await;
                Err(error)
            }
        }
    }

    /// Each consumer gets a fresh connection; a used PipeWire socket is never reused.
    pub async fn remote(&self) -> Result<OwnedFd> {
        self.proxy
            .open_pipe_wire_remote(self.session.as_ref().expect("live portal session"))
            .await
            .map_err(portal_error)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            runtime().spawn(async move {
                let _ = session.close().await;
            });
        }
    }
}

pub async fn screenshot() -> Result<PathBuf> {
    initialize().await?;
    let response = ashpd::desktop::screenshot::Screenshot::request()
        .interactive(false)
        .modal(true)
        .send()
        .await
        .map_err(portal_error)?
        .response()
        .map_err(portal_error)?;
    response
        .uri()
        .to_file_path()
        .map_err(|_| "Portal returned a non-local screenshot".into())
}

fn portal_error(error: ashpd::Error) -> String {
    format!(
        "Desktop portal: {error}. Check that xdg-desktop-portal and your desktop's portal backend are installed. You can cancel and try again."
    )
}
