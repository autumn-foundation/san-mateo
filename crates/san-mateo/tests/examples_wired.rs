//! The runnable examples stay wired (EX11). For each example, this test
//! checks the Cargo target, a self-check and a failure test in the source,
//! the CI commands and the README row. See `docs/adr/0006-runnable-examples.md`.
//!
//! The test reads files of the repository, so `Cargo.toml` leaves it out
//! of the published crate. It uses text search: keep the fixed formats below.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

struct Example {
    name: &'static str,
    /// The Cargo manifest that declares the target.
    manifest: &'static str,
    /// The target header: `[[example]]` or `[[bin]]`.
    target: &'static str,
    source: &'static str,
    /// Commands that the CI workflow must run.
    ci: &'static [&'static str],
    /// The command that the README must show.
    readme: &'static str,
}

const EXAMPLES: &[Example] = &[
    Example {
        name: "embedded",
        manifest: "crates/san-mateo/Cargo.toml",
        target: "[[example]]",
        source: "crates/san-mateo/examples/embedded.rs",
        ci: &["cargo run --locked -p san-mateo --example embedded"],
        readme: "cargo run --locked -p san-mateo --example embedded",
    },
    Example {
        name: "autumn_app",
        manifest: "crates/autumn-plugin-san-mateo/Cargo.toml",
        target: "[[example]]",
        source: "crates/autumn-plugin-san-mateo/examples/autumn_app/main.rs",
        ci: &["bash scripts/smoke-autumn-example.sh"],
        readme: "cargo run --locked -p autumn-plugin-san-mateo --example autumn_app",
    },
    Example {
        name: "harvest_workflow",
        manifest: "crates/san-mateo-harvest/Cargo.toml",
        target: "[[example]]",
        source: "crates/san-mateo-harvest/examples/harvest_workflow.rs",
        ci: &["cargo run --locked -p san-mateo-harvest --example harvest_workflow"],
        readme: "cargo run --locked -p san-mateo-harvest --example harvest_workflow",
    },
    Example {
        name: "harvest_camel",
        manifest: "examples/harvest-camel/Cargo.toml",
        target: "[[bin]]",
        source: "examples/harvest-camel/src/main.rs",
        // The root jobs do not see this workspace, so CI must run its tests.
        ci: &[
            "cargo test --locked --manifest-path examples/harvest-camel/Cargo.toml",
            "cargo run --locked --manifest-path examples/harvest-camel/Cargo.toml",
        ],
        readme: "cargo run --locked --manifest-path examples/harvest-camel/Cargo.toml",
    },
];

