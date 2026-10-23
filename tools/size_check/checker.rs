use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};
use syn::{
    Attribute, Item, Meta,
    spanned::Spanned,
    visit::{self, Visit},
};

const FILE_LIMIT: usize = 299;
const CALLABLE_LIMIT: usize = 29;

#[derive(Clone)]
struct Source {
    text: String,
    lines: Vec<usize>,
    path: PathBuf,
    fs_path: PathBuf,
}
impl Source {
    fn new(path: PathBuf, fs_path: PathBuf, text: String) -> Self {
        let lines = line_starts(&text);
        Self {
            text,
            lines,
            path,
            fs_path,
        }
    }
    fn line(&self, pos: usize) -> usize {
        self.lines.partition_point(|p| *p <= pos).max(1)
    }
    fn line_count(&self) -> usize {
        physical_lines(&self.text)
    }
    fn offset(&self, line: usize) -> usize {
        self.lines
            .get(line.saturating_sub(1))
            .copied()
            .unwrap_or(self.text.len())
    }
    fn callable_start(&self, ast_start: usize) -> usize {
        let ln = self.line(ast_start);
        let prefix_kinds = prefix_line_kinds(&self.text);
        let mut first = ln;
        // Attach contiguous comments/outer attributes, retaining intervening blanks.
        let mut cursor = ln.saturating_sub(1);
        while cursor > 0 {
            let prior = cursor;
            let line = self.text.lines().nth(prior - 1).unwrap_or("").trim();
            if line.is_empty() {
                if prior >= 2 && prefix_kinds[prior - 2] {
                    cursor -= 1;
                    continue;
                }
                break;
            }
            if prefix_kinds[prior - 1] {
                first = prior;
                cursor -= 1;
                continue;
            }
            break;
        }
        self.offset(first)
    }
    fn count_span(&self, start: usize, end: usize) -> usize {
        let s = self.line(start);
        let e = self.line(end.saturating_sub(1).max(start));
        e.saturating_sub(s) + 1
    }
}
fn prefix_line_kinds(source: &str) -> Vec<bool> {
    #[derive(Clone, Copy)]
    enum LexicalState {
        Normal,
        BlockComment { depth: usize, leading: bool },
        Quoted { quote: u8, escaped: bool },
        RawString { hashes: usize },
    }

    // Prefix recognition is lexical: comment-looking lines inside any string
    // literal are content, not comments. Keep enough state across physical
    // lines to distinguish raw strings, escaped quoted strings, and nested
    // block comments before classifying a line.
    let mut state = LexicalState::Normal;
    let mut kinds = Vec::new();
    for raw in source.lines() {
        let line = raw.trim();
        let bytes = raw.as_bytes();
        let starts_comment = match state {
            LexicalState::BlockComment { leading, .. } => leading,
            LexicalState::Normal => {
                (line.starts_with("//") && !line.starts_with("//!"))
                    || (line.starts_with("/*") && !line.starts_with("/*!"))
            }
            LexicalState::Quoted { .. } | LexicalState::RawString { .. } => false,
        };
        let attribute = matches!(state, LexicalState::Normal) && line.starts_with("#[");
        let mut i = 0;
        while i < bytes.len() {
            match state {
                LexicalState::Normal => {
                    if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                        break;
                    }
                    if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        let leading = bytes[..i].iter().all(u8::is_ascii_whitespace);
                        state = LexicalState::BlockComment { depth: 1, leading };
                        i += 2;
                    } else if let Some((quote, hashes, prefix_len)) = raw_string_start(bytes, i) {
                        debug_assert_eq!(quote, b'"');
                        state = LexicalState::RawString { hashes };
                        i += prefix_len;
                    } else if (bytes[i] == b'b' || bytes[i] == b'c')
                        && bytes.get(i + 1) == Some(&b'"')
                    {
                        state = LexicalState::Quoted {
                            quote: b'"',
                            escaped: false,
                        };
                        i += 2;
                    } else if bytes[i] == b'"' {
                        state = LexicalState::Quoted {
                            quote: b'"',
                            escaped: false,
                        };
                        i += 1;
                    } else if bytes[i] == b'\'' && looks_like_char_literal(bytes, i) {
                        state = LexicalState::Quoted {
                            quote: b'\'',
                            escaped: false,
                        };
                        i += 1;
                    } else {
                        i += 1;
                    }
                }
                LexicalState::BlockComment { depth, leading } => {
                    if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        state = LexicalState::BlockComment {
                            depth: depth + 1,
                            leading,
                        };
                        i += 2;
                    } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        state = if depth == 1 {
                            LexicalState::Normal
                        } else {
                            LexicalState::BlockComment {
                                depth: depth - 1,
                                leading,
                            }
                        };
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                LexicalState::Quoted { quote, escaped } => {
                    if escaped {
                        state = LexicalState::Quoted {
                            quote,
                            escaped: false,
                        };
                    } else if bytes[i] == b'\\' {
                        state = LexicalState::Quoted {
                            quote,
                            escaped: true,
                        };
                    } else if bytes[i] == quote {
                        state = LexicalState::Normal;
                    }
                    i += 1;
                }
                LexicalState::RawString { hashes } => {
                    if bytes[i] == b'"' && (0..hashes).all(|n| bytes.get(i + 1 + n) == Some(&b'#'))
                    {
                        i += hashes + 1;
                        state = LexicalState::Normal;
                    } else {
                        i += 1;
                    }
                }
            }
        }
        kinds.push(starts_comment || attribute);
    }
    kinds
}

