//! Module: ui. Purpose: desktop capture workflow, compact recording controls and export popups.
//! Created: 2026-10-19. Architecture: all GTK widgets stay on the main thread.
use crate::i18n::{self, tr};
use crate::{
    capture::{self, Area, Prepared, Recording},
    files::{self, Result},
    preferences::Preferences,
    preset::{self, Preset},
    selection, shortcut,
};
use gtk::{gdk_pixbuf::Pixbuf, prelude::*};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

/// Styles for export popups; the overlay toolbar keeps its own OSD rule in selection.
const DIALOG_CSS: &str = "
.snappy-backdrop { background-color: alpha(@theme_fg_color, 0.07); border-radius: 10px; padding: 14px; }
.snappy-preview { border-radius: 6px; box-shadow: 0 2px 10px alpha(black, 0.45); }
.snappy-badge { background-color: alpha(black, 0.62); color: white; border-radius: 99px; padding: 2px 10px; margin: 10px; font-size: smaller; }
";

enum Job {
    Prepared(Result<Prepared>),
    RecordingPrepared {
        result: Result<Prepared>,
        area: Area,
        bounds: Area,
        single_monitor: bool,
    },
    Started(Result<Recording>),
    Finished(Result<PathBuf>),
}

pub struct Controller {
    owner: gtk::ApplicationWindow,
    recorder: gtk::ApplicationWindow,
    elapsed: gtk::Label,
    preferences: RefCell<Preferences>,
    preset: Cell<Preset>,
    sound: Cell<bool>,
    video: Cell<bool>,
    busy: Cell<bool>,
    recording: RefCell<Option<Recording>>,
    saving: RefCell<Option<gtk::Dialog>>,
    overlay: glib::WeakRef<gtk::Dialog>,
    settings_dialog: glib::WeakRef<gtk::Dialog>,
    recapturing: Cell<bool>,
    pending_capture: RefCell<Option<Prepared>>,
    quit_after: Cell<bool>,
    wayland: bool,
    jobs: glib::Sender<Job>,
    dimensions: Cell<(i32, i32)>,
    frame_rate: Cell<u32>,
    shortcut_status: RefCell<String>,
}

