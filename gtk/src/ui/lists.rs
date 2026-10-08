//! The content pane's pages besides a note: nothing open, Recent, Search, Shared and Trash.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::{
    sidebar::{self, title_of},
    store::now_ms,
    window::Main,
};
use crate::{
    api::{Role, TreeNote},
    bg,
};

fn new_note_button() -> gtk::Button {
    gtk::Button::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text("New Note (Ctrl+N)")
        .action_name("win.new-note")
        .css_classes(["suggested-action"])
        .build()
}

fn page(tag: &str, title: &str, header: adw::HeaderBar, content: &impl IsA<gtk::Widget>) -> adw::NavigationPage {
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(content));
    adw::NavigationPage::builder().title(title).tag(tag).child(&view).build()
}

fn header() -> adw::HeaderBar {
    let h = adw::HeaderBar::new();
    h.pack_end(&new_note_button());
    h
}

/// A scrolling column of boxed lists.
fn column() -> (gtk::ScrolledWindow, gtk::Box) {
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    let clamp = adw::Clamp::builder().maximum_size(720).child(&b).build();
    let s = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&clamp).build();
    (s, b)
}

fn boxed_list() -> gtk::ListBox {
    gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build()
}

/// "5 min ago", "Yesterday", "Oct 3".
pub fn when(ms: i64) -> String {
    let secs = (now_ms() - ms).max(0) / 1000;
    if secs < 60 {
        return "Just now".into();
    }
    if secs < 3600 {
        return format!("{} min ago", secs / 60);
    }
    let then = glib::DateTime::from_unix_local(ms / 1000).ok();
    let now = glib::DateTime::now_local().ok();
    match (then, now) {
        (Some(t), Some(n)) => {
            let days = n.ymd().2 as i64 - t.ymd().2 as i64;
            if secs < 86_400 && days == 0 {
                t.format("%-l:%M %p").map(|s| s.trim().to_string()).unwrap_or_default()
            } else if secs < 2 * 86_400 && (days == 1 || days < 0) {
                "Yesterday".into()
            } else if t.year() == n.year() {
                t.format("%b %-d").map(|s| s.to_string()).unwrap_or_default()
            } else {
                t.format("%b %-d, %Y").map(|s| s.to_string()).unwrap_or_default()
            }
        }
        _ => String::new(),
    }
}

fn note_row(main: &Rc<Main>, n: &TreeNote, subtitle: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(title_of(n)))
        .subtitle(subtitle)
        .subtitle_lines(2)
        .activatable(true)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("text-x-generic-symbolic"));
    if n.shared || n.role != Role::Owner {
        let badge = gtk::Image::from_icon_name("avatar-default-symbolic");
        badge.add_css_class("shared-badge");
        row.add_suffix(&badge);
    }
    let weak = Rc::downgrade(main);
    let id = n.id.clone();
    row.connect_activated(move |_| {
        if let Some(m) = weak.upgrade() {
            m.open_note(&id, true);
        }
    });
    row
}

/// The note's notebook, and its preview, as a list row says it.
fn where_and_preview(main: &Main, n: &TreeNote) -> String {
    let tree = main.store.tree.borrow();
    let path = sidebar::path_of(&tree, n.notebook_id.clone());
    let place = if path.is_empty() && n.role == Role::Owner { "Notes".to_owned() } else if path.is_empty() { n.owner.clone() } else { path };
    let preview = n.preview.trim();
    let line = if preview.is_empty() { place } else { format!("{place} · {preview}") };
    glib::markup_escape_text(&line).to_string()
}

pub fn empty_page(_main: &Rc<Main>) -> adw::NavigationPage {
    let button = gtk::Button::builder()
        .label("New Note")
        .action_name("win.new-note")
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    let status = adw::StatusPage::builder()
        .icon_name(super::APP_ID)
        .title("No Note Selected")
        .description("Pick a note in the sidebar, or start a new one")
        .child(&button)
        .vexpand(true)
        .build();
    let h = adw::HeaderBar::builder().show_title(false).build();
    let p = page("empty", "Gnotes", h, &status);
    // Enter starts a note.
    p.connect_shown(move |_| {
        button.grab_focus();
    });
    p
}

