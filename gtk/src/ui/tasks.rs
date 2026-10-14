//! Tasks, like the PWA's (web/src/Tasks.svelte): every open checkbox in every note you can see,
//! by due date, from the server's task index. Ticking one edits its note on the server; activating
//! one opens the note at that line. Also the New Task dialog, which adds to your Inbox note.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::glib;

use super::{sidebar, window::Main};
use crate::{
    api::{Error, TaskItem},
    bg,
};

/// Today as "YYYY-MM-DD" in local time, the form due dates are written in.
pub fn today() -> String {
    glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d").ok()).map(|s| s.to_string()).unwrap_or_default()
}

/// "YYYY-MM-DD" `n` days from today.
pub fn in_days(n: i32) -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.add_days(n).ok())
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn date(s: &str) -> Option<glib::DateTime> {
    let mut parts = s.split('-').map(|p| p.parse::<i32>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    glib::DateTime::from_local(y, m, d, 12, 0, 0.0).ok()
}

/// "Today", "Tomorrow", "2 days late", "Fri", "Oct 20", and the style class it reads in.
pub fn due_label(due: &str) -> (String, &'static str) {
    let (Some(then), Some(now)) = (date(due), date(&today())) else { return (due.to_owned(), "") };
    let days = (then.difference(&now).as_seconds() as f64 / 86_400.0).round() as i64;
    match days {
        0 => ("Today".into(), "accent"),
        1 => ("Tomorrow".into(), ""),
        -1 => ("Yesterday".into(), "error"),
        d if d < 0 => (format!("{} days late", -d), "error"),
        d if d < 7 => (then.format("%a").map(|s| s.to_string()).unwrap_or_default(), ""),
        _ => (then.format("%b %-d").map(|s| s.to_string()).unwrap_or_default(), ""),
    }
}

/// Where a task lives: "Home › Kitchen · Remodel › Doing".
fn crumb(main: &Main, t: &TaskItem) -> String {
    let path = sidebar::path_of(&main.store.tree.borrow(), t.notebook.clone());
    let title = if t.title.trim().is_empty() { sidebar::UNTITLED } else { &t.title };
    let note = match &t.column {
        Some(c) => format!("{title} › {c}"),
        None => title.to_owned(),
    };
    if path.is_empty() { note } else { format!("{path} · {note}") }
}

struct State {
    items: Option<Vec<TaskItem>>,
    query: String,
    show_done: bool,
}

pub fn page(main: &Rc<Main>) -> adw::NavigationPage {
    let state = Rc::new(RefCell::new(State { items: None, query: String::new(), show_done: false }));
    let col = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    let search = gtk::SearchEntry::builder().placeholder_text("Search tasks").build();
    let groups = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).build();
    col.append(&search);
    col.append(&groups);
    let clamp = adw::Clamp::builder().maximum_size(720).child(&col).build();
    let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&clamp).build();

    let header = adw::HeaderBar::new();
    let add = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text("New Task (Ctrl+T)").action_name("win.new-task").build();
    header.pack_start(&add);

    let fill: Rc<dyn Fn()> = {
        let (main, state, groups) = (Rc::downgrade(main), state.clone(), groups.downgrade());
        Rc::new_cyclic(|me: &std::rc::Weak<Box<dyn Fn()>>| {
            let me = me.clone();
            Box::new(move || {
                let (Some(main), Some(groups)) = (main.upgrade(), groups.upgrade()) else { return };
                fill(&main, &state, &groups, &me);
            }) as Box<dyn Fn()>
        })
    };
    {
        let (state, fill) = (state.clone(), fill.clone());
        search.connect_search_changed(move |s| {
            state.borrow_mut().query = s.text().trim().to_lowercase();
            fill();
        });
    }

    let load = {
        let (weak, state, fill) = (Rc::downgrade(main), state.clone(), fill.clone());
        move || {
            let Some(main) = weak.upgrade() else { return };
            let (state, fill) = (state.clone(), fill.clone());
            glib::spawn_future_local(async move {
                let api = main.store.api.clone();
                match bg(async move { api.tasks().await }).await {
                    Ok(items) => state.borrow_mut().items = Some(items),
                    Err(Error::Offline(_)) => {
                        let mut s = state.borrow_mut();
                        s.items.get_or_insert_with(Vec::new);
                    }
                    Err(e) => main.toast(&e.to_string()),
                }
                fill();
            });
        }
    };
    load();
    // Refetched whenever a note's tasks change anywhere, and redrawn when names or hidden notes change.
    {
        let (alive, reload) = (groups.downgrade(), load.clone());
        main.store.on_tasks(move || {
            if alive.upgrade().is_none() {
                return false;
            }
            reload();
            true
        });
        let alive = groups.downgrade();
        main.store.on_tree(move || {
            if alive.upgrade().is_none() {
                return false;
            }
            load();
            true
        });
    }

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scroll));
    adw::NavigationPage::builder().title("Tasks").tag("tasks").child(&view).build()
}

