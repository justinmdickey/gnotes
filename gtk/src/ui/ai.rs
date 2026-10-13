//! The note's AI tools, when AI Summaries is set up: Summary, Tidy Up, Edit with AI and the AI
//! Prompt, like web/src/Editor.svelte and web/src/lib/aiPrompt.ts. They only ever change the note
//! through the editor, as the smallest edits, so the change syncs and merges like typing and one
//! Undo takes it back.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::{editor::Editor, styling, window::Main};
use crate::{api::NoteSummary, bg};

/// One edit: characters `from..to` of the old text become `insert`.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub from: usize,
    pub to: usize,
    pub insert: String,
}

/// Lines with their line breaks, so joining them gives the text back.
fn pieces(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// Above this many line pairs, the changed middle is replaced in one piece instead of diffed.
const MAX_PAIRS: usize = 4_000_000;

/// Edits from `a` to `b`, in `a`'s character positions, sorted and not overlapping, like
/// `changesBetween` in web/src/lib/tidy.ts. Unchanged lines are matched by a longest common
/// subsequence; each changed run of lines then loses the characters it starts and ends with in common.
pub fn changes_between(a: &str, b: &str) -> Vec<Change> {
    let (pa, pb) = (pieces(a), pieces(b));
    let mut start = 0;
    while start < pa.len() && start < pb.len() && pa[start] == pb[start] {
        start += 1;
    }
    let mut end = 0;
    while end < pa.len() - start && end < pb.len() - start && pa[pa.len() - 1 - end] == pb[pb.len() - 1 - end] {
        end += 1;
    }
    let am = &pa[start..pa.len() - end];
    let bm = &pb[start..pb.len() - end];
    let (n, m) = (am.len(), bm.len());

    let mut pairs = Vec::new();
    if n * m <= MAX_PAIRS {
        let w = m + 1;
        let mut lcs = vec![0u32; (n + 1) * w];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * w + j] =
                    if am[i] == bm[j] { lcs[(i + 1) * w + j + 1] + 1 } else { lcs[(i + 1) * w + j].max(lcs[i * w + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if am[i] == bm[j] {
                pairs.push((i, j));
                i += 1;
                j += 1;
            } else if lcs[(i + 1) * w + j] >= lcs[i * w + j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    pairs.push((n, m));

    let mut pos: usize = pa[..start].iter().map(|p| p.chars().count()).sum();
    let mut changes = Vec::new();
    let (mut i, mut j) = (0, 0);
    for (pi, pj) in pairs {
        let old: Vec<char> = am[i..pi].concat().chars().collect();
        let now: Vec<char> = bm[j..pj].concat().chars().collect();
        if old != now {
            let mut head = 0;
            while head < old.len() && head < now.len() && old[head] == now[head] {
                head += 1;
            }
            let mut tail = 0;
            while tail < old.len() - head && tail < now.len() - head && old[old.len() - 1 - tail] == now[now.len() - 1 - tail] {
                tail += 1;
            }
            changes.push(Change { from: pos + head, to: pos + old.len() - tail, insert: now[head..now.len() - tail].iter().collect() });
        }
        pos += old.len() + if pi < n { am[pi].chars().count() } else { 0 };
        i = pi + 1;
        j = pj + 1;
    }
    changes
}

#[derive(Clone, Copy, PartialEq)]
pub enum Rewrite {
    Tidy,
    Edit,
}

impl Rewrite {
    fn title(self) -> &'static str {
        match self {
            Rewrite::Tidy => "Tidy Up",
            Rewrite::Edit => "Edit with AI",
        }
    }
}

/// The note rewritten by the chat model: (the text it was given, the result). Nothing is saved.
pub async fn rewrite(main: &Main, editor: &Editor, kind: Rewrite, instruction: &str) -> Result<(String, String), String> {
    let original = editor.shown_text();
    let (api, id, text, asked) = (main.store.api.clone(), editor.id.clone(), original.clone(), instruction.to_owned());
    let out = bg(async move {
        match kind {
            Rewrite::Tidy => api.tidy(&id, &text).await,
            Rewrite::Edit => api.edit_with_ai(&id, &asked, &text).await,
        }
    })
    .await;
    out.map(|text| (original, text)).map_err(|e| e.to_string())
}

/// Puts a rewrite into the note, if it hasn't changed meanwhile, with Undo in the toast.
pub fn apply(main: &Rc<Main>, editor: &Rc<Editor>, kind: Rewrite, original: &str, text: &str) -> bool {
    if !editor.replace_text(original, &changes_between(original, text)) {
        main.toast(&format!("The note changed meanwhile. Try {} again.", kind.title()));
        return false;
    }
    let e = editor.clone();
    main.toast_undo(if kind == Rewrite::Tidy { "Note tidied up" } else { "Note edited" }, move || e.run_undo(false));
    true
}

/// Tidy Up, or Edit with AI after asking what to change: a preview to apply.
pub fn rewrite_dialog(main: &Rc<Main>, editor: &Rc<Editor>, kind: Rewrite) {
    let dialog = adw::Dialog::builder().title(kind.title()).content_width(620).content_height(560).build();
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vexpand(true).build();

    // Asking (Edit with AI): what to change, with a few ideas that ask straight away.
    let ask = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(24).margin_bottom(24).margin_start(18).margin_end(18).build();
    let entry = gtk::Entry::builder().placeholder_text("Describe a change…").activates_default(false).build();
    ask.append(&entry);
    let ideas = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).column_spacing(6).row_spacing(6).build();
    for idea in ["Make it all bullets", "Add action items at the end", "Fix spelling and grammar"] {
        let chip = gtk::Button::builder().label(idea).css_classes(["pill", "chip"]).build();
        ideas.append(&chip);
        let e = entry.clone();
        chip.connect_clicked(move |_| {
            e.set_text(idea);
            e.emit_activate();
        });
    }
    ask.append(&ideas);
    stack.add_named(&ask, Some("ask"));

    let busy = adw::StatusPage::builder().title(if kind == Rewrite::Tidy { "Tidying Up…" } else { "Editing…" }).child(&adw::Spinner::builder().width_request(32).height_request(32).build()).build();
    stack.add_named(&busy, Some("busy"));
    let failed = adw::StatusPage::builder().icon_name("dialog-warning-symbolic").title("Couldn't Do That").build();
    stack.add_named(&failed, Some("error"));
    let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
    stack.add_named(&scroll, Some("preview"));

    let cancel = gtk::Button::with_label("Cancel");
    let again = gtk::Button::builder().label("Try Again").visible(false).build();
    let apply_button = gtk::Button::builder().label("Apply").css_classes(["suggested-action"]).sensitive(false).build();
    let bar = gtk::Box::builder().spacing(12).halign(gtk::Align::End).margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).build();
    bar.append(&cancel);
    bar.append(&again);
    bar.append(&apply_button);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&stack));
    view.add_bottom_bar(&bar);
    dialog.set_child(Some(&view));

    // What came back, to apply: (the text it was made from, the result).
    let result: Rc<std::cell::RefCell<Option<(String, String)>>> = Default::default();
    // Each run gets a number, so an answer for one that was cancelled or redone is dropped.
    let runs = Rc::new(std::cell::Cell::new(0u32));
    let run = {
        let (main, editor, stack, failed, scroll, apply_button, again, result, runs, dialog) =
            (main.clone(), editor.clone(), stack.clone(), failed.clone(), scroll.clone(), apply_button.clone(), again.clone(), result.clone(), runs.clone(), dialog.clone());
        Rc::new(move |asked: String| {
            let n = runs.get() + 1;
            runs.set(n);
            stack.set_visible_child_name("busy");
            apply_button.set_sensitive(false);
            again.set_visible(false);
            let (main, editor, stack, failed, scroll, apply_button, again, result, runs, dialog) =
                (main.clone(), editor.clone(), stack.clone(), failed.clone(), scroll.clone(), apply_button.clone(), again.clone(), result.clone(), runs.clone(), dialog.clone());
            glib::spawn_future_local(async move {
                let out = rewrite(&main, &editor, kind, &asked).await;
                if runs.get() != n {
                    return;
                }
                again.set_visible(true);
                match out {
                    Ok((original, text)) if original == text => {
                        main.toast(if kind == Rewrite::Tidy { "This note is already tidy" } else { "The AI left the note as it was" });
                        dialog.close();
                    }
                    Ok((original, text)) => {
                        scroll.set_child(Some(&styling::preview(&text)));
                        result.replace(Some((original, text)));
                        stack.set_visible_child_name("preview");
                        apply_button.set_sensitive(true);
                        apply_button.grab_focus();
                    }
                    Err(e) => {
                        failed.set_description(Some(&glib::markup_escape_text(&e)));
                        stack.set_visible_child_name("error");
                    }
                }
            });
        })
    };
    let asked = Rc::new(std::cell::RefCell::new(String::new()));
    {
        let (run, asked) = (run.clone(), asked.clone());
        entry.connect_activate(move |e| {
            let text = e.text().trim().to_owned();
            if !text.is_empty() {
                asked.replace(text.clone());
                run(text);
            }
        });
    }
    {
        let (run, asked) = (run.clone(), asked.clone());
        again.connect_clicked(move |_| run(asked.borrow().clone()));
    }
    {
        let (d, runs) = (dialog.clone(), runs.clone());
        cancel.connect_clicked(move |_| {
            runs.set(runs.get() + 1);
            d.close();
        });
    }
    {
        let (main, editor, d) = (main.clone(), editor.clone(), dialog.clone());
        apply_button.connect_clicked(move |_| {
            if let Some((original, text)) = result.take() {
                apply(&main, &editor, kind, &original, &text);
            }
            d.close();
        });
    }
    if kind == Rewrite::Tidy {
        run(String::new());
    } else {
        stack.set_visible_child_name("ask");
        let e = entry.clone();
        dialog.connect_map(move |_| {
            e.grab_focus();
        });
    }
    dialog.present(Some(&main.window));
}

