//! New and renamed notebooks, Move to, Share, and moving things to the trash.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::{
    sidebar::{Item, title_of},
    window::{Main, Place},
};
use crate::{
    api::{Role, ShareInfo, TreeNotebook},
    bg,
};

/// Asks for a name; `done` gets it, trimmed and not empty.
fn ask_name(main: &Main, heading: &str, action: &str, initial: &str, done: impl Fn(String) + 'static) {
    let dialog = adw::AlertDialog::new(Some(heading), None);
    let entry = gtk::Entry::builder().text(initial).placeholder_text("Name").activates_default(true).build();
    dialog.set_extra_child(Some(&entry));
    dialog.add_responses(&[("cancel", "Cancel"), ("ok", action)]);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");
    dialog.set_response_enabled("ok", !initial.trim().is_empty());
    {
        let dialog = dialog.clone();
        entry.connect_changed(move |e| dialog.set_response_enabled("ok", !e.text().trim().is_empty()));
    }
    let e = entry.clone();
    dialog.connect_response(None, move |_, r| {
        let name = e.text().trim().to_owned();
        if r == "ok" && !name.is_empty() {
            done(name);
        }
    });
    dialog.present(Some(&main.window));
    entry.grab_focus();
}

/// Runs a request, then refreshes the tree; a failure shows in a toast.
fn request<F>(main: &Rc<Main>, f: F)
where
    F: std::future::Future<Output = crate::api::Result<()>> + Send + 'static,
{
    let main = main.clone();
    glib::spawn_future_local(async move {
        match bg(f).await {
            Ok(()) => main.store.refresh_tree(),
            Err(e) => main.toast(&e.to_string()),
        }
    });
}

pub fn new_notebook(main: &Rc<Main>, parent: Option<String>) {
    let heading = if parent.is_some() { "New Notebook Inside" } else { "New Notebook" };
    let m = main.clone();
    ask_name(main, heading, "Create", "", move |name| create_notebook(&m, parent.clone(), name));
}

/// Makes a notebook and opens it, so new notes go there.
pub fn create_notebook(main: &Rc<Main>, parent: Option<String>, name: String) {
    let (api, m) = (main.store.api.clone(), main.clone());
    glib::spawn_future_local(async move {
        match bg(async move { api.create_notebook(&name, parent.as_deref()).await }).await {
            Ok(id) => {
                m.folder.replace(Some(id.clone()));
                m.sidebar().unfold(&id);
                m.store.refresh_tree();
            }
            Err(e) => m.toast(&e.to_string()),
        }
    });
}

pub fn rename_notebook(main: &Rc<Main>, nb: &TreeNotebook) {
    let (m, id) = (main.clone(), nb.id.clone());
    ask_name(main, "Rename Notebook", "Rename", &nb.name, move |name| rename_to(&m, &id, name));
}

pub fn rename_to(main: &Rc<Main>, id: &str, name: String) {
    let (api, id) = (main.store.api.clone(), id.to_owned());
    request(main, async move { api.rename_notebook(&id, &name).await });
}

/// Moves a note or notebook into `target`, or the top level with None.
pub fn move_item(main: &Rc<Main>, item: &Item, target: Option<String>) {
    let api = main.store.api.clone();
    match item {
        Item::Note(n) => {
            let id = n.id.clone();
            request(main, async move { api.move_note(&id, target.as_deref()).await });
        }
        Item::Notebook(nb) => {
            let id = nb.id.clone();
            request(main, async move { api.move_notebook(&id, target.as_deref()).await });
        }
    }
}

/// Takes something out of the trash.
pub fn restore(main: &Rc<Main>, kind: &str, id: &str, then: impl FnOnce() + 'static) {
    let (api, kind, id, m) = (main.store.api.clone(), kind.to_owned(), id.to_owned(), main.clone());
    glib::spawn_future_local(async move {
        match bg(async move { api.restore(&kind, &id).await }).await {
            Ok(()) => {
                m.store.refresh_tree();
                then();
            }
            Err(e) => m.toast(&e.to_string()),
        }
    });
}

