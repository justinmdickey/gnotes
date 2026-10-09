//! For gtk/scripts/shot.mjs: `GNOTES_SHOT=<file.png>` saves the window a moment after it opens, then
//! quits. Rendered here with GSK, so a screenshot shows exactly what GTK drew. `GNOTES_SIZE=<w>x<h>`
//! sets the window size.

use adw::prelude::*;
use gtk::{glib, graphene, gsk};

/// `GNOTES_STYLE=dark|light` forces a style, whatever the desktop prefers.
pub fn force_style() {
    // Headless runs (screenshots, e2e) barely advance animations: show things in their end state.
    if (std::env::var_os("GNOTES_SHOT").is_some() || std::env::var_os("GNOTES_DRIVE").is_some())
        && let Some(s) = gtk::Settings::default()
    {
        s.set_gtk_enable_animations(false);
    }
    let scheme = match std::env::var("GNOTES_STYLE").as_deref() {
        Ok("dark") => adw::ColorScheme::ForceDark,
        Ok("light") => adw::ColorScheme::ForceLight,
        _ => return,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

pub fn size() -> Option<(i32, i32)> {
    let s = std::env::var("GNOTES_SIZE").ok()?;
    let (w, h) = s.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

pub fn schedule(window: &adw::ApplicationWindow) {
    let Some(path) = std::env::var_os("GNOTES_SHOT") else { return };
    let delay = std::env::var("GNOTES_SHOT_DELAY").ok().and_then(|d| d.parse().ok()).unwrap_or(2500);
    let window = window.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
        // Right after a frame is painted, when everything has its size.
        let Some(clock) = window.frame_clock() else { return };
        let handler = std::rc::Rc::new(std::cell::RefCell::new(None));
        let h = handler.clone();
        let id = clock.connect_after_paint(move |clock| {
            if save(&window, &path)
                && let Some(id) = h.take()
            {
                clock.disconnect(id);
                if let Some(app) = window.application() {
                    app.quit();
                }
            }
        });
        handler.replace(Some(id));
        clock.request_phase(gtk::gdk::FrameClockPhase::PAINT);
    });
}

/// False when there was nothing to draw yet. An open popover (a menu) is its own surface, so it goes
/// in a second file next to it, `<name>-popover.png`.
pub fn save(window: &adw::ApplicationWindow, path: &std::ffi::OsStr) -> bool {
    fn popover(w: &gtk::Widget) -> Option<gtk::Popover> {
        if let Some(p) = w.downcast_ref::<gtk::Popover>()
            && p.is_visible()
        {
            return Some(p.clone());
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            if let Some(p) = popover(&c) {
                return Some(p);
            }
            child = c.next_sibling();
        }
        None
    }
    if let Some(p) = popover(window.upcast_ref()) {
        let file = std::path::Path::new(path);
        let side = file.with_file_name(format!("{}-popover.png", file.file_stem().unwrap_or_default().to_string_lossy()));
        render(p.upcast_ref(), side.as_os_str());
    }
    render(window.upcast_ref(), path)
}

fn render(widget: &gtk::Widget, path: &std::ffi::OsStr) -> bool {
    let (w, h) = (widget.width(), widget.height());
    let window = widget;
    let snapshot = gtk::Snapshot::new();
    gtk::WidgetPaintable::new(Some(widget)).snapshot(&snapshot, w as f64, h as f64);
    let Some(node) = snapshot.to_node() else {
        window.queue_draw();
        return false;
    };
    let renderer = gsk::CairoRenderer::new();
    if let Err(e) = renderer.realize_for_display(&WidgetExt::display(window)) {
        eprintln!("screenshot renderer: {e}");
        return true;
    }
    let texture = renderer.render_texture(&node, Some(&graphene::Rect::new(0.0, 0.0, w as f32, h as f32)));
    if let Err(e) = texture.save_to_png(path) {
        eprintln!("saving {}: {e}", path.to_string_lossy());
    }
    renderer.unrealize();
    true
}