/// Files that users copy. They must not turn on the dev-only proxy flags.
const COPIED_FILES: &[&str] = &[
    "crates/autumn-plugin-san-mateo/examples/autumn_app/autumn.toml",
    "scripts/smoke-autumn-example.sh",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The lines of the target section `name` in a manifest, up to the next
/// section header.
fn target_section(manifest: &str, header: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = manifest.lines().collect();
    let name_line = format!("name = \"{name}\"");
    lines.iter().enumerate().find_map(|(i, line)| {
        if line.trim() != header || lines.get(i + 1).map(|l| l.trim()) != Some(&name_line) {
            return None;
        }
        let end = lines[i + 1..]
            .iter()
            .position(|l| l.starts_with('['))
            .map_or(lines.len(), |p| i + 1 + p);
        Some(lines[i..end].join("\n"))
    })
}

/// The workflow lines that run commands, with no comment lines.
fn ci_lines() -> Vec<String> {
    read(".github/workflows/ci.yml")
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn each_example_has_a_test_a_ci_step_and_a_readme_row() {
    let ci = ci_lines();
    let readme = read("README.md");
    for ex in EXAMPLES {
        let manifest = read(ex.manifest);
        let section = target_section(&manifest, ex.target, ex.name)
            .unwrap_or_else(|| panic!("{}: no {} target in {}", ex.name, ex.target, ex.manifest));
        let has = |line: &str| section.lines().any(|l| l.trim() == line);
        if ex.target == "[[example]]" {
            assert!(
                has("test = true"),
                "{}: the target needs `test = true`",
                ex.name
            );
        } else {
            assert!(
                !has("test = false"),
                "{}: the target must keep its tests",
                ex.name
            );
        }
        let source = read(ex.source);
        for must in [
            "const RUN_TIMEOUT",
            "#[cfg(test)]",
            "fn check(",
            "expect_err(",
        ] {
            assert!(
                source.contains(must),
                "{}: the source needs {must}",
                ex.name
            );
        }
        assert!(
            !source.contains("full_migrations_sql"),
            "{}: use `migrate::apply`",
            ex.name
        );
        for command in ex.ci {
            let lines: Vec<&String> = ci.iter().filter(|l| l.contains(command)).collect();
            assert!(
                !lines.is_empty(),
                "{}: CI does not run `{command}`",
                ex.name
            );
            assert!(
                lines.iter().all(|l| !l.contains("||")),
                "{}: CI must not hide a failure of `{command}`",
                ex.name
            );
        }
        assert!(
            readme.contains(ex.readme),
            "{}: the README does not show `{}`",
            ex.name,
            ex.readme
        );
        assert!(
            readme.contains(&format!("| [`{}`](", ex.name)),
            "{}: the README Examples table has no row",
            ex.name
        );
    }
    assert!(
        !ci.iter().any(|l| l.contains("continue-on-error")),
        "a CI step must not hide a failure"
    );
}

#[test]
fn no_example_turns_on_the_dev_only_proxy_flags() {
    let files = EXAMPLES
        .iter()
        .map(|ex| ex.source)
        .chain(COPIED_FILES.iter().copied());
    for path in files {
        let text = read(path).to_ascii_lowercase();
        for flag in ["allow_private_hosts", "allow_plain_http"] {
            assert!(!text.contains(flag), "{path} has {flag}");
        }
    }
}

#[test]
fn each_example_file_is_in_the_list() {
    let listed = |name: &str| EXAMPLES.iter().any(|ex| ex.name == name);
    for krate in std::fs::read_dir(root().join("crates")).unwrap() {
        let dir = krate.unwrap().path().join("examples");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            // A cargo example is a `.rs` file or a folder with `main.rs`.
            let is_example =
                path.extension().is_some_and(|e| e == "rs") || path.join("main.rs").exists();
            if is_example && !name.starts_with('.') {
                assert!(listed(&name), "the example {name} is not in EXAMPLES");
            }
        }
    }
    // An example with its own workspace is a folder of `examples/`.
    for entry in std::fs::read_dir(root().join("examples")).unwrap() {
        let dir = entry.unwrap().path();
        let manifest = dir.join("Cargo.toml");
        if manifest.exists() {
            let relative = format!(
                "examples/{}/Cargo.toml",
                dir.file_name().unwrap().to_string_lossy()
            );
            assert!(
                EXAMPLES.iter().any(|ex| ex.manifest == relative),
                "the workspace {relative} is not in EXAMPLES"
            );
        }
    }
}

#[test]
fn the_harvest_camel_workspace_stays_apart() {
    let root_manifest = read("Cargo.toml");
    assert!(
        root_manifest.contains("exclude = [\"examples/harvest-camel\"]"),
        "the root workspace must exclude examples/harvest-camel"
    );
    // These crates come only with harvest-camel. They must not enter the
    // main workspace (ADR 0006).
    let lock = read("Cargo.lock");
    for name in [
        "harvest-camel",
        "autumn-harvest-plugin",
        "pq-src",
        "libsqlite3-sys",
    ] {
        assert!(
            !lock.contains(&format!("name = \"{name}\"")),
            "the root Cargo.lock has {name}"
        );
    }
    // The example uses the same lints as the main workspace. Compare the
    // lint sections only, up to the next other section.
    let lints = |text: &str, from: &str| -> String {
        let start = text.find(from).unwrap_or_else(|| panic!("no {from}"));
        text[start..]
            .lines()
            .map(|l| l.replace("[workspace.lints.", "[lints."))
            .take_while(|l| !l.starts_with('[') || l.starts_with("[lints."))
            .filter(|l| !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        lints(&read("examples/harvest-camel/Cargo.toml"), "[lints.rust]"),
        lints(&root_manifest, "[workspace.lints.rust]"),
        "examples/harvest-camel must use the root lints"
    );
}
