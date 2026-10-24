// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Registering a function and documenting it are one act. [`hint`] is `register_fn` with the
//! parameter spelling and doc comment an editor shows, and no way to skip them, so the host API
//! cannot quietly become undocumented. `grind definitions` writes what every call supplies as a
//! `.d.rhai` file, and `--snippets` as VS Code snippets. Needs Rhai's `metadata` and `internals`
//! features.

use rhai::{Engine, FuncRegistration, RhaiNativeFunc, Variant};

/// Register one function, with the parameter spelling and the documentation an editor shows.
///
/// `params` is one entry per parameter and then the return type, in Rhai's definition spelling:
/// `"name: string"`, `"sheet: Sheet"`, `"int"`. A method's receiver is its first parameter. `doc`
/// is doc-comment lines, each starting with `///`, kept verbatim.
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

/// The same for a *property* — `s.rows` beside `s.rows()` — so a getter is documented like any
/// other function.
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
