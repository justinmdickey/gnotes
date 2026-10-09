//! Settings, like web/src/Settings.svelte: your account, notes import, devices and API keys; for
//! admins, people and invites, and the optional AI services (web/src/ServiceForm.svelte).

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use super::{lists::when, window::Main};
use crate::{
    api::{AdminUser, ApiKey, PendingInvite, Role, ServiceInput, ServiceSettings},
    bg,
};

const API_DOCS: &str = "https://github.com/justinmdickey/gnotes/blob/main/docs/API.md";

/// Runs a request off the GTK thread; a failure shows as a toast and gives None.
async fn call<T, F>(main: &Main, f: F) -> Option<T>
where
    F: std::future::Future<Output = crate::api::Result<T>> + Send + 'static,
    T: Send + 'static,
{
    match bg(f).await {
        Ok(v) => Some(v),
        Err(e) => {
            main.toast(&e.to_string());
            None
        }
    }
}

/// The invite link for a token, on this server.
pub fn invite_url(main: &Main, token: &str) -> String {
    format!("{}/join/{token}", main.store.api.base)
}

fn copy(text: &str) {
    if let Some(display) = gdk::Display::default() {
        display.clipboard().set_text(text);
    }
}

fn button(label: &str) -> gtk::Button {
    gtk::Button::builder().label(label).valign(gtk::Align::Center).build()
}

pub fn open(main: &Rc<Main>) {
    open_at(main, "account");
}

/// Opens Settings on a page: account, people or ai.
pub fn open_at(main: &Rc<Main>, page: &str) {
    let dialog = adw::PreferencesDialog::builder().title("Settings").search_enabled(false).build();
    dialog.add(&account_page(main, &dialog));
    // Notes on this computer have no one else to manage; AI services still apply.
    if main.store.user().is_admin && !main.store.local {
        dialog.add(&people_page(main, &dialog));
    }
    if main.store.user().is_admin {
        dialog.add(&services_page(main, &dialog));
    }
    dialog.set_visible_page_name(page);
    dialog.present(Some(&main.window));
}

// Your account.