pub fn recent_page(main: &Rc<Main>) -> adw::NavigationPage {
    let (scroll, col) = column();
    let stack = gtk::Stack::new();
    let empty = adw::StatusPage::builder()
        .icon_name("document-open-recent-symbolic")
        .title("No Notes Yet")
        .description("Notes you write or open show up here, newest first")
        .build();
    stack.add_named(&scroll, Some("list"));
    stack.add_named(&empty, Some("empty"));
    let fill = {
        let (main, col, stack) = (Rc::downgrade(main), col.downgrade(), stack.downgrade());
        move || {
            let (Some(main), Some(col), Some(stack)) = (main.upgrade(), col.upgrade(), stack.upgrade()) else { return false };
            while let Some(c) = col.first_child() {
                col.remove(&c);
            }
            let mut notes = main.store.tree.borrow().notes.clone();
            notes.sort_by_key(|n| -n.updated_at);
            let list = boxed_list();
            for n in notes.iter().take(100) {
                let row = note_row(&main, n, &where_and_preview(&main, n));
                let time = gtk::Label::builder().label(when(n.updated_at)).css_classes(["dim-label", "caption"]).build();
                row.add_suffix(&time);
                list.append(&row);
            }
            col.append(&list);
            stack.set_visible_child_name(if notes.is_empty() { "empty" } else { "list" });
            true
        }
    };
    fill();
    main.store.on_tree(fill);
    page("recent", "Recent", header(), &stack)
}

fn snippet_markup(parts: &[crate::api::SnippetPart]) -> String {
    parts
        .iter()
        .map(|p| {
            let t = glib::markup_escape_text(&p.text.replace('\n', " "));
            if p.hit { format!("<b>{t}</b>") } else { t.to_string() }
        })
        .collect()
}

pub fn search_page(main: &Rc<Main>) -> adw::NavigationPage {
    let entry = gtk::SearchEntry::builder().placeholder_text("Search notes").hexpand(true).build();
    let clamp = adw::Clamp::builder().maximum_size(480).child(&entry).build();
    let h = header();
    h.set_title_widget(Some(&clamp));

    let (scroll, col) = column();
    let stack = gtk::Stack::new();
    let start = adw::StatusPage::builder()
        .icon_name("system-search-symbolic")
        .title("Search Your Notes")
        .description("Find notes by any word in them")
        .build();
    let none = adw::StatusPage::builder().icon_name("system-search-symbolic").title("No Results").build();
    stack.add_named(&start, Some("start"));
    stack.add_named(&scroll, Some("list"));
    stack.add_named(&none, Some("none"));

    // Each search gets a number, so a slow answer to an old query never replaces a newer one.
    let generation = Rc::new(std::cell::Cell::new(0u32));
    let weak = Rc::downgrade(main);
    let results = stack.clone();
    entry.connect_search_changed(move |entry| {
        let stack = &results;
        let Some(main) = weak.upgrade() else { return };
        let q = entry.text().trim().to_owned();
        let n = generation.get() + 1;
        generation.set(n);
        if q.is_empty() {
            stack.set_visible_child_name("start");
            return;
        }
        let (col, stack, none, generation) = (col.clone(), stack.clone(), none.clone(), generation.clone());
        glib::spawn_future_local(async move {
            let api = main.store.api.clone();
            let query = q.clone();
            let found = bg(async move { api.search(&query).await }).await;
            if generation.get() != n {
                return;
            }
            while let Some(c) = col.first_child() {
                col.remove(&c);
            }
            let list = boxed_list();
            let mut count = 0;
            match found {
                Ok(results) => {
                    for r in results {
                        let Some(note) = main.store.note(&r.note) else { continue };
                        list.append(&note_row(&main, &note, &snippet_markup(&r.snippet)));
                        count += 1;
                    }
                }
                Err(_) => {
                    // Offline: titles and previews of the notes on this computer.
                    let words: Vec<String> = q.to_lowercase().split_whitespace().map(str::to_owned).collect();
                    let notes = main.store.tree.borrow().notes.clone();
                    for n in notes {
                        let hay = format!("{} {}", n.title, n.preview).to_lowercase();
                        if words.iter().all(|w| hay.contains(w.as_str())) {
                            list.append(&note_row(&main, &n, &where_and_preview(&main, &n)));
                            count += 1;
                        }
                    }
                }
            }
            col.append(&list);
            none.set_description(Some(&format!("Nothing matches “{}”", glib::markup_escape_text(&q))));
            stack.set_visible_child_name(if count == 0 { "none" } else { "list" });
        });
    });
    let query = main.query.take();
    if !query.is_empty() {
        entry.set_text(&query);
    }
    let p = page("search", "Search", h, &stack);
    let e = entry.clone();
    p.connect_shown(move |_| {
        e.grab_focus();
    });
    p
}

