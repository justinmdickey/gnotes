//! The "/" menu and emoji by name, like web/src/lib/slash.ts and web/src/lib/emoji.ts: type a slash
//! at the start of a line or after a space for styles and things to add, or ":" and a few letters for
//! an emoji. Typing more narrows the list; the slash or colon and what was typed go away on pick.
//! Typing a whole ":name:" turns it into the emoji straight away.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// A line style: the prefix set_block takes.
    Block(&'static str),
    CodeBlock,
    Table,
    Board,
    /// Opens the emoji list at the cursor.
    Emoji,
    Photo,
    Voice,
    Prompt,
    Tidy,
    /// Text to type, e.g. an emoji.
    Text(String),
}

#[derive(Clone)]
pub struct Item {
    pub label: String,
    /// A symbolic icon's name, or the emoji itself.
    pub glyph: String,
    pub detail: String,
    pub pick: Pick,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Slash,
    Emoji,
}

/// What the text before the cursor on its line asks for: the kind, how many characters back the
/// trigger starts ("/" or ":"), and what's been typed after it.
#[cfg(test)]
fn trigger(before: &str) -> Option<(Kind, usize, String)> {
    trigger_with(before, 2)
}

/// `trigger`, with the fewest letters after ":" that open the emoji list (0 once it was asked for).
pub fn trigger_with(before: &str, emoji_letters: usize) -> Option<(Kind, usize, String)> {
    let chars: Vec<char> = before.chars().collect();
    for (i, &c) in chars.iter().enumerate().rev() {
        let after: String = chars[i + 1..].iter().collect();
        let starts_word = i == 0 || chars[i - 1].is_whitespace() || (c == ':' && chars[i - 1] == '(');
        match c {
            '/' if starts_word => {
                // "/word" or "/two words".
                let ok = after.split(' ').count() <= 2 && after.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == ' ');
                return ok.then(|| (Kind::Slash, chars.len() - i, after));
            }
            ':' if starts_word => {
                let ok = after.chars().count() >= emoji_letters && after.chars().all(|ch| ch.is_alphanumeric() || "_+-".contains(ch));
                return ok.then(|| (Kind::Emoji, chars.len() - i, after));
            }
            _ if c.is_whitespace() && after.contains(' ') => return None,
            _ => {}
        }
        if chars.len() - i > 40 {
            return None;
        }
    }
    None
}

/// The emoji for a whole ":name:" just typed, and how many characters it replaces.
pub fn emoji_on_colon(before: &str) -> Option<(&'static str, usize)> {
    let body = before.strip_suffix(':')?;
    let start = body.rfind(':')?;
    let name = &body[start + 1..];
    let ok_before = start == 0 || body[..start].ends_with(|c: char| c.is_whitespace() || c == '(');
    if !ok_before || name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || "_+-".contains(c)) {
        return None;
    }
    let emoji = emojis::get_by_shortcode(name)?;
    Some((emoji.as_str(), name.chars().count() + 2))
}

/// Other words for the AI items: typing the start of one finds it too.
const ALIASES: &[(&str, &[&str])] = &[("Tidy Up", &["clean up", "format", "improve"]), ("AI Prompt", &["ai", "prompt", "write"])];