fn account_page(main: &Rc<Main>, dialog: &adw::PreferencesDialog) -> adw::PreferencesPage {
    let me = main.store.user();
    let page = adw::PreferencesPage::builder().name("account").title("Account").icon_name("avatar-default-symbolic").build();

    let profile = adw::PreferencesGroup::new();
    let who = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&me.display_name))
        .subtitle(if main.store.local { "Notes on this computer".to_owned() } else { format!("@{}{}", me.username, if me.is_admin { " · Admin" } else { "" }) })
        .build();
    who.add_prefix(&super::person(&me.display_name, crate::note::color_for(&me.id), 32));
    profile.add(&who);
    page.add(&profile);

    let account = adw::PreferencesGroup::builder().title("Account").build();
    let name = adw::EntryRow::builder().title("Your Name").text(&me.display_name).show_apply_button(true).build();
    let m = main.clone();
    name.connect_apply(move |row| {
        let (m, name, who) = (m.clone(), row.text().trim().to_owned(), who.clone());
        glib::spawn_future_local(async move {
            if set_name(&m, &name).await {
                who.set_title(&glib::markup_escape_text(&name));
            }
        });
    });
    account.add(&name);
    let password = adw::ActionRow::builder().title("Password").build();
    let change = button("Change…");
    let (m, d) = (main.clone(), dialog.clone());
    change.connect_clicked(move |_| change_password(&m, &d));
    password.add_suffix(&change);
    // The account for notes on this computer has a password nobody types.
    password.set_visible(!main.store.local);
    account.add(&password);
    page.add(&account);

    let notes = adw::PreferencesGroup::builder().title("Notes").build();
    let import = adw::ActionRow::builder().title("Import Notes").subtitle("Markdown files, or a zip of them, as from Obsidian or Bear").build();
    let pick = button("Import…");
    let m = main.clone();
    pick.connect_clicked(move |_| pick_import(&m));
    import.add_suffix(&pick);
    notes.add(&import);
    page.add(&notes);

    let devices = adw::PreferencesGroup::builder().title("Devices").build();
    let others = adw::ActionRow::builder().title("Other Devices").subtitle("Signs out everywhere but here").build();
    let out = button("Log Out Others");
    let m = main.clone();
    out.connect_clicked(move |_| {
        let m = m.clone();
        glib::spawn_future_local(async move {
            let api = m.store.api.clone();
            if call(&m, async move { api.logout_others().await }).await.is_some() {
                m.toast("Other devices were signed out");
            }
        });
    });
    others.add_suffix(&out);
    devices.add(&others);
    let here = adw::ActionRow::builder().title("This Computer").subtitle("Log out, and delete the notes kept here").build();
    let logout = button("Log Out");
    logout.add_css_class("destructive-action");
    logout.set_action_name(Some("win.logout"));
    here.add_suffix(&logout);
    devices.add(&here);
    // Notes on this computer: no other devices or API to speak of, but a server to move to.
    if main.store.local {
        devices.set_visible(false);
        let here = adw::PreferencesGroup::builder()
            .title("This Computer")
            .description("Your notes are kept on this computer only.")
            .build();
        let row = adw::ActionRow::builder()
            .title("Move to a Server")
            .subtitle("Use your notes on other devices, and share them")
            .build();
        let go = button("Move…");
        go.add_css_class("suggested-action");
        let (m, d) = (main.clone(), dialog.clone());
        go.connect_clicked(move |_| move_dialog(&m, Some(&d)));
        row.add_suffix(&go);
        here.add(&row);
        page.add(&here);
    }
    page.add(&devices);

    let keys = adw::PreferencesGroup::builder()
        .title("API Keys")
        .description(format!("Let an agent or script read and change your notes as you, through the <a href=\"{API_DOCS}\">Gnotes API</a>."))
        .build();
    let new_key = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text("New API Key").css_classes(["flat"]).build();
    keys.set_header_suffix(Some(&new_key));
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    keys.add(&list);
    keys.set_visible(!main.store.local);
    page.add(&keys);
    let reload_keys = {
        let (m, list, dialog) = (main.clone(), list.clone(), dialog.clone());
        Rc::new(move || {
            let (m, list, dialog) = (m.clone(), list.clone(), dialog.clone());
            glib::spawn_future_local(async move {
                let api = m.store.api.clone();
                let Some(keys) = call(&m, async move { api.api_keys().await }).await else { return };
                show_keys(&m, &dialog, &list, keys);
            });
        })
    };
    reload_keys();
    let (m, d, r) = (main.clone(), dialog.clone(), reload_keys.clone());
    new_key.connect_clicked(move |_| new_key_dialog(&m, &d, r.clone()));

    let about = adw::PreferencesGroup::new();
    let version = gtk::Label::builder().css_classes(["dim-label", "caption"]).build();
    let features = main.store.features.borrow().version.clone();
    version.set_label(&format!("Gnotes {} · this app {}", if features.is_empty() { "?" } else { &features }, env!("CARGO_PKG_VERSION")));
    about.add(&version);
    page.add(&about);
    page
}

/// Saves your display name; true when it worked.
pub async fn set_name(main: &Main, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let (api, n) = (main.store.api.clone(), name.to_owned());
    match call(main, async move { api.update_me(&n).await }).await {
        Some(user) => {
            main.store.set_user(user);
            main.toast("Name saved");
            true
        }
        None => false,
    }
}

fn change_password(main: &Rc<Main>, parent: &adw::PreferencesDialog) {
    let dialog = adw::AlertDialog::new(Some("Change Password"), Some("Your other devices are signed out."));
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    let current = adw::PasswordEntryRow::builder().title("Current Password").build();
    let next = adw::PasswordEntryRow::builder().title("New Password (8+ characters)").build();
    let again = adw::PasswordEntryRow::builder().title("Confirm New Password").build();
    for r in [&current, &next, &again] {
        list.append(r);
    }
    dialog.set_extra_child(Some(&list));
    dialog.add_responses(&[("cancel", "Cancel"), ("change", "Change")]);
    dialog.set_response_appearance("change", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("change"));
    dialog.set_close_response("cancel");
    let m = main.clone();
    dialog.connect_response(None, move |_, r| {
        if r != "change" {
            return;
        }
        let (cur, new) = (current.text().to_string(), next.text().to_string());
        if new != again.text().as_str() {
            m.toast("The new passwords don't match");
            return;
        }
        let m = m.clone();
        glib::spawn_future_local(async move {
            let api = m.store.api.clone();
            if call(&m, async move { api.change_password(&cur, &new).await }).await.is_some() {
                m.toast("Password changed. Your other devices were signed out.");
            }
        });
    });
    dialog.present(Some(parent));
}

