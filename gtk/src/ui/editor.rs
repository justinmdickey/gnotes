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
    noteview::{self, NoteView, Picture, RemoteCursor},
    sidebar::{self, Item, title_of},
    styling,
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
    /// The "/" menu and emoji list, and whether a check of what's typed is due.
    completer: Rc<super::complete::Completer>,
    complete_queued: Cell<bool>,
    colon_typed: Cell<bool>,
    /// The lines whose Markdown marks show, around the cursor while the text has focus.
    active: Cell<Option<(i32, i32)>>,
    styled: RefCell<Styled>,
    timers: RefCell<Vec<glib::SourceId>>,
    title: adw::WindowTitle,
    peers_box: gtk::Box,
    banner: adw::Banner,
    undo_button: gtk::Button,
    redo_button: gtk::Button,
    format_bar: gtk::Widget,
    style_button: gtk::MenuButton,
    share_button: gtk::Button,
    menu_button: gtk::MenuButton,
    /// Whose editing session each Loro peer was, from the server, and when that was last asked.
    authors: RefCell<HashMap<String, crate::api::Author>>,
    authors_asked: Cell<i64>,
    authors_retry: Cell<bool>,
    /// Lines others wrote: where each starts, and who and when, for its tooltip.
    blame: RefCell<Vec<(i32, String)>>,
    /// A voice memo being recorded, and its bar.
    recording: RefCell<Option<Recording>>,
    /// Where the memo goes; kept after Done until it's in the note, as the last words come in.
    spot: RefCell<Option<Spot>>,
    /// A memo is being saved into the note: closing waits for it, so leaving right after Done keeps it.
    saving: Cell<bool>,
    close_after_save: Cell<bool>,
    rec_bar: gtk::Box,
    rec_time: gtk::Label,
    rec_status: gtk::Label,
    /// Tables and boards drawn as widgets over their Markdown.
    blocks: RefCell<Vec<BlockView>>,
    next_block: Cell<u64>,
    /// The block being dragged by its grip, as lines.
    dragging: Cell<Option<(i32, i32)>>,
    block_check: Cell<bool>,
    placing_blocks: Cell<bool>,
    /// A block whose widget changed its shape (a card added, removed or moved): drawn again even
    /// while something in it has focus.
    redraw_block: Cell<Option<u64>>,
    spare_slots: RefCell<Vec<gtk::Box>>,
    /// The start of the table or board being edited as its Markdown (Edit as Text), if one is.
    text_block: RefCell<Option<gtk::TextMark>>,
    /// Where the cursor was, to know which way it was going.
    last_cursor: Cell<usize>,
    /// Attachments shown in the note, by id.
    media: RefCell<HashMap<String, Media>>,
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

