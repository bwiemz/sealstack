//! Boot a gateway, run a fixed set of scenarios, capture request and
//! response pairs as JSON files into `contracts/fixtures/`.
//!
//! Phase 1 stub: prints the expected output dir and exits 0. Phase 4
//! wires real scenarios.

fn main() {
    let cwd = std::env::current_dir().expect("cwd");
    let out = cwd.join("contracts/fixtures");
    println!(
        "emit-fixtures: stub. would write to {} once scenario list is wired",
        out.display()
    );
}
