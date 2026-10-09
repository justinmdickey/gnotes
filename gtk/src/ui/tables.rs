//! Tables and kanban boards in notes, like web/src/lib/tables.ts and web/src/lib/kanban.ts. The note
//! keeps them as plain Markdown (GFM pipe tables; a ```kanban block of "## Column" headings and "- card"
//! lines); the editor draws them as widgets over their text, and edits go back as the smallest change.

use std::{
    cell::{Cell as Flag, RefCell},
    rc::Rc,
};

use adw::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Look {
    Bold,
    Italic,
    Strike,
    Code,
    Link,
}

/// Inline Markdown as it reads: the text without its marks, and which bytes of that are bold,
/// italic, struck, code or a link. `**[[nimbus|the server]]**` reads "the server", bold and linked.
fn render(src: &str) -> (String, Vec<(usize, usize, Look)>) {
    fn walk(src: &str, out: &mut String, looks: &mut Vec<(usize, usize, Look)>) {
        let mut rest = src;
        while let Some(c) = rest.chars().next() {
            // A pair of marks around something, as (open, close, look).
            let pairs = [("**", "**", Look::Bold), ("~~", "~~", Look::Strike), ("`", "`", Look::Code), ("*", "*", Look::Italic)];
            let mut done = false;
            for (open, close, look) in pairs {
                if let Some(after) = rest.strip_prefix(open)
                    && let Some(end) = after.find(close).filter(|&e| e > 0)
                {
                    let start = out.len();
                    if look == Look::Code {
                        out.push_str(&after[..end]);
                    } else {
                        walk(&after[..end], out, looks);
                    }
                    looks.push((start, out.len(), look));
                    rest = &after[end + close.len()..];
                    done = true;
                    break;
                }
            }
            if done {
                continue;
            }
            // [[Note]] or [[Note|what it says]]
            if let Some(after) = rest.strip_prefix("[[")
                && let Some(end) = after.find("]]")
            {
                let inner = &after[..end];
                let shown = inner.split_once('|').map_or(inner, |(_, alias)| alias);
                let start = out.len();
                out.push_str(shown.trim());
                looks.push((start, out.len(), Look::Link));
                rest = &after[end + 2..];
                continue;
            }
            // [text](address)
            if let Some(after) = rest.strip_prefix('[')
                && let Some(close) = after.find("](")
                && let Some(end) = after[close + 2..].find(')')
            {
                let start = out.len();
                walk(&after[..close], out, looks);
                looks.push((start, out.len(), Look::Link));
                rest = &after[close + 2 + end + 1..];
                continue;
            }
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    let mut out = String::new();
    let mut looks = Vec::new();
    walk(src, &mut out, &mut looks);
    (out, looks)
}

/// An entry that shows its Markdown formatted, and as written while it has focus to edit.
#[derive(Clone)]
struct RichEntry {
    entry: gtk::Entry,
    raw: Rc<RefCell<String>>,
    editing: Rc<Flag<bool>>,
    /// Set while the text is swapped between the two, so that isn't taken as typing.
    quiet: Rc<Flag<bool>>,
}

impl RichEntry {
    fn new(entry: gtk::Entry, raw: &str) -> RichEntry {
        let rich = RichEntry { entry, raw: Rc::new(RefCell::new(raw.to_owned())), editing: Default::default(), quiet: Default::default() };
        rich.show_formatted();
        let focus = gtk::EventControllerFocus::new();
        let r = rich.clone();
        focus.connect_enter(move |_| {
            r.editing.set(true);
            r.swap(&r.raw.borrow().clone(), None);
            // Focus selects all of an entry's text; a card or cell is for typing in, so the cursor
            // goes to the end instead, unless a click has put it somewhere.
            let e = r.entry.clone();
            gtk::glib::idle_add_local_once(move || {
                if e.selection_bounds().is_some_and(|(a, b)| a == 0 && b == e.text_length() as i32) {
                    e.set_position(-1);
                }
            });
        });
        let r = rich.clone();
        focus.connect_leave(move |_| {
            r.raw.replace(r.entry.text().to_string());
            r.editing.set(false);
            r.show_formatted();
        });
        rich.entry.add_controller(focus);
        rich
    }

    /// The Markdown, as written.
    fn text(&self) -> String {
        if self.editing.get() { self.entry.text().to_string() } else { self.raw.borrow().clone() }
    }

    /// A change the user made, not a swap.
    fn typed(&self) -> bool {
        !self.quiet.get()
    }

    fn show_formatted(&self) {
        let (shown, looks) = render(&self.raw.borrow());
        let attrs = gtk::pango::AttrList::new();
        for (s, e, look) in looks {
            let mut a: gtk::pango::Attribute = match look {
                Look::Bold => gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold).into(),
                Look::Italic => gtk::pango::AttrInt::new_style(gtk::pango::Style::Italic).into(),
                Look::Strike => gtk::pango::AttrInt::new_strikethrough(true).into(),
                Look::Code => gtk::pango::AttrString::new_family(&super::styling::monospace()).into(),
                Look::Link => {
                    let c = super::styling::accent();
                    let color = |v: f32| (v * 65535.0) as u16;
                    let mut fg: gtk::pango::Attribute = gtk::pango::AttrColor::new_foreground(color(c.red()), color(c.green()), color(c.blue())).into();
                    fg.set_start_index(s as u32);
                    fg.set_end_index(e as u32);
                    attrs.insert(fg);
                    gtk::pango::AttrInt::new_underline(gtk::pango::Underline::Single).into()
                }
            };
            a.set_start_index(s as u32);
            a.set_end_index(e as u32);
            attrs.insert(a);
        }
        self.swap(&shown, Some(&attrs));
    }

    fn swap(&self, text: &str, attrs: Option<&gtk::pango::AttrList>) {
        self.quiet.set(true);
        if self.entry.text() != text {
            self.entry.set_text(text);
        }
        self.entry.set_attributes(attrs.unwrap_or(&gtk::pango::AttrList::new()));
        self.quiet.set(false);
    }
}

/// A new 3×2 table, as the "/" menu puts it in.
pub const TABLE_TEMPLATE: &str = "| Column 1 | Column 2 | Column 3 |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |";

/// A new board with To do, Doing and Done, ready for its first card.
pub const BOARD_TEMPLATE: &str = "```kanban\n## To do\n- \n\n## Doing\n\n## Done\n```";

/// One cell: its text, unescaped, and where that is in the table's text (characters, trimmed).
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub text: String,
    pub from: usize,
    pub to: usize,
    /// Everything between its pipes, spaces and all, for filling an empty cell.
    pub seg: (usize, usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub header: Vec<Cell>,
    /// The `---` row's cells, kept as written (alignment).
    pub delimiter: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

/// Splits one table line at its unescaped pipes; `offset` is where the line starts (characters).
fn parse_row(line: &str, offset: usize) -> Vec<Cell> {
    let chars: Vec<char> = line.chars().collect();
    let mut pipes = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 1;
        } else if chars[i] == '|' {
            pipes.push(i);
        }
        i += 1;
    }
    // Leading and trailing pipes are optional.
    let mut bounds = Vec::new();
    let first = chars.iter().position(|c| !c.is_whitespace()).unwrap_or(0);
    let last = chars.iter().rposition(|c| !c.is_whitespace()).map_or(0, |p| p + 1);
    let mut start = if pipes.first() == Some(&first) { first + 1 } else { first };
    let from = start;
    for &p in pipes.iter().filter(|&&p| p >= from) {
        bounds.push((start, p));
        start = p + 1;
    }
    if start < last {
        bounds.push((start, last));
    }
    bounds
        .into_iter()
        .map(|(a, b)| {
            let mut s = a;
            while s < b && chars[s].is_whitespace() {
                s += 1;
            }
            let mut e = b;
            while e > s && chars[e - 1].is_whitespace() {
                e -= 1;
            }
            let raw: String = chars[s..e].iter().collect();
            Cell { text: raw.replace("\\|", "|"), from: offset + s, to: offset + e, seg: (offset + a, offset + b) }
        })
        .collect()
}

