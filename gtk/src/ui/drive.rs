//! Drives the app from a script, for gtk/scripts/e2e.mjs: `GNOTES_DRIVE` holds one command per line.
//! A `wait-…` that times out ends the app with exit code 3, saying what it waited for.
//!
//! ```text
//! wait-tree                 the tree has loaded
//! open <screen>             as GNOTES_OPEN: note:<title>, notebook:<name>, recent, search:<query>, …
//! wait-loaded               the open note's content has arrived
//! wait-text <text>          the open note shows <text> (\n for a new line)
//! wait-gone <text>          … and no longer shows it
//! wait-row <label>          the sidebar has a row <label>
//! wait-status <status>      online, offline or connecting
//! wait-peer <name>          <name> has the open note open too
//! wait-list <title>        the content pane lists a row <title> (Recent, Search, …)
//! signal <file>             creates <file>, for the script driving this to see
//! wait-file <file>          waits for the script to create <file>
//! sleep <ms>
//! type <text>               typed at the end of the open note (\n presses Enter)
//! enter | undo | redo
//! cursor <n>                the cursor at the end of line n (from 0)
//! wait-shown <text>         <text> shows whole, no marks hidden in it
//! wait-hidden <text>        some of <text> is hidden by live preview
//! tick <n>                  clicks the nth checkbox, from 0
//! format <style>            title, heading, subheading, body, check, bullet, number, quote, bold, italic
//! new-note
//! add-file <path>           as Add Photo, into the open note
//! wait-media <n>            the open note shows n photos or recordings
//! make-notebook <name>      at the top level
//! rename-notebook <old>|<new>
//! move-note <title>|<notebook>   ("Notes" is the top level)
//! trash-note <title>
//! restore <name>            from the trash
//! share-note <title>|<username>|<editor or viewer>
//! dump <file.json>          the open note's text, title and peers, the sidebar's rows, the status
//! shot <file.png>
//! quit
//! ```

use std::{rc::Rc, time::Duration};

use adw::prelude::*;
use gtk::glib;
use serde_json::json;

use super::{
    dialogs,
    sidebar::Item,
    window::{Main, Place},
};
use crate::sync::Status;

/// Opens a screen by name: `note:<title>`, `notebook:<name>`, `recent`, `search:<query>`, `shared`,
/// `trash`, `share:<title>`, `move:<title>`, `new-notebook`, `home`.
pub fn open_screen(main: &Rc<Main>, target: &str) {
    let (kind, arg) = target.split_once(':').unwrap_or((target, ""));
    let note = |title: &str| main.store.tree.borrow().notes.iter().find(|n| n.title == title).cloned();
    let notebook = |name: &str| main.store.tree.borrow().notebooks.iter().find(|n| n.name == name).cloned();
    match kind {
        "note" => match note(arg) {
            Some(n) => main.open_note(&n.id, false),
            None => eprintln!("no note called {arg}"),
        },
        "notebook" => {
            if let Some(nb) = notebook(arg) {
                main.sidebar().unfold(&nb.id);
            }
        }
        "recent" => main.show_place(Place::Recent),
        "search" => {
            main.query.replace(arg.to_owned());
            main.show_place(Place::Search);
        }
        "shared" => main.show_place(Place::Shared),
        "trash" => main.show_place(Place::Trash),
        "share" | "move" => {
            if let Some(n) = note(arg) {
                main.open_note(&n.id, false);
                let item = Item::Note(n);
                if kind == "share" { dialogs::share(main, &item) } else { dialogs::move_to(main, &item) }
            }
        }
        "new-notebook" => dialogs::new_notebook(main, None),
        "home" | "login" => {}
        _ => eprintln!("unknown screen {target}"),
    }
}

pub fn start(main: &Rc<Main>) {
    let Ok(script) = std::env::var("GNOTES_DRIVE") else { return };
    let main = main.clone();
    glib::spawn_future_local(async move {
        for line in script.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
            let arg = arg.replace("\\n", "\n");
            run(&main, cmd, &arg).await;
        }
    });
}

