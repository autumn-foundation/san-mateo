//! The runnable examples stay wired (EX11): each one has a test, a CI step
//! and a README row. See `docs/adr/0006-runnable-examples.md`.
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
    /// Text that the CI workflow must run.
    ci: &'static str,
    /// Text that the README must show.
    readme: &'static str,
}

const EXAMPLES: &[Example] = &[
    Example {
        name: "embedded",
        manifest: "crates/san-mateo/Cargo.toml",
        target: "[[example]]",
        source: "crates/san-mateo/examples/embedded.rs",
        ci: "cargo run --locked -p san-mateo --example embedded",
        readme: "cargo run --locked -p san-mateo --example embedded",
    },
    Example {
        name: "autumn_app",
        manifest: "crates/autumn-plugin-san-mateo/Cargo.toml",
        target: "[[example]]",
        source: "crates/autumn-plugin-san-mateo/examples/autumn_app/main.rs",
        ci: "bash scripts/smoke-autumn-example.sh",
        readme: "cargo run --locked -p autumn-plugin-san-mateo --example autumn_app",
    },
    Example {
        name: "harvest_workflow",
        manifest: "crates/san-mateo-harvest/Cargo.toml",
        target: "[[example]]",
        source: "crates/san-mateo-harvest/examples/harvest_workflow.rs",
        ci: "cargo run --locked -p san-mateo-harvest --example harvest_workflow",
        readme: "cargo run --locked -p san-mateo-harvest --example harvest_workflow",
    },
    Example {
        name: "harvest_camel",
        manifest: "examples/harvest-camel/Cargo.toml",
        target: "[[bin]]",
        source: "examples/harvest-camel/src/main.rs",
        ci: "cargo run --locked --manifest-path examples/harvest-camel/Cargo.toml",
        readme: "cargo run --locked --manifest-path examples/harvest-camel/Cargo.toml",
    },
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
        if ex.target == "[[example]]" {
            assert!(
                section.lines().any(|l| l.trim() == "test = true"),
                "{}: the target needs `test = true`",
                ex.name
            );
        }
        let source = read(ex.source);
        for must in ["const RUN_TIMEOUT", "#[cfg(test)]"] {
            assert!(source.contains(must), "{}: the source needs {must}", ex.name);
        }
        for never in ["allow_private_hosts", "allow_plain_http", "full_migrations_sql"] {
            assert!(!source.contains(never), "{}: the source has {never}", ex.name);
        }
        assert!(
            ci.iter().any(|l| l.contains(ex.ci)),
            "{}: CI does not run `{}`",
            ex.name,
            ex.ci
        );
        assert!(
            readme.contains(ex.readme),
            "{}: the README does not show `{}`",
            ex.name,
            ex.readme
        );
    }
    assert!(
        !ci.iter().any(|l| l.contains("continue-on-error")),
        "a CI step must not hide a failure"
    );
}

#[test]
fn each_example_file_is_in_the_list() {
    let mut found = Vec::new();
    for krate in std::fs::read_dir(root().join("crates")).unwrap() {
        let dir = krate.unwrap().path().join("examples");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            found.push(stem);
        }
    }
    for name in found {
        assert!(
            EXAMPLES.iter().any(|ex| ex.name == name),
            "the example {name} is not in EXAMPLES"
        );
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
    for name in ["harvest-camel", "autumn-harvest-plugin", "pq-src", "libsqlite3-sys"] {
        assert!(
            !lock.contains(&format!("name = \"{name}\"")),
            "the root Cargo.lock has {name}"
        );
    }
    // The example uses the same lints as the main workspace.
    let lints = |text: &str, from: &str| -> String {
        let start = text.find(from).unwrap_or_else(|| panic!("no {from}"));
        text[start..]
            .lines()
            .map(|l| l.replace("[workspace.lints.", "[lints."))
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
