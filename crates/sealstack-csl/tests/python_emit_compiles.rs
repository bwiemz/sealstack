//! Verifies that the Python code emitted by `CompileTargets::PYTHON` passes
//! `mypy --strict` via a real subprocess run, not just syntactic inspection.
//! Gated behind the `slow-tests` feature because it shells out to Python.
//!
//! Skipped with a message if Python 3.11+ is not on PATH. The snapshot +
//! unit tests already cover most correctness; this is defense-in-depth.

#![cfg(feature = "slow-tests")]

use std::process::Command;

use sealstack_csl::{CompileTargets, compile};

/// Tries `python3`, `python`, `py` in order. Returns the first command name
/// whose `--version` reports Python 3.11+.
///
/// On Windows, the default `python`/`python3` aliases are Microsoft Store
/// stubs that print a "Python was not found" banner — we detect those and
/// skip to `py` (the real PEP 397 launcher).
///
/// Older Pythons print the version banner to stderr (Python 3.3 and earlier
/// quirk); modern 3.11+ prints to stdout. We check both for safety.
fn find_python() -> Option<&'static str> {
    for candidate in ["python3", "python", "py"] {
        let Ok(output) = Command::new(candidate).arg("--version").output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}");
        // Microsoft Store stub on Windows prints this banner but may exit 0.
        if combined.contains("was not found") || combined.contains("Microsoft Store") {
            continue;
        }
        if parse_python_minor(&combined).is_some_and(|minor| minor >= 11) {
            return Some(candidate);
        }
    }
    None
}

/// Extract the minor version number from a `--version` banner like
/// `"Python 3.11.4"`. Returns `None` if the string isn't a recognizable
/// `Python 3.<minor>...` banner.
fn parse_python_minor(banner: &str) -> Option<u32> {
    let trimmed = banner.trim();
    let rest = trimmed.strip_prefix("Python 3.")?;
    let minor_str = rest.split(|c: char| !c.is_ascii_digit()).next()?;
    minor_str.parse::<u32>().ok()
}

#[test]
fn generated_python_passes_mypy_strict() {
    let Some(py) = find_python() else {
        eprintln!("skipping: python 3.11+ not on PATH");
        return;
    };

    let src = include_str!("fixtures/rust_shapes.csl");
    let out = compile(src, CompileTargets::PYTHON).expect("compile");

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::write(root.join("generated.py"), &out.python).unwrap();
    std::fs::write(
        root.join("pyproject.toml"),
        r#"[tool.mypy]
strict = true
"#,
    )
    .unwrap();

    // Install mypy into an ephemeral vendored dir. Using `--target` instead
    // of a full venv to keep setup fast; mypy has no import-time side
    // effects that would care.
    let install = Command::new(py)
        .args([
            "-m",
            "pip",
            "install",
            "--quiet",
            "--target",
            "./vendor",
            "mypy>=1.11",
        ])
        .current_dir(root)
        .output()
        .expect("spawn pip install");
    assert!(
        install.status.success(),
        "pip install mypy failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&install.stdout),
        String::from_utf8_lossy(&install.stderr),
    );

    // PYTHONPATH must point to the vendored dir so `python -m mypy` finds it.
    let mypy = Command::new(py)
        .args(["-m", "mypy", "--strict", "generated.py"])
        .env("PYTHONPATH", root.join("vendor"))
        .current_dir(root)
        .output()
        .expect("spawn mypy");

    assert!(
        mypy.status.success(),
        "generated Python failed mypy --strict:\nstdout: {}\nstderr: {}\ngenerated.py:\n{}",
        String::from_utf8_lossy(&mypy.stdout),
        String::from_utf8_lossy(&mypy.stderr),
        out.python,
    );

    // Spec §0 success criteria: zero errors AND zero warnings.
    assert!(
        mypy.stderr.is_empty(),
        "mypy --strict exited 0 but emitted stderr:\n{}",
        String::from_utf8_lossy(&mypy.stderr),
    );
}
