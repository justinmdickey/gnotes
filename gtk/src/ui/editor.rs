//! An open note: its Loro document bound to a text view, synced live over the websocket, like
//! web/src/Editor.svelte. Typing becomes Loro edits; edits from others, and undo, come back as ops
//! applied to the buffer.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
    time::Duration,
};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use loro::VersionVector;

use super::{
    dialogs,
    markdown::{self, Styled},
    noteview::{NoteView, Picture, RemoteCursor},
    sidebar::{self, Item, title_of},
    store::{NoteHandler, Store},
    window::Main,
};
use crate::{
    api::Role,
    bg,
    note::{NoteDoc, Op, color_for, color_rgb},
};

/// Photos are at most this wide, even in a wide window.
const MAX_PHOTO: f32 = 560.0;

pub struct Editor {
    me: Weak<Editor>,
    pub id: String,
    pub page: adw::NavigationPage,
    main: Weak<Main>,
    store: Rc<Store>,
    view: NoteView,
    buffer: gtk::TextBuffer,
    doc: RefCell<NoteDoc>,
    handler: RefCell<Option<Rc<dyn NoteHandler>>>,
    /// Set while the buffer changes to match the doc, so that isn't taken as typing.
    applying: Cell<bool>,
    role: Cell<Option<Role>>,
    lost: Cell<bool>,
    /// May have edits the server lacks.
    dirty: Cell<bool>,
    closed: Cell<bool>,
    save_timer: RefCell<Option<glib::SourceId>>,
    style_queued: Cell<bool>,
    styled: RefCell<Styled>,
    timers: RefCell<Vec<glib::SourceId>>,
    title: adw::WindowTitle,
    peers_box: gtk::Box,
    banner: adw::Banner,
    undo_button: gtk::Button,
    redo_button: gtk::Button,
    format_bar: gtk::Widget,
    share_button: gtk::Button,
    menu_button: gtk::MenuButton,
    /// Photos by attachment id; None while loading, or when it isn't a picture.
    textures: RefCell<HashMap<String, Option<gdk::Texture>>>,
    /// Spacing tags made for photos, by height.
    photo_tags: RefCell<HashMap<i32, gtk::TextTag>>,
    player: RefCell<Option<(String, gtk::MediaFile)>>,
    /// Typing focuses a new note once its content has arrived.
    focus_when_ready: Cell<bool>,
}

/// Forwards websocket events to the editor without keeping it alive.
struct Handler(Weak<Editor>);

impl NoteHandler for Handler {
    fn version(&self) -> Option<VersionVector> {
        let e = self.0.upgrade()?;
        let doc = e.doc.borrow();
        doc.has_data.then(|| doc.version())
    }

    fn joined(&self, role: Role, server: VersionVector) {
        if let Some(e) = self.0.upgrade() {
            e.joined(role, server);
        }
    }

    fn update(&self, data: &[u8]) {
        if let Some(e) = self.0.upgrade() {
            e.remote_update(data);
        }
    }

    fn presence(&self, data: &[u8]) {
        if let Some(e) = self.0.upgrade() {
            e.doc.borrow().apply_presence(data);
            e.show_peers();
        }
    }

    fn role(&self, role: Role) {
        if let Some(e) = self.0.upgrade() {
            e.set_role(role);
        }
    }

    fn lost(&self, reason: &str) {
        if let Some(e) = self.0.upgrade() {
            e.lost.set(true);
            e.banner.set_title(if reason == "revoked" {
                "This note is no longer shared with you"
            } else {
                "This note was deleted"
            });
            e.banner.set_revealed(true);
            e.refresh_editable();
            e.store.cache.forget(&e.id);
        }
    }

    fn out_of_sync(&self) {
        if let Some(e) = self.0.upgrade() {
            e.store.rejoin(&e.id);
        }
    }
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder().icon_name(icon).tooltip_text(tooltip).css_classes(["flat"]).build()
}

fn text_button(label: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder().label(label).tooltip_text(tooltip).css_classes(["flat", "text-style"]).build()
}

/// Where the cursor should go after ops: the end of the last change.
fn ops_end(ops: &[Op]) -> Option<usize> {
    let mut pos = 0;
    let mut last = None;
    for op in ops {
        match op {
            Op::Retain(n) => pos += n,
            Op::Insert(s) => {
                pos += s.chars().count();
                last = Some(pos);
            }
            Op::Delete(_) => last = Some(pos),
        }
    }
    last
}

impl Editor {
    pub fn new(main: &Rc<Main>, id: &str) -> Rc<Editor> {
        let store = main.store.clone();
        let view = NoteView::new();
        view.add_css_class("note-text");
        view.set_editable(false);
        let buffer = view.buffer();
        buffer.set_enable_undo(false);
        for name in markdown::TAGS {
            buffer.create_tag(Some(name), &[]);
        }

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&view)
            .build();