fn show_keys(main: &Rc<Main>, dialog: &adw::PreferencesDialog, list: &gtk::ListBox, keys: Vec<ApiKey>) {
    list.remove_all();
    if keys.is_empty() {
        list.append(&adw::ActionRow::builder().title("No keys yet").css_classes(["dim-label"]).build());
    }
    for key in keys {
        let used = key.last_used_at.map_or("Never used".to_owned(), |t| format!("Used {}", when(t).to_lowercase()));
        let scope = if key.scope == "write" { "Read and write" } else { "Read only" };
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(&key.name)).subtitle(format!("{scope} · {used}")).build();
        let revoke = button("Revoke");
        revoke.add_css_class("destructive-action");
        let (m, d, list2) = (main.clone(), dialog.clone(), list.clone());
        revoke.connect_clicked(move |_| {
            let ask = adw::AlertDialog::new(Some(&format!("Revoke “{}”?", key.name)), Some("Anything using this key stops working right away."));
            ask.add_responses(&[("cancel", "Cancel"), ("revoke", "Revoke")]);
            ask.set_response_appearance("revoke", adw::ResponseAppearance::Destructive);
            ask.set_close_response("cancel");
            let (m, id, d2, list2) = (m.clone(), key.id.clone(), d.clone(), list2.clone());
            ask.connect_response(None, move |_, r| {
                if r != "revoke" {
                    return;
                }
                let (m, id, d2, list2) = (m.clone(), id.clone(), d2.clone(), list2.clone());
                glib::spawn_future_local(async move {
                    if revoke_key(&m, &id).await {
                        let api = m.store.api.clone();
                        if let Some(keys) = call(&m, async move { api.api_keys().await }).await {
                            show_keys(&m, &d2, &list2, keys);
                        }
                    }
                });
            });
            ask.present(Some(&d));
        });
        row.add_suffix(&revoke);
        list.append(&row);
    }
}

/// Makes an API key; its token, shown only now.
pub async fn make_key(main: &Main, name: &str, scope: &str) -> Option<String> {
    let (api, n, s) = (main.store.api.clone(), name.to_owned(), scope.to_owned());
    call(main, async move { api.create_api_key(&n, &s).await }).await
}

pub async fn revoke_key(main: &Main, id: &str) -> bool {
    let (api, id) = (main.store.api.clone(), id.to_owned());
    let ok = call(main, async move { api.revoke_api_key(&id).await }).await.is_some();
    if ok {
        main.toast("Key revoked");
    }
    ok
}

fn new_key_dialog(main: &Rc<Main>, parent: &adw::PreferencesDialog, reload: Rc<dyn Fn()>) {
    let dialog = adw::AlertDialog::new(Some("New API Key"), None);
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    let name = adw::EntryRow::builder().title("Name, e.g. Claude agent").build();
    let read = gtk::CheckButton::new();
    let write = gtk::CheckButton::builder().group(&read).build();
    read.set_active(true);
    let read_row = adw::ActionRow::builder().title("Read only").subtitle("Can look at your notes").activatable_widget(&read).build();
    read_row.add_prefix(&read);
    let write_row = adw::ActionRow::builder().title("Read and write").subtitle("Can also make and change notes").activatable_widget(&write).build();
    write_row.add_prefix(&write);
    list.append(&name);
    list.append(&read_row);
    list.append(&write_row);
    dialog.set_extra_child(Some(&list));
    dialog.add_responses(&[("cancel", "Cancel"), ("create", "Create")]);
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("cancel");
    dialog.set_response_enabled("create", false);
    {
        let d = dialog.clone();
        name.connect_changed(move |e| d.set_response_enabled("create", !e.text().trim().is_empty()));
    }
    let (m, p) = (main.clone(), parent.clone());
    dialog.connect_response(None, move |_, r| {
        if r != "create" {
            return;
        }
        let (m, p, reload) = (m.clone(), p.clone(), reload.clone());
        let (n, scope) = (name.text().trim().to_owned(), if write.is_active() { "write" } else { "read" });
        glib::spawn_future_local(async move {
            let Some(token) = make_key(&m, &n, scope).await else { return };
            reload();
            let show = adw::AlertDialog::new(
                Some("Copy Your Key"),
                Some("This is the only time it's shown. Give it to your agent as Authorization: Bearer <key>."),
            );
            let field = gtk::Entry::builder().text(&token).editable(false).css_classes(["monospace"]).build();
            show.set_extra_child(Some(&field));
            show.add_responses(&[("copy", "Copy"), ("done", "Done")]);
            show.set_response_appearance("done", adw::ResponseAppearance::Suggested);
            show.set_close_response("done");
            let m2 = m.clone();
            show.connect_response(Some("copy"), move |_, _| {
                copy(&token);
                m2.toast("Key copied");
            });
            show.present(Some(&p));
        });
    });
    dialog.present(Some(parent));
}