pub fn shared_page(main: &Rc<Main>) -> adw::NavigationPage {
    let (scroll, col) = column();
    let stack = gtk::Stack::new();
    let empty = adw::StatusPage::builder()
        .icon_name("folder-publicshare-symbolic")
        .title("Nothing Shared Yet")
        .description("Notes and notebooks shared with you, and the ones you share, show up here")
        .build();
    stack.add_named(&scroll, Some("list"));
    stack.add_named(&empty, Some("empty"));

    let fill = {
        let (weak, col, stack) = (Rc::downgrade(main), col.downgrade(), stack.downgrade());
        move || {
            let (Some(main), Some(col), Some(stack)) = (weak.upgrade(), col.upgrade(), stack.upgrade()) else { return false };
            while let Some(c) = col.first_child() {
                col.remove(&c);
            }
            let tree = main.store.tree.borrow().clone();
            let with_me = adw::PreferencesGroup::builder().title("Shared with You").build();
            let by_me = adw::PreferencesGroup::builder().title("Shared by You").build();
            let (mut a, mut b) = (0, 0);
            for s in tree.shared.iter().filter(|s| !s.hidden) {
                let row = if s.resource_type == "note" {
                    let Some(n) = tree.notes.iter().find(|n| n.id == s.resource_id) else { continue };
                    note_row(&main, n, &glib::markup_escape_text(&format!("From {}", n.owner)))
                } else {
                    let Some(nb) = tree.notebooks.iter().find(|n| n.id == s.resource_id) else { continue };
                    notebook_row(&main, &nb.id, &nb.name, &format!("From {}", nb.owner))
                };
                let hide = gtk::Button::builder().icon_name("window-close-symbolic").tooltip_text("Hide from Your List").valign(gtk::Align::Center).css_classes(["flat"]).build();
                let (weak, share_id) = (Rc::downgrade(&main), s.share_id.clone());
                hide.connect_clicked(move |_| {
                    let Some(main) = weak.upgrade() else { return };
                    let (api, id) = (main.store.api.clone(), share_id.clone());
                    glib::spawn_future_local(async move {
                        match bg(async move { api.hide_share(&id, true).await }).await {
                            Ok(()) => main.store.refresh_tree(),
                            Err(e) => main.toast(&e.to_string()),
                        }
                    });
                });
                row.add_suffix(&hide);
                with_me.add(&row);
                a += 1;
            }
            let label = |names: &[String]| match names.len() {
                0 => "Shared".to_owned(),
                1 | 2 => format!("Shared with {}", names.join(" and ")),
                n => format!("Shared with {n} people"),
            };
            for nb in tree.notebooks.iter().filter(|n| n.role == Role::Owner && n.shared) {
                by_me.add(&notebook_row(&main, &nb.id, &nb.name, &label(&nb.shared_with)));
                b += 1;
            }
            for n in tree.notes.iter().filter(|n| n.role == Role::Owner && n.shared) {
                by_me.add(&note_row(&main, n, &glib::markup_escape_text(&label(&n.shared_with))));
                b += 1;
            }
            if a > 0 {
                col.append(&with_me);
            }
            if b > 0 {
                col.append(&by_me);
            }
            stack.set_visible_child_name(if a + b == 0 { "empty" } else { "list" });
            true
        }
    };
    fill();
    main.store.on_tree(fill);
    page("shared", "Shared", header(), &stack)
}

/// A notebook in a list: opens it in the sidebar.
fn notebook_row(main: &Rc<Main>, id: &str, name: &str, subtitle: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(name))
        .subtitle(glib::markup_escape_text(subtitle))
        .activatable(true)
        .build();
    let icon = gtk::Image::from_icon_name("folder-symbolic");
    icon.add_css_class("folder-icon");
    row.add_prefix(&icon);
    let (weak, id) = (Rc::downgrade(main), id.to_owned());
    row.connect_activated(move |_| {
        if let Some(m) = weak.upgrade() {
            m.sidebar().unfold(&id);
            m.split.set_show_content(false);
        }
    });
    row
}

