// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The grid, inside an `NSScrollView` — the spreadsheet's document view, and the one place this
//! shell draws a sheet on screen (decision 3: AppKit draws the chrome, this draws the document).
//!
//! Four flipped views over one [`Pane`]: the grid itself, as the scroll view's document view and as
//! large as the part of the sheet worth scrolling to; the column and row header bands, as the
//! scroll view's **floating subviews**, so each stays put along its own axis and scrolls with the
//! cells along the other; and the corner where they meet, over both. The scroll view does the
//! scrolling — elastic, with the system's own scrollers — and every view draws through
//! `sheet::paint` and [`crate::render::draw`], the path `--render-to` takes too.
//!
//! The grid view's own coordinates put the sheet's top-left cell at `(HEADER_W, HEADER_H)`: the
//! bands float over that margin, so at rest they cover nothing but it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use grind_core::color::{self, Rgb};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSColorSpace, NSEventGestureAxis, NSGraphicsContext,
    NSScrollView, NSView,
};
use objc2_core_graphics::CGContext;
use objc2_foundation::{NSPoint, NSRect, NSSize};

use crate::metrics::{BASE_PT, CoreText};
use crate::render;
use crate::sheet::geom::{Grid, HEADER_H, HEADER_W, Rect};
use crate::sheet::paint::{self, Op, Palette};

/// What every view of one spreadsheet window draws from: the document, which sheet is showing,
/// where its cells are, and the fonts it is measured in.
pub struct Pane {
    pub app: Arc<grind_sheet::App>,
    pub sheet: Cell<usize>,
    pub grid: RefCell<Grid>,
    pub text: CoreText,
}

impl Pane {
    pub fn new(app: Arc<grind_sheet::App>) -> Rc<Pane> {
        let grid = Grid::of(&app, 0);
        Rc::new(Pane {
            app,
            sheet: Cell::new(0),
            grid: RefCell::new(grid),
            text: CoreText::new(BASE_PT),
        })
    }
}

/// A colour AppKit resolves for the current drawing appearance, as three bytes. A colour with no
/// sRGB form — a pattern — is a mid grey, which reads on either appearance.
fn rgb(color: &NSColor) -> Rgb {
    let Some(srgb) = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()) else {
        return (0x80, 0x80, 0x80);
    };
    let byte = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    (
        byte(srgb.redComponent()),
        byte(srgb.greenComponent()),
        byte(srgb.blueComponent()),
    )
}

/// The system's semantic colours, resolved in the appearance AppKit is drawing in — a view's
/// `drawRect:` runs with the view's own `effectiveAppearance` current, so dark mode, high contrast
/// and a window forced into one appearance all follow with nothing here knowing (decision 8).
fn palette() -> Palette {
    let page = rgb(&NSColor::textBackgroundColor());
    Palette {
        page,
        ink: rgb(&NSColor::textColor()),
        grid: rgb(&NSColor::gridColor()),
        header: rgb(&NSColor::controlBackgroundColor()),
        header_ink: rgb(&NSColor::secondaryLabelColor()),
        dark: color::luminance(page) < 0.5,
    }
}

/// One device pixel, in points — how thin a grid line is on the screen `view` is on.
fn hairline(view: &NSView) -> f64 {
    view.window()
        .map_or(1.0, |window| window.backingScaleFactor())
        .max(1.0)
        .recip()
}

fn rect(frame: NSRect) -> Rect {
    Rect::new(
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
    )
}

/// The context AppKit is drawing into right now, and `ops` put down on it.
fn draw(ops: &[Op], pane: &Pane) {
    let Some(context) = NSGraphicsContext::currentContext() else {
        return;
    };
    let cg: Retained<CGContext> = context.CGContext();
    render::draw(&cg, ops, &pane.text);
}

define_class!(
    /// The cells, as the scroll view's document view.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindGridView"]
    #[ivars = Rc<Pane>]
    pub struct GridView;

    impl GridView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            let pane = self.ivars();
            // The dirty rectangle in the sheet's own coordinates: the view keeps a margin the
            // header bands float over.
            let view = rect(dirty).offset(-HEADER_W, -HEADER_H);
            let ops: Vec<Op> = paint::cells(
                &pane.app,
                pane.sheet.get(),
                &pane.grid.borrow(),
                view,
                &palette(),
                &pane.text,
                hairline(self),
            )
            .into_iter()
            .map(|op| shift(op, HEADER_W, HEADER_H))
            .collect();
            draw(&ops, pane);
        }
    }
);

