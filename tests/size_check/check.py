#!/usr/bin/env python3
"""Black-box fixtures for the maintained-source size diagnostic."""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
ARTIFACTS = ROOT / ".antislop"
BINARY = ROOT / ".antislop" / "target" / "debug" / "diagnostic-size-check"
RESULTS = []


def invoke(files, relative_root=False, parent_relative_root=False):
    with tempfile.TemporaryDirectory(prefix="size-check-", dir=ARTIFACTS) as temp:
        root = Path(temp)
        assert root.resolve().parent == ARTIFACTS.resolve(), root
        for name, body in files.items():
            if name == "Cargo.toml":
                target = root / name
            elif name.startswith("@root/"):
                target = root / name.removeprefix("@root/")
            else:
                target = root / "src" / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(body, newline="")
        if parent_relative_root:
            (root / "src" / "bin").mkdir(parents=True, exist_ok=True)
            root_argument = "../.."
            cwd = root / "src" / "bin"
        else:
            root_argument = f"./{root.relative_to(ROOT)}" if relative_root else root
            cwd = ROOT
        result = subprocess.run(
            [str(BINARY), "--root", str(root_argument)],
            cwd=cwd,
            text=True,
            capture_output=True,
        )
    assert not root.exists(), f"fixture tree was not cleaned up: {root}"
    RESULTS.append((
        "fixture-artifacts",
        0,
        "fixture tree contained in .antislop and removed; no external fixture tree",
        str(root.relative_to(ROOT)),
    ))
    return result


def ok(files, needle="", relative_root=False):
    result = invoke(files, relative_root=relative_root)
    assert result.returncode == 0, result.stderr + result.stdout
    assert needle in result.stdout, result.stdout
    RESULTS.append(("pass", result.returncode, needle, (result.stderr + result.stdout).strip()))


def bad(files, needle, line=None, path="src/lib.rs", extra_needles=(), relative_root=False, parent_relative_root=False):
    result = invoke(files, relative_root=relative_root, parent_relative_root=parent_relative_root)
    assert result.returncode != 0, result.stdout
    output = result.stderr + result.stdout
    assert needle in output, output
    for extra in extra_needles:
        assert extra in output, output
    if line is not None:
        assert f"{path}:{line}:" in output, output
    RESULTS.append(("fail", result.returncode, needle, output.strip()))


