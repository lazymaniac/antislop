use super::*;

/// The longest effect summary a card gets, in bytes of stored text.
pub const EFFECT_LIMIT: usize = 240;
/// The longest integration detail a card gets, in bytes of stored text.
pub const DETAIL_LIMIT: usize = 600;
/// How many file names a card lists; the rest are only counted.
pub const FILE_LIMIT: usize = 20;
/// How many unknown ids a coverage preview lists; the rest are only counted.
pub const UNKNOWN_ID_LIMIT: usize = 8;
/// The longest unknown id a coverage preview lists, in bytes of stored text.
pub const UNKNOWN_ID_BYTES: usize = 32;
/// The mark of a cut: the last grapheme of text [`bound_cut`] trimmed for
/// having no room for the whole.
pub const CUT_MARK: char = '…';

/// Cuts text to `limit` bytes — the budget a stored payload pays in UTF-8,
/// never graphemes, since one cluster can cost tens of bytes — taking the
/// longest run of whole grapheme clusters that still leaves room for
/// [`CUT_MARK`], so a stored card string never splits a combined character
/// (ZWJ families, flags, combining marks) and never grows past `limit`.
/// The mark is the cut's last grapheme. When even the first cluster finds
/// no room beside the mark: a cluster that fits `limit` whole is kept as
/// the whole answer (no mark can join it), one that fits neither becomes
/// the mark alone if the mark fits, else nothing; `limit` 0 is always
/// empty, and at `limit` 1 the answer is the first cluster whole when it
/// is a single byte, else empty — never the mark standing in for content.
/// The flag says whether the text was trimmed: that is the effect's
/// `effect_cut` metadata, recorded when the cut happens — a consumer must
/// not recognise a cut by a trailing mark, which honest text can end with
/// and a kept whole cluster lacks.
pub fn bound_cut(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_owned(), false);
    }
    let budget = limit.saturating_sub(CUT_MARK.len_utf8());
    let end = grapheme_prefix_end(text, budget);
    let head = text[..end].trim_end();
    if !head.is_empty() {
        return (format!("{head}{CUT_MARK}"), true);
    }
    overflow_cluster(text, limit)
}

fn grapheme_prefix_end(text: &str, budget: usize) -> usize {
    let mut end = 0;
    for (index, grapheme) in text.grapheme_indices(true) {
        let next = index + grapheme.len();
        if next > budget {
            break;
        }
        end = next;
    }
    end
}

fn overflow_cluster(text: &str, limit: usize) -> (String, bool) {
    let first = text.graphemes(true).next().unwrap_or_default();
    if first.len() <= limit {
        return (first.to_owned(), true);
    }
    if limit >= CUT_MARK.len_utf8() {
        return (CUT_MARK.to_string(), true);
    }
    (String::new(), true)
}

/// [`bound_cut`] with only the text.
pub fn bound(text: &str, limit: usize) -> String {
    bound_cut(text, limit).0
}
