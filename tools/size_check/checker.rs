//! Repository size audit: enumerate `src/**/*.rs`, then follow every parsed
//! module and `#[path]` edge, including the diagnostic binary's maintained
//! checker modules outside `src`. Only test-only and generated paths are
//! excluded, and every decision is printed as `SOURCE` or `EXCLUDED`.
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
const FILE_LIMIT: usize = 300;
const CALLABLE_LIMIT: usize = 30;
#[path = "classify.rs"]
mod classify;
#[path = "inventory.rs"]
mod inventory;
#[path = "runner.rs"]
mod runner;
#[path = "scan.rs"]
mod scan;
#[path = "source.rs"]
mod source;
#[path = "walker_core.rs"]
mod walker_core;
#[path = "walker_macros.rs"]
mod walker_macros;
#[path = "walker_modules.rs"]
mod walker_modules;
#[path = "walker_visit.rs"]
mod walker_visit;
use classify::*;
use inventory::*;
use scan::*;
use source::*;
use walker_core::*;
#[derive(Default)]
struct Results {
    errors: Vec<String>,
    violations: Vec<String>,
}
pub fn run() {
    runner::run();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn production_lines(text: &str) -> usize {
        let source = Source::new(PathBuf::from("fixture.rs"), PathBuf::new(), text.into());
        source.production_line_count(&syn::parse_file(text).unwrap())
    }

    #[test]
    fn excludes_embedded_test_modules_blocks_and_match_arms() {
        let text = "fn before() {}\n\n// production comment\n#[cfg(test)]\nmod tests {\n\nfn long() {\n1;\n2;\n}\n}\nfn after() {\n#[cfg(test)]\n{\n1;\n2;\n}\nmatch 1 {\n#[cfg(test)]\n0 => {\n1;\n2\n},\n_ => {}\n}\n}\n";
        assert_eq!(production_lines(text), 8);
    }

    #[test]
    fn excludes_cfg_test_associated_items_inside_production_containers() {
        let text = "trait Contract {\n    fn production(&self);\n    #[cfg(test)]\n    fn test_contract(&self) {\n        // test comment\n        let _ = 1;\n    }\n}\nstruct Service;\nimpl Service {\n    fn production(&self) {\n        // production comment\n        let _ = 1;\n    }\n\n    #[cfg(test)]\n    fn test_method(&self) {\n        let _ = 1;\n        let _ = 2;\n    }\n}\n";
        let all_lines = text.lines().count();
        let excluded_lines = "    #[cfg(test)]\n    fn test_contract(&self) {\n        // test comment\n        let _ = 1;\n    }\n".lines().count()
            + "    #[cfg(test)]\n    fn test_method(&self) {\n        let _ = 1;\n        let _ = 2;\n    }\n".lines().count();
        assert_eq!(production_lines(text), all_lines - excluded_lines);
    }

    #[test]
    fn file_limit_counts_surrounding_production_lines_and_blanks() {
        let allowed = format!(
            "{}\n#[cfg(test)]\nmod tests {{\n{}\n}}\n",
            "fn f() {}\n".repeat(299),
            "fn test() {}\n".repeat(500)
        );
        assert_eq!(production_lines(&allowed), 300); // 299 functions plus a production blank line.
        let violation = format!(
            "{}\n#[cfg(test)]\nmod tests {{\n{}\n}}\n",
            "fn f() {}\n".repeat(300),
            "fn test() {}\n".repeat(500)
        );
        assert_eq!(production_lines(&violation), 301);
        let excluded_only = format!(
            "{}#[cfg(test)]\nmod tests {{\n{}\n}}\n",
            "\n".repeat(298),
            "fn test() {}\n".repeat(500)
        );
        assert!(production_lines(&excluded_only) <= FILE_LIMIT);
    }

    #[test]
    fn classifies_generated_test_only_and_handwritten_vendor_sources() {
        let source_root = PathBuf::from("src");
        let references = BTreeMap::from([(PathBuf::from("src/test_support.rs"), (false, true))]);
        assert!(matches!(
            runner::source_class(
                Path::new("src/generated/output.rs"),
                &source_root,
                &references
            ),
            runner::SourceClass::Generated
        ));
        assert!(matches!(
            runner::source_class(Path::new("src/test_support.rs"), &source_root, &references),
            runner::SourceClass::TestOnly
        ));
        assert!(matches!(
            runner::source_class(
                Path::new("src/vendor/maintained.rs"),
                &source_root,
                &references
            ),
            runner::SourceClass::Production
        ));
    }

    #[test]
    fn discovers_path_modules_outside_src() {
        let root = tempfile::tempdir().unwrap();
        let source_root = root.path().join("src");
        let tool_root = root.path().join("tools");
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::create_dir_all(&tool_root).unwrap();
        std::fs::write(
            source_root.join("main.rs"),
            "#[path = \"../tools/entry.rs\"]\nmod entry;\n",
        )
        .unwrap();
        std::fs::write(tool_root.join("entry.rs"), "pub fn run() {}\n").unwrap();

        let (files, _) = runner::inventory(root.path(), &mut Vec::new());

        assert!(files.contains(&tool_root.join("entry.rs")));
    }

    #[test]
    fn callable_limit_starts_at_signature_and_counts_through_brace() {
        let allowed = format!(
            "// preceding comment\nfn f() {{\n{}\n}}\n",
            "    let _ = ();\n".repeat(27)
        );
        assert!(callable_violations(&allowed).is_empty());
        let too_long = format!(
            "// preceding comment\nfn f() {{\n{}\n}}\n",
            "    let _ = ();\n".repeat(28)
        );
        assert_eq!(callable_violations(&too_long).len(), 1);
    }

    fn callable_violations(text: &str) -> Vec<String> {
        let source = Source::new(PathBuf::from("fixture.rs"), PathBuf::new(), text.into());
        let mut results = Results::default();
        scan::scan_source(source, true, &BTreeSet::new(), &mut results);
        results.violations
    }
}
