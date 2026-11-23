use super::super::wrap;
use super::*;
use crate::lean::tui::sanitize::sanitize;
use unicode_segmentation::UnicodeSegmentation;

// Frozen pre-change wrapper: a compatibility oracle, never a production path.
/// Word wrap by display width, respecting grapheme cluster boundaries.
/// Wrap at spaces, splitting only a word wider than a row; a list item's
/// continuation rows line up under the text after its marker.
fn eager_before(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![];
    let sanitized = sanitize(text);
    for paragraph in sanitized.split('\n') {
        let hang = hanging_indent(paragraph).filter(|hang| hang * 2 <= width);
        let indent = hang.map_or(String::new(), |hang| " ".repeat(hang));
        let mut line = String::new();
        let mut used = 0;
        let mut break_line = |line: &mut String, used: &mut usize| {
            lines.push(std::mem::replace(line, indent.clone()));
            *used = indent.len();
        };
        for word in paragraph.split_inclusive(' ') {
            let w = word.width();
            if used > indent.len() && used + w.min(width) > width {
                break_line(&mut line, &mut used);
            }
            for g in word.graphemes(true) {
                let gw = g.width();
                if used + gw > width && used > indent.len() {
                    break_line(&mut line, &mut used);
                }
                line.push_str(g);
                used += gw;
            }
        }
        lines.push(line);
    }
    lines
}

#[test]
fn shared_wrapper_matches_original_and_fragmented_input() {
    let samples = [
        "",
        "a",
        "a\n",
        "\n\n",
        "cat on mat",
        "   - list text wraps here",
        "1. numbered list with words",
        "123. numbered",
        "1234. plain",
        "• bullet list",
        "-a plain",
        " é 👩‍👩‍👧‍👦 🇵🇱 words",
        "\x1b[31mred\x1b[0m\tend",
        "a\x1b]52;c;hidden\x1b\\b",
        "a\u{9d}hidden\u{9c}é",
        "Safe\x1b[31",
        "a\r\nb",
        "لا لا",
        "👩‍💻\u{200b}",
        "👩‍💻 \u{200b}",
        "\u{200b}",
        " ⵏ⵿ⴾ",
        " xⵏ⵿ⴾ",
        " ⵏ‍ⴾ",
        " א‍ל",
        " ᨕᨗ‍ᨐ",
        " ꓹꓼ",
        " 𐰲‍𐰃",
        "a\u{202e}b",
    ];
    for text in samples {
        for width in 0..40 {
            assert_eq!(
                wrap(text, width),
                eager_before(text, width),
                "{text:?} width={width}"
            );
            for boundary in text.char_indices().map(|(i, _)| i).chain([text.len()]) {
                let fragments = [&text[..boundary], &text[boundary..]];
                assert_eq!(
                    wrapped_rows(fragments, width, Paragraphs::Split).collect::<Vec<_>>(),
                    wrap(text, width),
                    "fragment at {boundary} {text:?}"
                );
                let expected = sanitize(text)
                    .lines()
                    .flat_map(|line| eager_before(line, width))
                    .collect::<Vec<_>>();
                assert_eq!(
                    wrapped_rows(fragments, width, Paragraphs::Lines).collect::<Vec<_>>(),
                    expected
                );
            }
        }
    }
    let pieces = [
        "a",
        " ",
        "é",
        "👩‍💻",
        "-",
        "1.",
        "\n",
        "\t",
        "\x1b[31m",
        "\u{7}",
        "ⵏ⵿ⴾ",
        "ᨕᨗ‍ᨐ",
        "א‍ל",
        "ꓹꓼ",
        "𐰲‍𐰃",
        "🇵‍🇱",
    ];
    let mut seed = 7_u64;
    for _ in 0..1000 {
        let mut text = String::new();
        for _ in 0..20 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            text.push_str(pieces[(seed >> 32) as usize % pieces.len()]);
        }
        for width in [1, 2, 3, 7, 16, 32] {
            assert_eq!(
                wrap(&text, width),
                eager_before(&text, width),
                "{text:?} width={width}"
            );
        }
    }
}

#[test]
fn sanitized_stream_keeps_extended_graphemes_whole() {
    for text in [
        "a\x1b[31m\u{301}b",
        "👩\x1b]hidden\x07‍💻x",
        "🇵🇱🇺🇦x",
        "क्‍षx",
        " \u{301}x",
    ] {
        let clean = sanitize(text);
        let expected = clean.graphemes(true).map(str::to_owned).collect::<Vec<_>>();
        let actual = Graphemes {
            characters: crate::lean::tui::sanitize::sanitized_characters([text]),
            cluster: String::new(),
        }
        .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{text:?}");
    }
}

#[test]
fn wrap_preserves_a_space_with_its_combining_mark() {
    let text = "a \u{301}b";
    let before = eager_before(text, 2);
    eprintln!("original space-combining boundary: {before:?}");
    assert_eq!(before, ["a ", "\u{301}b"]);
    assert_eq!(wrap(text, 2), ["a \u{301}", "b"]);
    assert_eq!(wrap("- \u{301}abc def", 6), ["- \u{301}abc ", "  def"]);
}
