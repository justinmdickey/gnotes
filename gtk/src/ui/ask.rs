//! Ask: a chat with your notes, like web/src/Ask.svelte and web/src/lib/ask.svelte.ts. Each question
//! shows the notes found as cards, then an answer streamed from them with numbered links. Follow-ups
//! build on the turns before. The conversation stays while you visit other places, until New Chat or
//! logging out; only the app keeps it.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use adw::prelude::*;
use gtk::glib;

use super::{sidebar, window::Main};
use crate::api::{AskEvent, AskMessage, AskNote, SnippetPart};

#[derive(Default)]
struct Turn {
    question: String,
    /// The notes found; None while searching.
    notes: Option<Vec<AskNote>>,
    terms: Vec<String>,
    /// The answer so far, as it streams in.
    answer: String,
    /// Set once the answer is finished: the notes it cites.
    cited: Option<Vec<u32>>,
    /// The notes didn't say.
    none: bool,
    error: Option<String>,
    widget: gtk::Box,
}

pub struct Ask {
    main: Weak<Main>,
    pub page: adw::NavigationPage,
    stack: gtk::Stack,
    turns_box: gtk::Box,
    disclaimer: gtk::Label,
    scroll: gtk::ScrolledWindow,
    pub entry: gtk::Entry,
    send: gtk::Button,
    new_chat: gtk::Button,
    turns: RefCell<Vec<Turn>>,
    task: RefCell<Option<tokio::task::JoinHandle<()>>>,
    busy: Cell<bool>,
    /// Bumped by New Chat, so a stopped answer's last pieces are dropped.
    chat: Cell<u32>,
}

fn snippet_markup(parts: &[SnippetPart]) -> String {
    parts
        .iter()
        .map(|p| {
            let t = glib::markup_escape_text(&p.text.replace('\n', " "));
            if p.hit { format!("<b>{t}</b>") } else { t.to_string() }
        })
        .collect()
}

/// Whether a search reads like a question, so Ask Your Notes comes first.
pub fn questionish(q: &str) -> bool {
    let q = q.trim().to_lowercase();
    let first = q.split_whitespace().next().unwrap_or_default();
    q.ends_with('?')
        || ["who", "what", "when", "where", "why", "how", "which", "is", "are", "do", "does", "did", "can", "should"].contains(&first)
}