fn pick_import(main: &Rc<Main>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Markdown, text and zip files"));
    for p in ["*.md", "*.markdown", "*.txt", "*.zip"] {
        filter.add_pattern(p);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder().title("Import Notes").filters(&filters).modal(true).build();
    let m = main.clone();
    dialog.open_multiple(Some(&main.window), gio::Cancellable::NONE, move |files| {
        let Ok(files) = files else { return };
        let files: Vec<gio::File> = (0..files.n_items()).filter_map(|i| files.item(i).and_downcast::<gio::File>()).collect();
        let m = m.clone();
        glib::spawn_future_local(async move { import(&m, files).await });
    });
}

/// Whether a file can be imported: Markdown, text, or a zip of them.
pub fn importable(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".md", ".markdown", ".txt", ".zip"].iter().any(|e| lower.ends_with(e))
}

/// Imports files into the current notebook, as Settings › Import and dropping files on the window do.
pub async fn import(main: &Rc<Main>, files: Vec<gio::File>) {
    let mut parts = Vec::new();
    for f in files {
        let name = f.basename().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        if !importable(&name) {
            continue;
        }
        match f.load_contents_future().await {
            Ok((bytes, _)) => parts.push((name, bytes.to_vec())),
            Err(e) => main.toast(&format!("Couldn't read {name}: {e}")),
        }
    }
    if parts.is_empty() {
        main.toast("Only Markdown, text and zip files can be imported");
        return;
    }
    let (api, folder) = (main.store.api.clone(), main.folder.borrow().clone());
    let Some(r) = call(main, async move { api.import(folder.as_deref(), parts).await }).await else { return };
    let mut msg = format!("Imported {} {}", r.notes, if r.notes == 1 { "note" } else { "notes" });
    if r.notebooks > 0 {
        msg.push_str(&format!(" in {} {}", r.notebooks, if r.notebooks == 1 { "notebook" } else { "notebooks" }));
    }
    if !r.skipped.is_empty() {
        msg.push_str(&format!(", skipped {}", r.skipped.len()));
    }
    main.toast(&msg);
    main.store.refresh_tree();
    if let Some(nb) = r.notebook_id {
        main.sidebar().unfold(&nb);
    }
}

// People and invites, for admins.

fn people_page(main: &Rc<Main>, dialog: &adw::PreferencesDialog) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder().name("people").title("People").icon_name("system-users-symbolic").build();
    let people = adw::PreferencesGroup::builder().title("People").description("Everyone with an account on this server").build();
    let invite = gtk::Button::builder().child(&adw::ButtonContent::builder().icon_name("list-add-symbolic").label("Invite").build()).css_classes(["flat"]).build();
    people.set_header_suffix(Some(&invite));
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    people.add(&list);
    page.add(&people);
    let pending = adw::PreferencesGroup::builder().title("Unused Invites").visible(false).build();
    let invites = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    pending.add(&invites);
    page.add(&pending);

    let view = Rc::new(People { main: main.clone(), dialog: dialog.clone(), list, invites, pending, reload: Default::default() });
    let reload: Rc<dyn Fn()> = {
        let view = Rc::downgrade(&view);
        Rc::new(move || {
            if let Some(v) = view.upgrade() {
                v.reload();
            }
        })
    };
    view.reload.replace(Some(reload.clone()));
    // The page keeps the view alive.
    unsafe { page.set_data("gnotes-people", view) };
    reload();
    let (m, d, r) = (main.clone(), dialog.clone(), reload.clone());
    invite.connect_clicked(move |_| {
        let (m, d, r) = (m.clone(), d.clone(), r.clone());
        glib::spawn_future_local(async move {
            let Some(link) = make_invite(&m).await else { return };
            r();
            let show = adw::AlertDialog::new(Some("Invite Someone"), Some("Send them this link. It works once, for 7 days."));
            show.set_extra_child(Some(&gtk::Entry::builder().text(&link).editable(false).build()));
            show.add_responses(&[("copy", "Copy"), ("done", "Done")]);
            show.set_response_appearance("done", adw::ResponseAppearance::Suggested);
            show.set_close_response("done");
            let m2 = m.clone();
            show.connect_response(Some("copy"), move |_, _| {
                copy(&link);
                m2.toast("Invite link copied");
            });
            show.present(Some(&d));
        });
    });
    page
}

