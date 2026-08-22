// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The OpenFormula engine (ODF 1.4 Part 4) — phase 4, see doc/plan.md.
//!
//! Built in the order the plan fixes: [`value`] first, because the parser, the dependency
//! graph and all 110 Small Group functions inherit their correctness from it.

pub mod date;
pub mod display;
pub mod eval;
pub mod friendly;
pub mod funcs;
pub mod lex;
pub mod parse;
pub mod serialize;
pub mod shift;
pub mod value;
pub mod wildcard;