/// Highest refresh rate among the connected monitors, in frames per second. GDK reports
/// millihertz and zero when a backend does not know; 60 is the fallback.
fn display_frame_rate() -> u32 {
    let best = gtk::gdk::Display::default()
        .map(|display| {
            (0..display.n_monitors())
                .filter_map(|index| display.monitor(index))
                .map(|monitor| monitor.refresh_rate())
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    let rate = (f64::from(best) / 1000.0).round() as u32;
    if rate == 0 { 60 } else { rate.clamp(24, 240) }
}

#[allow(deprecated)] // GTK 3 GLib channels wake the main loop as soon as work finishes.
pub fn build(app: &gtk::Application) -> Rc<Controller> {
    let preferences = Preferences::load();
    i18n::set_language(preferences.language);
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_theme_name(Some("Yaru"));
    }
    let styles = gtk::CssProvider::new();
    if let Err(error) = styles.load_from_data(DIALOG_CSS.as_bytes()) {
        eprintln!("Snappy: dialog stylesheet rejected: {error}");
    } else if let Some(screen) = gtk::gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &styles,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    gtk::gdk::set_program_class(crate::APP_ID);
    gtk::Window::set_default_icon_name(crate::APP_ID);
    let wayland = crate::is_wayland_session()
        || gtk::gdk::Display::default()
            .map(|display| display.type_().name().contains("Wayland"))
            .unwrap_or(false);
    eprintln!(
        "Snappy: capture={}, display={}",
        if wayland { "wayland" } else { "x11" },
        gtk::gdk::Display::default()
            .map(|display| display.type_().name().to_owned())
            .unwrap_or_else(|| "unavailable".into())
    );
    // This hidden application window owns dialogs and keeps the background shortcut listener alive.
    let owner = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Snappy")
        .default_width(1)
        .default_height(1)
        .build();
    owner.set_position(gtk::WindowPosition::Center);
    owner.set_icon_name(Some(crate::APP_ID));
    let recorder = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Snappy recording")
        .decorated(false)
        .resizable(false)
        .build();
    recorder.style_context().add_class("osd");
    recorder.set_keep_above(true);
    recorder.set_position(gtk::WindowPosition::None);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    bar.set_border_width(16);
    bar.style_context().add_class("osd");
    let elapsed = gtk::Label::new(Some("● 00:00"));
    let stop = gtk::Button::with_label(tr("Stop"));
    stop.style_context().add_class("destructive-action");
    bar.pack_start(&elapsed, false, false, 0);
    bar.pack_start(&stop, false, false, 0);
    recorder.add(&bar);
    // The bar is part of the screen, so it appears in a recording that covers its
    // position. Pressing anywhere on the bar itself drags it; Stop handles its own
    // presses. begin_move_drag works on X11 and through the compositor on Wayland.
    recorder.add_events(gtk::gdk::EventMask::BUTTON_PRESS_MASK);
    recorder.set_tooltip_text(Some(tr("Drag to move")));
    recorder.connect_button_press_event(|window, event| {
        if event.button() != 1 {
            return glib::Propagation::Proceed;
        }
        let (x, y) = event.root();
        window.begin_move_drag(1, x as i32, y as i32, event.time());
        glib::Propagation::Stop
    });
    if !wayland {
        // Place the bar near the bottom once; later layout passes must not undo a drag.
        let placed = Cell::new(false);
        recorder.connect_size_allocate(move |window, allocation| {
            if placed.replace(true) {
                return;
            }
            if let Some(screen) = gtk::prelude::GtkWindowExt::screen(window) {
                if let Some(root) = screen.root_window() {
                    window.move_(
                        (root.width() - allocation.width()) / 2,
                        root.height() - allocation.height() - 24,
                    );
                }
            }
        });
    }

    recorder.connect_delete_event(|window, _| {
        window.hide();
        glib::Propagation::Stop
    });
    let (jobs, receiver) = glib::MainContext::channel(glib::Priority::DEFAULT);
    let state = Rc::new(Controller {
        owner,
        recorder,
        elapsed,
        preferences: RefCell::new(preferences),
        preset: Cell::new(Preset::Discord),
        sound: Cell::new(true),
        video: Cell::new(false),
        busy: Cell::new(false),
        recording: RefCell::new(None),
        saving: RefCell::new(None),
        overlay: glib::WeakRef::new(),
        settings_dialog: glib::WeakRef::new(),
        recapturing: Cell::new(false),
        pending_capture: RefCell::new(None),
        quit_after: Cell::new(false),
        wayland,
        jobs,
        dimensions: Cell::new((1920, 1080)),
        frame_rate: Cell::new(60),
        shortcut_status: RefCell::new("Registering Print Screen…".into()),
    });
    let copy = state.clone();
    stop.connect_clicked(move |_| copy.finish(false));
    let (sender, shortcuts) = glib::MainContext::channel(glib::Priority::DEFAULT);
    shortcut::start(wayland, sender);
    let copy = state.clone();
    shortcuts.attach(None, move |event| {
        match event {
            shortcut::Event::Open => copy.activate(),
            shortcut::Event::Status(text) => {
                copy.shortcut_status.replace(text);
            }
        }
        glib::ControlFlow::Continue
    });
    let copy = state.clone();
    receiver.attach(None, move |job| {
        copy.receive(job);
        glib::ControlFlow::Continue
    });
    let copy = state.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let ended = copy
            .recording
            .borrow_mut()
            .as_mut()
            .map(|r| r.exited().unwrap_or(true))
            .unwrap_or(false);
        if ended {
            copy.finish(false);
        }
        if let Some(recording) = copy.recording.borrow().as_ref() {
            let seconds = recording.elapsed();
            copy.elapsed.set_text(&format!(
                "● {:02}:{:02}",
                seconds as u64 / 60,
                seconds as u64 % 60
            ));
            let (width, height) = copy.dimensions.get();
            copy.recorder.set_tooltip_text(Some(&format!(
                "{}\n{}",
                copy.preset.get().estimate(
                    seconds,
                    copy.sound.get(),
                    width,
                    height,
                    copy.frame_rate.get()
                ),
                tr("Drag to move")
            )));
        }
        glib::ControlFlow::Continue
    });
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let copy = state.clone();
        glib::source::unix_signal_add_local(signal, move || {
            copy.finish(true);
            glib::ControlFlow::Continue
        });
    }
    state
}

impl Controller {
    pub fn activate(self: &Rc<Self>) {
        if let Some(overlay) = self.overlay.upgrade().filter(|dialog| dialog.is_visible()) {
            let settings_open = self
                .settings_dialog
                .upgrade()
                .is_some_and(|dialog| dialog.is_visible());
            if settings_open && !self.recapturing.replace(true) {
                // Print Screen while Settings is open captures the desktop as it is now,
                // Settings included; the result replaces the selector once it arrives.
                eprintln!("Snappy activation: capturing the open settings");
                let sender = self.jobs.clone();
                let wayland = self.wayland;
                std::thread::spawn(move || {
                    let _ = sender.send(Job::Prepared(capture::prepare(wayland, false)));
                });
                return;
            }
            eprintln!("Snappy activation: presenting selection");
            overlay.present();
            return;
        }
        if self.recording.borrow().is_some() {
            eprintln!("Snappy activation: presenting recording controls");
            self.recorder.show_all();
            self.recorder.present();
            return;
        }
        if self.busy.replace(true) {
            eprintln!("Snappy activation: capture preparation or export already running");
            if let Some(saving) = self.saving.borrow().as_ref() {
                saving.present();
            }
            return;
        }
        eprintln!("Snappy activation: preparing a new capture");
        // Pending exports are part of the visible desktop and may be the subject
        // of the next capture. Only the selector itself covers them temporarily.
        let sender = self.jobs.clone();
        let wayland = self.wayland;
        std::thread::spawn(move || {
            // Showing the controls needs a still image, not a screen-sharing session.
            let _ = sender.send(Job::Prepared(capture::prepare(wayland, false)));
        });
    }

