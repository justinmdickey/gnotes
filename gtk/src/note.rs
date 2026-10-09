//! A note's Loro document, kept apart from GTK so it can be tested on its own. The editor feeds it
//! typing and applies the [`Op`]s it hands back for everything else: edits from others and undo.
//! Positions are in Unicode characters, like GTK text iters and Loro's own text indexes.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use loro::{
    ContainerTrait,
    ExportMode, LoroDoc, LoroText, LoroValue, Subscription, TextDelta, UndoManager, VersionVector,
    awareness::EphemeralStore,
    cursor::{Cursor, Side},
    event::Diff,
};

/// Presence entries expire this long after their last refresh, as on the server.
pub const PRESENCE_TIMEOUT_MS: i64 = 30_000;

/// One step of a change to the text, in Unicode characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Retain(usize),
    Insert(String),
    Delete(usize),
}

/// Someone else with the note open, as loro-codemirror announces them.
#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    pub peer: String,
    pub name: String,
    /// A `user-*` color class name from the PWA, e.g. `user-blue`.
    pub color: String,
    /// Where their cursor is, and the other end of their selection, if they have one in the text.
    pub cursor: Option<(usize, usize)>,
}

/// `colorFor` in web/src/lib/store.svelte.ts: the same user gets the same color everywhere.
pub fn color_for(user_id: &str) -> &'static str {
    const COLORS: [&str; 5] = ["user-blue", "user-green", "user-orange", "user-purple", "user-red"];
    let mut h: u32 = 0;
    for c in user_id.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(c as u32);
    }
    COLORS[h as usize % COLORS.len()]
}

/// The RGB of a `user-*` color class, as app.css defines them.
pub fn color_rgb(class: &str) -> (f32, f32, f32) {
    let hex = match class {
        "user-blue" => 0x3584e4,
        "user-green" => 0x2ec27e,
        "user-orange" => 0xe66100,
        "user-purple" => 0x9141ac,
        "user-red" => 0xe01b24,
        _ => 0x77767b,
    };
    let c = |shift: u32| ((hex >> shift) & 0xff) as f32 / 255.0;
    (c(16), c(8), c(0))
}

pub struct NoteDoc {
    pub doc: LoroDoc,
    text: LoroText,
    undo: UndoManager,
    pub presence: EphemeralStore,
    /// Changes to the text that didn't come from typing, waiting for the editor.
    pending: Arc<Mutex<Vec<Op>>>,
    typing: Arc<AtomicBool>,
    /// Local updates waiting to be sent.
    outbox: Arc<Mutex<Vec<Vec<u8>>>>,
    presence_out: Arc<Mutex<Vec<Vec<u8>>>>,
    /// Something changed in others' presence since the last look.
    presence_changed: Arc<AtomicBool>,
    _subs: Vec<Subscription>,
    /// Content has arrived (from the server or a saved copy), so it's safe to edit.
    pub has_data: bool,
}

fn to_ops(delta: &[TextDelta]) -> Vec<Op> {
    delta
        .iter()
        .map(|d| match d {
            TextDelta::Retain { retain, .. } => Op::Retain(*retain),
            TextDelta::Insert { insert, .. } => Op::Insert(insert.clone()),
            TextDelta::Delete { delete } => Op::Delete(*delete),
        })
        .collect()
}

/// Applies `ops` to a plain string, as the editor does to its buffer. For tests and checks.
pub fn apply_ops(text: &str, ops: &[Op]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    for op in ops {
        match op {
            Op::Retain(n) => {
                out.extend(&chars[at..at + n]);
                at += n;
            }
            Op::Insert(s) => out.push_str(s),
            Op::Delete(n) => at += n,
        }
    }
    out.extend(&chars[at..]);
    out
}

/// Adds one change after those already waiting. Each is relative to the text after the one before,
/// so a marker keeps them apart.
fn push_delta(pending: &mut Vec<Op>, ops: Vec<Op>) {
    if !pending.is_empty() {
        pending.push(Op::Retain(usize::MAX));
    }
    pending.extend(ops);
}

/// Splits what `push_delta` collected back into one list of ops per change.
pub fn split_changes(ops: Vec<Op>) -> Vec<Vec<Op>> {
    let mut out = vec![Vec::new()];
    for op in ops {
        if op == Op::Retain(usize::MAX) {
            out.push(Vec::new());
        } else {
            out.last_mut().expect("one list").push(op);
        }
    }
    out.retain(|c| !c.is_empty());
    out
}