impl Ask {
    pub fn new(main: &Rc<Main>) -> Rc<Ask> {
        let new_chat = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text("New Chat").visible(false).build();
        let header = adw::HeaderBar::new();
        header.pack_end(&new_chat);

        let empty = adw::StatusPage::builder()
            .icon_name("starred-symbolic")
            .title("Ask Your Notes")
            .description("Ask a question, and get an answer from your notes with links to them.")
            .vexpand(true)
            .build();
        let turns_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(28).build();
        let disclaimer = gtk::Label::builder()
            .label("Written by AI from your notes. It can get things wrong.")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label", "caption"])
            .visible(false)
            .build();
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(24).margin_top(24).margin_bottom(24).margin_start(12).margin_end(12).build();
        column.append(&turns_box);
        column.append(&disclaimer);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&adw::Clamp::builder().maximum_size(720).child(&column).build())
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scroll, Some("chat"));

        let entry = gtk::Entry::builder()
            .placeholder_text("Ask your notes…")
            .primary_icon_name("starred-symbolic")
            .max_length(1000)
            .hexpand(true)
            .build();
        let send = gtk::Button::builder().icon_name("go-up-symbolic").tooltip_text("Ask").css_classes(["suggested-action", "circular"]).sensitive(false).build();
        let composer = gtk::Box::builder().spacing(8).margin_top(8).margin_bottom(12).margin_start(12).margin_end(12).build();
        composer.append(&entry);
        composer.append(&send);
        let bottom = adw::Clamp::builder().maximum_size(720).child(&composer).build();

        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&stack));
        view.add_bottom_bar(&bottom);
        let page = adw::NavigationPage::builder().title("Ask").tag("ask").child(&view).build();

        let ask = Rc::new(Ask {
            main: Rc::downgrade(main),
            page,
            stack,
            turns_box,
            disclaimer,
            scroll,
            entry: entry.clone(),
            send: send.clone(),
            new_chat: new_chat.clone(),
            turns: Default::default(),
            task: Default::default(),
            busy: Cell::new(false),
            chat: Cell::new(0),
        });
        let a = Rc::downgrade(&ask);
        entry.connect_changed(move |_| {
            if let Some(a) = a.upgrade() {
                a.refresh_send();
            }
        });
        let a = Rc::downgrade(&ask);
        entry.connect_activate(move |_| {
            if let Some(a) = a.upgrade() {
                a.send_draft();
            }
        });
        let a = Rc::downgrade(&ask);
        send.connect_clicked(move |_| {
            if let Some(a) = a.upgrade() {
                a.send_draft();
            }
        });
        let a = Rc::downgrade(&ask);
        new_chat.connect_clicked(move |_| {
            if let Some(a) = a.upgrade() {
                a.end();
                a.entry.grab_focus();
            }
        });
        let e = entry.clone();
        ask.page.connect_shown(move |_| {
            e.grab_focus();
        });
        ask
    }

    fn refresh_send(&self) {
        self.send.set_sensitive(!self.busy.get() && !self.entry.text().trim().is_empty());
    }

    fn send_draft(self: &Rc<Self>) {
        let q = self.entry.text().trim().to_owned();
        if q.is_empty() || self.busy.get() {
            return;
        }
        self.entry.set_text("");
        self.ask(&q);
    }

    /// Drops the conversation and stops any answer still coming.
    pub fn end(&self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        self.chat.set(self.chat.get() + 1);
        self.turns.borrow_mut().clear();
        while let Some(c) = self.turns_box.first_child() {
            self.turns_box.remove(&c);
        }
        self.busy.set(false);
        self.entry.set_text("");
        self.refresh();
    }

    /// Starts a new conversation with `question`.
    pub fn start(self: &Rc<Self>, question: &str) {
        self.end();
        self.ask(question);
    }

    fn refresh(&self) {
        let chatting = !self.turns.borrow().is_empty();
        self.stack.set_visible_child_name(if chatting { "chat" } else { "empty" });
        self.new_chat.set_visible(chatting);
        self.entry.set_placeholder_text(Some(if chatting { "Ask a follow-up…" } else { "Ask your notes…" }));
        self.disclaimer.set_visible(self.turns.borrow().iter().any(|t| !t.answer.is_empty()));
        self.refresh_send();
    }

    /// Asks the next question of the conversation.
    pub fn ask(self: &Rc<Self>, question: &str) {
        let Some(main) = self.main.upgrade() else { return };
        // The earlier turns as the server wants them: each question, then its answer if it had one.
        let mut messages: Vec<AskMessage> = Vec::new();
        for t in self.turns.borrow().iter() {
            messages.push(AskMessage { role: "user".into(), text: t.question.clone() });
            if !t.answer.is_empty() && t.cited.is_some() {
                messages.push(AskMessage { role: "assistant".into(), text: t.answer.clone() });
            }
        }
        messages.push(AskMessage { role: "user".into(), text: question.to_owned() });
        let widget = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        self.turns_box.append(&widget);
        self.turns.borrow_mut().push(Turn { question: question.to_owned(), widget, ..Default::default() });
        let index = self.turns.borrow().len() - 1;
        self.busy.set(true);
        self.refresh();
        self.draw(index);

        let (tx, rx) = async_channel::unbounded();
        let api = main.store.api.clone();
        let task = crate::runtime().spawn(async move {
            let sent = tx.clone();
            if let Err(e) = api.ask(&messages, move |ev| {
                let _ = sent.try_send(ev);
            })
            .await
            {
                let _ = tx.try_send(AskEvent::Error(e.to_string()));
            }
        });
        if let Some(old) = self.task.replace(Some(task)) {
            old.abort();
        }
        let (me, chat) = (Rc::downgrade(self), self.chat.get());
        glib::spawn_future_local(async move {
            while let Ok(ev) = rx.recv().await {
                let Some(a) = me.upgrade() else { return };
                if a.chat.get() != chat {
                    return;
                }
                let last = matches!(ev, AskEvent::Done(_) | AskEvent::Error(_));
                a.on_event(index, ev);
                if last {
                    break;
                }
            }
        });
    }

    fn on_event(&self, index: usize, ev: AskEvent) {
        {
            let mut turns = self.turns.borrow_mut();
            let Some(t) = turns.get_mut(index) else { return };
            match ev {
                AskEvent::Sources(s) => {
                    t.notes = Some(s.notes);
                    t.terms = s.terms;
                }
                AskEvent::Text(piece) => t.answer.push_str(&piece),
                AskEvent::Done(d) => {
                    t.none = d.answer.is_none();
                    t.answer = d.answer.unwrap_or_default();
                    t.cited = Some(d.cited);
                    // Only notes that came back: a done with nothing found had no sources.
                    t.notes.get_or_insert_with(Vec::new);
                }
                AskEvent::Error(e) => t.error = Some(e),
            }
            if t.cited.is_some() || t.error.is_some() {
                self.busy.set(false);
            }
        }
        self.draw(index);
        self.refresh();
        // Keep the newest at the bottom in view.
        let adj = self.scroll.vadjustment();
        glib::idle_add_local_once(move || adj.set_value(adj.upper()));
    }

    /// Draws a turn again: question, status, answer and cards.
    fn draw(&self, index: usize) {
        let turns = self.turns.borrow();
        let Some(t) = turns.get(index) else { return };
        while let Some(c) = t.widget.first_child() {
            t.widget.remove(&c);
        }
        let question = gtk::Label::builder()
            .label(&t.question)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(48)
            .xalign(0.0)
            .halign(gtk::Align::End)
            .css_classes(["ask-question"])
            .build();
        t.widget.append(&question);
        let status = |text: &str, spin: bool| {
            let b = gtk::Box::builder().spacing(8).build();
            if spin {
                b.append(&adw::Spinner::new());
            }
            b.append(&gtk::Label::builder().label(text).wrap(true).xalign(0.0).css_classes(["dim-label"]).build());
            b
        };
        if let Some(e) = &t.error {
            t.widget.append(&gtk::Label::builder().label(e).wrap(true).xalign(0.0).css_classes(["error"]).build());
            return;
        }
        let Some(notes) = &t.notes else {
            t.widget.append(&status("Searching your notes…", true));
            return;
        };
        if notes.is_empty() {
            t.widget.append(&status("Nothing in your notes matches that. Try asking another way.", false));
            return;
        }
        if !t.answer.is_empty() {
            let answer = gtk::Label::builder().wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).xalign(0.0).selectable(true).css_classes(["ask-answer"]).build();
            answer.set_markup(&answer_markup(&t.answer, notes));
            let main = self.main.clone();
            answer.connect_activate_link(move |_, uri| {
                if let (Some(m), Some(id)) = (main.upgrade(), uri.strip_prefix("note:")) {
                    m.open_note(id, true);
                }
                glib::Propagation::Stop
            });
            t.widget.append(&answer);
        } else if t.cited.is_none() {
            let n = notes.len();
            t.widget.append(&status(&format!("Reading {}…", if n == 1 { "1 note".to_owned() } else { format!("{n} notes") }), true));
        } else if t.none {
            t.widget.append(&status("Your notes don't seem to say. These came closest:", false));
        }
        // Every note found, then only the ones the finished answer cites.
        let shown: Vec<&AskNote> = match &t.cited {
            Some(c) if !c.is_empty() => c.iter().filter_map(|n| notes.iter().find(|x| x.n == *n)).collect(),
            _ => notes.iter().collect(),
        };
        let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
        let Some(main) = self.main.upgrade() else { return };
        for note in shown {
            let tree_note = main.store.note(&note.note);
            let place = tree_note.as_ref().map(|n| sidebar::path_of(&main.store.tree.borrow(), n.notebook_id.clone())).unwrap_or_default();
            let mut sub = snippet_markup(&note.snippet);
            if !place.is_empty() {
                sub = format!("{sub}\n<small>{}</small>", glib::markup_escape_text(&place));
            }
            let title = if note.title.trim().is_empty() { sidebar::UNTITLED } else { &note.title };
            let row = adw::ActionRow::builder().title(glib::markup_escape_text(title)).subtitle(sub).subtitle_lines(3).activatable(true).build();
            row.add_prefix(&gtk::Label::builder().label(note.n.to_string()).css_classes(["cite-number"]).valign(gtk::Align::Center).build());
            let (m, id) = (self.main.clone(), note.note.clone());
            row.connect_activated(move |_| {
                if let Some(m) = m.upgrade() {
                    m.open_note(&id, true);
                }
            });
            list.append(&row);
        }
        t.widget.append(&list);
    }

    /// The last answer, for tests.
    pub fn last_answer(&self) -> String {
        self.turns.borrow().last().map(|t| t.answer.clone()).unwrap_or_default()
    }

    /// The note the last answer's citation `n` links to.
    pub fn cited_note(&self, n: u32) -> Option<String> {
        let turns = self.turns.borrow();
        turns.last()?.notes.as_ref()?.iter().find(|x| x.n == n).map(|x| x.note.clone())
    }
}

