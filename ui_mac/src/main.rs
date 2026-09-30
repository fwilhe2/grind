// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind-mac` — the macOS shell over the suite, shipped as `Grind.app`.
//!
//! **M2: the application, the document and the read-only grid.** `doc/macos-shell.md` is
//! normative for this crate and is where the decisions, the milestones and the named gaps live. It is written
//! so that as much as possible of this shell can be built and tested on Linux, because the
//! machine this repository is developed on is not a Mac:
//!
//! * Everything that can be a pure function is one, and compiles and tests on any host. That is
//!   the argument parsing here, and from M2 the menus, the selector tables, the edit modes and
//!   the Info.plist.
//! * Everything that needs AppKit is behind `cfg(target_os = "macos")`. The Linux machine
//!   type-checks and lints it with `--target aarch64-apple-darwin` (nothing links, so no SDK is
//!   needed), and `artifacts.yml`'s `macos` job is the one place it links and runs.
//!
//! This file owns the process: it resolves the command line into what to open, and hands that to
//! `--render-to`'s windowless frame or to `app.rs`'s window. Off a Mac it says what a window would
//! have done, which is what makes the argument handling runnable there rather than only testable —
//! the same thing `ui_win32`'s W0 did.

// Portable, and reached from the AppKit half — which is not compiled off macOS, so on Linux the
// only callers of much of this are the tests.
#[cfg(target_os = "macos")]
mod accessory;
#[cfg(target_os = "macos")]
mod app;
mod args;
#[cfg(target_os = "macos")]
mod banner;
#[cfg(target_os = "macos")]
mod clipboard;
#[cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]
mod document;
#[cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]
mod drive;
#[cfg(target_os = "macos")]
mod editor;
#[cfg(target_os = "macos")]
mod find_bar;
#[cfg(target_os = "macos")]
mod grid_view;
#[cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]
mod import;
#[cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]
mod keys;
#[cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]
mod menu;
#[cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]
mod metrics;
#[cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]
mod notice;
#[cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]
mod ops;
#[cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]
mod png;
#[cfg(target_os = "macos")]
mod render;
#[cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]
mod sheet;
#[cfg(target_os = "macos")]
mod sidebar;
#[cfg(target_os = "macos")]
mod watch;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::{Command, Drive};
use grind_core::DocumentKind;

fn version() -> String {
    format!(
        "grind-mac {}",
        grind_core::build_info::describe_version(env!("CARGO_PKG_VERSION"))
    )
}

/// What kind of document a file holds, read from its bytes — the rule the document controller
/// answers `typeForContentsOfURL:error:` with (`document::sniff`), asked here before there is a
/// controller to ask.
fn sniff(path: &Path) -> Result<DocumentKind, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    document::sniff(&path.display().to_string(), &bytes)
}

/// What this invocation opens, resolved from the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opening {
    /// `None` because **"no document" is an answer**: the welcome window (decision 6).
    pub kind: Option<DocumentKind>,
    pub path: Option<PathBuf>,
    /// A render: where the frame goes, and whether it is drawn dark.
    pub render: Option<(PathBuf, bool)>,
    pub drive: Option<Drive>,
}

/// Resolve the command line into what a window would open, or a message saying why it cannot.
/// The whole of the decision and none of the platform, so it is tested on any host.
fn resolve(command: Command) -> Result<Opening, String> {
    let Command::Open {
        kind,
        path,
        render_to,
        dark,
        drive,
    } = command
    else {
        unreachable!("help, version and errors are handled before this")
    };
    let render = render_to.map(|target| (target, dark));
    let kind = match &path {
        Some(file) => {
            let found = sniff(file)?;
            Some(grind_core::kind::reconcile(
                kind,
                found,
                &file.display().to_string(),
            )?)
        }
        None => kind,
    };
    Ok(Opening {
        kind,
        path,
        render,
        drive,
    })
}

/// What a window would do with `opening`, in a sentence — the first line of a drive's transcript,
/// and what this binary says off a Mac.
pub fn intent(opening: &Opening) -> String {
    let what = match (opening.kind, &opening.path) {
        (Some(kind), Some(path)) => {
            format!("open {} as a {}", path.display(), kind.label())
        }
        (Some(kind), None) => format!("start an empty {}", kind.label()),
        (None, _) => "show the welcome window".to_owned(),
    };
    match &opening.render {
        Some((target, dark)) => format!(
            "{what}, and draw it to {} in the {} appearance",
            target.display(),
            if *dark { "dark" } else { "light" }
        ),
        None => what,
    }
}

/// The size of a `--render-to` frame in points, and how many pixels a point is — a Retina screen's
/// two, which is what somebody looking at the window sees.
#[cfg(target_os = "macos")]
const FRAME: (f64, f64) = (1024.0, 640.0);
#[cfg(target_os = "macos")]
const SCALE: f64 = 2.0;