/// Shares a note or notebook (`kind`) with someone; true when it worked.
pub async fn share_with(main: &Rc<Main>, kind: &'static str, id: &str, username: &str, role: Role) -> bool {
    let (api, id, username) = (main.store.api.clone(), id.to_owned(), username.to_owned());
    match bg(async move { api.share(kind, &id, &username, role).await }).await {
        Ok(()) => {
            main.store.refresh_tree();
            true
        }
        Err(e) => {
            main.toast(&e.to_string());
            false
        }
    }
}

/// Notebooks `id` can't go into: itself and everything inside it.
fn inside(notebooks: &[TreeNotebook], id: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::from([id.to_owned()]);
    loop {
        let before = out.len();
        for nb in notebooks {
            if nb.parent_id.as_ref().is_some_and(|p| out.contains(p)) {
                out.insert(nb.id.clone());
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

pub fn move_to(main: &Rc<Main>, item: &Item) {
    let tree = main.store.tree.borrow().clone();
    let (name, current, blocked) = match item {
        Item::Note(n) => (title_of(n).to_owned(), n.notebook_id.clone(), Default::default()),
        Item::Notebook(nb) => (nb.name.clone(), nb.parent_id.clone(), inside(&tree.notebooks, &nb.id)),
    };
    let owner = match item {
        Item::Note(n) => n.role == Role::Owner,
        Item::Notebook(nb) => nb.role == Role::Owner,
    };
    let dialog = adw::Dialog::builder().title(format!("Move “{name}”")).content_width(380).content_height(480).build();
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    let mut targets: Vec<(Option<String>, String, usize)> = Vec::new();
    // Only your own things can go to your top level; shared ones move within their shared notebook.
    if owner {
        targets.push((None, "Notes".into(), 0));
    }
    fn walk(tree: &crate::api::Tree, parent: Option<&str>, depth: usize, out: &mut Vec<(Option<String>, String, usize)>) {
        let mut kids: Vec<&TreeNotebook> = tree
            .notebooks
            .iter()
            .filter(|n| n.parent_id.as_deref() == parent || (parent.is_none() && n.parent_id.as_ref().is_some_and(|p| !tree.notebooks.iter().any(|x| &x.id == p))))
            .collect();
        kids.sort_by_key(|n| n.name.to_lowercase());
        for nb in kids {
            if nb.role.can_edit() {
                out.push((Some(nb.id.clone()), nb.name.clone(), depth));
            }
            walk(tree, Some(&nb.id), depth + 1, out);
        }
    }
    walk(&tree, None, 0, &mut targets);
    for (id, label, depth) in targets {
        if id.as_ref().is_some_and(|i| blocked.contains(i)) {
            continue;
        }
        let here = id == current;
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(&label)).activatable(!here).sensitive(!here).build();
        let icon = gtk::Image::from_icon_name(if id.is_none() { "user-home-symbolic" } else { "folder-symbolic" });
        icon.set_margin_start(depth as i32 * 18);
        row.add_prefix(&icon);
        if here {
            row.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
        }
        let (m, item, dialog) = (main.clone(), item.clone(), dialog.clone());
        row.connect_activated(move |_| {
            move_item(&m, &item, id.clone());
            dialog.close();
        });
        list.append(&row);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&adw::Clamp::builder().child(&list).margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).build())
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scroll));
    dialog.set_child(Some(&view));
    dialog.present(Some(&main.window));
}

