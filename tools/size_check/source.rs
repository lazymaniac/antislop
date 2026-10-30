use super::*;
#[derive(Clone)]
pub(super) struct Source {
    pub(super) text: String,
    pub(super) lines: Vec<usize>,
    pub(super) path: PathBuf,
    pub(super) fs_path: PathBuf,
}
impl Source {
    pub(super) fn new(path: PathBuf, fs_path: PathBuf, text: String) -> Self {
        let lines = line_starts(&text);
        Self {
            text,
            lines,
            path,
            fs_path,
        }
    }
    pub(super) fn line(&self, pos: usize) -> usize {
        self.lines.partition_point(|p| *p <= pos).max(1)
    }
    pub(super) fn line_count(&self) -> usize {
        physical_lines(&self.text)
    }
    pub(super) fn production_line_count(&self, ast: &syn::File) -> usize {
        let mut excluded = TestLineCollector { ranges: Vec::new() };
        excluded.visit_file(ast);
        let mut lines = vec![false; self.line_count()];
        for (start, end) in excluded.ranges {
            let first = start.saturating_sub(1);
            let last = end.min(lines.len());
            for line in lines.get_mut(first..last).into_iter().flatten() {
                *line = true;
            }
        }
        lines.iter().filter(|excluded| !**excluded).count()
    }
    pub(super) fn offset(&self, line: usize) -> usize {
        self.lines
            .get(line.saturating_sub(1))
            .copied()
            .unwrap_or(self.text.len())
    }
    pub(super) fn count_span(&self, start: usize, end: usize) -> usize {
        let s = self.line(start);
        let e = self.line(end.saturating_sub(1).max(start));
        e.saturating_sub(s) + 1
    }
}

struct TestLineCollector {
    ranges: Vec<(usize, usize)>,
}
impl TestLineCollector {
    fn exclude(&mut self, attrs: &[Attribute], start: usize, end: usize) -> bool {
        if attrs_imply_test(attrs).unwrap_or(false) {
            self.ranges.push((start, end));
            true
        } else {
            false
        }
    }
}
impl<'ast> Visit<'ast> for TestLineCollector {
    fn visit_item(&mut self, item: &'ast Item) {
        let attrs = item_attrs(item);
        let start = attrs
            .iter()
            .map(Spanned::span)
            .map(|span| span.start().line)
            .min()
            .unwrap_or_else(|| item.span().start().line);
        if self.exclude(attrs, start, item.span().end().line) {
            return;
        }
        visit::visit_item(self, item);
    }
    fn visit_expr(&mut self, expr: &'ast syn::Expr) {
        if let syn::Expr::Block(block) = expr
            && self.exclude(
                &block.attrs,
                expr.span().start().line,
                expr.span().end().line,
            )
        {
            return;
        }
        visit::visit_expr(self, expr);
    }
    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        if self.exclude(&arm.attrs, arm.span().start().line, arm.span().end().line) {
            return;
        }
        visit::visit_arm(self, arm);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        let (attrs, span) = impl_item_parts(item);
        if self.exclude(attrs, span.start().line, span.end().line) {
            return;
        }
        visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        let (attrs, span) = trait_item_parts(item);
        if self.exclude(attrs, span.start().line, span.end().line) {
            return;
        }
        visit::visit_trait_item(self, item);
    }

    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        let (attrs, span) = foreign_item_parts(item);
        if self.exclude(attrs, span.start().line, span.end().line) {
            return;
        }
        visit::visit_foreign_item(self, item);
    }
}

fn impl_item_parts(item: &syn::ImplItem) -> (&[Attribute], proc_macro2::Span) {
    match item {
        syn::ImplItem::Const(item) => (&item.attrs, item.span()),
        syn::ImplItem::Fn(item) => (&item.attrs, item.span()),
        syn::ImplItem::Macro(item) => (&item.attrs, item.span()),
        syn::ImplItem::Type(item) => (&item.attrs, item.span()),
        _ => (&[], item.span()),
    }
}

fn trait_item_parts(item: &syn::TraitItem) -> (&[Attribute], proc_macro2::Span) {
    match item {
        syn::TraitItem::Const(item) => (&item.attrs, item.span()),
        syn::TraitItem::Fn(item) => (&item.attrs, item.span()),
        syn::TraitItem::Macro(item) => (&item.attrs, item.span()),
        syn::TraitItem::Type(item) => (&item.attrs, item.span()),
        _ => (&[], item.span()),
    }
}

fn foreign_item_parts(item: &syn::ForeignItem) -> (&[Attribute], proc_macro2::Span) {
    match item {
        syn::ForeignItem::Fn(item) => (&item.attrs, item.span()),
        syn::ForeignItem::Static(item) => (&item.attrs, item.span()),
        syn::ForeignItem::Type(item) => (&item.attrs, item.span()),
        syn::ForeignItem::Macro(item) => (&item.attrs, item.span()),
        _ => (&[], item.span()),
    }
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        _ => &[],
    }
}
pub(super) fn line_starts(s: &str) -> Vec<usize> {
    let mut lines = vec![0];
    for (index, byte) in s.bytes().enumerate() {
        if byte == b'\n' && index + 1 < s.len() {
            lines.push(index + 1);
        }
    }
    lines
}

pub(super) fn physical_lines(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.bytes().filter(|b| *b == b'\n').count() + usize::from(!s.ends_with('\n'))
    }
}
pub(super) fn offset(span: proc_macro2::Span, src: &Source) -> usize {
    src.offset(span.start().line)
}