fn raw_string_start(bytes: &[u8], start: usize) -> Option<(u8, usize, usize)> {
    let mut i = start;
    if bytes.get(i) == Some(&b'b') {
        i += 1;
    }
    if bytes.get(i) != Some(&b'r') {
        return None;
    }
    i += 1;
    let hashes_start = i;
    while bytes.get(i) == Some(&b'#') {
        i += 1;
    }
    (bytes.get(i) == Some(&b'"')).then_some((b'"', i - hashes_start, i - start + 1))
}

fn looks_like_char_literal(bytes: &[u8], start: usize) -> bool {
    let mut i = start + 1;
    if bytes.get(i) == Some(&b'\\') {
        i += 2;
    } else {
        i += 1;
    }
    bytes.get(i) == Some(&b'\'')
}
fn line_starts(s: &str) -> Vec<usize> {
    let mut v = vec![0];
    for (i, b) in s.bytes().enumerate() {
        if b == b'\n' && i + 1 < s.len() {
            v.push(i + 1);
        }
    }
    v
}
fn physical_lines(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.bytes().filter(|b| *b == b'\n').count() + usize::from(!s.ends_with('\n'))
    }
}
fn offset(span: proc_macro2::Span, src: &Source) -> usize {
    src.offset(span.start().line)
}