/// A note's summary, as the note's Summary view: the saved one, or a new one the first time the
/// view is opened. `page` goes in the editor's view stack, `bar` under it in place of the format bar.
pub struct SummaryPane {
    pub page: gtk::Stack,
    pub bar: gtk::Box,
    scroll: gtk::ScrolledWindow,
    load: Rc<dyn Fn()>,
}

impl SummaryPane {
    pub fn new(main: &Rc<Main>, id: &str, title: &adw::WindowTitle) -> SummaryPane {
        let stack = gtk::Stack::builder().vexpand(true).build();
        let busy = adw::StatusPage::builder().title("Summarizing…").child(&adw::Spinner::builder().width_request(32).height_request(32).build()).build();
        stack.add_named(&busy, Some("busy"));
        let failed = adw::StatusPage::builder().icon_name("dialog-warning-symbolic").title("Couldn't Summarize").build();
        stack.add_named(&failed, Some("error"));
        let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        stack.add_named(&scroll, Some("summary"));
        let note = gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["dim-label"]).build();
        let redo = gtk::Button::with_label("Regenerate");
        let bar = gtk::Box::builder().spacing(12).margin_top(6).margin_bottom(6).margin_start(18).margin_end(12).css_classes(["toolbar"]).build();
        bar.append(&note);
        bar.append(&redo);

