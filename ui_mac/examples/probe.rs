// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! M0's probe: the four facts `doc/macos-shell.md` rests on, measured on a real Mac.
//!
//! **Not the shell, and never shipped.** It's an example so that `cargo build` never builds it.
//! It is also type-checked and linted from Linux with the rest of the crate
//! (`--all-targets`), which is how it was written: on a machine that has never run it.
//! `macos.yml`'s `probe` job runs it on `macos-15` and `macos-26` and uploads what it writes,
//! and the answers go into the doc's *Evidence* table. Each one decides something:
//!
//! 1. **Headless drawing.** CoreGraphics and CoreText draw into a bitmap with no application
//!    and no window. Decision 9's `--render-to` is only possible if they do, and only useful if
//!    two renders are the same bytes.
//! 2. **What CoreText measures.** The caret offset at every UTF-16 index of a precomposed `é`,
//!    a decomposed one, an emoji family and a Devanagari conjunct. Decision 4 claims a shaping
//!    engine closes GDI's gaps. These numbers are that claim checked.
//! 3. **A window, snapshotted.** Whether the runner has a GUI session at all. If it does,
//!    whether a window's own frame view can be cached into a bitmap in-process, which needs no
//!    Screen Recording permission.
//! 4. **Synthesized events.** Whether key events built with `NSEvent` and handed to
//!    `NSApp.sendEvent` reach a text field's field editor. `--drive` is built on that.
//!
//! Every answer is a `key: value` line on stdout, so a failed measurement is still a
//! measurement.
//!
//! ```sh
//! cargo run -p grind-mac --example probe -- /tmp/probe
//! ```

#[cfg(target_os = "macos")]
mod mac {
    use std::path::Path;
    use std::ptr::null_mut;

