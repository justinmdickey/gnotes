//! The signed-in app's state, like `web/src/lib/store.svelte.ts`: the tree, the connection, and the
//! notes open over the websocket. Lives on the GTK thread.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use gtk::glib;
use loro::VersionVector;

use crate::{
    api::{Api, Features, Role, Tree, TreeNote, User},
    bg,
    cache::{Cache, Queued, Session},
    note::NoteDoc,
    sync::{Event, Status, SyncClient},
};

/// An open note, as the websocket sees it. See `NoteHandler` in web/src/lib/sync.ts.
pub trait NoteHandler {
    /// The local version to join with, or None to get a full snapshot.
    fn version(&self) -> Option<VersionVector>;
    fn joined(&self, role: Role, server: VersionVector);
    fn update(&self, data: &[u8]);
    fn presence(&self, data: &[u8]);
    fn role(&self, role: Role);
    /// Access was removed (`revoked`), or the note is gone (`not_found`).
    fn lost(&self, reason: &str);
    fn out_of_sync(&self);
}

type Listener = Box<dyn Fn()>;
/// Called on a change; returns false once it's no longer needed (its page or editor is gone).
type Watcher = Rc<dyn Fn() -> bool>;

/// Calls each watcher, dropping those that are done. Watchers may add others while this runs.
fn notify(watchers: &RefCell<Vec<Watcher>>) {
    let now: Vec<Watcher> = watchers.borrow().clone();
    let done: Vec<*const ()> = now.iter().filter(|f| !f()).map(|f| Rc::as_ptr(f) as *const ()).collect();
    if !done.is_empty() {
        watchers.borrow_mut().retain(|f| !done.contains(&(Rc::as_ptr(f) as *const ())));
    }
}

pub struct Store {
    pub api: Api,
    /// Notes on this computer, through the built-in server.
    pub local: bool,
    user: RefCell<User>,
    /// Which optional services the server has on; refreshed when they may have changed.
    pub features: RefCell<Features>,
    features_watchers: RefCell<Vec<Watcher>>,
    pub cache: Cache,
    pub sync: SyncClient,
    pub tree: RefCell<Tree>,
    pub status: Cell<Status>,
    notes: RefCell<HashMap<String, Rc<dyn NoteHandler>>>,
    tree_watchers: RefCell<Vec<Watcher>>,
    status_watchers: RefCell<Vec<Watcher>>,
    /// Shows a short message at the bottom of the window.
    pub toast: RefCell<Option<Box<dyn Fn(&str)>>>,
    /// The server no longer knows this session.
    pub on_signed_out: RefCell<Option<Listener>>,
    refreshing: Cell<bool>,
    refresh_again: Cell<bool>,
    /// Notes this client just made, known to be empty, so they can be typed in before the server answers.
    fresh: RefCell<std::collections::HashSet<String>>,
}

impl Store {
    pub fn start(session: &Session) -> Rc<Store> {
        let api = Api::resume(session.server.clone(), session.token.clone());
        // The built-in server's address changes each start; its copies stay in one place.
        let cache = Cache::for_account(if session.local { "local" } else { &session.server }, &session.user.id);
        let (sync, events) = {
            let _rt = crate::runtime().enter();
            SyncClient::start(api.clone())
        };
        let mut tree = cache.tree().unwrap_or_default();
        add_queued(&mut tree, &cache.queued());
        let store = Rc::new(Store {
            api,
            local: session.local,
            user: RefCell::new(session.user.clone()),
            features: Default::default(),
            features_watchers: Default::default(),
            cache,
            sync,
            tree: RefCell::new(tree),
            status: Cell::new(Status::Connecting),
            notes: Default::default(),
            tree_watchers: Default::default(),
            status_watchers: Default::default(),
            toast: Default::default(),
            on_signed_out: Default::default(),
            refreshing: Cell::new(false),
            refresh_again: Cell::new(false),
            fresh: Default::default(),
        });
        store.refresh_features();
        let weak = Rc::downgrade(&store);
        glib::spawn_future_local(async move {
            while let Ok(ev) = events.recv().await {
                let Some(store) = weak.upgrade() else { break };
                store.handle(ev);
            }
        });
        store
    }

