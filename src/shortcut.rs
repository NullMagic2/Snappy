//! Module: shortcut. Purpose: global Print Screen activation with explicit conflict reporting.
//! Created: 2026-10-19. Architecture: workers send events; only the UI touches windows.
use crate::{
    files::Result,
    i18n::{self, tr},
    portal,
};
use glib::Sender;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

pub enum Event {
    Open,
    Status(Status),
}

pub enum Status {
    Registering,
    Managed,
    Ready,
    Bound(String),
    Unavailable(String),
}

impl Status {
    pub fn label(&self) -> String {
        match self {
            Self::Registering => tr("Registering Print Screen…").into(),
            Self::Managed => tr("Print Screen: managed by the desktop").into(),
            Self::Ready => tr("Print Screen: ready").into(),
            Self::Bound(trigger) => format!("{}: {trigger}", tr("Capture shortcut")),
            Self::Unavailable(error) => format!(
                "{}\n{}",
                i18n::error(error),
                tr("Bind Print Screen to snappy --capture in system Keyboard Shortcuts.")
            ),
        }
    }
}

pub fn start(wayland: bool, sender: Sender<Event>) {
    if crate::default_capture::is_managed() {
        let _ = sender.send(Event::Status(Status::Managed));
        return;
    }
    thread::spawn(move || {
        let result = if wayland {
            portal::runtime().block_on(wayland_shortcut(&sender))
        } else {
            x11_shortcut(&sender)
        };
        if let Err(error) = result {
            let _ = sender.send(Event::Status(Status::Unavailable(error)));
        }
    });
}

async fn wayland_shortcut(sender: &Sender<Event>) -> Result<()> {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    portal::initialize().await?;
    let proxy = GlobalShortcuts::new()
        .await
        .map_err(|e| format!("Global shortcut portal unavailable: {e}"))?;
    let session = proxy.create_session().await.map_err(|e| e.to_string())?;
    let mut events = proxy.receive_activated().await.map_err(|e| e.to_string())?;
    let shortcut = NewShortcut::new("capture", tr("Open Snappy")).preferred_trigger("Print");
    let response = proxy
        .bind_shortcuts(&session, &[shortcut], None)
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;
    let description = response
        .shortcuts()
        .first()
        .map(|s| s.trigger_description())
        .unwrap_or(tr("System shortcut"));
    let _ = sender.send(Event::Status(Status::Bound(description.into())));
    while events.next().await.is_some() {
        if sender.send(Event::Open).is_err() {
            break;
        }
    }
    let _ = session.close().await;
    Ok(())
}

static GRAB_FAILED: AtomicBool = AtomicBool::new(false);
unsafe extern "C" fn grab_error(
    _: *mut x11_dl::xlib::Display,
    _: *mut x11_dl::xlib::XErrorEvent,
) -> i32 {
    GRAB_FAILED.store(true, Ordering::Relaxed);
    0
}

fn x11_shortcut(sender: &Sender<Event>) -> Result<()> {
    use x11_dl::xlib;
    let x = xlib::Xlib::open().map_err(|e| e.to_string())?;
    // SAFETY: this worker owns one independent X connection. Event storage is correctly
    // aligned XEvent. The temporary handler catches asynchronous grab errors, then is restored.
    unsafe {
        let display = (x.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return Err("Cannot register Print Screen without an X11 display".into());
        }
        let root = (x.XDefaultRootWindow)(display);
        let key = (x.XKeysymToKeycode)(display, 0xff61) as i32;
        let previous = (x.XSetErrorHandler)(Some(grab_error));
        GRAB_FAILED.store(false, Ordering::Relaxed);
        for modifier in [
            0,
            xlib::LockMask,
            xlib::Mod2Mask,
            xlib::LockMask | xlib::Mod2Mask,
        ] {
            (x.XGrabKey)(
                display,
                key,
                modifier,
                root,
                0,
                xlib::GrabModeAsync,
                xlib::GrabModeAsync,
            );
        }
        (x.XSync)(display, 0);
        (x.XSetErrorHandler)(previous);
        if GRAB_FAILED.load(Ordering::Relaxed) {
            (x.XUngrabKey)(display, key, xlib::AnyModifier, root);
            (x.XCloseDisplay)(display);
            return Err("Print Screen is already assigned to another application. Change its system shortcut to use Snappy.".into());
        }
        let _ = sender.send(Event::Status(Status::Ready));
        loop {
            while (x.XPending)(display) > 0 {
                let mut event: xlib::XEvent = std::mem::zeroed();
                (x.XNextEvent)(display, &mut event);
                if event.get_type() == xlib::KeyPress && sender.send(Event::Open).is_err() {
                    (x.XCloseDisplay)(display);
                    return Ok(());
                }
            }
            thread::sleep(Duration::from_millis(40));
        }
    }
}
