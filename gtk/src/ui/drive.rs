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
//! tick <n>                  clicks the nth checkbox, from 0
//! format <style>            title, heading, subheading, body, check, bullet, number, quote, bold, italic
//! new-note
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
        "dump" => {
            let e = editor();
            let v = json!({
                "text": e.as_ref().map(|e| e.shown_text()),
                "title": e.as_ref().map(|e| e.page.title().to_string()),
                "editable": e.as_ref().map(|e| e.editable()),
                "banner": e.as_ref().and_then(|e| e.banner()),
                "peers": e.as_ref().map(|e| e.peer_names()).unwrap_or_default(),
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