        let show = {
            let (stack, scroll, note, redo, title) = (stack.clone(), scroll.clone(), note.clone(), redo.clone(), title.clone());
            move |s: NoteSummary| {
                let Some(text) = s.summary else { return false };
                // Under the note's title, so the summary reads as the note's body.
                scroll.set_child(Some(&styling::preview(&format!("{}\n{text}", title.title()))));
                note.set_label(&if s.stale {
                    "The note has changed since this summary.".to_owned()
                } else {
                    format!("Made {}", super::lists::when(s.created_at.unwrap_or_else(super::store::now_ms)).to_lowercase())
                });
                note.set_css_classes(if s.stale { &["warning"] } else { &["dim-label"] });
                redo.set_label(if s.stale { "Update" } else { "Regenerate" });
                if s.stale {
                    redo.add_css_class("suggested-action");
                } else {
                    redo.remove_css_class("suggested-action");
                }
                stack.set_visible_child_name("summary");
                true
            }
        };
        let show = Rc::new(show);
        let make = {
            let (main, id, stack, failed, show, redo) = (main.clone(), id.to_owned(), stack.clone(), failed.clone(), show.clone(), redo.clone());
            Rc::new(move || {
                stack.set_visible_child_name("busy");
                redo.set_sensitive(false);
                let (main, id, stack, failed, show, redo) = (main.clone(), id.clone(), stack.clone(), failed.clone(), show.clone(), redo.clone());
                glib::spawn_future_local(async move {
                    let api = main.store.api.clone();
                    let out = bg(async move { api.summarize(&id).await }).await;
                    redo.set_sensitive(true);
                    match out {
                        Ok(s) => {
                            if !show(s) {
                                failed.set_description(Some("The summary service sent back nothing"));
                                stack.set_visible_child_name("error");
                            }
                        }
                        Err(e) => {
                            failed.set_description(Some(&glib::markup_escape_text(&e.to_string())));
                            stack.set_visible_child_name("error");
                        }
                    }
                });
            })
        };
        {
            let make = make.clone();
            redo.connect_clicked(move |_| make());
        }
        // Opening the view is the trigger: the saved summary, or a new one when there's none.
        let load = {
            let (main, id, redo) = (main.clone(), id.to_owned(), redo.clone());
            Rc::new(move || {
                if !redo.is_sensitive() {
                    return;
                }
                let (main, id, show, make) = (main.clone(), id.clone(), show.clone(), make.clone());
                glib::spawn_future_local(async move {
                    let api = main.store.api.clone();
                    let saved = bg(async move { api.summary(&id).await }).await;
                    if !saved.is_ok_and(|s| show(s)) {
                        make();
                    }
                });
            })
        };
        SummaryPane { page: stack, bar, scroll, load }
    }

    /// Fetches the summary, or makes one, unless one is being made.
    pub fn load(&self) {
        (self.load)()
    }

    /// The summary as shown, for drive.rs.
    pub fn text(&self) -> String {
        if self.page.visible_child_name().as_deref() != Some("summary") {
            return String::new();
        }
        self.scroll
            .child()
            .and_then(|c| c.downcast::<gtk::TextView>().ok())
            .map(|v| {
                let b = v.buffer();
                b.text(&b.start_iter(), &b.end_iter(), true).to_string()
            })
            .unwrap_or_default()
    }
}