    fn receive(self: &Rc<Self>, job: Job) {
        // The selector runs a nested main loop, and GLib does not dispatch a source
        // again while its callback is still running. Results that may open the
        // selector are handed to a one-shot idle callback so this worker event source
        // returns at once and stays free to deliver a fresh capture taken while the
        // selector or Settings is open.
        match job {
            Job::Prepared(result) => {
                let state = self.clone();
                let recapture = self.recapturing.replace(false);
                glib::idle_add_local_once(move || match result {
                    Ok(prepared) if recapture => state.replace_selector(prepared),
                    Ok(prepared) => state.prepared(prepared),
                    Err(error) => state.error(error),
                });
            }
            Job::RecordingPrepared {
                result,
                area,
                bounds,
                single_monitor,
            } => {
                let state = self.clone();
                glib::idle_add_local_once(move || match result {
                    Ok(prepared) => {
                        let same_pixels = Pixbuf::from_file(&prepared.image).is_ok_and(|image| {
                            image.width() == bounds.width && image.height() == bounds.height
                        });
                        if single_monitor && same_pixels {
                            state.start_recording(prepared, area, bounds);
                        } else {
                            // Different monitor layouts require selection on the granted stream.
                            state.prepared(prepared);
                        }
                    }
                    Err(error) => state.error(error),
                });
            }
            Job::Started(result) => match result {
                Ok(recording) => {
                    self.recording.replace(Some(recording));
                    self.recorder.show_all();
                    self.recorder.present();
                }
                Err(error) => self.error(error),
            },
            Job::Finished(result) => {
                if let Some(dialog) = self.saving.take() {
                    dismiss_dialog(&dialog);
                }
                self.busy.set(false);
                match result {
                    Ok(path) => {
                        let bytes = path.metadata().map(|m| m.len()).unwrap_or(0);
                        self.saved(path, bytes);
                    }
                    Err(error) => {
                        self.quit_after.set(false);
                        self.error(error);
                    }
                }
            }
        }
    }

    /// Closes Settings, keeping its changes, and the current selector; the selector
    /// reopens on this fresh capture as soon as its nested loops have unwound.
    fn replace_selector(self: &Rc<Self>, prepared: Prepared) {
        eprintln!("Snappy activation: fresh capture ready, closing settings and selector");
        self.pending_capture.replace(Some(prepared));
        if let Some(settings) = self
            .settings_dialog
            .upgrade()
            .filter(|dialog| dialog.is_visible())
        {
            settings.response(gtk::ResponseType::Ok);
        }
        match self.overlay.upgrade().filter(|dialog| dialog.is_visible()) {
            Some(overlay) => overlay.response(gtk::ResponseType::Cancel),
            None => {
                if let Some(next) = self.pending_capture.take() {
                    self.prepared(next);
                }
            }
        }
    }

    fn prepared(self: &Rc<Self>, prepared: Prepared) {
        let image = match Pixbuf::from_file(&prepared.image) {
            Ok(image) => image,
            Err(error) => {
                self.error(error.to_string());
                return;
            }
        };
        let bounds = Area {
            x: 0,
            y: 0,
            width: image.width(),
            height: image.height(),
        };
        self.dimensions.set((bounds.width, bounds.height));
        let position = match &prepared.source {
            capture::Source::Wayland(stream) => stream.position,
            _ => None,
        };
        let state = self.clone();
        let choice = selection::choose(
            &self.owner,
            &image,
            self.video.get(),
            self.sound.get(),
            move |parent, sound| {
                state.sound.set(sound);
                state.settings(parent);
                state.sound.get()
            },
            {
                let state = self.clone();
                move |language| {
                    state.preferences.borrow_mut().language = language;
                    if let Err(error) = state.preferences.borrow().save() {
                        eprintln!("Snappy: {error}");
                    }
                }
            },
            self.wayland,
            position,
            {
                let state = self.clone();
                move |dialog| state.overlay.set(Some(dialog))
            },
        );
        self.overlay.set(None);
        let Some(choice) = choice else {
            if let Some(next) = self.pending_capture.take() {
                // The selector was closed only to show a fresh capture.
                eprintln!("Snappy activation: reopening selector on the fresh capture");
                self.prepared(next);
                return;
            }
            self.busy.set(false);
            if self.quit_after.get() {
                if let Some(app) = self.owner.application() {
                    app.quit();
                }
            }
            return;
        };
        self.video.set(choice.video);
        self.sound.set(choice.sound);
        let area = choice.area;
        self.dimensions.set((area.width, area.height));
        if choice.video {
            if matches!(&prepared.source, capture::Source::Screenshot) {
                let sender = self.jobs.clone();
                let single_monitor =
                    gtk::gdk::Display::default().is_some_and(|display| display.n_monitors() == 1);
                std::thread::spawn(move || {
                    let _ = sender.send(Job::RecordingPrepared {
                        result: capture::prepare(true, true),
                        area,
                        bounds,
                        single_monitor,
                    });
                });
            } else {
                self.start_recording(prepared, area, bounds);
            }
        } else {
            let cropped = image.new_subpixbuf(area.x, area.y, area.width, area.height);
            let image_folder = self.preferences.borrow().images.clone();
            let state = self.clone();
            save_screenshot(
                &self.owner,
                &cropped,
                &image_folder,
                move |result| match result {
                    Ok(Some((path, size))) => {
                        if let Err(error) = state.preferences.borrow_mut().remember_image(&path) {
                            state.error(error);
                        }
                        state.saved(path, size as u64);
                    }
                    Ok(None) => (),
                    Err(error) => state.error(error),
                },
            );
            self.busy.set(false);
        }
    }

