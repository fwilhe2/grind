# Repository Guidelines

<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
SPDX-License-Identifier: CC-BY-4.0
-->

## Project Structure & Module Organization

Grind is a Rust 2024 workspace. Product components are separate crates: `core`, `sheet`, `text`, `build`, `xlsx`, `ooxml`, `docx`, and `print` hold shared models and document handling; `cli`, `lsp`, and `open` provide command-line and integration layers. Platform clients live in `ui_*` crates. Rust sources are in each crate's `src/`, with integration tests and fixtures under `tests/`. Project specifications and contributor documentation are in `doc/`; examples are in `examples/`.

## Build, Test, and Development Commands

- `cargo build --locked` builds the default workspace members using the committed lockfile.
- `cargo test --locked` runs their unit and integration tests.
- `cargo test --locked -p grind-sheet` limits testing to one crate; use the crate's package name from its `Cargo.toml`.
- `cargo fmt --all -- --check` checks Rust formatting; `cargo fmt --all` applies it.
- `cargo clippy --locked --all-targets -- -D warnings` runs lint checks.
- `reuse lint` checks required copyright and license metadata.

The default members omit GTK crates so the standard commands work without GTK system development packages. To build the full workspace, use `--workspace`; native clients may require platform libraries. Some corpus tests accept `GRIND_LO_CORPUS=/path/to/libreoffice/core` and skip when it is unset.

## Coding Style & Naming Conventions

Follow `rustfmt.toml` and the workspace's Rust conventions: four-space indentation, `snake_case` for modules/functions, and `UpperCamelCase` for types. Keep functionality in the relevant crate and add focused integration tests under its `tests/` directory. Every new file needs SPDX metadata; see `CONTRIBUTING.md` for the correct license by file type and REUSE workflow.

## Testing Guidelines

Use `cargo test --locked` for the normal suite and the targeted package command while iterating. Keep test names descriptive of behavior and place shared test helpers in `tests/common/`. For format changes, consult `CONTRIBUTING.md`'s round-trip and corpus test guidance. Run `reuse lint` before submitting new files.

## Commit & Pull Request Guidelines

Recent commits use concise, sentence-style summaries describing the behavior or fix (for example, “A viewport is bounded…”). Follow that descriptive style. Pull requests should explain the user-visible change, list relevant checks, link related issues when applicable, and include screenshots for UI changes. Format and license new files correctly, keep `Cargo.lock` changes intentional, and observe the clean-room and licensing requirements in `CONTRIBUTING.md`.