/// The AI Prompt: a field at the cursor whose answer is written there, with the note as context.
pub fn prompt_popover(main: &Rc<Main>, editor: &Rc<Editor>) {
    let Some((view, rect)) = editor.cursor_rect() else { return };
    let popover = gtk::Popover::builder().has_arrow(true).position(gtk::PositionType::Bottom).build();
    let entry = gtk::Entry::builder().placeholder_text("Ask AI to write…").width_chars(36).primary_icon_name("starred-symbolic").build();
    let spinner = adw::Spinner::builder().visible(false).build();
    let row = gtk::Box::builder().spacing(8).build();
    row.append(&entry);
    row.append(&spinner);
    popover.set_child(Some(&row));
    popover.set_parent(&view);
    popover.set_pointing_to(Some(&rect));
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    let (main, editor, p, s) = (main.clone(), editor.clone(), popover.clone(), spinner.clone());
    entry.connect_activate(move |e| {
        let asked = e.text().trim().to_owned();
        if asked.is_empty() {
            p.popdown();
            return;
        }
        e.set_sensitive(false);
        s.set_visible(true);
        let (main, editor, p, e) = (main.clone(), editor.clone(), p.clone(), e.clone());
        glib::spawn_future_local(async move {
            if write_here(&main, &editor, &asked).await {
                p.popdown();
            } else {
                e.set_sensitive(true);
            }
        });
    });
    popover.popup();
    entry.grab_focus();
}

/// Asks the chat model to write at the cursor, and puts its answer there. True when it did.
pub async fn write_here(main: &Main, editor: &Rc<Editor>, asked: &str) -> bool {
    let text = editor.shown_text();
    let at = editor.cursor_offset();
    // The server counts positions as the PWA does, in UTF-16 code units.
    let at16: usize = text.chars().take(at).map(char::len_utf16).sum();
    let (api, id, prompt) = (main.store.api.clone(), editor.id.clone(), asked.to_owned());
    match bg(async move { api.write_with_ai(&id, &prompt, &text, at16).await }).await {
        Ok(written) if !written.trim().is_empty() => {
            editor.write_at_line(at, written.trim_end());
            true
        }
        Ok(_) => {
            main.toast("The AI wrote nothing");
            false
        }
        Err(e) => {
            main.toast(&e.to_string());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, changes: &[Change]) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let mut at = 0;
        for c in changes {
            out.extend(&chars[at..c.from]);
            out.push_str(&c.insert);
            at = c.to;
        }
        out.extend(&chars[at..]);
        out
    }

    #[test]
    fn smallest_edits_get_there() {
        let a = "# Groceries\nmilk\neggs 🥚\nbread\n";
        let b = "# Groceries\n\n## To buy\n- milk\n- eggs 🥚\n- bread\n";
        let changes = changes_between(a, b);
        assert_eq!(apply(a, &changes), b);
        // The title line is left alone.
        assert!(changes.iter().all(|c| c.from >= "# Groceries\n".chars().count()));
        assert!(changes_between(a, a).is_empty());
        assert_eq!(apply("x", &changes_between("x", "")), "");
        assert_eq!(apply("", &changes_between("", "new\nlines")), "new\nlines");
    }
}
