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

- Shows git status
- Displays package info
- Checks crate existence on crates.io
