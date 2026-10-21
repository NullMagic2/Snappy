//! Module: selection. Purpose: direct desktop region selection with an on-screen toolbar.
//! Created: 2026-10-19. Architecture: one overlay serves screenshots and recordings.
use crate::capture::Area;
use crate::i18n::{self, Language, tr};
use gtk::{gdk, gdk_pixbuf::Pixbuf, prelude::*};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy)]
enum Drag {
    New((f64, f64)),
    Move {
        anchor: (f64, f64),
        original: Area,
    },
    Resize {
        original: Area,
        left: bool,
        right: bool,
        top: bool,
        bottom: bool,
    },
}

pub struct Choice {
    pub area: Area,
    pub video: bool,
    pub sound: bool,
    pub show_pointer: bool,
    pub selection: Option<Area>,
    pub select_region: bool,
}

/// Places a draggable, resizable selection and capture controls directly over the desktop.
/// Coordinates returned to the backend remain in native capture pixels.
pub fn choose(
    parent: &gtk::ApplicationWindow,
    image: &Pixbuf,
    video: bool,
    sound: bool,
    show_pointer: bool,
    selection: Option<Area>,
    select_region: bool,
    settings: impl Fn(&gtk::Dialog, bool) -> bool + 'static,
    language_changed: impl Fn(Language) + 'static,
    wayland: bool,
    position: Option<(i32, i32)>,
    on_open: impl FnOnce(&gtk::Dialog),
) -> Option<Choice> {
    let window = gtk::Dialog::with_buttons(
        Some("Snappy capture overlay"),
        Some(parent),
        gtk::DialogFlags::MODAL,
        &[],
    );
    window.set_application(parent.application().as_ref());
    window.set_decorated(false);
    // Wayland must be free to apply the compositor's fullscreen configure size.
    window.set_resizable(true);
    window.set_type_hint(gdk::WindowTypeHint::Normal);
    window.set_skip_taskbar_hint(false);
    window.set_skip_pager_hint(true);
    window.set_keep_above(true);
    window.set_position(gtk::WindowPosition::None);
    window.set_border_width(0);
    window.content_area().set_border_width(0);
    window.content_area().set_spacing(0);
    let overlay = gtk::Overlay::new();
    let canvas = gtk::DrawingArea::new();
    canvas.set_hexpand(true);
    canvas.set_vexpand(true);
    canvas.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK,
    );
    overlay.add(&canvas);
    window.content_area().pack_start(&overlay, true, true, 0);

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bar.style_context().add_class("osd");
    bar.style_context().add_class("snappy-toolbar");
    let provider = gtk::CssProvider::new();
    let _ = provider.load_from_data(b".snappy-toolbar { padding: 12px; border-radius: 16px; background-color: rgba(35,35,35,0.97); color: white; }");
    bar.style_context()
        .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    // An input-only event box lets the bar be dragged by its padding and gaps while
    // the controls inside keep their own clicks.
    let toolbar = gtk::EventBox::new();
    toolbar.set_visible_window(false);
    toolbar.add(&bar);
    toolbar.set_halign(gtk::Align::Center);
    toolbar.set_valign(gtk::Align::End);
    toolbar.set_margin_bottom(32);
    toolbar.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK,
    );
    // A position chosen by dragging replaces the automatic bottom/top placement.
    let manual_position: Rc<Cell<Option<(i32, i32)>>> = Rc::new(Cell::new(None));
    let toolbar_drag: Rc<Cell<Option<(f64, f64, i32, i32)>>> = Rc::new(Cell::new(None));
    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    modes.style_context().add_class("linked");
    let photo = gtk::RadioButton::with_label(tr("Screenshot"));
    photo.set_mode(false);
    let movie = gtk::RadioButton::with_label_from_widget(&photo, tr("Video"));
    movie.set_mode(false);
    movie.set_active(video);
    modes.pack_start(&photo, false, false, 0);
    modes.pack_start(&movie, false, false, 0);
    let targets = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    targets.style_context().add_class("linked");
    let region = gtk::RadioButton::with_label(tr("Selection"));
    region.set_mode(false);
    let screen = gtk::RadioButton::with_label_from_widget(&region, tr("Screen"));
    screen.set_mode(false);
    screen.set_active(!select_region || selection.is_none());
    region.set_active(select_region && selection.is_some());
    targets.pack_start(&region, false, false, 0);
    targets.pack_start(&screen, false, false, 0);
    // Shows only a live selection size; the mode buttons already say the rest.
    let dimensions = gtk::Label::new(None);
    dimensions.set_no_show_all(true);
    let audio = gtk::ToggleButton::new();
    if let Some(accessible) = audio.accessible() {
        accessible.set_name(tr("Desktop sound"));
    }
    audio.set_active(sound);
    update_audio_button(&audio);
    audio.connect_toggled(update_audio_button);
    let pointer = gtk::ToggleButton::new();
    pointer.set_active(show_pointer);
    update_pointer_button(&pointer);
    pointer.connect_toggled(update_pointer_button);
    pointer.connect_scale_factor_notify(update_pointer_button);
    let gear = gtk::Button::from_icon_name(Some("emblem-system-symbolic"), gtk::IconSize::Button);
    gear.set_tooltip_text(Some(tr("Settings")));
    if let Some(accessible) = gear.accessible() {
        accessible.set_name(tr("Settings"));
    }
    let cancel = gtk::Button::from_icon_name(Some("window-close-symbolic"), gtk::IconSize::Button);
    cancel.set_tooltip_text(Some(tr("Cancel")));
    if let Some(accessible) = cancel.accessible() {
        accessible.set_name(tr("Cancel"));
    }
    let languages = gtk::ComboBoxText::new();
    for language in Language::ALL {
        languages.append(Some(language.code()), language.name());
    }
    languages.set_active_id(Some(i18n::language().code()));
    languages.set_tooltip_text(Some(tr("Language")));
    if let Some(accessible) = languages.accessible() {
        accessible.set_name(tr("Language"));
    }
    let accept = gtk::Button::with_label(if video { tr("Record") } else { tr("Capture") });
    accept.style_context().add_class("suggested-action");
    update_controls(&dimensions, &accept, selection, region.is_active());
    for widget in [
        cancel.upcast_ref::<gtk::Widget>(),
        modes.upcast_ref(),
        targets.upcast_ref(),
        dimensions.upcast_ref(),
        audio.upcast_ref(),
        pointer.upcast_ref(),
        gear.upcast_ref(),
        languages.upcast_ref(),
        accept.upcast_ref(),
    ] {
        bar.pack_start(widget, false, false, 0);
    }
    overlay.add_overlay(&toolbar);
    let drag_state = toolbar_drag.clone();
    let drag_parent = overlay.clone();
    toolbar.connect_button_press_event(move |toolbar, event| {
        if event.button() != 1 {
            return glib::Propagation::Proceed;
        }
        let (x, y) = toolbar
            .translate_coordinates(&drag_parent, 0, 0)
            .unwrap_or((0, 0));
        let (root_x, root_y) = event.root();
        drag_state.set(Some((root_x, root_y, x, y)));
        glib::Propagation::Stop
    });
    let drag_state = toolbar_drag.clone();
    let drag_parent = overlay.clone();
    let dragged_position = manual_position.clone();
    toolbar.connect_motion_notify_event(move |toolbar, event| {
        let Some((start_root_x, start_root_y, start_x, start_y)) = drag_state.get() else {
            return glib::Propagation::Proceed;
        };
        let (root_x, root_y) = event.root();
        let position = (
            (start_x + (root_x - start_root_x).round() as i32).clamp(
                0,
                (drag_parent.allocated_width() - toolbar.allocated_width()).max(0),
            ),
            (start_y + (root_y - start_root_y).round() as i32).clamp(
                0,
                (drag_parent.allocated_height() - toolbar.allocated_height()).max(0),
            ),
        );
        dragged_position.set(Some(position));
        place_toolbar_at(toolbar, position);
        glib::Propagation::Stop
    });
    let drag_state = toolbar_drag.clone();
    toolbar.connect_button_release_event(move |_, event| {
        if event.button() == 1 {
            drag_state.set(None);
        }
        glib::Propagation::Stop
    });
    let capture_button = accept.clone();
    movie.connect_toggled(move |movie| {
        capture_button.set_label(if movie.is_active() {
            tr("Record")
        } else {
            tr("Capture")
        });
    });
    let copy = window.clone();
    let audio_button = audio.clone();
    let settings = Rc::new(settings);
    gear.connect_clicked(move |_| {
        let copy = copy.clone();
        let audio_button = audio_button.clone();
        let settings = settings.clone();
        glib::idle_add_local_once(move || {
            audio_button.set_active(settings(&copy, audio_button.is_active()));
        });
    });

    let selected = Rc::new(Cell::new(selection));
    {
        let photo = photo.clone();
        let movie = movie.clone();
        let region = region.clone();
        let screen = screen.clone();
        let audio = audio.clone();
        let pointer = pointer.clone();
        let gear = gear.clone();
        let cancel = cancel.clone();
        let dimensions = dimensions.clone();
        let accept = accept.clone();
        let selected = selected.clone();
        languages.connect_changed(move |combo| {
            let Some(code) = combo.active_id() else {
                return;
            };
            let language = Language::from_code(&code);
            i18n::set_language(language);
            language_changed(language);
            combo.set_tooltip_text(Some(tr("Language")));
            if let Some(accessible) = combo.accessible() {
                accessible.set_name(tr("Language"));
            }
            if let Some(accessible) = gear.accessible() {
                accessible.set_name(tr("Settings"));
            }
            if let Some(accessible) = cancel.accessible() {
                accessible.set_name(tr("Cancel"));
            }
            photo.set_label(tr("Screenshot"));
            movie.set_label(tr("Video"));
            region.set_label(tr("Selection"));
            screen.set_label(tr("Screen"));
            gear.set_tooltip_text(Some(tr("Settings")));
            cancel.set_tooltip_text(Some(tr("Cancel")));
            accept.set_label(if movie.is_active() {
                tr("Record")
            } else {
                tr("Capture")
            });
            update_audio_button(&audio);
            update_pointer_button(&pointer);
            update_controls(&dimensions, &accept, selected.get(), region.is_active());
        });
    }

    let drag = Rc::new(Cell::new(None::<Drag>));
    let image = image.clone();
    let width = image.width();
    let height = image.height();
    let last_allocation = Cell::new((0, 0));
    canvas.connect_size_allocate(move |canvas, allocation| {
        let size = (allocation.width(), allocation.height());
        if last_allocation.replace(size) != size {
            eprintln!(
                "Snappy overlay: capture={}x{}, viewport={}x{} logical, scale={}",
                width,
                height,
                size.0,
                size.1,
                canvas.scale_factor()
            );
        }
    });
    // Convert the immutable screenshot once, rather than rebuilding its Cairo source
    // on every pointer-motion redraw. Capture pixels retain native coordinates;
    // transform maps them to GTK logical coordinates at the current allocation.
    let background = {
        use gdk::prelude::GdkPixbufExt;
        image.create_surface(1, None::<&gdk::Window>)
    };
    let drawing_selection = selected.clone();
    let drawing_mode = region.clone();
    canvas.connect_draw(move |canvas, context| {
        use gdk::prelude::GdkContextExt;
        let (scale, left, top) = transform(canvas, width, height);
        context.set_source_rgb(0.0, 0.0, 0.0);
        let _ = context.paint();
        context.translate(left, top);
        context.scale(scale, scale);
        if let Some(background) = &background {
            let _ = context.set_source_surface(background, 0.0, 0.0);
        } else {
            context.set_source_pixbuf(&image, 0.0, 0.0);
        }
        let _ = context.paint();
        context.set_source_rgba(0.0, 0.0, 0.0, 0.48);
        context.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
        if let Some(area) = drawing_selection.get().filter(|_| drawing_mode.is_active()) {
            context.rectangle(
                f64::from(area.x),
                f64::from(area.y),
                f64::from(area.width),
                f64::from(area.height),
            );
            context.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
            let _ = context.fill();
            context.set_fill_rule(gtk::cairo::FillRule::Winding);
            context.set_source_rgb(0.91, 0.33, 0.13);
            context.set_line_width(2.0 / scale);
            context.rectangle(
                f64::from(area.x),
                f64::from(area.y),
                f64::from(area.width),
                f64::from(area.height),
            );
            let _ = context.stroke();
            let handle = 8.0 / scale;
            for (x, y) in handles(area) {
                context.rectangle(x - handle / 2.0, y - handle / 2.0, handle, handle);
                let _ = context.fill();
            }
        } else {
            let _ = context.fill();
        }
        glib::Propagation::Proceed
    });
    let selection = selected.clone();
    let pressing = drag.clone();
    let drag_mode = region.clone();
    let drag_toolbar = toolbar.clone();
    canvas.connect_button_press_event(move |canvas, event| {
        if event.button() == 1 {
            drag_mode.set_active(true);
            let point = native_point(canvas, event.position(), width, height);
            let tolerance = 10.0 / transform(canvas, width, height).0;
            pressing.set(Some(start_drag(selection.get(), point, tolerance)));
            // Keep the entire desktop available while drawing, moving or resizing.
            drag_toolbar.hide();
        }
        glib::Propagation::Stop
    });
    let selection = selected.clone();
    let moving = drag.clone();
    let size_label = dimensions.clone();
    let capture_button = accept.clone();
    let area_mode = region.clone();
    canvas.connect_motion_notify_event(move |canvas, event| {
        if let Some(drag) = moving.get() {
            let area = update_drag(
                drag,
                native_point(canvas, event.position(), width, height),
                width,
                height,
            );
            let previous = selection.replace(area);
            if previous != area {
                update_controls(&size_label, &capture_button, area, area_mode.is_active());
                redraw_selection(canvas, previous, area, width, height);
            }
        }
        glib::Propagation::Stop
    });
    let selection = selected.clone();
    let releasing = drag.clone();
    let size_label = dimensions.clone();
    let capture_button = accept.clone();
    let area_mode = region.clone();
    let drag_toolbar = toolbar.clone();
    let dragged_position = manual_position.clone();
    canvas.connect_button_release_event(move |canvas, event| {
        if event.button() == 1 {
            if let Some(drag) = releasing.take() {
                let area = update_drag(
                    drag,
                    native_point(canvas, event.position(), width, height),
                    width,
                    height,
                );
                let previous = selection.replace(area);
                update_controls(&size_label, &capture_button, area, area_mode.is_active());
                place_toolbar(
                    canvas,
                    &drag_toolbar,
                    area,
                    width,
                    height,
                    dragged_position.get(),
                );
                drag_toolbar.show();
                redraw_selection(canvas, previous, area, width, height);
            }
        }
        glib::Propagation::Stop
    });
    let selected_area = selected.clone();
    let size_label = dimensions.clone();
    let capture_button = accept.clone();
    let redraw = canvas.clone();
    region.connect_toggled(move |region| {
        update_controls(
            &size_label,
            &capture_button,
            selected_area.get(),
            region.is_active(),
        );
        redraw.queue_draw();
    });
    let copy = window.clone();
    cancel.connect_clicked(move |_| copy.response(gtk::ResponseType::Cancel));
    let copy = window.clone();
    accept.connect_clicked(move |_| copy.response(gtk::ResponseType::Accept));
    let keyboard_selection = selected.clone();
    let keyboard_mode = region.clone();
    window.connect_key_press_event(move |window, event| {
        match event.keyval() {
            gdk::keys::constants::Escape => window.response(gtk::ResponseType::Cancel),
            gdk::keys::constants::Return | gdk::keys::constants::KP_Enter
                if !keyboard_mode.is_active() || keyboard_selection.get().is_some() =>
            {
                window.response(gtk::ResponseType::Accept)
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    canvas.connect_realize(set_selection_cursor);
    canvas.connect_scale_factor_notify(set_selection_cursor);
    if let Some(screen) = gtk::prelude::GtkWindowExt::screen(parent) {
        if wayland {
            let index = monitor_for(parent, position, width, height);
            if let Some(monitor) = screen.display().monitor(index) {
                let geometry = monitor.geometry();
                // GDK monitor geometry is already logical; do not divide it by scale again.
                window.set_default_size(geometry.width(), geometry.height());
            }
            window.fullscreen_on_monitor(&screen, index);
        } else {
            if let Some(root) = screen.root_window() {
                window.set_default_size(root.width(), root.height());
            }
            window.move_(0, 0);
            window.connect_realize(|window| {
                if let Some(surface) = window.window() {
                    surface.set_fullscreen_mode(gdk::FullscreenMode::AllMonitors);
                }
            });
            window.fullscreen();
        }
    }
    on_open(&window);
    window.show_all();
    window.present();
    let response = window.run();
    let result = if response == gtk::ResponseType::Accept {
        let area = if region.is_active() {
            selected.get()
        } else {
            Some(Area {
                x: 0,
                y: 0,
                width,
                height,
            })
        };
        area.map(|area| Choice {
            area,
            video: movie.is_active(),
            sound: audio.is_active(),
            show_pointer: pointer.is_active(),
            selection: selected.get(),
            select_region: region.is_active(),
        })
    } else {
        None
    };
    crate::ui::dismiss_dialog(&window);
    result
}

/// Only changed selection pixels need repainting; Cairo clips the cached desktop to this region.
fn redraw_selection(
    canvas: &gtk::DrawingArea,
    before: Option<Area>,
    after: Option<Area>,
    width: i32,
    height: i32,
) {
    if before == after {
        return;
    }
    let (scale, left, top) = transform(canvas, width, height);
    let mut damage: Option<(f64, f64, f64, f64)> = None;
    for area in before.into_iter().chain(after) {
        let bounds = (
            left + f64::from(area.x) * scale - 10.0,
            top + f64::from(area.y) * scale - 10.0,
            left + f64::from(area.x + area.width) * scale + 10.0,
            top + f64::from(area.y + area.height) * scale + 10.0,
        );
        damage = Some(match damage {
            None => bounds,
            Some(old) => (
                old.0.min(bounds.0),
                old.1.min(bounds.1),
                old.2.max(bounds.2),
                old.3.max(bounds.3),
            ),
        });
    }
    if let Some((x, y, right, bottom)) = damage {
        let x = x.floor().max(0.0) as i32;
        let y = y.floor().max(0.0) as i32;
        let right = (right.ceil() as i32).min(canvas.allocated_width());
        let bottom = (bottom.ceil() as i32).min(canvas.allocated_height());
        if right > x && bottom > y {
            canvas.queue_draw_area(x, y, right - x, bottom - y);
        }
    }
}

/// A classic arrow pointer at the 16-pixel button icon size, solid white.
fn pointer_arrow(scale: i32) -> Option<gtk::cairo::ImageSurface> {
    let surface =
        gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 16 * scale, 16 * scale)
            .ok()?;
    surface.set_device_scale(f64::from(scale), f64::from(scale));
    let context = gtk::cairo::Context::new(&surface).ok()?;
    context.set_line_join(gtk::cairo::LineJoin::Round);
    context.move_to(3.5, 1.5);
    context.line_to(3.5, 13.0);
    context.line_to(6.5, 10.5);
    context.line_to(8.5, 14.5);
    context.line_to(10.5, 13.5);
    context.line_to(8.5, 9.5);
    context.line_to(12.5, 9.5);
    context.close_path();
    context.set_source_rgb(1.0, 1.0, 1.0);
    context.fill_preserve().ok()?;
    context.set_line_width(1.0);
    context.stroke().ok()?;
    Some(surface)
}

/// Draw a plain outlined plus instead of inheriting decorative crosshair cursor themes.
fn set_selection_cursor(canvas: &gtk::DrawingArea) {
    let Some(window) = canvas.window() else {
        return;
    };
    let scale = canvas.scale_factor().max(1);
    let cursor = (|| {
        let surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 25 * scale, 25 * scale)
                .ok()?;
        surface.set_device_scale(f64::from(scale), f64::from(scale));
        let context = gtk::cairo::Context::new(&surface).ok()?;
        context.set_line_cap(gtk::cairo::LineCap::Square);
        for (width, shade) in [(3.0, 0.0), (1.0, 1.0)] {
            context.set_source_rgb(shade, shade, shade);
            context.set_line_width(width);
            context.move_to(4.5, 12.5);
            context.line_to(20.5, 12.5);
            context.move_to(12.5, 4.5);
            context.line_to(12.5, 20.5);
            context.stroke().ok()?;
        }
        Some(gdk::Cursor::from_surface(
            &window.display(),
            &surface,
            12.5,
            12.5,
        ))
    })();
    window.set_cursor(cursor.as_ref());
}

/// Pins the controls at a logical position inside the overlay, chosen by dragging.
fn place_toolbar_at(toolbar: &gtk::EventBox, position: (i32, i32)) {
    toolbar.set_halign(gtk::Align::Start);
    toolbar.set_valign(gtk::Align::Start);
    toolbar.set_margin_start(position.0);
    toolbar.set_margin_top(position.1);
    toolbar.set_margin_bottom(0);
}

/// Move released controls above a selection that covers their usual bottom position.
/// Selection pixels are transformed into the same logical coordinates as GTK allocations.
/// A position chosen by dragging is kept as it is.
fn place_toolbar(
    canvas: &gtk::DrawingArea,
    toolbar: &gtk::EventBox,
    area: Option<Area>,
    width: i32,
    height: i32,
    manual: Option<(i32, i32)>,
) {
    if let Some(position) = manual {
        place_toolbar_at(toolbar, position);
        return;
    }
    let overlaps_bottom = area.is_some_and(|area| {
        let (scale, left, top) = transform(canvas, width, height);
        let toolbar_left = f64::from(canvas.allocated_width() - toolbar.allocated_width()) / 2.0;
        let toolbar_top = f64::from(canvas.allocated_height() - 32 - toolbar.allocated_height());
        let selection_left = left + f64::from(area.x) * scale;
        let selection_top = top + f64::from(area.y) * scale;
        selection_left < toolbar_left + f64::from(toolbar.allocated_width())
            && selection_left + f64::from(area.width) * scale > toolbar_left
            && selection_top < toolbar_top + f64::from(toolbar.allocated_height())
            && selection_top + f64::from(area.height) * scale > toolbar_top
    });
    toolbar.set_valign(if overlaps_bottom {
        gtk::Align::Start
    } else {
        gtk::Align::End
    });
    toolbar.set_margin_top(if overlaps_bottom { 32 } else { 0 });
    toolbar.set_margin_bottom(if overlaps_bottom { 0 } else { 32 });
}

fn update_audio_button(button: &gtk::ToggleButton) {
    let enabled = button.is_active();
    button.set_image(Some(&gtk::Image::from_icon_name(
        Some(if enabled {
            "audio-volume-high-symbolic"
        } else {
            "audio-volume-muted-symbolic"
        }),
        gtk::IconSize::Button,
    )));
    button.set_tooltip_text(Some(if enabled {
        tr("Desktop sound: On (video)")
    } else {
        tr("Desktop sound: Off (video)")
    }));
    if let Some(accessible) = button.accessible() {
        accessible.set_name(tr("Desktop sound"));
        accessible.set_description(if enabled { tr("On") } else { tr("Off") });
    }
}

fn update_controls(label: &gtk::Label, button: &gtk::Button, area: Option<Area>, region: bool) {
    let size = area
        .filter(|_| region)
        .map(|area| format!("{} × {}", area.width, area.height));
    label.set_text(size.as_deref().unwrap_or(""));
    label.set_visible(size.is_some());
    button.set_sensitive(!region || area.is_some());
}

fn handles(area: Area) -> [(f64, f64); 8] {
    let x = f64::from(area.x);
    let y = f64::from(area.y);
    let right = x + f64::from(area.width);
    let bottom = y + f64::from(area.height);
    [
        (x, y),
        ((x + right) / 2.0, y),
        (right, y),
        (right, (y + bottom) / 2.0),
        (right, bottom),
        ((x + right) / 2.0, bottom),
        (x, bottom),
        (x, (y + bottom) / 2.0),
    ]
}

fn start_drag(area: Option<Area>, point: (f64, f64), tolerance: f64) -> Drag {
    if let Some(area) = area {
        let x = f64::from(area.x);
        let y = f64::from(area.y);
        let right = f64::from(area.x + area.width);
        let bottom = f64::from(area.y + area.height);
        if point.0 >= x - tolerance
            && point.0 <= right + tolerance
            && point.1 >= y - tolerance
            && point.1 <= bottom + tolerance
        {
            let left = (point.0 - x).abs() <= tolerance;
            let right_edge = (point.0 - right).abs() <= tolerance;
            let top = (point.1 - y).abs() <= tolerance;
            let bottom_edge = (point.1 - bottom).abs() <= tolerance;
            if left || right_edge || top || bottom_edge {
                return Drag::Resize {
                    original: area,
                    left,
                    right: right_edge,
                    top,
                    bottom: bottom_edge,
                };
            }
            return Drag::Move {
                anchor: point,
                original: area,
            };
        }
    }
    Drag::New(point)
}

fn update_drag(drag: Drag, point: (f64, f64), width: i32, height: i32) -> Option<Area> {
    match drag {
        Drag::New(start) => Area::from_points(start, point, width, height),
        Drag::Move { anchor, original } => Some(Area {
            x: (f64::from(original.x) + point.0 - anchor.0)
                .round()
                .clamp(0.0, f64::from(width - original.width)) as i32,
            y: (f64::from(original.y) + point.1 - anchor.1)
                .round()
                .clamp(0.0, f64::from(height - original.height)) as i32,
            ..original
        }),
        Drag::Resize {
            original,
            left,
            right,
            top,
            bottom,
        } => {
            let start = (
                if left { point.0 } else { f64::from(original.x) },
                if top { point.1 } else { f64::from(original.y) },
            );
            let end = (
                if right {
                    point.0
                } else {
                    f64::from(original.x + original.width)
                },
                if bottom {
                    point.1
                } else {
                    f64::from(original.y + original.height)
                },
            );
            Area::from_points(start, end, width, height)
        }
    }
}

fn transform(canvas: &gtk::DrawingArea, width: i32, height: i32) -> (f64, f64, f64) {
    let scale = (f64::from(canvas.allocated_width()) / f64::from(width))
        .min(f64::from(canvas.allocated_height()) / f64::from(height))
        .max(0.001);
    (
        scale,
        (f64::from(canvas.allocated_width()) - f64::from(width) * scale) / 2.0,
        (f64::from(canvas.allocated_height()) - f64::from(height) * scale) / 2.0,
    )
}

fn native_point(
    canvas: &gtk::DrawingArea,
    point: (f64, f64),
    width: i32,
    height: i32,
) -> (f64, f64) {
    let (scale, left, top) = transform(canvas, width, height);
    (
        ((point.0 - left) / scale).clamp(0.0, f64::from(width)),
        ((point.1 - top) / scale).clamp(0.0, f64::from(height)),
    )
}

fn monitor_for(
    parent: &gtk::ApplicationWindow,
    position: Option<(i32, i32)>,
    width: i32,
    height: i32,
) -> i32 {
    let Some(display) = gdk::Display::default() else {
        return 0;
    };
    let mut matching_size = None;
    for index in 0..display.n_monitors() {
        if let Some(monitor) = display.monitor(index) {
            let geometry = monitor.geometry();
            if position == Some((geometry.x(), geometry.y())) {
                return index;
            }
            if geometry.width() * monitor.scale_factor() == width
                && geometry.height() * monitor.scale_factor() == height
            {
                matching_size = Some(index);
            }
        }
    }
    if let Some(index) = matching_size {
        return index;
    }
    if let Some(window) = parent.window() {
        if let Some(monitor) = display.monitor_at_window(&window) {
            for index in 0..display.n_monitors() {
                if display.monitor(index).as_ref() == Some(&monitor) {
                    return index;
                }
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_and_resize_preserve_valid_desktop_bounds() {
        let original = Area {
            x: 10,
            y: 10,
            width: 30,
            height: 20,
        };
        let movement = start_drag(Some(original), (25.0, 20.0), 2.0);
        assert_eq!(
            update_drag(movement, (200.0, 200.0), 100, 80),
            Some(Area {
                x: 70,
                y: 60,
                ..original
            })
        );
        let resize = start_drag(Some(original), (10.0, 10.0), 2.0);
        assert_eq!(
            update_drag(resize, (-10.0, -10.0), 100, 80),
            Some(Area {
                x: 0,
                y: 0,
                width: 40,
                height: 30
            })
        );
    }
}

/// Keep the icon-only control understandable to keyboard and screen-reader users.
/// The icon is a plain white arrow pointer drawn here, not a themed mouse picture, so
/// it contrasts with the dark toolbar. It is the same in both states: only the pressed
/// button shows that the pointer is included, because a differently shaded arrow would
/// read as another valid pointer rather than as a state.
fn update_pointer_button(button: &gtk::ToggleButton) {
    if let Some(surface) = pointer_arrow(button.scale_factor().max(1)) {
        button.set_image(Some(&gtk::Image::from_surface(Some(&surface))));
    }
    button.set_tooltip_text(Some(tr(if button.is_active() {
        "Include pointer in captures: On"
    } else {
        "Include pointer in captures: Off"
    })));
    if let Some(accessible) = button.accessible() {
        accessible.set_name(tr("Mouse pointer"));
    }
}