/// Draws the groups: Overdue, Today, Upcoming, No Date, then Done if asked for.
fn fill(main: &Rc<Main>, state: &Rc<RefCell<State>>, groups: &gtk::Box, me: &std::rc::Weak<Box<dyn Fn()>>) {
    while let Some(c) = groups.first_child() {
        groups.remove(&c);
    }
    let s = state.borrow();
    let Some(items) = &s.items else {
        groups.append(&adw::Spinner::builder().height_request(32).build());
        return;
    };
    let q = &s.query;
    let shown: Vec<&TaskItem> = items.iter().filter(|t| q.is_empty() || t.text.to_lowercase().contains(q) || t.title.to_lowercase().contains(q)).collect();
    let today = today();
    let open: Vec<&TaskItem> = shown.iter().copied().filter(|t| !t.done).collect();
    let mut upcoming: Vec<&TaskItem> = open.iter().copied().filter(|t| t.due.as_deref().is_some_and(|d| d > today.as_str())).collect();
    upcoming.sort_by(|a, b| a.due.cmp(&b.due));
    let sets: [(&str, Vec<&TaskItem>); 4] = [
        ("Overdue", open.iter().copied().filter(|t| t.due.as_deref().is_some_and(|d| d < today.as_str())).collect()),
        ("Today", open.iter().copied().filter(|t| t.due.as_deref() == Some(today.as_str())).collect()),
        ("Upcoming", upcoming),
        ("No Date", open.iter().copied().filter(|t| t.due.is_none()).collect()),
    ];
    let done: Vec<&TaskItem> = shown.iter().copied().filter(|t| t.done).collect();
    let mut any = false;
    for (label, list) in sets {
        if list.is_empty() {
            continue;
        }
        any = true;
        group(main, groups, label, &list, me);
    }
    if !any {
        let status = if q.is_empty() {
            adw::StatusPage::builder()
                .icon_name("checkbox-checked-symbolic")
                .title("Nothing to Do")
                .description("Checkboxes in your notes show up here. Type “- [ ]” in a note, or add one with +.")
                .build()
        } else {
            adw::StatusPage::builder().icon_name("system-search-symbolic").title("No Results").description("No open task has those words").build()
        };
        status.add_css_class("compact");
        groups.append(&status);
    }
    if !done.is_empty() {
        if s.show_done {
            group(main, groups, "Done", &done, me);
        }
        let toggle = gtk::Button::builder()
            .label(if s.show_done { "Hide Done".to_owned() } else { format!("Show {} Done", done.len()) })
            .halign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let (state, me) = (state.clone(), me.clone());
        toggle.connect_clicked(move |_| {
            let show = !state.borrow().show_done;
            state.borrow_mut().show_done = show;
            if let Some(f) = me.upgrade() {
                f();
            }
        });
        groups.append(&toggle);
    }
    let hidden = main.store.tree.borrow().notes.iter().filter(|n| n.hide_tasks).count();
    if hidden > 0 {
        let line = if hidden == 1 { "1 note is hidden from Tasks".to_owned() } else { format!("{hidden} notes are hidden from Tasks") };
        groups.append(&gtk::Label::builder().label(line).css_classes(["dim-label", "caption"]).build());
    }
}

