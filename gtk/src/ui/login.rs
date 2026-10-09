//! Sign in, like web/src/Login.svelte with the server added: or, on a new server, make its first
//! account (the admin); or join with an invite link, like web/src/Join.svelte.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use gtk::glib;

use crate::{api::Api, bg, cache::Session};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    SignIn,
    /// The server has no accounts yet: this one becomes its admin.
    Setup,
    Join,
}

struct Form {
    mode: Cell<Mode>,
    status: adw::StatusPage,
    server: adw::EntryRow,
    link: adw::EntryRow,
    username: adw::EntryRow,
    name: adw::EntryRow,
    password: adw::PasswordEntryRow,
    error: gtk::Label,
    button: gtk::Button,
    switch: gtk::Button,
    spinner: adw::Spinner,
    /// Each check of the server or link gets a number, so a slow answer never undoes a newer one.
    checks: Cell<u32>,
    timer: RefCell<Option<glib::SourceId>>,
    done: Box<dyn Fn(Session)>,
}

/// The sign-in page; `done` gets the new session.
pub fn page(last_server: Option<String>, done: impl Fn(Session) + 'static) -> gtk::Widget {
    let server = adw::EntryRow::builder().title("Server").text(last_server.unwrap_or_default()).build();
    server.set_input_purpose(gtk::InputPurpose::Url);
    let link = adw::EntryRow::builder().title("Invite Link").visible(false).build();
    let username = adw::EntryRow::builder().title("Username").build();
    let name = adw::EntryRow::builder().title("Your Name").visible(false).build();
    let password = adw::PasswordEntryRow::builder().title("Password").build();
    let group = adw::PreferencesGroup::new();
    for row in [link.upcast_ref::<gtk::Widget>(), server.upcast_ref(), username.upcast_ref(), name.upcast_ref(), password.upcast_ref()] {
        group.add(row);
    }

    let error = gtk::Label::builder().wrap(true).css_classes(["error"]).visible(false).build();
    let button = gtk::Button::builder().label("Sign In").halign(gtk::Align::Center).css_classes(["suggested-action", "pill"]).build();
    let switch = gtk::Button::builder().label("Have an invite link?").halign(gtk::Align::Center).css_classes(["flat"]).build();
    let spinner = adw::Spinner::builder().visible(false).build();

    let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(24).build();
    column.append(&group);
    column.append(&error);
    column.append(&button);
    column.append(&spinner);
    column.append(&switch);

    let status = adw::StatusPage::builder()
        .icon_name(super::APP_ID)
        .title("Gnotes")
        .description("Sign in to your notes server")
        .child(&adw::Clamp::builder().maximum_size(400).child(&column).build())
        .vexpand(true)
        .build();
    let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&status).build();
    let header = adw::HeaderBar::builder().show_title(false).build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scroll));

    let form = Rc::new(Form {
        mode: Cell::new(Mode::SignIn),
        status,
        server: server.clone(),
        link: link.clone(),
        username: username.clone(),
        name,
        password: password.clone(),
        error,
        button: button.clone(),
        switch: switch.clone(),
        spinner,
        checks: Cell::new(0),
        timer: RefCell::new(None),
        done: Box::new(done),
    });

    for entry in [server.upcast_ref::<adw::EntryRow>(), &link, &username, &form.name, password.upcast_ref()] {
        let f = form.clone();
        entry.connect_entry_activated(move |_| f.submit());
    }
    let f = form.clone();
    button.connect_clicked(move |_| f.submit());
    let f = form.clone();
    switch.connect_clicked(move |_| {
        let next = if f.mode.get() == Mode::Join { Mode::SignIn } else { Mode::Join };
        f.set_mode(next);
        if next == Mode::Join {
            f.link.grab_focus();
        } else {
            f.check_later();
        }
    });
    // A new server may need its first account; an invite link says who it's from.
    let f = form.clone();
    server.connect_changed(move |_| f.check_later());
    let f = form.clone();
    link.connect_changed(move |_| f.check_later());
    form.check_later();

    // For gtk/scripts/e2e.mjs: fill the form and submit it.
    //   GNOTES_LOGIN="<server> <username> <password>"
    //   GNOTES_SETUP="<server> <username> <name> <password>"
    //   GNOTES_JOIN="<invite link> <username> <name> <password>"
    let script = |var: &str| std::env::var(var).ok().map(|v| v.split_whitespace().map(str::to_owned).collect::<Vec<_>>());
    let auto = if let Some([s, u, p]) = script("GNOTES_LOGIN").as_deref() {
        server.set_text(s);
        username.set_text(u);
        password.set_text(p);
        Some(Mode::SignIn)
    } else if let Some([s, u, n, p]) = script("GNOTES_SETUP").as_deref() {
        server.set_text(s);
        username.set_text(u);
        form.name.set_text(n);
        password.set_text(p);
        Some(Mode::Setup)
    } else if let Some([l, u, n, p]) = script("GNOTES_JOIN").as_deref() {
        form.set_mode(Mode::Join);
        link.set_text(l);
        username.set_text(u);
        form.name.set_text(n);
        password.set_text(p);
        Some(Mode::Join)
    } else {
        None
    };
    if let Some(mode) = auto {
        let f = form.clone();
        // After the server check has picked the form.
        glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
            if f.mode.get() != mode {
                eprintln!("login: the server didn't call for this form");
                std::process::exit(3);
            }
            f.submit();
        });
    }
    view.upcast()
}

