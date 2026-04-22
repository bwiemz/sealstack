//! Verifies that the TypeScript code emitted by `CompileTargets::TYPESCRIPT`
//! is valid TypeScript via a real `tsc --noEmit` run, not just a syntactic
//! inspection. Gated behind the `slow-tests` feature because it shells out
//! to `pnpm`.
//!
//! If pnpm is not on PATH, the test is skipped with a message rather than
//! failing — local developer environments vary, and the snapshot + unit
//! tests already catch most real issues. CI sets up Node/pnpm and enforces
//! the full run.

#![cfg(feature = "slow-tests")]

use std::process::Command;

use sealstack_csl::{CompileTargets, compile};

fn pnpm_available() -> bool {
    // `pnpm --version` exits 0 iff pnpm is on PATH and functional.
    Command::new("pnpm")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn generated_typescript_compiles_under_tsc() {
    if !pnpm_available() {
        eprintln!("skipping: pnpm not on PATH");
        return;
    }

    let src = include_str!("fixtures/rust_shapes.csl");
    let out = compile(src, CompileTargets::TYPESCRIPT).expect("compile");

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::write(
        root.join("package.json"),
        r#"{
  "name": "generated-check",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "devDependencies": {
    "typescript": "^5.7.0"
  }
}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "exactOptionalPropertyTypes": true,
    "noEmit": true,
    "skipLibCheck": true,
    "isolatedModules": true
  },
  "include": ["generated.ts"]
}
"#,
    )
    .unwrap();
    std::fs::write(root.join("generated.ts"), &out.typescript).unwrap();

    let install = Command::new("pnpm")
        .args(["install", "--silent", "--ignore-scripts"])
        .current_dir(root)
        .output()
        .expect("pnpm install");
    assert!(
        install.status.success(),
        "pnpm install failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&install.stdout),
        String::from_utf8_lossy(&install.stderr),
    );

    let tsc = Command::new("pnpm")
        .args(["exec", "tsc", "--noEmit"])
        .current_dir(root)
        .output()
        .expect("pnpm exec tsc");

    assert!(
        tsc.status.success(),
        "generated TypeScript failed tsc --noEmit:\nstdout: {}\nstderr: {}\ngenerated.ts:\n{}",
        String::from_utf8_lossy(&tsc.stdout),
        String::from_utf8_lossy(&tsc.stderr),
        out.typescript,
    );

    // Spec §11.3 also requires empty stderr — tsc under --strict on a
    // self-contained file produces none today. Any future deprecation /
    // info message surfaces here rather than passing silently.
    assert!(
        tsc.stderr.is_empty(),
        "tsc --noEmit exited 0 but emitted stderr:\n{}",
        String::from_utf8_lossy(&tsc.stderr),
    );
}
