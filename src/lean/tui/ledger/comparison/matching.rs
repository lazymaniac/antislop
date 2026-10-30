// Hirschberg LCS: rows use O(n) memory; no matrix proportional to both texts.
pub(super) fn lcs_pairs(
    a: &[&str],
    b: &[&str],
    ai: usize,
    bi: usize,
    out: &mut Vec<(usize, usize)>,
) {
    if a.is_empty() || b.is_empty() {
        return;
    }
    if a.len().saturating_mul(b.len()) > 1_000_000 {
        append_lcs_edges(a, b, ai, bi, out);
        return;
    }
    if a.len() == 1 {
        if let Some(index) = b.iter().position(|v| *v == a[0]) {
            out.push((ai, bi + index));
        }
        return;
    }
    let mid = a.len() / 2;
    let split = lcs_split(a, b, mid);
    lcs_pairs(&a[..mid], &b[..split], ai, bi, out);
    lcs_pairs(&a[mid..], &b[split..], ai + mid, bi + split, out);
}

fn append_lcs_edges(a: &[&str], b: &[&str], ai: usize, bi: usize, out: &mut Vec<(usize, usize)>) {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    out.extend((0..prefix).map(|i| (ai + i, bi + i)));
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    out.extend((0..suffix).map(|i| (ai + a.len() - suffix + i, bi + b.len() - suffix + i)));
}

fn lcs_split(a: &[&str], b: &[&str], mid: usize) -> usize {
    let left = lcs_row(&a[..mid], b, false);
    let right = lcs_row(&a[mid..], b, true);
    (0..=b.len())
        .max_by_key(|&i| left[i] + right[b.len() - i])
        .unwrap_or(0)
}

fn lcs_row(left: &[&str], right: &[&str], reverse: bool) -> Vec<usize> {
    let mut values = vec![0usize; right.len() + 1];
    for i in 0..left.len() {
        lcs_row_step(left, right, reverse, i, &mut values);
    }
    values
}

fn lcs_row_step(left: &[&str], right: &[&str], reverse: bool, i: usize, values: &mut [usize]) {
    let mut diagonal = 0;
    for j in 0..right.len() {
        let previous = values[j + 1];
        let x = left[if reverse { left.len() - 1 - i } else { i }];
        let y = right[if reverse { right.len() - 1 - j } else { j }];
        values[j + 1] = if x == y {
            diagonal + 1
        } else {
            values[j + 1].max(values[j])
        };
        diagonal = previous;
    }
}

pub(super) fn changed_graphemes(before: &str, after: &str) -> (Vec<bool>, Vec<bool>) {
    use unicode_segmentation::UnicodeSegmentation;
    let old: Vec<_> = before.graphemes(true).collect();
    let new: Vec<_> = after.graphemes(true).collect();
    let mut old_changed = vec![true; old.len()];
    let mut new_changed = vec![true; new.len()];
    let a: Vec<_> = before.split_word_bounds().collect();
    let b: Vec<_> = after.split_word_bounds().collect();
    let ao = token_offsets(&a);
    let bo = token_offsets(&b);
    let alignment = TokenAlignment {
        a: &a,
        b: &b,
        ao: &ao,
        bo: &bo,
    };
    let mut changes = GraphemeChanges {
        old: &old,
        new: &new,
        old_changed: &mut old_changed,
        new_changed: &mut new_changed,
    };
    mark_common_tokens(&alignment, &mut changes);
    (old_changed, new_changed)
}

struct TokenAlignment<'a> {
    a: &'a [&'a str],
    b: &'a [&'a str],
    ao: &'a [(usize, usize)],
    bo: &'a [(usize, usize)],
}

struct GraphemeChanges<'a> {
    old: &'a [&'a str],
    new: &'a [&'a str],
    old_changed: &'a mut [bool],
    new_changed: &'a mut [bool],
}

fn token_offsets(tokens: &[&str]) -> Vec<(usize, usize)> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut offset = 0;
    tokens
        .iter()
        .map(|token| {
            let start = offset;
            offset += token.graphemes(true).count();
            (start, offset)
        })
        .collect()
}

fn mark_common_tokens(alignment: &TokenAlignment<'_>, changes: &mut GraphemeChanges<'_>) {
    let mut pairs = Vec::new();
    lcs_pairs(alignment.a, alignment.b, 0, 0, &mut pairs);
    mark_word_pairs(alignment, changes, &pairs);
}

fn mark_word_pairs(
    alignment: &TokenAlignment<'_>,
    changes: &mut GraphemeChanges<'_>,
    pairs: &[(usize, usize)],
) {
    let mut previous = (0, 0);
    for (i, j) in pairs
        .iter()
        .copied()
        .chain(std::iter::once((alignment.a.len(), alignment.b.len())))
    {
        mark_word_pair((i, j), alignment, changes, &mut previous);
    }
}

fn mark_word_pair(
    pair: (usize, usize),
    alignment: &TokenAlignment<'_>,
    changes: &mut GraphemeChanges<'_>,
    previous: &mut (usize, usize),
) {
    let (i, j) = pair;
    let end = (
        alignment.ao.get(i).map_or(changes.old.len(), |item| item.0),
        alignment.bo.get(j).map_or(changes.new.len(), |item| item.0),
    );
    mark_token_gap(changes, *previous, end);
    if i < alignment.a.len() && j < alignment.b.len() {
        changes.old_changed[alignment.ao[i].0..alignment.ao[i].1].fill(false);
        changes.new_changed[alignment.bo[j].0..alignment.bo[j].1].fill(false);
        *previous = (alignment.ao[i].1, alignment.bo[j].1);
    }
}

fn mark_token_gap(changes: &mut GraphemeChanges<'_>, start: (usize, usize), end: (usize, usize)) {
    let mut pairs = Vec::new();
    lcs_pairs(
        &changes.old[start.0..end.0],
        &changes.new[start.1..end.1],
        start.0,
        start.1,
        &mut pairs,
    );
    for (old_index, new_index) in pairs {
        changes.old_changed[old_index] = false;
        changes.new_changed[new_index] = false;
    }
}