/// The titles of the rows the content pane lists.
fn list_rows(main: &Main) -> Vec<String> {
    fn walk(w: &gtk::Widget, out: &mut Vec<String>) {
        if let Some(row) = w.downcast_ref::<adw::PreferencesRow>() {
            out.push(row.title().to_string());
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c, out);
            child = c.next_sibling();
        }
    }
    let mut out = Vec::new();
    if let Some(page) = main.nav.visible_page() {
        walk(page.upcast_ref(), &mut out);
    }
    out
}

async fn until(what: &str, ok: impl Fn() -> bool) {
    for _ in 0..200 {
        if ok() {
            return;
        }
        glib::timeout_future(Duration::from_millis(50)).await;
    }
    eprintln!("drive: timed out waiting for {what}");
    std::process::exit(3);
}

async fn run(main: &Rc<Main>, cmd: &str, arg: &str) {
    let editor = || main.editor();
    let status = |s: Status| match s {
        Status::Online => "online",
        Status::Offline => "offline",
        Status::Connecting => "connecting",
    };
    match cmd {
        "wait-tree" => until("the tree", || !main.store.tree.borrow().notes.is_empty()).await,
        "open" => open_screen(main, arg),
        "wait-loaded" => until("the note to load", || editor().is_some_and(|e| e.loaded())).await,
        "wait-text" => until(&format!("text {arg:?}"), || editor().is_some_and(|e| e.shown_text().contains(arg))).await,
        "wait-gone" => until(&format!("no text {arg:?}"), || editor().is_some_and(|e| !e.shown_text().contains(arg))).await,
        "wait-row" => until(&format!("row {arg:?}"), || main.sidebar().labels().iter().any(|l| l.trim_start_matches("> ") == arg)).await,
        "wait-status" => until(&format!("status {arg}"), || status(main.store.status.get()) == arg).await,
        "wait-peer" => until(&format!("peer {arg}"), || editor().is_some_and(|e| e.peer_names().iter().any(|n| n == arg))).await,
        "wait-list" => until(&format!("list row {arg:?}"), || list_rows(main).iter().any(|t| t == arg)).await,
        "signal" => {
            if let Err(e) = std::fs::write(arg, b"") {
                eprintln!("drive: signal {arg}: {e}");
            }
        }
        "wait-file" => {
            // Longer than other waits: the script may be restarting the server.
            for _ in 0..600 {
                if std::path::Path::new(arg).exists() {
                    return;
                }
                glib::timeout_future(Duration::from_millis(50)).await;
            }
            eprintln!("drive: timed out waiting for {arg}");
            std::process::exit(3);
        }
        "sleep" => glib::timeout_future(Duration::from_millis(arg.parse().unwrap_or(100))).await,
        "type" => {
            if let Some(e) = editor() {
                e.type_text(arg);
            }
        }
        "cursor" => {
            if let Some(e) = editor() {
                e.cursor_to_line(arg.parse().unwrap_or(0));
            }
        }
        "wait-shown" => until(&format!("{arg:?} shown"), || editor().is_some_and(|e| e.shows(arg))).await,
        "wait-hidden" => until(&format!("marks hidden in {arg:?}"), || editor().is_some_and(|e| !e.shows(arg))).await,
        "enter" => {
            if let Some(e) = editor() {
                e.press_enter();
            }
        }
        "undo" | "redo" => {
            if let Some(e) = editor() {
                e.run_undo(cmd == "redo");
            }
        }
        "tick" => {
            if let Some(e) = editor()
                && !e.toggle_task(arg.parse().unwrap_or(0))
            {
                eprintln!("drive: no checkbox {arg}");
            }
        }
        "format" => {
            if let Some(e) = editor() {
                e.format(arg);
            }
        }
        "new-note" => {
            let before = main.open_id();
            main.new_note();
            until("the new note", || main.open_id().is_some() && main.open_id() != before).await;
        }
        "add-file" => {
            if let Some(e) = editor() {
                e.add_file(arg);
            }
        }
        "wait-media" => until(&format!("{arg} photos or recordings"), || editor().is_some_and(|e| e.media_shown().len() >= arg.parse().unwrap_or(1))).await,
        "make-notebook" => dialogs::create_notebook(main, None, arg.to_owned()),
        "rename-notebook" => {
            let (old, new) = arg.split_once('|').unwrap_or((arg, arg));
            let id = main.store.tree.borrow().notebooks.iter().find(|n| n.name == old).map(|n| n.id.clone());
            match id {
                Some(id) => dialogs::rename_to(main, &id, new.to_owned()),
                None => eprintln!("drive: no notebook {old}"),
            }
        }
        "move-note" => {
            let (title, to) = arg.split_once('|').unwrap_or((arg, "Notes"));
            let tree = main.store.tree.borrow().clone();
            let target = tree.notebooks.iter().find(|n| n.name == to).map(|n| n.id.clone());
            match tree.notes.iter().find(|n| n.title == title) {
                Some(n) => dialogs::move_item(main, &Item::Note(n.clone()), target),
                None => eprintln!("drive: no note {title}"),
            }
        }
        "trash-note" => {
            let id = main.store.tree.borrow().notes.iter().find(|n| n.title == arg).map(|n| n.id.clone());
            match id {
                Some(id) => dialogs::trash_note(main, &id),
                None => eprintln!("drive: no note {arg}"),
            }
        }
        "restore" => {
            let api = main.store.api.clone();
            let items = crate::bg(async move { api.trash().await }).await.unwrap_or_default();
            match items.iter().find(|i| i.name == arg) {
                Some(i) => dialogs::restore(main, &i.resource_type, &i.id, || {}),
                None => eprintln!("drive: nothing called {arg} in the trash"),
            }
        }
        "share-note" => {
            let parts: Vec<&str> = arg.split('|').collect();
            let id = main.store.tree.borrow().notes.iter().find(|n| n.title == parts[0]).map(|n| n.id.clone());
            let role = if parts.get(2) == Some(&"viewer") { crate::api::Role::Viewer } else { crate::api::Role::Editor };
            match (id, parts.get(1)) {
                (Some(id), Some(user)) => {
                    dialogs::share_with(main, "note", &id, user, role).await;
                }
                _ => eprintln!("drive: share-note <title>|<username>|<role>"),
            }
        }
        "dump" => {
            let e = editor();
            let v = json!({
                "text": e.as_ref().map(|e| e.shown_text()),
                "title": e.as_ref().map(|e| e.page.title().to_string()),
                "editable": e.as_ref().map(|e| e.editable()),
                "banner": e.as_ref().and_then(|e| e.banner()),
                "peers": e.as_ref().map(|e| e.peer_names()).unwrap_or_default(),
                "media": e.as_ref().map(|e| e.media_shown()).unwrap_or_default(),
                "note": main.open_id(),
                "sidebar": main.sidebar().labels(),
                "list": list_rows(main),
                "status": status(main.store.status.get()),
                "dirty": main.store.cache.dirty(),
                "queued": main.store.cache.queued().iter().map(|q| q.id.clone()).collect::<Vec<_>>(),
            });
            if let Err(err) = std::fs::write(arg, serde_json::to_vec_pretty(&v).unwrap_or_default()) {
                eprintln!("drive: writing {arg}: {err}");
            }
        }
        "shot" => {
            let path = std::ffi::OsString::from(arg);
            until("something to draw", || super::shot::save(&main.window, &path)).await;
        }
        "quit" => {
            // Closing the window closes the note cleanly first.
            main.window.close();
            glib::timeout_future(Duration::from_millis(300)).await;
            if let Some(app) = main.window.application() {
                app.quit();
            }
        }
        _ => eprintln!("drive: unknown command {cmd}"),
    }
}
