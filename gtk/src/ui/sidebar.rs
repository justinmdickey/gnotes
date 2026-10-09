//! The sidebar, like the PWA's desktop one (web/src/Sidebar.svelte): Search, Recent, your notebooks
//! with their notes, what's shared with you, Trash, and the connection at the bottom.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::{Rc, Weak},
};

use adw::prelude::*;
use gtk::gio;

use super::{
    dialogs,
    window::{Main, Place},
};
use crate::{
    api::{Tree, TreeNote, TreeNotebook},
    sync::Status,
};

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Ask,
    Search,
    Recent,
    Heading,
    Notebook(String),
    Note(String),
    Shared,
    Trash,
}

pub struct Sidebar {
    pub widget: gtk::Widget,
    main: Weak<Main>,
    list: gtk::ListBox,
    rows: RefCell<Vec<Row>>,
    /// Notebooks showing what's inside. Notebooks start folded.
    open: RefCell<HashSet<String>>,
    status: gtk::Label,
    dot: gtk::Box,
    account: gtk::MenuButton,
    /// Set while rows are rebuilt or selected from code, so that isn't taken as a click.
    quiet: std::cell::Cell<bool>,
}

pub const UNTITLED: &str = "New Note";

pub fn title_of(n: &TreeNote) -> &str {
    if n.title.trim().is_empty() { UNTITLED } else { &n.title }
}

/// Notebooks by parent; a shared notebook whose parent I can't see is a root for me.
fn by_parent(tree: &Tree) -> HashMap<Option<String>, Vec<TreeNotebook>> {
    let ids: HashSet<&str> = tree.notebooks.iter().map(|n| n.id.as_str()).collect();
    let mut map: HashMap<Option<String>, Vec<TreeNotebook>> = HashMap::new();
    for nb in &tree.notebooks {
        let parent = nb.parent_id.clone().filter(|p| ids.contains(p.as_str()));
        map.entry(parent).or_default().push(nb.clone());
    }
    for v in map.values_mut() {
        v.sort_by_key(|n| n.name.to_lowercase());
    }
    map
}

fn sorted(mut notes: Vec<TreeNote>) -> Vec<TreeNote> {
    notes.sort_by_key(|n| title_of(n).to_lowercase());
    notes
}

/// Your own notes at the top level.
pub fn root_notes(tree: &Tree) -> Vec<TreeNote> {
    sorted(tree.notes.iter().filter(|n| n.role == crate::api::Role::Owner && n.notebook_id.is_none()).cloned().collect())
}

/// Notes shared with you one by one, not as part of a notebook.
pub fn shared_notes(tree: &Tree) -> Vec<TreeNote> {
    let direct: HashSet<&str> = tree
        .shared
        .iter()
        .filter(|s| s.resource_type == "note" && !s.hidden)
        .map(|s| s.resource_id.as_str())
        .collect();
    sorted(tree.notes.iter().filter(|n| direct.contains(n.id.as_str())).cloned().collect())
}

pub fn notes_in(tree: &Tree, notebook: &str) -> Vec<TreeNote> {
    sorted(tree.notes.iter().filter(|n| n.notebook_id.as_deref() == Some(notebook)).cloned().collect())
}

/// "Home › Kitchen" for a notebook.
pub fn path_of(tree: &Tree, mut id: Option<String>) -> String {
    let mut parts = Vec::new();
    while let Some(nb) = id.and_then(|i| tree.notebooks.iter().find(|n| n.id == i)) {
        parts.push(nb.name.clone());
        id = nb.parent_id.clone();
    }
    parts.reverse();
    parts.join(" › ")
}

fn row_box(depth: usize, icon: &str, label: &str) -> (gtk::Box, gtk::Label) {
    let b = gtk::Box::builder().spacing(10).margin_start(depth as i32 * 18).build();
    b.append(&gtk::Image::from_icon_name(icon));
    let l = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    b.append(&l);
    (b, l)
}

fn count_label(n: usize) -> gtk::Label {
    gtk::Label::builder().label(n.to_string()).css_classes(["dim-label", "caption", "numeric"]).build()
}

fn shared_badge() -> gtk::Image {
    let i = gtk::Image::from_icon_name("avatar-default-symbolic");
    i.add_css_class("shared-badge");
    i.set_tooltip_text(Some("Shared"));
    i
}

impl Sidebar {
    pub fn new(main: &Rc<Main>) -> Rc<Sidebar> {
        // Where you are is marked with a style class, not the list's selection, which follows keyboard focus.
        let list = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).selection_mode(gtk::SelectionMode::None).build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();