/// Moves a note to the trash, with Undo instead of "are you sure?".
pub fn trash_note(main: &Rc<Main>, id: &str) {
    let m = main.clone();
    let id = id.to_owned();
    glib::spawn_future_local(async move {
        let (api, nid) = (m.store.api.clone(), id.clone());
        match bg(async move { api.delete_note(&nid).await }).await {
            Ok(()) => {
                if m.open_id().as_deref() == Some(&id) {
                    m.show_place(Place::Nothing);
                }
                m.store.forget_note(&id);
                m.store.cache.forget(&id);
                m.store.refresh_tree();
                let m2 = m.clone();
                m.toast_undo("Moved to the trash", move || restore(&m2, "note", &id, || {}));
            }
            Err(e) => m.toast(&e.to_string()),
        }
    });
}

pub fn trash_notebook(main: &Rc<Main>, nb: &TreeNotebook) {
    let m = main.clone();
    let id = nb.id.clone();
    let name = nb.name.clone();
    glib::spawn_future_local(async move {
        let (api, nid) = (m.store.api.clone(), id.clone());
        match bg(async move { api.delete_notebook(&nid).await }).await {
            Ok(()) => {
                // The open note may have been inside it.
                if let Some(open) = m.open_id() {
                    let tree = m.store.tree.borrow().clone();
                    let mut nb = tree.notes.iter().find(|n| n.id == open).and_then(|n| n.notebook_id.clone());
                    let gone = inside(&tree.notebooks, &id);
                    let mut inside_it = false;
                    while let Some(x) = nb {
                        if gone.contains(&x) {
                            inside_it = true;
                            break;
                        }
                        nb = tree.notebooks.iter().find(|n| n.id == x).and_then(|n| n.parent_id.clone());
                    }
                    if inside_it {
                        m.show_place(Place::Nothing);
                    }
                }
                if m.folder.borrow().as_deref() == Some(&id) {
                    m.folder.replace(None);
                }
                m.store.refresh_tree();
                let m2 = m.clone();
                m.toast_undo(&format!("“{name}” moved to the trash"), move || restore(&m2, "notebook", &id, || {}));
            }
            Err(e) => m.toast(&e.to_string()),
        }
    });
}

