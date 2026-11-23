#[allow(dead_code)]
mod fixture_paths {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/external.rs"));
}

/// Makes a headless-style test script also answer as an interactive agent
/// CLI. Started in a terminal (as the daemon does) the script runs
/// `pty_shim.py`, which takes the pasted prompt, runs the script itself once
/// per turn the old way and ends the turn through the MCP gateway; started any
/// other way it is unchanged. Edits to the script keep working because the
/// script stays at its path and keeps its text.
pub fn interactive(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    fixture_paths::external(path.parent().unwrap());
    let source = std::fs::read_to_string(path).unwrap();
    if !source.contains("FIXTURE_PTY") {
        let (shebang, rest) = source.split_once('\n').unwrap();
        let header = if shebang.contains("python") {
            "import os as _fo, sys as _fs\n\
             if not _fo.environ.get('FIXTURE_HEADLESS') and _fo.isatty(0):  # FIXTURE_PTY\n    \
             _fo.execvp('python3', ['python3', __file__ + '.shim.py', __file__] + _fs.argv[1:])\n"
        } else {
            "if [ -z \"$FIXTURE_HEADLESS\" ] && [ -t 0 ]; then exec python3 \"$0.shim.py\" \"$0\" \"$@\"; fi # FIXTURE_PTY\n"
        };
        std::fs::write(path, format!("{shebang}\n{header}{rest}")).unwrap();
    }
    let mut shim = path.as_os_str().to_owned();
    shim.push(".shim.py");
    std::fs::write(&shim, include_str!("pty_shim.py")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