/// The paragraph styles the format bar offers, with the Markdown that starts their line.
const PARAGRAPH_STYLES: [(&str, &str); 4] = [("Title", "# "), ("Heading", "## "), ("Subheading", "### "), ("Body", "")];

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
        styling::make_tags(&buffer);

        // External: the view may be wider than the window for a moment, while tables and boards
        // catch up with a narrower window, without holding the window at their width.
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vexpand(true)
            .child(&view)
            .build();

        let title = adw::WindowTitle::new(sidebar::UNTITLED, "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();
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
        header.pack_end(&menu_button);
        header.pack_end(&share_button);
        header.pack_end(&peers_box);

        // The format bar, like the PWA's: undo, text styles, emphasis, lists, photo.
        let bar = gtk::Box::builder().spacing(2).css_classes(["format-bar", "toolbar"]).halign(gtk::Align::Center).build();
        let undo_button = icon_button("edit-undo-symbolic", "Undo (Ctrl+Z)");
        let redo_button = icon_button("edit-redo-symbolic", "Redo (Ctrl+Shift+Z)");
        // Narrow windows keep the formatting; undo is on the keyboard, photos and memos in the note menu.
        let history = gtk::Box::builder().spacing(2).build();
        history.append(&undo_button);
        history.append(&redo_button);
        history.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        bar.append(&history);
        // The paragraph style, named for the line the cursor is on.
        let styles = gio::Menu::new();
        for (label, prefix) in PARAGRAPH_STYLES {
            let item = gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(Some("format.style"), Some(&prefix.to_variant()));
            styles.append_item(&item);
        }
        let style_button = gtk::MenuButton::builder()
            .label("Body")
            .tooltip_text("Paragraph Style")
            .menu_model(&styles)
            .css_classes(["flat", "paragraph-style"])
            .build();
        bar.append(&style_button);
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let bold = icon_button("format-text-bold-symbolic", "Bold (Ctrl+B)");
        let italic = icon_button("format-text-italic-symbolic", "Italic (Ctrl+I)");
        let strike = icon_button("format-text-strikethrough-symbolic", "Strikethrough (Ctrl+Shift+X)");
        let code = gtk::Button::builder().label("</>").tooltip_text("Code").css_classes(["flat", "code-button"]).build();
        bar.append(&bold);
        bar.append(&italic);
        // Strikethrough and code are typed as Markdown on narrow windows.
        let extra = gtk::Box::builder().spacing(2).build();
        extra.append(&strike);
        extra.append(&code);
        bar.append(&extra);
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let check = icon_button("checkbox-checked-symbolic", "Checklist (Ctrl+Shift+L)");
        let bullets = icon_button("view-list-bullet-symbolic", "Bulleted List (Ctrl+Shift+8)");
        let numbers = icon_button("view-list-ordered-symbolic", "Numbered List (Ctrl+Shift+7)");
        let quote = gtk::Button::builder().label("“").tooltip_text("Quote").css_classes(["flat", "quote-button"]).build();
        for b in [&check, &bullets, &numbers, &quote] {
            bar.append(b);
        }
        let media = gtk::Box::builder().spacing(2).build();
        media.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let photo = icon_button("camera-photo-symbolic", "Add Photo");
        media.append(&photo);
        let mic = icon_button("audio-input-microphone-symbolic", "Voice Memo");
        media.append(&mic);
        bar.append(&media);
        let bar_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .child(&bar)
            .build();
        // What fits on the bar at its width and the font in use: undo and media go first, then
        // strikethrough and code.
        {
            let (bar, history, extra, media) = (bar.clone(), history.clone(), extra.clone(), media.clone());
            let queued = Rc::new(Cell::new(false));
            bar_scroll.hadjustment().connect_page_size_notify(move |adj| {
                if queued.replace(true) {
                    return;
                }
                let (bar, history, extra, media, queued) = (bar.clone(), history.clone(), extra.clone(), media.clone(), queued.clone());
                let room = adj.page_size() as i32;
                glib::idle_add_local_once(move || {
                    queued.set(false);
                    let fits = || bar.measure(gtk::Orientation::Horizontal, -1).1 <= room;
                    for part in [&history, &extra, &media] {
                        part.set_visible(true);
                    }
                    if !fits() {
                        history.set_visible(false);
                        media.set_visible(false);
                        if !fits() {
                            extra.set_visible(false);
                        }
                    }
                });
            });
        }
        bar_scroll.set_propagate_natural_width(false);

        let banner = adw::Banner::new("");
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&bar_scroll);
        toolbar.add_top_bar(&banner);
        // The recording bar floats over the note, so a live transcript can be read as it's written.
        let rec_time = gtk::Label::builder().label("0:00").css_classes(["numeric"]).build();
        let rec_status = gtk::Label::builder().css_classes(["dim-label"]).build();
        let rec_cancel = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text("Discard Recording").css_classes(["flat", "circular"]).build();
        let rec_stop = gtk::Button::builder().label("Done").css_classes(["suggested-action", "pill"]).build();
        let rec_dot = gtk::Label::builder().label("●").css_classes(["recording-dot"]).build();
        let rec_bar = gtk::Box::builder()
            .spacing(10)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .margin_bottom(18)
            .css_classes(["recording-bar", "toolbar", "osd"])
            .visible(false)
            .build();
        for w in [rec_dot.upcast_ref::<gtk::Widget>(), rec_time.upcast_ref(), rec_status.upcast_ref(), rec_cancel.upcast_ref(), rec_stop.upcast_ref()] {
            rec_bar.append(w);
        }
        let overlay = gtk::Overlay::builder().child(&scroll).build();
        overlay.add_overlay(&rec_bar);
        toolbar.set_content(Some(&overlay));
        toolbar.set_top_bar_style(adw::ToolbarStyle::Raised);

        let page = adw::NavigationPage::builder().title(sidebar::UNTITLED).tag(format!("note:{id}")).child(&toolbar).build();

        let mut doc = NoteDoc::new();
        let me = store.user();
        doc.set_user(&me.display_name, color_for(&me.id));
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
            completer: super::complete::Completer::new(&view),
            complete_queued: Cell::new(false),
            colon_typed: Cell::new(false),
            active: Cell::new(None),
            styled: RefCell::new(Styled::default()),
            timers: RefCell::new(Vec::new()),
            title,
            peers_box,
            banner,
            undo_button: undo_button.clone(),
            redo_button: redo_button.clone(),
            format_bar: bar_scroll.upcast(),
            style_button: style_button.clone(),
            share_button: share_button.clone(),
            menu_button,
            media: Default::default(),
            blocks: Default::default(),
            next_block: Cell::new(0),
            dragging: Cell::new(None),
            block_check: Cell::new(false),
            placing_blocks: Cell::new(false),
            redraw_block: Cell::new(None),
            spare_slots: Default::default(),
            text_block: Default::default(),
            last_cursor: Cell::new(0),
            recording: Default::default(),
            spot: Default::default(),
            saving: Cell::new(false),
            close_after_save: Cell::new(false),
            rec_bar: rec_bar.clone(),
            rec_time,
            rec_status,
            authors: Default::default(),
            authors_asked: Cell::new(0),
            authors_retry: Cell::new(false),
            blame: Default::default(),
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

        {
            let weak = Rc::downgrade(&editor);
            editor.completer.on_pick(move |pick| {
                if let Some(e) = weak.upgrade() {
                    e.apply_pick(pick);
                }
            });
        }
        {
            let weak = Rc::downgrade(&editor);
            editor.view.on_width(move || {
                if let Some(e) = weak.upgrade() {
                    e.queue_restyle();
                }
            });
            // A narrower window narrows the scrolled window before the view, which tables and
            // boards hold at their width until they're laid out again.
            let weak = Rc::downgrade(&editor);
            scroll.hadjustment().connect_page_size_notify(move |_| {
                if let Some(e) = weak.upgrade() {
                    e.queue_restyle();
                }
            });
        }
        editor.connect_buffer();
        editor.connect_view();
        editor.connect_buttons(&style_button, [&bold, &italic, &strike, &code], [&check, &bullets, &numbers, &quote], &photo);
        for (button, save) in [(&mic, None), (&rec_stop, Some(true)), (&rec_cancel, Some(false))] {
            let weak = Rc::downgrade(&editor);
            button.connect_clicked(move |_| {
                let Some(e) = weak.upgrade() else { return };
                match save {
                    None => e.start_recording(),
                    Some(save) => e.stop_recording(save),
                }
            });
        }
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
            // The AI tools come and go with the server's services.
            let weak = Rc::downgrade(&editor);
            store.on_features(move || match weak.upgrade() {
                Some(e) if !e.closed.get() => {
                    e.refresh_editable();
                    true
                }
                _ => false,
            });
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
            // Light or dark, a new accent or monospace font: after the new style reaches the view.
            let restyle = {
                let weak = Rc::downgrade(&editor);
                move |_: &adw::StyleManager| {
                    let weak = weak.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(e) = weak.upgrade() {
                            e.color_tags();
                            e.restyle();
                        }
                    });
                }
            };
            let manager = adw::StyleManager::default();
            manager.connect_dark_notify(restyle.clone());
            manager.connect_accent_color_notify(restyle.clone());
            manager.connect_monospace_font_name_notify(restyle);
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
        let ai = gio::Menu::new();
        let rest = gio::Menu::new();
        menu.append_section(None, &ai);
        menu.append_section(None, &rest);
        let action = |section: &gio::Menu, label: &str, name: &str, f: Box<dyn Fn(&Rc<Main>, &str)>| {
            let a = gio::SimpleAction::new(name, None);
            let (weak, id) = (self.main.clone(), self.id.clone());
            a.connect_activate(move |_, _| {
                if let Some(m) = weak.upgrade() {
                    f(&m, &id);
                }
            });
            group.add_action(&a);
            section.append(Some(label), Some(&format!("note.{name}")));
        };
        // The AI tools, with AI Summaries set up, on the note open now.
        let open = |f: fn(&Rc<Main>, &Rc<Editor>)| -> Box<dyn Fn(&Rc<Main>, &str)> {
            Box::new(move |m, id| {
                if let Some(e) = m.editor().filter(|e| e.id == id) {
                    f(m, &e);
                }
            })
        };
        if self.store.features.borrow().summaries && self.doc.borrow().has_data && !self.lost.get() {
            action(&ai, "Summary", "summary", open(super::ai::summary_dialog));
            if self.can_edit() {
                action(&ai, "Edit with AI…", "edit-ai", open(|m, e| super::ai::rewrite_dialog(m, e, super::ai::Rewrite::Edit)));
                action(&ai, "Tidy Up", "tidy", open(|m, e| super::ai::rewrite_dialog(m, e, super::ai::Rewrite::Tidy)));
                action(&ai, "Write with AI Here…", "ai-prompt", open(super::ai::prompt_popover));
            }
        }
        let add = |label: &str, name: &str, f: Box<dyn Fn(&Rc<Main>, &str)>| action(&rest, label, name, f);
        if role.is_some_and(Role::can_edit) {
            add("Move to…", "move", Box::new(|m, id| {
                if let Some(n) = m.store.note(id) {
                    dialogs::move_to(m, &Item::Note(n));
                }
            }));
        }
        if role.is_some_and(Role::can_edit) {
            let media = gio::Menu::new();
            menu.insert_section(1, None, &media);
            action(&media, "Add Photo…", "photo", open(|_, e| e.pick_photo()));
            action(&media, "Record Voice Memo", "record", open(|_, e| e.start_recording()));
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
            if text.ends_with(':') {
                e.colon_typed.set(true);
            }
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
        self.queue_complete();
    }

    /// Looks at what's before the cursor once the buffer has settled: the "/" menu, emoji.
    fn queue_complete(&self) {
        if self.complete_queued.replace(true) {
            return;
        }
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(e) = weak.upgrade() {
                e.complete_queued.set(false);
                e.check_complete();
            }
        });
    }

    fn check_complete(self: &Rc<Self>) {
        let colon = self.colon_typed.replace(false);
        if !self.view.is_focus() || !self.can_edit() || self.buffer.has_selection() {
            self.completer.hide();
            return;
        }
        let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let at = cursor.offset() as usize;
        // Code keeps its slashes and colons.
        let in_code = self.styled.borrow().spans.iter().any(|s| (s.tag == "codeblock" || s.tag == "code") && s.start < at && at <= s.end);
        if in_code {
            self.completer.hide();
            return;
        }
        let mut start = cursor;
        start.set_line_offset(0);
        let before = self.buffer.text(&start, &cursor, true).to_string();
        // Like Slack: the closing colon of a whole ":name:" makes it the emoji.
        if colon && let Some((emoji, n)) = super::complete::emoji_on_colon(&before) {
            self.completer.hide();
            let from = at as i32 - n as i32;
            self.edit(|b| {
                b.delete(&mut b.iter_at_offset(from), &mut b.iter_at_offset(at as i32));
                b.insert(&mut b.iter_at_offset(from), emoji);
            });
            return;
        }
        let open_emoji = self.completer.is_open() && self.completer.kind.get() == super::complete::Kind::Emoji;
        let Some((kind, back, query)) = super::complete::trigger_with(&before, if open_emoji { 0 } else { 2 }) else {
            self.completer.hide();
            return;
        };
        let items = match kind {
            super::complete::Kind::Slash => {
                let ai = self.store.features.borrow().summaries;
                super::complete::slash_items(&query, ai)
            }
            super::complete::Kind::Emoji => super::complete::emoji_items(&query),
        };
        if let Some((_, rect)) = self.cursor_rect() {
            self.completer.show(&rect, kind, back, items);
        }
    }

    /// Runs a pick from the "/" menu or the emoji list, in place of what was typed for it.
    fn apply_pick(self: &Rc<Self>, pick: super::complete::Pick) {
        use super::complete::Pick;
        let at = self.cursor_offset() as i32;
        let from = at - self.completer.back.get() as i32;
        self.edit(|b| b.delete(&mut b.iter_at_offset(from.max(0)), &mut b.iter_at_offset(at)));
        let Some(main) = self.main.upgrade() else { return };
        match pick {
            Pick::Block(p) => self.set_block(p),
            Pick::CodeBlock => self.insert_block("```\n\n```", 1),
            Pick::Table => self.insert_block(super::tables::TABLE_TEMPLATE, 0),
            Pick::Board => self.insert_block(super::tables::BOARD_TEMPLATE, 2),
            Pick::Emoji => {
                let at = self.cursor_offset() as i32;
                let after_text = at > 0 && !self.buffer.iter_at_offset(at - 1).char().is_whitespace();
                self.edit(|b| b.insert(&mut b.iter_at_offset(at), if after_text { " :" } else { ":" }));
                if let Some((_, rect)) = self.cursor_rect() {
                    self.completer.show(&rect, super::complete::Kind::Emoji, 1, super::complete::emoji_items(""));
                }
            }
            Pick::Photo => self.pick_photo(),
            Pick::Voice => self.start_recording(),
            Pick::Prompt => super::ai::prompt_popover(&main, self),
            Pick::Tidy => super::ai::rewrite_dialog(&main, self, super::ai::Rewrite::Tidy),
            Pick::Text(t) => {
                let at = self.cursor_offset() as i32;
                self.edit(|b| b.insert(&mut b.iter_at_offset(at), &t));
            }
        }
        self.view.grab_focus();
    }

    /// Puts a block (code, table, board) on lines of its own at the cursor, with the cursor on its
    /// line `line`: in place of an empty line, else under the cursor's line.
    fn insert_block(&self, template: &str, line: i32) {
        let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let mut start = cursor;
        start.set_line_offset(0);
        let mut end = cursor;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let empty = self.buffer.text(&start, &end, true).trim().is_empty();
        let (s, e) = (start.offset(), end.offset());
        let first_line = if empty { start.line() } else { start.line() + 2 };
        self.edit(|b| {
            if empty {
                b.delete(&mut b.iter_at_offset(s), &mut b.iter_at_offset(e));
                b.insert(&mut b.iter_at_offset(s), template);
            } else {
                b.insert(&mut b.iter_at_offset(e), &format!("\n\n{template}"));
            }
        });
        if let Some(mut it) = self.buffer.iter_at_line(first_line + line) {
            if !it.ends_line() {
                it.forward_to_line_end();
            }
            self.buffer.place_cursor(&it);
        }
    }

    /// The lines the cursor and selection are on while the text has focus: their marks show.
    fn active_lines(&self) -> Option<(i32, i32)> {
        if !self.view.is_focus() || !self.view.is_editable() {
            return None;
        }
        let a = self.buffer.iter_at_mark(&self.buffer.get_insert()).line();
        let b = self.buffer.iter_at_mark(&self.buffer.selection_bound()).line();
        Some((a.min(b), a.max(b)))
    }

    /// Opens the note menu, or the paragraph styles, for screenshots.
    pub fn open_menu(&self, styles: bool) {
        if styles { self.style_button.popup() } else { self.menu_button.popup() }
    }

    /// The style the format bar names. For tests.
    pub fn paragraph_style(&self) -> String {
        self.style_button.label().map(|l| l.to_string()).unwrap_or_default()
    }

    /// Names the cursor's line's style on the format bar.
    fn show_paragraph_style(&self) {
        let mut start = self.buffer.iter_at_mark(&self.buffer.get_insert());
        start.set_line_offset(0);
        let mut end = start;
        end.forward_chars(4);
        let head = self.buffer.text(&start, &end, true);
        let name = PARAGRAPH_STYLES.iter().find(|(_, p)| !p.is_empty() && head.starts_with(p)).map_or("Body", |(n, _)| n);
        if self.style_button.label().as_deref() != Some(name) {
            self.style_button.set_label(name);
        }
    }

    /// Tells the others where the cursor is, while the text has focus.
    /// Keeps the cursor out of the Markdown under a table or board, as the PWA does: it steps over
    /// the block, down or up the way it was going. Edit as Text lets it in.
    fn skip_blocks(&self) {
        let insert = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let here = insert.offset() as usize;
        let before = self.last_cursor.replace(here);
        if !self.view.is_focus() || self.buffer.has_selection() || self.text_block.borrow().is_some() {
            return;
        }
        let len = self.buffer.char_count() as usize;
        let Some((s, e)) = self.blocks.borrow().iter().map(|b| (b.start, b.end.min(len))).find(|&(s, e)| here >= s && here <= e) else { return };
        let weak = self.me.clone();
        // Not from inside the mark-set handler this runs in.
        glib::idle_add_local_once(move || {
            let Some(ed) = weak.upgrade() else { return };
            let b = &ed.buffer;
            let target = if here >= before {
                let mut it = b.iter_at_offset(e as i32);
                if !it.forward_line() && ed.can_edit() {
                    // Nothing after it: a line to type on.
                    b.insert(&mut b.end_iter(), "\n");
                    it = b.end_iter();
                }
                it
            } else {
                let mut it = b.iter_at_offset(s as i32);
                it.backward_char();
                it
            };
            b.place_cursor(&target);
        });
    }

    fn announce_cursor(&self) {
        self.skip_blocks();
        // Live preview: marks show on the line being edited.
        let active = self.active_lines();
        if self.active.replace(active) != active {
            self.queue_restyle();
        }
        self.show_paragraph_style();
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
                e.completer.hide();
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

        // A checkbox, link or code label acts on press, like the web app's mousedown, and keeps the
        // press from the text view: otherwise it moves the cursor there first, which shows the
        // line's Markdown and shifts the text out from under the pointer.
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_PRIMARY).propagation_phase(gtk::PropagationPhase::Capture).build();
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |g, n, x, y| {
            let Some(e) = weak.upgrade() else { return };
            // Not a press on a table or board over the text: that's theirs.
            let on_text = e.view.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|w| w == *e.view.upcast_ref::<gtk::Widget>());
            if n == 1 && on_text && e.click(x, y) {
                g.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        self.view.add_controller(click);

        // Pointing beside a marked line says who wrote it.
        self.view.set_has_tooltip(true);
        let weak = Rc::downgrade(self);
        self.view.connect_query_tooltip(move |view, x, y, _, tip| {
            let Some(e) = weak.upgrade() else { return false };
            if x > view.left_margin() {
                return false;
            }
            let (_, by) = view.window_to_buffer_coords(gtk::TextWindowType::Widget, x, y);
            let (it, _) = view.line_at_y(by);
            match e.line_writer(it.line()) {
                Some(w) => {
                    tip.set_text(Some(&w));
                    true
                }
                None => false,
            }
        });

        let right = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let weak = Rc::downgrade(self);
        right.connect_pressed(move |g, _, x, y| {
            if let Some(e) = weak.upgrade()
                && e.attachment_menu(x, y)
            {
                g.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        self.view.add_controller(right);

        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            let Some(e) = weak.upgrade() else { return };
            if e.dragging.get().is_some() {
                return;
            }
            // A grip shows beside the line pointed at, when it's a block that can move.
            let (_, by) = e.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            let (it, _) = e.view.line_at_y(by);
            let block = e.can_edit().then(|| e.block_at(it.line())).flatten();
            e.view.set_grip(block.map(|(first, _)| e.buffer.iter_at_line(first).map_or(0, |i| i.offset() as usize)));
            let on_grip = block.is_some() && (x as i32) < e.view.left_margin() - 8 && (x as i32) > e.view.left_margin() - 40;
            let over = on_grip || e.target_at(x, y).is_some();
            e.view.set_cursor_from_name(Some(if on_grip { "grab" } else if over { "pointer" } else { "text" }));
        });
        let weak = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(e) = weak.upgrade()
                && e.dragging.get().is_none()
            {
                e.view.set_grip(None);
            }
        });
        self.view.add_controller(motion);

        // Dragging the grip moves its block: a line, a list item with those under it, or a whole code
        // block, table or board.
        // Before the text view's own drag, which would start selecting text there.
        let drag = gtk::GestureDrag::builder().propagation_phase(gtk::PropagationPhase::Capture).build();
        let weak = Rc::downgrade(self);
        drag.connect_drag_begin(move |g, x, y| {
            let Some(e) = weak.upgrade() else { return };
            let on_grip = (x as i32) < e.view.left_margin() - 8 && (x as i32) > e.view.left_margin() - 40;
            let (_, by) = e.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            let (it, _) = e.view.line_at_y(by);
            match e.can_edit().then(|| e.block_at(it.line())).flatten() {
                Some(block) if on_grip => {
                    e.dragging.set(Some(block));
                    g.set_state(gtk::EventSequenceState::Claimed);
                }
                _ => {
                    g.set_state(gtk::EventSequenceState::Denied);
                }
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_update(move |g, dx, dy| {
            let Some(e) = weak.upgrade() else { return };
            let Some((sx, sy)) = g.start_point() else { return };
            let _ = dx;
            let target = e.drop_line(sx, sy + dy);
            e.view.set_drop(target.and_then(|l| e.line_start(l)));
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_end(move |g, _, dy| {
            let Some(e) = weak.upgrade() else { return };
            if let Some((sx, sy)) = g.start_point() {
                e.end_drag(sx, sy + dy);
            }
        });
        self.view.add_controller(drag);

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
            // Markdown and zips are notes to import; anything else goes in this note.
            let (notes, attach): (Vec<gio::File>, Vec<gio::File>) = files
                .files()
                .into_iter()
                .partition(|f| f.basename().is_some_and(|n| super::settings::importable(&n.to_string_lossy())));
            if !notes.is_empty()
                && let Some(main) = e.main.upgrade()
            {
                glib::spawn_future_local(async move { super::settings::import(&main, notes).await });
            }
            for file in attach {
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
        // Keys typed in a table's or board's fields are theirs, not the note's.
        if !self.view.has_focus() {
            return glib::Propagation::Proceed;
        }
        let lower = key.to_lower();
        if self.completer.is_open() {
            match key {
                gdk::Key::Up => self.completer.step(-1),
                gdk::Key::Down => self.completer.step(1),
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::Tab => self.completer.pick(),
                gdk::Key::Escape => self.completer.hide(),
                _ => return glib::Propagation::Proceed,
            }
            return stop;
        }
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
        let at = self.view.iter_at_location(bx, by).map_or(usize::MAX, |it| it.offset() as usize);
        let styled = self.styled.borrow();
        if self.can_edit()
            && let Some((start, done)) = styled.tasks.iter().find(|(s, _)| (*s..*s + 3).contains(&at))
        {
            return Some(Target::Task(*start, *done));
        }
        if let Some(l) = styled.links.iter().find(|l| (l.start..l.end).contains(&at)) {
            return Some(Target::Link(l.url.clone()));
        }
        drop(styled);
        let p = self.view.picture_at(bx as f32, by as f32)?;
        p.texture.is_none().then_some(Target::Recording(p.id))
    }

    /// Ticks or unticks the checkbox whose `[` is at `start`.
    fn tick(&self, start: usize, done: bool) {
        let s = start as i32 + 1;
        self.edit(|b| {
            b.delete(&mut b.iter_at_offset(s), &mut b.iter_at_offset(s + 1));
            b.insert(&mut b.iter_at_offset(s), if done { " " } else { "x" });
        });
    }

    /// The language picker of code block `n`, from its label.
    fn pick_language(self: &Rc<Self>, n: usize, x: f64, y: f64) {
        let Some(block) = self.styled.borrow().code_blocks.get(n).cloned() else { return };
        let menu = gio::Menu::new();
        menu.append(Some("Guess"), Some("code.lang::"));
        menu.append(Some("Plain text"), Some(&format!("code.lang::{}", super::code::PLAIN)));
        let langs = gio::Menu::new();
        for (id, name) in super::code::COMMON {
            langs.append(Some(name), Some(&format!("code.lang::{id}")));
        }
        menu.append_section(None, &langs);
        let group = gio::SimpleActionGroup::new();
        let action = gio::SimpleAction::new_stateful("lang", Some(glib::VariantTy::STRING), &block.info.to_variant());
        let weak = self.me.clone();
        action.connect_activate(move |_, v| {
            let (Some(e), Some(lang)) = (weak.upgrade(), v.and_then(|v| v.get::<String>())) else { return };
            e.set_code_language(n, &lang);
        });
        group.add_action(&action);
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.insert_action_group("code", Some(&group));
        popover.set_parent(&self.view);
        popover.set_has_arrow(false);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
    }

    /// Writes a code block's language after its opening fence ("" to let it be guessed).
    pub fn set_code_language(&self, n: usize, lang: &str) {
        let Some(block) = self.styled.borrow().code_blocks.get(n).cloned() else { return };
        let (s, e) = (block.info_range.0 as i32, block.info_range.1 as i32);
        self.edit(|b| {
            b.delete(&mut b.iter_at_offset(s), &mut b.iter_at_offset(e));
            b.insert(&mut b.iter_at_offset(s), lang);
        });
    }

    /// A click on a checkbox ticks it; on a link, opens it. True when it did something.
    fn click(self: &Rc<Self>, x: f64, y: f64) -> bool {
        if self.buffer.has_selection() {
            return false;
        }
        let (bx, by) = self.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        if self.can_edit()
            && let Some(n) = self.view.code_label_at(bx as f32, by as f32)
        {
            self.pick_language(n, x, y);
            return true;
        }
        match self.target_at(x, y) {
            Some(Target::Task(start, done)) => {
                self.tick(start, done);
                true
            }
            Some(Target::Link(url)) => self.follow(&url),
            Some(Target::Recording(id)) => {
                self.play(id);
                true
            }
            None => false,
        }
    }

    /// Opens a link: a [[wiki link]]'s note here, a web or mail link outside.
    fn follow(&self, url: &str) -> bool {
        let Some(main) = self.main.upgrade() else { return false };
        if let Some(title) = url.strip_prefix("wiki:") {
            let found = main.store.tree.borrow().notes.iter().find(|n| n.title.trim().eq_ignore_ascii_case(title.trim())).map(|n| n.id.clone());
            match found {
                Some(id) => main.open_note(&id, false),
                None => main.toast(&format!("No note called “{title}”")),
            }
        } else if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
            gtk::UriLauncher::new(url).launch(Some(&main.window), gio::Cancellable::NONE, |_| {});
        }
        true
    }

    fn connect_buttons(self: &Rc<Self>, styles: &gtk::MenuButton, inline: [&gtk::Button; 4], blocks: [&gtk::Button; 4], photo: &gtk::Button) {
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
        let style = gio::SimpleAction::new("style", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        style.connect_activate(move |_, prefix| {
            if let (Some(e), Some(p)) = (weak.upgrade(), prefix.and_then(|p| p.str())) {
                e.set_block(p);
                e.view.grab_focus();
            }
        });
        let group = gio::SimpleActionGroup::new();
        group.add_action(&style);
        styles.insert_action_group("format", Some(&group));
        for (b, m) in inline.iter().zip(["**", "*", "~~", "`"]) {
            on(b, Box::new(move |e| e.wrap(m)));
        }
        for (b, p) in blocks.iter().zip(["- [ ] ", "- ", "1. ", "> "]) {
            on(b, Box::new(move |e| e.set_block(p)));
        }
        let weak = Rc::downgrade(self);
        photo.connect_clicked(move |_| {
            if let Some(e) = weak.upgrade() {
                e.pick_photo();
            }
        });
    }

    /// Add Photo: picks image files and puts them in at the cursor.
    fn pick_photo(self: &Rc<Self>) {
        {
            let e = self;
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
        }
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

    /// Starts a voice memo: where it goes is picked now, and shown in the bar while recording.
    pub fn start_recording(self: &Rc<Self>) {
        if self.recording.borrow().is_some() || self.spot.borrow().is_some() || !self.can_edit() {
            return;
        }
        // The cursor's line when you were typing, else the end of the note.
        let at = if self.view.is_focus() {
            let mut it = self.buffer.iter_at_mark(&self.buffer.get_insert());
            if !it.ends_line() {
                it.forward_to_line_end();
            }
            it
        } else {
            self.buffer.end_iter()
        };
        let spot = Spot { anchor: self.buffer.create_mark(None, &at, false), from: None, to: None, stretch: None };
        self.spot.replace(Some(Spot { anchor: spot.anchor.clone(), from: None, to: None, stretch: None }));
        let live_on = self.store.features.borrow().live_transcription;
        let (pcm, live) = if live_on {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let (ev_tx, ev_rx) = async_channel::unbounded();
            let task = crate::runtime().spawn(crate::recorder::live(self.store.api.clone(), rx, ev_tx));
            let weak = self.me.clone();
            glib::spawn_future_local(async move {
                let mut warned = false;
                while let Ok(ev) = ev_rx.recv().await {
                    let Some(e) = weak.upgrade() else { return };
                    match ev {
                        crate::recorder::LiveEvent::Delta(t) => e.spot_delta(&t),
                        crate::recorder::LiveEvent::Final(t) => e.spot_final(&t),
                        crate::recorder::LiveEvent::Error(m) if !warned => {
                            warned = true;
                            e.store.toast(&m);
                        }
                        _ => {}
                    }
                }
            });
            (Some(tx), Some(task))
        } else {
            (None, None)
        };
        let rec = match crate::recorder::Recording::start(&crate::cache::data_dir().join("recordings"), pcm) {
            Ok(r) => r,
            Err(e) => {
                self.store.toast(&e);
                self.buffer.delete_mark(&spot.anchor);
                self.spot.take();
                if let Some(t) = live {
                    t.abort();
                }
                return;
            }
        };
        let weak = self.me.clone();
        let timer = glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(e) = weak.upgrade() else { return glib::ControlFlow::Break };
            if let Some(r) = &*e.recording.borrow() {
                let s = r.started.elapsed().as_secs();
                e.rec_time.set_label(&format!("{}:{:02}", s / 60, s % 60));
            }
            glib::ControlFlow::Continue
        });
        self.rec_time.set_label("0:00");
        self.rec_status.set_label(if live_on { "Listening…" } else { "Recording…" });
        self.rec_bar.set_visible(true);
        self.recording.replace(Some(Recording { rec, started: std::time::Instant::now(), timer, live }));
    }

    /// Stops the voice memo: saves it into the note, or throws it away.
    pub fn stop_recording(self: &Rc<Self>, save: bool) {
        let Some(r) = self.recording.take() else { return };
        r.timer.remove();
        if !save {
            r.rec.cancel();
            if let Some(t) = r.live {
                t.abort();
            }
            if let Some(spot) = self.spot.take() {
                self.spot_remove(&spot);
            }
            self.rec_bar.set_visible(false);
            return;
        }
        self.rec_status.set_label("Saving…");
        self.saving.set(true);
        let e = self.clone();
        glib::spawn_future_local(async move {
            e.save_recording(r.rec, r.live).await;
            e.saving.set(false);
            if e.close_after_save.get() {
                e.close();
            }
        });
    }

    async fn save_recording(self: &Rc<Self>, rec: crate::recorder::Recording, live: Option<tokio::task::JoinHandle<()>>) {
        let e = self;
        {
            let bytes = rec.stop().await;
            // The live transcript's last words arrive after the recording stops.
            if let Some(t) = live {
                let _ = t.await;
            }
            e.rec_bar.set_visible(false);
            let bytes = match bytes {
                Ok(b) => b,
                Err(err) => {
                    e.store.toast(&err);
                    if let Some(spot) = e.spot.take() {
                        e.spot_remove(&spot);
                    }
                    return;
                }
            };
            let had_text = e.spot.borrow().as_ref().is_some_and(|s| e.spot_has_text(s));
            let (api, id) = (e.store.api.clone(), e.id.clone());
            let att = match bg(async move { api.upload(&id, "Voice memo.webm", "audio/webm", bytes).await }).await {
                Ok(att) => att,
                Err(err) => {
                    e.store.toast(&format!("Couldn't save the recording: {err}"));
                    if let Some(spot) = e.spot.take() {
                        e.spot_done(&spot);
                    }
                    return;
                }
            };
            if let Some(spot) = e.spot.take() {
                e.spot_embed(spot, &att.id);
            }
            // Without a live transcript, the whole recording is transcribed under it.
            if !had_text && e.store.features.borrow().transcription {
                e.transcribe(&att.id).await;
            }
        }
    }

    /// Makes the transcript's line: an empty line is used as it is (except the title), else a new one after it.
    fn spot_open(&self, spot: &mut Spot) {
        if spot.from.is_some() {
            return;
        }
        let it = self.buffer.iter_at_mark(&spot.anchor);
        let mut start = it;
        start.set_line_offset(0);
        let mut end = it;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let empty = self.buffer.text(&start, &end, true).trim().is_empty() && start.line() > 0;
        let at = if empty {
            start.offset()
        } else {
            let o = end.offset();
            self.buffer.insert(&mut self.buffer.iter_at_offset(o), "\n");
            o + 1
        };
        let it = self.buffer.iter_at_offset(at);
        spot.from = Some(self.buffer.create_mark(None, &it, true));
        spot.to = Some(self.buffer.create_mark(None, &it, false));
        spot.stretch = Some(self.buffer.create_mark(None, &it, true));
    }

    /// `text` with a space before it unless it follows one, so words don't run together.
    fn spaced(&self, spot: &Spot, at: &gtk::TextIter, text: &str) -> String {
        let from = spot.from.as_ref().map(|m| self.buffer.iter_at_mark(m).offset()).unwrap_or(0);
        let before = if at.offset() > from {
            let mut b = *at;
            b.backward_char();
            b.char()
        } else {
            ' '
        };
        if before.is_whitespace() || text.starts_with(char::is_whitespace) { text.to_owned() } else { format!(" {text}") }
    }

    /// More words of a live transcript.
    pub fn spot_delta(&self, text: &str) {
        let mut spot = self.spot.borrow_mut();
        let Some(spot) = spot.as_mut() else { return };
        self.spot_open(spot);
        let to = self.buffer.iter_at_mark(spot.to.as_ref().expect("to"));
        let insert = self.spaced(spot, &to, text);
        self.buffer.insert(&mut self.buffer.iter_at_offset(to.offset()), &insert);
    }

    /// The finished text for the stretch heard so far, in place of its deltas.
    pub fn spot_final(&self, text: &str) {
        let mut spot = self.spot.borrow_mut();
        let Some(spot) = spot.as_mut() else { return };
        self.spot_open(spot);
        let (stretch, to) = (spot.stretch.clone().expect("stretch"), spot.to.clone().expect("to"));
        let s = self.buffer.iter_at_mark(&stretch).offset();
        let t = self.buffer.iter_at_mark(&to).offset();
        let clean = text.trim();
        let insert = if clean.is_empty() { String::new() } else { self.spaced(spot, &self.buffer.iter_at_offset(s), clean) };
        if t > s {
            self.buffer.delete(&mut self.buffer.iter_at_offset(s), &mut self.buffer.iter_at_offset(t));
        }
        self.buffer.insert(&mut self.buffer.iter_at_offset(s), &insert);
        self.buffer.move_mark(&stretch, &self.buffer.iter_at_mark(&to));
    }

    fn spot_has_text(&self, spot: &Spot) -> bool {
        match (&spot.from, &spot.to) {
            (Some(f), Some(t)) => self.buffer.iter_at_mark(t).offset() > self.buffer.iter_at_mark(f).offset(),
            _ => false,
        }
    }

    /// Puts the recording's player at the spot, above the transcript.
    fn spot_embed(&self, mut spot: Spot, id: &str) {
        self.spot_open(&mut spot);
        let at = self.buffer.iter_at_mark(spot.from.as_ref().expect("from")).offset();
        self.edit(|b| b.insert(&mut b.iter_at_offset(at), &format!("![Voice memo](att:{id})\n")));
        self.spot_done(&spot);
    }

    /// Takes back a discarded recording's transcript and its line.
    fn spot_remove(&self, spot: &Spot) {
        if let (Some(f), Some(t)) = (&spot.from, &spot.to) {
            let mut from = self.buffer.iter_at_mark(f);
            let to = self.buffer.iter_at_mark(t);
            // The line it was given, joined back to the one above.
            if from.starts_line() && from.line() > 0 {
                from.backward_char();
            }
            let (a, b) = (from.offset(), to.offset());
            self.buffer.delete(&mut self.buffer.iter_at_offset(a), &mut self.buffer.iter_at_offset(b));
        }
        self.spot_done(spot);
    }

    fn spot_done(&self, spot: &Spot) {
        for m in [Some(&spot.anchor), spot.from.as_ref(), spot.to.as_ref(), spot.stretch.as_ref()].into_iter().flatten() {
            if !m.is_deleted() {
                self.buffer.delete_mark(m);
            }
        }
    }

    /// Puts text on the line under an attachment's line: into the empty line there, or a new one.
    fn write_under(&self, id: &str, text: &str) {
        let all = self.shown_text();
        let needle = format!("att:{id}");
        let Some(line) = all.lines().position(|l| l.contains(&needle)) else { return };
        let Some(mut end) = self.buffer.iter_at_line(line as i32) else { return };
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let next_empty = all.lines().nth(line + 1).is_some_and(|l| l.trim().is_empty());
        let at = end.offset();
        self.edit(|b| {
            if next_empty {
                b.insert(&mut b.iter_at_offset(at + 1), text);
            } else {
                b.insert(&mut b.iter_at_offset(at), &format!("\n{text}"));
            }
        });
    }

    /// Transcribe on a recording: the whole of it, onto the line under it.
    pub async fn transcribe(&self, id: &str) {
        let (api, att) = (self.store.api.clone(), id.to_owned());
        match bg(async move { api.transcribe(&att).await }).await {
            Ok(t) if !t.trim().is_empty() => self.write_under(id, t.trim()),
            Ok(_) => self.store.toast("No speech found in this recording"),
            Err(e) => self.store.toast(&format!("Couldn't transcribe the recording: {e}")),
        }
    }

    /// Get Text on a photo: the text the server reads in it, onto the line under it.
    pub async fn photo_text(&self, id: &str) {
        let (api, att) = (self.store.api.clone(), id.to_owned());
        match bg(async move { api.photo_text(&att).await }).await {
            Ok(t) if !t.trim().is_empty() => self.write_under(id, t.trim()),
            Ok(_) => self.store.toast("No text found in this photo"),
            Err(e) => self.store.toast(&format!("Couldn't read the photo: {e}")),
        }
    }

    /// Takes a photo's or recording's line out of the note, with Undo. Text under it stays.
    pub fn remove_attachment(self: &Rc<Self>, id: &str, what: &str) {
        let all = self.shown_text();
        let needle = format!("att:{id}");
        let Some(line) = all.lines().position(|l| l.contains(&needle)) else { return };
        let Some(start) = self.buffer.iter_at_line(line as i32) else { return };
        let mut end = start;
        end.forward_line();
        let (a, b) = (start.offset(), end.offset());
        self.edit(|buf| buf.delete(&mut buf.iter_at_offset(a), &mut buf.iter_at_offset(b)));
        if let Some(main) = self.main.upgrade() {
            let e = self.clone();
            main.toast_undo(&format!("{what} removed"), move || e.run_undo(false));
        }
    }

    /// Right-click on a photo or recording: its actions.
    fn attachment_menu(self: &Rc<Self>, x: f64, y: f64) -> bool {
        let (bx, by) = self.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let Some(p) = self.view.picture_at(bx as f32, by as f32) else { return false };
        let photo = p.texture.is_some();
        let features = self.store.features.borrow().clone();
        let menu = gio::Menu::new();
        let group = gio::SimpleActionGroup::new();
        let add = |label: &str, name: &str, f: Box<dyn Fn(&Rc<Editor>)>| {
            let a = gio::SimpleAction::new(name, None);
            let weak = self.me.clone();
            a.connect_activate(move |_, _| {
                if let Some(e) = weak.upgrade() {
                    f(&e);
                }
            });
            group.add_action(&a);
            menu.append(Some(label), Some(&format!("media.{name}")));
        };
        let id = p.id.clone();
        if !photo {
            let i = id.clone();
            add(if self.is_playing(&id) { "Pause" } else { "Play" }, "play", Box::new(move |e| e.play(i.clone())));
        }
        if self.can_edit() {
            if photo && features.photo_text {
                let i = id.clone();
                add("Get Text", "text", Box::new(move |e| {
                    let (e, i) = (e.clone(), i.clone());
                    glib::spawn_future_local(async move { e.photo_text(&i).await });
                }));
            }
            if !photo && features.transcription {
                let i = id.clone();
                add("Transcribe", "transcribe", Box::new(move |e| {
                    let (e, i) = (e.clone(), i.clone());
                    glib::spawn_future_local(async move { e.transcribe(&i).await });
                }));
            }
            let i = id.clone();
            let what = if photo { "Photo" } else { "Recording" };
            add(&format!("Remove {what}"), "remove", Box::new(move |e| e.remove_attachment(&i, what)));
        }
        if menu.n_items() == 0 {
            return false;
        }
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.insert_action_group("media", Some(&group));
        popover.set_parent(&self.view);
        popover.set_has_arrow(false);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
        true
    }

    /// Plays a recording, or pauses it if it's playing.
    fn play(&self, id: String) {
        if let Some((playing, media)) = &*self.player.borrow()
            && *playing == id
        {
            media.set_playing(!media.is_playing());
            return;
        }
        let Some(Media::Recording(path)) = self.media.borrow().get(&id).cloned() else { return };
        if let Some((_, old)) = self.player.take() {
            old.pause();
        }
        let media = gtk::MediaFile::for_filename(&path);
        let weak = self.me.clone();
        media.connect_playing_notify(move |_| {
            if let Some(e) = weak.upgrade() {
                e.queue_restyle();
            }
        });
        let weak = self.me.clone();
        media.connect_error_notify(move |m| {
            if let (Some(e), Some(err)) = (weak.upgrade(), m.error()) {
                e.store.toast(&format!("Couldn't play the recording: {err}"));
            }
        });
        media.play();
        self.player.replace(Some((id, media)));
    }

    fn is_playing(&self, id: &str) -> bool {
        self.player.borrow().as_ref().is_some_and(|(p, m)| p == id && m.is_playing())
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
        for tag in self.photo_tags.borrow().values() {
            self.buffer.remove_tag(tag, &start, &end);
        }
        // Live preview: marks hide except on the lines being edited.
        let active = self.active_lines();
        self.active.set(active);
        let styled = styling::apply(&self.view, &text, active);
        let at = |o: usize| self.buffer.iter_at_offset(o as i32);
        let mut pictures = Vec::new();
        let column = self.view.column_width() as f32;
        for img in &styled.images {
            let Some(id) = img.url.strip_prefix("att:") else { continue };
            let media = self.media.borrow().get(id).cloned();
            let (texture, label, w, h) = match media {
                None => {
                    self.load_media(id);
                    continue;
                }
                Some(Media::Loading | Media::Missing) => continue,
                Some(Media::Picture(t)) => {
                    let w = (t.width() as f32).min(column).min(MAX_PHOTO);
                    let h = (w * t.height() as f32 / t.width().max(1) as f32).round();
                    (Some(t), String::new(), w, h)
                }
                Some(Media::Recording(_)) => {
                    let label = if self.is_playing(id) { "⏸  Pause" } else { "▶  Play Recording" };
                    (None, label.to_owned(), noteview::BUTTON.0, noteview::BUTTON.1)
                }
            };
            // Room under the link's line for it.
            let tag = self.photo_tag(h as i32 + 16);
            let mut line_start = at(img.start);
            line_start.set_line_offset(0);
            let mut line_end = at(img.end);
            if !line_end.ends_line() {
                line_end.forward_to_line_end();
            }
            self.buffer.apply_tag(&tag, &line_start, &line_end);
            pictures.push(Picture { at: img.end, id: id.to_owned(), texture, label, width: w, height: h });
        }
        self.view.set_pictures(pictures);
        self.show_blocks(&text, &styled, active);
        self.styled.replace(styled);
        self.show_blame(&text);
    }

    /// Tables and boards as widgets over their Markdown, which shrinks away, except while the
    /// cursor is in them: then the Markdown shows, to edit as text.
    fn show_blocks(&self, text: &str, styled: &markdown::Styled, active: Option<(i32, i32)>) {
        // Measuring a block can resize the view and restyle it from inside here; that pass would
        // take the blocks list half-built and leave its own widgets behind. Do it again after instead.
        if self.placing_blocks.replace(true) {
            self.queue_restyle();
            return;
        }
        self.place_blocks(text, styled, active);
        self.placing_blocks.set(false);
    }

    fn place_blocks(&self, text: &str, styled: &markdown::Styled, active: Option<(i32, i32)>) {
        let chars: Vec<char> = text.chars().collect();
        let mut wanted: Vec<(BlockKind, usize, usize)> = styled.tables.iter().map(|&(s, e)| (BlockKind::Table, s, e)).collect();
        wanted.extend(styled.code_blocks.iter().filter(|b| styling::is_board(b)).map(|b| (BlockKind::Board, b.fence, b.end)));
        wanted.sort_by_key(|w| w.1);
        let at = |o: usize| self.buffer.iter_at_offset(o as i32);
        // A block shows as its Markdown only after Edit as Text, and only while the cursor is in it.
        let as_text = self.text_block.borrow().as_ref().map(|m| self.buffer.iter_at_mark(m).offset() as usize);
        let mut still_text = false;
        wanted.retain(|&(_, s, e)| {
            let (a, b) = (at(s).line(), at(e).line());
            let cursor_in = active.is_some_and(|(x, y)| y >= a && x <= b);
            let editing = cursor_in && as_text.is_some_and(|t| (s..=e).contains(&t));
            still_text |= editing;
            !editing
        });
        if !still_text && let Some(m) = self.text_block.take() {
            self.buffer.delete_mark(&m);
        }
        let focus = self.view.root().and_then(|r| r.focus());
        // By kind and order, as they were; taken out as they're matched.
        let mut old: Vec<Option<BlockView>> = self.blocks.take().into_iter().map(Some).collect();
        let mut kept = Vec::new();
        let editable = self.can_edit();
        // The column the window has room for, not the view's own width: blocks as wide as the view
        // would keep it from ever getting narrower.
        let width = self.view.fit_width();
        for (i, &(kind, s, e)) in wanted.iter().enumerate() {
            let block_text: String = chars[s..e].iter().collect();
            // The same block as before, by kind and order: kept as it is when unchanged, or while
            // someone types in it; drawn again otherwise.
            let nth = wanted[..i].iter().filter(|w| w.0 == kind).count();
            let prev = old.iter().enumerate().filter(|(_, b)| b.as_ref().is_some_and(|b| b.kind == kind)).nth(nth).map(|(p, _)| p);
            let typing_in = |w: &gtk::Widget| focus.as_ref().is_some_and(|f| f.is_ancestor(w) || f == w);
            let fresh = |slot: gtk::Box| {
                let id = self.next_block.get() + 1;
                self.next_block.set(id);
                while let Some(c) = slot.first_child() {
                    slot.remove(&c);
                }
                slot.append(&self.block_widget(kind, id, &block_text, editable));
                (id, slot)
            };
            let (id, slot) = match prev.and_then(|p| old[p].take()) {
                Some(b) if b.text == block_text || (typing_in(b.slot.upcast_ref()) && self.redraw_block.get() != Some(b.id)) => (b.id, b.slot),
                Some(b) => fresh(b.slot),
                None => fresh(self.free_slot()),
            };
            slot.set_size_request(width, -1);
            // The Markdown shrinks away; its first line makes room for the widget.
            self.buffer.apply_tag_by_name("hidden", &at(s), &at(e));
            // What it measures at, or what it was given if that's more, once it's been laid out.
            let (_, measured, _, _) = slot.measure(gtk::Orientation::Vertical, width);
            let h = measured.max(slot.height());
            let mut line_end = at(s);
            if !line_end.ends_line() {
                line_end.forward_to_line_end();
            }
            self.buffer.apply_tag(&self.photo_tag(h + 12), &at(s), &line_end);
            let loc = self.view.iter_location(&at(s));
            self.view.move_overlay(&slot, loc.x(), loc.y() + 4);
            kept.push(BlockView { id, kind, start: s, end: e, text: block_text, slot });
        }
        for b in old.into_iter().flatten() {
            self.release_slot(b.slot);
        }
        // Once laid out, a block may turn out taller than it measured: make room again.
        let pending: Vec<(gtk::Box, i32)> = kept.iter().map(|b| (b.slot.clone(), b.slot.height())).collect();
        self.blocks.replace(kept);
        if !pending.is_empty() && !self.block_check.replace(true) {
            let weak = self.me.clone();
            glib::timeout_add_local_once(Duration::from_millis(100), move || {
                let Some(e) = weak.upgrade() else { return };
                e.block_check.set(false);
                if pending.iter().any(|(w, h)| (w.height() - h).abs() > 2) {
                    e.queue_restyle();
                }
            });
        }
    }

    /// An empty slot over the text for a table or board. GTK has no way to take a widget back out
    /// of a text view's overlays (gtk_text_view_remove only knows anchored children), so slots are
    /// added once and kept: emptied and hidden when their block goes, and used again for the next.
    fn free_slot(&self) -> gtk::Box {
        if let Some(slot) = self.spare_slots.borrow_mut().pop() {
            slot.set_visible(true);
            return slot;
        }
        let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // Clicks on a table or board are its own: once its buttons and fields have had them, they
        // stop here, or the text view under it would take focus and put its cursor in the Markdown.
        let clicks = gtk::EventControllerLegacy::new();
        clicks.connect_event(|_, event| match event.event_type() {
            gdk::EventType::ButtonPress | gdk::EventType::ButtonRelease => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        });
        slot.add_controller(clicks);
        self.view.add_overlay(&slot, 0, 0);
        slot
    }

    fn release_slot(&self, slot: gtk::Box) {
        while let Some(c) = slot.first_child() {
            slot.remove(&c);
        }
        slot.set_visible(false);
        self.spare_slots.borrow_mut().push(slot);
    }

    /// The widget for block `id`, wired to change its Markdown.
    fn block_widget(&self, kind: BlockKind, id: u64, text: &str, editable: bool) -> gtk::Widget {
        let weak = self.me.clone();
        let on: super::tables::OnChange = Rc::new(move |change| {
            if let Some(e) = weak.upgrade() {
                e.block_change(id, change);
            }
        });
        match kind {
            BlockKind::Table => match super::tables::parse_table(text) {
                Some(t) => super::tables::table_widget(&t, editable, on),
                None => gtk::Label::new(None).upcast(),
            },
            BlockKind::Board => super::tables::board_widget(&super::tables::parse_board(text), editable, on),
        }
    }

    /// A change from a table's or board's widget, made to its Markdown as the smallest edit.
    fn block_change(&self, id: u64, change: super::tables::Change) {
        use super::tables::Change;
        let Some((start, end)) = self.blocks.borrow().iter().find(|b| b.id == id).map(|b| (b.start, b.end)) else { return };
        let all: Vec<char> = self.shown_text().chars().collect();
        let Some(old) = all.get(start..end.min(all.len())).map(|c| c.iter().collect::<String>()) else { return };
        let reshape = matches!(change, Change::Reshape(_));
        match change {
            Change::Cell { row, col, text } => {
                let Some(t) = super::tables::parse_table(&old) else { return };
                let cell = if row == 0 { t.header.get(col) } else { t.rows.get(row - 1).and_then(|r| r.get(col)) };
                let new = match cell {
                    Some(c) => {
                        // An empty cell has no place of its own between its spaces: fill the space between the pipes.
                        let empty = c.from == c.to;
                        let (s, e) = if empty { (c.seg.0, c.seg.1) } else { (c.from, c.to) };
                        let (s, e) = ((start + s) as i32, (start + e) as i32);
                        let esc = if empty { format!(" {} ", super::tables::escape_cell(&text)) } else { super::tables::escape_cell(&text) };
                        self.edit(|b| {
                            b.delete(&mut b.iter_at_offset(s), &mut b.iter_at_offset(e));
                            b.insert(&mut b.iter_at_offset(s), &esc);
                        });
                        None
                    }
                    // A cell its row was missing: the table is written out again.
                    None => {
                        let header: Vec<String> = t.header.iter().map(|c| c.text.clone()).collect();
                        let mut rows: Vec<Vec<String>> = t.rows.iter().map(|r| r.iter().map(|c| c.text.clone()).collect()).collect();
                        if let Some(r) = rows.get_mut(row.wrapping_sub(1)) {
                            r.resize(col + 1, String::new());
                            r[col] = text;
                        }
                        Some(super::tables::table_markdown(&header, &t.delimiter, &rows))
                    }
                };
                if let Some(new) = new {
                    self.rewrite_block(start, &old, &new);
                }
            }
            Change::Rewrite(change) | Change::Reshape(change) => {
                if let Some(new) = change(&old).filter(|new| *new != old) {
                    if reshape {
                        self.redraw_block.set(Some(id));
                    }
                    self.rewrite_block(start, &old, &new);
                }
            }
            Change::EditText => {
                if let Some(m) = self.text_block.take() {
                    self.buffer.delete_mark(&m);
                }
                let it = self.buffer.iter_at_offset(start as i32);
                self.text_block.replace(Some(self.buffer.create_mark(None, &it, true)));
                self.view.grab_focus();
                self.buffer.place_cursor(&it);
                self.queue_restyle();
            }
        }
    }

    fn rewrite_block(&self, start: usize, old: &str, new: &str) {
        let changes = super::ai::changes_between(old, new);
        self.edit(|b| {
            for c in changes.iter().rev() {
                let (s, e) = ((start + c.from) as i32, (start + c.to) as i32);
                if e > s {
                    b.delete(&mut b.iter_at_offset(s), &mut b.iter_at_offset(e));
                }
                if !c.insert.is_empty() {
                    b.insert(&mut b.iter_at_offset(s), &c.insert);
                }
            }
        });
    }

    /// The block line `n` belongs to, as (first, last) lines; like blockAt in web/src/lib/dragHandles.ts.
    /// The title line has none: it stays first.
    pub fn block_at(&self, n: i32) -> Option<(i32, i32)> {
        if n <= 0 {
            return None;
        }
        let text = self.shown_text();
        let lines: Vec<&str> = text.split('\n').collect();
        let line = *lines.get(n as usize)?;
        let start = self.buffer.iter_at_line(n)?.offset() as usize;
        // Code blocks, tables and boards move whole.
        let styled = self.styled.borrow();
        let fenced = styled
            .code_blocks
            .iter()
            .map(|b| (b.fence, b.end))
            .chain(styled.tables.iter().copied())
            .find(|&(s, e)| s <= start && start <= e);
        if let Some((s, e)) = fenced {
            let (first, last) = (self.buffer.iter_at_offset(s as i32).line(), self.buffer.iter_at_offset(e as i32).line());
            return (first > 0).then_some((first, last));
        }
        if line.trim().is_empty() {
            return None;
        }
        let prefix = markdown::line_prefix(line).trim_start();
        let listed = prefix.starts_with(['-', '*', '+']) || prefix.starts_with(|c: char| c.is_ascii_digit());
        if !listed {
            return Some((n, n));
        }
        // A list item carries the items nested under it.
        let indent = |l: &str| l.len() - l.trim_start().len();
        let mut last = n as usize;
        while let Some(next) = lines.get(last + 1) {
            if next.trim().is_empty() || indent(next) <= indent(line) {
                break;
            }
            last += 1;
        }
        Some((n, last as i32))
    }

    fn line_start(&self, n: i32) -> Option<usize> {
        if n >= self.buffer.line_count() {
            return Some(self.buffer.end_iter().offset() as usize);
        }
        self.buffer.iter_at_line(n).map(|i| i.offset() as usize)
    }

    /// Where a dragged block would go for a point: before the block or blank line under it, or the end.
    /// Drops the block being dragged by its grip at (x, y), in widget coordinates.
    fn end_drag(&self, x: f64, y: f64) {
        let Some((first, last)) = self.dragging.get() else { return };
        self.view.set_drop(None);
        // Where it goes, while drop_line still knows what's being dragged.
        let before = self.drop_line(x, y);
        self.dragging.set(None);
        if let Some(before) = before {
            self.move_block(first, last, before);
        }
    }

    /// Drags line `n`'s block by its grip and drops it on the top half of line `onto`, the way a
    /// pointer would.
    pub fn grip_drag(&self, n: i32, onto: i32) {
        let Some(block) = self.block_at(n) else { return };
        let Some(it) = self.buffer.iter_at_line(onto) else { return };
        let (y, _) = self.view.line_yrange(&it);
        let (_, wy) = self.view.buffer_to_window_coords(gtk::TextWindowType::Widget, 0, y + 1);
        self.dragging.set(Some(block));
        self.end_drag(self.view.left_margin() as f64 - 20.0, wy as f64);
    }

    fn drop_line(&self, x: f64, y: f64) -> Option<i32> {
        let (first, last) = self.dragging.get()?;
        let (_, by) = self.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let (it, top) = self.view.line_at_y(by);
        let (_, h) = self.view.line_yrange(&it);
        let mut n = it.line();
        // The lower half of a line drops after it.
        if by > top + h / 2 {
            n = self.block_at(n).map_or(n, |(_, l)| l) + 1;
        } else if let Some((f, _)) = self.block_at(n) {
            n = f;
        }
        let n = n.max(1);
        (n < first || n > last + 1).then_some(n)
    }

    /// Moves lines `first..=last` to before line `before` (the line count for the end), as one change.
    pub fn move_block(&self, first: i32, last: i32, before: i32) {
        if before >= first && before <= last + 1 {
            return;
        }
        let old = self.shown_text();
        let mut lines: Vec<&str> = old.split('\n').collect();
        let (f, l, b) = (first as usize, last as usize, (before as usize).min(lines.len()));
        if l >= lines.len() {
            return;
        }
        let moving: Vec<&str> = lines.drain(f..=l).collect();
        let at = if b > l { b - moving.len() } else { b };
        for (i, line) in moving.into_iter().enumerate() {
            lines.insert(at + i, line);
        }
        let new = lines.join("\n");
        self.rewrite_block(0, &old, &new);
    }

    /// Others can edit this note, so lines they wrote are marked.
    fn shared(&self) -> bool {
        self.store.note(&self.id).is_some_and(|n| n.shared || n.role != Role::Owner)
    }

    /// Bars beside the lines someone else typed in last, like web/src/lib/blame.ts.
    fn show_blame(&self, text: &str) {
        if !self.shared() {
            self.view.set_bars(Vec::new());
            self.blame.borrow_mut().clear();
            return;
        }
        let mut ranges = Vec::new();
        let mut at = 0;
        for line in text.split('\n') {
            let n = line.chars().count();
            ranges.push((at, at + n));
            at += n + 1;
        }
        let doc = self.doc.borrow();
        let writers = doc.writers(&ranges);
        let mine = doc.peer_id();
        drop(doc);
        let me = self.store.user();
        let mut bars = Vec::new();
        let mut labels = Vec::new();
        let mut unknown = false;
        for (line, (range, writer)) in ranges.iter().zip(writers).enumerate() {
            let Some((peer, ts)) = writer else { continue };
            let who = if peer == mine {
                Some((Some(me.id.clone()), me.display_name.clone()))
            } else {
                self.authors.borrow().get(&peer.to_string()).map(|a| (a.user_id.clone(), a.name.clone().unwrap_or_else(|| "Someone".into())))
            };
            let Some((user, name)) = who else {
                unknown = true;
                continue;
            };
            if user.as_deref() == Some(me.id.as_str()) {
                continue;
            }
            let color = user.as_deref().map_or("user-unknown", color_for);
            bars.push((range.0, color_rgb(color)));
            let when = glib::DateTime::from_unix_local(ts).ok().filter(|_| ts > 0).and_then(|t| t.format("%b %-d, %-l:%M %p").ok());
            labels.push((line as i32, when.map_or(name.clone(), |w| format!("{name} · {w}"))));
        }
        self.view.set_bars(bars);
        self.blame.replace(labels);
        if unknown {
            self.fetch_authors();
        }
    }

    /// Asks the server whose sessions wrote this note; again later if a new one turns up.
    fn fetch_authors(&self) {
        let now = super::store::now_ms();
        let wait = 5000 - (now - self.authors_asked.get());
        if wait > 0 {
            // Asked a moment ago: look again once the wait is up, if still needed.
            if !self.authors_retry.replace(true) {
                let weak = self.me.clone();
                glib::timeout_add_local_once(Duration::from_millis(wait as u64), move || {
                    if let Some(e) = weak.upgrade() {
                        e.authors_retry.set(false);
                        e.queue_restyle();
                    }
                });
            }
            return;
        }
        self.authors_asked.set(now);
        let (api, id, weak) = (self.store.api.clone(), self.id.clone(), self.me.clone());
        glib::spawn_future_local(async move {
            let Ok(authors) = bg(async move { api.authors(&id).await }).await else { return };
            let Some(e) = weak.upgrade() else { return };
            e.authors.replace(authors);
            e.queue_restyle();
        });
    }

    /// Who wrote line `n`, if someone else did.
    pub fn line_writer(&self, n: i32) -> Option<String> {
        self.blame.borrow().iter().find(|(l, _)| *l == n).map(|(_, w)| w.clone())
    }

    fn photo_tag(&self, space: i32) -> gtk::TextTag {
        self.photo_tags
            .borrow_mut()
            .entry(space)
            .or_insert_with(|| self.buffer.create_tag(None, &[("pixels-below-lines", &space)]).expect("tag"))
            .clone()
    }

    /// Fetches an attachment: a photo to show, or a recording to play (attachments are one or the other).
    fn load_media(&self, id: &str) {
        self.media.borrow_mut().insert(id.to_owned(), Media::Loading);
        let api = self.store.api.clone();
        let weak = self.me.clone();
        let id = id.to_owned();
        glib::spawn_future_local(async move {
            let att = id.clone();
            let bytes = bg(async move { api.attachment(&att).await }).await;
            let Some(e) = weak.upgrade() else { return };
            let media = match bytes {
                Err(_) => Media::Missing,
                Ok(b) => match gdk::Texture::from_bytes(&glib::Bytes::from(&b)) {
                    Ok(t) => Media::Picture(t),
                    Err(_) => {
                        let path = crate::cache::data_dir().join("recordings").join(&id);
                        let saved = std::fs::create_dir_all(path.parent().expect("folder")).and_then(|_| std::fs::write(&path, &b));
                        if saved.is_ok() { Media::Recording(path) } else { Media::Missing }
                    }
                },
            };
            e.media.borrow_mut().insert(id, media);
            e.queue_restyle();
        });
    }

    /// Colors for the Markdown tags, for the light or dark style.
    fn color_tags(&self) {
        styling::color_tags(&self.view);
    }

    /// Applies `changes` (from ai::changes_between) as one undo step, if the text is still
    /// `original`. False when it changed meanwhile.
    pub fn replace_text(&self, original: &str, changes: &[super::ai::Change]) -> bool {
        if !self.can_edit() || self.shown_text() != original {
            return false;
        }
        // Back to front, so earlier positions stay put.
        self.edit(|b| {
            for c in changes.iter().rev() {
                let (from, to) = (c.from as i32, c.to as i32);
                if to > from {
                    b.delete(&mut b.iter_at_offset(from), &mut b.iter_at_offset(to));
                }
                if !c.insert.is_empty() {
                    b.insert(&mut b.iter_at_offset(from), &c.insert);
                }
            }
        });
        true
    }

    /// Table or board `n`'s widget, as drawn now.
    /// Tables and boards attached to the view, and how many the editor keeps track of. For tests:
    /// the two differ when one has been left behind.
    pub fn block_counts(&self) -> (usize, usize) {
        fn count(w: &gtk::Widget) -> usize {
            let mut n = usize::from(w.has_css_class("note-block"));
            let mut c = w.first_child();
            while let Some(child) = c {
                n += count(&child);
                c = child.next_sibling();
            }
            n
        }
        (count(self.view.upcast_ref()), self.blocks.borrow().len())
    }

    pub fn block_widget_at(&self, n: usize) -> Option<gtk::Widget> {
        self.blocks.borrow().get(n).map(|b| b.slot.clone().upcast())
    }

    /// Shows the grip beside line `n` and a drop line before line `drop`, as during a drag. For screenshots.
    pub fn show_grip(&self, n: i32, drop: i32) {
        self.view.set_grip(self.line_start(n));
        self.view.set_drop(self.line_start(drop));
    }

    /// The "/" menu or emoji list is showing.
    pub fn menu_open(&self) -> bool {
        self.completer.is_open()
    }

    /// Picks an item from the "/" menu or emoji list by its label.
    pub fn pick_from_menu(&self, label: &str) -> bool {
        self.completer.pick_named(label)
    }

    /// Where the cursor is, in characters.
    pub fn cursor_offset(&self) -> usize {
        self.buffer.iter_at_mark(&self.buffer.get_insert()).offset() as usize
    }

    /// The text view and the cursor's rectangle in it, for a popover to point at.
    pub fn cursor_rect(&self) -> Option<(gtk::Widget, gdk::Rectangle)> {
        if !self.can_edit() {
            return None;
        }
        let it = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let loc = self.view.iter_location(&it);
        let (x, y) = self.view.buffer_to_window_coords(gtk::TextWindowType::Widget, loc.x(), loc.y());
        Some((self.view.clone().upcast(), gdk::Rectangle::new(x, y, 1, loc.height())))
    }

    /// Writes `text` at line of `at`: in place of a blank line, or one with just a list marker or
    /// checkbox; otherwise on new lines under it. As one undo step.
    pub fn write_at_line(&self, at: usize, text: &str) {
        let it = self.buffer.iter_at_offset(at as i32);
        let mut start = it;
        start.set_line_offset(0);
        let mut end = it;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let line = self.buffer.text(&start, &end, true).to_string();
        let empty = line.trim().is_empty() || line.len() == markdown::line_prefix(&line).len();
        let (s, e) = (start.offset(), end.offset());
        self.edit(|b| {
            if empty {
                b.delete(&mut b.iter_at_offset(s), &mut b.iter_at_offset(e));
                b.insert(&mut b.iter_at_offset(s), text);
            } else {
                b.insert(&mut b.iter_at_offset(e), &format!("\n{text}"));
            }
        });
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

    /// Puts the cursor at the end of line `n` (from 0), with the text focused.
    pub fn cursor_to_line(&self, n: i32) {
        self.view.grab_focus();
        if let Some(mut it) = self.buffer.iter_at_line(n) {
            if !it.ends_line() {
                it.forward_to_line_end();
            }
            self.buffer.place_cursor(&it);
        }
    }

    /// Whether `text` shows whole on screen: none of it hidden by live preview.
    pub fn shows(&self, text: &str) -> bool {
        let all = self.shown_text();
        let Some(byte) = all.find(text) else { return false };
        let start = all[..byte].chars().count() as i32;
        let hidden = self.buffer.tag_table().lookup("hidden").expect("tag");
        (start..start + text.chars().count() as i32).all(|o| !self.buffer.iter_at_offset(o).has_tag(&hidden))
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

    /// Follows the nth link in the note, as a click on it would.
    pub fn follow_link(&self, n: usize) -> bool {
        self.restyle();
        let Some(url) = self.styled.borrow().links.get(n).map(|l| l.url.clone()) else { return false };
        self.follow(&url)
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

    /// What shows under the text: "photo" or the recording button's label, per attachment.
    pub fn media_shown(&self) -> Vec<String> {
        self.view.pictures().iter().map(|p| if p.texture.is_some() { "photo".to_owned() } else { p.label.trim().to_owned() }).collect()
    }

    /// The photos and recordings showing, by attachment id.
    pub fn media_ids(&self) -> Vec<String> {
        self.view.pictures().iter().map(|p| p.id.clone()).collect()
    }

    /// Adds a file as Add Photo does.
    pub fn add_file(self: &Rc<Self>, path: &str) {
        let (e, file) = (self.clone(), gio::File::for_path(path));
        glib::spawn_future_local(async move { e.upload_file(&file).await });
    }

    pub fn peer_names(&self) -> Vec<String> {
        self.doc.borrow().peers().into_iter().map(|p| p.name).collect()
    }

    pub fn banner(&self) -> Option<String> {
        self.banner.is_revealed().then(|| self.banner.title().to_string())
    }

    /// Leaves the note: drops presence, saves, and discards it if it was left blank.
    pub fn close(&self) {
        if self.saving.get() {
            self.close_after_save.set(true);
            return;
        }
        if self.closed.replace(true) {
            return;
        }
        for t in self.timers.take() {
            t.remove();
        }
        if let Some(t) = self.save_timer.take() {
            t.remove();
        }
        self.completer.hide();
        if let Some((_, media)) = self.player.take() {
            media.pause();
        }
        if let Some(r) = self.recording.take() {
            r.timer.remove();
            r.rec.cancel();
            if let Some(t) = r.live {
                t.abort();
            }
        }
        if let Some(spot) = self.spot.take() {
            self.spot_done(&spot);
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

/// Where a voice memo goes, picked when recording starts: on a line of its own under the cursor, or at
/// the end. Marks follow everyone's edits. A live transcript is written there as it's heard: deltas are
/// added, and each final replaces the stretch of deltas it covers. The player goes above it.
struct Spot {
    anchor: gtk::TextMark,
    /// The transcript's start and end, and where the stretch still being heard starts, once it has a line.
    from: Option<gtk::TextMark>,
    to: Option<gtk::TextMark>,
    stretch: Option<gtk::TextMark>,
}

struct Recording {
    rec: crate::recorder::Recording,
    started: std::time::Instant,
    timer: glib::SourceId,
    /// The live transcript's link, which ends once its last words are in.
    live: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Clone, Copy, PartialEq)]
enum BlockKind {
    Table,
    Board,
}

/// A table or board shown as a widget: where its Markdown is, and what it was drawn from.
struct BlockView {
    /// Its widget's edits find it by this, wherever it has moved to.
    id: u64,
    kind: BlockKind,
    start: usize,
    end: usize,
    text: String,
    /// The slot in the view its widget is in (see `free_slot`).
    slot: gtk::Box,
}

#[derive(Clone)]
enum Media {
    Loading,
    Picture(gdk::Texture),
    /// Saved where the player can open it.
    Recording(std::path::PathBuf),
    /// Gone, or the server couldn't be reached.
    Missing,
}

enum Target {
    Task(usize, bool),
    Link(String),
    Recording(String),
}