impl NoteDoc {
    pub fn new() -> NoteDoc {
        let doc = LoroDoc::new();
        // Times on each change, so who-wrote-what can say when.
        doc.set_record_timestamp(true);
        let text = doc.get_text("body");
        let pending: Arc<Mutex<Vec<Op>>> = Default::default();
        let typing = Arc::new(AtomicBool::new(false));
        let outbox: Arc<Mutex<Vec<Vec<u8>>>> = Default::default();
        let presence_out: Arc<Mutex<Vec<Vec<u8>>>> = Default::default();
        let presence_changed = Arc::new(AtomicBool::new(false));

        let sub_text = {
            let pending = pending.clone();
            let typing = typing.clone();
            doc.subscribe(
                &text.id(),
                Arc::new(move |e| {
                    if e.triggered_by.is_local() && typing.load(Ordering::Relaxed) {
                        return;
                    }
                    let mut pending = pending.lock().expect("pending");
                    for c in e.events {
                        if let Diff::Text(delta) = &c.diff {
                            push_delta(&mut pending, to_ops(delta));
                        }
                    }
                }),
            )
        };
        let sub_local = {
            let outbox = outbox.clone();
            doc.subscribe_local_update(Box::new(move |bytes| {
                outbox.lock().expect("outbox").push(bytes.clone());
                true
            }))
        };
        let presence = EphemeralStore::new(PRESENCE_TIMEOUT_MS);
        let sub_presence_out = {
            let out = presence_out.clone();
            presence.subscribe_local_updates(Box::new(move |bytes| {
                out.lock().expect("presence").push(bytes.clone());
                true
            }))
        };
        let sub_presence = {
            let changed = presence_changed.clone();
            presence.subscribe(Box::new(move |_| {
                changed.store(true, Ordering::Relaxed);
                true
            }))
        };

        let mut undo = UndoManager::new(&doc);
        undo.set_merge_interval(1000);
        NoteDoc {
            doc,
            text,
            undo,
            presence,
            pending,
            typing,
            outbox,
            presence_out,
            presence_changed,
            _subs: vec![sub_text, sub_local, sub_presence_out, sub_presence],
            has_data: false,
        }
    }

    pub fn text(&self) -> String {
        self.text.to_string()
    }

    pub fn len(&self) -> usize {
        self.text.len_unicode()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn typed(&self, edit: impl FnOnce(&LoroText)) {
        self.typing.store(true, Ordering::Relaxed);
        edit(&self.text);
        self.doc.commit();
        self.typing.store(false, Ordering::Relaxed);
    }

    /// Typing: the editor already shows it.
    pub fn insert(&self, pos: usize, s: &str) {
        self.typed(|t| {
            let _ = t.insert(pos, s);
        });
    }

    pub fn delete(&self, pos: usize, len: usize) {
        if len > 0 {
            self.typed(|t| {
                let _ = t.delete(pos, len);
            });
        }
    }

    /// Replaces `len` characters at `pos` with `s`, e.g. for a toggled checkbox, as one undo step.
    /// Returns the ops for the editor.
    pub fn replace(&self, pos: usize, len: usize, s: &str) -> Vec<Op> {
        let _ = self.text.delete(pos, len);
        let _ = self.text.insert(pos, s);
        self.doc.commit();
        self.take_ops()
    }

    /// Ends the current undo step, so the next typing is undone separately.
    pub fn checkpoint(&mut self) {
        let _ = self.undo.record_new_checkpoint();
    }

    /// Changes from elsewhere for the editor to apply, each relative to the text after the one before.
    pub fn take_changes(&self) -> Vec<Vec<Op>> {
        split_changes(std::mem::take(&mut *self.pending.lock().expect("pending")))
    }

    fn take_ops(&self) -> Vec<Op> {
        // Only used right after one commit, which makes one change.
        self.take_changes().into_iter().flatten().collect()
    }

    /// Imports an update or snapshot. Returns the changes for the editor.
    pub fn import(&mut self, bytes: &[u8]) -> Result<Vec<Vec<Op>>, loro::LoroError> {
        self.doc.import(bytes)?;
        self.has_data = true;
        Ok(self.take_changes())
    }

    pub fn undo(&mut self) -> Vec<Vec<Op>> {
        let _ = self.undo.undo();
        self.take_changes()
    }

    pub fn redo(&mut self) -> Vec<Vec<Op>> {
        let _ = self.undo.redo();
        self.take_changes()
    }

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    pub fn version(&self) -> VersionVector {
        self.doc.oplog_vv()
    }

    /// Everything `from` doesn't have.
    pub fn export_from(&self, from: &VersionVector) -> Vec<u8> {
        self.doc.export(ExportMode::updates(from)).unwrap_or_default()
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.doc.export(ExportMode::Snapshot).unwrap_or_default()
    }

    /// Local updates made since the last call, to send.
    pub fn take_outbox(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.outbox.lock().expect("outbox"))
    }

