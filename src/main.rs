//! Module: application. Purpose: GTK startup and single-instance command routing.
//! Created: 2026-10-19. Architecture: UI owns the main thread; workers own capture.
mod capture;
mod default_capture;
mod desktop;
mod files;
mod gnome;
mod i18n;
mod image_export;
mod portal;
mod preferences;
mod preset;
mod selection;
mod shortcut;
mod ui;

use gtk::{gio, prelude::*};

pub const APP_ID: &str = "io.github.snappy.Capture";

/// XWayland exposes an X11 display but cannot capture a Wayland desktop's pixels.
fn is_wayland_session() -> bool {
    session_uses_wayland(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
    )
}

fn session_uses_wayland(session: Option<&str>, display: Option<&str>) -> bool {
    match session {
        Some(kind) if kind.eq_ignore_ascii_case("wayland") => true,
        Some(kind) if kind.eq_ignore_ascii_case("x11") => false,
        _ => display.is_some_and(|name| !name.is_empty()),
    }
}

fn main() -> glib::ExitCode {
    if std::env::args().any(|arg| arg == "--version") {
        println!("Snappy {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    if std::env::args().any(|arg| arg == "--help") {
        println!(
            "Snappy\n\nUsage: snappy [--capture | --install [--autostart] [--no-default] | --restore-shortcuts | --install-integration | --version | --help]\n\n--capture  Open the capture controls.\n--install  Install launcher/icons and assign Print Screen to Snappy (GNOME/Cinnamon).\n--no-default  Install without changing keyboard shortcuts.\n--restore-shortcuts  Restore keyboard preferences saved during installation.\nUse Settings to quit. Recording continues until Stop is pressed. PNG, JPEG, BMP or GIF screenshots and MP4 or MKV recordings.\nRequires a graphical session; Wayland uses desktop portals."
        );
        return glib::ExitCode::SUCCESS;
    }

    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.as_slice() == ["--install-integration"] {
        return match gnome::install() {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if arguments.as_slice() == ["--restore-shortcuts"] {
        return match default_capture::restore() {
            Ok(()) => {
                println!("Restored previous Print Screen settings");
                glib::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if arguments.first().map(String::as_str) == Some("--install") {
        if arguments
            .iter()
            .skip(1)
            .any(|arg| arg != "--autostart" && arg != "--no-default")
        {
            return glib::ExitCode::from(2);
        }
        let make_default = !arguments.iter().any(|arg| arg == "--no-default");
        let installed = desktop::install(arguments.iter().any(|arg| arg == "--autostart"))
            .and_then(|()| {
                if make_default {
                    default_capture::install()
                } else {
                    Ok(())
                }
            });
        return match installed {
            Ok(()) => {
                println!("Installed Snappy launcher and dock icons");
                if make_default {
                    println!(
                        "Print Screen now opens Snappy. Restore with: snappy --restore-shortcuts"
                    );
                }
                glib::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if is_wayland_session() {
        // SAFETY: startup is still single-threaded, before GTK, icon loading or workers.
        // Prefer native Wayland even when a launching terminal forced GDK_BACKEND=x11.
        unsafe {
            std::env::set_var("GDK_BACKEND", "wayland,x11");
        }
    }
    let language = preferences::Preferences::load()
        .language
        .code()
        .replace('-', "_");
    // SAFETY: no GTK or capture workers have started; toolkit dialogs read locale at startup.
    unsafe {
        std::env::set_var("LANGUAGE", language);
    }
    glib::set_prgname(Some("snappy"));
    glib::set_application_name("Snappy");
    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE);
    let controller = std::rc::Rc::new(std::cell::RefCell::new(None::<std::rc::Rc<ui::Controller>>));
    app.connect_startup(|_| {
        if let Err(error) = desktop::ensure_identity() {
            eprintln!("Desktop integration: {error}");
        }
    });
    app.connect_activate(move |app| {
        let existing = controller.borrow().clone();
        let state = existing.unwrap_or_else(|| {
            let state = ui::build(app);
            controller.replace(Some(state.clone()));
            state
        });
        state.activate();
    });
    app.connect_command_line(|app, command| {
        let valid = command
            .arguments()
            .iter()
            .skip(1)
            .all(|arg| arg == "--capture");
        if !valid {
            eprintln!("Unknown option. Use --help for usage.");
            return 2;
        }
        app.activate();
        0
    });
    app.run()
}

#[cfg(test)]
mod session_tests {
    use super::session_uses_wayland;

    #[test]
    fn compositor_session_takes_priority_over_compatibility_displays() {
        assert!(session_uses_wayland(Some("wayland"), Some("wayland-0")));
        assert!(session_uses_wayland(Some("wayland"), None));
        assert!(!session_uses_wayland(Some("x11"), Some("wayland-0")));
        assert!(session_uses_wayland(None, Some("wayland-0")));
        assert!(!session_uses_wayland(None, None));
    }
}