def main():
    subprocess.run(
        [
            "cargo",
            "build",
            "--locked",
            "--quiet",
            "--target-dir",
            ".antislop/target",
            "--bin",
            "diagnostic-size-check",
        ],
        cwd=ROOT,
        check=True,
    )

    # Physical line boundaries, LF/CRLF, and final newline behavior.
    ok({"lib.rs": "\n".join(["// line"] * 300) + "\n"}, "no violations")
    ok({"lib.rs": "\n".join(["// line"] * 300)}, "no violations")
    bad({"lib.rs": "\n".join(["// line"] * 301)}, "file is 301 lines")
    ok({"lib.rs": "\r\n".join(["// line"] * 300) + "\r\n"}, "no violations")
    bad({"lib.rs": "\r\n".join(["// line"] * 301) + "\r\n"}, "file is 301 lines")

    # Exactly 30 versus 31 physical lines in callable bodies.
    ok({"lib.rs": "fn small() {\n" + "\n".join("    let _ = 1;" for _ in range(28)) + "\n}"})
    bad({"lib.rs": "fn large() {\n" + "\n".join("    let _ = 1;" for _ in range(29)) + "\n}"}, "callable large is 31 lines")
    multiline_restricted = "pub(\n    crate\n)\nfn restricted() {\n" + "\n".join(
        "    let _ = 1;" for _ in range(27)
    ) + "\n}\n"
    bad({"lib.rs": multiline_restricted}, "callable restricted is 32 lines", 1)

    # Free, nested, inherent, trait declaration/default, and foreign declarations.
    forms = '''
fn free() { fn nested() {} }
struct S;
impl S { fn inherent() {} }
trait T { fn declaration(&self); fn defaulted(&self) {} }
unsafe extern "C" { fn foreign(); }
'''
    ok({"lib.rs": forms})
    body = "{\n" + "\n".join("    let _ = 1;" for _ in range(29)) + "\n}"
    callable_forms = {
        "free": "fn free() " + body,
        "nested": "fn outer() {\n    fn nested() " + body + "\n}",
        "inherent": "struct S;\nimpl S {\n    fn inherent() " + body + "\n}",
        "trait_default": "trait T {\n    fn trait_default(&self) " + body + "\n}",
    }
    for symbol, source in callable_forms.items():
        bad({"lib.rs": source + "\n"}, f"callable {symbol}")
    long_foreign = "unsafe extern \"C\" {\n    fn foreign(\n" + "\n".join(
        f"        arg{index}: u8," for index in range(29)
    ) + "\n    );\n}\n"
    bad({"lib.rs": long_foreign}, "callable foreign")
    long_trait_decl = "trait T {\n    fn declared(\n" + "\n".join(
        f"        arg{index}: u8," for index in range(29)
    ) + "\n    );\n}\n"
    bad({"lib.rs": long_trait_decl}, "callable declared")
    attached_exact = "// attached\n#[inline]\nfn exactly() {\n" + "\n".join(
        "    let _ = 1;" for _ in range(26)
    ) + "\n}\n"
    ok({"lib.rs": attached_exact}, "no violations")
    attached_over = "// attached\n#[inline]\nfn over() {\n" + "\n".join(
        "    let _ = 1;" for _ in range(29)
    ) + "\n}\n"
    bad({"lib.rs": attached_over}, "callable over is 31 lines", 3)

    # Callable spans start at the signature and include the closing brace.
    attached = "/// heading\n// note\n\n#[allow(dead_code)]\npub fn wide(\n    a: u8,\n) {\n}\n"
    ok({"lib.rs": attached})
    counted_prefix = "/// heading\n// note\n\n#[allow(dead_code)]\npub fn wide(\n    a: u8,\n) {\n" + "\n".join("    let _ = 1;" for _ in range(27)) + "\n}\n"
    bad({"lib.rs": counted_prefix}, "callable wide", 5)
    multiline_attr = "#[allow(\n    dead_code,\n)]\nfn attributed() {\n" + "\n".join("    let _ = 1;" for _ in range(29)) + "\n}\n"
    bad({"lib.rs": multiline_attr}, "callable attributed", 4)
    block_comment = "/** docs */\n/* lead */\nfn documented() {\n" + "\n".join("    let _ = 1;" for _ in range(29)) + "\n}\n"
    bad({"lib.rs": block_comment}, "callable documented", 3)
    # A leading dereference is code, not a continuation of an imaginary block comment.
    dereference_before_nested = "fn outer() {\n    *ptr;\n    fn nested() {\n" + "\n".join(
        "        let _ = 1;" for _ in range(29)
    ) + "\n    }\n}\n"
    bad({"lib.rs": dereference_before_nested}, "callable nested is 31 lines", 3)
    actual_block_before_nested = "fn outer() {\n    /* block\n     * continued\n     */\n    fn nested() {\n" + "\n".join(
        "        let _ = 1;" for _ in range(29)
    ) + "\n    }\n}\n"
    bad({"lib.rs": actual_block_before_nested}, "callable nested is 31 lines", 5)
    multiline_prefix_block = "/* lead\n   block continuation\n*/\nfn block_prefixed() {\n" + "\n".join(
        "    let _ = 1;" for _ in range(29)
    ) + "\n}\n"
    bad({"lib.rs": multiline_prefix_block}, "callable block_prefixed is 31 lines", 4)
    plain_block_before_nested = "fn outer() {\n    /* block\n    continuation without a star\n    */\n    fn nested() {\n" + "\n".join(
        "        let _ = 1;" for _ in range(29)
    ) + "\n    }\n}\n"
    bad({"lib.rs": plain_block_before_nested}, "callable nested is 31 lines", 5)
    raw_string_before_nested = (
        'fn outer() {\n    let text = r##"\n'
        '    // this looks like a leading comment but is raw-string content\n'
        '    "##; fn nested() {\n'
        + "\n".join("        let _ = 1;" for _ in range(29))
        + "\n    }\n}\n"
    )
    bad(
        {"lib.rs": raw_string_before_nested},
        "src/lib.rs:4: callable nested is 31 lines",
        4,
    )
    nested_test_code = "#[cfg(not(test))]\nfn outer() {\n    #[cfg(test)]\n    fn nested_test() {\n" + "\n".join(
        "        let _ = 1;" for _ in range(25)
    ) + "\n    }\n}\n"
    nested_test_result = invoke({"lib.rs": nested_test_code})
    nested_test_output = nested_test_result.stderr + nested_test_result.stdout
    assert nested_test_result.returncode == 0, nested_test_output
    assert "callable outer" not in nested_test_output and "callable nested_test" not in nested_test_output
    RESULTS.append(("pass", nested_test_result.returncode, "nested test lines excluded from production callable span", nested_test_output.strip()))
    # Prefix comments do not change the signature-based callable start line.
    prefixed = "// detached\n\nfn f() {\n" + "\n".join("    let _ = 0;" for _ in range(29)) + "\n}\n"
    bad({"lib.rs": prefixed}, "callable f is 31 lines", 3)
    separated = "// old comment\n\n\n// leading comment\nfn separated() {\n" + "\n".join("    let _ = 1;" for _ in range(29)) + "\n}\n"
    bad({"lib.rs": separated}, "callable separated is 31 lines", 5)
    unrelated = "// unrelated\n\n\nfn independent() {\n" + "\n".join(
        "    let _ = 1;" for _ in range(29)
    ) + "\n}\n"
    bad({"lib.rs": unrelated}, "callable independent is 31 lines", 4)
    contiguous = "// leading\n/// docs\n#[inline]\nfn prefixed() {\n" + "\n".join(
        "    let _ = 1;" for _ in range(29)
    ) + "\n}\n"
    bad({"lib.rs": contiguous}, "callable prefixed is 31 lines", 4)
    ok({"lib.rs": "//! inner docs\n\nfn f() {}\n"})
    inner_docs = "//! crate docs\n\nfn public_api() {\n" + "\n".join("    let _ = 1;" for _ in range(29)) + "\n}\n"
    bad({"lib.rs": inner_docs}, "callable public_api", 3)
    file_test = "#![cfg(test)]\nfn test_only() {\n" + "\n".join("    let _ = 1;" for _ in range(28)) + "\n}\n"
    ok({"lib.rs": file_test})
    inline_file_test = "mod test_module {\n    #![cfg(test)]\n    fn only_here() {\n" + "\n".join("        let _ = 1;" for _ in range(28)) + "\n    }\n}\n"
    ok({"lib.rs": inline_file_test})

    # Test-only contexts, symbolic cfg implication, and test ancestor propagation.
    test_only = "#[cfg(test)]\nmod tests {\n" + "\n".join("fn hidden() {}" for _ in range(40)) + "\n}\n#[test]\nfn unit() {}\n"
    ok({"lib.rs": test_only})
    mixed_large = "#[cfg(test)]\nmod tests {\n" + "\n".join("fn hidden() {}" for _ in range(298)) + "\n}\n"
    ok({"lib.rs": mixed_large}, "no violations")
    long_fn = "{\n" + "\n".join("    let _ = 1;" for _ in range(29)) + "\n}"
    ok({"lib.rs": "#[cfg(all(test, feature = \"fast\"))]\nfn only_test() " + long_fn + "\n"})
    bad({"lib.rs": "#[cfg(not(test))]\nfn production() " + long_fn + "\n"}, "callable production")
    bad({"lib.rs": "#[cfg(any(test, feature = \"fast\"))]\nfn production_possible() {\n" + "\n".join("    let _ = 1;" for _ in range(29)) + "\n}\n"}, "callable production_possible")
    # External and inline module handling; unreferenced source files are still included.
    ok({"lib.rs": "#[cfg(test)]\nmod test_child;\n", "test_child.rs": "fn hidden() {}\n"})
    test_child_large = "#[cfg(test)]\nmod test_child;\n"
    ok({"lib.rs": test_child_large, "test_child.rs": "\n".join(["// test-only"] * 350)})
    ok({"lib.rs": test_child_large, "test_child.rs": "fn hidden() {}\n"}, "1 maintained Rust files")
    ok({"lib.rs": test_child_large, "test_child.rs": "mod nested;\n", "test_child/nested.rs": "\n".join(["// nested test-only"] * 350)})
    # Inner file cfg must flow through module references; descendants stay test-only.
    ok({
        "lib.rs": "mod test_child;\n",
        "test_child.rs": "#![cfg(test)]\nmod nested;\n",
        "test_child/nested.rs": "\n".join(["// nested test-only"] * 350),
    }, "2 maintained Rust files")
    ok({"generated/output.rs": "\n".join(["// generated output"] * 350)}, "EXCLUDED src/generated/output.rs: generated source")
    bad({"vendor/copied.rs": "\n".join(["// vendor source"] * 350)}, "file is 350 lines", path="src/vendor/copied.rs")
    ok({"lib.rs": "mod child;\n", "child.rs": "fn visible() {}\n", "orphan.rs": "fn still_production() {}\n"})
    bad({"lib.rs": "mod missing;\n"}, "unresolved module missing")
    bad(
        {
            "lib.rs": "mod duplicate;\n",
            "duplicate.rs": "fn first_candidate() {}\n",
            "duplicate/mod.rs": "fn second_candidate() {}\n",
        },
        "ambiguous module duplicate",
        1,
    )
    ok({"lib.rs": "#[path = \"alt.rs\"]\nmod chosen;\n", "alt.rs": "fn chosen() {}\n"})
    bad({"lib.rs": "#[path = \"nope.rs\"]\nmod chosen;\n"}, "unresolved module file")
    for malformed_path in ["#[path]\nmod chosen;\n", "#[path(foo)]\nmod chosen;\n", "#[path = 7]\nmod chosen;\n"]:
        bad({"lib.rs": malformed_path}, "cannot classify module path", 1)
    # A nested lib.rs/main.rs is not a crate root when reached only from tests.
    for filename in ["nested/lib.rs", "nested/main.rs"]:
        ok({
            "lib.rs": "#[cfg(test)]\nmod tests;\n",
            "tests.rs": f'#[path = "{filename}"]\nmod nested_crate;\n',
            filename: "\n".join(["// test-only nested crate file"] * 350),
        }, "1 maintained Rust files")
    conditional_paths = {
        "lib.rs": '#[cfg(test)]\n#[path = "test_part.rs"]\nmod selected;\n#[cfg(not(test))]\n#[path = "prod_part.rs"]\nmod selected;\n',
        "test_part.rs": "\n".join(["// test-only"] * 350),
        "prod_part.rs": "\n".join(["// production alternate"] * 301),
    }
    bad(conditional_paths, "file is 301 lines", 1, "src/prod_part.rs")
    # Every path override on one conditional module declaration is a reference edge.
    # The second path must remain production even though the first path is valid.
    bad(
        {
            "lib.rs": '#[path = "small.rs"]\n#[path = "large.rs"]\nmod selected;\n',
            "small.rs": "fn small() {}\n",
            "large.rs": "\n".join(["// production alternate"] * 301),
        },
        "src/large.rs:1: file is 301 lines",
        1,
        "src/large.rs",
    )
    linked_source = invoke({
        "bin/entry.rs": '#[path = "../../tools/size_check/helper.rs"]\nmod helper;\n',
        "@root/tools/size_check/helper.rs": "fn external_helper() {}\n",
    })
    linked_output = linked_source.stderr + linked_source.stdout
    assert linked_source.returncode == 0 and "SOURCE tools/size_check/helper.rs" in linked_output, linked_output
    RESULTS.append(("pass", linked_source.returncode, "path-linked maintained source is enumerated", linked_output.strip()))

    linked_helper = "fn shared_helper() {\n" + "\n".join(
        "    let _ = 1;" for _ in range(29)
    ) + "\n}\n"
    bad(
        {
            "lib.rs": '#[cfg(test)]\n#[path = "shared.rs"]\nmod test_shared;\n',
            "bin/diagnostic.rs": '#[path = "../shared.rs"]\nmod production_shared;\n',
            "shared.rs": linked_helper + "\n".join(["// shared source"] * 270) + "\n",
        },
        "src/shared.rs:1: callable shared_helper is 31 lines",
        1,
        "src/shared.rs",
        extra_needles=("src/shared.rs:1: file is 301 lines", "0 classification errors"),
        relative_root=True,
    )
    ok(
        {
            "lib.rs": '#[cfg(test)]\n#[path = "test_helper.rs"]\nmod test_helper;\n',
            "test_helper.rs": "\n".join(["// test only"] * 350),
        },
        "1 maintained Rust files",
        relative_root=True,
    )
    ok(
        {
            "lib.rs": '#[cfg(test)]\n#[path = "../test_helper.rs"]\nmod test_helper;\n',
            "@root/test_helper.rs": "\n".join(["// test only"] * 350),
        },
        "1 maintained Rust files",
    )
    # Repeated leading parents in a relative root must retain their location.
    # The helper is reached from both a test path and a production path.
    bad(
        {
            "lib.rs": '#[cfg(test)]\n#[path = "../shared.rs"]\nmod test_shared;\n',
            "bin/entry.rs": '#[path = "../../shared.rs"]\nmod production_shared;\n',
            "@root/shared.rs": "fn reused_helper() {\n"
            + "".join("    let _ = 1;\n" for _ in range(29))
            + "}\n"
            + "".join("// shared source\n" for _ in range(270)),
        },
        "shared.rs:1: callable reused_helper is 31 lines",
        1,
        "shared.rs",
        extra_needles=("shared.rs:1: file is 301 lines", "0 classification errors"),
        parent_relative_root=True,
    )

    # cfg_attr is always unsupported, regardless of its predicate/result or location.
    cfg_attr_cases = [
        '#[cfg_attr(feature = "x", cfg(test))]\nfn f() {}\n',
        '#[cfg_attr(test, cfg(test))]\nfn f() {}\n',
        '#[cfg_attr(not(test), cfg(test))]\nfn f() {}\n',
        '#[cfg_attr(feature = "x", cfg_attr(test, cfg(test)))]\nfn f() {}\n',
        '#[cfg_attr(feature = "x", doc = "x")]\nfn f() {}\n',
        '#[cfg_attr(feature = "x", test)]\nfn f() {}\n',
        '#[cfg_attr(feature = "x", cfg(test))]\nmod external;\n',
        '#[cfg_attr(feature = "x", doc = "x")]\nstruct S;\n',
        '#![cfg_attr(feature = "x", doc = "crate docs")]\nfn f() {}\n',
    ]
    for body in cfg_attr_cases:
        bad({"lib.rs": body, "external.rs": "fn hidden() {}\n"}, "unsupported cfg_attr classification", 1)
    nested_result = invoke({"lib.rs": '#[cfg_attr(feature = "x",\n    cfg_attr(test, cfg(test)))]\nfn f() {}\n'})
    assert nested_result.returncode != 0 and "src/lib.rs:2: unsupported cfg_attr classification" in nested_result.stderr
    # Attribute on the external file itself is also fail-closed.
    bad({"lib.rs": "mod external;\n", "external.rs": "#[cfg_attr(test, doc = \"x\")]\nfn f() {}\n"}, "unsupported cfg_attr classification", 1, "src/external.rs")
    bad({"lib.rs": "mod nested {\n    #![cfg_attr(feature = \"x\", doc = \"x\")]\n}\n"}, "unsupported cfg_attr classification", 2)

    # Strings/comments do not create syntax; malformed Rust and unresolved includes fail.
    ok({"lib.rs": 'const S: &str = "include!(\\\"lost.rs\\\")";\n// cfg_attr(test, cfg(test))\n'})
    bad({"lib.rs": "fn broken( {\n"}, "parse error")
    bad({"lib.rs": 'const X: &str = include_str!("lost.txt");\n'}, "unresolved include_str! include")

    # Item-position macros fail closed; expression macros remain in their source context.
    bad({"lib.rs": "make_items!();\n"}, "unresolved item-position macro")
    bad({"lib.rs": "macro_rules! makes_fn { () => { fn generated() {} } }\n"}, "unsupported callable-producing macro definition")
    bad(
        {
            "lib.rs": 'macro_rules! with_cfg_attr {\n    () => {\n        #[cfg_attr(feature = "x", doc = "x")]\n        const HIDDEN: u8 = 0;\n    };\n}\n'
        },
        "unsupported cfg_attr classification",
        3,
    )
    ok({"Cargo.toml": '[package]\nname="fixture"\nversion="0.1.0"\nedition="2024"\n[dependencies]\nexternal_crate="1"\n', "lib.rs": "external_crate::build_items!();\n"})
    ok({"Cargo.toml": '[dependencies]\nexternal_crate="1"\n', "lib.rs": "use external_crate::build_items;\nbuild_items!();\n"})
    ok({"lib.rs": "fn f() { let _ = vec![1, 2]; }\n"})
    ok({"lib.rs": 'fn f() { let _ = stringify!(fn not_a_callable()); }\n'})
    ok({"lib.rs": "fn f() { unknown_statement_macro!(); }\n"})
    long_macro_fn = "fn hidden() {\n" + "\n".join("    let _ = 1;" for _ in range(28)) + "\n}"
    bad({"lib.rs": "fn f() { let _ = opaque!({ " + long_macro_fn + " }); }\n"}, "unsupported callable-producing macro form opaque!", 1)
    bad({"lib.rs": "#[custom::test]\nfn production() " + long_fn + "\n"}, "callable production", 2)
    bad({"lib.rs": "#[custom::cfg(test)]\nfn production() " + long_fn + "\n"}, "callable production", 2)
    missing_root = subprocess.run([str(BINARY), "--root"], text=True, capture_output=True)
    assert missing_root.returncode != 0 and "--root requires" in missing_root.stderr
    RESULTS.append(("fail", missing_root.returncode, "--root requires", missing_root.stderr.strip()))
    unknown = subprocess.run([str(BINARY), "--unexpected"], text=True, capture_output=True)
    assert unknown.returncode == 2 and "unknown argument: --unexpected" in unknown.stderr
    RESULTS.append(("fail", unknown.returncode, "unknown argument", unknown.stderr.strip()))
    repository_run = subprocess.run(
        [str(BINARY), "--root", "."], cwd=ROOT, text=True, capture_output=True
    )
    run_output = repository_run.stderr + repository_run.stdout
    (ROOT / "tests" / "size_check" / "diagnostic-run.txt").write_text(
        f"command: {BINARY} --root . (cwd: repository root)\nexit: {repository_run.returncode}\n{run_output}"
    )
    RESULTS.append((
        "repository-diagnostic",
        repository_run.returncode,
        "expected diagnostic run; see diagnostic-run.txt",
        f"{len(run_output.splitlines())} output lines; exit {repository_run.returncode}",
    ))
    (ROOT / "tests" / "size_check" / "validation-results.txt").write_text(
        "\n".join(
            f"{index:03} {status} exit={code} expected={expected!r} output={output!r}"
            for index, (status, code, expected, output) in enumerate(RESULTS, 1)
        ) + "\n"
    )
    print("size checker fixtures passed")


if __name__ == "__main__":
    main()
