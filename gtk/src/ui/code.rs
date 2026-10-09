//! Fenced code blocks, like web/src/lib/codeBlocks.ts: colored by language, and blocks without a
//! language get a best guess, used for colors only, so nothing is written into a shared note behind
//! anyone's back. The language can be picked from a label on the block.

use std::sync::LazyLock;

use regex::Regex;
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

/// The picker's choices: what's written after the fence, and what it's called.
pub const COMMON: &[(&str, &str)] = &[
    ("bash", "Shell"),
    ("c", "C"),
    ("cpp", "C++"),
    ("csharp", "C#"),
    ("css", "CSS"),
    ("diff", "Diff"),
    ("dockerfile", "Dockerfile"),
    ("go", "Go"),
    ("html", "HTML"),
    ("java", "Java"),
    ("javascript", "JavaScript"),
    ("json", "JSON"),
    ("kotlin", "Kotlin"),
    ("lua", "Lua"),
    ("markdown", "Markdown"),
    ("php", "PHP"),
    ("powershell", "PowerShell"),
    ("python", "Python"),
    ("ruby", "Ruby"),
    ("rust", "Rust"),
    ("sql", "SQL"),
    ("swift", "Swift"),
    ("toml", "TOML"),
    ("typescript", "TypeScript"),
    ("xml", "XML"),
    ("yaml", "YAML"),
];

/// Written after the fence to turn guessing off for a block.
pub const PLAIN: &str = "text";

pub fn name_of(lang: &str) -> String {
    COMMON.iter().find(|(id, _)| *id == lang).map_or_else(|| lang.to_owned(), |(_, n)| (*n).to_owned())
}

