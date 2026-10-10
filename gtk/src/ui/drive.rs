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
//! wait-menu                the "/" menu or emoji list shows (type "/che" or ":ta" to open it)
//! pick <label>             picks from it
//! code-lang <n>|<lang>     as picking the nth code block's language
//! block-entry <n>|<k>|<text>     types into the kth field of table or board n (cells row by row)
//! block-click <n>|<label>|<k>    clicks the kth button labeled <label> in table or board n
//! move-line <line>|<before>      as dragging line's block (from 0) to before line <before>
//! tick <n>                  clicks the nth checkbox, from 0
//! format <style>            title, heading, subheading, body, check, bullet, number, quote, bold, italic
//! new-note
//! add-file <path>           as Add Photo, into the open note
//! wait-media <n>            the open note shows n photos or recordings
//! make-notebook <name>      at the top level
//! rename-notebook <old>|<new>
//! move-note <title>|<notebook>   ("Notes" is the top level)
//! drag-note <title>|<notebook>   as dragging it onto that notebook in the sidebar ("Notes": the top)
//! trash-note <title>
//! restore <name>            from the trash
//! share-note <title>|<username>|<editor or viewer>
//! set-name <name>
//! make-key <name>|<read or write>|<file>   the key's token goes in <file>
//! revoke-key <name>
//! invite <file>             a new account's invite link goes in <file>
//! save-service <kind>|<url>|<model>         kind: whisper, vision, summary or embed
//! import <path>             as Settings › Import
//! summary                  opens the open note's Summary
//! wait-dialog <text>       the dialog showing has <text> in it
//! tidy | edit-ai <instruction>   Tidy Up or Edit with AI, applied
//! ai-write <prompt>        the AI Prompt at the cursor
//! wait-writer <text>|<name>  the line with <text> is marked as written by <name>
//! record-start | record-stop | record-cancel   a voice memo (GNOTES_AUDIO_SRC stands in for the mic)
//! live-delta <text> | live-final <text>       live transcript words, as the server sends them
//! get-text <n>             Get Text on the open note's nth photo or recording (from 0)
//! wait-answer <text>       Ask's last answer has <text>
//! cite <n>                 opens the note the last answer's [n] links to
//! move-to-server <server> <username> <password>   Move to a Server, signing in there
//! dump <file.json>          the open note's text, title and peers, the sidebar's rows, the status
//! resize <width>x<height>  the window
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
/// `trash`, `share:<title>`, `move:<title>`, `new-notebook`, `menu`, `shortcuts`, `notemenu:<title>`,
/// `styles:<title>`, `home`.
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
        "ask" => {
            if arg.is_empty() {
                main.show_place(Place::Ask);
            } else {
                main.ask_question(arg);
            }
        }
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
        // With AI services: the note's Summary, Tidy Up's preview, Edit with AI asking.
        "summary" | "tidy" | "editai" | "record" | "slash" | "emoji" | "grip" => {
            if let Some(n) = note(arg) {
                main.open_note(&n.id, false);
                let (m, kind) = (main.clone(), kind.to_owned());
                // Once the note has loaded.
                glib::timeout_add_local_once(Duration::from_millis(800), move || {
                    let Some(e) = m.editor() else { return };
                    match kind.as_str() {
                        "summary" => super::ai::summary_dialog(&m, &e),
                        "tidy" => super::ai::rewrite_dialog(&m, &e, super::ai::Rewrite::Tidy),
                        "record" => e.start_recording(),
                        "slash" => e.type_text("\n/"),
                        "grip" => e.show_grip(6, 9),
                        "emoji" => e.type_text("\n:sm"),
                        _ => super::ai::rewrite_dialog(&m, &e, super::ai::Rewrite::Edit),
                    }
                });
            }
        }
        "settings" => super::settings::open_at(main, if arg.is_empty() { "account" } else { arg }),
        "menu" => {
            // Once the window has settled; a popover asked for sooner doesn't open.
            let m = main.clone();
            glib::timeout_add_local_once(Duration::from_millis(500), move || m.sidebar().open_menu());
        }
        "shortcuts" => {
            WidgetExt::activate_action(&main.window, "win.shortcuts", None).ok();
        }
        // The note menu, or the paragraph styles, of a note.
        "notemenu" | "styles" => {
            if let Some(n) = note(arg) {
                main.open_note(&n.id, false);
                let (m, kind) = (main.clone(), kind.to_owned());
                glib::timeout_add_local_once(Duration::from_millis(800), move || {
                    if let Some(e) = m.editor() {
                        e.open_menu(kind == "styles");
                    }
                });
            }
        }
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
            if std::env::var_os("GNOTES_TRACE").is_some() {
                eprintln!("drive> {line}");
            }
            run(&main, cmd, &arg).await;
        }
    });
}