    fn start_recording(self: &Rc<Self>, prepared: Prepared, area: Area, bounds: Area) {
        let sender = self.jobs.clone();
        let folder = self.preferences.borrow().videos.clone();
        let sound = self.sound.get();
        let preset = self.preset.get();
        let frame_rate = display_frame_rate();
        self.frame_rate.set(frame_rate);
        let native_stream = matches!(
            prepared.source,
            capture::Source::Wayland(_) | capture::Source::Gnome(_)
        );
        let start = move || {
            std::thread::spawn(move || {
                let _ = sender.send(Job::Started(Recording::start(
                    prepared, area, bounds, sound, preset, &folder, frame_rate,
                )));
            });
        };
        if native_stream {
            // Native preparation has already taken a fresh frame after the selector closed.
            start();
        } else {
            // X11 needs the compositor to finish unmapping the selection surface first.
            glib::timeout_add_local_once(Duration::from_millis(350), start);
        }
    }

    fn settings(&self, parent: &gtk::Dialog) {
        let dialog = gtk::Dialog::with_buttons(
            Some(tr("Snappy settings")),
            Some(parent),
            gtk::DialogFlags::MODAL,
            &[
                (tr("Quit Snappy"), gtk::ResponseType::Reject),
                (tr("Done"), gtk::ResponseType::Ok),
            ],
        );
        self.settings_dialog.set(Some(&dialog));
        center_dialog(&dialog, parent);
        dialog.set_default_response(gtk::ResponseType::Ok);
        if let Some(done) = dialog.widget_for_response(gtk::ResponseType::Ok) {
            done.style_context().add_class("suggested-action");
        }
        // A gap sets Quit apart from Done, as Cancel is set apart in the save dialog.
        if let Some(quit) = dialog.widget_for_response(gtk::ResponseType::Reject) {
            quit.set_margin_end(24);
        }
        dialog.content_area().set_border_width(18);
        dialog.content_area().set_spacing(14);
        let labels = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        let images = folder_setting(
            &dialog,
            &labels,
            tr("Images"),
            tr("Images"),
            &self.preferences.borrow().images,
            Some(&Preferences::default_images()),
        );
        let videos = folder_setting(
            &dialog,
            &labels,
            tr("Videos"),
            tr("Videos"),
            &self.preferences.borrow().videos,
            Some(&Preferences::default_videos()),
        );
        let quality = gtk::ComboBoxText::new();
        for preset in Preset::ALL {
            quality.append_text(tr(preset.label()));
        }
        quality.set_active(
            Preset::ALL
                .iter()
                .position(|p| *p == self.preset.get())
                .map(|i| i as u32),
        );
        settings_row(&dialog, &labels, tr("Video quality"), &quality);
        let sound = gtk::Switch::new();
        sound.set_active(self.sound.get());
        sound.set_halign(gtk::Align::Start);
        settings_row(&dialog, &labels, tr("Desktop sound"), &sound);
        let estimate = gtk::Label::new(None);
        estimate.set_xalign(0.0);
        estimate.set_line_wrap(true);
        estimate.set_max_width_chars(48);
        estimate.set_tooltip_text(Some(
            "Approximate size per minute. Recording continues until Stop is pressed.",
        ));
        let (width, height) = self.dimensions.get();
        let frame_rate = display_frame_rate();
        let update = Rc::new({
            let quality = quality.clone();
            let sound = sound.clone();
            let estimate = estimate.clone();
            move || {
                // The row title already says Estimated size; the shared text repeats it.
                estimate.set_text(
                    &Preset::ALL[quality.active().unwrap_or(0) as usize]
                        .estimate(60.0, sound.is_active(), width, height, frame_rate)
                        .replace(&format!("{}: ", tr("Estimated size")), "")
                        .replace("for 60s", tr("per minute")),
                );
            }
        });
        update();
        let copy = update.clone();
        let quality_handler = quality.connect_changed(move |_| copy());
        let copy = update.clone();
        let sound_handler = sound.connect_active_notify(move |_| copy());
        settings_row(&dialog, &labels, tr("Estimated size"), &estimate);
        let shortcut = gtk::Label::new(Some(&self.shortcut_status.borrow()));
        shortcut.set_selectable(true);
        shortcut.set_line_wrap(true);
        shortcut.set_max_width_chars(48);
        shortcut.set_xalign(0.0);
        settings_row(&dialog, &labels, tr("Shortcut"), &shortcut);
        dialog.show_all();
        let response = dialog.run();
        let mut folders = self.preferences.borrow_mut();
        if let Some(path) = images.0.filename() {
            folders.images = path;
        }
        if let Some(path) = videos.0.filename() {
            folders.videos = path;
        }
        let saved = folders.save();
        drop(folders);
        self.preset
            .set(Preset::ALL[quality.active().unwrap_or(0) as usize]);
        self.sound.set(sound.is_active());
        quality.disconnect(quality_handler);
        sound.disconnect(sound_handler);
        self.settings_dialog.set(None);
        dismiss_dialog(&dialog);
        dismiss_dialog(&images.1);
        dismiss_dialog(&videos.1);
        if let Err(error) = saved {
            self.error(error);
        }
        if response == gtk::ResponseType::Reject {
            self.quit_after.set(true);
            parent.response(gtk::ResponseType::Cancel);
        }
    }

