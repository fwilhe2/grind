// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The engine, and everything `doc/dsl.md` §2 promises about it.
//!
//! §4.2's third argument for Rhai was that it is *built to be restricted* — keywords and
//! operators can be turned off one at a time and every limit is the host's to set — and this
//! module is that argument spent. It is deliberately one screen: a sandbox nobody can read in
//! one sitting is a sandbox nobody checks.
//!
//! **What the language cannot do at all**, and needs nothing from this file to be unable to:
//! there is no filesystem, no network, no environment and no randomness in core Rhai. Those
//! live in crates one does not take, which is the difference §4.2 draws between *sandboxing a
//! language* and using one designed to be reduced to a DSL.
//!
//! **What `Cargo.toml` takes away**, because a feature flag outranks a line of code somebody
//! could delete: `no_time` removes `timestamp()`, and turning off `ahash/runtime-rng` takes
//! the operating system's seed out of the hasher, so a map iterates in the same order on every
//! machine. §2 asks for the same source to produce the same bytes; those two are why it does.
//!
//! **What this file takes away**: `eval`, module imports, and unbounded anything.

use std::rc::Rc;

use rhai::Engine;

use crate::data::Data;

/// How many operations a script may perform before it is a build error.
///
/// §2: "a generator that does not terminate is a build error with a line number, not a hang".
/// Ten million is far more than a document needs — `examples/budget.rhai` uses about four
/// thousand — and small enough that a runaway loop stops in about a second rather than never.
const MAX_OPERATIONS: u64 = 10_000_000;

/// How deep a call may nest. A recursive function that never bottoms out is the other way a
/// script fails to terminate, and this is the one that catches it.
const MAX_CALL_DEPTH: usize = 64;

/// How long a string may get. The string builder is how a bounded loop still exhausts memory:
/// `s += s` doubles, so thirty iterations is a gigabyte and the operation limit never notices.
const MAX_STRING: usize = 1_000_000;

/// How many elements an array or a map may hold, for the same reason.
const MAX_ARRAY: usize = 1_000_000;

/// The engine a script runs in.
///
/// Every restriction is here rather than spread over the callers, so that "what may a script
/// do" is answered by reading one function.
pub fn engine(data: Rc<dyn Data>) -> Engine {
    let mut engine = Engine::new();

    // `eval` is a script rewriting itself at run time. Nothing a generator wants needs it, and
    // it defeats reading a script to know what it does — which is the property that makes a
    // generated document reviewable.
    engine.disable_symbol("eval");

    // No modules, so `import` resolves nothing. §9: an `import` that reaches a URL is the
    // supply chain this project does not have, and one that reaches a path is I/O.
    engine.set_max_modules(0);

    engine.set_max_operations(MAX_OPERATIONS);
    engine.set_max_call_levels(MAX_CALL_DEPTH);
    engine.set_max_string_size(MAX_STRING);
    engine.set_max_array_size(MAX_ARRAY);
    engine.set_max_map_size(MAX_ARRAY);

    // `print` and `debug` are the only output a script has, and they go to stderr: stdout
    // belongs to the command's own report, which may be JSON, and a script printing into the
    // middle of it would corrupt it.
    engine.on_print(|text| eprintln!("{text}"));
    engine.on_debug(|text, source, pos| match source {
        Some(source) => eprintln!("{source} @ {pos:?}: {text}"),
        None => eprintln!("{pos:?}: {text}"),
    });

    crate::sheet::register(&mut engine);
    crate::text::register(&mut engine);
    // The one door outward, and `data.rs` is the whole of what it opens onto.
    crate::data::register(&mut engine, data);
    engine
}

#[cfg(test)]
mod tests {
    use rhai::Dynamic;

    /// The clock is gone from the *language*, not unregistered by hand — `no_time` in
    /// `Cargo.toml`. This test is what notices somebody taking the feature flag back off.
    #[test]
    fn there_is_no_clock() {
        let error = super::engine(std::rc::Rc::new(crate::data::NoData))
            .eval::<Dynamic>("timestamp()")
            .expect_err("timestamp() is not a function this build has");
        assert!(error.to_string().contains("Function not found"), "{error}");
    }

    #[test]
    fn a_script_cannot_rewrite_itself() {
        let error = super::engine(std::rc::Rc::new(crate::data::NoData))
            .eval::<Dynamic>(r#"eval("1 + 1")"#)
            .expect_err("eval is disabled");
        assert!(error.to_string().contains("eval"), "{error}");
    }

    /// The promise that matters most, because it is the one a mistake trips rather than an
    /// attacker: a loop that never ends stops, and says where.
    #[test]
    fn a_loop_that_does_not_terminate_is_an_error_with_a_line_number() {
        let error = super::engine(std::rc::Rc::new(crate::data::NoData))
            .eval::<Dynamic>("let i = 0;\nloop { i += 1; }")
            .expect_err("the operation limit is reached");
        assert!(error.to_string().contains("operations"), "{error}");
        assert_eq!(error.position().line(), Some(2));
    }

    #[test]
    fn a_string_cannot_be_grown_without_bound() {
        let error = super::engine(std::rc::Rc::new(crate::data::NoData))
            .eval::<Dynamic>("let s = \"x\";\nloop { s += s; }")
            .expect_err("the string limit is reached");
        assert!(error.to_string().contains("too large"), "{error}");
    }

    /// Not a sandbox this file built — core Rhai has no such function — but the day somebody
    /// adds a package to the engine above, this is the test that asks what came with it.
    #[test]
    fn there_is_no_way_to_reach_a_file() {
        for call in [
            "open_file(\"/etc/passwd\")",
            "read_file(\"x\")",
            "system(\"ls\")",
        ] {
            let error = super::engine(std::rc::Rc::new(crate::data::NoData))
                .eval::<Dynamic>(call)
                .expect_err("no I/O is registered");
            assert!(error.to_string().contains("Function not found"), "{error}");
        }
    }
}
