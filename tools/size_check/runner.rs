use super::*;

pub(super) fn run() {
    match parse_root(env::args().skip(1)) {
        Ok(root) => execute(root),
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    }
}

fn parse_root(args: impl Iterator<Item = String>) -> Result<PathBuf, String> {
    let mut root = PathBuf::from(".");
    let mut args = args;
    while let Some(arg) = args.next() {
        if arg != "--root" {
            return Err(format!("unknown argument: {arg}"));
        }
        root = PathBuf::from(args.next().ok_or("--root requires a directory argument")?);
    }
    Ok(root)
}

fn execute(root: PathBuf) {
    let mut results = Results::default();
    let (files, classifications) = inventory(&root, &mut results.errors);
    scan_sources(&root, &files, &classifications, &mut results);
    normalize_diagnostics(&mut results);
    report(&root, &files, &classifications, &results);
    if !results.errors.is_empty() || !results.violations.is_empty() {
        std::process::exit(1);
    }
}

#[derive(Clone, Copy)]
pub(super) enum SourceClass {
    Production,
    TestOnly,
    Generated,
}

impl SourceClass {
    fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }

    fn exclusion_reason(self) -> &'static str {
        match self {
            Self::Production => "production source",
            Self::TestOnly => "test-only source",
            Self::Generated => "generated source",
        }
    }
}

pub(super) fn inventory(
    root: &Path,
    errors: &mut Vec<String>,
) -> (Vec<PathBuf>, BTreeMap<PathBuf, SourceClass>) {
    let source_root = root.join("src");
    let mut files = Vec::new();
    collect(&source_root, &mut files, errors);
    files.sort();
    let references = module_references(&mut files, &source_root);
    let classifications = files
        .iter()
        .map(|path| {
            let class = source_class(path, &source_root, &references);
            (path.clone(), class)
        })
        .collect();
    (files, classifications)
}

pub(super) fn source_class(
    path: &Path,
    source_root: &Path,
    references: &BTreeMap<PathBuf, (bool, bool)>,
) -> SourceClass {
    if path
        .components()
        .any(|part| matches!(part.as_os_str().to_str(), Some("generated")))
    {
        return SourceClass::Generated;
    }
    let (production_ref, test_ref) = references.get(path).copied().unwrap_or((false, false));
    if is_crate_root(path, source_root) || production_ref || !test_ref {
        SourceClass::Production
    } else {
        SourceClass::TestOnly
    }
}

fn scan_sources(
    root: &Path,
    files: &[PathBuf],
    classifications: &BTreeMap<PathBuf, SourceClass>,
    results: &mut Results,
) {
    let external_crates = dependency_crates(root);
    for path in files {
        let display = path.strip_prefix(root).unwrap_or(path);
        let production = classifications
            .get(path)
            .is_none_or(|class| class.is_production());
        scan_file(path, display, production, &external_crates, results);
    }
}

fn normalize_diagnostics(results: &mut Results) {
    results.errors.sort();
    results.errors.dedup();
    results.violations.sort();
}

fn report(
    root: &Path,
    files: &[PathBuf],
    classifications: &BTreeMap<PathBuf, SourceClass>,
    results: &Results,
) {
    report_inventory(root, files, classifications);
    report_diagnostics(results);
    report_summary(classifications, results);
}

fn report_inventory(
    root: &Path,
    files: &[PathBuf],
    classifications: &BTreeMap<PathBuf, SourceClass>,
) {
    for path in files {
        let display = path.strip_prefix(root).unwrap_or(path);
        let class = classifications
            .get(path)
            .copied()
            .unwrap_or(SourceClass::Production);
        if class.is_production() {
            println!("SOURCE {}", display.display());
        } else {
            println!(
                "EXCLUDED {}: {}",
                display.display(),
                class.exclusion_reason()
            );
        }
    }
}

fn report_diagnostics(results: &Results) {
    for error in &results.errors {
        eprintln!("ERROR {error}");
    }
    for violation in &results.violations {
        println!("VIOLATION {violation}");
    }
}

fn report_summary(classifications: &BTreeMap<PathBuf, SourceClass>, results: &Results) {
    if results.errors.is_empty() && results.violations.is_empty() {
        let count = classifications
            .values()
            .filter(|class| class.is_production())
            .count();
        println!("size diagnostic: {count} maintained Rust files, no violations");
    } else {
        println!(
            "size diagnostic failed: {} classification errors, {} violations",
            results.errors.len(),
            results.violations.len()
        );
    }
}

fn collect(directory: &Path, output: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!(
                "{}:0: source traversal failure: {error}",
                directory.display()
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
                    directory.display()
                ));
                continue;
            }
        };
        let path = entry.path();
        if path.is_dir() {
            collect(&path, output, errors);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            output.push(path);
        }
    }
}
