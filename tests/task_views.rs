use architect::workspace::views::{Repo, git, merge_file};
use std::path::Path;

fn sh(dir: &Path, args: &[&str]) {
    let output = git(dir, args, &[], None).unwrap();
    assert_eq!(output.status, 0, "{args:?}: {}", output.stderr);
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    sh(root, &["init", "--quiet"]);
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(root.join("b.txt"), "bee\n").unwrap();
    sh(root, &["add", "."]);
    sh(root, &["commit", "--quiet", "-m", "init"]);
    // Dirty tracked edit, untracked file, ignored output and runtime state.
    std::fs::write(root.join("b.txt"), "bee local\n").unwrap();
    std::fs::write(root.join("new.txt"), "fresh\n").unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("target/out"), "binary").unwrap();
    std::fs::create_dir_all(root.join(".antislop")).unwrap();
    std::fs::write(root.join(".antislop/db"), "state").unwrap();
    dir
}

#[test]
fn capture_records_the_current_checkout_without_ignored_or_runtime_files() {
    let project = project();
    let data = tempfile::tempdir().unwrap();
    let repo = Repo::open_or_init(&data.path().join("repo.git")).unwrap();
    let base = repo.capture(project.path(), None, "base").unwrap();
    assert_eq!(repo.show(&base, "b.txt").unwrap().unwrap(), b"bee local\n");
    assert_eq!(repo.show(&base, "new.txt").unwrap().unwrap(), b"fresh\n");
    assert!(repo.show(&base, "target/out").unwrap().is_none());
    assert!(repo.show(&base, ".antislop/db").unwrap().is_none());
    // The user's repository is not touched.
    let status = git(project.path(), &["status", "--porcelain"], &[], None).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&status.stdout),
        " M b.txt\n?? .antislop/\n?? new.txt\n"
    );
    // Reopening keeps the same repository.
    let again = Repo::open_or_init(&data.path().join("repo.git")).unwrap();
    assert_eq!(
        again.show(&base, "a.txt").unwrap().unwrap(),
        b"one\ntwo\nthree\n"
    );
}

#[test]
fn writer_branches_stack_and_combine_dependency_results() {
    let project = project();
    let data = tempfile::tempdir().unwrap();
    let repo = Repo::open_or_init(&data.path().join("repo.git")).unwrap();
    let base = repo.capture(project.path(), None, "base").unwrap();

    let first = data.path().join("tasks/first");
    repo.checkout(&first, &base, Some("task/first")).unwrap();
    std::fs::write(first.join("a.txt"), "ONE\ntwo\nthree\n").unwrap();
    let r1 = repo.commit(&first, "first").unwrap();
    assert_ne!(r1, base);
    assert!(repo.clean(&first).unwrap());
    // An unchanged checkout keeps its commit.
    assert_eq!(repo.commit(&first, "noop").unwrap(), r1);

    let second = data.path().join("tasks/second");
    repo.checkout(&second, &base, Some("task/second")).unwrap();
    std::fs::write(second.join("a.txt"), "one\ntwo\nTHREE\n").unwrap();
    std::fs::remove_file(second.join("new.txt")).unwrap();
    let r2 = repo.commit(&second, "second").unwrap();

    assert_eq!(
        repo.changed(&base, &r2).unwrap(),
        vec![('M', "a.txt".to_owned()), ('D', "new.txt".to_owned())]
    );
    assert!(repo.diff(&base, &r1).unwrap().contains("+ONE"));

    let merged = repo.combine(&[r1.clone(), r2.clone()]).unwrap().unwrap();
    assert_eq!(
        repo.show(&merged, "a.txt").unwrap().unwrap(),
        b"ONE\ntwo\nTHREE\n"
    );
    assert!(repo.show(&merged, "new.txt").unwrap().is_none());
    assert!(repo.is_ancestor(&r1, &merged).unwrap());
    // A descendant subsumes its ancestor without a new commit.
    assert_eq!(
        repo.combine(&[base.clone(), r1.clone()]).unwrap().unwrap(),
        r1
    );

    // A reader view of the merged result.
    let view = data.path().join("views/merged");
    repo.checkout(&view, &merged, None).unwrap();
    assert_eq!(repo.head(&view).unwrap(), merged);
    std::fs::write(view.join("a.txt"), "scribble").unwrap();
    assert!(!repo.clean(&view).unwrap());
    repo.reset(&view, &merged).unwrap();
    assert!(repo.clean(&view).unwrap());
    repo.remove(&view).unwrap();
    assert!(!view.exists());
}

#[test]
fn overlapping_results_report_conflicting_paths() {
    let project = project();
    let data = tempfile::tempdir().unwrap();
    let repo = Repo::open_or_init(&data.path().join("repo.git")).unwrap();
    let base = repo.capture(project.path(), None, "base").unwrap();
    let mut results = vec![];
    for (name, text) in [("x", "X\n"), ("y", "Y\n")] {
        let path = data.path().join(name);
        repo.checkout(&path, &base, Some(&format!("task/{name}")))
            .unwrap();
        std::fs::write(path.join("b.txt"), text).unwrap();
        results.push(repo.commit(&path, name).unwrap());
    }
    assert_eq!(
        repo.combine(&results).unwrap().unwrap_err(),
        vec!["b.txt".to_owned()]
    );
}

#[test]
fn file_merges_are_three_way() {
    let scratch = tempfile::tempdir().unwrap();
    let base = b"one\ntwo\nthree\n".as_slice();
    let (merged, conflict) = merge_file(
        scratch.path(),
        Some(base),
        Some(b"ONE\ntwo\nthree\n"),
        Some(b"one\ntwo\nTHREE\n"),
        ("project", "task"),
    )
    .unwrap();
    assert!(!conflict);
    assert_eq!(merged.unwrap(), b"ONE\ntwo\nTHREE\n");

    let (merged, conflict) = merge_file(
        scratch.path(),
        Some(base),
        Some(b"one\nTWO\nthree\n"),
        Some(b"one\n2\nthree\n"),
        ("project", "task"),
    )
    .unwrap();
    assert!(conflict);
    let text = String::from_utf8(merged.unwrap()).unwrap();
    assert!(text.contains("<<<<<<< project") && text.contains(">>>>>>> task"));

    // Unchanged on one side takes the other, including deletion.
    assert_eq!(
        merge_file(scratch.path(), Some(base), Some(base), None, ("p", "t")).unwrap(),
        (None, false)
    );
    // Deleted on one side but changed on the other is a conflict.
    assert!(
        merge_file(
            scratch.path(),
            Some(base),
            None,
            Some(b"changed\n"),
            ("p", "t")
        )
        .unwrap()
        .1
    );
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
}
