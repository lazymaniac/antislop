use super::*;
use quote::ToTokens;
pub(super) fn callable_token_start(
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
pub(super) fn meta_expr(m: &Meta) -> String {
    m.to_token_stream().to_string()
}
pub(super) fn token_has_ident(tokens: &proc_macro2::TokenStream, name: &str) -> bool {
    tokens.clone().into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(ident) => ident == name,
        proc_macro2::TokenTree::Group(group) => token_has_ident(&group.stream(), name),
        _ => false,
    })
}
pub(super) fn dependency_crates(root: &Path) -> BTreeSet<String> {
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
pub(super) fn imported_external_macros(
    file: &syn::File,
    crates: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for item in &file.items {
        if let Item::Use(item) = item {
            collect_external_use(&item.tree, &mut Vec::new(), crates, &mut found);
        }
    }
    found
}

fn collect_external_use(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    crates: &BTreeSet<String>,
    found: &mut BTreeSet<String>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_external_use(&path.tree, prefix, crates, found);
            prefix.pop();
        }
        syn::UseTree::Name(name) if prefix.first().is_some_and(|root| crates.contains(root)) => {
            found.insert(name.ident.to_string());
        }
        syn::UseTree::Rename(rename)
            if prefix.first().is_some_and(|root| crates.contains(root)) =>
        {
            found.insert(rename.rename.to_string());
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_external_use(item, prefix, crates, found);
            }
        }
        syn::UseTree::Name(_) | syn::UseTree::Rename(_) | syn::UseTree::Glob(_) => {}
    }
}
pub(super) fn implies_test(m: &Meta) -> Result<bool, String> {
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
pub(super) fn collect_atoms(m: &Meta, a: &mut BTreeSet<String>) -> Result<(), String> {
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
pub(super) fn eval(m: &Meta, e: &BTreeMap<String, bool>) -> Result<bool, String> {
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