pub fn parse_table(text: &str) -> Option<Table> {
    let mut at = 0;
    let mut lines = Vec::new();
    for line in text.split('\n') {
        lines.push((line, at));
        at += line.chars().count() + 1;
    }
    let lines: Vec<(&str, usize)> = lines.into_iter().filter(|(l, _)| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return None;
    }
    let header = parse_row(lines[0].0, lines[0].1);
    let delimiter = parse_row(lines[1].0, lines[1].1).into_iter().map(|c| c.text).collect();
    let rows = lines[2..].iter().map(|(l, o)| parse_row(l, *o)).collect();
    Some(Table { header, delimiter, rows })
}

/// A cell's text as written in Markdown: pipes escaped, one line.
pub fn escape_cell(text: &str) -> String {
    text.replace('\n', " ").replace('|', "\\|")
}

/// The table written out, `| a | b |` rows, each row as wide as the header.
pub fn table_markdown(header: &[String], delimiter: &[String], rows: &[Vec<String>]) -> String {
    let n = header.len().max(1);
    let line = |cells: &[String]| {
        let mut out = String::from("|");
        for i in 0..n {
            let c = cells.get(i).map(|c| escape_cell(c)).unwrap_or_default();
            out.push_str(&format!(" {c} |"));
        }
        out
    };
    let delim: Vec<String> = (0..n).map(|i| delimiter.get(i).cloned().filter(|d| !d.is_empty()).unwrap_or_else(|| "---".into())).collect();
    let mut out = vec![line(header), format!("|{}", delim.iter().map(|d| format!(" {d} |")).collect::<String>())];
    out.extend(rows.iter().map(|r| line(r)));
    out.join("\n")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub text: String,
    /// None for a plain card; for a task, whether it's done.
    pub done: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub title: String,
    pub cards: Vec<Card>,
}

