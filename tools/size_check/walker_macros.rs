use super::*;
use quote::ToTokens;
impl Walker<'_> {
    pub(super) fn inspect_include(&mut self, call: &syn::Macro, name: &str) {
        let parsed = syn::parse2::<syn::LitStr>(call.tokens.clone());
        match parsed {
            Ok(literal) => self.report_include_path(call, name, &literal.value()),
            Err(error) => self.result.errors.push(format!(
                "{}:{}: unresolved {name}! path: {error}",
                self.src.path.display(),
                call.span().start().line
            )),
        }
    }

    pub(super) fn report_include_path(&mut self, call: &syn::Macro, name: &str, value: &str) {
        let path = self
            .src
            .fs_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(value);
        if !path.is_file() {
            self.result.errors.push(format!(
                "{}:{}: unresolved {}! include {}",
                self.src.path.display(),
                call.span().start().line,
                name,
                path.display()
            ));
        } else if name == "include" {
            self.result.errors.push(format!(
                "{}:{}: unsupported maintained include! expansion",
                self.src.path.display(),
                call.span().start().line
            ));
        }
    }

    pub(super) fn inspect_expression_macro(&mut self, call: &syn::Macro, name: &str) {
        let first = call
            .path
            .segments
            .first()
            .map(|part| part.ident.to_string())
            .unwrap_or_default();
        if self.external_crates.contains(&first)
            || self.external_macros.contains(name)
            || matches!(first.as_str(), "std" | "core" | "alloc")
            || matches!(
                name,
                "stringify" | "concat" | "include_str" | "include_bytes"
            )
        {
            return;
        }
        let has_callable = syn::parse2::<syn::Block>(call.tokens.clone())
            .map(|block| block.stmts.iter().any(stmt_has_callable))
            .unwrap_or_else(|_| token_has_ident(&call.tokens, "fn"));
        if has_callable {
            self.result.errors.push(format!(
                "{}:{}: unsupported callable-producing macro form {}!",
                self.src.path.display(),
                call.span().start().line,
                call.path.to_token_stream()
            ));
        }
    }

    pub(super) fn visit_macro_definition(&mut self, item: &syn::ItemMacro) -> bool {
        if item.ident.is_some() && item.mac.path.is_ident("macro_rules") {
            self.inspect_macro_rules(item);
            return true;
        }
        self.report_item_macro(item);
        false
    }

    fn inspect_macro_rules(&mut self, item: &syn::ItemMacro) {
        for attr in &item.attrs {
            self.visit_attribute(attr);
        }
        self.cfg_attr_token_errors(&item.mac.tokens);
        if token_has_ident(&item.mac.tokens, "fn") {
            self.result.errors.push(format!(
                "{}:{}: unsupported callable-producing macro definition",
                self.src.path.display(),
                item.span().start().line
            ));
        }
    }

    fn report_item_macro(&mut self, item: &syn::ItemMacro) {
        let first = item
            .mac
            .path
            .segments
            .first()
            .map(|part| part.ident.to_string())
            .unwrap_or_default();
        let name = item
            .mac
            .path
            .segments
            .last()
            .map(|part| part.ident.to_string())
            .unwrap_or_default();
        let external = self.external_crates.contains(&first)
            || self.external_macros.contains(&name)
            || matches!(first.as_str(), "std" | "core" | "alloc");
        if !external {
            self.result.errors.push(format!(
                "{}:{}: unresolved item-position macro {}!",
                self.src.path.display(),
                item.span().start().line,
                item.mac.path.to_token_stream()
            ));
        }
    }
}