// Each rule adds to a language's score when it matches; the highest score wins. As in the PWA.
static RULES: LazyLock<Vec<(&str, Regex, i32)>> = LazyLock::new(|| {
    let r = |s: &str| Regex::new(s).expect("rule");
    vec![
        ("python", r(r"(?m)^\s*(def \w+\(|class \w+(\(.*\))?:|from [\w.]+ import |import \w+$|elif |if __name__)"), 3),
        ("python", r(r"(?m):\s*$\n\s{4}\S"), 1),
        ("python", r(r"\b(self\.|print\(|None\b|True\b|False\b)"), 1),
        ("rust", r(r"\b(fn \w+|let mut |impl\b|pub fn|println!|-> \w+ \{|&mut |use \w+::)"), 3),
        ("go", r(r"(?m)^package \w+|\bfunc (\(\w+ \*?\w+\) )?\w+\(|:= |fmt\."), 3),
        ("typescript", r(r"\b(interface \w+|type \w+ =|: (string|number|boolean)\b|as const\b|<\w+>\()"), 3),
        ("javascript", r(r#"\b(const|let|var) \w+ =|=> |function\s*\w*\(|console\.log|require\(|import .+ from ['"]|export (default|const|function)"#), 2),
        ("java", r(r"\b(public|private|protected) (static )?(class|void|final|\w+ \w+\()|System\.out\."), 3),
        ("cpp", r(r"#include\s*<|std::|cout\s*<<"), 3),
        ("c", r(r"#include\s*<\w+\.h>|\bprintf\(|int main\("), 2),
        ("bash", r(r"(?m)^\s*(\$ |#!/.*\b(ba|z)?sh|sudo |apt |brew |npm |npx |cd |ls |echo |export |git |curl |docker |kubectl |cargo )"), 3),
        ("sql", r(r"(?im)^\s*(select .+ from|insert into|update \w+ set|delete from|create (table|index)|alter table)\b"), 4),
        ("css", r(r"(?m)^\s*[.#]?[\w-]+(\s*[,>+~]?\s*[.#]?[\w-]+)*\s*\{\s*$"), 1),
        ("css", r(r"(?m)^\s*[\w-]+:\s*[^;{]+;\s*$"), 2),
        ("html", r(r"(?im)^\s*<(!doctype|html|head|body|div|span|p|a|ul|li|section|script)\b"), 4),
        ("yaml", r(r"(?m)^\s*[\w-]+:( .+)?$"), 1),
        ("yaml", r(r"(?m)^\s*- [\w-]+: "), 2),
        ("lua", r(r"(?m)\blocal (function )?\w+|\belseif\b|~=|\bthen\s*$|\bnil\b|\w+:\w+\("), 2),
        ("lua", r(r"(?m)^\s*end[),]*\s*$|\bvim\.\w"), 2),
        ("lua", r(r#"(?m)\bi?pairs\(|\brequire\(?["']|^\s*--[^-]"#), 1),
        ("toml", r(r"(?m)^\s*\[\[?[\w.-]+\]\]?\s*$"), 2),
        ("toml", r(r#"(?m)^\s*[\w-]+ = ("|'|\d|true\b|false\b|\[|\{)"#), 1),
        ("dockerfile", r(r"(?m)^(FROM \S+|RUN |COPY |WORKDIR |ENTRYPOINT |CMD \[)"), 3),
        ("diff", r(r"(?m)^(diff --git|@@ -\d+(,\d+)? \+\d+(,\d+)? @@|--- a/|\+\+\+ b/)"), 4),
    ]
});

/// A best guess at the language of some code, by its id in COMMON, or None when nothing fits.
pub fn guess_language(code: &str) -> Option<&'static str> {
    let text = code.trim();
    if text.is_empty() {
        return None;
    }
    if text.starts_with(['[', '{']) && serde_json::from_str::<serde_json::Value>(text).is_ok() {
        return Some("json");
    }
    let mut scores: Vec<(&'static str, i32)> = Vec::new();
    for (lang, re, n) in RULES.iter() {
        if re.is_match(text) {
            match scores.iter_mut().find(|(l, _)| l == lang) {
                Some((_, s)) => *s += n,
                None => scores.push((lang, *n)),
            }
        }
    }
    // Braces and semicolons mean it isn't YAML.
    if text.contains(['{', '}', ';']) {
        scores.retain(|(l, _)| *l != "yaml");
    }
    let mut best = None;
    let mut top = 1;
    for (lang, n) in scores {
        if n > top || (n == top && best.is_none()) {
            best = Some(lang);
            top = n;
        }
    }
    best
}

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

fn theme(dark: bool) -> &'static Theme {
    &THEMES.themes[if dark { "base16-ocean.dark" } else { "InspiredGitHub" }]
}

/// Colored runs of `code` in `lang`: (start, end) in characters, and the color. Text in the theme's
/// own foreground is left out, so it keeps the note's text color.
pub fn highlight(code: &str, lang: &str, dark: bool) -> Vec<(usize, usize, (u8, u8, u8))> {
    // Grammars the default set has under another name, or a close cousin.
    let token = match lang {
        "typescript" | "ts" | "jsx" | "tsx" => "js",
        "kotlin" | "csharp" | "swift" => "java",
        "shell" | "sh" | "zsh" | "console" | "dockerfile" | "powershell" => "bash",
        "yml" => "yaml",
        "cpp" | "c++" => "cpp",
        "toml" => "ini",
        other => other,
    };
    let Some(syntax) = SYNTAXES.find_syntax_by_token(token) else { return Vec::new() };
    let theme = theme(dark);
    let default = theme.settings.foreground;
    let mut h = HighlightLines::new(syntax, theme);
    let mut out = Vec::new();
    let mut at = 0;
    for line in LinesWithEndings::from(code) {
        let Ok(parts) = h.highlight_line(line, &SYNTAXES) else { return out };
        for (style, piece) in parts {
            let n = piece.chars().count();
            let c = style.foreground;
            if Some(c) != default && !piece.trim().is_empty() {
                out.push((at, at + n, (c.r, c.g, c.b)));
            }
            at += n;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_and_colors() {
        assert_eq!(guess_language("fn main() {\n    let mut x = 1;\n}"), Some("rust"));
        assert_eq!(guess_language("kubectl get pods -A"), Some("bash"));
        assert_eq!(guess_language("{\"a\": 1}"), Some("json"));
        assert_eq!(guess_language("just some words"), None);
        let runs = highlight("fn main() {}\n", "rust", false);
        assert!(!runs.is_empty());
        assert!(runs.iter().all(|(s, e, _)| s < e && *e <= 13));
    }
}