        let header = adw::HeaderBar::new();
        let new_notebook = gtk::Button::builder()
            .icon_name("folder-new-symbolic")
            .tooltip_text("New Notebook")
            .action_name("win.new-notebook")
            .build();
        header.pack_start(&new_notebook);

        let account_button = gtk::MenuButton::builder().tooltip_text("Account").css_classes(["flat", "circular", "account"]).build();
        header.pack_end(&account_button);

        let dot = gtk::Box::builder().css_classes(["status-dot"]).valign(gtk::Align::Center).build();
        let status = gtk::Label::builder().xalign(0.0).css_classes(["caption", "dim-label"]).build();
        let footer = gtk::Box::builder().spacing(8).css_classes(["sidebar-footer"]).build();
        footer.append(&dot);
        footer.append(&status);

        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&scroll));
        view.add_bottom_bar(&footer);

        let sidebar = Rc::new(Sidebar {
            widget: view.upcast(),
            main: Rc::downgrade(main),
            list: list.clone(),
            rows: Default::default(),
            open: Default::default(),
            status,
            dot,
            account: account_button,
            quiet: Default::default(),
        });

        let weak = Rc::downgrade(&sidebar);
        list.connect_row_activated(move |_, row| {
            if let Some(s) = weak.upgrade() {
                s.activate(row.index());
            }
        });

        let weak = Rc::downgrade(&sidebar);
        main.store.on_tree(move || weak.upgrade().map(|s| s.rebuild()).is_some());
        let weak = Rc::downgrade(&sidebar);
        main.store.on_status(move || weak.upgrade().map(|s| s.show_status()).is_some());
        let weak = Rc::downgrade(&sidebar);
        main.store.on_features(move || weak.upgrade().map(|s| s.rebuild()).is_some());
        sidebar.rebuild();
        sidebar.show_status();
        sidebar
    }

    /// Your avatar and its menu, which follow a name change.
    fn show_account(&self) {
        let Some(main) = self.main.upgrade() else { return };
        let me = main.store.user();
        let menu = gio::Menu::new();
        let items = gio::Menu::new();
        items.append(Some("Settings"), Some("win.settings"));
        items.append(Some("About Gnotes"), Some("win.about"));
        items.append(Some("Log Out"), Some("win.logout"));
        menu.append_section(Some(&me.display_name), &items);
        self.account.set_menu_model(Some(&menu));
        self.account.set_child(Some(&super::person(&me.display_name, crate::note::color_for(&me.id), 26)));
    }

    fn show_status(&self) {
        let Some(main) = self.main.upgrade() else { return };
        let (text, class) = match main.store.status.get() {
            Status::Online => ("Connected", "online"),
            Status::Connecting => ("Connecting…", "connecting"),
            Status::Offline => ("Offline", "offline"),
        };
        self.status.set_label(text);
        for c in ["online", "connecting", "offline"] {
            self.dot.remove_css_class(c);
        }
        self.dot.add_css_class(class);
    }

    fn activate(self: &Rc<Self>, index: i32) {
        if self.quiet.get() {
            return;
        }
        let Some(main) = self.main.upgrade() else { return };
        let Some(row) = self.rows.borrow().get(index as usize).cloned() else { return };
        match row {
            Row::Ask => main.show_place(Place::Ask),
            Row::Search => main.show_place(Place::Search),
            Row::Recent => main.show_place(Place::Recent),
            Row::Shared => main.show_place(Place::Shared),
            Row::Trash => main.show_place(Place::Trash),
            Row::Note(id) => main.open_note(&id, false),
            Row::Notebook(id) => {
                let can_add = main.store.tree.borrow().notebooks.iter().any(|n| n.id == id && n.role.can_edit());
                if can_add {
                    main.folder.replace(Some(id.clone()));
                }
                self.toggle(&id);
            }
            Row::Heading => {}
        }
    }

    fn toggle(self: &Rc<Self>, id: &str) {
        {
            let mut open = self.open.borrow_mut();
            if !open.remove(id) {
                open.insert(id.to_owned());
            }
        }
        self.rebuild();
    }

    /// Unfolds the notebooks a note is in, so its row shows.
    pub fn reveal(self: &Rc<Self>, note: &str) {
        let Some(main) = self.main.upgrade() else { return };
        let mut changed = false;
        {
            let tree = main.store.tree.borrow();
            let mut id = tree.notes.iter().find(|n| n.id == note).and_then(|n| n.notebook_id.clone());
            let mut open = self.open.borrow_mut();
            while let Some(nb) = id.and_then(|i| tree.notebooks.iter().find(|n| n.id == i)) {
                changed |= open.insert(nb.id.clone());
                id = nb.parent_id.clone();
            }
        }
        if changed {
            self.rebuild();
        }
    }

    /// Each row's text, top to bottom, with a `> ` before the highlighted one. For tests.
    pub fn labels(&self) -> Vec<String> {
        fn first_label(w: &gtk::Widget) -> Option<String> {
            if let Some(l) = w.downcast_ref::<gtk::Label>() {
                return Some(l.label().to_string());
            }
            let mut child = w.first_child();
            while let Some(c) = child {
                if let Some(t) = first_label(&c) {
                    return Some(t);
                }
                child = c.next_sibling();
            }
            None
        }
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(row) = self.list.row_at_index(i) {
            let text = first_label(row.upcast_ref()).unwrap_or_default();
            out.push(if row.has_css_class("current") { format!("> {text}") } else { text });
            i += 1;
        }
        out
    }

    /// Opens a notebook and the ones it's in, and highlights it.
    pub fn unfold(self: &Rc<Self>, id: &str) {
        let Some(main) = self.main.upgrade() else { return };
        {
            let tree = main.store.tree.borrow();
            let mut open = self.open.borrow_mut();
            open.insert(id.to_owned());
            let mut parent = tree.notebooks.iter().find(|n| n.id == id).and_then(|n| n.parent_id.clone());
            while let Some(nb) = parent.and_then(|p| tree.notebooks.iter().find(|n| n.id == p)) {
                open.insert(nb.id.clone());
                parent = nb.parent_id.clone();
            }
        }
        self.rebuild();
        let index = self.rows.borrow().iter().position(|r| *r == Row::Notebook(id.to_owned()));
        self.mark(index);
    }

    /// Highlights the row for what the content pane shows.
    pub fn select(&self) {
        let Some(main) = self.main.upgrade() else { return };
        let want = match &*main.place.borrow() {
            Place::Note(id) => Row::Note(id.clone()),
            Place::Recent => Row::Recent,
            Place::Search => Row::Search,
            Place::Ask => Row::Ask,
            Place::Shared => Row::Shared,
            Place::Trash => Row::Trash,
            Place::Nothing => {
                self.mark(None);
                return;
            }
        };
        let index = self.rows.borrow().iter().position(|r| *r == want);
        self.mark(index);
    }

    /// Marks row `index` as where you are, and no other.
    fn mark(&self, index: Option<usize>) {
        let mut i = 0;
        while let Some(row) = self.list.row_at_index(i) {
            if index == Some(i as usize) {
                row.add_css_class("current");
            } else {
                row.remove_css_class("current");
            }
            i += 1;
        }
    }

    fn rebuild(self: &Rc<Self>) {
        let Some(main) = self.main.upgrade() else { return };
        self.show_account();
        self.quiet.set(true);
        self.list.remove_all();
        self.rows.borrow_mut().clear();
        let tree = main.store.tree.borrow().clone();
        let map = by_parent(&tree);
        let empty = Vec::new();
        let tops = map.get(&None).unwrap_or(&empty);
        let hidden: HashSet<&str> = tree.shared.iter().filter(|s| s.hidden).map(|s| s.resource_id.as_str()).collect();
        let own: Vec<&TreeNotebook> = tops.iter().filter(|n| n.role == crate::api::Role::Owner).collect();
        let shared: Vec<&TreeNotebook> =
            tops.iter().filter(|n| n.role != crate::api::Role::Owner && !hidden.contains(n.id.as_str())).collect();

        // Ask, the AI search, once AI services are set up; plain word search either way.
        if main.store.features.borrow().ask {
            self.add_simple(Row::Ask, "starred-symbolic", "Ask", None);
        }
        self.add_simple(Row::Search, "system-search-symbolic", "Search", None);
        self.add_simple(Row::Recent, "document-open-recent-symbolic", "Recent", None);
        self.add_heading("Notes");
        for nb in &own {
            self.add_notebook(&main, &tree, &map, nb, 0);
        }
        for n in root_notes(&tree) {
            self.add_note(&main, &n, 0);
        }
        let shared_notes = shared_notes(&tree);
        // The heading opens the Shared page: what's shared with you, then what you've shared.
        let l = gtk::Label::builder().label("Shared").xalign(0.0).css_classes(["heading", "dim-label", "sidebar-heading"]).build();
        self.push(Row::Shared, &l).add_css_class("heading-row");
        for nb in &shared {
            self.add_notebook(&main, &tree, &map, nb, 0);
        }
        for n in &shared_notes {
            self.add_note(&main, n, 0);
        }
        self.add_simple(Row::Trash, "user-trash-symbolic", "Trash", None);
        self.quiet.set(false);
        self.select();
    }

    fn push(&self, row: Row, child: &impl IsA<gtk::Widget>) -> gtk::ListBoxRow {
        let r = gtk::ListBoxRow::builder().child(child).build();
        if row == Row::Heading {
            r.set_activatable(false);
            r.set_selectable(false);
        }
        self.list.append(&r);
        self.rows.borrow_mut().push(row);
        r
    }

    fn add_simple(&self, row: Row, icon: &str, label: &str, count: Option<usize>) {
        let (b, _) = row_box(0, icon, label);
        if let Some(n) = count {
            b.append(&count_label(n));
        }
        self.push(row, &b);
    }

    fn add_heading(&self, label: &str) {
        let l = gtk::Label::builder().label(label).xalign(0.0).css_classes(["heading", "dim-label", "sidebar-heading"]).build();
        let row = self.push(Row::Heading, &l);
        row.add_css_class("heading-row");
        // Dropping on the Notes heading moves to your top level.
        if let Some(main) = self.main.upgrade() {
            drop_into(&main, &row, None);
        }
    }

    fn add_notebook(
        self: &Rc<Self>,
        main: &Rc<Main>,
        tree: &Tree,
        map: &HashMap<Option<String>, Vec<TreeNotebook>>,
        nb: &TreeNotebook,
        depth: usize,
    ) {
        let open = self.open.borrow().contains(&nb.id);
        let kids = map.get(&Some(nb.id.clone())).cloned().unwrap_or_default();
        let notes = notes_in(tree, &nb.id);
        let icon = if open { "folder-open-symbolic" } else { "folder-symbolic" };
        let (b, _) = row_box(depth, icon, &nb.name);
        b.first_child().expect("icon").add_css_class("folder-icon");
        if nb.shared {
            b.append(&shared_badge());
        }
        b.append(&count_label(kids.len() + notes.len()));
        let arrow = gtk::Image::from_icon_name(if open { "pan-down-symbolic" } else { "pan-end-symbolic" });
        arrow.add_css_class("dim-label");
        b.append(&arrow);
        let row = self.push(Row::Notebook(nb.id.clone()), &b);
        row.set_tooltip_text(Some(&nb.name));
        attach_menu(main, &row, Item::Notebook(nb.clone()));
        if nb.role.can_edit() {
            drag_from(&row, &format!("notebook:{}", nb.id));
            drop_into(main, &row, Some(nb.id.clone()));
        }
        if open {
            for k in &kids {
                self.add_notebook(main, tree, map, k, depth + 1);
            }
            for n in &notes {
                self.add_note(main, n, depth + 1);
            }
        }
    }

    fn add_note(&self, main: &Rc<Main>, n: &TreeNote, depth: usize) {
        let (b, label) = row_box(depth, "text-x-generic-symbolic", title_of(n));
        if n.title.trim().is_empty() {
            label.add_css_class("dim-label");
        }
        if n.shared && n.role == crate::api::Role::Owner {
            b.append(&shared_badge());
        }
        let row = self.push(Row::Note(n.id.clone()), &b);
        attach_menu(main, &row, Item::Note(n.clone()));
        if n.role.can_edit() {
            drag_from(&row, &format!("note:{}", n.id));
        }
    }
}