    pub fn take_presence_out(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.presence_out.lock().expect("presence"))
    }

    /// Whether others' presence changed since the last call.
    pub fn take_presence_changed(&self) -> bool {
        self.presence_changed.swap(false, Ordering::Relaxed)
    }

    fn user_key(&self) -> String {
        format!("{}-cm-user", self.doc.peer_id())
    }

    fn cursor_key(&self) -> String {
        format!("{}-cm-cursor", self.doc.peer_id())
    }

    /// Announces who this is, in the shape loro-codemirror reads.
    pub fn set_user(&self, name: &str, color: &str) {
        let user: LoroValue = vec![("name", LoroValue::from(name)), ("colorClassName", LoroValue::from(color))]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect::<std::collections::HashMap<_, _>>()
            .into();
        self.presence.set(&self.user_key(), user);
    }

    /// The local cursor, or None when the editor isn't focused.
    pub fn set_cursor(&self, sel: Option<(usize, usize)>) {
        let key = self.cursor_key();
        let Some((anchor, head)) = sel else {
            self.presence.delete(&key);
            return;
        };
        let encode = |pos: usize| self.text.get_cursor(pos, Side::default()).map(|c| LoroValue::Binary(c.encode().into()));
        let Some(a) = encode(anchor) else { return };
        let mut state = vec![("anchor".to_owned(), a)];
        if head != anchor
            && let Some(h) = encode(head)
        {
            state.push(("head".to_owned(), h));
        }
        self.presence.set(&key, state.into_iter().collect::<std::collections::HashMap<_, _>>());
    }

    /// Re-announces this user and cursor, since entries expire without a refresh.
    pub fn refresh_presence(&self) {
        for key in [self.user_key(), self.cursor_key()] {
            if let Some(v) = self.presence.get(&key) {
                self.presence.set(&key, v);
            }
        }
        self.presence.remove_outdated();
    }

    /// Takes this user out of the note's presence, e.g. on closing it.
    pub fn clear_presence(&self) {
        self.presence.delete(&self.cursor_key());
        self.presence.delete(&self.user_key());
    }

    pub fn apply_presence(&self, data: &[u8]) {
        let _ = self.presence.apply(data);
    }

    fn cursor_pos(&self, v: &LoroValue) -> Option<usize> {
        let LoroValue::Binary(b) = v else { return None };
        let cursor = Cursor::decode(b).ok()?;
        Some(self.doc.get_cursor_pos(&cursor).ok()?.current.pos)
    }

    /// Who typed last in each of these character ranges (lines): the peer of the character with the
    /// highest Lamport time, and when (Unix seconds; 0 if not recorded). None for an empty range.
    pub fn writers(&self, ranges: &[(usize, usize)]) -> Vec<Option<(u64, i64)>> {
        ranges
            .iter()
            .map(|&(from, to)| {
                let mut last: Option<(u32, u64, i64)> = None;
                for i in from..to {
                    let Some(id) = self.text.get_cursor(i, Side::Middle).and_then(|c| c.id) else { continue };
                    let Some(change) = self.doc.get_change(id) else { continue };
                    let lamport = change.lamport + (id.counter - change.id.counter) as u32;
                    if last.is_none_or(|(l, _, _)| lamport > l) {
                        last = Some((lamport, id.peer, change.timestamp));
                    }
                }
                last.map(|(_, peer, ts)| (peer, ts))
            })
            .collect()
    }

    pub fn peer_id(&self) -> u64 {
        self.doc.peer_id()
    }

    /// Everyone else with the note open, one entry per editing session.
    pub fn peers(&self) -> Vec<Peer> {
        let mine = self.doc.peer_id().to_string();
        let states = self.presence.get_all_states();
        let mut peers: Vec<Peer> = Vec::new();
        for (key, value) in &states {
            let Some(peer) = key.strip_suffix("-cm-user") else { continue };
            if peer == mine {
                continue;
            }
            let LoroValue::Map(user) = value else { continue };
            let text = |k: &str| match user.get(k) {
                Some(LoroValue::String(s)) => s.to_string(),
                _ => String::new(),
            };
            let cursor = match states.get(&format!("{peer}-cm-cursor")) {
                Some(LoroValue::Map(c)) => c.get("anchor").and_then(|a| self.cursor_pos(a)).map(|a| {
                    let h = c.get("head").and_then(|h| self.cursor_pos(h)).unwrap_or(a);
                    (a, h)
                }),
                _ => None,
            };
            peers.push(Peer { peer: peer.to_owned(), name: text("name"), color: text("colorClassName"), cursor });
        }
        peers.sort_by(|a, b| a.name.cmp(&b.name).then(a.peer.cmp(&b.peer)));
        peers
    }
}