    use objc2::rc::Retained;
    use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapImageFileType,
        NSBitmapImageRep, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSScreen,
        NSTextField, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_core_foundation::{
        CFAttributedString, CFDictionary, CFIndex, CFRetained, CFString, CFType, CGFloat, CGPoint,
        CGRect, CGSize,
    };
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext,
        CGImageAlphaInfo, kCGColorSpaceSRGB,
    };
    use objc2_core_text::{CTFont, CTFontUIFontType, CTLine, kCTFontAttributeName};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSDictionary, NSPoint, NSRect, NSString};

    /// The strings decision 4 is about, each with what it is.
    ///
    /// The first two draw the same letter. The question is whether the decomposed one is
    /// measured as one cluster, with its combining mark adding no width, or as two boxes, which
    /// is what GDI does.
    const SAMPLES: [(&str, &str); 5] = [
        ("latin", "Grind"),
        ("precomposed", "\u{e9}"),
        ("decomposed", "e\u{301}"),
        ("zwj-family", "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"),
        ("devanagari", "\u{915}\u{94d}\u{937}\u{93f}"),
    ];

    const W: usize = 640;
    const H: usize = 240;

    pub fn run(out: &Path) -> Result<(), String> {
        std::fs::create_dir_all(out).map_err(|error| format!("{}: {error}", out.display()))?;
        let font = system_font(24.0);

        // 2. What CoreText measures.
        //
        // Font reports family and full name separately. On macOS 26 the family is sometimes
        // `.AppleSystemUIFont`, which is the system font under its private name.
        println!("font-family: {}", unsafe { font.family_name() });
        for (name, text) in SAMPLES {
            let line = line(text, &font);
            let units = text.encode_utf16().count();
            let offsets: Vec<String> = (0..=units)
                .map(|index| {
                    // SAFETY: a null secondary offset is documented as allowed.
                    let x = unsafe { line.offset_for_string_index(index as CFIndex, null_mut()) };
                    format!("{x:.2}")
                })
                .collect();
            // SAFETY: null out-pointers are documented as allowed; only the width is wanted.
            let width = unsafe { line.typographic_bounds(null_mut(), null_mut(), null_mut()) };
            println!(
                "coretext {name}: chars={} utf16={units} width={width:.2} offsets=[{}]",
                text.chars().count(),
                offsets.join(", ")
            );
        }

        // 1. Headless drawing, twice.
        let first = headless(&font)?;
        let second = headless(&font)?;
        println!("headless-bytes: {}", first.len());
        println!("headless-identical: {}", first == second);
        write(&out.join("headless.png"), &first)?;

        // 3 and 4 need the application object. Everything above ran without one.
        let mtm = MainThreadMarker::new().ok_or("the probe must run on the main thread")?;
        window_and_events(mtm, out)
    }

    /// The system font at `size`, as a document shell would ask for the UI font.
    fn system_font(size: CGFloat) -> CFRetained<CTFont> {
        // SAFETY: a null language means the user's own, which is what a UI font follows.
        unsafe { CTFont::new_ui_font_for_language(CTFontUIFontType::System, size, None) }
            .expect("the system UI font exists on every macOS")
    }

    /// One CoreText line of `text` set in `font`.
    fn line(text: &str, font: &CTFont) -> CFRetained<CTLine> {
        // SAFETY: `kCTFontAttributeName` is a constant CoreText exports for exactly this key.
        let key: &CFString = unsafe { kCTFontAttributeName };
        let value: &CFType = font.as_ref();
        let attributes = CFDictionary::<CFString, CFType>::from_slices(&[key], &[value]);
        let string = CFString::from_str(text);
        // SAFETY: the dictionary's keys are strings and its value a font, which is the shape
        // CoreText documents for an attributed string's attributes.
        let attributed =
            unsafe { CFAttributedString::new(None, Some(&string), Some(attributes.as_opaque())) }
                .expect("an attributed string from a string and one attribute");
        // SAFETY: the attributed string is valid for the duration of the call.
        unsafe { CTLine::with_attributed_string(&attributed) }
    }

    /// Every sample drawn into a bitmap with no application and no window, as a PNG.
    fn headless(font: &CTFont) -> Result<Vec<u8>, String> {
        // SAFETY: the name is one of CoreGraphics' own exported constants.
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
            .ok_or("no sRGB colour space")?;
        // SAFETY: a null data pointer asks CoreGraphics to allocate, and a row stride of zero
        // asks it to choose, both documented.
        let context = unsafe {
            CGBitmapContextCreate(
                null_mut(),
                W,
                H,
                8,
                0,
                Some(&space),
                CGImageAlphaInfo::PremultipliedLast.0,
            )
        }
        .ok_or("CGBitmapContextCreate returned nothing — no drawing without a window server?")?;
        let bitmap: &CGContext = &context;
        let context = Some(bitmap);

        CGContext::set_rgb_fill_color(context, 1.0, 1.0, 1.0, 1.0);
        CGContext::fill_rect(
            context,
            CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(W as f64, H as f64)),
        );
        CGContext::set_rgb_fill_color(context, 0.0, 0.0, 0.0, 1.0);
        // CoreGraphics' origin is bottom-left, so the first sample is drawn highest.
        for (row, (_, text)) in SAMPLES.iter().enumerate() {
            let y = H as f64 - 40.0 * (row as f64 + 1.0);
            CGContext::set_text_position(context, 20.0, y);
            // SAFETY: the context is a live bitmap context.
            unsafe { line(text, font).draw(bitmap) };
        }

        let image = CGBitmapContextCreateImage(context).ok_or("no image from the context")?;
        png_of_image(&image)
    }

    fn png_of_image(image: &objc2_core_graphics::CGImage) -> Result<Vec<u8>, String> {
        let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), image);
        png_of_rep(&rep)
    }

    fn png_of_rep(rep: &NSBitmapImageRep) -> Result<Vec<u8>, String> {
        // SAFETY: an empty property dictionary is the documented "no options".
        let data = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .ok_or("no PNG representation")?;
        Ok(data.to_vec())
    }

    fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
        std::fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Run the event loop by hand for `seconds`, dispatching whatever arrives.
    ///
    /// Without `NSApp.run`, nothing dispatches events, and a window that is never pumped is
    /// never drawn or made key.
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

    fn window_and_events(mtm: MainThreadMarker, out: &Path) -> Result<(), String> {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        app.finishLaunching();

        let screens = NSScreen::screens(mtm);
        println!("screens: {}", screens.count());

        let rect = NSRect::new(NSPoint::new(200.0, 200.0), CGSize::new(480.0, 160.0));
        // SAFETY: a titled, closable, resizable, buffered window is the ordinary kind.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: the window is owned by this `Retained`, so AppKit must not also release it
        // on close. That is objc2's documented rule for windows created from Rust.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("Grind probe"));

        let field = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
        window.setContentView(Some(&field));
        window.makeKeyAndOrderFront(None);
        app.activate();
        pump(&app, 1.0);

        println!("app-active: {}", app.isActive());
        println!("window-visible: {}", window.isVisible());
        println!("window-key: {}", window.isKeyWindow());
        println!("window-number: {}", window.windowNumber());
        println!("backing-scale: {}", window.backingScaleFactor());

        // 4. Synthesized events, through the application's own dispatch.
        let focused = window.makeFirstResponder(Some(&field));
        println!("field-focused: {focused}");
        // kVK_ANSI_H and kVK_ANSI_I, from Carbon's `Events.h`.
        for (character, code) in [("h", 0x04u16), ("i", 0x22u16)] {
            for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
                let characters = NSString::from_str(character);
                let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                    kind,
                    NSPoint::new(0.0, 0.0),
                    NSEventModifierFlags::empty(),
                    0.0,
                    window.windowNumber(),
                    None,
                    &characters,
                    &characters,
                    false,
                    code,
                )
                .ok_or("NSEvent refused to build a key event")?;
                app.sendEvent(&event);
            }
        }
        pump(&app, 0.5);
        println!("events-typed: {:?}", field.stringValue().to_string());

        // 3. The window, snapshotted in-process, frame and all when the frame view is there.
        let content = window
            .contentView()
            .ok_or("the window has no content view")?;
        // SAFETY: `superview` is an ordinary read of the view hierarchy.
        let frame_view: Retained<NSView> = unsafe { content.superview() }.unwrap_or(content);
        let bounds = frame_view.bounds();
        match frame_view.bitmapImageRepForCachingDisplayInRect(bounds) {
            Some(rep) => {
                frame_view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
                let png = png_of_rep(&rep)?;
                println!("snapshot-bytes: {}", png.len());
                write(&out.join("window.png"), &png)?;
            }
            None => println!("snapshot-bytes: none"),
        }

        window.close();
        Ok(())
    }
}

fn main() -> std::process::ExitCode {
    let out = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("grind-probe"));
    #[cfg(target_os = "macos")]
    {
        match mac::run(&out) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("probe: {message}");
                std::process::ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!(
            "probe: this measures a Mac, and runs on macOS only (it would have written to {})",
            out.display()
        );
        std::process::ExitCode::FAILURE
    }
}
