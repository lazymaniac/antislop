use super::*;
pub(super) fn scan_file(
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
    scan_source(src, production, external_crates, results);
}

fn report_file_size(src: &Source, ast: &syn::File, production: bool, results: &mut Results) {
    let count = src.production_line_count(ast);
    if production && count > FILE_LIMIT {
        results.violations.push(format!(
            "{}:1: file is {count} lines (limit {FILE_LIMIT})",
            src.path.display()
        ));
    }
}

pub(super) fn scan_source(
    src: Source,
    production: bool,
    external_crates: &BTreeSet<String>,
    results: &mut Results,
) {
    let ast = match syn::parse_file(&src.text) {
        Ok(ast) => ast,
        Err(error) => {
            results.errors.push(format!(
                "{}:{}: parse error: {error}",
                src.path.display(),
                error.span().start().line
            ));
            return;
        }
    };
    report_file_size(&src, &ast, production, results);
    walk_source(&src, &ast, production, external_crates, results);
}

fn walk_source(
    src: &Source,
    ast: &syn::File,
    production: bool,
    external_crates: &BTreeSet<String>,
    results: &mut Results,
) {
    let external_macros = imported_external_macros(ast, external_crates);
    let mut w = Walker {
        src,
        module_dir: module_directory(&src.fs_path),
        path_dir: src.fs_path.parent().unwrap_or(Path::new(".")).to_path_buf(),
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
