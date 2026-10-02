// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `--drive`'s script — what it says, parsed and checked with no Mac in sight (decision 9).
//!
//! A drive opens a real window and replays a script of synthesized events through
//! `NSApp.sendEvent`, so each one takes the path a real key takes: key equivalents and the menu
//! bar, the responder chain, `interpretKeyEvents:`. **The parser is portable** and this file is
//! all of it; only the replay (`app.rs`) needs AppKit. A script with a mistake in it fails here, on
//! its line, before a runner has spent a minute launching an application.
//!
//! One step a line; `#` starts a comment; blank lines are nothing.
//!
//! ```text
//! key cmd+b            a key, with its modifiers
//! type Total           text, a character at a time
//! click 120,40         a click at a point in the grid, in points from its top left
//! menu Format/Bold     a menu item, by its titles
//! wait 0.5             let the run loop turn for that many seconds
//! snap bold            the window as `bold.png` in the output directory
//! save                 File ▸ Save, and wait for it to land
//! mark ´               an input method's marked text, as `setMarkedText:` hands it over
//! commit é             an input method's commit, as `insertText:replacementRange:`
//! sidebar Data.C3      the sidebar row whose title holds this, chosen as a click chooses it
//! a11y                 what the key view tells VoiceOver, read back in-process and printed
//! ```
//!
//! `mark` and `commit` call the key view's `NSTextInputClient` methods directly, which is what an
//! input method does: the runner has no input method a script could select, and this proves the
//! protocol where it cannot prove the experience (*What the runner cannot speak for*).
//!
//! The transcript a drive prints, and the assertions made afterwards on the saved document through
//! the CLI, are what make it a test; the snapshots are for a person to look at.

use crate::menu::Mods;

/// One step of a drive.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Key(Stroke),
    /// Typed text, one key event per character.
    Type(String),
    /// A click in the grid view, in points from its top left.
    Click {
        x: f64,
        y: f64,
    },
    /// A menu item, by the titles from the menu bar down.
    Menu(Vec<String>),
    Wait(f64),
    /// A snapshot of the window, under this name.
    Snap(String),
    Save,
    /// Marked text, all of it the input method's selection's end — what a dead key leaves.
    Mark(String),
    /// Text an input method commits.
    Commit(String),
    /// The sidebar row whose title holds this text, chosen.
    Sidebar(String),
    /// The key view's accessibility attributes and the grid's last announcement, printed as an
    /// `a11y:` line — decision 10's floor, asserted rather than hoped for.
    Accessibility,
}

/// A key and its modifiers, as `NSEvent` wants them: the characters it produces and the
/// hardware key code of an ANSI keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stroke {
    pub mods: Mods,
    pub characters: String,
    pub code: u16,
}

/// The named keys a script may use, with the characters AppKit reports for each — a function key
/// is a private-use character from `NSEvent.h` — and its `kVK_` code from Carbon's `Events.h`.
const NAMED: &[(&str, &str, u16)] = &[
    ("return", "\r", 0x24),
    ("tab", "\t", 0x30),
    ("space", " ", 0x31),
    ("delete", "\u{7f}", 0x33),
    ("escape", "\u{1b}", 0x35),
    ("forwarddelete", "\u{f728}", 0x75),
    ("left", "\u{f702}", 0x7b),
    ("right", "\u{f703}", 0x7c),
    ("down", "\u{f701}", 0x7d),
    ("up", "\u{f700}", 0x7e),
    ("home", "\u{f729}", 0x73),
    ("end", "\u{f72b}", 0x77),
    ("pageup", "\u{f72c}", 0x74),
    ("pagedown", "\u{f72d}", 0x79),
    ("f1", "\u{f704}", 0x7a),
    ("f2", "\u{f705}", 0x78),
    ("f3", "\u{f706}", 0x63),
    ("f4", "\u{f707}", 0x76),
    ("f5", "\u{f708}", 0x60),
];

