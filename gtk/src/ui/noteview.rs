//! The note's text view: a `GtkTextView` that keeps the text in a centered column, and draws other
//! people's cursors and selections, and the photos in the note, over the text.

use std::cell::RefCell;

use gtk::{glib, graphene, gsk, prelude::*, subclass::prelude::*};

/// Someone else's cursor, in characters.
#[derive(Clone, Debug)]
pub struct RemoteCursor {
    pub anchor: usize,
    pub head: usize,
    pub name: String,
    pub rgb: (f32, f32, f32),
}

/// A photo, or a recording's play button, shown under the line that links it.
#[derive(Clone)]
pub struct Picture {
    /// Where its link ends.
    pub at: usize,
    /// The attachment.
    pub id: String,
    /// A photo; without one, a button with `label`.
    pub texture: Option<gtk::gdk::Texture>,
    pub label: String,
    /// The size it's drawn at.
    pub width: f32,
    pub height: f32,
}

/// What live preview draws in place of Markdown marks, in characters.
#[derive(Clone, Debug, Default)]
pub struct Decor {
    /// Each checkbox's `[`, and whether it's ticked.
    pub checks: Vec<(usize, bool)>,
    /// Bullet list markers.
    pub bullets: Vec<usize>,
    /// Each quoted line's `>`.
    pub quotes: Vec<usize>,
    /// The note is empty: "Title" shows where the title goes.
    pub placeholder: bool,
    pub accent: Option<gtk::gdk::RGBA>,
    /// Each code block's language, on its opening fence's line (where that starts).
    pub code_labels: Vec<(usize, String)>,
    /// Code blocks, from their first line to their last, drawn as a panel behind the text.
    pub code_areas: Vec<(usize, usize)>,
}

/// A recording's play button.
pub const BUTTON: (f32, f32) = (190.0, 36.0);

