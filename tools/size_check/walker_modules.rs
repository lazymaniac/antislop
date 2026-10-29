use super::*;
impl Walker<'_> {
    pub(super) fn visit_inline_module(&mut self, item: &syn::ItemMod) {
        if let Some((_, items)) = &item.content {
            self.module_dir.push(item.ident.to_string());
            self.path_dir.push(item.ident.to_string());
            for nested in items {
                self.visit_item(nested);
            }
            self.module_dir.pop();
            self.path_dir.pop();
        } else {
            self.visit_external_module(item);
        }
    }

    pub(super) fn visit_external_module(&mut self, item: &syn::ItemMod) {
        let explicit = item.attrs.iter().any(|attr| attr.path().is_ident("path"));
        let paths = module_paths(
            &item.attrs,
            &self.path_dir,
            &self.module_dir,
            &item.ident.to_string(),
        );
        let paths = match paths {
            Ok(paths) => paths,
            Err(error) => {
                self.result.errors.push(format!(
                    "{}:{}: cannot classify module path: {error}",
                    self.src.path.display(),
                    item.span().start().line
                ));
                return;
            }
        };
        self.report_module_candidates(item, explicit, paths);
    }

    pub(super) fn report_module_candidates(
        &mut self,
        item: &syn::ItemMod,
        explicit: bool,
        paths: Vec<PathBuf>,
    ) {
        let existing = paths.iter().filter(|candidate| candidate.is_file()).count();
        if !explicit {
            self.report_default_module(item, existing);
        } else {
            self.report_missing_explicit_modules(item, paths);
        }
    }

    fn report_default_module(&mut self, item: &syn::ItemMod, existing: usize) {
        let issue = match existing {
            0 => Some(format!("unresolved module {}", item.ident)),
            1 => None,
            _ => Some(format!(
                "ambiguous module {}: both default module paths exist",
                item.ident
            )),
        };
        if let Some(issue) = issue {
            self.result.errors.push(format!(
                "{}:{}: {issue}",
                self.src.path.display(),
                item.span().start().line
            ));
        }
    }

    fn report_missing_explicit_modules(&mut self, item: &syn::ItemMod, paths: Vec<PathBuf>) {
        for path in paths.into_iter().filter(|candidate| !candidate.is_file()) {
            self.result.errors.push(format!(
                "{}:{}: unresolved module file {}",
                self.src.path.display(),
                item.span().start().line,
                path.display()
            ));
        }
    }
}