pub fn slash_items(query: &str, ai: bool) -> Vec<Item> {
    let item = |label: &str, glyph: &str, pick: Pick| Item { label: label.into(), glyph: glyph.into(), detail: String::new(), pick };
    let mut all = vec![
        item("Title", "format-text-plaintext-symbolic", Pick::Block("# ")),
        item("Heading", "format-text-plaintext-symbolic", Pick::Block("## ")),
        item("Subheading", "format-text-plaintext-symbolic", Pick::Block("### ")),
        item("Body", "format-text-plaintext-symbolic", Pick::Block("")),
        item("Checklist", "checkbox-checked-symbolic", Pick::Block("- [ ] ")),
        item("Bulleted List", "view-list-bullet-symbolic", Pick::Block("- ")),
        item("Numbered List", "view-list-ordered-symbolic", Pick::Block("1. ")),
        item("Quote", "format-indent-more-symbolic", Pick::Block("> ")),
        item("Code Block", "utilities-terminal-symbolic", Pick::CodeBlock),
        item("Table", "x-office-spreadsheet-symbolic", Pick::Table),
        item("Kanban Board", "view-grid-symbolic", Pick::Board),
        item("Emoji", "face-smile-symbolic", Pick::Emoji),
        item("Photo", "camera-photo-symbolic", Pick::Photo),
        item("Voice Memo", "audio-input-microphone-symbolic", Pick::Voice),
    ];
    if ai {
        all.push(item("AI Prompt", "starred-symbolic", Pick::Prompt));
        all.push(item("Tidy Up", "edit-clear-all-symbolic", Pick::Tidy));
    }
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return all;
    }
    // Names whose words start with what's typed, best first; then the AI items under their other words.
    let mut found: Vec<(usize, Item)> = Vec::new();
    for (i, it) in all.into_iter().enumerate() {
        let name = it.label.to_lowercase();
        let rank = if name.starts_with(&q) {
            0
        } else if name.split(' ').any(|w| w.starts_with(&q)) {
            1
        } else if ALIASES.iter().any(|(l, words)| *l == it.label && words.iter().any(|w| w.starts_with(&q))) {
            2
        } else {
            continue;
        };
        found.push((rank * 100 + i, it));
    }
    found.sort_by_key(|(r, _)| *r);
    found.into_iter().map(|(_, it)| it).collect()
}

/// Emoji whose names start with what's typed, then have a word that does; at most 60.
pub fn emoji_items(query: &str) -> Vec<Item> {
    let q = query.to_lowercase();
    let mut found: Vec<(u8, usize, Item)> = Vec::new();
    for (order, e) in emojis::iter().enumerate() {
        for name in e.shortcodes() {
            let rank = if name.starts_with(&q) {
                0
            } else if name.contains(&format!("_{q}")) {
                1
            } else {
                continue;
            };
            found.push((rank, order, Item { label: format!(":{name}:"), glyph: e.as_str().into(), detail: e.name().into(), pick: Pick::Text(e.as_str().into()) }));
            break;
        }
    }
    found.sort_by_key(|(r, o, _)| (*r, *o));
    found.into_iter().take(60).map(|(_, _, it)| it).collect()
}

/// The list itself, a popover at the cursor that leaves the focus in the text.
pub struct Completer {
    popover: gtk::Popover,
    list: gtk::ListBox,
    scroll: gtk::ScrolledWindow,
    items: RefCell<Vec<Item>>,
    /// How many characters before the cursor the trigger starts.
    pub back: Cell<usize>,
    pub kind: Cell<Kind>,
    /// Called with a picked item.
    on_pick: RefCell<Option<Box<dyn Fn(Pick)>>>,
}

