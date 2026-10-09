// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The menu bar, drawn — the portable half: where its titles go, which one a point or a letter
//! means, and how the keyboard walks along it.
//!
//! Decision 4 keeps the menu bar as the surface that grows, and decision 13 is how it is drawn:
//! **by this shell, in the non-client band where Windows' own menu bar would be**, on the window's
//! own ground and in Fluent's *MenuBar* proportions. The system's bar is a strip of `COLOR_MENUBAR`
//! that ignores the dark theme entirely and draws a hairline under itself in a colour no palette
//! chose; nothing about it can be changed through a documented API. The bar's *popups* are still
//! Windows' own (`TrackPopupMenuEx` over the same `HMENU`s `build_menu` always made), so every item,
//! accelerator label, check mark and mnemonic inside a menu is exactly what it was.
//!
//! Living in the non-client area is what keeps this cheap: the client rectangle every pane already
//! lays itself out in, hit-tests in and positions its child controls in does not move.

use crate::sheet::geom::{Rect, scale};

/// Fluent's *MenuBar* at 100%: the band, an item inside it, and an item's text inset.
pub mod size {
    /// The band's height — Fluent's compact *MenuBar*, which leaves the window's chrome a row
    /// shorter than the standard 40 without the titles feeling cramped.
    pub const BAND: f64 = 36.0;
    /// An item's height, centred on the band.
    pub const ITEM_H: f64 = 28.0;
    /// An item's text inset, each side.
    pub const ITEM_PAD: f64 = 10.0;
    /// Between the window's edge and the first item, which lines the first title up with the
    /// format strip's first control below it.
    pub const LEAD: f64 = 6.0;
}

/// Where each title goes on a band, given each title's text width in pixels.
pub fn lay_out(widths: &[f64], band: Rect, dpi: u32) -> Vec<Rect> {
    let s = |v: f64| scale(v, dpi);
    let h = s(size::ITEM_H).min(band.h);
    let y = band.y + ((band.h - h) / 2.0).max(0.0);
    let mut x = band.x + s(size::LEAD);
    widths
        .iter()
        .map(|width| {
            let w = width + s(size::ITEM_PAD) * 2.0;
            let rect = Rect { x, y, w, h };
            x += w;
            rect
        })
        .collect()
}

/// Which title is under a point, if any.
pub fn hit(items: &[Rect], x: f64, y: f64) -> Option<usize> {
    items.iter().position(|item| item.contains(x, y))
}

/// A title as it is drawn: the `&` that marks its mnemonic taken out (a doubled one is a literal
/// ampersand and stays, once).
pub fn plain(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut chars = title.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                out.push('&');
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// A title's mnemonic — the letter after its single `&` — lower-cased, so Alt+F and Alt+Shift+F
/// mean one menu.
pub fn mnemonic(title: &str) -> Option<char> {
    let mut chars = title.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            match chars.next() {
                Some('&') => continue,
                Some(letter) => return letter.to_lowercase().next(),
                None => return None,
            }
        }
    }
    None
}

/// The title whose mnemonic is `letter`, if one has it.
pub fn by_mnemonic(titles: &[&str], letter: char) -> Option<usize> {
    let letter = letter.to_lowercase().next()?;
    titles
        .iter()
        .position(|title| mnemonic(title) == Some(letter))
}

/// One step along the bar, wrapping at both ends — what Left and Right do, in the bar and inside
/// an open menu alike.
pub fn step(from: usize, count: usize, forward: bool) -> usize {
    match (count, forward) {
        (0, _) => 0,
        (_, true) => (from + 1) % count,
        (_, false) => (from + count - 1) % count,
    }
}

