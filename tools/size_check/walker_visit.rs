use super::*;
impl<'ast> Visit<'ast> for Walker<'_> {
    fn visit_macro(&mut self, call: &'ast syn::Macro) {
        if !self.production || self.test_context {
            visit::visit_macro(self, call);
            return;
        }
        self.cfg_attr_token_errors(&call.tokens);
        let name = call
            .path
            .segments
            .last()
            .map(|part| part.ident.to_string())
            .unwrap_or_default();
        if matches!(name.as_str(), "include" | "include_str" | "include_bytes") {
            self.inspect_include(call, &name);
        }
        self.inspect_expression_macro(call, &name);
        visit::visit_macro(self, call);
    }
    fn visit_attribute(&mut self, a: &'ast Attribute) {
        if a.path().is_ident("cfg_attr") {
            self.cfg_attr_errors(a);
        }
        visit::visit_attribute(self, a);
    }
    fn visit_item(&mut self, item: &'ast Item) {
        if !self.production || self.test_context {
            return visit::visit_item(self, item);
        }
        if let Item::Macro(call) = item
            && self.visit_macro_definition(call)
        {
            return;
        }
        visit::visit_item(self, item)
    }
    fn visit_stmt_macro(&mut self, m: &'ast syn::StmtMacro) {
        // This is statement/expression context, not an item-position invocation.
        // `visit_macro` inspects its tokens for callable-producing syntax.
        visit::visit_stmt_macro(self, m);
    }
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let was_test = self.test_context;
        self.test_context |= self.attrs(&item.attrs);
        self.visit_inline_module(item);
        self.test_context = was_test;
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
