# Critical Rules (read first)

- **Never run `git commit`** unless the user explicitly says to.
- **Run end-of-feature checks** before declaring work done.

# General

Use the Read tool to read file contents. Do not use shell commands (`cat`, `head`, `tail`, `sed`, `awk`, etc.) to read files. Shell commands may be used to _find_ and _filter_ things in files.

# Commits

Commit messages must follow Conventional Commit formats. These should be single-line and under 100 characters long. This is enforced with a git hook. Do not ever manually run `git commit` unless explicitly instructed to by the user.

# Code changes

Never delete code just because it appears unused — not even when refactoring. Ask the user first. Suppressing the warning (`#[expect(dead_code)]`, `_` prefix, etc.) is always acceptable instead.

Compiler/lint errors during a multi-step edit are expected and should be ignored. Only run the end-of-feature checks (see below) once all planned changes are complete.

Every code change must be accompanied by one or more tests. If the code being modified has no existing test coverage, write a passing test that captures the current behavior _before_ making any changes — this acts as a regression guard. Then make the change and add or update tests to cover the new behavior.

When adding a crate, always check what the latest version is with `cargo search <crate-name> --limit 1`, never assume you know what the latest version is. Unless the user explicitly says otherwise, always use the latest version of a crate.

Once a backend feature is finished, run these checks in order:

1. `cargo clippy --workspace --all-targets` — Clippy/compiler errors
2. `cargo test --workspace` — tests

Organize functions and types top-down: higher-level concepts and callers come first, helpers and lower-level functions come later. A function should never call a function defined above it in the file. Types used by functions go below them, a function should never use a type declared above it in the file. Types used by other types go below them, a type should never use a type declared above it in the file.