/// The band's rectangle in **window** coordinates, given the client area's origin and size in
/// window coordinates: the band sits directly above the client, across the window's whole inner
/// width — over the vertical scroll bar's column too, since the scroll bar starts below it.
pub fn band(client_left: i32, client_top: i32, window_w: i32, dpi: u32) -> (i32, i32, i32, i32) {
    let h = scale(size::BAND, dpi).round() as i32;
    (
        client_left,
        client_top - h,
        (window_w - client_left).max(client_left),
        client_top,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_sit_side_by_side_centred_on_the_band() {
        let band = Rect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 36.0,
        };
        let items = lay_out(&[30.0, 28.0, 40.0], band, 96);
        assert!((items[0].x - 6.0).abs() < 1e-9);
        assert!((items[0].w - 50.0).abs() < 1e-9);
        assert!((items[1].x - (items[0].x + items[0].w)).abs() < 1e-9);
        for item in &items {
            assert!((item.y - 4.0).abs() < 1e-9 && (item.h - 28.0).abs() < 1e-9);
        }
        assert_eq!(hit(&items, 7.0, 10.0), Some(0));
        assert_eq!(hit(&items, 60.0, 10.0), Some(1));
        assert_eq!(hit(&items, 700.0, 10.0), None);
        assert_eq!(hit(&items, 7.0, 1.0), None, "above an item is not on it");
    }

    #[test]
    fn a_title_reads_without_its_mnemonic_marker() {
        assert_eq!(plain("&File"), "File");
        assert_eq!(plain("Fo&rmat"), "Format");
        assert_eq!(plain("Save && Close"), "Save & Close");
        assert_eq!(mnemonic("&File"), Some('f'));
        assert_eq!(mnemonic("Fo&rmat"), Some('r'));
        assert_eq!(mnemonic("Save && Close"), None);
        assert_eq!(mnemonic("Plain"), None);
    }

    #[test]
    fn every_title_in_the_bar_has_a_mnemonic_and_no_two_share_one() {
        let titles: Vec<&str> = crate::menu::MENUS.iter().map(|menu| menu.title).collect();
        let letters: Vec<char> = titles
            .iter()
            .map(|title| mnemonic(title).unwrap_or_else(|| panic!("{title} has no mnemonic")))
            .collect();
        for (i, letter) in letters.iter().enumerate() {
            assert_eq!(
                by_mnemonic(&titles, *letter),
                Some(i),
                "{} is shared",
                letter
            );
        }
        assert_eq!(
            by_mnemonic(&titles, 'F'),
            Some(0),
            "the shift state does not matter"
        );
    }

    #[test]
    fn the_arrows_wrap_at_both_ends() {
        assert_eq!(step(0, 4, false), 3);
        assert_eq!(step(3, 4, true), 0);
        assert_eq!(step(1, 4, true), 2);
        assert_eq!(step(0, 0, true), 0);
    }

    #[test]
    fn the_band_is_directly_above_the_client_area() {
        assert_eq!(band(8, 39, 1016, 96), (8, 3, 1008, 39));
        assert_eq!(band(0, 54, 800, 144), (0, 0, 800, 54));
    }
}

#[cfg(windows)]
pub use native::{
    install, key_menu, nc_button_down, nc_calc_size, nc_hit, nc_mouse_leave, nc_mouse_move,
    nc_paint, set_theme,
};