/// A ```kanban block's columns and cards. Lines that are neither stay in the text but aren't drawn.
pub fn parse_board(text: &str) -> Vec<Column> {
    let mut columns: Vec<Column> = Vec::new();
    let lines: Vec<&str> = text.split('\n').collect();
    let inner = if lines.len() > 1 && lines.last().is_some_and(|l| l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~")) {
        &lines[1..lines.len() - 1]
    } else {
        &lines[1.min(lines.len())..]
    };
    for line in inner {
        let t = line.trim_start();
        if let Some(title) = t.strip_prefix("## ").or_else(|| (t == "##").then_some("")) {
            columns.push(Column { title: title.to_owned(), cards: Vec::new() });
            continue;
        }
        let Some(col) = columns.last_mut() else { continue };
        let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| (t == "-").then_some("")) else { continue };
        let (done, text) = if let Some(r) = rest.strip_prefix("[ ]") {
            (Some(false), r.trim_start())
        } else if let Some(r) = rest.strip_prefix("[x]").or_else(|| rest.strip_prefix("[X]")) {
            (Some(true), r.trim_start())
        } else {
            (None, rest)
        };
        col.cards.push(Card { text: text.to_owned(), done });
    }
    columns
}

pub fn board_markdown(columns: &[Column]) -> String {
    let mut out = vec!["```kanban".to_owned()];
    for (i, col) in columns.iter().enumerate() {
        if i > 0 {
            out.push(String::new());
        }
        out.push(format!("## {}", col.title));
        for c in &col.cards {
            let text = c.text.replace('\n', " ");
            out.push(match c.done {
                None => format!("- {text}"),
                Some(false) => format!("- [ ] {text}"),
                Some(true) => format!("- [x] {text}"),
            });
        }
    }
    out.push("```".into());
    out.join("\n")
}

