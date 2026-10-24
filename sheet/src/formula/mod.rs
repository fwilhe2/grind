// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The OpenFormula engine (ODF 1.4 Part 4). [`value`] is the value model everything else inherits
//! its correctness from.

pub mod array;
pub mod assist;
pub mod date;
pub mod display;
pub mod eval;
pub mod friendly;
pub mod funcs;
pub mod lex;
pub mod parse;
pub mod rename;
pub mod serialize;
pub mod shift;
pub mod trace;
pub mod value;
pub mod wildcard;
