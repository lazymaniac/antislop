//! Grapheme-safe ASCII replacements used by the theme.

use unicode_width::UnicodeWidthStr;

pub(super) fn ascii_grapheme(grapheme: &str) -> String {
    if grapheme.is_ascii() {
        return grapheme.to_owned();
    }
    ascii_symbol(grapheme)
        .map(str::to_owned)
        .unwrap_or_else(|| "?".repeat(grapheme.width().max(1)))
}

pub(in crate::lean::tui) fn ascii_character(ch: char) -> char {
    if ch.is_ascii() {
        return ch;
    }
    screen_ascii_symbol(&ch.to_string())
        .and_then(|symbol| symbol.chars().next())
        .unwrap_or('?')
}

fn ascii_symbol(grapheme: &str) -> Option<&'static str> {
    ascii_polish(grapheme).or_else(|| ascii_basic_grapheme(grapheme))
}

fn screen_ascii_symbol(grapheme: &str) -> Option<&'static str> {
    ascii_polish(grapheme)
        .or_else(|| ascii_borders(grapheme))
        .or_else(|| ascii_marks(grapheme))
}

fn ascii_basic_grapheme(grapheme: &str) -> Option<&'static str> {
    Some(match grapheme {
        "→" | "▶" | "↳" => ">",
        "←" | "◀" => "<",
        "↑" | "▲" => "^",
        "↓" | "▼" => "v",
        "✓" => "+",
        "✕" | "✗" => "x",
        "·" | "–" | "—" | "−" => "-",
        "…" => ".",
        _ => return None,
    })
}

fn ascii_polish(grapheme: &str) -> Option<&'static str> {
    Some(match grapheme {
        "ą" => "a",
        "ć" => "c",
        "ę" => "e",
        "ł" => "l",
        "ń" => "n",
        "ó" => "o",
        "ś" => "s",
        "ź" | "ż" => "z",
        "Ą" => "A",
        "Ć" => "C",
        "Ę" => "E",
        "Ł" => "L",
        "Ń" => "N",
        "Ó" => "O",
        "Ś" => "S",
        "Ź" | "Ż" => "Z",
        _ => return None,
    })
}

fn ascii_borders(grapheme: &str) -> Option<&'static str> {
    Some(match grapheme {
        "─" | "–" | "—" | "−" | "·" => "-",
        "═" | "‖" => "=",
        "│" | "║" | "▌" | "⏸" => "|",
        "┌" | "┐" | "└" | "┘" | "├" | "┤" | "┬" | "┴" | "┼" | "╔" | "╗" | "╚" | "╝" | "╭" | "╮"
        | "╯" | "╰" | "⎿" => "+",
        _ => return None,
    })
}

fn ascii_marks(grapheme: &str) -> Option<&'static str> {
    Some(match grapheme {
        "▶" | "→" | "↳" | "›" | "▸" | "◂" => ">",
        "◀" | "←" => "<",
        "▲" | "↑" => "^",
        "▼" | "↓" => "v",
        "…" => ".",
        "○" => "o",
        "•" | "⏺" | "✻" | "✦" | "●" | "⠋" | "⠙" | "⠹" | "⠸" | "⠼" | "⠴" | "⠦" | "⠧" | "⠇" | "⠏" => {
            "*"
        }
        "█" => "#",
        "✕" | "✗" => "x",
        "◆" => "O",
        "◇" => "P",
        "≡" => "A",
        "◈" => "V",
        "✓" => "+",
        "⚠" => "!",
        "⚙" | "✎" => "#",
        "✉" => "@",
        "§" => "S",
        _ => return None,
    })
}