/// What a block's widget asks the editor to do to its Markdown.
pub enum Change {
    /// A table cell's new text: row 0 is the header.
    Cell { row: usize, col: usize, text: String },
    /// The block's whole new text, put in as the smallest edits.
    Rewrite(String),
    /// Show the Markdown itself, cursor at the block's start.
    EditText,
}

pub type OnChange = Rc<dyn Fn(Change)>;

/// The grid for a table, with a cell entry per cell.
pub fn table_widget(table: &Table, editable: bool, on_change: OnChange) -> gtk::Widget {
    let grid = gtk::Grid::builder().column_homogeneous(true).css_classes(["note-table"]).build();
    let header: Vec<String> = table.header.iter().map(|c| c.text.clone()).collect();
    let rows: Vec<Vec<String>> = table.rows.iter().map(|r| r.iter().map(|c| c.text.clone()).collect()).collect();
    let n = header.len();
    let mut entries: Vec<Vec<gtk::Entry>> = Vec::new();
    for (r, row) in std::iter::once(&table.header).chain(table.rows.iter()).enumerate() {
        let mut line = Vec::new();
        for c in 0..n {
            let cell = row.get(c);
            let entry = gtk::Entry::builder()
                .has_frame(false)
                .editable(editable)
                .hexpand(true)
                .css_classes(if r == 0 { vec!["table-cell", "table-head"] } else { vec!["table-cell"] })
                .build();
            let rich = RichEntry::new(entry.clone(), cell.map(|c| c.text.as_str()).unwrap_or_default());
            let on = on_change.clone();
            entry.connect_changed(move |e| {
                if rich.typed() {
                    on(Change::Cell { row: r, col: c, text: e.text().to_string() })
                }
            });
            grid.attach(&entry, c as i32, r as i32, 1, 1);
            line.push(entry);
        }
        entries.push(line);
    }
    // Tab and Enter move between cells.
    let all: Vec<gtk::Entry> = entries.iter().flatten().cloned().collect();
    for (i, e) in all.iter().enumerate() {
        let next = all.get(i + n).cloned();
        e.connect_activate(move |_| {
            if let Some(n) = &next {
                n.grab_focus();
            }
        });
    }
    let menu = gtk::gio::Menu::new();
    for (label, action) in [("Add Row", "row"), ("Add Column", "col"), ("Remove Last Row", "unrow"), ("Remove Last Column", "uncol"), ("Edit as Text", "text")] {
        menu.append(Some(label), Some(&format!("table.{action}")));
    }
    let group = gtk::gio::SimpleActionGroup::new();
    for action in ["row", "col", "unrow", "uncol", "text"] {
        let a = gtk::gio::SimpleAction::new(action, None);
        let (on, header, rows, delim) = (on_change.clone(), header.clone(), rows.clone(), table.delimiter.clone());
        a.connect_activate(move |_, _| {
            let (mut header, mut rows, mut delim) = (header.clone(), rows.clone(), delim.clone());
            match action {
                "row" => rows.push(vec![String::new(); header.len()]),
                "col" => {
                    header.push(format!("Column {}", header.len() + 1));
                    delim.push("---".into());
                    for r in &mut rows {
                        r.resize(header.len(), String::new());
                    }
                }
                "unrow" => {
                    rows.pop();
                }
                "uncol" if header.len() > 1 => {
                    header.pop();
                    delim.truncate(header.len());
                    for r in &mut rows {
                        r.truncate(header.len());
                    }
                }
                "text" => return on(Change::EditText),
                _ => return,
            }
            on(Change::Rewrite(table_markdown(&header, &delim, &rows)));
        });
        group.add_action(&a);
    }
    let more = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .menu_model(&menu)
        .css_classes(["flat", "circular", "block-menu"])
        .valign(gtk::Align::Start)
        .tooltip_text("Table")
        .sensitive(editable)
        .build();
    more.insert_action_group("table", Some(&group));
    let wrap = gtk::Box::builder().spacing(4).css_classes(["note-block"]).build();
    let frame = gtk::Frame::builder().child(&grid).hexpand(true).css_classes(["table-frame"]).build();
    // A table wider than the note scrolls sideways, rather than widening the note.
    let scroll = gtk::ScrolledWindow::builder()
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .propagate_natural_height(true)
        .child(&frame)
        .build();
    wrap.append(&scroll);
    wrap.append(&more);
    wrap.upcast()
}