fn group(main: &Rc<Main>, groups: &gtk::Box, label: &str, items: &[&TaskItem], me: &std::rc::Weak<Box<dyn Fn()>>) {
    let title = gtk::Label::builder().label(label).xalign(0.0).css_classes(["heading"]).build();
    if label == "Overdue" {
        title.add_css_class("error");
    }
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    for t in items {
        list.append(&row(main, t, me));
    }
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
    b.append(&title);
    b.append(&list);
    groups.append(&b);
}

fn row(main: &Rc<Main>, t: &TaskItem, me: &std::rc::Weak<Box<dyn Fn()>>) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&t.text))
        .subtitle(glib::markup_escape_text(&crumb(main, t)))
        .activatable(true)
        .build();
    let check = gtk::CheckButton::builder().active(t.done).valign(gtk::Align::Center).tooltip_text(if t.done { "Mark Not Done" } else { "Mark Done" }).build();
    check.add_css_class("selection-mode");
    row.add_prefix(&check);
    if t.done {
        row.add_css_class("dim-label");
    }
    if let Some(due) = &t.due {
        let (text, class) = due_label(due);
        let l = gtk::Label::builder().label(text).css_classes(["caption"]).build();
        if !class.is_empty() {
            l.add_css_class(class);
            l.add_css_class("heading");
        }
        row.add_suffix(&l);
    }
    {
        let (weak, task, me) = (Rc::downgrade(main), t.clone(), me.clone());
        check.connect_toggled(move |c| {
            let Some(main) = weak.upgrade() else { return };
            set_done(&main, &task, c.is_active(), me.clone());
        });
    }
    {
        let (weak, note, line) = (Rc::downgrade(main), t.note.clone(), t.line);
        row.connect_activated(move |_| {
            if let Some(main) = weak.upgrade() {
                main.open_task(&note, line);
            }
        });
    }
    row
}

/// Ticks or unticks a task in its note, with Undo; the list redraws from the server after.
pub fn set_done(main: &Rc<Main>, task: &TaskItem, done: bool, redraw: std::rc::Weak<Box<dyn Fn()>>) {
    let (api, m, task) = (main.store.api.clone(), main.clone(), task.clone());
    glib::spawn_future_local(async move {
        let (a, t) = (api.clone(), task.clone());
        match bg(async move { a.toggle_task(&t, done).await }).await {
            Ok(()) if done => {
                let (api, task) = (api.clone(), task.clone());
                m.toast_undo("Checked off", move || {
                    let (api, task) = (api.clone(), task.clone());
                    glib::spawn_future_local(async move {
                        let _ = bg(async move { api.toggle_task(&TaskItem { done: true, ..task }, false).await }).await;
                    });
                });
            }
            Ok(()) => {}
            Err(Error::Server { status: 409, .. }) => m.toast("That task changed in its note"),
            Err(Error::Offline(_)) => m.toast("Tasks can't be checked off offline yet"),
            Err(e) => m.toast(&e.to_string()),
        }
        // The server's tasks_changed redraws the list; this covers a failure, which sends none.
        if let Some(f) = redraw.upgrade() {
            f();
        }
    });
}

/// Adds a task to the end of your Inbox note, from the New Task dialog or a driven run.
pub fn add(main: &Rc<Main>, text: String, due: Option<String>) {
    let (api, m) = (main.store.api.clone(), main.clone());
    glib::spawn_future_local(async move {
        match bg(async move { api.add_task(&text, due.as_deref()).await }).await {
            Ok(_) => {
                m.toast("Added to Inbox");
                m.store.refresh_tree();
            }
            Err(Error::Offline(_)) => m.toast("Tasks can't be added offline yet"),
            Err(e) => m.toast(&e.to_string()),
        }
    });
}