/// The People page's lists, which reload after each change.
struct People {
    main: Rc<Main>,
    dialog: adw::PreferencesDialog,
    list: gtk::ListBox,
    invites: gtk::ListBox,
    pending: adw::PreferencesGroup,
    reload: std::cell::RefCell<Option<Rc<dyn Fn()>>>,
}

impl People {
    fn reload(self: &Rc<Self>) {
        let v = self.clone();
        glib::spawn_future_local(async move {
            let api = v.main.store.api.clone();
            let Some((users, open)) = call(&v.main, async move { Ok((api.admin_users().await?, api.invites().await?)) }).await else { return };
            let Some(again) = v.reload.borrow().clone() else { return };
            show_people(&v.main, &v.dialog, &v.list, users, again.clone());
            show_invites(&v.main, &v.invites, &v.pending, open, again);
        });
    }
}

/// A new account's invite link.
pub async fn make_invite(main: &Main) -> Option<String> {
    let api = main.store.api.clone();
    let token = call(main, async move { api.invite(None, Role::Editor).await }).await?;
    Some(invite_url(main, &token))
}

fn show_people(main: &Rc<Main>, dialog: &adw::PreferencesDialog, list: &gtk::ListBox, users: Vec<AdminUser>, reload: Rc<dyn Fn()>) {
    list.remove_all();
    let me = main.store.user();
    for u in users {
        let mut about = vec![format!("@{}", u.username)];
        if u.is_admin {
            about.push("Admin".into());
        }
        if u.disabled {
            about.push("Can't log in".into());
        }
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(&u.display_name)).subtitle(about.join(" · ")).build();
        row.add_prefix(&super::person(&u.display_name, crate::note::color_for(&u.id), 32));
        if u.id != me.id {
            let menu = gio::Menu::new();
            let group = gio::SimpleActionGroup::new();
            let action = |name: &str, label: &str, f: Box<dyn Fn()>| {
                let a = gio::SimpleAction::new(name, None);
                a.connect_activate(move |_, _| f());
                group.add_action(&a);
                menu.append(Some(label), Some(&format!("person.{name}")));
            };
            let patch = |body: serde_json::Value, done: String| -> Box<dyn Fn()> {
                let (m, id, r) = (main.clone(), u.id.clone(), reload.clone());
                Box::new(move || {
                    let (m, id, r, body, done) = (m.clone(), id.clone(), r.clone(), body.clone(), done.clone());
                    glib::spawn_future_local(async move {
                        let api = m.store.api.clone();
                        if call(&m, async move { api.admin_update_user(&id, body).await }).await.is_some() {
                            m.toast(&done);
                            r();
                        }
                    });
                })
            };
            let name = &u.display_name;
            if u.is_admin {
                action("admin", "Remove Admin", patch(serde_json::json!({ "is_admin": false }), format!("{name} is no longer an admin")));
            } else {
                action("admin", "Make Admin", patch(serde_json::json!({ "is_admin": true }), format!("{name} is now an admin")));
            }
            let (m, d, id, n) = (main.clone(), dialog.clone(), u.id.clone(), name.clone());
            action("reset", "Reset Password…", Box::new(move || reset_password(&m, &d, &id, &n)));
            if u.disabled {
                action("disable", "Enable Account", patch(serde_json::json!({ "disabled": false }), format!("{name} can log in again")));
            } else {
                action("disable", "Disable Account", patch(serde_json::json!({ "disabled": true }), format!("{name} was signed out and can't log in")));
            }
            let button = gtk::MenuButton::builder().icon_name("view-more-symbolic").menu_model(&menu).valign(gtk::Align::Center).css_classes(["flat"]).tooltip_text(format!("Manage {name}")).build();
            button.insert_action_group("person", Some(&group));
            row.add_suffix(&button);
        }
        list.append(&row);
    }
}