pub fn trash_page(main: &Rc<Main>) -> adw::NavigationPage {
    let (scroll, col) = column();
    let stack = gtk::Stack::new();
    let empty = adw::StatusPage::builder()
        .icon_name("user-trash-symbolic")
        .title("Trash Is Empty")
        .description("Deleted notes and notebooks stay here for 30 days")
        .build();
    let spinner = adw::Spinner::new();
    stack.add_named(&spinner, Some("loading"));
    stack.add_named(&scroll, Some("list"));
    stack.add_named(&empty, Some("empty"));

    let h = header();
    let empty_button = gtk::Button::builder().label("Empty Trash").css_classes(["destructive-action"]).visible(false).build();
    h.pack_start(&empty_button);

    let load: Rc<dyn Fn()> = {
        let (weak, col, stack, empty_button) = (Rc::downgrade(main), col.clone(), stack.clone(), empty_button.clone());
        Rc::new_cyclic(|me: &std::rc::Weak<Box<dyn Fn()>>| {
            let me = me.clone();
            Box::new(move || {
                let Some(main) = weak.upgrade() else { return };
                let (col, stack, empty_button, me) = (col.clone(), stack.clone(), empty_button.clone(), me.clone());
                glib::spawn_future_local(async move {
                    let api = main.store.api.clone();
                    let items = match bg(async move { api.trash().await }).await {
                        Ok(items) => items,
                        Err(e) => {
                            main.toast(&e.to_string());
                            Vec::new()
                        }
                    };
                    while let Some(c) = col.first_child() {
                        col.remove(&c);
                    }
                    let list = boxed_list();
                    for item in &items {
                        let days = ((item.purge_at - now_ms()) / 86_400_000).max(0);
                        let row = adw::ActionRow::builder()
                            .title(glib::markup_escape_text(if item.name.trim().is_empty() { sidebar::UNTITLED } else { &item.name }))
                            .subtitle(format!("Deleted {} · gone in {days} days", when(item.deleted_at)))
                            .build();
                        let icon = if item.resource_type == "note" { "text-x-generic-symbolic" } else { "folder-symbolic" };
                        row.add_prefix(&gtk::Image::from_icon_name(icon));
                        let restore = gtk::Button::builder().label("Restore").valign(gtk::Align::Center).build();
                        let forever = gtk::Button::builder()
                            .icon_name("user-trash-full-symbolic")
                            .tooltip_text("Delete Forever")
                            .valign(gtk::Align::Center)
                            .css_classes(["flat"])
                            .build();
                        row.add_suffix(&restore);
                        row.add_suffix(&forever);
                        for (button, kind) in [(restore, "restore"), (forever, "forever")] {
                            let (weak, me) = (Rc::downgrade(&main), me.clone());
                            let (rt, id) = (item.resource_type.clone(), item.id.clone());
                            button.connect_clicked(move |_| {
                                let Some(main) = weak.upgrade() else { return };
                                let (api, rt, id, me) = (main.store.api.clone(), rt.clone(), id.clone(), me.clone());
                                if kind == "restore" {
                                    super::dialogs::restore(&main, &rt, &id, move || {
                                        if let Some(f) = me.upgrade() {
                                            f();
                                        }
                                    });
                                    return;
                                }
                                glib::spawn_future_local(async move {
                                    let r = bg(async move { api.delete_forever(&rt, &id).await }).await;
                                    match r {
                                        Ok(()) => {
                                            main.store.refresh_tree();
                                            if let Some(f) = me.upgrade() {
                                                f();
                                            }
                                        }
                                        Err(e) => main.toast(&e.to_string()),
                                    }
                                });
                            });
                        }
                        list.append(&row);
                    }
                    col.append(&list);
                    empty_button.set_visible(!items.is_empty());
                    stack.set_visible_child_name(if items.is_empty() { "empty" } else { "list" });
                });
            })
        })
    };
    load();
    {
        let (weak, load) = (Rc::downgrade(main), load.clone());
        empty_button.connect_clicked(move |b| {
            let Some(main) = weak.upgrade() else { return };
            let dialog = adw::AlertDialog::new(Some("Empty Trash?"), Some("Everything in the trash will be deleted for good."));
            dialog.add_responses(&[("cancel", "Cancel"), ("empty", "Empty Trash")]);
            dialog.set_response_appearance("empty", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            let load = load.clone();
            let m = main.clone();
            dialog.connect_response(None, move |_, r| {
                if r != "empty" {
                    return;
                }
                let (api, m, load) = (m.store.api.clone(), m.clone(), load.clone());
                glib::spawn_future_local(async move {
                    match bg(async move { api.empty_trash().await }).await {
                        Ok(()) => load(),
                        Err(e) => m.toast(&e.to_string()),
                    }
                });
            });
            dialog.present(Some(b));
        });
    }
    page("trash", "Trash", h, &stack)
}