/// Draw one frame of the document `opening` names into the PNG it names, with no window — decision
/// 9's `--render-to`, through the same `sheet::paint` and `render::draw` a view calls.
///
/// The palette is the fixed light or dark one rather than the system's current colours, so a
/// frame is a function of its command line alone and two renders are the same bytes.
#[cfg(target_os = "macos")]
fn render_to(opening: &Opening) -> Result<(), String> {
    use sheet::paint::{self, Look, Palette};

    let Some((target, dark)) = &opening.render else {
        unreachable!("only a render reaches here")
    };
    match opening.kind {
        Some(DocumentKind::Spreadsheet) => {
            let app = grind_sheet::App::new();
            if let Some(path) = &opening.path {
                let bytes =
                    std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
                let opened = import::open(&path.display().to_string(), &bytes)?;
                app.open_bytes(&opened.name, &opened.bytes)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
            }
            let grid = sheet::geom::Grid::of(&app, 0);
            let text = metrics::CoreText::new(metrics::BASE_PT);
            let palette = match dark {
                true => Palette::DARK,
                false => Palette::LIGHT,
            };
            let look = Look {
                palette: &palette,
                metrics: &text,
                hairline: 1.0 / SCALE,
            };
            // The cursor where a window opens with it, at A1, so a frame shows what the window
            // would.
            let selection = grind_sheet::nav::Selection::default();
            let ops = paint::frame(&app, 0, &grid, FRAME, (0.0, 0.0), selection, &look);
            let (w, h, rgba) = render::bitmap(FRAME.0, FRAME.1, SCALE, |context| {
                render::draw(context, &ops, &text)
            })?;
            std::fs::write(target, png::encode(w, h, &rgba))
                .map_err(|error| format!("{}: {error}", target.display()))
        }
        Some(DocumentKind::Text) => Err("a text document's page is drawn from M6 on".into()),
        _ => Err("the welcome window is drawn from M9 on; name a document or --sheet".into()),
    }
}

/// Off a Mac there is nothing to draw with: CoreGraphics and CoreText are the renderer.
#[cfg(not(target_os = "macos"))]
fn render_to(_: &Opening) -> Result<(), String> {
    Err("frames are drawn by CoreGraphics, on macOS only".into())
}

/// Open the window — the application, its menu bar and the document — and run until it quits.
#[cfg(target_os = "macos")]
fn window(opening: Opening) -> ExitCode {
    app::run(opening)
}

/// Off a Mac there is no window to open: say what one would have done, and fail, so no script
/// mistakes this for a shell that did what it was asked.
#[cfg(not(target_os = "macos"))]
fn window(opening: Opening) -> ExitCode {
    eprintln!(
        "grind-mac opens windows on macOS only. There it would {}.",
        intent(&opening)
    );
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    match args::parse(std::env::args().skip(1)) {
        Command::Help => {
            print!("{}", args::USAGE);
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("{}", version());
            ExitCode::SUCCESS
        }
        Command::Error(message) => {
            eprintln!("grind-mac: {message}\n\n{}", args::USAGE);
            ExitCode::from(2)
        }
        open => match resolve(open) {
            Err(message) => {
                eprintln!("grind-mac: {message}");
                ExitCode::FAILURE
            }
            Ok(opening) if opening.render.is_some() => match render_to(&opening) {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    eprintln!("grind-mac: --render-to: {message}");
                    ExitCode::FAILURE
                }
            },
            Ok(opening) => window(opening),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(path: Option<&str>, kind: Option<DocumentKind>) -> Command {
        Command::Open {
            kind,
            path: path.map(PathBuf::from),
            render_to: None,
            dark: false,
            drive: None,
        }
    }

    fn opening(kind: Option<DocumentKind>, path: Option<&str>) -> Opening {
        Opening {
            kind,
            path: path.map(PathBuf::from),
            render: None,
            drive: None,
        }
    }

    #[test]
    fn an_empty_invocation_names_no_document() {
        assert_eq!(resolve(open(None, None)).unwrap(), opening(None, None));
        assert_eq!(intent(&opening(None, None)), "show the welcome window");
    }

    #[test]
    fn the_flag_decides_when_there_is_no_file() {
        assert_eq!(
            resolve(open(None, Some(DocumentKind::Text))).unwrap(),
            opening(Some(DocumentKind::Text), None)
        );
    }

    /// The type comes out of the bytes, not the name — so a text document called `.fods` still
    /// opens as a text document.
    #[test]
    fn the_bytes_decide_and_not_the_extension() {
        let dir = std::env::temp_dir().join(format!("grind-mac-args-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lying = dir.join("actually-a-document.fods");
        let bytes =
            grind_text::write_bytes(&grind_text::Document::default(), grind_core::Form::Flat)
                .expect("a default document writes");
        std::fs::write(&lying, bytes).unwrap();

        let resolved = resolve(open(Some(lying.to_str().unwrap()), None)).unwrap();
        assert_eq!(
            resolved.kind,
            Some(DocumentKind::Text),
            "the name says sheet, the bytes say text"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_is_reported_rather_than_invented() {
        let error = resolve(open(Some("no-such-file-here.fods"), None)).unwrap_err();
        assert!(error.contains("no-such-file-here.fods"), "{error}");
    }

    #[test]
    fn a_render_says_where_and_in_which_appearance() {
        let opening = Opening {
            render: Some((PathBuf::from("frame.png"), true)),
            ..opening(Some(DocumentKind::Spreadsheet), Some("book.fods"))
        };
        assert_eq!(
            intent(&opening),
            "open book.fods as a spreadsheet, and draw it to frame.png in the dark appearance"
        );
    }
}