    pub fn user(&self) -> User {
        self.user.borrow().clone()
    }

    /// After a name change: kept for next time too.
    pub fn set_user(&self, user: User) {
        let session = Session { server: self.api.base.clone(), token: self.api.token.clone(), user: user.clone(), local: self.local };
        let _ = session.save();
        *self.user.borrow_mut() = user;
        self.tree_changed();
    }

    /// Calls `f` when the server's optional services change, until it returns false.
    pub fn on_features(&self, f: impl Fn() -> bool + 'static) {
        self.features_watchers.borrow_mut().push(Rc::new(f));
    }

    pub fn refresh_features(self: &Rc<Self>) {
        let store = self.clone();
        glib::spawn_future_local(async move {
            let api = store.api.clone();
            if let Ok(f) = bg(async move { api.features().await }).await {
                *store.features.borrow_mut() = f;
                notify(&store.features_watchers);
            }
        });
    }

    pub fn toast(&self, msg: &str) {
        if let Some(t) = &*self.toast.borrow() {
            t(msg);
        }
    }

    /// Calls `f` after each change to the tree, until it returns false.
    pub fn on_tree(&self, f: impl Fn() -> bool + 'static) {
        self.tree_watchers.borrow_mut().push(Rc::new(f));
    }

    /// Calls `f` when the connection changes, until it returns false.
    pub fn on_status(&self, f: impl Fn() -> bool + 'static) {
        self.status_watchers.borrow_mut().push(Rc::new(f));
    }

    fn tree_changed(&self) {
        notify(&self.tree_watchers);
    }

    fn handler(&self, note: &str) -> Option<Rc<dyn NoteHandler>> {
        self.notes.borrow().get(note).cloned()
    }

    fn handle(self: &Rc<Self>, ev: Event) {
        match ev {
            Event::Status(s) => {
                self.status.set(s);
                notify(&self.status_watchers);
                if s == Status::Online {
                    self.refresh_features();
                    let store = self.clone();
                    glib::spawn_future_local(async move { store.back_online().await });
                }
            }
            Event::TreeChanged => self.refresh_tree(),
            Event::Joined { note, role, version } => {
                if let Some(h) = self.handler(&note) {
                    h.joined(role, version);
                }
            }
            Event::Update { note, data } => {
                if let Some(h) = self.handler(&note) {
                    h.update(&data);
                }
            }
            Event::Presence { note, data } => {
                if let Some(h) = self.handler(&note) {
                    h.presence(&data);
                }
            }
            Event::Role { note, role } => {
                if let Some(h) = self.handler(&note) {
                    h.role(role);
                }
            }
            Event::Revoked { note } => {
                if let Some(h) = self.notes.borrow_mut().remove(&note) {
                    h.lost("revoked");
                }
            }
            Event::Error { note: Some(note), code } => match code.as_str() {
                "not_found" => {
                    if let Some(h) = self.notes.borrow_mut().remove(&note) {
                        h.lost("not_found");
                    }
                }
                "out_of_sync" => {
                    if let Some(h) = self.handler(&note) {
                        h.out_of_sync();
                    }
                }
                _ => eprintln!("sync error on {note}: {code}"),
            },
            Event::Error { note: None, code } => eprintln!("sync error: {code}"),
            Event::SignedOut => {
                if let Some(f) = &*self.on_signed_out.borrow() {
                    f();
                }
            }
        }
    }

