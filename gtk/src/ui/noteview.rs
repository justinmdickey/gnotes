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
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NoteView {
        const NAME: &'static str = "GnotesNoteView";
        type Type = super::NoteView;
        type ParentType = gtk::TextView;
    }

    impl ObjectImpl for NoteView {}

    impl WidgetImpl for NoteView {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let view = self.obj();
            let margin = ((width - COLUMN) / 2).max(MIN_MARGIN);
            if view.left_margin() != margin {
                // Changing margins resizes the view; do it after this allocation.
                let view = view.clone();
                glib::idle_add_local_once(move || {
                    view.set_left_margin(margin);
                    view.set_right_margin(margin);
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

    pub fn pictures(&self) -> Vec<Picture> {
        self.imp().pictures.borrow().clone()
    }

    /// The width a photo may take: the text column.
    pub fn column_width(&self) -> i32 {
        (self.width() - self.left_margin() - self.right_margin()).max(100)
    }
}

impl Default for NoteView {
    fn default() -> Self {
        Self::new()
    }
}