/// Every widget of type T under `w`, in order.
fn widgets<T: IsA<gtk::Widget>>(w: &gtk::Widget) -> Vec<T> {
    let mut out = Vec::new();
    if let Some(t) = w.downcast_ref::<T>() {
        out.push(t.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        out.extend(widgets::<T>(&c));
        child = c.next_sibling();
    }
    out
}

/// The text in the dialog showing: its labels and text views.
fn dialog_text(main: &Main) -> String {
    fn walk(w: &gtk::Widget, out: &mut String) {
        if let Some(l) = w.downcast_ref::<gtk::Label>() {
            out.push_str(&l.label());
            out.push('\n');
        }
        if let Some(t) = w.downcast_ref::<gtk::TextView>() {
            let b = t.buffer();
            out.push_str(&b.text(&b.start_iter(), &b.end_iter(), true));
            out.push('\n');
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c, out);
            child = c.next_sibling();
        }
    }
    let mut out = String::new();
    if let Some(d) = main.window.visible_dialog() {
        walk(d.upcast_ref(), &mut out);
    }
    out
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
        "resize" => {
            if let Some((w, h)) = arg.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
                main.window.set_default_size(w, h);
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
        "wait-menu" => until("the menu", || editor().is_some_and(|e| e.menu_open())).await,
        "pick" => {
            if !editor().is_some_and(|e| e.pick_from_menu(arg)) {
                eprintln!("drive: no {arg} in the menu");
            }
        }
        "code-lang" => {
            let (n, lang) = arg.split_once('|').unwrap_or((arg, ""));
            if let Some(e) = editor() {
                e.set_code_language(n.parse().unwrap_or(0), lang);
            }
        }
        "pick-block" => {
            // Which widget a click at (x, y) inside table or board n lands on, outermost last.
            let parts: Vec<i32> = arg.split('|').filter_map(|p| p.parse().ok()).collect();
            if let (Some(e), &[n, x, y]) = (editor(), &parts[..])
                && let Some(w) = e.block_widget_at(n as usize)
                && let Some(p) = w.compute_point(&main.window, &gtk::graphene::Point::new(x as f32, y as f32))
            {
                let mut chain = Vec::new();
                let mut at = main.window.pick(p.x() as f64, p.y() as f64, gtk::PickFlags::DEFAULT);
                while let Some(w) = at {
                    chain.push(format!("{}{:?}", w.type_().name(), w.css_classes()));
                    at = w.parent();
                }
                eprintln!("drive: pick {arg} at {p:?}: {}", chain.join(" < "));
            }
        }
        "block-action" => {
            // An item in a table's or board's ⋮ menu, by action name, like table.row.
            let (n, action) = arg.split_once('|').unwrap_or((arg, ""));
            let button = editor().and_then(|e| e.block_widget_at(n.parse().unwrap_or(0))).and_then(|w| widgets::<gtk::MenuButton>(&w).into_iter().next());
            if button.is_none_or(|b| b.activate_action(action, None).is_err()) {
                eprintln!("drive: no action {arg}");
            }
        }
        "block-focus" => {
            let (n, a) = arg.split_once('|').unwrap_or((arg, "0"));
            let entry = editor().and_then(|e| e.block_widget_at(n.parse().unwrap_or(0))).and_then(|w| widgets::<gtk::Entry>(&w).into_iter().nth(a.parse().unwrap_or(0)));
            match entry {
                Some(entry) => {
                    entry.grab_focus();
                }
                None => eprintln!("drive: no field {arg}"),
            }
        }
        "block-entry" | "block-click" => {
            let parts: Vec<&str> = arg.splitn(3, '|').collect();
            let (Some(e), &[n, a, b]) = (editor(), &parts[..]) else { return };
            let Some(w) = e.block_widget_at(n.parse().unwrap_or(0)) else {
                eprintln!("drive: no table or board {n}");
                return;
            };
            if cmd == "block-entry" {
                let entries = widgets::<gtk::Entry>(&w);
                match entries.get(a.parse::<usize>().unwrap_or(0)) {
                    Some(entry) => {
                        entry.set_text(b);
                        entry.emit_activate();
                    }
                    None => eprintln!("drive: no field {a}"),
                }
            } else {
                let buttons: Vec<gtk::Button> = widgets::<gtk::Button>(&w)
                    .into_iter()
                    .filter(|btn| widgets::<gtk::Label>(btn.upcast_ref()).iter().any(|l| l.label() == a))
                    .collect();
                match buttons.get(b.parse::<usize>().unwrap_or(0)) {
                    Some(btn) => btn.emit_clicked(),
                    None => eprintln!("drive: no button {a}"),
                }
            }
        }
        "move-line" => {
            let (a, b) = arg.split_once('|').unwrap_or((arg, "0"));
            // By its grip, through the same drop as a pointer's drag.
            if let Some(e) = editor() {
                e.grip_drag(a.parse().unwrap_or(0), b.parse().unwrap_or(0));
            }
        }
        "tick" => {
            if let Some(e) = editor()
                && !e.toggle_task(arg.parse().unwrap_or(0))
            {
                eprintln!("drive: no checkbox {arg}");
            }
        }
        "follow" => {
            if let Some(e) = editor()
                && !e.follow_link(arg.parse().unwrap_or(0))
            {
                eprintln!("drive: no link {arg}");
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
        "drag-note" => {
            let (title, to) = arg.split_once('|').unwrap_or((arg, "Notes"));
            let tree = main.store.tree.borrow().clone();
            let target = tree.notebooks.iter().find(|n| n.name == to).map(|n| n.id.clone());
            match tree.notes.iter().find(|n| n.title == title) {
                Some(n) => {
                    if !super::sidebar::move_dropped(main, &format!("note:{}", n.id), target) {
                        eprintln!("drive: {title} can't be dropped on {to}");
                    }
                }
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
        "set-name" => {
            super::settings::set_name(main, arg).await;
        }
        "make-key" => {
            let parts: Vec<&str> = arg.split('|').collect();
            if let [name, scope, file] = parts[..]
                && let Some(token) = super::settings::make_key(main, name, scope).await
            {
                let _ = std::fs::write(file, token);
            }
        }
        "revoke-key" => {
            let api = main.store.api.clone();
            let keys = crate::bg(async move { api.api_keys().await }).await.unwrap_or_default();
            match keys.iter().find(|k| k.name == arg) {
                Some(k) => {
                    super::settings::revoke_key(main, &k.id).await;
                }
                None => eprintln!("drive: no key {arg}"),
            }
        }
        "invite" => {
            if let Some(link) = super::settings::make_invite(main).await {
                let _ = std::fs::write(arg, link);
            }
        }
        "save-service" => {
            let parts: Vec<&str> = arg.split('|').collect();
            if let [kind, url, model] = parts[..] {
                let body = crate::api::ServiceInput { url: url.into(), model: model.into(), key: None, realtime_url: None, enabled: true };
                super::settings::save_service(main, kind, body).await;
            }
        }
        "import" => super::settings::import(main, vec![gtk::gio::File::for_path(arg)]).await,
        "summary" => {
            if let Some(e) = editor() {
                super::ai::summary_dialog(main, &e);
            }
        }
        "wait-dialog" => until(&format!("a dialog with {arg:?}"), || dialog_text(main).contains(arg)).await,
        "tidy" | "edit-ai" => {
            let kind = if cmd == "tidy" { super::ai::Rewrite::Tidy } else { super::ai::Rewrite::Edit };
            if let Some(e) = editor() {
                match super::ai::rewrite(main, &e, kind, arg).await {
                    Ok((original, text)) => {
                        super::ai::apply(main, &e, kind, &original, &text);
                    }
                    Err(err) => eprintln!("drive: {cmd}: {err}"),
                }
            }
        }
        "ai-write" => {
            if let Some(e) = editor() {
                super::ai::write_here(main, &e, arg).await;
            }
        }
        "wait-writer" => {
            let (text, name) = arg.split_once('|').unwrap_or((arg, ""));
            until(&format!("{text:?} marked as {name}'s"), || {
                editor().is_some_and(|e| {
                    let all = e.shown_text();
                    let Some(line) = all.lines().position(|l| l.contains(text)) else { return false };
                    e.line_writer(line as i32).is_some_and(|w| w.starts_with(name))
                })
            })
            .await
        }
        "record-start" => {
            if let Some(e) = editor() {
                e.start_recording();
            }
        }
        "record-stop" | "record-cancel" => {
            if let Some(e) = editor() {
                e.stop_recording(cmd == "record-stop");
            }
        }
        "live-delta" => {
            if let Some(e) = editor() {
                e.spot_delta(arg);
            }
        }
        "live-final" => {
            if let Some(e) = editor() {
                e.spot_final(arg);
            }
        }
        "get-text" => {
            if let Some(e) = editor()
                && let Some(id) = e.media_ids().get(arg.parse::<usize>().unwrap_or(0)).cloned()
            {
                e.photo_text(&id).await;
            }
        }
        "wait-answer" => until(&format!("an answer with {arg:?}"), || main.ask.get().is_some_and(|a| a.last_answer().contains(arg))).await,
        "cite" => {
            if let Some(id) = main.ask.get().and_then(|a| a.cited_note(arg.parse().unwrap_or(1))) {
                main.open_note(&id, true);
            }
        }
        "move-to-server" => {
            let parts: Vec<String> = arg.split_whitespace().map(str::to_owned).collect();
            let [server, user, pass] = &parts[..] else { return };
            let (s, u, p) = (server.clone(), user.clone(), pass.clone());
            match crate::bg(async move { crate::api::Api::login(&s, &u, &p).await }).await {
                Ok((api, user)) => {
                    let session = crate::cache::Session { server: api.base.clone(), token: api.token.clone(), user, local: false };
                    let dialog = adw::Dialog::new();
                    dialog.present(Some(&main.window));
                    if !super::settings::move_to(main, &dialog, session).await {
                        eprintln!("drive: the move failed");
                    }
                }
                Err(e) => eprintln!("drive: signing in to {server}: {e}"),
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
                "offline_banner": main.sidebar().offline_shown(),
                "paragraph_style": e.as_ref().map(|e| e.paragraph_style()),
                "blocks": e.as_ref().map(|e| e.block_counts()),
                "list": list_rows(main),
                "dialog_text": dialog_text(main),
                "ask_answer": main.ask.get().map(|a| a.last_answer()),
                "status": status(main.store.status.get()),
                "user": main.store.user().display_name,
                "local": main.store.local,
                "dialog": main.window.visible_dialog().map(|d| d.title().to_string()),
                "features": {
                    "summaries": main.store.features.borrow().summaries,
                    "transcription": main.store.features.borrow().transcription,
                    "ask": main.store.features.borrow().ask,
                },
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
