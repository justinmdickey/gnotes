//! The window: sign-in, or the sidebar and the open note side by side (one at a time when narrow).

use std::{
    cell::{OnceCell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use gtk::{gio, glib};

use super::{dialogs, editor::Editor, lists, login, sidebar::Sidebar, store::Store};
use crate::{bg, cache::Session};

/// What the content pane shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    Nothing,
    Note(String),
    Tasks,
    Recent,
    Search,
    Ask,
    Shared,
    Trash,
}

pub struct Window;

impl Window {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(app: &adw::Application) -> adw::ApplicationWindow {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Gnotes")
            .default_width(1100)
            .default_height(720)
            .width_request(360)
            .height_request(400)
            .build();
        if let Some((w, h)) = super::shot::size() {
            window.set_default_size(w, h);
        }
        super::shot::schedule(&window);
        match Session::load() {
            Some(session) if session.local => open_local(&window),
            Some(session) => Main::show(&window, session),
            None => show_login(&window, None),
        }
        window
    }
}

fn show_login(window: &adw::ApplicationWindow, server: Option<String>) {
    let w = window.clone();
    window.set_content(Some(&login::page(server, true, move |session| {
        if let Err(e) = session.save() {
            eprintln!("saving the session: {e}");
        }
        Main::show(&w, session);
    })));
}

/// Starts the built-in server for the notes on this computer, then shows them.
fn open_local(window: &adw::ApplicationWindow) {
    let opening = adw::StatusPage::builder()
        .title("Opening Your Notes…")
        .child(&adw::Spinner::builder().width_request(32).height_request(32).build())
        .build();
    window.set_content(Some(&opening));
    let w = window.clone();
    glib::spawn_future_local(async move {
        match start_local().await {
            Ok(session) => Main::show(&w, session),
            Err(e) => {
                eprintln!("opening the notes on this computer: {e}");
                show_login(&w, None);
            }
        }
    });
}

/// The notes on this computer: starts the built-in server, signs in, and keeps that as the session.
pub async fn start_local() -> Result<Session, String> {
    let name = glib::real_name().to_string_lossy().trim().to_owned();
    let name = if name.is_empty() || name == "Unknown" { glib::user_name().to_string_lossy().into_owned() } else { name };
    let dir = crate::local::data_dir();
    let (api, user) = bg(async move { crate::local::start(&dir, &name).await }).await.map_err(|e| format!("{e:#}"))?;
    let session = Session { server: api.base.clone(), token: api.token.clone(), user, local: true };
    if let Err(e) = session.save() {
        eprintln!("saving the session: {e}");
    }
    Ok(session)
}

/// The signed-in window.
pub struct Main {
    pub store: Rc<Store>,
    pub window: adw::ApplicationWindow,
    pub split: adw::NavigationSplitView,
    pub nav: adw::NavigationView,
    toasts: adw::ToastOverlay,
    pub place: RefCell<Place>,
    /// The notebook New Note goes into; None is the top level.
    pub folder: RefCell<Option<String>>,
    pub sidebar: OnceCell<Rc<Sidebar>>,
    /// The Ask conversation, kept while you're elsewhere.
    pub ask: OnceCell<Rc<super::ask::Ask>>,
    /// Text the Search page starts with.
    pub query: RefCell<String>,
    /// Which place Tasks shows: a top notebook's id, "shared", "notes" or "all".
    pub tasks_scope: RefCell<String>,
    editor: RefCell<Option<Rc<Editor>>>,
}

impl Main {
    fn show(window: &adw::ApplicationWindow, session: Session) {
        let store = Store::start(&session);
        let toasts = adw::ToastOverlay::new();
        let nav = adw::NavigationView::new();
        let split = adw::NavigationSplitView::builder()
            .min_sidebar_width(240.0)
            .max_sidebar_width(300.0)
            .sidebar_width_fraction(0.25)
            .build();
        let main = Rc::new(Main {
            store: store.clone(),
            window: window.clone(),
            split: split.clone(),
            nav: nav.clone(),
            toasts: toasts.clone(),
            place: RefCell::new(Place::Nothing),
            folder: RefCell::new(None),
            sidebar: OnceCell::new(),
            query: Default::default(),
            tasks_scope: RefCell::new("all".into()),
            ask: OnceCell::new(),
            editor: RefCell::new(None),
        });

        {
            let toasts = toasts.clone();
            *store.toast.borrow_mut() = Some(Box::new(move |msg| {
                if std::env::var_os("GNOTES_DRIVE").is_some() {
                    eprintln!("toast: {msg}");
                }
                toasts.add_toast(adw::Toast::new(msg))
            }));
        }
        {
            let w = window.clone();
            let weak = Rc::downgrade(&main);
            *store.on_signed_out.borrow_mut() = Some(Box::new(move || {
                if let Some(main) = weak.upgrade() {
                    main.sign_out(false);
                    WidgetExt::activate_action(&w, "win.signed-out-toast", None).ok();
                }
            }));
        }

        let sidebar = Sidebar::new(&main);
        let _ = main.sidebar.set(sidebar.clone());
        split.set_sidebar(Some(&adw::NavigationPage::builder().title("Gnotes").tag("sidebar").child(&sidebar.widget).build()));
        split.set_content(Some(&adw::NavigationPage::builder().title("Note").tag("content").child(&nav).build()));
        main.show_place(Place::Nothing);

        // Back from a note opened from a list returns to the list, and closes the note.
        {
            let weak = Rc::downgrade(&main);
            nav.connect_popped(move |_, page| {
                let Some(main) = weak.upgrade() else { return };
                let closing = main.editor.borrow().as_ref().is_some_and(|e| e.page == *page);
                if closing && let Some(e) = main.editor.take() {
                    e.close();
                    if let Some(prev) = main.nav.visible_page().and_then(|p| p.tag()) {
                        main.place.replace(match prev.as_str() {
                            "tasks" => Place::Tasks,
                            "recent" => Place::Recent,
                            "ask" => Place::Ask,
                            "search" => Place::Search,
                            "shared" => Place::Shared,
                            "trash" => Place::Trash,
                            _ => Place::Nothing,
                        });
                    }
                    main.sidebar().select();
                }
            });
        }

        let bp = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 640sp").expect("breakpoint"));
        bp.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(bp);

        main.add_actions();
        // Markdown files or zips dropped anywhere but a note are imported into the current notebook.
        let target = gtk::DropTarget::new(gtk::gdk::FileList::static_type(), gtk::gdk::DragAction::COPY);
        let weak = Rc::downgrade(&main);
        target.connect_drop(move |_, value, _, _| {
            let (Some(main), Ok(files)) = (weak.upgrade(), value.get::<gtk::gdk::FileList>()) else { return false };
            glib::spawn_future_local(async move { super::settings::import(&main, files.files()).await });
            true
        });
        split.add_controller(target);
        toasts.set_child(Some(&split));
        window.set_content(Some(&toasts));

        // Close the open note cleanly (presence, a blank note's discard) when the window goes.
        {
            let weak = Rc::downgrade(&main);
            window.connect_close_request(move |w| {
                if let Some(main) = weak.upgrade()
                    && let Some(e) = main.editor.take()
                {
                    e.close();
                }
                // A moment for the last edits, presence and a blank note's discard to reach the server.
                if let Some(app) = w.application() {
                    let hold = app.hold();
                    glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || drop(hold));
                }
                glib::Propagation::Proceed
            });
        }
        store.refresh_tree();
        if let Ok(target) = std::env::var("GNOTES_OPEN") {
            main.open_when_loaded(target);
        }
        super::drive::start(&main);
        // Keep the Main alive as long as the window shows it.
        unsafe { window.set_data("gnotes-main", main) };
    }

    /// Opens a screen by name once the tree has loaded, for screenshots (see `drive::open_screen`).
    fn open_when_loaded(self: &Rc<Self>, target: String) {
        let done = Rc::new(std::cell::Cell::new(false));
        let weak = Rc::downgrade(self);
        self.store.on_tree(move || {
            let Some(main) = weak.upgrade() else { return false };
            if done.replace(true) {
                return false;
            }
            // Dialogs need a window on screen to show over.
            if main.window.is_mapped() {
                super::drive::open_screen(&main, &target);
            } else {
                let (m, t) = (main.clone(), target.clone());
                main.window.connect_map(move |_| super::drive::open_screen(&m, &t));
            }
            false
        });
    }

    pub fn ask(self: &Rc<Self>) -> &Rc<super::ask::Ask> {
        self.ask.get_or_init(|| super::ask::Ask::new(self))
    }

    /// Opens Ask and asks `question`, as a new conversation.
    pub fn ask_question(self: &Rc<Self>, question: &str) {
        self.show_place(Place::Ask);
        self.ask().start(question);
    }

    pub fn sidebar(&self) -> &Rc<Sidebar> {
        self.sidebar.get().expect("sidebar")
    }

    pub fn toast(&self, msg: &str) {
        // Driven runs (e2e) say what the user would have been told.
        if std::env::var_os("GNOTES_DRIVE").is_some() {
            eprintln!("toast: {msg}");
        }
        self.toasts.add_toast(adw::Toast::new(msg));
    }

    /// A toast with Undo.
    pub fn toast_undo(&self, msg: &str, undo: impl Fn() + 'static) {
        let toast = adw::Toast::builder().title(msg).button_label("Undo").build();
        toast.connect_button_clicked(move |_| undo());
        self.toasts.add_toast(toast);
    }

    fn add_actions(self: &Rc<Self>) {
        let action = |name: &str, f: Box<dyn Fn(&Rc<Main>)>| {
            let a = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                if let Some(main) = weak.upgrade() {
                    f(&main);
                }
            });
            self.window.add_action(&a);
        };
        action("new-note", Box::new(|m| m.new_note()));
        action("new-notebook", Box::new(|m| dialogs::new_notebook(m, m.folder.borrow().clone())));
        action("search", Box::new(|m| m.show_place(Place::Search)));
        action("recent", Box::new(|m| m.show_place(Place::Recent)));
        action("tasks", Box::new(|m| m.show_place(Place::Tasks)));
        action("new-task", Box::new(super::tasks::new_task));
        action("logout", Box::new(|m| m.sign_out(true)));
        action("about", Box::new(|m| m.about()));
        action("shortcuts", Box::new(|m| m.shortcuts()));
        action("import", Box::new(super::settings::pick_import));
        action("settings", Box::new(super::settings::open));
        action("move-to-server", Box::new(|m| super::settings::move_dialog(m, None)));
        action("signed-out-toast", Box::new(|m| m.toast("Signed out. Sign in again to keep syncing.")));
        if let Some(app) = self.window.application() {
            app.set_accels_for_action("win.new-note", &["<Control>n"]);
            app.set_accels_for_action("win.new-notebook", &["<Control><Shift>n"]);
            app.set_accels_for_action("win.new-task", &["<Control>t"]);
            app.set_accels_for_action("win.search", &["<Control>f", "<Control>k"]);
            app.set_accels_for_action("win.settings", &["<Control>comma"]);
            app.set_accels_for_action("win.shortcuts", &["<Control>question"]);
        }
    }

    /// Shows a list, or nothing, in the content pane.
    pub fn show_place(self: &Rc<Self>, place: Place) {
        if let Place::Note(id) = place {
            self.open_note(&id, false);
            return;
        }
        if let Some(e) = self.editor.take() {
            e.close();
        }
        let page = match &place {
            Place::Nothing => lists::empty_page(self),
            Place::Tasks => super::tasks::page(self),
            Place::Recent => lists::recent_page(self),
            Place::Search => lists::search_page(self),
            Place::Ask => self.ask().page.clone(),
            Place::Shared => lists::shared_page(self),
            Place::Trash => lists::trash_page(self),
            Place::Note(_) => unreachable!(),
        };
        self.nav.replace(&[page]);
        let show = place != Place::Nothing;
        self.place.replace(place);
        if show {
            self.split.set_show_content(true);
        }
        if let Some(s) = self.sidebar.get() {
            s.select();
        }
    }

    /// Opens a note in the content pane. `over_list` puts it on top of the list showing, so Back returns there.
    pub fn open_note(self: &Rc<Self>, id: &str, over_list: bool) {
        if self.editor.borrow().as_ref().is_some_and(|e| e.id == id) {
            self.split.set_show_content(true);
            return;
        }
        if let Some(e) = self.editor.take() {
            e.close();
        }
        let editor = Editor::new(self, id);
        let page = editor.page.clone();
        *self.editor.borrow_mut() = Some(editor);
        let listing = matches!(&*self.place.borrow(), Place::Tasks | Place::Recent | Place::Search | Place::Ask | Place::Shared | Place::Trash);
        if over_list && listing {
            self.nav.push(&page);
        } else {
            self.nav.replace(&[page]);
        }
        if let Some(note) = self.store.note(id) {
            self.folder.replace(note.notebook_id.clone().filter(|_| note.role.can_edit()));
        }
        self.place.replace(Place::Note(id.to_owned()));
        self.split.set_show_content(true);
        self.sidebar().reveal(id);
        self.sidebar().select();
    }

    /// Opens a task's note over the Tasks list, at the task's line.
    pub fn open_task(self: &Rc<Self>, id: &str, line: usize) {
        self.open_note(id, true);
        if let Some(e) = self.editor() {
            e.go_to_line(line);
        }
    }

    pub fn editor(&self) -> Option<Rc<Editor>> {
        self.editor.borrow().clone()
    }

    /// The note open right now.
    pub fn open_id(&self) -> Option<String> {
        self.editor.borrow().as_ref().map(|e| e.id.clone())
    }

    /// A note in the current notebook, open with the cursor in it.
    pub fn new_note(self: &Rc<Self>) {
        let main = self.clone();
        let folder = self.folder.borrow().clone();
        glib::spawn_future_local(async move {
            if let Some(id) = main.store.create_note(folder).await {
                main.open_note(&id, false);
            }
        });
    }

    /// Closes this account's window contents and opens `session` instead, e.g. after Move to a Server.
    pub fn switch_to(self: &Rc<Self>, session: Session) {
        if let Some(e) = self.editor.take() {
            e.close();
        }
        if let Some(a) = self.ask.get() {
            a.end();
        }
        self.store.sync.stop();
        if let Err(e) = session.save() {
            eprintln!("saving the session: {e}");
        }
        unsafe {
            let _ = self.window.steal_data::<Rc<Main>>("gnotes-main");
        }
        Main::show(&self.window, session);
    }

    /// Closes everything and goes back to sign-in. `server_too` also ends the session on the server
    /// and deletes this account's offline copies.
    pub fn sign_out(self: &Rc<Self>, server_too: bool) {
        if let Some(e) = self.editor.take() {
            e.close();
        }
        if let Some(a) = self.ask.get() {
            a.end();
        }
        self.store.sync.stop();
        if server_too {
            let api = self.store.api.clone();
            glib::spawn_future_local(async move {
                let _ = bg(async move { api.logout().await }).await;
            });
            self.store.cache.wipe();
        }
        Session::clear();
        let server = self.store.api.base.clone();
        unsafe {
            let _ = self.window.steal_data::<Rc<Main>>("gnotes-main");
        }
        show_login(&self.window, Some(server));
    }

    fn about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("Gnotes")
            .application_icon(super::APP_ID)
            .version(crate::VERSION)
            .license_type(gtk::License::MitX11)
            .comments("Markdown notes on your computer, or shared live through your own server")
            .website("https://github.com/justinmdickey/gnotes")
            .build();
        about.present(Some(&self.window));
    }

    /// Keyboard Shortcuts, from the main menu or Ctrl+?.
    fn shortcuts(&self) {
        let dialog = adw::ShortcutsDialog::new();
        let sections: [(&str, &[(&str, &str)]); 3] = [
            (
                "General",
                &[
                    ("New Note", "<Control>n"),
                    ("New Notebook", "<Control><Shift>n"),
                    ("New Task", "<Control>t"),
                    ("Search", "<Control>f"),
                    ("Preferences", "<Control>comma"),
                    ("Keyboard Shortcuts", "<Control>question"),
                ],
            ),
            (
                "Editing",
                &[
                    ("Undo", "<Control>z"),
                    ("Redo", "<Control><Shift>z"),
                    ("Bold", "<Control>b"),
                    ("Italic", "<Control>i"),
                    ("Strikethrough", "<Control><Shift>x"),
                    ("Insert", "slash"),
                ],
            ),
            (
                "Paragraphs",
                &[
                    ("Title", "<Control><Shift>t"),
                    ("Heading", "<Control><Shift>h"),
                    ("Subheading", "<Control><Shift>j"),
                    ("Body", "<Control><Shift>b"),
                    ("Checklist", "<Control><Shift>l"),
                    ("Bulleted List", "<Control><Shift>8"),
                    ("Numbered List", "<Control><Shift>7"),
                ],
            ),
        ];
        for (title, items) in sections {
            let section = adw::ShortcutsSection::new(Some(title));
            for (name, accel) in items {
                section.add(adw::ShortcutsItem::new(name, accel));
            }
            dialog.add(section);
        }
        dialog.present(Some(&self.window));
    }
}