    /// Creates notes made offline, then rejoins open notes and pushes edits made to closed ones.
    async fn back_online(self: &Rc<Self>) {
        for q in self.cache.queued() {
            let api = self.api.clone();
            let (id, nb) = (q.id.clone(), q.notebook_id.clone());
            let mut made = bg(async move { api.create_note(nb.as_deref(), Some(&id)).await }).await;
            if made.as_ref().is_err_and(|e| e.status() == Some(404)) {
                // Its notebook is gone: make it at the top level instead.
                let api = self.api.clone();
                let id = q.id.clone();
                made = bg(async move { api.create_note(None, Some(&id)).await }).await;
            }
            match made {
                Ok(_) => self.cache.unqueue(&q.id),
                Err(e) if e.is_offline() => return,
                Err(e) => eprintln!("creating offline note {}: {e}", q.id),
            }
        }
        let open: Vec<(String, Rc<dyn NoteHandler>)> =
            self.notes.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (id, h) in open {
            self.sync.join(&id, h.version().as_ref());
        }
        let dirty: Vec<String> =
            self.cache.dirty().into_iter().filter(|id| !self.notes.borrow().contains_key(id)).collect();
        for id in dirty {
            self.flush_note(&id);
        }
    }

    /// Whether this client just made the note (once: the answer is used up).
    pub fn take_fresh(&self, id: &str) -> bool {
        self.fresh.borrow_mut().remove(id)
    }

    pub fn is_queued(&self, id: &str) -> bool {
        self.cache.queued().iter().any(|q| q.id == id)
    }

    /// Starts listening to a note, joining it now if online.
    pub fn open(&self, id: &str, handler: Rc<dyn NoteHandler>) {
        let version = handler.version();
        self.notes.borrow_mut().insert(id.to_owned(), handler);
        if !self.is_queued(id) {
            self.sync.join(id, version.as_ref());
        }
    }

    /// With `handler`, only if it's still the one listening.
    pub fn close(&self, id: &str, handler: &Rc<dyn NoteHandler>) {
        let mut notes = self.notes.borrow_mut();
        if notes.get(id).is_some_and(|h| Rc::ptr_eq(h, handler)) {
            notes.remove(id);
            self.sync.leave(id);
        }
    }

    pub fn rejoin(&self, id: &str) {
        if let Some(h) = self.handler(id) {
            self.sync.join(id, h.version().as_ref());
        }
    }

    /// Pushes a closed note's offline edits: join with its saved version and send what the server lacks.
    pub fn flush_note(self: &Rc<Self>, id: &str) {
        let Some(saved) = self.cache.note(id) else {
            self.cache.clear_dirty(id);
            return;
        };
        let mut doc = NoteDoc::new();
        if doc.import(&saved).is_err() {
            return;
        }
        let h: Rc<dyn NoteHandler> =
            Rc::new(Flush { store: Rc::downgrade(self), id: id.to_owned(), doc: RefCell::new(doc), done: Cell::new(false) });
        self.open(id, h);
    }

    pub fn refresh_tree(self: &Rc<Self>) {
        if self.refreshing.replace(true) {
            self.refresh_again.set(true);
            return;
        }
        let store = self.clone();
        glib::spawn_future_local(async move {
            loop {
                store.refresh_again.set(false);
                let api = store.api.clone();
                match bg(async move { api.tree().await }).await {
                    Ok(mut tree) => {
                        store.cache.save_tree(&tree);
                        add_queued(&mut tree, &store.cache.queued());
                        *store.tree.borrow_mut() = tree;
                        store.tree_changed();
                    }
                    Err(crate::api::Error::Unauthorized) => {
                        if let Some(f) = &*store.on_signed_out.borrow() {
                            f();
                        }
                    }
                    Err(_) => {}
                }
                if !store.refresh_again.get() {
                    break;
                }
            }
            store.refreshing.set(false);
        });
    }

    /// Makes a note in `notebook_id` (None: the top level). Offline it's queued and made on reconnect.
    pub async fn create_note(self: &Rc<Self>, notebook_id: Option<String>) -> Option<String> {
        let id = uuid::Uuid::now_v7().to_string();
        let api = self.api.clone();
        let (nid, nb) = (id.clone(), notebook_id.clone());
        match bg(async move { api.create_note(nb.as_deref(), Some(&nid)).await }).await {
            Ok(_) => {}
            Err(e) if e.is_offline() => {
                self.cache.queue(Queued { id: id.clone(), notebook_id: notebook_id.clone() });
            }
            Err(e) => {
                self.toast(&e.to_string());
                return None;
            }
        }
        self.fresh.borrow_mut().insert(id.clone());
        let note = placeholder(&id, notebook_id, &self.user());
        self.tree.borrow_mut().notes.push(note);
        self.tree_changed();
        Some(id)
    }

