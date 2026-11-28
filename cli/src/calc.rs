// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind calc`: the tape (`doc/calc.md`) from the command line, one-shot or interactive.

use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use grind_sheet::Tape;
use grind_sheet::locale::Locale;

use crate::failure::{REFUSED, UNREADABLE, USAGE};
use crate::report::Format;

pub(crate) struct Options<'a> {
    pub lines: &'a [String],
    pub locale: Option<Locale>,
    pub open: Option<&'a Path>,
    pub save: Option<&'a Path>,
    pub force: bool,
    pub extract: Option<&'a [String]>,
    pub nth: Option<usize>,
    pub format: Format,
}

pub(crate) fn run(options: &Options) -> ExitCode {
    let mut tape = match options.open {
        Some(path) => match Tape::open_file(path) {
            Ok(tape) => tape,
            Err(e) => {
                eprintln!("grind calc: {}: {e}", path.display());
                return ExitCode::from(UNREADABLE);
            }
        },
        None => Tape::new(options.locale.clone()),
    };
    let stdin = std::io::stdin();
    if options.lines.is_empty() && stdin.is_terminal() {
        return interactive(&mut tape, stdin.lock());
    }
    let lines: Vec<String> = if options.lines.is_empty() {
        stdin
            .lock()
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.trim().is_empty())
            .collect()
    } else {
        options.lines.to_vec()
    };
    let before = tape.len();
    for (i, line) in lines.iter().enumerate() {
        if let Err(e) = tape.push(line) {
            eprintln!("grind calc: line {}: {e}", before + i + 1);
            return ExitCode::from(USAGE);
        }
    }
    if let Some([line, number, name]) = options.extract {
        let Some(at) = handle(line, tape.len()) else {
            eprintln!("grind calc: --extract: there is no line {line}");
            return ExitCode::from(USAGE);
        };
        if let Err(e) = tape.extract(at, number, options.nth, name) {
            eprintln!("grind calc: --extract: {e}");
            return ExitCode::from(REFUSED);
        }
    }
    if let Some(path) = options.save
        && let Err(message) = save(&tape, path, options.force)
    {
        eprintln!("grind calc: {message}");
        return ExitCode::from(REFUSED);
    }
    match options.format {
        Format::Json => println!("{}", json(&tape)),
        Format::Text if tape.len() == 1 && options.extract.is_none() => {
            println!("{}", tape.lines()[0].value);
        }
        Format::Text => print!("{}", rows(&tape)),
    }
    if let Some(circles) = tape.circles() {
        eprintln!("grind calc: {circles}");
    }
    ExitCode::SUCCESS
}

const HELP: &str = "\
A line is a formula without its leading =. #3 reads line 3; `vat = 0.19` names a line.
  :list                          show the tape
  :edit N <line>                 replace line N
  :insert N <line>               put a line before line N
  :del N                         delete line N
  :extract N <number> <name> [K] move a number in line N into a named line before it
  :undo  :redo
  :save <file> [!]               write the tape as a spreadsheet; ! overwrites
  :open <file> [!]               read a tape saved so back; ! drops an unsaved one
  :q                             quit";

fn interactive(tape: &mut Tape, input: impl BufRead) -> ExitCode {
    eprintln!("grind calc — :help for commands, :q to quit");
    let prompt = || {
        print!("> ");
        let _ = std::io::stdout().flush();
    };
    prompt();
    let mut saved = true;
    for line in input.lines().map_while(Result::ok) {
        let line = line.trim();
        match command(tape, line, &mut saved) {
            Step::Quit => return ExitCode::SUCCESS,
            Step::Show(text) => print!("{text}"),
            Step::Refused(message) => eprintln!("{message}"),
        }
        prompt();
    }
    if !saved && !tape.is_empty() {
        eprintln!("the tape was not saved");
    }
    ExitCode::SUCCESS
}

enum Step {
    Show(String),
    Refused(String),
    Quit,
}