/// Rows drag as `note:<id>` or `notebook:<id>`.
fn drag_from(row: &gtk::ListBoxRow, payload: &str) {
    let source = gtk::DragSource::builder().actions(gtk::gdk::DragAction::MOVE).build();
    let payload = payload.to_owned();
    source.connect_prepare(move |_, _, _| Some(gtk::gdk::ContentProvider::for_value(&payload.to_value())));
    let r = row.clone();
    source.connect_drag_begin(move |src, _| {
        src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&r))), 0, 0);
    });
    row.add_controller(source);
}

/// Something dropped on a notebook row (or the Notes heading, `None`) moves there.
fn drop_into(main: &Rc<Main>, row: &gtk::ListBoxRow, target: Option<String>) {
    let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
    let weak = Rc::downgrade(main);
    let r = row.clone();
    drop.connect_enter(move |_, _, _| {
        r.add_css_class("drop-here");
        gtk::gdk::DragAction::MOVE
    });
    let r = row.clone();
    drop.connect_leave(move |_| r.remove_css_class("drop-here"));
    let r = row.clone();
    drop.connect_drop(move |_, value, _, _| {
        r.remove_css_class("drop-here");
        let (Some(main), Ok(payload)) = (weak.upgrade(), value.get::<String>()) else { return false };
        move_dropped(&main, &payload, target.clone())
    });
    row.add_controller(drop);
}