/// The ANSI keyboard's letters, digits and punctuation with their `kVK_ANSI_` codes.
const ANSI: &[(char, u16)] = &[
    ('a', 0x00),
    ('s', 0x01),
    ('d', 0x02),
    ('f', 0x03),
    ('h', 0x04),
    ('g', 0x05),
    ('z', 0x06),
    ('x', 0x07),
    ('c', 0x08),
    ('v', 0x09),
    ('b', 0x0b),
    ('q', 0x0c),
    ('w', 0x0d),
    ('e', 0x0e),
    ('r', 0x0f),
    ('y', 0x10),
    ('t', 0x11),
    ('1', 0x12),
    ('2', 0x13),
    ('3', 0x14),
    ('4', 0x15),
    ('6', 0x16),
    ('5', 0x17),
    ('=', 0x18),
    ('9', 0x19),
    ('7', 0x1a),
    ('-', 0x1b),
    ('8', 0x1c),
    ('0', 0x1d),
    (']', 0x1e),
    ('o', 0x1f),
    ('u', 0x20),
    ('[', 0x21),
    ('i', 0x22),
    ('p', 0x23),
    ('l', 0x25),
    ('j', 0x26),
    ('\'', 0x27),
    ('k', 0x28),
    (';', 0x29),
    ('\\', 0x2a),
    (',', 0x2b),
    ('/', 0x2c),
    ('n', 0x2d),
    ('m', 0x2e),
    ('.', 0x2f),
    ('`', 0x32),
    (' ', 0x31),
];

/// The stroke a character is typed with: its ANSI key when it has one, and code zero otherwise —
/// AppKit's text system reads the characters, and the code only matters to a key equivalent.
pub fn stroke_for(c: char) -> Stroke {
    let lower = c.to_ascii_lowercase();
    let code = ANSI
        .iter()
        .find(|(key, _)| *key == lower)
        .map_or(0, |(_, code)| *code);
    Stroke {
        mods: Mods {
            shift: c.is_ascii_uppercase(),
            ..Mods::default()
        },
        characters: c.to_string(),
        code,
    }
}

/// `cmd+shift+z`, `left`, `f2`, `a` — modifiers joined with `+`, the key last.
fn parse_key(spec: &str) -> Result<Stroke, String> {
    let mut mods = Mods::default();
    let mut parts: Vec<&str> = spec.split('+').collect();
    let key = parts.pop().filter(|key| !key.is_empty()).ok_or("no key")?;
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" => mods.command = true,
            "shift" => mods.shift = true,
            "opt" | "option" | "alt" => mods.option = true,
            "ctrl" | "control" => mods.control = true,
            other => return Err(format!("`{other}` is not a modifier")),
        }
    }
    let lower = key.to_ascii_lowercase();
    if let Some((_, characters, code)) = NAMED.iter().find(|(name, ..)| *name == lower) {
        return Ok(Stroke {
            mods,
            characters: (*characters).to_owned(),
            code: *code,
        });
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(Stroke {
            mods,
            ..stroke_for(c)
        }),
        _ => Err(format!("`{key}` is not a key")),
    }
}

/// A script, parsed — or the first line that is not a step, 1-based, and why.
pub fn parse(script: &str) -> Result<Vec<Step>, (usize, String)> {
    let mut steps = Vec::new();
    for (index, line) in script.lines().enumerate() {
        let line = line
            .split_once('#')
            .map_or(line, |(before, _)| before)
            .trim();
        if line.is_empty() {
            continue;
        }
        let fail = |message: String| (index + 1, message);
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        let step = match verb {
            "key" => Step::Key(parse_key(rest).map_err(fail)?),
            "type" if !rest.is_empty() => Step::Type(rest.to_owned()),
            "click" => {
                let (x, y) = rest
                    .split_once(',')
                    .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))
                    .ok_or_else(|| fail(format!("`{rest}` is not a point like 120,40")))?;
                Step::Click { x, y }
            }
            "menu" if !rest.is_empty() => Step::Menu(
                rest.split('/')
                    .map(|title| title.trim().to_owned())
                    .collect(),
            ),
            "wait" => Step::Wait(
                rest.parse::<f64>()
                    .ok()
                    .filter(|seconds| (0.0..=60.0).contains(seconds))
                    .ok_or_else(|| fail(format!("`{rest}` is not a number of seconds")))?,
            ),
            "snap" if is_name(rest) => Step::Snap(rest.to_owned()),
            "snap" => return Err(fail(format!("`{rest}` is not a snapshot name"))),
            "save" if rest.is_empty() => Step::Save,
            "mark" if !rest.is_empty() => Step::Mark(rest.to_owned()),
            "commit" if !rest.is_empty() => Step::Commit(rest.to_owned()),
            "sidebar" if !rest.is_empty() => Step::Sidebar(rest.to_owned()),
            "a11y" if rest.is_empty() => Step::Accessibility,
            other => return Err(fail(format!("`{other}` is not a step"))),
        };
        steps.push(step);
    }
    Ok(steps)
}