/// The drawing and the tracking: Windows only, and **a nested message loop** in two places —
/// `TrackPopupMenuEx`, and the keyboard's walk along the bar — so, by decision 7's rule, nothing
/// here holds a borrow of the window's state, and this module's own state is borrowed only between
/// messages.
#[cfg(windows)]
mod native {
    use std::cell::RefCell;

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        DT_CENTER, DT_HIDEPREFIX, DT_SINGLELINE, DT_VCENTER, DrawTextW, GetWindowDC, ReleaseDC,
        SetBkMode, SetTextColor, SetViewportOrgEx, TRANSPARENT,
    };
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY, VK_DOWN,
        VK_ESCAPE, VK_F10, VK_LEFT, VK_MENU, VK_RETURN, VK_RIGHT, VK_UP,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DefWindowProcW, DestroyMenu, DispatchMessageW, EndMenu,
        GetForegroundWindow, GetMessageW, GetWindowRect, HHOOK, HMENU, HTMENU, MSG, MSGF_MENU,
        NCCALCSIZE_PARAMS, PostMessageW, PostQuitMessage, SetWindowsHookExW, TPM_LEFTALIGN,
        TPM_LEFTBUTTON, TPM_RETURNCMD, TPM_TOPALIGN, TPM_VERTICAL, TPMPARAMS, TrackPopupMenuEx,
        TranslateMessage, UnhookWindowsHookEx, WH_MSGFILTER, WM_CHAR, WM_KEYDOWN, WM_KEYUP,
        WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCLBUTTONDOWN,
        WM_NCRBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    use crate::gdi::{self, BackBuffer, Font, Measure, Selected};
    use crate::sheet::geom::{Rect, scale};
    use crate::theme::Theme;

    /// The bar as it stands: the `HMENU` it was built as (kept, never attached — attaching it is
    /// what would bring Windows' own bar back), its titles, and which one the pointer, an open menu
    /// or the keyboard is on.
    struct State {
        menu: Option<HMENU>,
        titles: Vec<&'static str>,
        popups: Vec<HMENU>,
        hot: Option<usize>,
        open: Option<usize>,
        focus: Option<usize>,
        /// Whether the mnemonics are underlined — only while the keyboard is driving, which is
        /// Windows' own rule for access keys.
        cues: bool,
        leaving: bool,
        theme: Theme,
    }

    thread_local! {
        static BAR: RefCell<State> = RefCell::new(State {
            menu: None,
            titles: Vec::new(),
            popups: Vec::new(),
            hot: None,
            open: None,
            focus: None,
            cues: false,
            leaving: false,
            theme: Theme::of(crate::theme::Mode::Light),
        });
        /// What the message filter learns while a menu is open: the titles' rectangles on the
        /// screen, which one is open, and — once the menu has been told to close — which to open
        /// next and whether the keyboard asked.
        static SWITCH: RefCell<Switch> = const { RefCell::new(Switch {
            items: Vec::new(),
            open: 0,
            next: None,
            escaped: false,
        }) };
    }

    struct Switch {
        items: Vec<RECT>,
        open: usize,
        next: Option<(usize, bool)>,
        escaped: bool,
    }

    /// Take a freshly built bar: its `HMENU`, and the title of each popup in it, in order.
    pub fn install(hwnd: HWND, menu: HMENU, titles: Vec<&'static str>, popups: Vec<HMENU>) {
        let old = BAR.with_borrow_mut(|bar| {
            let old = bar.menu.replace(menu);
            bar.titles = titles;
            bar.popups = popups;
            bar.hot = None;
            bar.open = None;
            bar.focus = None;
            old
        });
        if let Some(old) = old {
            // SAFETY: the previous bar is this module's and is attached to nothing.
            unsafe {
                let _ = DestroyMenu(old);
            }
        }
        nc_paint(hwnd);
    }

    pub fn set_theme(theme: Theme) {
        BAR.with_borrow_mut(|bar| bar.theme = theme);
    }

    fn dpi(hwnd: HWND) -> u32 {
        // SAFETY: a live window.
        unsafe { GetDpiForWindow(hwnd) }.max(96)
    }

    /// `WM_NCCALCSIZE`: whatever the default makes of the frame, with the band taken off the top
    /// of the client area.
    pub fn nc_calc_size(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: the default handler first, with the message's own arguments; then the rectangle
        // it wrote, which for `wparam` true is the first of `NCCALCSIZE_PARAMS`' three and
        // otherwise the `RECT` itself.
        unsafe {
            let result = DefWindowProcW(hwnd, WM_NCCALCSIZE, wparam, lparam);
            let rect = match wparam.0 {
                0 => &mut *(lparam.0 as *mut RECT),
                _ => &mut (*(lparam.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0],
            };
            let band = scale(super::size::BAND, dpi(hwnd)).round() as i32;
            rect.top = (rect.top + band).min(rect.bottom);
            result
        }
    }

    /// The band and each title, in window coordinates.
    /// The band's edges, each title's rectangle, and the font they are measured in.
    type Geometry = ((i32, i32, i32, i32), Vec<Rect>, Font);

    fn geometry(hwnd: HWND) -> Option<Geometry> {
        let dpi = dpi(hwnd);
        let font = Font::new(
            gdi::ui_face(),
            scale(crate::theme::text::BODY, dpi).round() as i32,
            false,
        );
        let mut window = RECT::default();
        let mut origin = POINT::default();
        // SAFETY: live locals.
        unsafe {
            GetWindowRect(hwnd, &mut window).ok()?;
            let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut origin);
        }
        let band = super::band(
            origin.x - window.left,
            origin.y - window.top,
            window.right - window.left,
            dpi,
        );
        let widths: Vec<f64> = {
            let measure = Measure::new(&font)?;
            BAR.with_borrow(|bar| {
                bar.titles
                    .iter()
                    .map(|title| f64::from(measure.width(&super::plain(title))))
                    .collect()
            })
        };
        let rect = Rect {
            x: f64::from(band.0),
            y: f64::from(band.1),
            w: f64::from(band.2 - band.0),
            h: f64::from(band.3 - band.1),
        };
        Some((band, super::lay_out(&widths, rect, dpi), font))
    }

    /// Draw the band: the window's ground, and each title on a subtle rounded ground when the
    /// pointer is on it, its menu is open or the keyboard is on it.
    pub fn nc_paint(hwnd: HWND) {
        let Some(((left, top, right, bottom), items, font)) = geometry(hwnd) else {
            return;
        };
        let (w, h) = (right - left, bottom - top);
        if w <= 0 || h <= 0 {
            return;
        }
        let dpi = dpi(hwnd);
        let (theme, titles, hot, open, focus, cues) = BAR.with_borrow(|bar| {
            (
                bar.theme,
                bar.titles.clone(),
                bar.hot,
                bar.open,
                bar.focus,
                bar.cues,
            )
        });
        // SAFETY: the window DC is released below; the back buffer is dropped before it is.
        unsafe {
            let dc = GetWindowDC(Some(hwnd));
            if dc.is_invalid() {
                return;
            }
            if let Some(buffer) = BackBuffer::new(dc, w, h) {
                let out = buffer.dc();
                buffer.clear(theme.backdrop);
                let radius = scale(crate::theme::space::RADIUS, dpi).round() as i32;
                let _font = Selected::font(out, &font);
                SetBkMode(out, TRANSPARENT);
                for (i, (item, title)) in items.iter().zip(&titles).enumerate() {
                    let (l, t, r, b) = item.edges();
                    let rect = RECT {
                        left: l - left,
                        top: t - top,
                        right: r - left,
                        bottom: b - top,
                    };
                    let ground = if open == Some(i) {
                        Some(theme.subtle_pressed)
                    } else if focus == Some(i) || hot == Some(i) {
                        Some(theme.subtle_hover)
                    } else {
                        None
                    };
                    if let Some(ground) = ground {
                        gdi::round_rect(out, rect, radius, ground, ground);
                    }
                    SetTextColor(
                        out,
                        windows::Win32::Foundation::COLORREF(theme.text.colorref()),
                    );
                    let mut text: Vec<u16> = title.encode_utf16().collect();
                    let mut rect = rect;
                    let mut flags = DT_SINGLELINE | DT_VCENTER | DT_CENTER;
                    if !cues {
                        flags |= DT_HIDEPREFIX;
                    }
                    if !text.is_empty() {
                        DrawTextW(out, &mut text, &mut rect, flags);
                    }
                }
                drop(_font);
                let _ = SetViewportOrgEx(dc, left, top, None);
                buffer.present(dc);
                let _ = SetViewportOrgEx(dc, 0, 0, None);
            }
            ReleaseDC(Some(hwnd), dc);
        }
    }

    /// A point on the screen in window coordinates.
    fn in_window(hwnd: HWND, x: i32, y: i32) -> Option<(f64, f64)> {
        let mut window = RECT::default();
        // SAFETY: a live local.
        unsafe { GetWindowRect(hwnd, &mut window).ok()? };
        Some((f64::from(x - window.left), f64::from(y - window.top)))
    }

    fn screen_point(lparam: LPARAM) -> (i32, i32) {
        (
            i32::from((lparam.0 & 0xffff) as i16),
            i32::from(((lparam.0 >> 16) & 0xffff) as i16),
        )
    }

    /// `WM_NCHITTEST`: the band is the menu, as far as Windows is concerned.
    pub fn nc_hit(hwnd: HWND, lparam: LPARAM) -> Option<LRESULT> {
        let (x, y) = screen_point(lparam);
        let (x, y) = in_window(hwnd, x, y)?;
        let ((left, top, right, bottom), _, _) = geometry(hwnd)?;
        let inside = x >= f64::from(left)
            && x < f64::from(right)
            && y >= f64::from(top)
            && y < f64::from(bottom);
        inside.then_some(LRESULT(HTMENU as isize))
    }

    fn under(hwnd: HWND, lparam: LPARAM) -> Option<usize> {
        let (x, y) = screen_point(lparam);
        let (x, y) = in_window(hwnd, x, y)?;
        let (_, items, _) = geometry(hwnd)?;
        super::hit(&items, x, y)
    }

    pub fn nc_mouse_move(hwnd: HWND, lparam: LPARAM) {
        let hot = under(hwnd, lparam);
        let (changed, track) = BAR.with_borrow_mut(|bar| {
            let changed = bar.hot != hot;
            bar.hot = hot;
            let track = !bar.leaving;
            bar.leaving = true;
            (changed, track)
        });
        if track {
            let mut event = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE | TME_NONCLIENT,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            // SAFETY: a live local naming this window.
            unsafe {
                let _ = TrackMouseEvent(&mut event);
            }
        }
        if changed {
            nc_paint(hwnd);
        }
    }

    pub fn nc_mouse_leave(hwnd: HWND) {
        let changed = BAR.with_borrow_mut(|bar| {
            bar.leaving = false;
            bar.hot.take().is_some()
        });
        if changed {
            nc_paint(hwnd);
        }
    }

    /// A press on the band: the menu under it, opened and tracked. The command chosen, if any.
    pub fn nc_button_down(hwnd: HWND, lparam: LPARAM) -> Option<u16> {
        let index = under(hwnd, lparam)?;
        track(hwnd, index, false)
    }

    /// `SC_KEYMENU`: Alt or F10 alone (`letter` zero) walks the bar from the keyboard, Alt and a
    /// letter opens the menu it names. `None` when this is not the bar's — Alt+Space is the
    /// window menu's, and is handed back.
    pub fn key_menu(hwnd: HWND, letter: u32) -> Option<Option<u16>> {
        match letter {
            0 => Some(walk(hwnd, 0)),
            0x20 => None,
            letter => {
                let letter = char::from_u32(letter)?;
                let index = BAR.with_borrow(|bar| super::by_mnemonic(&bar.titles, letter));
                Some(index.and_then(|index| track(hwnd, index, true)))
            }
        }
    }

    /// Open one menu under its title and track it — and the next, when Left, Right or the pointer
    /// moves on to another title while it is open. The command chosen, if any. Escape out of a
    /// menu opened from the keyboard goes back to the bar, as it does in every Windows menu bar.
    fn track(hwnd: HWND, mut index: usize, mut keyboard: bool) -> Option<u16> {
        loop {
            let (_, items, _) = geometry(hwnd)?;
            let mut window = RECT::default();
            // SAFETY: a live local.
            unsafe { GetWindowRect(hwnd, &mut window).ok()? };
            let screen: Vec<RECT> = items
                .iter()
                .map(|item| {
                    let (l, t, r, b) = item.edges();
                    RECT {
                        left: l + window.left,
                        top: t + window.top,
                        right: r + window.left,
                        bottom: b + window.top,
                    }
                })
                .collect();
            let popup = BAR.with_borrow_mut(|bar| {
                bar.open = Some(index);
                bar.focus = None;
                bar.hot = None;
                bar.cues = keyboard;
                bar.popups.get(index).copied()
            })?;
            nc_paint(hwnd);
            let at = screen[index];
            SWITCH.with_borrow_mut(|switch| {
                switch.items = screen;
                switch.open = index;
                switch.next = None;
                switch.escaped = false;
            });
            // SAFETY: the hook is this thread's and is removed before this function returns; the
            // popup is the bar's own. **A nested message loop**, with nothing borrowed across it.
            let picked = unsafe {
                let hook =
                    SetWindowsHookExW(WH_MSGFILTER, Some(filter), None, GetCurrentThreadId()).ok();
                if keyboard {
                    // A menu opened from the keyboard has its first item highlighted, which is
                    // what Down would do — so Down is what it is sent.
                    let _ = PostMessageW(
                        Some(hwnd),
                        WM_KEYDOWN,
                        WPARAM(VK_DOWN.0 as usize),
                        LPARAM(0),
                    );
                }
                let params = TPMPARAMS {
                    cbSize: std::mem::size_of::<TPMPARAMS>() as u32,
                    rcExclude: at,
                };
                let picked = TrackPopupMenuEx(
                    popup,
                    (TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_LEFTBUTTON | TPM_VERTICAL)
                        .0,
                    at.left,
                    at.bottom,
                    hwnd,
                    Some(&params),
                );
                if let Some(hook) = hook {
                    let _ = UnhookWindowsHookEx(hook);
                }
                picked.0
            };
            BAR.with_borrow_mut(|bar| {
                bar.open = None;
                bar.cues = false;
            });
            nc_paint(hwnd);
            let (next, escaped) =
                SWITCH.with_borrow_mut(|switch| (switch.next.take(), switch.escaped));
            if picked != 0 {
                return u16::try_from(picked).ok();
            }
            match next {
                Some((next, by_key)) => {
                    index = next;
                    keyboard = by_key;
                }
                None if escaped && keyboard => return walk(hwnd, index),
                None => return None,
            }
        }
    }

    /// While a menu is open: Left and Right move to the neighbouring menu, the pointer moving onto
    /// another title opens that one, and a press on the open menu's own title closes it.
    unsafe extern "system" fn filter(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == MSGF_MENU as i32 {
            // SAFETY: for `WH_MSGFILTER`, `lparam` is the message being processed.
            let message = unsafe { &*(lparam.0 as *const MSG) };
            let eat = SWITCH.with_borrow_mut(|switch| {
                let count = switch.items.len();
                let on = |pt: POINT| {
                    switch.items.iter().position(|r| {
                        pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
                    })
                };
                match message.message {
                    WM_KEYDOWN => {
                        let key = VIRTUAL_KEY(message.wParam.0 as u16);
                        if key == VK_LEFT || key == VK_RIGHT {
                            switch.next =
                                Some((super::step(switch.open, count, key == VK_RIGHT), true));
                            return true;
                        }
                        if key == VK_ESCAPE {
                            switch.escaped = true;
                        }
                        false
                    }
                    WM_MOUSEMOVE => match on(message.pt) {
                        Some(i) if i != switch.open => {
                            switch.next = Some((i, false));
                            true
                        }
                        _ => false,
                    },
                    WM_LBUTTONDOWN | WM_NCLBUTTONDOWN => match on(message.pt) {
                        Some(i) if i == switch.open => true,
                        Some(i) => {
                            switch.next = Some((i, false));
                            true
                        }
                        None => false,
                    },
                    _ => false,
                }
            });
            if eat {
                // SAFETY: closing the menu this thread is tracking.
                unsafe {
                    let _ = EndMenu();
                }
                return LRESULT(1);
            }
        }
        // SAFETY: the next hook in the chain, with this one's arguments.
        unsafe { CallNextHookEx(None::<HHOOK>, code, wparam, lparam) }
    }

    /// The keyboard on the bar with no menu open — after Alt or F10 on its own. Left and Right
    /// move, Enter, Up and Down open, a title's letter opens it, and Escape, Alt again, a click or
    /// any other key leave. The command chosen in a menu opened from here, if any.
    ///
    /// **A message loop of its own**, like a menu's: the keyboard has to be the bar's without the
    /// focus moving, since moving it would commit a half-typed cell. Everything that is not a key
    /// is dispatched as usual, so the window keeps painting.
    fn walk(hwnd: HWND, start: usize) -> Option<u16> {
        let count = BAR.with_borrow_mut(|bar| {
            bar.focus = Some(start);
            bar.cues = true;
            bar.titles.len()
        });
        if count == 0 {
            return None;
        }
        nc_paint(hwnd);
        let mut picked = None;
        let mut leaving_on_alt = false;
        let mut message = MSG::default();
        loop {
            // SAFETY: an ordinary message pump; nothing is borrowed across it.
            unsafe {
                if GetForegroundWindow() != hwnd {
                    break;
                }
                if GetMessageW(&mut message, None, 0, 0).0 <= 0 {
                    PostQuitMessage(0);
                    break;
                }
            }
            let at = BAR.with_borrow(|bar| bar.focus.unwrap_or(0));
            let key = VIRTUAL_KEY(message.wParam.0 as u16);
            match message.message {
                // Alt pressed again: leave once it is released, and swallow the release, which
                // would otherwise come straight back here as another `SC_KEYMENU`.
                WM_SYSKEYDOWN | WM_KEYDOWN if key == VK_MENU || key == VK_F10 => {
                    leaving_on_alt = true;
                }
                WM_SYSKEYUP | WM_KEYUP if leaving_on_alt => break,
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    if key == VK_LEFT || key == VK_RIGHT {
                        BAR.with_borrow_mut(|bar| {
                            bar.focus = Some(super::step(at, count, key == VK_RIGHT))
                        });
                        nc_paint(hwnd);
                    } else if key == VK_RETURN || key == VK_DOWN || key == VK_UP {
                        picked = track(hwnd, at, true);
                        break;
                    } else if key == VK_ESCAPE {
                        break;
                    } else {
                        // Let it become a character, which is how a mnemonic arrives.
                        // SAFETY: translating a message this loop owns.
                        unsafe {
                            let _ = TranslateMessage(&message);
                        }
                    }
                }
                WM_CHAR | WM_SYSCHAR => {
                    let letter = char::from_u32(message.wParam.0 as u32);
                    let index = letter.and_then(|letter| {
                        BAR.with_borrow(|bar| super::by_mnemonic(&bar.titles, letter))
                    });
                    if let Some(index) = index {
                        picked = track(hwnd, index, true);
                    }
                    break;
                }
                WM_KEYUP | WM_SYSKEYUP => {}
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_NCLBUTTONDOWN
                | WM_NCRBUTTONDOWN => {
                    BAR.with_borrow_mut(|bar| {
                        bar.focus = None;
                        bar.cues = false;
                    });
                    nc_paint(hwnd);
                    // SAFETY: the click is the user's, and goes where it was going.
                    unsafe {
                        DispatchMessageW(&message);
                    }
                    return None;
                }
                _ => {
                    // SAFETY: as above.
                    unsafe {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            }
        }
        BAR.with_borrow_mut(|bar| {
            bar.focus = None;
            bar.cues = false;
        });
        nc_paint(hwnd);
        picked
    }
}