/// The board: columns of cards, which can be edited, ticked, added and dragged between columns.
pub fn board_widget(columns: &[Column], editable: bool, on_change: OnChange) -> gtk::Widget {
    // The board as it is now: a card typed in stays as typed while the next one is edited, so
    // each change is made to the last one, not to the board as it was drawn.
    let cols = Rc::new(RefCell::new(columns.to_vec()));
    let rewrite = {
        let on = on_change.clone();
        let cols = cols.clone();
        Rc::new(move |f: &dyn Fn(&mut Vec<Column>)| {
            let mut next = cols.borrow().clone();
            f(&mut next);
            if next == *cols.borrow() {
                return;
            }
            cols.replace(next.clone());
            on(Change::Rewrite(board_markdown(&next)));
        })
    };
    let row = gtk::Box::builder().spacing(10).homogeneous(true).css_classes(["note-board"]).build();
    for (ci, col) in columns.iter().enumerate() {
        let lane = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).width_request(170).css_classes(["board-column", "card"]).build();
        let title = gtk::Entry::builder().text(&col.title).has_frame(false).editable(editable).css_classes(["board-title", "heading"]).build();
        let rw = rewrite.clone();
        title.connect_activate(move |e| {
            let t = e.text().to_string();
            rw(&|c| c[ci].title = t.clone());
        });
        let rw = rewrite.clone();
        let focus = gtk::EventControllerFocus::new();
        let t2 = title.clone();
        focus.connect_leave(move |_| {
            let t = t2.text().to_string();
            rw(&|c| c[ci].title = t.clone());
        });
        title.add_controller(focus);
        lane.append(&title);
        for (k, card) in col.cards.iter().enumerate() {
            let line = gtk::Box::builder().spacing(6).css_classes(["board-card"]).build();
            if let Some(done) = card.done {
                let check = gtk::CheckButton::builder().active(done).sensitive(editable).build();
                let rw = rewrite.clone();
                check.connect_toggled(move |c| {
                    let on = c.is_active();
                    rw(&|cols| cols[ci].cards[k].done = Some(on));
                });
                line.append(&check);
            }
            let entry = gtk::Entry::builder().has_frame(false).editable(editable).hexpand(true).build();
            let rich = RichEntry::new(entry.clone(), &card.text);
            let rw = rewrite.clone();
            let r = rich.clone();
            entry.connect_activate(move |_| {
                let t = r.text();
                rw(&|cols| cols[ci].cards[k].text = t.clone());
            });
            let rw = rewrite.clone();
            let focus = gtk::EventControllerFocus::new();
            focus.connect_leave(move |_| {
                let t = rich.text();
                rw(&|cols| cols[ci].cards[k].text = t.clone());
            });
            entry.add_controller(focus);
            line.append(&entry);
            if editable {
                let remove = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["flat", "circular", "card-remove"]).tooltip_text("Remove Card").build();
                let rw = rewrite.clone();
                remove.connect_clicked(move |_| rw(&|cols| {
                    cols[ci].cards.remove(k);
                }));
                line.append(&remove);
                // Cards drag between columns.
                let src = gtk::DragSource::builder().actions(gtk::gdk::DragAction::MOVE).build();
                src.connect_prepare(move |_, _, _| Some(gtk::gdk::ContentProvider::for_value(&format!("card:{ci}:{k}").to_value())));
                line.add_controller(src);
            }
            lane.append(&line);
        }
        if editable {
            let add = gtk::Button::builder().child(&adw::ButtonContent::builder().icon_name("list-add-symbolic").label("Add Card").build()).css_classes(["flat"]).build();
            let rw = rewrite.clone();
            add.connect_clicked(move |_| rw(&|cols| cols[ci].cards.push(Card { text: "New card".into(), done: None })));
            lane.append(&add);
            let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
            let rw = rewrite.clone();
            drop.connect_drop(move |_, value, _, _| {
                let Ok(v) = value.get::<String>() else { return false };
                let mut parts = v.strip_prefix("card:").unwrap_or_default().split(':').filter_map(|p| p.parse::<usize>().ok());
                let (Some(from), Some(k)) = (parts.next(), parts.next()) else { return false };
                rw(&|cols| {
                    if from < cols.len() && k < cols[from].cards.len() {
                        let card = cols[from].cards.remove(k);
                        cols[ci].cards.push(card);
                    }
                });
                true
            });
            lane.add_controller(drop);
        }
        row.append(&lane);
    }
    let menu = gtk::gio::Menu::new();
    menu.append(Some("Add Column"), Some("board.col"));
    menu.append(Some("Edit as Text"), Some("board.text"));
    let group = gtk::gio::SimpleActionGroup::new();
    let a = gtk::gio::SimpleAction::new("col", None);
    let rw = rewrite.clone();
    a.connect_activate(move |_, _| rw(&|cols| cols.push(Column { title: "New column".into(), cards: Vec::new() })));
    group.add_action(&a);
    let a = gtk::gio::SimpleAction::new("text", None);
    let on = on_change.clone();
    a.connect_activate(move |_, _| on(Change::EditText));
    group.add_action(&a);
    let more = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .menu_model(&menu)
        .css_classes(["flat", "circular", "block-menu"])
        .valign(gtk::Align::Start)
        .tooltip_text("Board")
        .sensitive(editable)
        .build();
    more.insert_action_group("board", Some(&group));
    let scroll = gtk::ScrolledWindow::builder().vscrollbar_policy(gtk::PolicyType::Never).hexpand(true).propagate_natural_height(true).child(&row).build();
    let wrap = gtk::Box::builder().spacing(4).css_classes(["note-block"]).build();
    wrap.append(&scroll);
    wrap.append(&more);
    wrap.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_read_without_their_marks() {
        let (text, looks) = render("**[[nimbus — the main|nimbus]]** and *a* [site](https://x.y) `ls`");
        assert_eq!(text, "nimbus and a site ls");
        assert!(looks.contains(&(0, 6, Look::Bold)) && looks.contains(&(0, 6, Look::Link)));
        assert!(looks.contains(&(11, 12, Look::Italic)) && looks.contains(&(18, 20, Look::Code)));
        // Underscores inside words stay.
        assert_eq!(render("snake_case_name").0, "snake_case_name");
    }

    #[test]
    fn tables_round_trip() {
        let t = parse_table("| Name | Qty |\n| --- | ---: |\n| milk \\| oat | 2 |\n| eggs |").unwrap();
        assert_eq!(t.header.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["Name", "Qty"]);
        assert_eq!(t.delimiter, ["---", "---:"]);
        assert_eq!(t.rows[0][0].text, "milk | oat");
        assert_eq!(t.rows[1].len(), 1);
        // Where a cell is, so typing changes only it.
        let text = "| Name | Qty |\n| --- | --- |\n| milk | 2 |";
        let t = parse_table(text).unwrap();
        let chars: Vec<char> = text.chars().collect();
        let c = &t.rows[0][1];
        assert_eq!(chars[c.from..c.to].iter().collect::<String>(), "2");
        let md = table_markdown(&["A".into(), "B".into()], &["---".into()], &[vec!["x".into()]]);
        assert_eq!(md, "| A | B |\n| --- | --- |\n| x |  |");
    }

    #[test]
    fn boards_round_trip() {
        let cols = parse_board(BOARD_TEMPLATE);
        assert_eq!(cols.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["To do", "Doing", "Done"]);
        assert_eq!(cols[0].cards, [Card { text: String::new(), done: None }]);
        let text = "```kanban\n## Now\n- [x] Ship it\n- Plain\n```";
        assert_eq!(board_markdown(&parse_board(text)), text);
    }
}