    fn saved(self: &Rc<Self>, path: PathBuf, bytes: u64) {
        let state = self.clone();
        // Confirmation must not hold the worker-event source inside its modal loop.
        glib::idle_add_local_once(move || {
            show_saved(&state.owner, &path, bytes);
            if state.quit_after.get() {
                if let Some(app) = state.owner.application() {
                    app.quit();
                }
            }
        });
    }

    fn error(&self, error: String) {
        self.busy.set(false);
        self.recorder.hide();
        let dialog = gtk::MessageDialog::new(
            Some(&self.owner),
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Error,
            gtk::ButtonsType::Close,
            &error,
        );
        dialog.set_title("Snappy");
        center_dialog(&dialog, &self.owner);
        dialog.connect_response(|dialog, _| {
            dialog.hide();
            let dialog = dialog.clone();
            glib::idle_add_local_once(move || dismiss_dialog(&dialog));
        });
        dialog.show_all();
        dialog.present();
    }

    fn finish(self: &Rc<Self>, quit: bool) {
        if let Some(recording) = self.recording.take() {
            self.quit_after.set(quit);
            self.recorder.hide();
            let dialog = gtk::Dialog::with_buttons(
                Some("Snappy"),
                Some(&self.owner),
                gtk::DialogFlags::MODAL,
                &[],
            );
            center_dialog(&dialog, &self.owner);
            dialog.content_area().set_border_width(24);
            dialog.content_area().pack_start(
                &gtk::Label::new(Some(tr("Saving…"))),
                false,
                false,
                0,
            );
            dialog.show_all();
            self.saving.replace(Some(dialog));
            let sender = self.jobs.clone();
            std::thread::spawn(move || {
                let _ = sender.send(Job::Finished(recording.finish()));
            });
        } else if quit {
            if let Some(app) = self.owner.application() {
                app.quit();
            }
        }
    }
}