define_class!(
    /// The column letters, floating over the grid's top margin.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindColumnHeader"]
    #[ivars = Rc<Pane>]
    pub struct ColumnHeader;

    impl ColumnHeader {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            let pane = self.ivars();
            let dirty = rect(dirty);
            let ops = paint::column_header(
                &pane.grid.borrow(),
                dirty.x,
                dirty.w,
                &palette(),
                &pane.text,
                hairline(self),
            );
            draw(&ops, pane);
        }
    }
);

define_class!(
    /// The row numbers, floating over the grid's left margin.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindRowHeader"]
    #[ivars = Rc<Pane>]
    pub struct RowHeader;

    impl RowHeader {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            let pane = self.ivars();
            let dirty = rect(dirty);
            let ops = paint::row_header(
                &pane.grid.borrow(),
                dirty.y,
                dirty.h,
                &palette(),
                &pane.text,
                hairline(self),
            );
            draw(&ops, pane);
        }
    }
);

define_class!(
    /// Where the two bands meet, over both, so neither's labels show through it.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindCorner"]
    #[ivars = Rc<Pane>]
    pub struct Corner;

    impl Corner {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let palette = palette();
            let line = hairline(self);
            let ops = [
                Op::Fill {
                    rect: Rect::new(0.0, 0.0, HEADER_W, HEADER_H),
                    color: palette.header,
                },
                Op::Fill {
                    rect: Rect::new(0.0, HEADER_H - line, HEADER_W, line),
                    color: palette.grid,
                },
                Op::Fill {
                    rect: Rect::new(HEADER_W - line, 0.0, line, HEADER_H),
                    color: palette.grid,
                },
            ];
            draw(&ops, self.ivars());
        }
    }
);

/// `op` drawn `(dx, dy)` further on, with nothing cut away.
fn shift(op: Op, dx: f64, dy: f64) -> Op {
    match op {
        Op::Fill { rect, color } => Op::Fill {
            rect: rect.offset(dx, dy),
            color,
        },
        Op::Text {
            x,
            top,
            text,
            style,
            color,
            clip,
        } => Op::Text {
            x: x + dx,
            top: top + dy,
            text,
            style,
            color,
            clip: clip.offset(dx, dy),
        },
    }
}

fn frame(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// The spreadsheet's scroll view, with the grid in it and the bands floating over it. The views
/// hold `pane` between them, and the scroll view holds the views.
pub fn sheet_view(pane: Rc<Pane>, size: NSSize, mtm: MainThreadMarker) -> Retained<NSScrollView> {
    {
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            frame(0.0, 0.0, size.width, size.height),
        );
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(true);

        let (w, h) = pane.grid.borrow().size();
        let grid: Retained<GridView> = {
            let this = GridView::alloc(mtm).set_ivars(pane.clone());
            // SAFETY: `initWithFrame:` is `NSView`'s designated initialiser.
            unsafe {
                msg_send![super(this), initWithFrame: frame(0.0, 0.0, HEADER_W + w, HEADER_H + h)]
            }
        };
        let columns: Retained<ColumnHeader> = {
            let this = ColumnHeader::alloc(mtm).set_ivars(pane.clone());
            // SAFETY: as above.
            unsafe { msg_send![super(this), initWithFrame: frame(HEADER_W, 0.0, w, HEADER_H)] }
        };
        let rows: Retained<RowHeader> = {
            let this = RowHeader::alloc(mtm).set_ivars(pane.clone());
            // SAFETY: as above.
            unsafe { msg_send![super(this), initWithFrame: frame(0.0, HEADER_H, HEADER_W, h)] }
        };
        let corner: Retained<Corner> = {
            let this = Corner::alloc(mtm).set_ivars(pane.clone());
            // SAFETY: as above.
            unsafe { msg_send![super(this), initWithFrame: frame(0.0, 0.0, HEADER_W, HEADER_H)] }
        };

        scroll.setDocumentView(Some(&grid));
        // The column band never moves down with the cells, and the row band never moves across.
        scroll.addFloatingSubview_forAxis(&columns, NSEventGestureAxis::Vertical);
        scroll.addFloatingSubview_forAxis(&rows, NSEventGestureAxis::Horizontal);
        // The corner moves with neither: it is the scroll view's own, over its content's top
        // left, added last so it is over both bands. A scroll view need not be flipped, so where
        // its top is — and which margin stretches as it grows — is asked rather than assumed.
        if !scroll.isFlipped() {
            corner.setFrameOrigin(NSPoint::new(0.0, size.height - HEADER_H));
            corner.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
        }
        scroll.addSubview(&corner);
        scroll
    }
}
