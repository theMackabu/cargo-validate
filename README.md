# cargo-validate

A Rust tool that enhances `cargo publish` with a validation and confirmation step.

## Install

```
cargo install cargo-validate
```

## Usage

```
cargo validate [-- <cargo publish args>]
```

Features:

- Informs of current git status
- Displays and validates package info
- Checks current username as owner
- Checks crate existence on crates.io
