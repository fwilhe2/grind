// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Print a prompt to copy into any model ([`grind_assist::copy`]).
//!
//! ```sh
//! cargo run -q -p grind-assist --example prompt -- ledger.fods G2:G40 \
//!     "sum all expenses of a month and show the result only in the first row of that month"
//! cargo run -q -p grind-assist --example prompt -- ledger.fods G2 "…" --no-values
//! ```
//!
//! The target is a cell, or a column range the formula will be filled down. `--no-values` keeps
//! every value at home: the prompt then carries headers and types only. Paste the model's answer
//! into the `answer` example.

use std::path::Path;

use grind_sheet::{App, a1};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let values = !args.iter().any(|a| a == "--no-values");
    let args: Vec<&String> = args.iter().filter(|a| *a != "--no-values").collect();
    let [file, target, request] = args.as_slice() else {
        eprintln!("usage: prompt FILE TARGET REQUEST [--no-values]");
        std::process::exit(2);
    };
    let app = App::new();
    app.open_file(Path::new(file.as_str()))
        .expect("the document opens");
    let reference = a1::parse(target).expect("a cell or a range");
    let (sheet, start, end) = a1::resolve(&app, &reference).expect("on a sheet that exists");
    print!(
        "{}",
        grind_assist::copy::copy_prompt(&app, sheet, start, end, request, values)
            .expect("a prompt")
    );
}