fn command(tape: &mut Tape, line: &str, saved: &mut bool) -> Step {
    if line.is_empty() {
        return Step::Show(String::new());
    }
    let Some(rest) = line.strip_prefix(':') else {
        return match tape.push(line) {
            Ok(n) => {
                *saved = false;
                let shown = &tape.lines()[n - 1];
                Step::Show(format!("#{n}  {}\n", shown.value))
            }
            Err(e) => Step::Refused(e.to_string()),
        };
    };
    let (verb, args) = rest.split_once(' ').unwrap_or((rest, ""));
    let args = args.trim();
    let len = tape.len();
    let edited = |result: grind_sheet::Result<()>, tape: &Tape, saved: &mut bool| match result {
        Ok(()) => {
            *saved = false;
            Step::Show(rows(tape))
        }
        Err(e) => Step::Refused(e.to_string()),
    };
    match verb {
        "q" | "quit" if !*saved && len > 0 => {
            Step::Refused("the tape is not saved: :save <file>, or :q! to quit anyway".into())
        }
        "q" | "quit" | "q!" => Step::Quit,
        "help" | "h" => Step::Show(format!("{HELP}\n")),
        "list" | "l" => Step::Show(rows(tape)),
        "undo" | "u" => edited(
            tape.undo()
                .then_some(())
                .ok_or(grind_sheet::Error::Formula("nothing to undo".into())),
            tape,
            saved,
        ),
        "redo" => edited(
            tape.redo()
                .then_some(())
                .ok_or(grind_sheet::Error::Formula("nothing to redo".into())),
            tape,
            saved,
        ),
        "del" | "d" => match handle(args, len) {
            Some(at) => edited(tape.delete(at), tape, saved),
            None => Step::Refused(format!("there is no line {args}")),
        },
        "edit" | "e" | "insert" | "i" => {
            let (n, text) = args.split_once(' ').unwrap_or((args, ""));
            let limit = if verb.starts_with('i') { len + 1 } else { len };
            match handle(n, limit) {
                Some(at) if verb.starts_with('i') => edited(tape.insert(at, text), tape, saved),
                Some(at) => edited(tape.edit(at, text), tape, saved),
                None => Step::Refused(format!("there is no line {n}")),
            }
        }
        "extract" | "x" => {
            let words: Vec<&str> = args.split_whitespace().collect();
            let (n, number, name, nth) = match words.as_slice() {
                [n, number, name] => (n, number, name, None),
                [n, number, name, k] => (n, number, name, k.parse().ok()),
                _ => return Step::Refused(":extract N <number> <name> [K]".into()),
            };
            match handle(n, len) {
                Some(at) => edited(tape.extract(at, number, nth, name), tape, saved),
                None => Step::Refused(format!("there is no line {n}")),
            }
        }
        "save" | "w" | "promote" => {
            let (file, force) = match args.strip_suffix('!') {
                Some(file) => (file.trim(), true),
                None => (args, false),
            };
            if file.is_empty() {
                return Step::Refused(":save <file>".into());
            }
            match save(tape, &PathBuf::from(file), force) {
                Ok(()) => {
                    *saved = true;
                    Step::Show(format!("saved {file}\n"))
                }
                Err(message) => Step::Refused(message),
            }
        }
        "open" | "o" => {
            let (file, force) = match args.strip_suffix('!') {
                Some(file) => (file.trim(), true),
                None => (args, false),
            };
            if file.is_empty() {
                return Step::Refused(":open <file>".into());
            }
            if !*saved && len > 0 && !force {
                return Step::Refused(format!(
                    "the tape is not saved: :save <file>, or :open {file} ! to drop it"
                ));
            }
            match Tape::open_file(Path::new(file)) {
                Ok(opened) => {
                    *tape = opened;
                    *saved = true;
                    Step::Show(rows(tape))
                }
                Err(e) => Step::Refused(format!("{file}: {e}")),
            }
        }
        _ => Step::Refused(format!("no command :{verb}; :help lists them")),
    }
}

/// `#3` or `3`, 1-based, as a 0-based index below `limit`.
fn handle(text: &str, limit: usize) -> Option<usize> {
    let n: usize = text.trim().trim_start_matches('#').parse().ok()?;
    (1..=limit).contains(&n).then(|| n - 1)
}

fn save(tape: &Tape, path: &Path, force: bool) -> Result<(), String> {
    if path.exists() && !force {
        return Err(format!(
            "{} exists; --force (or :save {} !) overwrites it",
            path.display(),
            path.display()
        ));
    }
    tape.save(path).map_err(|e| e.to_string())
}

fn rows(tape: &Tape) -> String {
    let lines = tape.lines();
    let width = lines
        .iter()
        .map(|l| l.text.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for line in lines {
        let handle = format!("#{}", line.handle);
        let _ = writeln!(out, "{handle:<4} {:<width$}  {}", line.text, line.value);
    }
    out
}

fn json(tape: &Tape) -> String {
    let lines: Vec<serde_json::Value> = tape
        .lines()
        .into_iter()
        .map(|l| {
            serde_json::json!({
                "handle": l.handle,
                "name": l.name,
                "text": l.text,
                "value": l.value,
                "error": l.error,
            })
        })
        .collect();
    serde_json::json!({ "lines": lines, "cycles": tape.cycles() }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `:open` at the interactive tape: refused over unsaved lines without `!`, then the saved
    /// tape replaces them, and a later `:q` needs no save.
    #[test]
    fn open_replaces_the_tape_and_guards_unsaved_lines() {
        let path =
            std::env::temp_dir().join(format!("grind-calc-open-{}.fods", std::process::id()));
        let mut saved = Tape::new(None);
        saved.push("a = 2").unwrap();
        saved.push("a * 21").unwrap();
        saved.save(&path).unwrap();

        let mut tape = Tape::new(None);
        let mut clean = true;
        assert!(matches!(command(&mut tape, "5", &mut clean), Step::Show(_)));
        let open = format!(":open {}", path.display());
        assert!(matches!(
            command(&mut tape, &open, &mut clean),
            Step::Refused(_)
        ));
        let Step::Show(rows) = command(&mut tape, &format!("{open} !"), &mut clean) else {
            panic!("not opened");
        };
        assert!(rows.contains("a = 2") && rows.contains("42"), "{rows}");
        assert!(matches!(command(&mut tape, ":q", &mut clean), Step::Quit));
        let _ = std::fs::remove_file(path);
    }
}