/// Who a note or notebook is shared with, and adding people: like web/src/ShareDialog.svelte.
pub fn share(main: &Rc<Main>, item: &Item) {
    let (kind, id, name) = match item {
        Item::Note(n) => ("note", n.id.clone(), title_of(n).to_owned()),
        Item::Notebook(nb) => ("notebook", nb.id.clone(), nb.name.clone()),
    };
    let dialog = adw::Dialog::builder().title(format!("Share “{name}”")).content_width(420).content_height(460).build();

    let people = gtk::StringList::new(&[]);
    let person = gtk::DropDown::builder().model(&people).hexpand(true).enable_search(true).build();
    let roles = gtk::DropDown::from_strings(&["Can Edit", "Can View"]);
    let add = gtk::Button::builder().label("Share").css_classes(["suggested-action"]).build();
    let add_row = gtk::Box::builder().spacing(6).build();
    add_row.append(&person);
    add_row.append(&roles);
    add_row.append(&add);

    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    let group = adw::PreferencesGroup::builder().title("People with Access").build();
    group.add(&list);
    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(12).margin_bottom(24).margin_start(12).margin_end(12).build();
    let everyone = gtk::Label::builder()
        .label("Everyone on this server has access")
        .css_classes(["dim-label"])
        .visible(false)
        .build();
    body.append(&add_row);
    body.append(&everyone);
    body.append(&group);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&adw::Clamp::builder().child(&body).build()).build()));
    dialog.set_child(Some(&view));

    // Usernames in the dropdown, in its order.
    let usernames: Rc<std::cell::RefCell<Vec<String>>> = Default::default();

    let reload: Rc<dyn Fn()> = {
        let (main, list, people, usernames, add_row, everyone) =
            (main.clone(), list.clone(), people.clone(), usernames.clone(), add_row.clone(), everyone.clone());
        let id = id.clone();
        Rc::new(move || {
            let (main, list, people, usernames, add_row, everyone, id) =
                (main.clone(), list.clone(), people.clone(), usernames.clone(), add_row.clone(), everyone.clone(), id.clone());
            glib::spawn_future_local(async move {
                let (api, sid) = (main.store.api.clone(), id.clone());
                let shares = bg(async move { api.shares(kind, &sid).await }).await;
                let api = main.store.api.clone();
                let users = bg(async move { api.users().await }).await.unwrap_or_default();
                let shares: Vec<ShareInfo> = match shares {
                    Ok(s) => s,
                    Err(e) => {
                        main.toast(&e.to_string());
                        return;
                    }
                };
                while let Some(c) = list.first_child() {
                    list.remove(&c);
                }
                let me = &main.store.user();
                let owner = adw::ActionRow::builder().title(glib::markup_escape_text(&me.display_name)).subtitle("Owner").build();
                owner.add_prefix(&super::person(&me.display_name, crate::note::color_for(&me.id), 32));
                list.append(&owner);
                for s in &shares {
                    let row = adw::ActionRow::builder().title(glib::markup_escape_text(&s.display_name)).subtitle(glib::markup_escape_text(&s.username)).build();
                    let id = users.iter().find(|u| u.username == s.username).map_or("", |u| u.id.as_str());
                    row.add_prefix(&super::person(&s.display_name, crate::note::color_for(id), 32));
                    let role = gtk::DropDown::from_strings(&["Can Edit", "Can View"]);
                    role.set_selected(if s.role == Role::Viewer { 1 } else { 0 });
                    role.set_valign(gtk::Align::Center);
                    let (m, share_id) = (main.clone(), s.id.clone());
                    role.connect_selected_notify(move |d| {
                        let role = if d.selected() == 1 { Role::Viewer } else { Role::Editor };
                        let (api, sid) = (m.store.api.clone(), share_id.clone());
                        request(&m, async move { api.set_share_role(&sid, role).await });
                    });
                    let remove = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text("Stop Sharing").valign(gtk::Align::Center).css_classes(["flat"]).build();
                    let (m, share_id, list2, row2) = (main.clone(), s.id.clone(), list.clone(), row.clone());
                    remove.connect_clicked(move |_| {
                        let (api, sid, m, list2, row2) = (m.store.api.clone(), share_id.clone(), m.clone(), list2.clone(), row2.clone());
                        glib::spawn_future_local(async move {
                            match bg(async move { api.unshare(&sid).await }).await {
                                Ok(()) => {
                                    list2.remove(&row2);
                                    m.store.refresh_tree();
                                }
                                Err(e) => m.toast(&e.to_string()),
                            }
                        });
                    });
                    row.add_suffix(&role);
                    row.add_suffix(&remove);
                    list.append(&row);
                }
                let taken: std::collections::HashSet<&str> = shares.iter().map(|s| s.username.as_str()).collect();
                let mut names = Vec::new();
                let mut labels = Vec::new();
                for u in users.iter().filter(|u| u.id != me.id && !taken.contains(u.username.as_str())) {
                    names.push(u.username.clone());
                    labels.push(format!("{} ({})", u.display_name, u.username));
                }
                people.splice(0, people.n_items(), &labels.iter().map(String::as_str).collect::<Vec<_>>());
                // With everyone in already, there's no one to pick.
                add_row.set_visible(!names.is_empty());
                everyone.set_visible(names.is_empty());
                usernames.replace(names);
            });
        })
    };
    reload();
    {
        let (main, id, reload) = (main.clone(), id.clone(), reload.clone());
        add.connect_clicked(move |_| {
            let Some(username) = usernames.borrow().get(person.selected() as usize).cloned() else { return };
            let role = if roles.selected() == 1 { Role::Viewer } else { Role::Editor };
            let (id, m, reload) = (id.clone(), main.clone(), reload.clone());
            glib::spawn_future_local(async move {
                if share_with(&m, kind, &id, &username, role).await {
                    reload();
                }
            });
        });
    }
    dialog.present(Some(&main.window));
}