impl Completer {
    pub fn new(parent: &impl IsA<gtk::Widget>) -> Rc<Completer> {
        let list = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).can_focus(false).build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(320)
            .propagate_natural_height(true)
            .width_request(260)
            .child(&list)
            .build();
        let popover = gtk::Popover::builder()
            .autohide(false)
            .can_focus(false)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Start)
            .child(&scroll)
            .css_classes(["completer"])
            .build();
        popover.set_parent(parent);
        let c = Rc::new(Completer {
            popover,
            list: list.clone(),
            scroll,
            items: Default::default(),
            back: Cell::new(0),
            kind: Cell::new(Kind::Slash),
            on_pick: Default::default(),
        });
        let weak = Rc::downgrade(&c);
        list.connect_row_activated(move |_, row| {
            if let Some(c) = weak.upgrade() {
                c.list.select_row(Some(row));
                c.pick();
            }
        });
        c
    }

    pub fn on_pick(&self, f: impl Fn(Pick) + 'static) {
        self.on_pick.replace(Some(Box::new(f)));
    }

    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    pub fn show(&self, at: &gtk::gdk::Rectangle, kind: Kind, back: usize, items: Vec<Item>) {
        if items.is_empty() {
            self.hide();
            return;
        }
        self.kind.set(kind);
        self.back.set(back);
        self.list.remove_all();
        for it in &items {
            let b = gtk::Box::builder().spacing(10).build();
            if it.glyph.ends_with("-symbolic") {
                b.append(&gtk::Image::from_icon_name(&it.glyph));
            } else {
                b.append(&gtk::Label::builder().label(&it.glyph).css_classes(["emoji-glyph"]).build());
            }
            b.append(&gtk::Label::builder().label(&it.label).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
            let row = gtk::ListBoxRow::builder().child(&b).can_focus(false).build();
            if !it.detail.is_empty() {
                row.set_tooltip_text(Some(&it.detail));
            }
            self.list.append(&row);
        }
        self.items.replace(items);
        self.list.select_row(self.list.row_at_index(0).as_ref());
        self.popover.set_pointing_to(Some(at));
        if !self.popover.is_visible() {
            self.popover.popup();
        }
    }

    pub fn hide(&self) {
        if self.popover.is_visible() {
            self.popover.popdown();
        }
    }

    /// Up or down the list.
    pub fn step(&self, by: i32) {
        let n = self.items.borrow().len() as i32;
        if n == 0 {
            return;
        }
        let at = self.list.selected_row().map_or(0, |r| r.index());
        let next = (at + by).rem_euclid(n);
        if let Some(row) = self.list.row_at_index(next) {
            self.list.select_row(Some(&row));
            // Keep it in view.
            let adj = self.scroll.vadjustment();
            let Some(b) = row.compute_bounds(&self.list) else { return };
            let (y, h) = (b.y() as f64, b.height() as f64);
            if y < adj.value() {
                adj.set_value(y);
            } else if y + h > adj.value() + adj.page_size() {
                adj.set_value(y + h - adj.page_size());
            }
        }
    }

    /// Picks the highlighted item.
    pub fn pick(&self) {
        let Some(row) = self.list.selected_row() else { return };
        let Some(item) = self.items.borrow().get(row.index() as usize).cloned() else { return };
        self.hide();
        if let Some(f) = &*self.on_pick.borrow() {
            f(item.pick);
        }
    }

    /// Picks the first item named `label`, for tests.
    pub fn pick_named(&self, label: &str) -> bool {
        let Some(i) = self.items.borrow().iter().position(|it| it.label == label) else { return false };
        if let Some(row) = self.list.row_at_index(i as i32) {
            self.list.select_row(Some(&row));
        }
        self.pick();
        true
    }
}

impl Drop for Completer {
    fn drop(&mut self) {
        self.popover.unparent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers() {
        assert_eq!(trigger("/"), Some((Kind::Slash, 1, String::new())));
        assert_eq!(trigger("text /che"), Some((Kind::Slash, 4, "che".into())));
        assert_eq!(trigger("and/or"), None);
        assert_eq!(trigger("at 10:30"), None);
        assert_eq!(trigger("I'm :ta"), Some((Kind::Emoji, 3, "ta".into())));
        assert_eq!(trigger(":t"), None);
        assert_eq!(emoji_on_colon("lunch :taco:"), Some(("🌮", 6)));
        assert_eq!(emoji_on_colon("10:30:"), None);
        assert!(emoji_items("tac").iter().any(|i| i.glyph == "🌮"));
        assert_eq!(slash_items("che", false)[0].label, "Checklist");
        assert_eq!(slash_items("kanban", false)[0].label, "Kanban Board");
        assert_eq!(slash_items("board", false)[0].label, "Kanban Board");
        assert_eq!(slash_items("clean", true)[0].label, "Tidy Up");
        assert!(slash_items("ai", false).is_empty());
    }
}
