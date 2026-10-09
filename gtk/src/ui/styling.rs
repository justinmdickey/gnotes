//! Markdown styled in place, as text tags: the editor's look, shared with read-only previews (the
//! summary, Tidy Up's result).

use adw::prelude::*;
use gtk::gdk;

use super::{
    markdown::{self, Styled},
    noteview::{Decor, NoteView}
};

pub fn make_tags(buffer: &gtk::TextBuffer) {
    for name in markdown::TAGS {
        buffer.create_tag(Some(name), &[]);
    }
}

/// Colors for the Markdown tags, from the view's own text color, so they follow the style in use.
pub fn color_tags(view: &NoteView) {
        // The text's own color, so tags follow the style in use rather than a guess at it.
        let fg = view.color();
        let dark = fg.red() + fg.green() + fg.blue() > 1.5;
        let dim = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.45);
        let accent = adw::StyleManager::default().accent_color().to_standalone_rgba(dark);
        let code_bg = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), if dark { 0.08 } else { 0.06 });
        let table = view.buffer().tag_table();
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
        // A checkbox's `[ ]`: monospace, so the box drawn over it has room.
        set("task", &[("family", &"monospace")]);
        set("done", &[("foreground-rgba", &dim), ("strikethrough", &true)]);
        // Live preview: marks hidden off the edited line; bullets and boxes drawn over clear text.
        // Pango takes an alpha of 0 as no alpha at all, so this is the least above it.
        let clear = gdk::RGBA::new(0.0, 0.0, 0.0, 1.0 / 65535.0);
        // Hidden marks stay in the text, shrunk to nothing, rather than `invisible`: GTK places things
        // after invisible text as if it were there, which would put cursors and boxes in the wrong spot.
        set("hidden", &[("scale", &0.01f64), ("foreground-rgba", &clear)]);
        set("ghost", &[("foreground-rgba", &clear)]);
        // Higher tags win: marks stay dim inside headings and links.
        for name in ["link", "mark", "task", "ghost", "hidden"] {
            let t = tag(name);
            t.set_priority(table.size() - 1);
        }
}

/// Styles `text` (the buffer's) and draws checkboxes and bullets. `active`: the lines whose marks
/// show, around the cursor; None hides them all.
pub fn apply(view: &NoteView, text: &str, active: Option<(i32, i32)>) -> Styled {
    let buffer = view.buffer();
    let (start, end) = buffer.bounds();
    let styled = markdown::analyze(text);
    for name in markdown::TAGS {
        buffer.remove_tag_by_name(name, &start, &end);
    }
    let at = |o: usize| buffer.iter_at_offset(o as i32);
    // Like Apple Notes, the first line is the title even without a "#".
    if !styled.titled && !text.is_empty() {
        let mut eol = buffer.start_iter();
        if !eol.ends_line() {
            eol.forward_to_line_end();
        }
        buffer.apply_tag_by_name("h1", &start, &eol);
    }
    for s in &styled.spans {
        buffer.apply_tag_by_name(s.tag, &at(s.start), &at(s.end));
    }
    for &(s, e) in &styled.hidden {
        let line = at(s).line();
        if active.is_none_or(|(a, b)| line < a || line > b) {
            buffer.apply_tag_by_name("hidden", &at(s), &at(e));
        }
    }
    let fg = view.color();
    let dark = fg.red() + fg.green() + fg.blue() > 1.5;
    view.set_decor(Decor {
        checks: styled.tasks.clone(),
        bullets: styled.bullets.clone(),
        quotes: styled.quotes.clone(),
        placeholder: text.is_empty() && view.is_editable(),
        accent: Some(adw::StyleManager::default().accent_color().to_standalone_rgba(dark)),
    });
    styled
}

/// A read-only view of Markdown, drawn like a note.
pub fn preview(text: &str) -> NoteView {
    let view = NoteView::new();
    view.add_css_class("note-text");
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_top_margin(12);
    view.set_bottom_margin(24);
    let buffer = view.buffer();
    make_tags(&buffer);
    buffer.set_text(text);
    let restyle = {
        let text = text.to_owned();
        move |v: &NoteView| {
            color_tags(v);
            apply(v, &text, None);
        }
    };
    restyle(&view);
    let r = restyle.clone();
    view.connect_realize(move |v| r(v));
    let weak = view.downgrade();
    let manager = adw::StyleManager::default();
    let id = manager.connect_dark_notify(move |_| {
        let weak = weak.clone();
        let restyle = restyle.clone();
        gtk::glib::idle_add_local_once(move || {
            if let Some(v) = weak.upgrade() {
                restyle(&v);
            }
        });
    });
    let id = std::cell::Cell::new(Some(id));
    view.connect_destroy(move |_| {
        if let Some(id) = id.take() {
            manager.disconnect(id);
        }
    });
    view
}
