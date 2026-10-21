// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Check what a model answered to a `prompt`: read it in display or Excel syntax
//! ([`grind_assist::read`]), put it in the target and fill it down **in memory**, recalculate, and
//! show every target cell's value beside its row. Nothing is written unless `--write` names a
//! file — and then to that file, never over the one read.
//!
//! ```sh
//! cargo run -q -p grind-assist --example answer -- ledger.fods G2:G40 '=IF(MONTH(A2)<>MONTH(A1),…)'
//! cargo run -q -p grind-assist --example answer -- ledger.fods G2:G40 '=…' --write checked.fods
//! ```

use std::path::Path;

use grind_assist::Spelling;
use grind_sheet::{App, Pos, a1};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let write = args.iter().position(|a| a == "--write").map(|at| {
        let path = args.get(at + 1).cloned().expect("--write wants a file");
        args.drain(at..=at + 1);
        path
    });
    let [file, target, answer] = args.as_slice() else {
        eprintln!("usage: answer FILE TARGET FORMULA [--write OUT]");
        std::process::exit(2);
    };
    let app = App::new();
    app.open_file(Path::new(file)).expect("the document opens");
    let (sheet, start, end) =
        a1::resolve(&app, &a1::parse(target).expect("a cell or a range")).expect("a sheet");
    let tried =
        grind_assist::copy::try_answer(&app, sheet, start, end, answer).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });
    println!(
        "read as {}: {}",
        match tried.spelling {
            Spelling::Display => "display syntax",
            Spelling::Excel => "Excel syntax, translated",
        },
        tried.formula
    );
    let (_, cols) = app.used_extent(sheet).unwrap();
    for (pos, value) in &tried.values {
        let mut line = vec![format!("{:>4}", pos.row + 1)];
        for col in (0..cols)
            .filter(|c| !(start.col..=end.col).contains(c))
            .take(4)
        {
            line.push(
                app.value_text(sheet, Pos { row: pos.row, col })
                    .unwrap_or_default(),
            );
        }
        line.push(format!("→ {value}"));
        println!("{}", line.join(" | "));
    }
    if let Some(out) = write {
        grind_assist::copy::apply_answer(&app, sheet, start, end, &tried).expect("applied");
        app.save_file(Path::new(&out)).expect("saved");
        println!("written to {out}");
    }
}