impl Form {
    fn set_mode(&self, mode: Mode) {
        self.mode.set(mode);
        let (title, description, action) = match mode {
            Mode::SignIn => ("Gnotes", "Sign in to your notes server", "Sign In"),
            Mode::Setup => ("Set Up Gnotes", "This server is new. Make the first account; it's the admin.", "Create Account"),
            Mode::Join => ("Join Gnotes", "Paste the invite link someone sent you", "Join"),
        };
        self.status.set_title(title);
        self.status.set_description(Some(description));
        self.button.set_label(action);
        self.link.set_visible(mode == Mode::Join);
        self.server.set_visible(mode != Mode::Join);
        self.name.set_visible(mode != Mode::SignIn);
        self.switch.set_label(if mode == Mode::Join { "Sign in instead" } else { "Have an invite link?" });
        self.switch.set_visible(mode != Mode::Setup);
        self.error.set_visible(false);
    }

    /// Checks the server or invite link once typing pauses.
    fn check_later(self: &Rc<Self>) {
        if let Some(t) = self.timer.take() {
            t.remove();
        }
        let f = self.clone();
        *self.timer.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            f.timer.take();
            f.check();
        }));
    }

    fn check(self: &Rc<Self>) {
        let n = self.checks.get() + 1;
        self.checks.set(n);
        let f = self.clone();
        if self.mode.get() == Mode::Join {
            let link = self.link.text().trim().to_owned();
            if link.is_empty() {
                return;
            }
            // A whole link names its server; a bare token uses the one typed before.
            if let Some((server, _)) = link.split_once("/join/") {
                self.server.set_text(server);
            }
            let server = self.server.text().to_string();
            glib::spawn_future_local(async move {
                let preview = bg(async move { Api::preview_invite(&server, &link).await }).await;
                if f.checks.get() != n || f.mode.get() != Mode::Join {
                    return;
                }
                match preview {
                    Ok(p) => {
                        let shared = p.shared.map(|s| format!(" and shared “{s}” with you")).unwrap_or_default();
                        f.status.set_description(Some(&format!("{} invited you{shared}. Make your account.", p.inviter)));
                        f.error.set_visible(false);
                    }
                    Err(e) => f.show_error(&if e.status() == Some(404) { "This invite link was used or has expired".to_owned() } else { e.to_string() }),
                }
            });
            return;
        }
        let server = self.server.text().trim().to_owned();
        if server.is_empty() {
            return;
        }
        glib::spawn_future_local(async move {
            let needed = bg(async move { Api::setup_needed(&server).await }).await;
            if f.checks.get() != n || f.mode.get() == Mode::Join {
                return;
            }
            let want = if matches!(needed, Ok(true)) { Mode::Setup } else { Mode::SignIn };
            if f.mode.get() != want {
                f.set_mode(want);
            }
        });
    }

    fn show_error(&self, msg: &str) {
        self.error.set_label(msg);
        self.error.set_visible(true);
    }

    fn submit(self: &Rc<Self>) {
        let mode = self.mode.get();
        let server = self.server.text().trim().to_owned();
        let link = self.link.text().trim().to_owned();
        let user = self.username.text().trim().to_owned();
        let name = self.name.text().trim().to_owned();
        let pass = self.password.text().to_string();
        let missing = server.is_empty() || user.is_empty() || pass.is_empty() || (mode == Mode::Join && link.is_empty());
        if missing {
            self.show_error(match mode {
                Mode::Join => "Fill in the invite link, a username and a password",
                _ => "Fill in the server, your username and your password",
            });
            return;
        }
        if mode != Mode::SignIn && pass.chars().count() < 8 {
            self.show_error("Use a password of at least 8 characters");
            return;
        }
        self.error.set_visible(false);
        self.button.set_sensitive(false);
        self.spinner.set_visible(true);
        let f = self.clone();
        glib::spawn_future_local(async move {
            let result = bg(async move {
                match mode {
                    Mode::SignIn => Api::login(&server, &user, &pass).await,
                    Mode::Setup => Api::setup(&server, &user, &name, &pass).await,
                    Mode::Join => Api::join(&server, &link, &user, &name, &pass).await,
                }
            })
            .await;
            f.button.set_sensitive(true);
            f.spinner.set_visible(false);
            match result {
                Ok((api, user)) => (f.done)(Session { server: api.base.clone(), token: api.token.clone(), user }),
                Err(e) => f.show_error(&e.to_string()),
            }
        });
    }
}
