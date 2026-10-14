//! Markdown styled in place, as text tags: the editor's look, shared with read-only previews (the
//! summary, Tidy Up's result).

use adw::prelude::*;
use gtk::gdk;

use super::{
    code,
    markdown::{self, Styled},
    noteview::{Decor, NoteView}
};

/// A ```kanban block, drawn as a board rather than as code.
pub fn is_board(b: &markdown::CodeBlock) -> bool {
    b.info.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("kanban"))
}

/// The theme's accent color. Read off a widget styled `.accent` rather than from the style manager,
/// so a theme or a gtk.css that sets its own accent is followed too.
pub fn accent() -> gdk::RGBA {
    let probe = gtk::Label::builder().css_classes(["accent"]).build();
    probe.color()
}

/// The theme's color for errors, like `accent`.
pub fn error() -> gdk::RGBA {
    let probe = gtk::Label::builder().css_classes(["error"]).build();
    probe.color()
}

/// The monospace font family the desktop uses.
pub fn monospace() -> String {
    let name = adw::StyleManager::default().monospace_font_name();
    gtk::pango::FontDescription::from_string(&name).family().map(|f| f.to_string()).unwrap_or_else(|| "monospace".into())
}

pub fn make_tags(buffer: &gtk::TextBuffer) {
    for name in markdown::TAGS {
        buffer.create_tag(Some(name), &[]);
    }
    // The line a task in Tasks pointed at, lit for a moment. Not a Markdown tag, so restyling leaves it.
    buffer.create_tag(Some("jumped"), &[]);
}

/// Colors for the Markdown tags, from the view's own text color, so they follow the style in use.
pub fn color_tags(view: &NoteView) {
        // The text's own color, so tags follow the style in use rather than a guess at it.
        let fg = view.color();
        let dark = fg.red() + fg.green() + fg.blue() > 1.5;
        let dim = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.45);
        let accent = accent();
        let mono = monospace();
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
        set("code", &[("family", &mono), ("background-rgba", &code_bg)]);
        // Its panel is drawn behind it (noteview.rs), within the text column.
        set("codeblock", &[("family", &mono)]);
        set("table", &[("family", &mono)]);
        set("quote", &[("foreground-rgba", &dim)]);
        set("mark", &[("foreground-rgba", &dim)]);
        set("bullet", &[("foreground-rgba", &dim), ("weight", &700i32)]);
        set("link", &[("foreground-rgba", &accent), ("underline", &gtk::pango::Underline::Single)]);
        // A checkbox's `[ ]`: monospace, so the box drawn over it has room.
        set("task", &[("family", &mono)]);
        set("done", &[("foreground-rgba", &dim), ("strikethrough", &true)]);
        set("due", &[("foreground-rgba", &dim), ("scale", &0.85f64), ("weight", &700i32)]);
        set("late", &[("foreground-rgba", &error())]);
        set("jumped", &[("paragraph-background-rgba", &gdk::RGBA::new(accent.red(), accent.green(), accent.blue(), 0.22))]);
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
    let table = buffer.tag_table();
    let mut colors = Vec::new();
    table.foreach(|t| {
        if t.name().is_some_and(|n| n.starts_with("syn-")) {
            colors.push(t.clone());
        }
    });
    for t in colors {
        buffer.remove_tag(&t, &start, &end);
    }
    let at = |o: usize| buffer.iter_at_offset(o as i32);
    // Like Apple Notes, the first line is the title even without a "#".
    if !styled.titled && !text.is_empty() {
        let mut eol = buffer.start_iter();
        if !eol.ends_line() {
            eol.forward_to_line_end();
        } else {
            // A blank first line: the tag on its newline gives the cursor the title's size.
            eol.forward_char();
        }
        buffer.apply_tag_by_name("h1", &start, &eol);
    }
    // Nothing to tag in an empty note: the view itself takes the title's look (mod.rs) and space.
    let placeholder = text.is_empty() && view.is_editable();
    if placeholder {
        view.add_css_class("empty-title");
    } else {
        view.remove_css_class("empty-title");
    }
    view.set_pixels_above_lines(if placeholder { 10 } else { 0 });
    for s in &styled.spans {
        buffer.apply_tag_by_name(s.tag, &at(s.start), &at(s.end));
    }
    // A task's due date: small and dim, red once it's late and the box is still open.
    let today = super::tasks::today();
    for (s, e, date, open) in &styled.dues {
        buffer.apply_tag_by_name("due", &at(*s), &at(*e));
        if *open && date.as_str() < today.as_str() {
            buffer.apply_tag_by_name("late", &at(*s), &at(*e));
        }
    }
    for (ranges, tag) in [(&styled.hidden, "hidden"), (&styled.ghosted, "ghost")] {
        for &(s, e) in ranges {
            let line = at(s).line();
            if active.is_none_or(|(a, b)| line < a || line > b) {
                buffer.apply_tag_by_name(tag, &at(s), &at(e));
            }
        }
    }
    let fg = view.color();
    let dark = fg.red() + fg.green() + fg.blue() > 1.5;
    // Code by language; a block with none gets a guess, for colors only.
    let chars: Vec<char> = text.chars().collect();
    let mut labels = Vec::new();
    for b in styled.code_blocks.iter().filter(|b| !is_board(b)) {
        let code: String = chars[b.content.0..b.content.1].iter().collect();
        let named = b.info.split_whitespace().next().unwrap_or_default().to_lowercase();
        let (lang, label) = if named == code::PLAIN {
            (None, "Plain text".to_owned())
        } else if !named.is_empty() {
            (Some(named.clone()), code::name_of(&named))
        } else {
            match code::guess_language(&code) {
                Some(g) => (Some(g.to_owned()), format!("{} (guess)", code::name_of(g))),
                None => (None, "Plain text".to_owned()),
            }
        };
        labels.push((b.fence, label));
        let Some(lang) = lang else { continue };
        for (s, e, (r, g, bl)) in code::highlight(&code, &lang, dark) {
            let name = format!("syn-{r:02x}{g:02x}{bl:02x}");
            let tag = table.lookup(&name).unwrap_or_else(|| {
                let t = buffer.create_tag(Some(&name), &[("foreground-rgba", &gdk::RGBA::new(r as f32 / 255.0, g as f32 / 255.0, bl as f32 / 255.0, 1.0))]).expect("tag");
                t.set_priority(table.size() - 1);
                t
            });
            buffer.apply_tag(&tag, &at(b.content.0 + s), &at(b.content.0 + e));
        }
    }
    view.set_decor(Decor {
        code_labels: labels,
        code_areas: styled.code_blocks.iter().filter(|b| !is_board(b)).map(|b| (b.fence, b.end)).collect(),
        checks: styled.tasks.clone(),
        bullets: styled.bullets.clone(),
        quotes: styled.quotes.clone(),
        placeholder,
        accent: Some(accent()),
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
