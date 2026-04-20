# Contributing to SealStack

Thanks for your interest.

## Development

```bash
cargo check --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

## Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/): `feat(scope): subject`.

## Pull requests

- All PRs require a passing CI run and at least one maintainer review.
- Small, focused PRs land faster than large omnibus ones.

## CLA

By submitting a contribution, you agree to the Contributor License Agreement.
The CLA bot will prompt you on your first PR.