/// New Task: one line and a due date, for the end of your Inbox note.
pub fn new_task(main: &Rc<Main>) {
    let dialog = adw::AlertDialog::new(Some("New Task"), Some("It goes on the end of your Inbox note"));
    let entry = gtk::Entry::builder().placeholder_text("What needs doing?").activates_default(true).build();
    // Wraps onto a second line in a narrow window rather than running off the dialog.
    let dates = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(6)
        .row_spacing(6)
        .max_children_per_line(5)
        .halign(gtk::Align::Center)
        .build();
    let picked: Rc<RefCell<Option<String>>> = Default::default();
    let mut first: Option<gtk::ToggleButton> = None;
    let weekend = {
        let weekday = glib::DateTime::now_local().map(|d| d.day_of_week()).unwrap_or(1);
        let ahead = (6 - weekday).rem_euclid(7);
        in_days(if ahead == 0 { 7 } else { ahead })
    };
    for (label, value) in [("None", None), ("Today", Some(in_days(0))), ("Tomorrow", Some(in_days(1))), ("Weekend", Some(weekend)), ("Next Week", Some(in_days(7)))] {
        let b = gtk::ToggleButton::builder().label(label).build();
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => {
                b.set_active(true);
                first = Some(b.clone());
            }
        }
        let (picked, entry) = (picked.clone(), entry.clone());
        b.connect_toggled(move |b| {
            if b.is_active() {
                picked.replace(value.clone());
                // The cursor goes back to the text, so Enter still adds the task.
                entry.grab_focus();
            }
        });
        dates.append(&b);
    }
    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
    body.append(&entry);
    body.append(&dates);
    dialog.set_extra_child(Some(&body));
    dialog.add_responses(&[("cancel", "Cancel"), ("add", "Add")]);
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");
    dialog.set_response_enabled("add", false);
    {
        let dialog = dialog.clone();
        entry.connect_changed(move |e| dialog.set_response_enabled("add", !e.text().trim().is_empty()));
    }
    let m = main.clone();
    dialog.connect_response(None, move |_, response| {
        let text = entry.text().trim().to_owned();
        if response == "add" && !text.is_empty() {
            add(&m, text, picked.borrow().clone());
        }
    });
    dialog.present(Some(&main.window));
}

/// The row on the Tasks page showing the task `text`, and its checkbox (for driven runs).
pub fn find_row(main: &Main, text: &str) -> Option<(adw::ActionRow, gtk::CheckButton)> {
    fn walk<T: IsA<gtk::Widget>>(w: &gtk::Widget, out: &mut Vec<T>) {
        if let Some(t) = w.downcast_ref::<T>() {
            out.push(t.clone());
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c, out);
            child = c.next_sibling();
        }
    }
    let page = main.nav.visible_page()?;
    let mut rows: Vec<adw::ActionRow> = Vec::new();
    walk(page.upcast_ref(), &mut rows);
    let row = rows.into_iter().find(|r| r.title() == glib::markup_escape_text(text).as_str())?;
    let mut checks: Vec<gtk::CheckButton> = Vec::new();
    walk(row.upcast_ref(), &mut checks);
    Some((row, checks.into_iter().next()?))
}

#[cfg(test)]
mod tests {
    use super::{due_label, in_days};

    #[test]
    fn due_dates_read_relative_to_today() {
        assert_eq!(due_label(&in_days(0)).0, "Today");
        assert_eq!(due_label(&in_days(1)).0, "Tomorrow");
        assert_eq!(due_label(&in_days(-1)), ("Yesterday".into(), "error"));
        assert_eq!(due_label(&in_days(-3)), ("3 days late".into(), "error"));
        assert_eq!(due_label("not a date").0, "not a date");
    }
}