fn reset_password(main: &Rc<Main>, parent: &adw::PreferencesDialog, id: &str, name: &str) {
    let dialog = adw::AlertDialog::new(Some(&format!("Reset {name}'s Password")), Some("They're signed out everywhere and log in with this one."));
    let field = adw::PasswordEntryRow::builder().title("New Password (8+ characters)").build();
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    list.append(&field);
    dialog.set_extra_child(Some(&list));
    dialog.add_responses(&[("cancel", "Cancel"), ("reset", "Reset")]);
    dialog.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    let (m, id, name) = (main.clone(), id.to_owned(), name.to_owned());
    dialog.connect_response(Some("reset"), move |_, _| {
        let (m, id, name, pw) = (m.clone(), id.clone(), name.clone(), field.text().to_string());
        glib::spawn_future_local(async move {
            let api = m.store.api.clone();
            if call(&m, async move { api.admin_reset_password(&id, &pw).await }).await.is_some() {
                m.toast(&format!("{name}'s password was reset"));
            }
        });
    });
    dialog.present(Some(parent));
}

fn show_invites(main: &Rc<Main>, list: &gtk::ListBox, group: &adw::PreferencesGroup, invites: Vec<PendingInvite>, reload: Rc<dyn Fn()>) {
    list.remove_all();
    group.set_visible(!invites.is_empty());
    let tree = main.store.tree.borrow().clone();
    for inv in invites {
        let what = match (&inv.resource_type, &inv.resource_id) {
            (Some(kind), Some(id)) => {
                let name = if kind == "notebook" {
                    tree.notebooks.iter().find(|n| &n.id == id).map(|n| n.name.clone())
                } else {
                    tree.notes.iter().find(|n| &n.id == id).map(|n| n.title.clone())
                };
                format!("Shares “{}”", name.unwrap_or_else(|| "a note".into()))
            }
            _ => "New account".into(),
        };
        let row = adw::ActionRow::builder().title(what).subtitle(format!("Made {} · until {}", when(inv.created_at).to_lowercase(), when(inv.expires_at))).build();
        let cancel = button("Cancel");
        let (m, id, r) = (main.clone(), inv.id.clone(), reload.clone());
        cancel.connect_clicked(move |_| {
            let (m, id, r) = (m.clone(), id.clone(), r.clone());
            glib::spawn_future_local(async move {
                let api = m.store.api.clone();
                if call(&m, async move { api.revoke_invite(&id).await }).await.is_some() {
                    m.toast("Invite cancelled");
                    r();
                }
            });
        });
        row.add_suffix(&cancel);
        list.append(&row);
    }
}

// AI services, for admins.

struct Service {
    kind: &'static str,
    title: &'static str,
    subtitle: &'static str,
    explain: &'static str,
    url: &'static str,
    model: &'static str,
}

const SERVICES: [Service; 4] = [
    Service {
        kind: "whisper",
        title: "Speech-to-Text",
        subtitle: "Transcribe voice memos",
        explain: "Voice memos are sent here to be transcribed. Any OpenAI-compatible service works, such as faster-whisper-server or OpenAI. Add a live URL to see words in the note while you record.",
        url: "http://whisper:8000/v1",
        model: "whisper-1",
    },
    Service {
        kind: "vision",
        title: "Text from Photos",
        subtitle: "Read the text in photos",
        explain: "Get Text on a photo sends it here, and the text in it is written under the photo. Any OpenAI-compatible service with a vision model works.",
        url: "http://ollama:11434/v1",
        model: "qwen2.5vl",
    },
    Service {
        kind: "summary",
        title: "AI Summaries",
        subtitle: "Summarize notes, and answer Ask Your Notes",
        explain: "Summaries, Tidy Up, Edit with AI, the AI Prompt block and Ask's answers use this chat model. Nothing is sent until someone asks. Any OpenAI-compatible chat service works.",
        url: "http://ollama:11434/v1",
        model: "llama3.1",
    },
    Service {
        kind: "embed",
        title: "Semantic Search",
        subtitle: "Find notes by meaning in Ask",
        explain: "Notes are sent here in pieces to be indexed, and again after edits, so Ask finds them by meaning. Any OpenAI-compatible embeddings service works, such as Ollama with nomic-embed-text.",
        url: "http://ollama:11434/v1",
        model: "nomic-embed-text",
    },
];