        let title = adw::WindowTitle::new(sidebar::UNTITLED, "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        let new_note = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text("New Note")
            .action_name("win.new-note")
            .css_classes(["suggested-action"])
            .build();
        let menu_button = gtk::MenuButton::builder().icon_name("view-more-symbolic").tooltip_text("Note Menu").build();
        let share_content = adw::ButtonContent::builder().icon_name("send-to-symbolic").label("Share").build();
        // Narrow windows have room for the icon only.
        main.split
            .bind_property("collapsed", &share_content, "label")
            .transform_to(|_, collapsed: bool| Some(if collapsed { "" } else { "Share" }))
            .sync_create()
            .build();
        let share_button = gtk::Button::builder()
            .child(&share_content)
            .tooltip_text("Share")
            .css_classes(["flat"])
            .visible(false)
            .build();
        let peers_box = gtk::Box::builder().spacing(4).valign(gtk::Align::Center).build();
        header.pack_end(&new_note);
        header.pack_end(&menu_button);
        header.pack_end(&share_button);
        header.pack_end(&peers_box);

        // The format bar, like the PWA's: undo, text styles, emphasis, lists, photo.
        let bar = gtk::Box::builder().spacing(2).css_classes(["format-bar", "toolbar"]).halign(gtk::Align::Center).build();
        let undo_button = icon_button("edit-undo-symbolic", "Undo (Ctrl+Z)");
        let redo_button = icon_button("edit-redo-symbolic", "Redo (Ctrl+Shift+Z)");
        bar.append(&undo_button);
        bar.append(&redo_button);
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let styles = [
            ("Title", "Title (Ctrl+Shift+T)", "# "),
            ("Heading", "Heading (Ctrl+Shift+H)", "## "),
            ("Subheading", "Subheading (Ctrl+Shift+J)", "### "),
            ("Body", "Body (Ctrl+Shift+B)", ""),
        ];
        let mut style_buttons = Vec::new();
        for (label, tip, prefix) in styles {
            let b = text_button(label, tip);
            bar.append(&b);
            style_buttons.push((b, prefix));
        }
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let bold = icon_button("format-text-bold-symbolic", "Bold (Ctrl+B)");
        let italic = icon_button("format-text-italic-symbolic", "Italic (Ctrl+I)");
        let strike = icon_button("format-text-strikethrough-symbolic", "Strikethrough (Ctrl+Shift+X)");
        let code = gtk::Button::builder().label("</>").tooltip_text("Code").css_classes(["flat", "code-button"]).build();
        for b in [&bold, &italic, &strike, &code] {
            bar.append(b);
        }
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let check = icon_button("checkbox-checked-symbolic", "Checklist (Ctrl+Shift+L)");
        let bullets = icon_button("view-list-bullet-symbolic", "Bulleted List (Ctrl+Shift+8)");
        let numbers = icon_button("view-list-ordered-symbolic", "Numbered List (Ctrl+Shift+7)");
        let quote = gtk::Button::builder().label("“").tooltip_text("Quote").css_classes(["flat", "text-style", "quote-button"]).build();
        for b in [&check, &bullets, &numbers, &quote] {
            bar.append(b);
        }
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let photo = icon_button("camera-photo-symbolic", "Add Photo");
        bar.append(&photo);
        let bar_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .child(&bar)
            .build();
        bar_scroll.set_propagate_natural_width(false);

        let banner = adw::Banner::new("");
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&bar_scroll);
        toolbar.add_top_bar(&banner);
        toolbar.set_content(Some(&scroll));
        toolbar.set_top_bar_style(adw::ToolbarStyle::Raised);

        let page = adw::NavigationPage::builder().title(sidebar::UNTITLED).tag(format!("note:{id}")).child(&toolbar).build();

        let mut doc = NoteDoc::new();
        doc.set_user(&store.user.display_name, color_for(&store.user.id));
        let editor = Rc::new_cyclic(|me| Editor {
            me: me.clone(),
            id: id.to_owned(),
            page,
            main: Rc::downgrade(main),
            store: store.clone(),
            view: view.clone(),
            buffer: buffer.clone(),
            doc: RefCell::new(NoteDoc::new()),
            handler: RefCell::new(None),
            applying: Cell::new(false),
            role: Cell::new(None),
            lost: Cell::new(false),
            dirty: Cell::new(store.cache.dirty().iter().any(|d| d == id)),
            closed: Cell::new(false),
            save_timer: RefCell::new(None),
            style_queued: Cell::new(false),
            styled: RefCell::new(Styled::default()),
            timers: RefCell::new(Vec::new()),
            title,
            peers_box,
            banner,
            undo_button: undo_button.clone(),
            redo_button: redo_button.clone(),
            format_bar: bar_scroll.upcast(),
            share_button: share_button.clone(),
            menu_button,
            textures: Default::default(),
            photo_tags: Default::default(),
            player: Default::default(),
            focus_when_ready: Cell::new(false),
        });

        // Open from this computer's copy first, so the note shows with no network and the join
        // carries its version. A note made offline has nothing to wait for.
        let changes = match store.cache.note(id) {
            Some(saved) => doc.import(&saved).unwrap_or_default(),
            None => {
                // A note made here, now or offline, starts empty; nothing to wait for.
                if store.take_fresh(id) || store.is_queued(id) {
                    doc.has_data = true;
                }
                Vec::new()
            }
        };
        let empty = store.note(id).is_none_or(|n| n.title.is_empty()) && doc.is_empty();
        editor.focus_when_ready.set(empty);
        editor.doc.replace(doc);
        editor.apply(changes);
        editor.take_outbox();
        editor.color_tags();
        editor.restyle();
        editor.show_title();
        editor.refresh_editable();
        editor.refresh_history();

        editor.connect_buffer();
        editor.connect_view();
        editor.connect_buttons(&style_buttons, [&bold, &italic, &strike, &code], [&check, &bullets, &numbers, &quote], &photo);
        {
            let weak = Rc::downgrade(&editor);
            share_button.connect_clicked(move |_| {
                if let Some(e) = weak.upgrade()
                    && let (Some(main), Some(n)) = (e.main.upgrade(), e.store.note(&e.id))
                {
                    dialogs::share(&main, &Item::Note(n));
                }
            });
        }
        {
            let weak = Rc::downgrade(&editor);
            store.on_tree(move || match weak.upgrade() {
                Some(e) if !e.closed.get() => {
                    e.show_title();
                    true
                }
                _ => false,
            });
        }
        {
            let weak = Rc::downgrade(&editor);
            adw::StyleManager::default().connect_dark_notify(move |_| {
                // After the new style reaches the view.
                let weak = weak.clone();
                glib::idle_add_local_once(move || {
                    if let Some(e) = weak.upgrade() {
                        e.color_tags();
                    }
                });
            });
            let weak = Rc::downgrade(&editor);
            view.connect_realize(move |_| {
                if let Some(e) = weak.upgrade() {
                    e.color_tags();
                }
            });
        }