/// The widest the text column gets.
pub const COLUMN: i32 = 720;
const MIN_MARGIN: i32 = 24;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct NoteView {
        pub cursors: RefCell<Vec<RemoteCursor>>,
        pub pictures: RefCell<Vec<Picture>>,
        pub decor: RefCell<Decor>,
        /// Called when the width changes, as text rewraps and what's placed over it moves.
        pub on_width: RefCell<Option<Box<dyn Fn()>>>,
        pub last_width: std::cell::Cell<i32>,
        /// Where each code block's language label was drawn, to click.
        pub label_rects: RefCell<Vec<graphene::Rect>>,
        /// Lines others wrote, by where they start, in their writer's color.
        pub bars: RefCell<Vec<(usize, (f32, f32, f32))>>,
        /// The grip beside the line pointed at (where it starts), and where a dragged block would land.
        pub grip: std::cell::Cell<Option<usize>>,
        pub drop_at: std::cell::Cell<Option<usize>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NoteView {
        const NAME: &'static str = "GnotesNoteView";
        type Type = super::NoteView;
        type ParentType = gtk::TextView;
    }

    impl ObjectImpl for NoteView {
        fn dispose(&self) {
            // Popovers set on the view (the "/" menu, the AI Prompt, a photo's menu) aren't children
            // GtkTextView knows of; it would loop forever trying to remove them.
            let mut child = self.obj().first_child();
            while let Some(c) = child {
                child = c.next_sibling();
                if c.is::<gtk::Popover>() {
                    c.unparent();
                }
            }
        }
    }

    impl WidgetImpl for NoteView {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let view = self.obj();
            if self.last_width.replace(width) != width
                && let Some(f) = &*self.on_width.borrow()
            {
                f();
            }
            let margin = ((width - COLUMN) / 2).max(MIN_MARGIN);
            if view.left_margin() != margin {
                // Changing margins resizes the view; do it after this allocation.
                let view = view.clone();
                glib::idle_add_local_once(move || {
                    view.set_left_margin(margin);
                    view.set_right_margin(margin);
                    // The text moved: so does what's placed over it.
                    if let Some(f) = &*view.imp().on_width.borrow() {
                        f();
                    }
                });
            }
            self.parent_size_allocate(width, height, baseline);
        }
    }

    impl TextViewImpl for NoteView {
        fn snapshot_layer(&self, layer: gtk::TextViewLayer, snapshot: gtk::Snapshot) {
            let view = self.obj();
            let buffer = view.buffer();
            if layer == gtk::TextViewLayer::BelowText {
                let fg = view.color();
                let dark = fg.red() + fg.green() + fg.blue() > 1.5;
                let panel = gtk::gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), if dark { 0.07 } else { 0.05 });
                for &(s, e) in &self.decor.borrow().code_areas {
                    let (y0, _) = view.line_yrange(&buffer.iter_at_offset(s as i32));
                    let (y1, h1) = view.line_yrange(&buffer.iter_at_offset(e as i32));
                    let x = view.left_margin() as f32 - 12.0;
                    let rect = graphene::Rect::new(x, y0 as f32, view.column_width() as f32 + 24.0, (y1 + h1 - y0) as f32);
                    snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, 10.0));
                    snapshot.append_color(&panel, &rect);
                    snapshot.pop();
                }
                for c in self.cursors.borrow().iter() {
                    if c.anchor == c.head {
                        continue;
                    }
                    let (a, b) = (c.anchor.min(c.head), c.anchor.max(c.head));
                    let color = gtk::gdk::RGBA::new(c.rgb.0, c.rgb.1, c.rgb.2, 0.25);
                    let mut it = buffer.iter_at_offset(a as i32);
                    let end = buffer.iter_at_offset(b as i32);
                    while it < end {
                        let mut line_end = it;
                        view.forward_display_line_end(&mut line_end);
                        if line_end > end || line_end <= it {
                            line_end = end;
                        }
                        let s = view.iter_location(&it);
                        let e = view.iter_location(&line_end);
                        let w = (e.x() - s.x()).max(4);
                        snapshot.append_color(&color, &graphene::Rect::new(s.x() as f32, s.y() as f32, w as f32, s.height() as f32));
                        if !view.forward_display_line(&mut it) {
                            break;
                        }
                    }
                }
                return;
            }
            view.draw_decor(&self.decor.borrow(), &snapshot);
            // The grip to drag a block by, and the line where a dragged one would go.
            let fg = view.color();
            if let Some(o) = self.grip.get() {
                let (x, y, h) = view.grip_place(o);
                for row in 0..3 {
                    for col in 0..2 {
                        let r = graphene::Rect::new(x + 3.0 + col as f32 * 5.0, y + h / 2.0 - 6.0 + row as f32 * 5.0, 2.5, 2.5);
                        snapshot.append_color(&gtk::gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.5), &r);
                    }
                }
            }
            if let Some(o) = self.drop_at.get() {
                let it = buffer.iter_at_offset(o as i32);
                let (y, _) = view.line_yrange(&it);
                let accent = crate::ui::styling::accent();
                snapshot.append_color(&accent, &graphene::Rect::new(view.left_margin() as f32 - 8.0, y as f32 - 1.0, view.column_width() as f32 + 16.0, 2.0));
            }
            // Who wrote a line someone else typed in, as a bar in their color beside it.
            for &(o, (r, g, b)) in self.bars.borrow().iter() {
                let it = buffer.iter_at_offset(o as i32);
                let (y, h) = view.line_yrange(&it);
                let rect = graphene::Rect::new(view.left_margin() as f32 - 20.0, y as f32 + 1.0, 3.0, h as f32 - 2.0);
                snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, 1.5));
                snapshot.append_color(&gtk::gdk::RGBA::new(r, g, b, 0.8), &rect);
                snapshot.pop();
            }
            for p in self.pictures.borrow().iter() {
                let rect = view.picture_rect(p);
                let radius = if p.texture.is_some() { 8.0 } else { rect.height() / 2.0 };
                snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, radius));
                match &p.texture {
                    Some(t) => snapshot.append_texture(t, &rect),
                    None => {
                        let fg = view.color();
                        snapshot.append_color(&gtk::gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.1), &rect);
                        let layout = view.create_pango_layout(Some(&p.label));
                        let (lw, lh) = layout.pixel_size();
                        snapshot.save();
                        snapshot.translate(&graphene::Point::new(
                            rect.x() + (rect.width() - lw as f32) / 2.0,
                            rect.y() + (rect.height() - lh as f32) / 2.0,
                        ));
                        snapshot.append_layout(&layout, &fg);
                        snapshot.restore();
                    }
                }
                snapshot.pop();
            }
            for c in self.cursors.borrow().iter() {
                let it = buffer.iter_at_offset(c.head as i32);
                let loc = view.iter_location(&it);
                let color = gtk::gdk::RGBA::new(c.rgb.0, c.rgb.1, c.rgb.2, 1.0);
                snapshot.append_color(
                    &color,
                    &graphene::Rect::new(loc.x() as f32 - 1.0, loc.y() as f32, 2.0, loc.height() as f32),
                );
                // The name in a small tag above the cursor.
                let layout = view.create_pango_layout(Some(&c.name));
                let mut font = gtk::pango::FontDescription::new();
                font.set_weight(gtk::pango::Weight::Bold);
                font.set_absolute_size(10.0 * gtk::pango::SCALE as f64);
                layout.set_font_description(Some(&font));
                let (_, ink) = layout.pixel_extents();
                let (w, h) = (ink.width() as f32 + 8.0, 14.0);
                let tag = graphene::Rect::new(loc.x() as f32 - 1.0, loc.y() as f32 - h, w, h);
                snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(tag, 3.0));
                snapshot.append_color(&color, &tag);
                snapshot.pop();
                snapshot.save();
                snapshot.translate(&graphene::Point::new(tag.x() + 4.0, tag.y() + (h - layout.pixel_size().1 as f32) / 2.0));
                snapshot.append_layout(&layout, &gtk::gdk::RGBA::WHITE);
                snapshot.restore();
            }
        }
    }
}