/// The answer with its [n] citations as links to their notes; numbers that aren't a note found are dropped.
fn answer_markup(answer: &str, notes: &[AskNote]) -> String {
    let mut out = String::new();
    let mut rest = answer;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|c| open + c) else { break };
        let inside = &rest[open + 1..close];
        let nums: Option<Vec<u32>> = inside.split(',').map(|n| n.trim().parse().ok()).collect();
        let Some(nums) = nums.filter(|n| !n.is_empty()) else {
            out.push_str(&glib::markup_escape_text(&rest[..=open]));
            rest = &rest[open + 1..];
            continue;
        };
        out.push_str(&glib::markup_escape_text(rest[..open].trim_end()));
        for n in nums {
            if let Some(note) = notes.iter().find(|x| x.n == n) {
                let title = glib::markup_escape_text(if note.title.is_empty() { sidebar::UNTITLED } else { &note.title });
                out.push_str(&format!(" <a href=\"note:{}\" title=\"{title}\"><sup>{n}</sup></a>", note.note));
            }
        }
        rest = &rest[close + 1..];
    }
    out.push_str(&glib::markup_escape_text(rest));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citations_become_links() {
        let notes = vec![AskNote { n: 1, note: "a".into(), title: "Groceries".into(), snippet: vec![] }];
        let m = answer_markup("Buy milk [1]. Also [2] & [x].", &notes);
        assert_eq!(m, "Buy milk <a href=\"note:a\" title=\"Groceries\"><sup>1</sup></a>. Also &amp; [x].");
        assert!(questionish("where is the gift list?") && questionish("How do I") && !questionish("milk"));
    }
}
