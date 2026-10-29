//! Repository size audit: enumerate `src/**/*.rs`, then follow every parsed
//! module and `#[path]` edge, including the diagnostic binary's maintained
//! checker modules outside `src`. Only test-only and generated/vendor paths
//! are excluded, and every decision is printed as `SOURCE` or `EXCLUDED`.
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