fn services_page(main: &Rc<Main>, dialog: &adw::PreferencesDialog) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder().name("ai").title("AI Services").icon_name("starred-symbolic").build();
    let group = adw::PreferencesGroup::builder()
        .title("AI Services")
        .description("Optional. Each one sends data to a service you choose, and nothing is sent while it's off.")
        .build();
    page.add(&group);
    let (m, d, g) = (main.clone(), dialog.clone(), group.clone());
    glib::spawn_future_local(async move {
        let api = m.store.api.clone();
        let Some(all) = call(&m, async move { api.services().await }).await else { return };
        for s in &SERVICES {
            let settings = match s.kind {
                "whisper" => all.whisper.clone(),
                "vision" => all.vision.clone(),
                "summary" => all.summary.clone(),
                _ => all.embed.clone(),
            };
            g.add(&service_row(&m, &d, s, settings));
        }
    });
    page
}

/// A service's row: a switch, and when open its URL, model, key, Test and Save.
fn service_row(main: &Rc<Main>, dialog: &adw::PreferencesDialog, s: &'static Service, settings: ServiceSettings) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::builder()
        .title(s.title)
        .subtitle(s.subtitle)
        .show_enable_switch(true)
        .enable_expansion(settings.enabled)
        .expanded(false)
        .build();
    let explain = gtk::Label::builder().label(s.explain).wrap(true).xalign(0.0).css_classes(["dim-label", "caption"]).margin_top(10).margin_bottom(10).margin_start(12).margin_end(12).build();
    row.add_row(&explain);
    let url = adw::EntryRow::builder().title(format!("URL, e.g. {}", s.url)).text(&settings.url).build();
    let model = adw::EntryRow::builder().title(format!("Model, e.g. {}", s.model)).text(&settings.model).build();
    let key = adw::PasswordEntryRow::builder().title(if settings.has_key { "API Key (saved; type to replace)" } else { "API Key (optional)" }).build();
    row.add_row(&url);
    row.add_row(&model);
    row.add_row(&key);
    let live = (s.kind == "whisper").then(|| {
        let live = adw::EntryRow::builder().title("Live URL (optional), e.g. ws://whisper:8000/v1/realtime").text(settings.realtime_url.clone().unwrap_or_default()).build();
        row.add_row(&live);
        live
    });
    if settings.from_env {
        row.add_row(&adw::ActionRow::builder().title("Set by the server's environment").subtitle("Saving here takes over from it").css_classes(["dim-label"]).build());
    }
    let actions = adw::ActionRow::new();
    let result = gtk::Label::builder().wrap(true).xalign(0.0).hexpand(true).build();
    actions.add_prefix(&result);
    let test = button("Test");
    let save = button("Save");
    save.add_css_class("suggested-action");
    actions.add_suffix(&test);
    actions.add_suffix(&save);
    if s.kind == "embed" {
        let reindex = button("Re-index");
        reindex.set_tooltip_text(Some("Embed every note again, e.g. after swapping the model under the same name"));
        let m = main.clone();
        reindex.connect_clicked(move |_| {
            let m = m.clone();
            glib::spawn_future_local(async move {
                let api = m.store.api.clone();
                if let Some(n) = call(&m, async move { api.reindex().await }).await {
                    m.toast(&format!("Re-indexing {n} {}", if n == 1 { "note" } else { "notes" }));
                }
            });
        });
        actions.add_suffix(&reindex);
    }
    row.add_row(&actions);

    let input = {
        let (url, model, key, live) = (url.clone(), model.clone(), key.clone(), live.clone());
        move |enabled: bool| ServiceInput {
            url: url.text().trim().to_owned(),
            model: model.text().trim().to_owned(),
            key: Some(key.text().to_string()).filter(|k| !k.is_empty()),
            realtime_url: live.as_ref().map(|l| l.text().trim().to_owned()),
            enabled,
        }
    };
    let input = Rc::new(input);
    let (m, i, r) = (main.clone(), input.clone(), result.clone());
    test.connect_clicked(move |_| {
        let (m, body, r) = (m.clone(), i(true), r.clone());
        r.set_label("Testing…");
        glib::spawn_future_local(async move {
            let api = m.store.api.clone();
            let out = bg(async move { api.test_service(s.kind, &body).await }).await;
            let (ok, msg) = match out {
                Ok(t) => (t.ok, t.message),
                Err(e) => (false, e.to_string()),
            };
            r.set_label(&msg);
            r.set_css_classes(if ok { &["success"] } else { &["error"] });
        });
    });
    let (m, i, r, row2, key2) = (main.clone(), input.clone(), result.clone(), row.clone(), key.clone());
    save.connect_clicked(move |_| {
        let (m, body, r, row2, key2) = (m.clone(), i(true), r.clone(), row2.clone(), key2.clone());
        glib::spawn_future_local(async move {
            if let Some(saved) = save_service(&m, s.kind, body).await {
                r.set_label("");
                key2.set_text("");
                row2.set_enable_expansion(saved.enabled);
                m.toast(if saved.enabled { "Saved" } else { "Turned off" });
            }
        });
    });
    // The switch turns a saved service on or off, keeping what's filled in.
    let (m, i) = (main.clone(), input.clone());
    row.connect_enable_expansion_notify(move |row| {
        let on = row.enables_expansion();
        let body = i(on);
        if body.url.is_empty() {
            if on {
                row.set_expanded(true);
            }
            return;
        }
        let m = m.clone();
        glib::spawn_future_local(async move {
            if save_service(&m, s.kind, body).await.is_some() {
                m.toast(if on { "Turned on" } else { "Turned off" });
            }
        });
    });
    let _ = dialog;
    row
}