/// A snapshot's name becomes a file name, so it is letters, digits, `-` and `_` only.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(target_os = "macos")]
pub use mac::replay;

thread_local! {
    /// What the views were told since a transcript last asked — `keyDown:`, the selector the
    /// text system answered with, the text it inserted — so a key that changed nothing says
    /// where it stopped.
    static HEARD: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A view was told `what`: kept for the next transcript line. Costs one push outside a drive.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn hear(what: impl Into<String>) {
    HEARD.with(|heard| {
        let mut heard = heard.borrow_mut();
        // Bounded, so a long session that is never driven does not keep growing it.
        if heard.len() < 64 {
            heard.push(what.into());
        }
    });
}

/// Everything heard since the last call, and forget it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn heard() -> String {
    HEARD.with(|heard| heard.borrow_mut().drain(..).collect::<Vec<_>>().join(" "))
}

/// The replay — synthesized events through `NSApp.sendEvent`, so each takes the path a real one
/// takes, in-process and so with no Accessibility or Input Monitoring permission.
#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObjectProtocol};
    use objc2::{MainThreadMarker, MainThreadOnly, msg_send, sel};
    use objc2_app_kit::{
        NSApplication, NSBitmapImageFileType, NSDocument, NSDocumentController, NSEvent,
        NSEventMask, NSEventModifierFlags, NSEventType, NSMenu, NSMenuItem, NSResponder, NSView,
        NSWindow,
    };
    use objc2_foundation::{
        NSDate, NSDefaultRunLoopMode, NSDictionary, NSPoint, NSProcessInfo, NSRange, NSString,
    };

    use super::{Step, Stroke, parse, stroke_for};
    use crate::args::Drive;
    use crate::document::Document;
    use crate::text::input::NOT_FOUND;

    /// Let the run loop turn for `seconds`, dispatching whatever arrives — how a drive waits for
    /// a window to draw, a menu action to land or a save to finish.
    fn pump(app: &NSApplication, seconds: f64) {
        let until = NSDate::dateWithTimeIntervalSinceNow(seconds);
        // SAFETY: the default run-loop mode is a constant Foundation exports.
        let mode = unsafe { NSDefaultRunLoopMode };
        while let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::Any,
            Some(&until),
            mode,
            true,
        ) {
            app.sendEvent(&event);
        }
    }

    /// The modifiers a real keyboard would have set: the stroke's own, and for a key from
    /// AppKit's function-key range (U+F700–U+F8FF) the Function flag — with NumericPad as well
    /// for the four arrows, which is what the hardware sends for them.
    fn flags(stroke: &Stroke) -> NSEventModifierFlags {
        let mut flags = NSEventModifierFlags::empty();
        if let Some(c) = stroke.characters.chars().next()
            && ('\u{f700}'..='\u{f8ff}').contains(&c)
        {
            flags |= NSEventModifierFlags::Function;
            if ('\u{f700}'..='\u{f703}').contains(&c) {
                flags |= NSEventModifierFlags::NumericPad;
            }
        }
        for (on, flag) in [
            (stroke.mods.command, NSEventModifierFlags::Command),
            (stroke.mods.shift, NSEventModifierFlags::Shift),
            (stroke.mods.option, NSEventModifierFlags::Option),
            (stroke.mods.control, NSEventModifierFlags::Control),
        ] {
            if on {
                flags |= flag;
            }
        }
        flags
    }

    /// The window a step acts on: the key window, or failing that the first one there is.
    fn window(app: &NSApplication) -> Result<Retained<NSWindow>, String> {
        app.keyWindow()
            .or_else(|| app.windows().firstObject())
            .ok_or_else(|| "there is no window".to_owned())
    }

    /// The item a ⌘-key is the key equivalent of, anywhere in `menu`.
    fn equivalent(menu: &NSMenu, stroke: &Stroke) -> Option<Retained<NSMenuItem>> {
        let modifiers = NSEventModifierFlags::Command
            | NSEventModifierFlags::Shift
            | NSEventModifierFlags::Option
            | NSEventModifierFlags::Control;
        for item in menu.itemArray().iter() {
            if let Some(submenu) = item.submenu() {
                if let Some(found) = equivalent(&submenu, stroke) {
                    return Some(found);
                }
                continue;
            }
            let key = item.keyEquivalent().to_string();
            if !key.is_empty()
                && key == stroke.characters.to_lowercase()
                && item.keyEquivalentModifierMask() & modifiers == flags(stroke) & modifiers
            {
                return Some(item);
            }
        }
        None
    }

    /// Send `item`'s action to `target` if it answers it — validated first, as the menu would
    /// have validated it. `Ok(false)` when it does not answer it.
    fn deliver(app: &NSApplication, target: &AnyObject, item: &NSMenuItem) -> Result<bool, String> {
        let Some(action) = item.action() else {
            return Err(format!("{} has no action", item.title()));
        };
        if !target.class().responds_to(action) {
            return Ok(false);
        }
        if target.class().responds_to(sel!(validateMenuItem:)) {
            // SAFETY: the target answers `validateMenuItem:`, which takes an item and answers a
            // BOOL.
            let enabled: bool = unsafe { msg_send![target, validateMenuItem: item] };
            if !enabled {
                return Err(format!("{} is disabled here", item.title()));
            }
        }
        // SAFETY: the target answers the action, which takes one sender.
        unsafe { app.sendAction_to_from(action, Some(target), Some(item)) };
        Ok(true)
    }

    /// `item`'s action, sent where AppKit would have sent it were `window` key.
    ///
    /// A runner's window never becomes key (*Evidence*), and an action with no target starts at
    /// the key window — so a ⌘-key a drive pressed there reached nothing: Copy copied nothing and
    /// Save saved nothing. This walks AppKit's own documented path for a document application
    /// from `window` instead: the first responder up through its views to the window, the
    /// window's delegate, its controller and the controller's document, then the application,
    /// its delegate and the document controller. Used only when the window is not key; a key
    /// window gets the real path.
    fn send(app: &NSApplication, window: &NSWindow, item: &NSMenuItem) -> Result<(), String> {
        let mut responder = window.firstResponder();
        while let Some(current) = responder {
            if deliver(app, &current, item)? {
                return Ok(());
            }
            // SAFETY: an ordinary read of the responder chain.
            responder = unsafe { current.nextResponder() };
        }
        if let Some(delegate) = window.delegate()
            && deliver(app, delegate.as_ref(), item)?
        {
            return Ok(());
        }
        if let Some(controller) = window.windowController() {
            if deliver(app, &controller, item)? {
                return Ok(());
            }
            // SAFETY: an ordinary read of the controller's document.
            if let Some(document) = unsafe { controller.document() }
                && deliver(app, &document, item)?
            {
                return Ok(());
            }
        }
        if deliver(app, app, item)? {
            return Ok(());
        }
        if let Some(delegate) = app.delegate()
            && deliver(app, delegate.as_ref(), item)?
        {
            return Ok(());
        }
        let controller = NSDocumentController::sharedDocumentController(app.mtm());
        match deliver(app, &controller, item)? {
            true => Ok(()),
            false => Err(format!("nothing answers {}", item.title())),
        }
    }

    fn key(app: &NSApplication, stroke: &Stroke) -> Result<(), String> {
        let window = window(app)?;
        // A key equivalent, when the window cannot be key to receive it the real way.
        if stroke.mods.command
            && !window.isKeyWindow()
            && let Some(item) = app.mainMenu().and_then(|menu| equivalent(&menu, stroke))
        {
            return send(app, &window, &item);
        }
        let characters = NSString::from_str(&stroke.characters);
        // A real event's timestamp is the system's uptime; one at zero is older than anything
        // the text-input system has seen, and an active application's input context may drop
        // it as stale.
        let now = NSProcessInfo::processInfo().systemUptime();
        for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind,
                NSPoint::new(0.0, 0.0),
                flags(stroke),
                now,
                window.windowNumber(),
                None,
                &characters,
                &characters,
                false,
                stroke.code,
            )
            .ok_or("NSEvent would not build a key event")?;
            app.sendEvent(&event);
        }
        Ok(())
    }

    fn click(
        app: &NSApplication,
        document: Option<&NSDocument>,
        x: f64,
        y: f64,
    ) -> Result<(), String> {
        let window = window(app)?;
        let (view, (dx, dy)) = document
            .and_then(|document| document.downcast_ref::<Document>())
            .and_then(Document::click_target)
            .ok_or("this window has no grid and no page")?;
        let at = view.convertPoint_toView(NSPoint::new(x + dx, y + dy), None);
        for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
            let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                at,
                NSEventModifierFlags::empty(),
                0.0,
                window.windowNumber(),
                None,
                0,
                1,
                1.0,
            )
            .ok_or("NSEvent would not build a mouse event")?;
            app.sendEvent(&event);
        }
        Ok(())
    }

    fn menu(app: &NSApplication, path: &[String]) -> Result<(), String> {
        let mut menu = app.mainMenu().ok_or("there is no menu bar")?;
        for (at, title) in path.iter().enumerate() {
            let item = menu
                .itemWithTitle(&NSString::from_str(title))
                .ok_or_else(|| format!("no menu item {title}"))?;
            if at + 1 == path.len() {
                let window = window(app)?;
                if !window.isKeyWindow() {
                    return send(app, &window, &item);
                }
                menu.performActionForItemAtIndex(menu.indexOfItem(&item));
                return Ok(());
            }
            menu = item
                .submenu()
                .ok_or_else(|| format!("{title} is not a menu"))?;
        }
        Err("a menu step names at least one item".into())
    }

    /// The key view, when it takes text from an input method — the page does; the grid does not.
    fn input_client(app: &NSApplication) -> Result<Retained<NSResponder>, String> {
        let responder = window(app)?
            .firstResponder()
            .ok_or("nothing has the keyboard")?;
        match responder.respondsToSelector(sel!(setMarkedText:selectedRange:replacementRange:)) {
            true => Ok(responder),
            false => Err("what has the keyboard takes no input method's text".into()),
        }
    }

    /// `setMarkedText:selectedRange:replacementRange:`, with the input method's caret after the
    /// marked text and nothing to replace.
    fn mark(app: &NSApplication, text: &str) -> Result<(), String> {
        let client = input_client(app)?;
        let text = NSString::from_str(text);
        let end = text.length();
        // SAFETY: the responder answers this selector (asked above), which takes an object and
        // two ranges and returns nothing.
        unsafe {
            let _: () = msg_send![&*client, setMarkedText: &*text, selectedRange: NSRange::new(end, 0), replacementRange: NSRange::new(NOT_FOUND, 0)];
        }
        Ok(())
    }

    /// `insertText:replacementRange:`, replacing nothing — an input method's commit.
    fn commit(app: &NSApplication, text: &str) -> Result<(), String> {
        let client = input_client(app)?;
        let text = NSString::from_str(text);
        // SAFETY: as above; this selector takes an object and a range.
        unsafe {
            let _: () = msg_send![&*client, insertText: &*text, replacementRange: NSRange::new(NOT_FOUND, 0)];
        }
        Ok(())
    }

    /// What the key view answers the questions a screen reader asks, and what the grid last
    /// announced — one line, for a transcript.
    fn accessibility(app: &NSApplication, document: Option<&NSDocument>) -> Result<String, String> {
        let responder = window(app)?
            .firstResponder()
            .ok_or("nothing has the keyboard")?;
        // SAFETY: every responder that is a view answers the informal NSAccessibility
        // protocol these four selectors are from, with these return types.
        let (role, value, range, line) = unsafe {
            let role: Option<Retained<NSString>> = msg_send![&*responder, accessibilityRole];
            let value: Option<Retained<AnyObject>> = msg_send![&*responder, accessibilityValue];
            let range: NSRange = msg_send![&*responder, accessibilitySelectedTextRange];
            let line: isize = msg_send![&*responder, accessibilityInsertionPointLineNumber];
            (role, value, range, line)
        };
        let value = value
            .and_then(|value| value.downcast::<NSString>().ok())
            .map(|value| value.to_string());
        let announced = document
            .and_then(|document| document.downcast_ref::<Document>())
            .and_then(Document::announced)
            .unwrap_or_default();
        Ok(format!(
            "a11y: role {:?}, value {:?}, selected {}+{}, line {line}, announced {announced:?}",
            role.map(|role| role.to_string()).unwrap_or_default(),
            value.unwrap_or_default(),
            range.location,
            range.length,
        ))
    }

    /// The window, frame and all, as a PNG — for a person to look at; the assertions are made on
    /// the saved document. Caching a view into a bitmap needs no Screen Recording permission.
    fn snap(app: &NSApplication, drive: &Drive, name: &str) -> Result<(), String> {
        let window = window(app)?;
        let content = window
            .contentView()
            .ok_or("the window has no content view")?;
        // SAFETY: `superview` is an ordinary read of the view hierarchy.
        let frame: Retained<NSView> = unsafe { content.superview() }.unwrap_or(content);
        let bounds = frame.bounds();
        let rep = frame
            .bitmapImageRepForCachingDisplayInRect(bounds)
            .ok_or("the window would not cache into a bitmap")?;
        frame.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
        // SAFETY: an empty property dictionary is the documented "no options".
        let png = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .ok_or("no PNG of the window")?;
        let path = drive.out.join(format!("{name}.png"));
        std::fs::write(&path, png.to_vec()).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// One line of what the drive left behind after a step: the window, whether the document is
    /// edited, and where the selection is — what M3's drives assert on.
    fn transcript(app: &NSApplication, document: Option<&NSDocument>) -> String {
        let title = window(app).map_or_else(|_| "no window".to_owned(), |w| w.title().to_string());
        let edited = document.is_some_and(|document| document.isDocumentEdited());
        let selection = document
            .and_then(|document| document.downcast_ref::<Document>())
            .map_or_else(|| "no grid".to_owned(), Document::selection_text);
        let heard = super::heard();
        format!("window {title:?}, edited {edited}, heard [{heard}], selection {selection}")
    }

    /// Which object has the keyboard — its class's name, for the first line of a transcript,
    /// since a key that changed nothing is otherwise silent about where it went.
    fn responder(app: &NSApplication) -> String {
        window(app)
            .ok()
            .and_then(|window| window.firstResponder())
            .map_or_else(
                || "nothing".to_owned(),
                |responder| responder.class().name().to_string_lossy().into_owned(),
            )
    }

    /// Replay `drive`'s script against the application `launch` just opened, and answer the
    /// process's exit code: zero when every step ran.
    pub fn replay(mtm: MainThreadMarker, drive: &Drive, document: Option<&NSDocument>) -> i32 {
        let script = match std::fs::read_to_string(&drive.script) {
            Ok(script) => script,
            Err(error) => {
                eprintln!("grind-mac: {}: {error}", drive.script.display());
                return 2;
            }
        };
        let steps = match parse(&script) {
            Ok(steps) => steps,
            Err((line, message)) => {
                eprintln!("grind-mac: {}:{line}: {message}", drive.script.display());
                return 2;
            }
        };
        if let Err(error) = std::fs::create_dir_all(&drive.out) {
            eprintln!("grind-mac: {}: {error}", drive.out.display());
            return 2;
        }
        let app = NSApplication::sharedApplication(mtm);
        app.activate();
        // Deprecated, and still the one call that may take focus from a runner's own session;
        // a window that cannot be key is driven anyway (`send`), and the first line says which.
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        if let Ok(window) = window(&app) {
            window.makeKeyAndOrderFront(None);
        }
        pump(&app, 1.0);
        let is_key = window(&app).is_ok_and(|window| window.isKeyWindow());
        println!(
            "start: {}, active {}, key window {is_key}, keyboard {}",
            transcript(&app, document),
            app.isActive(),
            responder(&app)
        );
        for (at, step) in steps.iter().enumerate() {
            let done = match step {
                Step::Key(stroke) => key(&app, stroke),
                Step::Type(text) => text.chars().try_for_each(|c| key(&app, &stroke_for(c))),
                Step::Click { x, y } => click(&app, document, *x, *y),
                Step::Menu(path) => menu(&app, path),
                Step::Wait(seconds) => {
                    pump(&app, *seconds);
                    Ok(())
                }
                Step::Snap(name) => snap(&app, drive, name),
                Step::Mark(text) => mark(&app, text),
                Step::Commit(text) => commit(&app, text),
                Step::Accessibility => accessibility(&app, document).map(|line| println!("{line}")),
                Step::Sidebar(text) => document
                    .and_then(|document| document.downcast_ref::<Document>())
                    .ok_or_else(|| "there is no document".to_owned())
                    .and_then(|document| document.choose_place(text)),
                Step::Save => match document {
                    // SAFETY: `saveDocument:` is an action, and a nil sender is allowed.
                    Some(document) => {
                        unsafe { document.saveDocument(None) };
                        Ok(())
                    }
                    None => Err("there is no document to save".into()),
                },
            };
            // Every step lets the run loop turn a little, so what it caused has happened before
            // the next one — and before the transcript says what the step left behind.
            pump(&app, 0.2);
            println!("step {}: {step:?} → {}", at + 1, transcript(&app, document));
            if let Err(message) = done {
                eprintln!("grind-mac: step {}: {message}", at + 1);
                return 1;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{CMD, SHIFT_CMD};

    #[test]
    fn a_script_is_one_step_a_line() {
        let script = "\
            # make the label bold\n\
            click 120,40\n\
            key cmd+b\n\
            \n\
            type Total   # and retype it\n\
            menu Format/Bold\n\
            wait 0.5\n\
            snap bold\n\
            save\n";
        let steps = parse(script).unwrap();
        assert_eq!(steps.len(), 7);
        assert_eq!(steps[0], Step::Click { x: 120.0, y: 40.0 });
        assert_eq!(
            steps[1],
            Step::Key(Stroke {
                mods: CMD,
                characters: "b".into(),
                code: 0x0b
            })
        );
        assert_eq!(steps[2], Step::Type("Total".into()));
        assert_eq!(steps[3], Step::Menu(vec!["Format".into(), "Bold".into()]));
        assert_eq!(steps[4], Step::Wait(0.5));
        assert_eq!(steps[5], Step::Snap("bold".into()));
        assert_eq!(steps[6], Step::Save);
    }

    #[test]
    fn named_keys_are_the_characters_appkit_reports() {
        let Ok(Step::Key(left)) = parse("key left").map(|mut s| s.remove(0)) else {
            panic!()
        };
        assert_eq!((left.characters.as_str(), left.code), ("\u{f702}", 0x7b));
        let Ok(Step::Key(undo)) = parse("key cmd+shift+z").map(|mut s| s.remove(0)) else {
            panic!()
        };
        assert_eq!(undo.mods, SHIFT_CMD);
        assert_eq!(undo.code, 0x06);
    }

    #[test]
    fn an_input_method_marks_and_commits() {
        assert_eq!(
            parse("mark \u{b4}\ncommit \u{e9}").unwrap(),
            [Step::Mark("\u{b4}".into()), Step::Commit("\u{e9}".into())]
        );
        assert_eq!(stroke_for(' ').code, 0x31, "the space bar");
    }

    #[test]
    fn a_capital_is_typed_with_shift() {
        let stroke = stroke_for('T');
        assert!(stroke.mods.shift);
        assert_eq!((stroke.characters.as_str(), stroke.code), ("T", 0x11));
        assert_eq!(
            stroke_for('€').code,
            0,
            "no ANSI key, and AppKit reads the text"
        );
    }

    /// A mistake fails on its own line, before a runner launches anything.
    #[test]
    fn a_mistake_names_its_line() {
        assert_eq!(parse("key cmd+b\nclick here").unwrap_err().0, 2);
        assert_eq!(parse("frobnicate").unwrap_err().0, 1);
        assert!(parse("key hyper+b").unwrap_err().1.contains("modifier"));
        assert!(parse("key banana").is_err());
        assert!(parse("wait forever").is_err());
        assert!(
            parse("wait 600").is_err(),
            "a drive is not allowed to sleep"
        );
        assert!(parse("snap ../escape").is_err(), "a name, not a path");
        assert!(parse("save now").is_err());
        assert!(parse("type").is_err());
        assert!(parse("mark").is_err(), "marked text is some text");
        assert!(parse("sidebar").is_err(), "a row is named by some text");
        assert_eq!(parse("a11y").unwrap(), [Step::Accessibility]);
        assert!(parse("a11y now").is_err());
        assert_eq!(
            parse("sidebar Data.C3").unwrap(),
            [Step::Sidebar("Data.C3".into())]
        );
        assert_eq!(parse("# only a comment\n\n").unwrap(), vec![]);
    }
}
