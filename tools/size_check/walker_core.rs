use super::*;
pub(super) struct Walker<'a> {
    pub(super) src: &'a Source,
    pub(super) module_dir: PathBuf,
    pub(super) path_dir: PathBuf,
    pub(super) production: bool,
    pub(super) external_crates: &'a BTreeSet<String>,
    pub(super) external_macros: BTreeSet<String>,
    pub(super) test_context: bool,
    pub(super) result: &'a mut Results,
}
impl Walker<'_> {
    pub(super) fn cfg_attr_errors(&mut self, attr: &Attribute) {
        if !attr.path().is_ident("cfg_attr") {
            return;
        }
        let mut lines = vec![attr.span().start().line];
        if let Meta::List(list) = &attr.meta {
            collect_cfg_attr_tokens(list.tokens.clone(), &mut lines);
        }
        self.report_cfg_attr_lines(lines);
    }
    pub(super) fn attrs(&mut self, attrs: &[Attribute]) -> bool {
        for a in attrs {
            if a.path().is_ident("cfg_attr") {
                self.cfg_attr_errors(a);
            }
        }
        match attrs_imply_test(attrs) {
            Ok(test) => test,
            Err(error) => {
                let line = attrs
                    .iter()
                    .find(|attr| attr.path().is_ident("cfg"))
                    .map(|attr| attr.span().start().line)
                    .unwrap_or(0);
                self.result.errors.push(format!(
                    "{}:{line}: cannot classify cfg: {error}",
                    self.src.path.display()
                ));
                false
            }
        }
    }
    pub(super) fn cfg_attr_token_errors(&mut self, tokens: &proc_macro2::TokenStream) {
        let mut lines = Vec::new();
        scan_cfg_attr_tokens(tokens.clone(), &mut lines);
        self.report_cfg_attr_lines(lines);
    }

    fn report_cfg_attr_lines(&mut self, lines: Vec<usize>) {
        for line in lines {
            self.result.errors.push(format!(
                "{}:{line}: unsupported cfg_attr classification",
                self.src.path.display()
            ));
        }
    }

    pub(super) fn callable(
        &mut self,
        attrs: &[Attribute],
        ident: &str,
        start: proc_macro2::Span,
        end: proc_macro2::Span,
        test_attr: bool,
    ) {
        let test_only = self.attrs(attrs) || test_attr || self.test_context;
        if !self.production || test_only {
            return;
        }
        if let Some((line, count)) = self.callable_measure(attrs, start, end) {
            self.report_callable(ident, line, count);
        }
    }

    fn callable_measure(
        &self,
        attrs: &[Attribute],
        start: proc_macro2::Span,
        end: proc_macro2::Span,
    ) -> Option<(usize, usize)> {
        let ast_start = offset(start, self.src);
        let first_attr = attrs
            .iter()
            .map(|a| a.span().start().line)
            .min()
            .map(|line| self.src.offset(line))
            .unwrap_or(ast_start);
        let begin = self.src.callable_start(first_attr);
        let finish = self.src.offset(end.end().line);
        Some((
            self.src.line(begin),
            self.src.count_span(begin, finish.saturating_add(1)),
        ))
    }

    fn report_callable(&mut self, ident: &str, line: usize, count: usize) {
        if count <= CALLABLE_LIMIT {
            return;
        }
        self.result.violations.push(format!(
            "{}:{line}: callable {ident} is {count} lines (limit {CALLABLE_LIMIT})",
            self.src.path.display()
        ));
    }
}
pub(super) fn attrs_imply_test(attrs: &[Attribute]) -> Result<bool, String> {
    let mut test = false;
    for attr in attrs {
        if attr.path().is_ident("test") {
            test = true;
        } else if attr.path().is_ident("cfg") {
            test |= implies_test(&attr.parse_args::<Meta>().map_err(|e| e.to_string())?)?;
        }
    }
    Ok(test)
}
pub(super) fn stmt_has_callable(stmt: &syn::Stmt) -> bool {
    struct CallableFinder(bool);
    impl<'ast> Visit<'ast> for CallableFinder {
        fn visit_item_fn(&mut self, _: &'ast syn::ItemFn) {
            self.0 = true;
        }
    }
    let mut finder = CallableFinder(false);
    finder.visit_stmt(stmt);
    finder.0
}

fn collect_cfg_attr_tokens(tokens: proc_macro2::TokenStream, lines: &mut Vec<usize>) {
    if let Ok(items) = syn::parse::Parser::parse2(
        syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
        tokens,
    ) {
        for item in items {
            collect_cfg_attr_meta(&item, lines);
        }
    }
}

fn collect_cfg_attr_meta(meta: &Meta, lines: &mut Vec<usize>) {
    let Meta::List(list) = meta else {
        return;
    };
    if list.path.is_ident("cfg_attr") {
        lines.push(meta.span().start().line);
    }
    if let Ok(items) = syn::parse::Parser::parse2(
        syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
        list.tokens.clone(),
    ) {
        for item in items {
            collect_cfg_attr_meta(&item, lines);
        }
    }
}

fn scan_cfg_attr_tokens(tokens: proc_macro2::TokenStream, lines: &mut Vec<usize>) {
    use proc_macro2::{Delimiter, TokenTree};
    for tree in tokens {
        let TokenTree::Group(group) = tree else {
            continue;
        };
        if group.delimiter() == Delimiter::Bracket
            && let Ok(meta) = syn::parse2::<Meta>(group.stream())
        {
            collect_cfg_attr_meta(&meta, lines);
        }
        scan_cfg_attr_tokens(group.stream(), lines);
    }
}