        // Presence expires after 30 seconds without a refresh, so re-announce while the note is open.
        let weak = Rc::downgrade(&editor);
        let heartbeat = glib::timeout_add_local(Duration::from_secs(10), move || {
            let Some(e) = weak.upgrade() else { return glib::ControlFlow::Break };
            e.doc.borrow().refresh_presence();
            e.take_outbox();
            e.show_peers();
            glib::ControlFlow::Continue
        });
        editor.timers.borrow_mut().push(heartbeat);

        let handler: Rc<dyn NoteHandler> = Rc::new(Handler(Rc::downgrade(&editor)));
        editor.handler.replace(Some(handler.clone()));
        store.open(id, handler);
        if editor.doc.borrow().has_data && editor.focus_when_ready.get() {
            editor.focus();
        }
        editor
    }

    fn can_edit(&self) -> bool {
        let role = self.role.get().or_else(|| self.store.note(&self.id).map(|n| n.role)).unwrap_or(Role::Owner);
        role.can_edit() && !self.lost.get() && self.doc.borrow().has_data
    }

    fn refresh_editable(&self) {
        let can = self.can_edit();
        self.view.set_editable(can);
        self.view.set_cursor_visible(can);
        self.format_bar.set_sensitive(can);
        let viewer = self.role.get() == Some(Role::Viewer);
        if viewer && !self.lost.get() {
            self.banner.set_title("You can read this note but not change it");
            self.banner.set_revealed(true);
        } else if !self.lost.get() {
            self.banner.set_revealed(false);
        }
        let note = self.store.note(&self.id);
        let owner = note.as_ref().is_some_and(|n| n.role == Role::Owner) && !self.store.is_queued(&self.id);
        self.share_button.set_visible(owner && !self.lost.get());
        self.menu_button.set_menu_model(Some(&self.menu(note.map(|n| n.role))));
    }

    fn menu(&self, role: Option<Role>) -> gio::Menu {
        let menu = gio::Menu::new();
        let group = gio::SimpleActionGroup::new();
        let add = |label: &str, name: &str, f: Box<dyn Fn(&Rc<Main>, &str)>| {
            let a = gio::SimpleAction::new(name, None);
            let (weak, id) = (self.main.clone(), self.id.clone());
            a.connect_activate(move |_, _| {
                if let Some(m) = weak.upgrade() {
                    f(&m, &id);
                }
            });
            group.add_action(&a);
            menu.append(Some(label), Some(&format!("note.{name}")));
        };
        if role.is_some_and(Role::can_edit) {
            add("Move to…", "move", Box::new(|m, id| {
                if let Some(n) = m.store.note(id) {
                    dialogs::move_to(m, &Item::Note(n));
                }
            }));
        }
        if role == Some(Role::Owner) {
            add("Move to Trash", "trash", Box::new(dialogs::trash_note));
        }
        self.page.insert_action_group("note", Some(&group));
        menu
    }

    fn set_role(&self, role: Role) {
        self.role.set(Some(role));
        self.refresh_editable();
    }

    fn show_title(&self) {
        let Some(note) = self.store.note(&self.id) else { return };
        let title = title_of(&note).to_owned();
        self.title.set_title(&title);
        self.page.set_title(&title);
        let tree = self.store.tree.borrow();
        let path = sidebar::path_of(&tree, note.notebook_id.clone());
        let subtitle = if note.role != Role::Owner && !note.owner.is_empty() {
            if path.is_empty() { format!("Shared by {}", note.owner) } else { format!("{path} · {}", note.owner) }
        } else {
            path
        };
        self.title.set_subtitle(&subtitle);
    }

    fn focus(&self) {
        self.focus_when_ready.set(false);
        let view = self.view.clone();
        glib::idle_add_local_once(move || {
            view.grab_focus();
        });
    }

    /// Applies changes from the doc to the buffer.
    fn apply(&self, changes: Vec<Vec<Op>>) -> Option<usize> {
        if changes.is_empty() {
            return None;
        }
        self.applying.set(true);
        let mut end = None;
        for ops in changes {
            let mut pos = 0usize;
            for op in &ops {
                match op {
                    Op::Retain(n) => pos += n,
                    Op::Insert(s) => {
                        let mut it = self.buffer.iter_at_offset(pos as i32);
                        self.buffer.insert(&mut it, s);
                        pos += s.chars().count();
                    }
                    Op::Delete(n) => {
                        let mut a = self.buffer.iter_at_offset(pos as i32);
                        let mut b = self.buffer.iter_at_offset((pos + n) as i32);
                        self.buffer.delete(&mut a, &mut b);
                    }
                }
            }
            end = ops_end(&ops).or(end);
        }
        self.applying.set(false);
        self.changed();
        end
    }

    /// Sends local updates and presence, or keeps the note dirty when offline.
    fn take_outbox(&self) {
        let doc = self.doc.borrow();
        let joined = self.role.get().is_some();
        for u in doc.take_outbox() {
            if !joined || !self.store.sync.send_update(&self.id, &u) {
                self.mark_dirty();
            }
        }
        // The server takes presence only from clients that have joined.
        let presence = doc.take_presence_out();
        if joined {
            for p in presence {
                self.store.sync.send_presence(&self.id, &p);
            }
        }
    }

    fn mark_dirty(&self) {
        if !self.dirty.replace(true) {
            self.store.cache.mark_dirty(&self.id);
        }
    }

    /// The text changed, from typing or elsewhere.
    fn changed(&self) {
        self.queue_restyle();
        self.queue_save();
        self.refresh_history();
        let text = self.doc.borrow().text();
        self.store.retitle(&self.id, &text);
    }

    fn refresh_history(&self) {
        let doc = self.doc.borrow();
        self.undo_button.set_sensitive(doc.can_undo());
        self.redo_button.set_sensitive(doc.can_redo());
    }

    fn queue_save(&self) {
        if self.save_timer.borrow().is_some() {
            return;
        }
        let weak = self.me.clone();
        *self.save_timer.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(800), move || {
            if let Some(e) = weak.upgrade() {
                e.save_timer.take();
                e.save();
            }
        }));
    }

    fn save(&self) {
        let doc = self.doc.borrow();
        if doc.has_data && !self.lost.get() {
            self.store.cache.save_note(&self.id, &doc.snapshot());
        }
    }

    fn joined(&self, role: Role, server: VersionVector) {
        self.set_role(role);
        {
            let doc = self.doc.borrow();
            let ahead = doc.version().partial_cmp(&server).is_none_or(|o| o == std::cmp::Ordering::Greater);
            if doc.has_data && role.can_edit() && ahead {
                // Edits made while disconnected. It stays dirty until a later join shows the server has them.
                self.store.sync.send_update(&self.id, &doc.export_from(&server));
            } else if self.dirty.replace(false) {
                self.store.cache.clear_dirty(&self.id);
            }
            doc.refresh_presence();
        }
        self.take_outbox();
    }

    fn remote_update(&self, data: &[u8]) {
        let first = !self.doc.borrow().has_data;
        let changes = match self.doc.borrow_mut().import(data) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("importing an update to {}: {e}", self.id);
                return;
            }
        };
        self.apply(changes);
        if first {
            self.refresh_editable();
            if self.focus_when_ready.get() {
                self.focus();
            }
        }
        self.show_peers();
    }

    /// Others' cursors in the text and their avatars in the headerbar.
    fn show_peers(&self) {
        let peers = self.doc.borrow().peers();
        let cursors = peers
            .iter()
            .filter_map(|p| p.cursor.map(|(a, h)| RemoteCursor { anchor: a, head: h, name: p.name.clone(), rgb: color_rgb(&p.color) }))
            .collect();
        self.view.set_cursors(cursors);
        while let Some(c) = self.peers_box.first_child() {
            self.peers_box.remove(&c);
        }
        let mut seen = std::collections::HashSet::new();
        for p in peers {
            if !seen.insert(p.name.clone()) {
                continue;
            }
            self.peers_box.append(&super::person(&p.name, &p.color, 24));
        }
    }

    fn connect_buffer(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.buffer.connect_insert_text(move |_, it, text| {
            let Some(e) = weak.upgrade() else { return };
            if e.applying.get() {
                return;
            }
            e.doc.borrow().insert(it.offset() as usize, text);
            e.after_typing();
        });
        let weak = Rc::downgrade(self);
        self.buffer.connect_delete_range(move |_, a, b| {
            let Some(e) = weak.upgrade() else { return };
            if e.applying.get() {
                return;
            }
            let (s, t) = (a.offset().min(b.offset()), a.offset().max(b.offset()));
            e.doc.borrow().delete(s as usize, (t - s) as usize);
            e.after_typing();
        });
        let weak = Rc::downgrade(self);
        self.buffer.connect_mark_set(move |buffer, _, mark| {
            let Some(e) = weak.upgrade() else { return };
            if mark.name().as_deref() == Some("insert") || mark == &buffer.selection_bound() {
                e.announce_cursor();
            }
        });
    }

    fn after_typing(&self) {
        self.take_outbox();
        self.changed();
    }

    /// Tells the others where the cursor is, while the text has focus.
    fn announce_cursor(&self) {
        let doc = self.doc.borrow();
        if !doc.has_data {
            return;
        }
        if self.view.is_focus() && self.view.is_editable() {
            let head = self.buffer.iter_at_mark(&self.buffer.get_insert()).offset() as usize;
            let anchor = self.buffer.iter_at_mark(&self.buffer.selection_bound()).offset() as usize;
            doc.set_cursor(Some((anchor, head)));
        } else {
            doc.set_cursor(None);
        }
        drop(doc);
        self.take_outbox();
    }

    fn undo(&self, redo: bool) {
        if !self.can_edit() {
            return;
        }
        let changes = {
            let mut doc = self.doc.borrow_mut();
            if redo { doc.redo() } else { doc.undo() }
        };
        if let Some(end) = self.apply(changes) {
            self.buffer.place_cursor(&self.buffer.iter_at_offset(end as i32));
            self.view.scroll_mark_onscreen(&self.buffer.get_insert());
        }
        self.take_outbox();
    }

    /// One undoable change made from code (format bar, checkbox), through the buffer like typing.
    fn edit(&self, f: impl FnOnce(&gtk::TextBuffer)) {
        if !self.can_edit() {
            return;
        }
        self.doc.borrow_mut().checkpoint();
        self.buffer.begin_user_action();
        f(&self.buffer);
        self.buffer.end_user_action();
        self.doc.borrow_mut().checkpoint();
    }

    fn connect_view(self: &Rc<Self>) {
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        focus.connect_enter(move |_| {
            if let Some(e) = weak.upgrade() {
                e.announce_cursor();
            }
        });
        let weak = Rc::downgrade(self);
        focus.connect_leave(move |_| {
            if let Some(e) = weak.upgrade() {
                e.announce_cursor();
            }
        });
        self.view.add_controller(focus);

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(e) = weak.upgrade() else { return glib::Propagation::Proceed };
            e.key(key, mods)
        });
        self.view.add_controller(keys);

        let click = gtk::GestureClick::builder().button(gdk::BUTTON_PRIMARY).build();
        let weak = Rc::downgrade(self);
        click.connect_released(move |g, n, x, y| {
            let Some(e) = weak.upgrade() else { return };
            if n == 1 && e.click(x, y) {
                g.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        self.view.add_controller(click);

        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            let Some(e) = weak.upgrade() else { return };
            let over = e.target_at(x, y).is_some();
            e.view.set_cursor_from_name(Some(if over { "pointer" } else { "text" }));
        });
        self.view.add_controller(motion);

        // Pasted or dropped pictures become photos in the note.
        let weak = Rc::downgrade(self);
        self.view.connect_paste_clipboard(move |view| {
            let Some(e) = weak.upgrade() else { return };
            let clipboard = view.clipboard();
            let formats = clipboard.formats();
            if formats.contains_type(gdk::Texture::static_type()) && !formats.contain_mime_type("text/plain") {
                view.stop_signal_emission_by_name("paste-clipboard");
                let e = e.clone();
                glib::spawn_future_local(async move {
                    if let Ok(Some(texture)) = clipboard.read_texture_future().await {
                        let bytes = texture.save_to_png_bytes().to_vec();
                        e.upload("Pasted photo.png", "image/png", bytes).await;
                    }
                });
            }
        });
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        drop.connect_drop(move |_, value, x, y| {
            let Some(e) = weak.upgrade() else { return false };
            let Ok(files) = value.get::<gdk::FileList>() else { return false };
            if !e.can_edit() {
                return false;
            }
            let (bx, by) = e.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            if let Some(it) = e.view.iter_at_location(bx, by) {
                e.buffer.place_cursor(&it);
            }
            for file in files.files() {
                let e = e.clone();
                glib::spawn_future_local(async move { e.upload_file(&file).await });
            }
            true
        });
        self.view.add_controller(drop);
    }

    fn key(&self, key: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let stop = glib::Propagation::Stop;
        let lower = key.to_lower();
        if ctrl {
            match (lower, shift) {
                (gdk::Key::z, false) => self.undo(false),
                (gdk::Key::z, true) | (gdk::Key::y, false) => self.undo(true),
                (gdk::Key::b, false) => self.wrap("**"),
                (gdk::Key::i, false) => self.wrap("*"),
                (gdk::Key::x, true) => self.wrap("~~"),
                (gdk::Key::t, true) => self.set_block("# "),
                (gdk::Key::h, true) => self.set_block("## "),
                (gdk::Key::j, true) => self.set_block("### "),
                (gdk::Key::b, true) => self.set_block(""),
                (gdk::Key::l, true) => self.set_block("- [ ] "),
                (gdk::Key::_7 | gdk::Key::ampersand, true) => self.set_block("1. "),
                (gdk::Key::_8 | gdk::Key::asterisk, true) => self.set_block("- "),
                _ => return glib::Propagation::Proceed,
            }
            return stop;
        }
        if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) && !shift && self.can_edit() && self.continue_list() {
            return stop;
        }
        glib::Propagation::Proceed
    }

    /// Enter in a list item starts the next one; Enter on an empty item ends the list.
    fn continue_list(&self) -> bool {
        let buffer = &self.buffer;
        if buffer.has_selection() {
            return false;
        }
        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        let mut start = cursor;
        start.set_line_offset(0);
        let line = buffer.text(&start, &cursor, true).to_string();
        let Some(next) = markdown::continue_prefix(&line) else { return false };
        let prefix = markdown::line_prefix(&line);
        let mut end = cursor;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let rest = buffer.text(&cursor, &end, true);
        if line.len() == prefix.len() && rest.trim().is_empty() {
            // An empty item: Enter ends the list.
            let mut s = start;
            let mut c = cursor;
            self.edit(|b| b.delete(&mut s, &mut c));
            return true;
        }
        let mut c = cursor;
        self.edit(|b| b.insert(&mut c, &format!("\n{next}")));
        self.view.scroll_mark_onscreen(&buffer.get_insert());
        true
    }

    /// Puts `marker` around the selection, or takes it away if it's there; with no selection, puts
    /// a pair at the cursor and the cursor between them.
    fn wrap(&self, marker: &str) {
        let buffer = self.buffer.clone();
        let n = marker.chars().count() as i32;
        self.edit(|b| {
            let (s, e) = b.selection_bounds().unwrap_or_else(|| {
                let c = b.iter_at_mark(&b.get_insert());
                (c, c)
            });
            let (so, eo) = (s.offset(), e.offset());
            let before = b.text(&b.iter_at_offset((so - n).max(0)), &s, true);
            let after = b.text(&e, &b.iter_at_offset(eo + n), true);
            if so != eo && before == marker && after == marker {
                b.delete(&mut b.iter_at_offset(eo), &mut b.iter_at_offset(eo + n));
                b.delete(&mut b.iter_at_offset(so - n), &mut b.iter_at_offset(so));
                b.select_range(&b.iter_at_offset(so - n), &b.iter_at_offset(eo - n));
                return;
            }
            b.insert(&mut b.iter_at_offset(eo), marker);
            b.insert(&mut b.iter_at_offset(so), marker);
            if so == eo {
                b.place_cursor(&b.iter_at_offset(so + n));
            } else {
                b.select_range(&b.iter_at_offset(so + n), &b.iter_at_offset(eo + n));
            }
        });
        let _ = buffer;
    }

    /// Makes each selected line a title, heading, list item… (`prefix`), or body text with "". Lines
    /// that all have it already go back to body text.
    fn set_block(&self, prefix: &str) {
        self.edit(|b| {
            let (s, e) = b.selection_bounds().unwrap_or_else(|| {
                let c = b.iter_at_mark(&b.get_insert());
                (c, c)
            });
            let (first, last) = (s.line(), e.line());
            let line_text = |l: i32| {
                let start = b.iter_at_line(l).unwrap_or(b.end_iter());
                let mut end = start;
                if !end.ends_line() {
                    end.forward_to_line_end();
                }
                b.text(&start, &end, true).to_string()
            };
            let kind = |p: &str| -> String {
                let t = p.trim_start();
                if t.starts_with(|c: char| c.is_ascii_digit()) { "1. ".into() } else { t.replace("[x]", "[ ]").replace("[X]", "[ ]") }
            };
            let all_have = !prefix.is_empty() && (first..=last).all(|l| kind(markdown::line_prefix(&line_text(l))) == prefix);
            for (i, l) in (first..=last).enumerate() {
                let text = line_text(l);
                let old = markdown::line_prefix(&text);
                let indent = &old[..old.len() - old.trim_start().len()];
                let new = if all_have || prefix.is_empty() {
                    indent.to_owned()
                } else if prefix == "1. " {
                    format!("{indent}{}. ", i + 1)
                } else if prefix.starts_with('#') {
                    prefix.to_owned()
                } else {
                    format!("{indent}{prefix}")
                };
                if old == new {
                    continue;
                }
                let start = b.iter_at_line(l).unwrap_or(b.end_iter()).offset();
                let old_len = old.chars().count() as i32;
                b.delete(&mut b.iter_at_offset(start), &mut b.iter_at_offset(start + old_len));
                b.insert(&mut b.iter_at_offset(start), &new);
            }
        });
    }

    /// What's under a point: a checkbox, a link, or a recording.
    fn target_at(&self, x: f64, y: f64) -> Option<Target> {
        let (bx, by) = self.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let it = self.view.iter_at_location(bx, by)?;
        let at = it.offset() as usize;
        let styled = self.styled.borrow();
        if self.can_edit()
            && let Some((start, done)) = styled.tasks.iter().find(|(s, _)| (*s..*s + 3).contains(&at))
        {
            return Some(Target::Task(*start, *done));
        }
        if let Some(l) = styled.links.iter().find(|l| (l.start..l.end).contains(&at)) {
            return Some(Target::Link(l.url.clone()));
        }
        let audio = styled.images.iter().find(|l| (l.start..l.end).contains(&at))?;
        let id = audio.url.strip_prefix("att:")?;
        let not_picture = matches!(self.textures.borrow().get(id), Some(None));
        not_picture.then(|| Target::Recording(id.to_owned()))
    }

    /// Ticks or unticks the checkbox whose `[` is at `start`.
    fn tick(&self, start: usize, done: bool) {
        let s = start as i32 + 1;
        self.edit(|b| {
            b.delete(&mut b.iter_at_offset(s), &mut b.iter_at_offset(s + 1));
            b.insert(&mut b.iter_at_offset(s), if done { " " } else { "x" });
        });
    }

    /// A click on a checkbox ticks it; on a link, opens it. True when it did something.
    fn click(self: &Rc<Self>, x: f64, y: f64) -> bool {
        if self.buffer.has_selection() {
            return false;
        }
        match self.target_at(x, y) {
            Some(Target::Task(start, done)) => {
                self.tick(start, done);
                true
            }
            Some(Target::Link(url)) => {
                let Some(main) = self.main.upgrade() else { return false };
                if let Some(title) = url.strip_prefix("wiki:") {
                    let found = main.store.tree.borrow().notes.iter().find(|n| n.title.trim().eq_ignore_ascii_case(title.trim())).map(|n| n.id.clone());
                    match found {
                        Some(id) => main.open_note(&id, false),
                        None => main.toast(&format!("No note called “{title}”")),
                    }
                } else if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
                    gtk::UriLauncher::new(&url).launch(Some(&main.window), gio::Cancellable::NONE, |_| {});
                }
                true
            }
            Some(Target::Recording(id)) => {
                self.play(id);
                true
            }
            None => false,
        }
    }

    fn connect_buttons(self: &Rc<Self>, styles: &[(gtk::Button, &'static str)], inline: [&gtk::Button; 4], blocks: [&gtk::Button; 4], photo: &gtk::Button) {
        let on = |b: &gtk::Button, f: Box<dyn Fn(&Editor)>| {
            let weak = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(e) = weak.upgrade() {
                    f(&e);
                    e.view.grab_focus();
                }
            });
        };
        on(&self.undo_button, Box::new(|e| e.undo(false)));
        on(&self.redo_button, Box::new(|e| e.undo(true)));
        for (b, prefix) in styles {
            let p = *prefix;
            on(b, Box::new(move |e| e.set_block(p)));
        }
        for (b, m) in inline.iter().zip(["**", "*", "~~", "`"]) {
            on(b, Box::new(move |e| e.wrap(m)));
        }
        for (b, p) in blocks.iter().zip(["- [ ] ", "- ", "1. ", "> "]) {
            on(b, Box::new(move |e| e.set_block(p)));
        }
        let weak = Rc::downgrade(self);
        photo.connect_clicked(move |_| {
            let Some(e) = weak.upgrade() else { return };
            let Some(main) = e.main.upgrade() else { return };
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Pictures"));
            filter.add_mime_type("image/*");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title("Add Photo").filters(&filters).modal(true).build();
            let e = e.clone();
            dialog.open(Some(&main.window), gio::Cancellable::NONE, move |file| {
                if let Ok(file) = file {
                    glib::spawn_future_local(async move { e.upload_file(&file).await });
                }
            });
        });
    }

    async fn upload_file(self: &Rc<Self>, file: &gio::File) {
        let Ok((bytes, _)) = file.load_contents_future().await else {
            self.store.toast("Couldn't read that file");
            return;
        };
        let name = file.basename().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".into());
        let (mime, _) = gio::content_type_guess(Some(&name), &bytes[..]);
        let mime = gio::content_type_get_mime_type(&mime).map(|m| m.to_string()).unwrap_or_else(|| "application/octet-stream".into());
        self.upload(&name, &mime, bytes.to_vec()).await;
    }

    /// Uploads a file and links it on its own line at the cursor.
    async fn upload(self: &Rc<Self>, name: &str, mime: &str, bytes: Vec<u8>) {
        if !self.can_edit() {
            return;
        }
        let api = self.store.api.clone();
        let (id, name2, mime2) = (self.id.clone(), name.to_owned(), mime.to_owned());
        match bg(async move { api.upload(&id, &name2, &mime2, bytes).await }).await {
            Ok(att) => {
                let alt: String = name.rsplit_once('.').map_or(name, |(a, _)| a).chars().filter(|c| !"[]".contains(*c)).collect();
                self.edit(|b| {
                    let mut at = b.iter_at_mark(&b.get_insert());
                    let before = if at.starts_line() { "" } else { "\n" };
                    b.insert(&mut at, &format!("{before}![{alt}](att:{})\n", att.id));
                });
            }
            Err(e) => self.store.toast(&format!("Couldn't add the file: {e}")),
        }
    }

    fn play(&self, id: String) {
        if let Some((playing, media)) = &*self.player.borrow()
            && *playing == id
        {
            media.set_playing(!media.is_playing());
            return;
        }
        let api = self.store.api.clone();
        let weak = self.me.clone();
        glib::spawn_future_local(async move {
            let att = id.clone();
            let Ok(bytes) = bg(async move { api.attachment(&att).await }).await else { return };
            let Some(e) = weak.upgrade() else { return };
            let path = crate::cache::data_dir().join("playing");
            if std::fs::write(&path, &bytes).is_err() {
                return;
            }
            let media = gtk::MediaFile::for_filename(&path);
            media.play();
            e.player.replace(Some((id, media)));
        });
    }

    fn queue_restyle(&self) {
        if self.style_queued.replace(true) {
            return;
        }
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(e) = weak.upgrade() {
                e.style_queued.set(false);
                e.restyle();
            }
        });
    }

    /// Styles the Markdown in place and lays out photos.
    fn restyle(&self) {
        let (start, end) = self.buffer.bounds();
        let text = self.buffer.text(&start, &end, true);
        let styled = markdown::analyze(&text);
        for name in markdown::TAGS {
            self.buffer.remove_tag_by_name(name, &start, &end);
        }
        for tag in self.photo_tags.borrow().values() {
            self.buffer.remove_tag(tag, &start, &end);
        }
        let at = |o: usize| self.buffer.iter_at_offset(o as i32);
        for s in &styled.spans {
            self.buffer.apply_tag_by_name(s.tag, &at(s.start), &at(s.end));
        }
        let mut pictures = Vec::new();
        let column = self.view.column_width() as f32;
        for img in &styled.images {
            let Some(id) = img.url.strip_prefix("att:") else { continue };
            let texture = self.textures.borrow().get(id).cloned();
            match texture {
                None => self.load_picture(id),
                Some(Some(t)) => {
                    let w = (t.width() as f32).min(column).min(MAX_PHOTO);
                    let h = (w * t.height() as f32 / t.width().max(1) as f32).round();
                    let tag = self.photo_tag(h as i32 + 16);
                    let mut line_start = at(img.start);
                    line_start.set_line_offset(0);
                    let mut line_end = at(img.end);
                    if !line_end.ends_line() {
                        line_end.forward_to_line_end();
                    }
                    self.buffer.apply_tag(&tag, &line_start, &line_end);
                    pictures.push(Picture { at: img.end, texture: t, width: w, height: h });
                }
                Some(None) => {}
            }
        }
        self.view.set_pictures(pictures);
        self.styled.replace(styled);
    }

    fn photo_tag(&self, space: i32) -> gtk::TextTag {
        self.photo_tags
            .borrow_mut()
            .entry(space)
            .or_insert_with(|| self.buffer.create_tag(None, &[("pixels-below-lines", &space)]).expect("tag"))
            .clone()
    }

    fn load_picture(&self, id: &str) {
        self.textures.borrow_mut().insert(id.to_owned(), None);
        let api = self.store.api.clone();
        let weak = self.me.clone();
        let id = id.to_owned();
        glib::spawn_future_local(async move {
            let att = id.clone();
            let bytes = bg(async move { api.attachment(&att).await }).await;
            let Some(e) = weak.upgrade() else { return };
            let texture = bytes.ok().and_then(|b| gdk::Texture::from_bytes(&glib::Bytes::from_owned(b)).ok());
            let ok = texture.is_some();
            e.textures.borrow_mut().insert(id, texture);
            if ok {
                e.queue_restyle();
            }
        });
    }

    /// Colors for the Markdown tags, for the light or dark style.
    fn color_tags(&self) {
        // The text's own color, so tags follow the style in use rather than a guess at it.
        let fg = self.view.color();
        let dark = fg.red() + fg.green() + fg.blue() > 1.5;
        let dim = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.45);
        let accent = adw::StyleManager::default().accent_color().to_standalone_rgba(dark);
        let code_bg = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), if dark { 0.08 } else { 0.06 });
        let table = self.buffer.tag_table();
        let tag = |name: &str| table.lookup(name).expect("tag");
        let set = |name: &str, props: &[(&str, &dyn ToValue)]| {
            let t = tag(name);
            for (k, v) in props {
                t.set_property_from_value(k, &v.to_value());
            }
        };
        set("h1", &[("scale", &1.75f64), ("weight", &800i32), ("pixels-above-lines", &10i32), ("pixels-below-lines", &6i32)]);
        set("h2", &[("scale", &1.4f64), ("weight", &700i32), ("pixels-above-lines", &14i32), ("pixels-below-lines", &4i32)]);
        set("h3", &[("scale", &1.18f64), ("weight", &700i32), ("pixels-above-lines", &10i32), ("pixels-below-lines", &2i32)]);
        set("bold", &[("weight", &700i32)]);
        set("italic", &[("style", &gtk::pango::Style::Italic)]);
        set("strike", &[("strikethrough", &true)]);
        set("code", &[("family", &"monospace"), ("background-rgba", &code_bg)]);
        set("codeblock", &[("family", &"monospace"), ("paragraph-background-rgba", &code_bg)]);
        set("table", &[("family", &"monospace")]);
        set("quote", &[("foreground-rgba", &dim)]);
        set("mark", &[("foreground-rgba", &dim)]);
        set("bullet", &[("foreground-rgba", &dim), ("weight", &700i32)]);
        set("link", &[("foreground-rgba", &accent), ("underline", &gtk::pango::Underline::Single)]);
        set("task", &[("foreground-rgba", &accent), ("family", &"monospace"), ("weight", &700i32)]);
        set("done", &[("foreground-rgba", &dim), ("strikethrough", &true)]);
        // Higher tags win: marks stay dim inside headings and links.
        for name in ["link", "mark", "task"] {
            let t = tag(name);
            t.set_priority(table.size() - 1);
        }
    }

    // What gtk/scripts/e2e.mjs does and looks at, through ui/drive.rs.

    /// The text as the editor shows it.
    pub fn shown_text(&self) -> String {
        let (s, e) = self.buffer.bounds();
        self.buffer.text(&s, &e, true).to_string()
    }

    /// Content has arrived and the server has answered the join.
    pub fn loaded(&self) -> bool {
        self.doc.borrow().has_data && self.role.get().is_some()
    }

    pub fn editable(&self) -> bool {
        self.view.is_editable()
    }

    /// Types `s` at the end of the note, as from the keyboard.
    pub fn type_text(&self, s: &str) {
        self.view.grab_focus();
        self.buffer.place_cursor(&self.buffer.end_iter());
        for (i, line) in s.split('\n').enumerate() {
            if i > 0 {
                self.press_enter();
            }
            self.buffer.insert_interactive_at_cursor(line, self.view.is_editable());
        }
    }

    /// Enter, with list items carried on.
    pub fn press_enter(&self) {
        if !self.continue_list() {
            self.buffer.insert_interactive_at_cursor("\n", self.view.is_editable());
        }
    }

    pub fn run_undo(&self, redo: bool) {
        self.undo(redo);
    }

    /// Clicks the `n`th checkbox (from 0).
    pub fn toggle_task(&self, n: usize) -> bool {
        self.restyle();
        let Some((start, done)) = self.styled.borrow().tasks.get(n).copied() else { return false };
        self.tick(start, done);
        true
    }

    /// The format bar's styles by name, on the line at the end.
    pub fn format(&self, name: &str) {
        self.buffer.place_cursor(&self.buffer.end_iter());
        match name {
            "title" => self.set_block("# "),
            "heading" => self.set_block("## "),
            "subheading" => self.set_block("### "),
            "body" => self.set_block(""),
            "check" => self.set_block("- [ ] "),
            "bullet" => self.set_block("- "),
            "number" => self.set_block("1. "),
            "quote" => self.set_block("> "),
            "bold" => self.wrap("**"),
            "italic" => self.wrap("*"),
            _ => eprintln!("drive: unknown format {name}"),
        }
    }

    pub fn peer_names(&self) -> Vec<String> {
        self.doc.borrow().peers().into_iter().map(|p| p.name).collect()
    }

    pub fn banner(&self) -> Option<String> {
        self.banner.is_revealed().then(|| self.banner.title().to_string())
    }

    /// Leaves the note: drops presence, saves, and discards it if it was left blank.
    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        for t in self.timers.take() {
            t.remove();
        }
        if let Some(t) = self.save_timer.take() {
            t.remove();
        }
        if let Some((_, media)) = self.player.take() {
            media.pause();
        }
        self.doc.borrow().clear_presence();
        self.take_outbox();
        let doc = self.doc.borrow();
        let owner = self.role.get().or_else(|| self.store.note(&self.id).map(|n| n.role)) == Some(Role::Owner);
        let queued = self.store.is_queued(&self.id);
        // Like Apple Notes, a note left blank doesn't stick around.
        let blank = doc.has_data && !self.lost.get() && (owner || queued) && doc.text().trim().is_empty();
        drop(doc);
        if !blank {
            self.save();
        }
        if let Some(h) = self.handler.take() {
            self.store.close(&self.id, &h);
        }
        if blank {
            self.store.cache.forget(&self.id);
            self.store.forget_note(&self.id);
            if !queued {
                let api = self.store.api.clone();
                let id = self.id.clone();
                glib::spawn_future_local(async move {
                    let _ = bg(async move { api.discard_note(&id).await }).await;
                });
            }
        } else if self.dirty.get() && self.store.sync.is_online() {
            // Left before the server confirmed it has everything: check, and push the rest.
            self.store.flush_note(&self.id);
        }
    }
}

enum Target {
    Task(usize, bool),
    Link(String),
    Recording(String),
}