#[derive(Default)]
struct Results {
    errors: Vec<String>,
    violations: Vec<String>,
}
struct Walker<'a> {
    src: &'a Source,
    module_dir: PathBuf,
    path_dir: PathBuf,
    production: bool,
    external_crates: &'a BTreeSet<String>,
    external_macros: BTreeSet<String>,
    test_context: bool,
    result: &'a mut Results,
}
impl Walker<'_> {
    fn cfg_attr_errors(&mut self, attr: &Attribute) {
        fn nested(meta: &Meta, lines: &mut Vec<usize>) {
            if let Meta::List(list) = meta {
                if list.path.is_ident("cfg_attr") {
                    lines.push(meta.span().start().line);
                }
                if let Ok(items) = syn::parse::Parser::parse2(
                    syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
                    list.tokens.clone(),
                ) {
                    for item in items {
                        nested(&item, lines);
                    }
                }
            }
        }
        if !attr.path().is_ident("cfg_attr") {
            return;
        }
        let mut lines = vec![attr.span().start().line];
        if let Meta::List(list) = &attr.meta
            && let Ok(items) = syn::parse::Parser::parse2(
                syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
                list.tokens.clone(),
            )
        {
            for item in items {
                nested(&item, &mut lines);
            }
        }
        for line in lines {
            self.result.errors.push(format!(
                "{}:{line}: unsupported cfg_attr classification",
                self.src.path.display()
            ));
        }
    }
    fn attrs(&mut self, attrs: &[Attribute]) -> bool {
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
    fn cfg_attr_token_errors(&mut self, tokens: &proc_macro2::TokenStream) {
        fn nested(meta: &Meta, lines: &mut Vec<usize>) {
            if let Meta::List(list) = meta {
                if list.path.is_ident("cfg_attr") {
                    lines.push(meta.span().start().line);
                }
                if let Ok(items) = syn::parse::Parser::parse2(
                    syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
                    list.tokens.clone(),
                ) {
                    for item in items {
                        nested(&item, lines);
                    }
                }
            }
        }
        fn scan(tokens: proc_macro2::TokenStream, lines: &mut Vec<usize>) {
            use proc_macro2::{Delimiter, TokenTree};

            let trees: Vec<_> = tokens.into_iter().collect();
            for tree in &trees {
                let TokenTree::Group(group) = tree else {
                    continue;
                };
                if group.delimiter() == Delimiter::Bracket
                    && let Ok(meta) = syn::parse2::<Meta>(group.stream())
                    && matches!(&meta, Meta::List(list) if list.path.is_ident("cfg_attr"))
                {
                    lines.push(group.span().start().line);
                    nested(&meta, lines);
                }
                scan(group.stream(), lines);
            }
        }

        let mut lines = Vec::new();
        scan(tokens.clone(), &mut lines);
        for line in lines {
            self.result.errors.push(format!(
                "{}:{line}: unsupported cfg_attr classification",
                self.src.path.display()
            ));
        }
    }
    fn callable(
        &mut self,
        attrs: &[Attribute],
        ident: &str,
        start: proc_macro2::Span,
        end: proc_macro2::Span,
        test_attr: bool,
    ) {
        let own = self.attrs(attrs);
        if !self.production || self.test_context || own || test_attr {
            return;
        }
        let ast = offset(start, self.src);
        let attribute_start = attrs
            .iter()
            .map(|a| a.span().start().line)
            .min()
            .map(|line| self.src.offset(line))
            .unwrap_or(ast);
        let begin = self.src.callable_start(attribute_start);
        let finish = self.src.offset(end.end().line); // closing brace / semicolon line
        let count = self.src.count_span(begin, finish.saturating_add(1));
        if count > CALLABLE_LIMIT {
            self.result.violations.push(format!(
                "{}:{}: callable {ident} is {count} lines (limit {CALLABLE_LIMIT})",
                self.src.path.display(),
                self.src.line(begin)
            ));
        }
    }
}
fn attrs_imply_test(attrs: &[Attribute]) -> Result<bool, String> {
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
fn stmt_has_callable(stmt: &syn::Stmt) -> bool {
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
impl<'ast> Visit<'ast> for Walker<'_> {
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        self.cfg_attr_token_errors(&m.tokens);
        let name = m
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if name == "include" || name == "include_str" || name == "include_bytes" {
            match syn::parse2::<syn::LitStr>(m.tokens.clone()) {
                Ok(lit) => {
                    let path = self
                        .src
                        .fs_path
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(lit.value());
                    if !path.is_file() {
                        self.result.errors.push(format!(
                            "{}:{}: unresolved {}! include {}",
                            self.src.path.display(),
                            m.span().start().line,
                            name,
                            path.display()
                        ));
                    } else if name == "include" {
                        self.result.errors.push(format!(
                            "{}:{}: unsupported maintained include! expansion",
                            self.src.path.display(),
                            m.span().start().line
                        ));
                    }
                }
                Err(e) => self.result.errors.push(format!(
                    "{}:{}: unresolved {name}! path: {e}",
                    self.src.path.display(),
                    m.span().start().line
                )),
            }
        }
        // Expression-position macros are opaque to syn. Parse their token body as Rust
        // where possible so a local item hidden in a block is still counted. For opaque
        // maintained macros, a `fn` token is ambiguous and therefore fails closed.
        let first = m
            .path
            .segments
            .first()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if !self.external_crates.contains(&first)
            && !self.external_macros.contains(&name)
            && !matches!(first.as_str(), "std" | "core" | "alloc")
            && !matches!(
                name.as_str(),
                "stringify" | "concat" | "include_str" | "include_bytes"
            )
        {
            let has_callable = syn::parse2::<syn::Block>(m.tokens.clone())
                .map(|block| block.stmts.iter().any(stmt_has_callable))
                .unwrap_or_else(|_| token_has_ident(&m.tokens, "fn"));
            if has_callable {
                self.result.errors.push(format!(
                    "{}:{}: unsupported callable-producing macro form {}!",
                    self.src.path.display(),
                    m.span().start().line,
                    m.path.to_token_stream()
                ));
            }
        }
        visit::visit_macro(self, m);
    }
    fn visit_attribute(&mut self, a: &'ast Attribute) {
        if a.path().is_ident("cfg_attr") {
            self.cfg_attr_errors(a);
        }
        visit::visit_attribute(self, a);
    }
    fn visit_item(&mut self, i: &'ast Item) {
        // Detect item-position macro invocations; their generated callable set is unknowable.
        if let Item::Macro(m) = i {
            if m.ident.is_some() && m.mac.path.is_ident("macro_rules") {
                for attr in &m.attrs {
                    self.visit_attribute(attr);
                }
                self.cfg_attr_token_errors(&m.mac.tokens);
                if token_has_ident(&m.mac.tokens, "fn") {
                    self.result.errors.push(format!(
                        "{}:{}: unsupported callable-producing macro definition",
                        self.src.path.display(),
                        m.span().start().line
                    ));
                }
                return;
            }
            let first = m
                .mac
                .path
                .segments
                .first()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            let macro_name = m
                .mac
                .path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            if self.external_crates.contains(&first)
                || self.external_macros.contains(&macro_name)
                || matches!(first.as_str(), "std" | "core" | "alloc")
            {
                return visit::visit_item(self, i);
            }
            let line = m.span().start().line;
            self.result.errors.push(format!(
                "{}:{line}: unresolved item-position macro {}!",
                self.src.path.display(),
                m.mac.path.to_token_stream()
            ));
        }
        visit::visit_item(self, i)
    }
    fn visit_stmt_macro(&mut self, m: &'ast syn::StmtMacro) {
        // This is statement/expression context, not an item-position invocation.
        // `visit_macro` inspects its tokens for callable-producing syntax.
        visit::visit_stmt_macro(self, m);
    }
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        let own = self.attrs(&i.attrs);
        let old = self.test_context;
        self.test_context = old || own;
        let old_dir = self.module_dir.clone();
        let old_path_dir = self.path_dir.clone();
        if let Some((_, items)) = &i.content {
            self.module_dir = self.module_dir.join(i.ident.to_string());
            self.path_dir = self.path_dir.join(i.ident.to_string());
            for item in items {
                self.visit_item(item);
            }
        } else {
            let explicit = i.attrs.iter().any(|a| a.path().is_ident("path"));
            let paths = module_paths(
                &i.attrs,
                &self.path_dir,
                &self.module_dir,
                &i.ident.to_string(),
            );
            let paths = match paths {
                Ok(paths) => paths,
                Err(error) => {
                    self.result.errors.push(format!(
                        "{}:{}: cannot classify module path: {error}",
                        self.src.path.display(),
                        i.span().start().line
                    ));
                    self.module_dir = old_dir;
                    self.path_dir = old_path_dir;
                    self.test_context = old;
                    return;
                }
            };
            let existing = paths.iter().filter(|candidate| candidate.is_file()).count();
            if !explicit && existing > 1 {
                self.result.errors.push(format!(
                    "{}:{}: ambiguous module {}: both default module paths exist",
                    self.src.path.display(),
                    i.span().start().line,
                    i.ident
                ));
            } else if !explicit && existing == 0 {
                self.result.errors.push(format!(
                    "{}:{}: unresolved module {}",
                    self.src.path.display(),
                    i.span().start().line,
                    i.ident
                ));
            } else if explicit {
                for candidate in paths.into_iter().filter(|candidate| !candidate.is_file()) {
                    self.result.errors.push(format!(
                        "{}:{}: unresolved module file {}",
                        self.src.path.display(),
                        i.span().start().line,
                        candidate.display()
                    ));
                }
            }
        }
        self.module_dir = old_dir;
        self.path_dir = old_path_dir;
        self.test_context = old;
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        let old = self.test_context;
        self.test_context = old || self.attrs(&i.attrs);
        visit::visit_item_impl(self, i);
        self.test_context = old;
    }
    fn visit_item_trait(&mut self, i: &'ast syn::ItemTrait) {
        let old = self.test_context;
        self.test_context = old || self.attrs(&i.attrs);
        visit::visit_item_trait(self, i);
        self.test_context = old;
    }
    fn visit_item_foreign_mod(&mut self, i: &'ast syn::ItemForeignMod) {
        let old = self.test_context;
        self.test_context = old || self.attrs(&i.attrs);
        visit::visit_item_foreign_mod(self, i);
        self.test_context = old;
    }
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let name = i.sig.ident.to_string();
        self.callable(
            &i.attrs,
            &name,
            callable_token_start(&i.sig, Some(&i.vis)),
            i.block.brace_token.span.close(),
            false,
        );
        let old = self.test_context;
        self.test_context = old || self.attrs(&i.attrs);
        visit::visit_item_fn(self, i);
        self.test_context = old;
    }
    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        let name = i.sig.ident.to_string();
        self.callable(
            &i.attrs,
            &name,
            callable_token_start(&i.sig, Some(&i.vis)),
            i.block.brace_token.span.close(),
            false,
        );
        let old = self.test_context;
        self.test_context = old || self.attrs(&i.attrs);
        visit::visit_impl_item_fn(self, i);
        self.test_context = old;
    }
    fn visit_trait_item_fn(&mut self, i: &'ast syn::TraitItemFn) {
        let name = i.sig.ident.to_string();
        let end = i
            .default
            .as_ref()
            .map(|b| b.brace_token.span.close())
            .unwrap_or_else(|| {
                i.semi_token
                    .as_ref()
                    .expect("trait function has semicolon or body")
                    .span
            });
        self.callable(&i.attrs, &name, i.sig.span(), end, false);
        let old = self.test_context;
        self.test_context = old || self.attrs(&i.attrs);
        visit::visit_trait_item_fn(self, i);
        self.test_context = old;
    }
    fn visit_foreign_item_fn(&mut self, i: &'ast syn::ForeignItemFn) {
        let name = i.sig.ident.to_string();
        self.callable(&i.attrs, &name, i.sig.span(), i.semi_token.span, false);
        let old = self.test_context;
        self.test_context = old || self.attrs(&i.attrs);
        visit::visit_foreign_item_fn(self, i);
        self.test_context = old;
    }
}
use quote::ToTokens;
fn callable_token_start(
    sig: &syn::Signature,
    visibility: Option<&syn::Visibility>,
) -> proc_macro2::Span {
    let sig_start = sig.span();
    if let Some(visibility @ (syn::Visibility::Public(_) | syn::Visibility::Restricted(_))) =
        visibility
    {
        let visibility_span = visibility.span();
        if visibility_span.start().line < sig_start.start().line {
            return visibility_span;
        }
    }
    sig_start
}
fn meta_expr(m: &Meta) -> String {
    m.to_token_stream().to_string()
}
fn token_has_ident(tokens: &proc_macro2::TokenStream, name: &str) -> bool {
    tokens.clone().into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(ident) => ident == name,
        proc_macro2::TokenTree::Group(group) => token_has_ident(&group.stream(), name),
        _ => false,
    })
}
fn dependency_crates(root: &Path) -> BTreeSet<String> {
    let Ok(text) = fs::read_to_string(root.join("Cargo.toml")) else {
        return BTreeSet::new();
    };
    let Ok(manifest) = toml::from_str::<toml::Value>(&text) else {
        return BTreeSet::new();
    };
    let Some(table) = manifest.as_table() else {
        return BTreeSet::new();
    };
    let mut crates = BTreeSet::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(dependencies) = table.get(section).and_then(toml::Value::as_table) {
            crates.extend(dependencies.keys().map(|name| name.replace('-', "_")));
        }
    }
    crates
}
fn imported_external_macros(file: &syn::File, crates: &BTreeSet<String>) -> BTreeSet<String> {
    fn walk(
        tree: &syn::UseTree,
        prefix: &mut Vec<String>,
        crates: &BTreeSet<String>,
        out: &mut BTreeSet<String>,
    ) {
        match tree {
            syn::UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                walk(&path.tree, prefix, crates, out);
                prefix.pop();
            }
            syn::UseTree::Name(name) => {
                if prefix.first().is_some_and(|root| crates.contains(root)) {
                    out.insert(name.ident.to_string());
                }
            }
            syn::UseTree::Rename(rename) => {
                if prefix.first().is_some_and(|root| crates.contains(root)) {
                    out.insert(rename.rename.to_string());
                }
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    walk(item, prefix, crates, out);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
    let mut out = BTreeSet::new();
    for item in &file.items {
        if let Item::Use(item) = item {
            walk(&item.tree, &mut Vec::new(), crates, &mut out);
        }
    }
    out
}
fn implies_test(m: &Meta) -> Result<bool, String> {
    let mut atoms = BTreeSet::new();
    collect_atoms(m, &mut atoms)?;
    if atoms.len() > 18 {
        return Err("cfg expression has more than 18 symbolic atoms".into());
    }
    let list: Vec<_> = atoms.into_iter().collect();
    for mask in 0..(1usize << list.len()) {
        let env: BTreeMap<_, _> = list
            .iter()
            .enumerate()
            .map(|(i, k)| (k.clone(), mask & (1 << i) != 0))
            .collect();
        if eval(m, &env)? && !env.get("test").copied().unwrap_or(false) {
            return Ok(false);
        }
    }
    Ok(true)
}
fn collect_atoms(m: &Meta, a: &mut BTreeSet<String>) -> Result<(), String> {
    match m {
        Meta::List(l)
            if l.path.is_ident("all") || l.path.is_ident("any") || l.path.is_ident("not") =>
        {
            let nested = syn::parse::Parser::parse2(
                syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
                l.tokens.clone(),
            )
            .map_err(|e| e.to_string())?;
            for n in nested {
                collect_atoms(&n, a)?;
            }
        }
        _ => {
            a.insert(meta_expr(m));
        }
    }
    Ok(())
}
fn eval(m: &Meta, e: &BTreeMap<String, bool>) -> Result<bool, String> {
    match m {
        Meta::List(l)
            if l.path.is_ident("all") || l.path.is_ident("any") || l.path.is_ident("not") =>
        {
            let xs = syn::parse::Parser::parse2(
                syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
                l.tokens.clone(),
            )
            .map_err(|x| x.to_string())?;
            let vals = xs
                .iter()
                .map(|x| eval(x, e))
                .collect::<Result<Vec<_>, _>>()?;
            if l.path.is_ident("all") {
                Ok(vals.into_iter().all(|v| v))
            } else if l.path.is_ident("any") {
                Ok(vals.into_iter().any(|v| v))
            } else if vals.len() == 1 {
                Ok(!vals[0])
            } else {
                Err("not() requires exactly one predicate".into())
            }
        }
        _ => Ok(e.get(&meta_expr(m)).copied().unwrap_or(false)),
    }
}

fn scan_file(
    path: &Path,
    display_path: &Path,
    production: bool,
    external_crates: &BTreeSet<String>,
    results: &mut Results,
) {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            results
                .errors
                .push(format!("{}:0: read failure: {e}", display_path.display()));
            return;
        }
    };
    let src = Source::new(display_path.to_path_buf(), path.to_path_buf(), text);
    let count = src.line_count();
    if production && count > FILE_LIMIT {
        results.violations.push(format!(
            "{}:1: file is {count} lines (limit {FILE_LIMIT})",
            display_path.display()
        ));
    }
    let ast = match syn::parse_file(&src.text) {
        Ok(a) => a,
        Err(e) => {
            results.errors.push(format!(
                "{}:{}: parse error: {e}",
                path.display(),
                e.span().start().line
            ));
            return;
        }
    };
    let external_macros = imported_external_macros(&ast, external_crates);
    let mut w = Walker {
        src: &src,
        module_dir: module_directory(path),
        path_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
        production,
        external_crates,
        external_macros,
        test_context: false,
        result: results,
    };
    w.test_context = w.attrs(&ast.attrs);
    for i in &ast.items {
        w.visit_item(i);
    }
}
fn module_references(files: &[PathBuf], source_root: &Path) -> BTreeMap<PathBuf, (bool, bool)> {
    struct RefWalker {
        dir: PathBuf,
        path_dir: PathBuf,
        test: bool,
        edges: Vec<(PathBuf, bool)>,
    }
    impl<'ast> Visit<'ast> for RefWalker {
        fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
            let test = self.test || attrs_imply_test(&item.attrs).unwrap_or(false);
            let old = self.test;
            let old_dir = self.dir.clone();
            let old_path_dir = self.path_dir.clone();
            self.test = test;
            if let Some((_, items)) = &item.content {
                self.dir = self.dir.join(item.ident.to_string());
                self.path_dir = self.path_dir.join(item.ident.to_string());
                for item in items {
                    self.visit_item(item);
                }
            } else {
                let paths = module_paths(
                    &item.attrs,
                    &self.path_dir,
                    &self.dir,
                    &item.ident.to_string(),
                );
                // An invalid path creates no guessed edge. The source walker emits
                // the user-facing classification error at the attribute location.
                let paths = paths.unwrap_or_default();
                for path in paths.into_iter().filter(|p| p.is_file()) {
                    self.edges.push((path, test));
                }
            }
            self.test = old;
            self.dir = old_dir;
            self.path_dir = old_path_dir;
        }
    }
    let mut graph: BTreeMap<PathBuf, Vec<(PathBuf, bool)>> = BTreeMap::new();
    let mut incoming = BTreeSet::new();
    for file in files {
        let Ok(text) = fs::read_to_string(file) else {
            continue;
        };
        let Ok(ast) = syn::parse_file(&text) else {
            continue;
        };
        let mut walker = RefWalker {
            dir: module_directory(file),
            path_dir: file.parent().unwrap_or(Path::new(".")).to_path_buf(),
            test: false,
            edges: Vec::new(),
        };
        walker.test = attrs_imply_test(&ast.attrs).unwrap_or(false);
        for item in &ast.items {
            walker.visit_item(item);
        }
        for (path, _) in &walker.edges {
            incoming.insert(path.clone());
        }
        graph.insert(file.clone(), walker.edges);
    }
    let is_root = |p: &PathBuf| {
        p == &source_root.join("lib.rs")
            || p == &source_root.join("main.rs")
            || p.parent()
                .is_some_and(|parent| parent == source_root.join("bin"))
    };
    let mut refs = BTreeMap::new();
    let mut visited = BTreeSet::new();
    let mut pending: Vec<(PathBuf, bool)> = files
        .iter()
        .filter(|p| is_root(p) || !incoming.contains(*p))
        .cloned()
        .map(|p| (p, false))
        .collect();
    while !pending.is_empty() {
        let mut next = Vec::new();
        for (file, test_context) in pending.drain(..) {
            if !visited.insert((file.clone(), test_context)) {
                continue;
            }
            let flags = refs.entry(file.clone()).or_insert((false, false));
            if test_context {
                flags.1 = true;
            } else {
                flags.0 = true;
            }
            if let Some(edges) = graph.get(&file) {
                next.extend(
                    edges
                        .iter()
                        .map(|(child, test_edge)| (child.clone(), test_context || *test_edge)),
                );
            }
        }
        pending = next;
    }
    // An unreferenced cycle has no zero-indegree seed, but remains production by default.
    let unvisited: Vec<_> = files
        .iter()
        .filter(|p| !refs.contains_key(*p))
        .cloned()
        .collect();
    for file in unvisited {
        let mut pending = vec![(file, false)];
        while let Some((file, test_context)) = pending.pop() {
            if !visited.insert((file.clone(), test_context)) {
                continue;
            }
            let flags = refs.entry(file.clone()).or_insert((false, false));
            if test_context {
                flags.1 = true;
            } else {
                flags.0 = true;
            }
            if let Some(edges) = graph.get(&file) {
                pending.extend(
                    edges
                        .iter()
                        .map(|(child, test_edge)| (child.clone(), test_context || *test_edge)),
                );
            }
        }
    }
    refs
}
fn module_directory(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or(Path::new("."));
    let stem = file.file_stem().unwrap_or_default();
    if file.file_name().is_some_and(|n| n == "mod.rs")
        || stem == "lib"
        || stem == "main"
        || (parent.file_name().is_some_and(|n| n == "bin")
            && parent
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "src"))
    {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    }
}
fn module_paths(
    attrs: &[Attribute],
    path_dir: &Path,
    module_dir: &Path,
    name: &str,
) -> Result<Vec<PathBuf>, String> {
    let mut overrides = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("path")) {
        let value = attr
            .meta
            .require_name_value()
            .map_err(|_| "expected #[path = \"...\"]".to_owned())?;
        let syn::Expr::Lit(lit) = &value.value else {
            return Err("expected a string literal path".into());
        };
        let syn::Lit::Str(path) = &lit.lit else {
            return Err("expected a string literal path".into());
        };
        overrides.push(path.value());
    }
    if overrides.is_empty() {
        Ok(vec![
            module_dir.join(format!("{name}.rs")),
            module_dir.join(name).join("mod.rs"),
        ])
    } else {
        Ok(overrides
            .into_iter()
            .map(|path| path_dir.join(path))
            .collect())
    }
}
pub fn run() {
    let mut root = PathBuf::from(".");
    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--root" {
            let Some(x) = args.next() else {
                eprintln!("--root requires a directory argument");
                std::process::exit(2)
            };
            root = PathBuf::from(x);
        } else {
            eprintln!("unknown argument: {a}");
            std::process::exit(2)
        }
    }
    let src = root.join("src");
    let external_crates = dependency_crates(&root);
    let mut files = Vec::new();
    let mut r = Results::default();
    collect(&src, &mut files, &mut r.errors);
    files.sort();
    let refs = module_references(&files, &src);
    let mut maintained_files = 0usize;
    for p in &files {
        let root_file = p == &src.join("lib.rs")
            || p == &src.join("main.rs")
            || p.parent().is_some_and(|parent| parent == src.join("bin"));
        let (production_ref, test_ref) = refs.get(p).copied().unwrap_or((false, false));
        let production = root_file || production_ref || !test_ref;
        maintained_files += usize::from(production);
        scan_file(
            p,
            p.strip_prefix(&root).unwrap_or(p),
            production,
            &external_crates,
            &mut r,
        )
    }
    r.errors.sort();
    r.errors.dedup();
    r.violations.sort();
    for e in &r.errors {
        eprintln!("ERROR {e}")
    }
    for v in &r.violations {
        println!("VIOLATION {v}")
    }
    if r.errors.is_empty() && r.violations.is_empty() {
        println!(
            "size diagnostic: {} maintained Rust files, no violations",
            maintained_files
        );
    } else {
        println!(
            "size diagnostic failed: {} classification errors, {} violations",
            r.errors.len(),
            r.violations.len()
        );
        std::process::exit(1)
    }
}
fn collect(d: &Path, out: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    let entries = match fs::read_dir(d) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!(
                "{}:0: source traversal failure: {error}",
                d.display()
            ));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!(
                    "{}:0: directory entry failure: {error}",
                    d.display()
                ));
                continue;
            }
        };
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out, errors);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}