impl Default for NoteDoc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two docs exchanging updates, the way the server relays them.
    fn sync(from: &NoteDoc, to: &mut NoteDoc) -> Vec<Vec<Op>> {
        let mut changes = Vec::new();
        for u in from.take_outbox() {
            changes.extend(to.import(&u).unwrap());
        }
        changes
    }

    #[test]
    fn typing_is_not_echoed_but_remote_edits_are() {
        let mut a = NoteDoc::new();
        let mut b = NoteDoc::new();
        a.insert(0, "Hello");
        assert!(a.take_changes().is_empty(), "typing isn't sent back to the editor");
        let changes = sync(&a, &mut b);
        assert_eq!(changes, vec![vec![Op::Insert("Hello".into())]]);

        b.insert(5, " world");
        let mut shown = a.text();
        for c in sync(&b, &mut a) {
            shown = apply_ops(&shown, &c);
        }
        assert_eq!(shown, "Hello world");
        assert_eq!(a.text(), "Hello world");
    }

    #[test]
    fn positions_are_characters() {
        let mut a = NoteDoc::new();
        let mut b = NoteDoc::new();
        a.insert(0, "🍎 é x");
        sync(&a, &mut b);
        b.delete(4, 1); // the x
        b.insert(2, "!");
        let mut shown = a.text();
        for c in sync(&b, &mut a) {
            shown = apply_ops(&shown, &c);
        }
        assert_eq!(shown, "🍎 !é ");
        assert_eq!(a.text(), shown);
    }

    #[test]
    fn undo_takes_back_only_own_edits() {
        let mut a = NoteDoc::new();
        let mut b = NoteDoc::new();
        a.insert(0, "mine ");
        a.checkpoint();
        sync(&a, &mut b);
        b.insert(5, "theirs");
        sync(&b, &mut a);
        let mut shown = a.text();
        for c in a.undo() {
            shown = apply_ops(&shown, &c);
        }
        assert_eq!(shown, "theirs");
        assert_eq!(a.text(), "theirs");
        for c in a.redo() {
            shown = apply_ops(&shown, &c);
        }
        assert_eq!(a.text(), "mine theirs");
        assert_eq!(shown, a.text());
    }

    #[test]
    fn presence_round_trips() {
        let a = NoteDoc::new();
        let mut b = NoteDoc::new();
        a.insert(0, "🍎 abc");
        sync(&a, &mut b);
        a.set_user("Ann", "user-red");
        a.set_cursor(Some((2, 5)));
        for p in a.take_presence_out() {
            b.apply_presence(&p);
        }
        assert!(b.take_presence_changed());
        let peers = b.peers();
        assert_eq!(peers.len(), 1);
        assert_eq!((peers[0].name.as_str(), peers[0].color.as_str()), ("Ann", "user-red"));
        assert_eq!(peers[0].cursor, Some((2, 5)));
        assert!(a.peers().is_empty(), "you aren't your own peer");
    }

    #[test]
    fn lines_go_to_who_typed_last() {
        let mut a = NoteDoc::new();
        let mut b = NoteDoc::new();
        a.insert(0, "first line\nsecond");
        sync(&a, &mut b);
        b.insert(17, " changed");
        sync(&b, &mut a);
        let w = a.writers(&[(0, 10), (11, 25), (25, 25)]);
        assert_eq!(w[0].map(|x| x.0), Some(a.peer_id()));
        assert_eq!(w[1].map(|x| x.0), Some(b.peer_id()));
        assert_eq!(w[2], None);
    }

    #[test]
    fn colors_match_the_web_app() {
        // colorFor("abc") in the PWA: h = ((97*31)+98)*31+99 = 96354; 96354 % 5 = 4.
        assert_eq!(color_for("abc"), "user-red");
    }
}