/// Moves a dragged `note:<id>` or `notebook:<id>` into `target`. False when it can't go there.
pub fn move_dropped(main: &Rc<Main>, payload: &str, target: Option<String>) -> bool {
    let tree = main.store.tree.borrow().clone();
    let Some((kind, id)) = payload.split_once(':') else { return false };
    let item = match kind {
        "note" => tree.notes.iter().find(|n| n.id == id).cloned().map(Item::Note),
        "notebook" => tree.notebooks.iter().find(|n| n.id == id).cloned().map(Item::Notebook),
        _ => None,
    };
    let Some(item) = item else { return false };
    // Only your own things go to your top level; a notebook can't go into itself or what's in it.
    let (owner, current) = match &item {
        Item::Note(n) => (n.role == crate::api::Role::Owner, n.notebook_id.clone()),
        Item::Notebook(nb) => (nb.role == crate::api::Role::Owner, nb.parent_id.clone()),
    };
    if target == current || (target.is_none() && !owner) {
        return false;
    }
    if let (Item::Notebook(nb), Some(t)) = (&item, &target) {
        let mut at = Some(t.clone());
        while let Some(x) = at {
            if x == nb.id {
                return false;
            }
            at = tree.notebooks.iter().find(|n| n.id == x).and_then(|n| n.parent_id.clone());
        }
    }
    if let Some(t) = &target
        && !tree.notebooks.iter().any(|n| &n.id == t && n.role.can_edit())
    {
        return false;
    }
    dialogs::move_item(main, &item, target.clone());
    if let Some(t) = target {
        main.sidebar().unfold(&t);
    }
    true
}