    /// Drops a note from the local tree right away (trash, discard), before the server's refresh.
    pub fn forget_note(&self, id: &str) {
        self.tree.borrow_mut().notes.retain(|n| n.id != id);
        self.tree_changed();
    }

    /// Changes a note's title and preview in the local tree as it's typed.
    pub fn retitle(&self, id: &str, text: &str) {
        let (title, preview) = title_and_preview(text);
        let mut tree = self.tree.borrow_mut();
        let Some(n) = tree.notes.iter_mut().find(|n| n.id == id) else { return };
        n.preview = preview;
        n.updated_at = now_ms();
        // Only titles show while a note is open; the rest catches up on the next change.
        if n.title == title {
            return;
        }
        n.title = title;
        drop(tree);
        self.tree_changed();
    }

    pub fn note(&self, id: &str) -> Option<TreeNote> {
        self.tree.borrow().notes.iter().find(|n| n.id == id).cloned()
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// The first line without its heading marks, and some of the text after it, as the server makes them.
pub fn title_and_preview(text: &str) -> (String, String) {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let title = lines.next().unwrap_or_default().trim_start_matches('#').trim().chars().take(200).collect();
    let preview = lines.collect::<Vec<_>>().join(" ").chars().take(200).collect();
    (title, preview)
}

fn placeholder(id: &str, notebook_id: Option<String>, user: &User) -> TreeNote {
    TreeNote {
        id: id.to_owned(),
        notebook_id,
        title: String::new(),
        preview: String::new(),
        owner: user.display_name.clone(),
        role: Role::Owner,
        updated_at: now_ms(),
        shared: false,
        shared_with: Vec::new(),
    }
}

/// Notes made offline aren't on the server yet; the tree shows them anyway.
fn add_queued(tree: &mut Tree, queued: &[Queued]) {
    for q in queued {
        if !tree.notes.iter().any(|n| n.id == q.id) {
            let mut n = placeholder(&q.id, q.notebook_id.clone(), &User {
                id: String::new(),
                username: String::new(),
                display_name: String::new(),
                is_admin: false,
            });
            n.owner.clear();
            tree.notes.push(n);
        }
    }
}

/// A closed note being pushed after a reconnect.
struct Flush {
    store: std::rc::Weak<Store>,
    id: String,
    doc: RefCell<NoteDoc>,
    done: Cell<bool>,
}

impl Flush {
    fn finish(&self) {
        if self.done.replace(true) {
            return;
        }
        if let Some(store) = self.store.upgrade() {
            let me = store.notes.borrow().get(&self.id).cloned();
            if let Some(me) = me {
                store.close(&self.id, &me);
            }
        }
    }
}

impl NoteHandler for Flush {
    fn version(&self) -> Option<VersionVector> {
        Some(self.doc.borrow().version())
    }

    fn joined(&self, role: Role, server: VersionVector) {
        let Some(store) = self.store.upgrade() else { return };
        let doc = self.doc.borrow();
        let ahead = doc.version().partial_cmp(&server).is_none_or(|o| o == std::cmp::Ordering::Greater);
        if ahead && role.can_edit() {
            store.sync.send_update(&self.id, &doc.export_from(&server));
        } else {
            store.cache.clear_dirty(&self.id);
        }
        drop(doc);
        // The server answers a join with its catch-up; once that's in, the next join sees it all.
        self.finish();
    }

    fn update(&self, _: &[u8]) {}
    fn presence(&self, _: &[u8]) {}
    fn role(&self, _: Role) {}

    fn lost(&self, _: &str) {
        if let Some(store) = self.store.upgrade() {
            store.cache.forget(&self.id);
        }
        self.done.set(true);
    }

    fn out_of_sync(&self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_skip_heading_marks() {
        assert_eq!(title_and_preview("# Groceries\n\n- milk\n- eggs"), ("Groceries".into(), "- milk - eggs".into()));
        assert_eq!(title_and_preview(""), (String::new(), String::new()));
    }
}