/// Shared folder chooser used by Settings and image export. The media name titles the
/// chooser; the row label may be shorter in a single-media dialog. A default folder adds
/// a reset button, which Settings offers and the save dialog leaves out.
fn folder_setting(
    dialog: &gtk::Dialog,
    labels: &gtk::SizeGroup,
    title: &str,
    label: &str,
    current: &Path,
    default: Option<&Path>,
) -> (gtk::FileChooserButton, gtk::FileChooserDialog) {
    let chooser = gtk::FileChooserDialog::with_buttons(
        Some(&format!("{title} · {}", tr("Folder"))),
        Some(dialog),
        gtk::FileChooserAction::SelectFolder,
        &[
            (tr("Cancel"), gtk::ResponseType::Cancel),
            (tr("Select folder"), gtk::ResponseType::Accept),
        ],
    );
    center_dialog(&chooser, dialog);
    chooser.set_modal(true);
    let folder = gtk::FileChooserButton::with_dialog(&chooser);
    let _ = std::fs::create_dir_all(current);
    folder.set_filename(current);
    if let Some(accessible) = folder.accessible() {
        accessible.set_name(&format!("{title} · {}", tr("Folder")));
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.pack_start(&folder, true, true, 0);
    if let Some(default) = default {
        let reset = gtk::Button::with_label(tr("Default"));
        reset.set_tooltip_text(Some(&format!("Use {}", default.display())));
        if let Some(accessible) = reset.accessible() {
            accessible.set_name(&format!("Default {title} folder"));
        }
        let copy = folder.clone();
        let default = default.to_owned();
        reset.connect_clicked(move |_| {
            if std::fs::create_dir_all(&default).is_ok() {
                copy.set_filename(&default);
            }
        });
        row.pack_start(&reset, false, false, 0);
    }
    settings_row(dialog, labels, label, &row);
    (folder, chooser)
}

/// One labeled row. Dimmed, left-aligned titles share one column per dialog, sized
/// by the widest title so translations never break the alignment.
fn settings_row(
    dialog: &gtk::Dialog,
    labels: &gtk::SizeGroup,
    title: &str,
    widget: &impl IsA<gtk::Widget>,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_valign(gtk::Align::Start);
    label.set_margin_top(4);
    label.style_context().add_class("dim-label");
    labels.add_widget(&label);
    row.pack_start(&label, false, false, 0);
    row.pack_start(widget, true, true, 0);
    dialog.content_area().pack_start(&row, false, false, 0);
}

fn show_saved(parent: &gtk::ApplicationWindow, path: &Path, bytes: u64) {
    let dialog = gtk::Dialog::with_buttons(
        Some(tr("Capture saved")),
        Some(parent),
        gtk::DialogFlags::MODAL,
        &[(tr("OK"), gtk::ResponseType::Ok)],
    );
    center_dialog(&dialog, parent);
    dialog.set_default_response(gtk::ResponseType::Ok);
    dialog.content_area().set_border_width(20);
    dialog.content_area().set_spacing(12);
    let capture_path = gtk::Label::new(Some(&path.to_string_lossy()));
    capture_path.set_selectable(true);
    capture_path.set_line_wrap(true);
    capture_path.set_max_width_chars(60);
    capture_path.set_xalign(0.0);
    let size = gtk::Label::new(Some(&format!(
        "{}: {}",
        tr("Size"),
        preset::size(bytes as f64)
    )));
    size.set_xalign(0.0);
    dialog
        .content_area()
        .pack_start(&capture_path, false, false, 0);
    dialog.content_area().pack_start(&size, false, false, 0);
    dialog.show_all();
    dialog.run();
    dismiss_dialog(&dialog);
}
/// Logical preview bounds; the card is drawn at the display's device scale for sharpness.
const PREVIEW_WIDTH: f64 = 560.0;
const PREVIEW_HEIGHT: f64 = 320.0;
const PREVIEW_RADIUS: f64 = 6.0;

/// The screenshot as a rounded card with a shadow and a native-size badge, on a tinted backdrop.
fn preview_card(image: &Pixbuf) -> gtk::Box {
    let scale = (PREVIEW_WIDTH / f64::from(image.width()))
        .min(PREVIEW_HEIGHT / f64::from(image.height()))
        .min(1.0);
    let width = (f64::from(image.width()) * scale).round().max(1.0) as i32;
    let height = (f64::from(image.height()) * scale).round().max(1.0) as i32;
    let canvas = gtk::DrawingArea::new();
    canvas.set_size_request(width, height);
    let source = image.clone();
    // Scaled once per device scale factor, not on every expose.
    let scaled: RefCell<Option<(i32, Pixbuf)>> = RefCell::new(None);
    canvas.connect_draw(move |canvas, context| {
        use gtk::gdk::prelude::GdkContextExt;
        let factor = canvas.scale_factor().max(1);
        let mut cache = scaled.borrow_mut();
        if cache.as_ref().map(|(cached, _)| *cached) != Some(factor) {
            *cache = source
                .scale_simple(
                    width * factor,
                    height * factor,
                    gtk::gdk_pixbuf::InterpType::Bilinear,
                )
                .map(|preview| (factor, preview));
        }
        let Some((_, preview)) = cache.as_ref() else {
            return glib::Propagation::Proceed;
        };
        rounded_rectangle(context, f64::from(width), f64::from(height), PREVIEW_RADIUS);
        context.clip();
        context.scale(1.0 / f64::from(factor), 1.0 / f64::from(factor));
        context.set_source_pixbuf(preview, 0.0, 0.0);
        let _ = context.paint();
        glib::Propagation::Stop
    });
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    card.style_context().add_class("snappy-preview");
    card.add(&canvas);
    let badge = gtk::Label::new(Some(&format!("{} × {}", image.width(), image.height())));
    badge.style_context().add_class("snappy-badge");
    badge.set_halign(gtk::Align::End);
    badge.set_valign(gtk::Align::End);
    let overlay = gtk::Overlay::new();
    overlay.add(&card);
    overlay.add_overlay(&badge);
    let backdrop = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    backdrop.style_context().add_class("snappy-backdrop");
    backdrop.pack_start(&overlay, true, false, 0);
    backdrop
}

/// Clockwise rounded rectangle path from the origin, used as a clip for the preview.
fn rounded_rectangle(context: &gtk::cairo::Context, width: f64, height: f64, radius: f64) {
    use std::f64::consts::FRAC_PI_2;
    let radius = radius.min(width / 2.0).min(height / 2.0);
    context.new_sub_path();
    context.arc(width - radius, radius, radius, -FRAC_PI_2, 0.0);
    context.arc(width - radius, height - radius, radius, 0.0, FRAC_PI_2);
    context.arc(radius, height - radius, radius, FRAC_PI_2, 2.0 * FRAC_PI_2);
    context.arc(radius, radius, radius, 2.0 * FRAC_PI_2, 3.0 * FRAC_PI_2);
    context.close_path();
}

#[allow(deprecated)] // Match the GLib version provided by GTK 3.
fn save_screenshot(
    parent: &gtk::ApplicationWindow,
    image: &Pixbuf,
    folder: &std::path::Path,
    completed: impl FnOnce(Result<Option<(PathBuf, usize)>>) + 'static,
) {
    // The file name carries the moment of capture rather than the moment Save is pressed.
    let stamp = match files::stamp() {
        Ok(stamp) => stamp,
        Err(error) => {
            completed(Err(error));
            return;
        }
    };
    // Nonmodal, so Print Screen can start another capture while this image waits.
    let dialog = gtk::Dialog::with_buttons(
        Some(tr("Save screenshot")),
        Some(parent),
        gtk::DialogFlags::empty(),
        &[
            (tr("Cancel"), gtk::ResponseType::Cancel),
            (tr("Copy"), gtk::ResponseType::Apply),
            (tr("Save"), gtk::ResponseType::Accept),
        ],
    );
    if let Some(save) = dialog.widget_for_response(gtk::ResponseType::Accept) {
        save.style_context().add_class("suggested-action");
    }
    // A gap sets Cancel apart from the two actions that use the image.
    if let Some(cancel) = dialog.widget_for_response(gtk::ResponseType::Cancel) {
        cancel.set_margin_end(24);
    }
    center_dialog(&dialog, parent);
    // The preview keeps its fixed size, so a larger window would only add empty space.
    dialog.set_resizable(false);
    dialog.set_default_response(gtk::ResponseType::Accept);
    dialog.content_area().set_border_width(18);
    dialog.content_area().set_spacing(10);
    // The file name heads the dialog and also carries the size: the encoded bytes are exactly what
    // Save writes, so this is the real size, not an estimate.
    let file_name = gtk::Label::new(None);
    file_name.set_xalign(0.0);
    file_name.set_selectable(true);
    file_name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let spinner = gtk::Spinner::new();
    let size = gtk::Label::new(Some(tr("Calculating size preview…")));
    size.set_xalign(1.0);
    size.set_line_wrap(true);
    size.set_max_width_chars(32);
    size.style_context().add_class("dim-label");
    // This heading row starts at the left edge, flush with the preview, rather than
    // in the label column shared by the choices beneath.
    let name_title = gtk::Label::new(Some(tr("File name")));
    name_title.style_context().add_class("dim-label");
    let name_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    // A small inset keeps the heading inside the preview card's rounded edges.
    name_row.set_margin_start(8);
    name_row.set_margin_end(8);
    name_row.pack_start(&name_title, false, false, 0);
    name_row.pack_start(&file_name, true, true, 0);
    name_row.pack_start(&spinner, false, false, 0);
    name_row.pack_start(&size, false, false, 0);
    dialog.content_area().pack_start(&name_row, false, false, 0);
    dialog
        .content_area()
        .pack_start(&preview_card(image), false, false, 0);
    let labels = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
    let profile = gtk::ComboBoxText::new();
    for text in [
        tr("Full quality · PNG · lossless"),
        tr("Balanced · JPEG · quality 90"),
        tr("Small file · JPEG · quality 70"),
        tr("Bitmap · BMP · uncompressed"),
        tr("Still image · GIF · up to 256 colors"),
    ] {
        profile.append_text(text);
    }
    if let Some(accessible) = profile.accessible() {
        accessible.set_name(tr("Format"));
    }
    profile.set_margin_top(8);
    settings_row(&dialog, &labels, tr("Format"), &profile);
    let destination = folder_setting(&dialog, &labels, tr("Images"), tr("Folder"), folder, None);
    let snapshot = crate::image_export::Snapshot::new(image);
    let encoded = Rc::new(RefCell::new(Vec::<u8>::new()));
    let generation = Rc::new(Cell::new(0u64));
    let (sender, receiver) = glib::MainContext::channel(glib::Priority::DEFAULT);
    let latest = generation.clone();
    let pending_size = size.clone();
    let pending_spinner = spinner.clone();
    let copy = dialog.clone();
    let name_stamp = stamp.clone();
    profile.connect_changed(move |profile| {
        let profile = profile.active().unwrap_or(0);
        file_name.set_text(&files::name(
            &name_stamp,
            0,
            crate::image_export::extension(profile),
        ));
        pending_size.set_text(tr("Calculating size preview…"));
        pending_spinner.show();
        pending_spinner.start();
        let revision = latest.get() + 1;
        latest.set(revision);
        copy.set_response_sensitive(gtk::ResponseType::Accept, false);
        let sender = sender.clone();
        let snapshot = snapshot.clone();
        std::thread::spawn(move || {
            let _ = sender.send((revision, crate::image_export::encode(snapshot, profile)));
        });
    });
    let data = encoded.clone();
    let copy = dialog.clone();
    let updates = receiver.attach(None, move |(revision, result)| {
        if revision != generation.get() {
            return glib::ControlFlow::Continue;
        }
        spinner.stop();
        spinner.hide();
        match result {
            Ok(bytes) => {
                size.set_text(&preset::size(bytes.len() as f64));
                data.replace(bytes);
                copy.set_response_sensitive(gtk::ResponseType::Accept, true);
            }
            Err(error) => size.set_text(&error),
        }
        glib::ControlFlow::Continue
    });
    // Selecting the initial format uses the same asynchronous path as later changes.
    profile.set_active(Some(0));
    let completed = RefCell::new(Some(completed));
    let image = image.clone();
    let folder = folder.to_owned();
    let updates = RefCell::new(Some(updates));
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Apply {
            // Copy leaves the dialog open so the same image can still be saved. The
            // clipboard must be set while the dialog is still shown and focused: GNOME
            // Shell on Wayland drops a clipboard offered by a client without keyboard
            // focus, so hiding the dialog first would silently lose the image.
            let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
            clipboard.set_image(&image);
            clipboard.store();
            let button = dialog
                .widget_for_response(gtk::ResponseType::Apply)
                .and_then(|widget| widget.downcast::<gtk::Button>().ok());
            if let Some(button) = button {
                button.set_label(tr("Copied"));
                glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                    button.set_label(tr("Copy"));
                });
            }
            return;
        }
        let Some(completed) = completed.borrow_mut().take() else {
            return;
        };
        let selected_profile = profile.active().unwrap_or(0);
        let destination_path = destination.0.filename().unwrap_or_else(|| folder.clone());
        if let Some(updates) = updates.borrow_mut().take() {
            updates.remove();
        }
        dialog.hide();
        destination.1.hide();
        let result = (|| {
            if response != gtk::ResponseType::Accept {
                return Ok(None);
            }
            let extension = crate::image_export::extension(selected_profile);
            let path = files::reserve_stamped(&destination_path, &stamp, extension)?;
            if let Err(error) = std::fs::write(&path, encoded.borrow().as_slice()) {
                let _ = std::fs::remove_file(&path);
                return Err(format!("Cannot save screenshot: {error}"));
            }
            let size = encoded.borrow().len();
            Ok(Some((path, size)))
        })();
        let dialog = dialog.clone();
        let chooser = destination.1.clone();
        // Destroying a widget also releases its response closure. Defer destruction
        // until this callback has returned and no captured state is still in use.
        glib::idle_add_local_once(move || {
            dismiss_dialog(&dialog);
            dismiss_dialog(&chooser);
            completed(result);
        });
    });
    dialog.show_all();
}

/// Releases a completed dialog after callers have collected all widget state.
pub(crate) fn dismiss_dialog(dialog: &impl IsA<gtk::Dialog>) {
    let dialog = dialog.as_ref();
    dialog.hide();
    // SAFETY: called on GTK's main thread after collecting widget state. Pending
    // preview callbacks are disabled before destruction.
    // Neither the dialog nor any of its children is queried afterwards.
    unsafe {
        dialog.destroy();
    }
}

/// Visible parents define modal placement; a hidden background owner falls back to the screen.
fn center_dialog(dialog: &impl IsA<gtk::Window>, parent: &impl IsA<gtk::Window>) {
    // Transience does not register a dialog with GtkApplication. Registration also
    // provides Wayland application identity and lets GTK route activation correctly.
    dialog
        .as_ref()
        .set_application(parent.as_ref().application().as_ref());
    dialog
        .as_ref()
        .set_position(if parent.as_ref().is_mapped() {
            gtk::WindowPosition::CenterOnParent
        } else {
            gtk::WindowPosition::CenterAlways
        });
}