/// What a row's menu acts on.
#[derive(Clone)]
pub enum Item {
    Note(TreeNote),
    Notebook(TreeNotebook),
}

/// Right-click (or long-press) a row for its actions.
fn attach_menu(main: &Rc<Main>, row: &gtk::ListBoxRow, item: Item) {
    let menu = gio::Menu::new();
    let group = gio::SimpleActionGroup::new();
    let add = |label: &str, name: &str, f: Box<dyn Fn(&Rc<Main>)>| {
        let a = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(main);
        a.connect_activate(move |_, _| {
            if let Some(m) = weak.upgrade() {
                f(&m);
            }
        });
        group.add_action(&a);
        menu.append(Some(label), Some(&format!("row.{name}")));
    };
    match &item {
        Item::Notebook(nb) => {
            let owner = nb.role == crate::api::Role::Owner;
            if nb.role.can_edit() {
                let id = nb.id.clone();
                add("New Note Here", "new-note", Box::new(move |m| {
                    m.folder.replace(Some(id.clone()));
                    m.new_note();
                }));
                let id = nb.id.clone();
                add("New Notebook Inside", "new-notebook", Box::new(move |m| dialogs::new_notebook(m, Some(id.clone()))));
            }
            if owner {
                let nb2 = nb.clone();
                add("Rename…", "rename", Box::new(move |m| dialogs::rename_notebook(m, &nb2)));
            }
            if nb.role.can_edit() {
                let it = item.clone();
                add("Move to…", "move", Box::new(move |m| dialogs::move_to(m, &it)));
            }
            if owner {
                let it = item.clone();
                add("Share…", "share", Box::new(move |m| dialogs::share(m, &it)));
                let nb2 = nb.clone();
                add("Move to Trash", "trash", Box::new(move |m| dialogs::trash_notebook(m, &nb2)));
            }
        }
        Item::Note(n) => {
            if n.role.can_edit() {
                let it = item.clone();
                add("Move to…", "move", Box::new(move |m| dialogs::move_to(m, &it)));
            }
            if n.role == crate::api::Role::Owner {
                let it = item.clone();
                add("Share…", "share", Box::new(move |m| dialogs::share(m, &it)));
                let id = n.id.clone();
                add("Move to Trash", "trash", Box::new(move |m| dialogs::trash_note(m, &id)));
            }
        }
    }
    if menu.n_items() == 0 {
        return;
    }
    row.insert_action_group("row", Some(&group));
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(row);
    popover.set_has_arrow(false);
    popover.set_halign(gtk::Align::Start);
    let show = {
        let popover = popover.clone();
        move |x: f64, y: f64| {
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        }
    };
    let click = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
    {
        let show = show.clone();
        click.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            show(x, y);
        });
    }
    row.add_controller(click);
    let long = gtk::GestureLongPress::new();
    long.set_touch_only(true);
    long.connect_pressed(move |_, x, y| show(x, y));
    row.add_controller(long);
    // Rows are rebuilt often; the popover goes with its row.
    row.connect_destroy(move |_| popover.unparent());
}