/// Saves a service's settings (an empty URL clears it); what the server kept.
pub async fn save_service(main: &Rc<Main>, kind: &str, body: ServiceInput) -> Option<ServiceSettings> {
    let (api, kind) = (main.store.api.clone(), kind.to_owned());
    let saved = call(main, async move { api.save_service(&kind, &body).await }).await;
    main.store.refresh_features();
    saved
}

/// Move to a Server: sign in (or set up, or join) on a server, then everything here moves there and
/// the app switches to it. The notes stay on this computer too, in a backup folder.
pub fn move_dialog(main: &Rc<Main>, parent: Option<&adw::PreferencesDialog>) {
    let dialog = adw::Dialog::builder().title("Move to a Server").content_width(460).content_height(640).build();
    let (m, d) = (main.clone(), dialog.clone());
    let signin = super::login::page(None, false, move |session| {
        let (m, d) = (m.clone(), d.clone());
        glib::spawn_future_local(async move { move_to(&m, &d, session).await });
    });
    dialog.set_child(Some(&signin));
    match parent {
        Some(p) => dialog.present(Some(p)),
        None => dialog.present(Some(&main.window)),
    }
}

/// Moves everything here to the server of `session`, showing how far it's got in `dialog`, then
/// switches to it. True when it did.
pub async fn move_to(main: &Rc<Main>, dialog: &adw::Dialog, session: crate::cache::Session) -> bool {
    let bar = gtk::ProgressBar::builder().show_text(true).text("Getting ready…").build();
    let page = adw::StatusPage::builder()
        .icon_name("send-to-symbolic")
        .title("Moving Your Notes…")
        .description(format!("To {}", session.server))
        .child(&adw::Clamp::builder().maximum_size(300).child(&bar).build())
        .build();
    dialog.set_child(Some(&page));
    dialog.set_can_close(false);
    let (tx, rx) = async_channel::unbounded::<(usize, usize)>();
    let b = bar.clone();
    glib::spawn_future_local(async move {
        while let Ok((done, total)) = rx.recv().await {
            b.set_fraction(if total == 0 { 1.0 } else { done as f64 / total as f64 });
            b.set_text(Some(&format!("{done} of {total} notes")));
        }
    });
    let (from, to) = (main.store.api.clone(), crate::api::Api::resume(session.server.clone(), session.token.clone()));
    let result = bg(async move { crate::migrate::move_all(&from, &to, move |d, t| { let _ = tx.try_send((d, t)); }).await }).await;
    dialog.set_can_close(true);
    match result {
        Ok(moved) => {
            // Kept, out of the way, so Start Writing begins afresh rather than with these again.
            let here = crate::local::data_dir();
            let stamp = glib::DateTime::now_local().ok().and_then(|t| t.format("%Y-%m-%d-%H%M%S").ok()).unwrap_or_default();
            if let Err(e) = std::fs::rename(&here, here.with_file_name(format!("local-moved-{stamp}"))) {
                eprintln!("keeping the moved notes aside: {e}");
            }
            dialog.force_close();
            main.switch_to(session);
            main.toast(&format!("Moved {} {} to the server", moved.notes, if moved.notes == 1 { "note" } else { "notes" }));
            true
        }
        Err(e) => {
            page.set_title("Couldn't Move Your Notes");
            page.set_description(Some(&glib::markup_escape_text(&format!("{e:#}. Your notes are still here."))));
            bar.set_visible(false);
            false
        }
    }
}