glib::wrapper! {
    pub struct NoteView(ObjectSubclass<imp::NoteView>)
        @extends gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl NoteView {
    pub fn new() -> NoteView {
        glib::Object::builder()
            .property("wrap-mode", gtk::WrapMode::WordChar)
            .property("top-margin", 24)
            .property("bottom-margin", 120)
            .property("left-margin", MIN_MARGIN)
            .property("right-margin", MIN_MARGIN)
            .property("pixels-below-lines", 2)
            .build()
    }

    pub fn set_cursors(&self, cursors: Vec<RemoteCursor>) {
        self.imp().cursors.replace(cursors);
        self.queue_draw();
    }

    pub fn set_pictures(&self, pictures: Vec<Picture>) {
        self.imp().pictures.replace(pictures);
        self.queue_draw();
    }

    /// Where a photo or play button is drawn, in buffer coordinates: under the line linking it.
    pub fn picture_rect(&self, p: &Picture) -> graphene::Rect {
        let it = self.buffer().iter_at_offset(p.at as i32);
        let (y, h) = self.line_yrange(&it);
        graphene::Rect::new(self.left_margin() as f32, (y + h) as f32 - p.height - 8.0, p.width, p.height)
    }

    /// The photo or play button at a point in buffer coordinates.
    pub fn picture_at(&self, x: f32, y: f32) -> Option<Picture> {
        let pictures = self.imp().pictures.borrow();
        pictures.iter().find(|p| self.picture_rect(p).contains_point(&graphene::Point::new(x, y))).cloned()
    }

    pub fn on_width(&self, f: impl Fn() + 'static) {
        self.imp().on_width.replace(Some(Box::new(f)));
    }

    /// Where the grip for the line starting at `o` goes: x, y and the line's height, in buffer coordinates.
    pub fn grip_place(&self, o: usize) -> (f32, f32, f32) {
        let (y, h) = self.line_yrange(&self.buffer().iter_at_offset(o as i32));
        // Its first display line only, for a line that wraps.
        let first = self.iter_location(&self.buffer().iter_at_offset(o as i32)).height().min(h);
        (self.left_margin() as f32 - 34.0, y as f32, first as f32)
    }

    pub fn set_grip(&self, line_start: Option<usize>) {
        if self.imp().grip.replace(line_start) != line_start {
            self.queue_draw();
        }
    }

    pub fn set_drop(&self, at: Option<usize>) {
        if self.imp().drop_at.replace(at) != at {
            self.queue_draw();
        }
    }

    pub fn set_bars(&self, bars: Vec<(usize, (f32, f32, f32))>) {
        self.imp().bars.replace(bars);
        self.queue_draw();
    }

    pub fn set_decor(&self, decor: Decor) {
        self.imp().decor.replace(decor);
        self.queue_draw();
    }

    /// Checkboxes, bullets, quote bars and the empty title, over the text.
    fn draw_decor(&self, d: &Decor, snapshot: &gtk::Snapshot) {
        let buffer = self.buffer();
        let fg = self.color();
        let tone = |a: f32| gtk::gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), a);
        let accent = d.accent.unwrap_or(gtk::gdk::RGBA::new(0.21, 0.52, 0.89, 1.0));
        let at = |o: usize| self.iter_location(&buffer.iter_at_offset(o as i32));
        for &(o, done) in &d.checks {
            let (s, e) = (at(o), at(o + 3));
            let size = (s.height() as f32 * 0.78).min(19.0);
            let (cx, cy) = ((s.x() + e.x()) as f32 / 2.0, s.y() as f32 + s.height() as f32 / 2.0);
            let rect = graphene::Rect::new(cx - size / 2.0, cy - size / 2.0, size, size);
            let round = gsk::RoundedRect::from_rect(rect, size / 2.0);
            if done {
                snapshot.push_rounded_clip(&round);
                snapshot.append_color(&accent, &rect);
                snapshot.pop();
                let tick = gsk::PathBuilder::new();
                tick.move_to(cx - size * 0.24, cy + size * 0.02);
                tick.line_to(cx - size * 0.06, cy + size * 0.2);
                tick.line_to(cx + size * 0.25, cy - size * 0.18);
                let stroke = gsk::Stroke::new(1.8);
                stroke.set_line_cap(gsk::LineCap::Round);
                stroke.set_line_join(gsk::LineJoin::Round);
                snapshot.append_stroke(&tick.to_path(), &stroke, &gtk::gdk::RGBA::WHITE);
            } else {
                let w = 1.5;
                snapshot.append_border(&round, &[w; 4], &[tone(0.5); 4]);
            }
        }
        for &o in &d.bullets {
            let s = at(o);
            let r = 3.0;
            let (cx, cy) = (s.x() as f32 + s.width().max(6) as f32 / 2.0, s.y() as f32 + s.height() as f32 / 2.0);
            let rect = graphene::Rect::new(cx - r, cy - r, r * 2.0, r * 2.0);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, r));
            snapshot.append_color(&tone(0.55), &rect);
            snapshot.pop();
        }
        for &o in &d.quotes {
            let it = buffer.iter_at_offset(o as i32);
            let (y, h) = self.line_yrange(&it);
            let x = self.iter_location(&it).x() as f32 - 12.0;
            let rect = graphene::Rect::new(x, y as f32 + 2.0, 3.0, h as f32 - 4.0);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, 1.5));
            snapshot.append_color(&tone(0.25), &rect);
            snapshot.pop();
        }
        let mut rects = Vec::new();
        for (o, label) in &d.code_labels {
            let it = buffer.iter_at_offset(*o as i32);
            let loc = self.iter_location(&it);
            let layout = self.create_pango_layout(Some(&format!("{label}  ▾")));
            // Caption-sized, like a code block's label in GNOME's Text Editor.
            let mut font = self.pango_context().font_description().unwrap_or_default();
            font.set_size(font.size() * 3 / 4);
            layout.set_font_description(Some(&font));
            let (w, h) = layout.pixel_size();
            let right = (self.left_margin() + self.column_width()) as f32;
            let rect = graphene::Rect::new(right - w as f32 - 16.0, loc.y() as f32 + 2.0, w as f32 + 12.0, h as f32 + 4.0);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, 6.0));
            snapshot.append_color(&tone(0.08), &rect);
            snapshot.pop();
            snapshot.save();
            snapshot.translate(&graphene::Point::new(rect.x() + 6.0, rect.y() + 2.0));
            snapshot.append_layout(&layout, &tone(0.6));
            snapshot.restore();
            rects.push(rect);
        }
        self.imp().label_rects.replace(rects);
        if d.placeholder {
            let layout = self.create_pango_layout(Some("Title"));
            let mut font = self.pango_context().font_description().unwrap_or_default();
            font.set_size((font.size() as f64 * 1.75) as i32);
            font.set_weight(gtk::pango::Weight::Heavy);
            layout.set_font_description(Some(&font));
            let s = at(0);
            snapshot.save();
            snapshot.translate(&graphene::Point::new(s.x() as f32, s.y() as f32 + self.pixels_above_lines() as f32 + 10.0));
            snapshot.append_layout(&layout, &tone(0.35));
            snapshot.restore();
        }
    }

    /// Which code block's language label is at a point in buffer coordinates.
    pub fn code_label_at(&self, x: f32, y: f32) -> Option<usize> {
        self.imp().label_rects.borrow().iter().position(|r| r.contains_point(&graphene::Point::new(x, y)))
    }

    pub fn pictures(&self) -> Vec<Picture> {
        self.imp().pictures.borrow().clone()
    }

    /// The width a photo may take: the text column.
    /// The text column the scrolled window around the view has room for, whatever width the view
    /// itself has right now.
    pub fn fit_width(&self) -> i32 {
        let room = self.parent().map_or_else(|| self.width(), |p| p.width());
        let margin = ((room - COLUMN) / 2).max(MIN_MARGIN);
        (room - 2 * margin).max(100)
    }

    pub fn column_width(&self) -> i32 {
        (self.width() - self.left_margin() - self.right_margin()).max(100)
    }
}

impl Default for NoteView {
    fn default() -> Self {
        Self::new()
    }
}
