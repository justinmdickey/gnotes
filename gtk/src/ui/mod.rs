//! The GTK app: sign-in, then the sidebar and the open note side by side.

// Boxed callbacks are how GTK code hands actions around.
#![allow(clippy::type_complexity)]

mod ai;
mod ask;
mod dialogs;
mod drive;
mod editor;
mod lists;
mod login;
mod markdown;
mod noteview;
mod settings;
mod shot;
mod sidebar;
mod store;
mod styling;
mod window;

use adw::prelude::*;

pub const APP_ID: &str = "com.erisdor.Gnotes";

/// A person as a dot with their initial in their user color, as the PWA shows them. `size` is 24, 26 or 32.
pub fn person(name: &str, color: &str, size: i32) -> gtk::Label {
    let initial: String = name.trim().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    gtk::Label::builder()
        .label(initial)
        .tooltip_text(name)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .css_classes(["peer", color, &format!("size-{size}")])
        .build()
}

/// The app icon, where the icon theme finds it even when the app isn't installed.
fn install_icon() {
    let dir = crate::cache::data_dir().join("icons");
    let path = dir.join("hicolor/scalable/apps").join(format!("{APP_ID}.svg"));
    let svg = include_bytes!("../../data/com.erisdor.Gnotes.svg");
    if std::fs::read(&path).ok().as_deref() != Some(&svg[..]) {
        let _ = std::fs::create_dir_all(path.parent().expect("icon folder"));
        let _ = std::fs::write(&path, svg);
    }
    gtk::IconTheme::for_display(&gtk::gdk::Display::default().expect("display")).add_search_path(dir);
}

pub fn run() -> gtk::glib::ExitCode {
    // Start the runtime before any window, so the first request doesn't wait for it.
    let _ = crate::runtime();
    let app = adw::Application::builder().application_id(APP_ID).build();
    // A run with its own folder (tests, screenshots) is its own instance, not a second window of yours.
    if std::env::var_os("GNOTES_GTK_DIR").is_some() {
        app.set_flags(gtk::gio::ApplicationFlags::NON_UNIQUE);
    }
    app.connect_startup(|_| {
        install_icon();
        shot::force_style();
        gtk::Window::set_default_icon_name(APP_ID);
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("display"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
    app.connect_activate(|app| {
        if let Some(win) = app.active_window() {
            win.present();
            return;
        }
        window::Window::new(app).present();
    });
    app.run()
}
