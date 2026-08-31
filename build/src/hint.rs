// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Registering a function and documenting it are one act.**
//!
//! Rhai's `Engine::register_fn` takes a name and a closure and nothing else, so the natural
//! shape of a host API is one where no function has a description and an editor can offer a
//! name and a guess at its types. That is a poor way to meet a language: the first thing
//! somebody writing a script wants is *what can I say here*, and the answer lived only in
//! `doc/generator-spec.md`, which is not open while they type.
//!
//! [`hint`] is `register_fn` with two more arguments and no way to skip them — the parameter
//! spelling an editor shows, and the doc comment it shows underneath. `grind definitions`
//! turns what every call here supplies into a `.d.rhai` file, which is what the Rhai language
//! server reads for completion and hover (`doc/generator-spec.md` §9).
//!
//! **Why this is a function rather than a convention.** A convention would be "remember to call
//! `FuncRegistration::new(…).with_comments(…)`", and half the registrations would end up as
//! plain `register_fn` within a year. Making the documented form the *only* form costs four
//! lines per function and buys an API that cannot quietly become undocumented.
//!
//! The cost is real and named: `build/Cargo.toml` takes Rhai's `metadata` and `internals`
//! features for this, which is what carries the strings into the engine and lets the
//! definitions writer read them back out.

use rhai::{Engine, FuncRegistration, RhaiNativeFunc, Variant};

/// Register one function, with the parameter spelling and the documentation an editor shows.
///
/// `params` is one entry per parameter and then the return type, in Rhai's own definition
/// spelling: `"name: string"`, `"sheet: Sheet"`, `"int"`. A method's receiver is its first
/// parameter, which is what `s.push(row)` looks like from the inside.
///
/// `doc` is the lines of a doc comment, each starting with `///`, exactly as they would be
/// written in Rust — Rhai keeps them verbatim and the definitions file prints them back.
pub fn hint<A, const N: usize, const X: bool, R, const F: bool, S, D>(
    engine: &mut Engine,
    name: &str,
    params: impl IntoIterator<Item = S>,
    doc: impl IntoIterator<Item = D>,
    func: impl RhaiNativeFunc<A, N, X, R, F> + 'static,
) where
    A: 'static,
    R: Variant + Clone,
    S: AsRef<str>,
    D: AsRef<str>,
{
    FuncRegistration::new(name)
        .with_params_info(params)
        .with_comments(doc)
        .register_into_engine(engine, func);
}

/// The same for a *property* — `s.rows` beside `s.rows()`.
///
/// A getter is a registration like any other, so it is documented like any other: an editor
/// completing `s.` should say what `rows` is, and "the property nobody wrote a line about" is
/// exactly the kind of gap that opens when two registration paths exist and only one of them
/// takes a comment.
pub fn hint_get<A, const N: usize, const X: bool, R, const F: bool, D>(
    engine: &mut Engine,
    property: &str,
    doc: impl IntoIterator<Item = D>,
    func: impl RhaiNativeFunc<A, N, X, R, F> + 'static,
) where
    A: 'static,
    R: Variant + Clone,
    D: AsRef<str>,
{
    FuncRegistration::new_getter(property)
        .with_comments(doc)
        .register_into_engine(engine, func);
}
