//! Sign in: server, username and password, like web/src/Login.svelte with the server added.

use adw::prelude::*;
use gtk::glib;

use crate::{api::Api, bg, cache::Session};

/// The sign-in page; `done` gets the new session.
pub fn page(last_server: Option<String>, done: impl Fn(Session) + 'static) -> gtk::Widget {
    let done = std::rc::Rc::new(done);
    let server = adw::EntryRow::builder().title("Server").text(last_server.unwrap_or_default()).build();
    server.set_input_purpose(gtk::InputPurpose::Url);
    let username = adw::EntryRow::builder().title("Username").build();
    let password = adw::PasswordEntryRow::builder().title("Password").build();
    let group = adw::PreferencesGroup::new();
    group.add(&server);
    group.add(&username);
    group.add(&password);

    let error = gtk::Label::builder().wrap(true).css_classes(["error"]).visible(false).build();
    let button = gtk::Button::builder()
        .label("Sign In")
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    let spinner = adw::Spinner::builder().visible(false).build();

    let form = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(24).build();
    form.append(&group);
    form.append(&error);
    form.append(&button);
    form.append(&spinner);

    let status = adw::StatusPage::builder()
        .icon_name(super::APP_ID)
        .title("Gnotes")
        .description("Sign in to your notes server")
        .child(&adw::Clamp::builder().maximum_size(400).child(&form).build())
        .vexpand(true)
        .build();

    let header = adw::HeaderBar::builder().show_title(false).build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&status));

    let submit = {
        let (server, username, password, error, button, spinner) =
            (server.clone(), username.clone(), password.clone(), error.clone(), button.clone(), spinner.clone());
        move || {
            let url = server.text().to_string();
            let user = username.text().trim().to_owned();
            let pass = password.text().to_string();
            if url.trim().is_empty() || user.is_empty() || pass.is_empty() {
                error.set_label("Fill in the server, your username and your password");
                error.set_visible(true);
                return;
            }
            error.set_visible(false);
            button.set_sensitive(false);
            spinner.set_visible(true);
            let (error, button, spinner) = (error.clone(), button.clone(), spinner.clone());
            let done = done.clone();
            glib::spawn_future_local(async move {
                let result = bg(async move { Api::login(&url, &user, &pass).await }).await;
                button.set_sensitive(true);
                spinner.set_visible(false);
                match result {
                    Ok((api, user)) => done(Session { server: api.base.clone(), token: api.token.clone(), user }),
                    Err(e) => {
                        error.set_label(&e.to_string());
                        error.set_visible(true);
                    }
                }
            });
        }
    };
    let submit = std::rc::Rc::new(submit);
    for entry in [server.upcast_ref::<adw::EntryRow>(), &username, password.upcast_ref()] {
        let submit = submit.clone();
        entry.connect_entry_activated(move |_| submit());
    }
    // For gtk/scripts/e2e.mjs: `GNOTES_LOGIN="<server> <username> <password>"` fills the form and signs in.
    if let Ok(login) = std::env::var("GNOTES_LOGIN")
        && let [s, u, p] = login.split_whitespace().collect::<Vec<_>>()[..]
    {
        server.set_text(s);
        username.set_text(u);
        password.set_text(p);
        let submit = submit.clone();
        glib::idle_add_local_once(move || submit());
    }
    button.connect_clicked(move |_| submit());
    view.upcast()
}
