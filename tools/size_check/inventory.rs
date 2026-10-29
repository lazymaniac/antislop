use super::*;
pub(super) struct RefWalker {
    pub(super) dir: PathBuf,
    pub(super) path_dir: PathBuf,
    pub(super) test: bool,
    pub(super) edges: Vec<(PathBuf, bool)>,
}

impl RefWalker {
    fn visit_module_body(&mut self, item: &syn::ItemMod, test: bool) {
        if let Some((_, items)) = &item.content {
            self.dir.push(item.ident.to_string());
            self.path_dir.push(item.ident.to_string());
            for nested in items {
                self.visit_item(nested);
            }
            self.dir.pop();
            self.path_dir.pop();
        } else {
            self.collect_module_edges(item, test);
        }
    }

    fn collect_module_edges(&mut self, item: &syn::ItemMod, test: bool) {
        let paths = module_paths(
            &item.attrs,
            &self.path_dir,
            &self.dir,
            &item.ident.to_string(),
        );
        for path in paths
            .unwrap_or_default()
            .into_iter()
            .filter(|path| path.is_file())
        {
            self.edges.push((path, test));
        }
    }
}

impl<'ast> Visit<'ast> for RefWalker {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let previous = self.test;
        let test = previous || attrs_imply_test(&item.attrs).unwrap_or(false);
        self.test = test;
        self.visit_module_body(item, test);
        self.test = previous;
    }
}

pub(super) fn module_references(
    files: &mut Vec<PathBuf>,
    source_root: &Path,
) -> BTreeMap<PathBuf, (bool, bool)> {
    let source_root = normalize_path(source_root);
    let mut known: BTreeSet<_> = files.iter().map(|file| normalize_path(file)).collect();
    let mut pending: Vec<_> = known.iter().cloned().collect();
    let graph = collect_module_graph(&mut pending, &mut known);
    *files = known.into_iter().collect();
    let incoming: BTreeSet<_> = graph
        .values()
        .flat_map(|edges| edges.iter().map(|(child, _)| child.clone()))
        .collect();
    reachable_references(&graph, files, &source_root, &incoming)
}

pub(super) fn collect_module_graph(
    pending: &mut Vec<PathBuf>,
    known: &mut BTreeSet<PathBuf>,
) -> BTreeMap<PathBuf, Vec<(PathBuf, bool)>> {
    let mut graph = BTreeMap::new();
    while let Some(file) = pending.pop() {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let Ok(ast) = syn::parse_file(&text) else {
            continue;
        };
        let mut walker = RefWalker {
            dir: module_directory(&file),
            path_dir: file.parent().unwrap_or(Path::new(".")).to_path_buf(),
            test: false,
            edges: Vec::new(),
        };
        walker.test = attrs_imply_test(&ast.attrs).unwrap_or(false);
        for item in &ast.items {
            walker.visit_item(item);
        }
        let edges = normalize_edges(walker.edges, pending, known);
        graph.insert(file, edges);
    }
    graph
}

pub(super) fn normalize_edges(
    edges: Vec<(PathBuf, bool)>,
    pending: &mut Vec<PathBuf>,
    known: &mut BTreeSet<PathBuf>,
) -> Vec<(PathBuf, bool)> {
    edges
        .into_iter()
        .map(|(path, test)| {
            let path = normalize_path(&path);
            if path.is_file() && known.insert(path.clone()) {
                pending.push(path.clone());
            }
            (path, test)
        })
        .collect()
}

pub(super) fn reachable_references(
    graph: &BTreeMap<PathBuf, Vec<(PathBuf, bool)>>,
    files: &[PathBuf],
    source_root: &Path,
    incoming: &BTreeSet<PathBuf>,
) -> BTreeMap<PathBuf, (bool, bool)> {
    let roots = files
        .iter()
        .filter(|path| is_crate_root(path, source_root) || !incoming.contains(*path))
        .cloned()
        .map(|path| (path, false));
    let mut refs = BTreeMap::new();
    let mut visited = BTreeSet::new();
    mark_reachable(graph, roots, &mut refs, &mut visited);
    let unvisited: Vec<_> = files
        .iter()
        .filter(|path| !refs.contains_key(*path))
        .cloned()
        .collect();
    for path in unvisited {
        mark_reachable(graph, [(path, false)], &mut refs, &mut visited);
    }
    refs
}

pub(super) fn is_crate_root(path: &Path, source_root: &Path) -> bool {
    path == source_root.join("lib.rs")
        || path == source_root.join("main.rs")
        || path
            .parent()
            .is_some_and(|parent| parent == source_root.join("bin"))
}

pub(super) fn mark_reachable(
    graph: &BTreeMap<PathBuf, Vec<(PathBuf, bool)>>,
    roots: impl IntoIterator<Item = (PathBuf, bool)>,
    refs: &mut BTreeMap<PathBuf, (bool, bool)>,
    visited: &mut BTreeSet<(PathBuf, bool)>,
) {
    let mut pending: Vec<_> = roots.into_iter().collect();
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

pub(super) fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if normalized.file_name().is_some() {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}
pub(super) fn module_directory(file: &Path) -> PathBuf {
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
pub(super) fn module_paths(
    attrs: &[Attribute],
    path_dir: &Path,
    module_dir: &Path,
    name: &str,
) -> Result<Vec<PathBuf>, String> {
    let overrides = path_overrides(attrs)?;
    if overrides.is_empty() {
        return Ok(default_module_paths(module_dir, name));
    }
    Ok(overrides
        .into_iter()
        .map(|path| path_dir.join(path))
        .collect())
}

fn path_overrides(attrs: &[Attribute]) -> Result<Vec<String>, String> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("path"))
        .map(path_override)
        .collect()
}

fn path_override(attr: &Attribute) -> Result<String, String> {
    let value = attr
        .meta
        .require_name_value()
        .map_err(|_| "expected #[path = \"...\"]".to_owned())?;
    let syn::Expr::Lit(literal) = &value.value else {
        return Err("expected a string literal path".into());
    };
    let syn::Lit::Str(path) = &literal.lit else {
        return Err("expected a string literal path".into());
    };
    Ok(path.value())
}

fn default_module_paths(module_dir: &Path, name: &str) -> Vec<PathBuf> {
    vec![
        module_dir.join(format!("{name}.rs")),
        module_dir.join(name).join("mod.rs"),
    ]
}
