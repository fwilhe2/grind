#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: MIT
#
# Tapes to play with: each written by `grind calc --save`, so each opens with
#
#   grind calc --open examples/tapes/quote.fods
#
# and in LibreOffice as the spreadsheet it promotes to — names in column A, the numbers and
# formulas in B, every name a named range. The amounts are made up to show the tape at work,
# except the household's, which are published averages (its comment says which). None of them
# is anybody's real quote, trip, loan or budget. Run again to write them afresh:
#
#   examples/tapes/make.sh [path/to/grind]

set -euo pipefail

grind="${1:-grind}"
cd "$(dirname "$0")"

# A freelance quote: hours at a day rate, VAT on top.
"$grind" calc --save quote.fods --force \
    'rate = 680 EUR' \
    'days = 4.5' \
    'net = rate * days' \
    'vat = 19%' \
    'tax = net * vat' \
    'net + tax'

# A German household's month, in German — **not a real household**: every amount is the average
# over all private households in Germany in the Einkommens- und Verbrauchsstichprobe (EVS) 2023
# of the Statistisches Bundesamt, per household and month. Net income from GENESIS-Online table
# 63221-0001 (4 282 EUR); consumer spending from Destatis' "Konsumausgaben privater Haushalte
# nach dem monatlichen Haushaltsnettoeinkommen" (EVS 2023, as of 9 December 2025): 3 032 EUR in
# all, of it 1 137 EUR on housing, energy and upkeep (rent with its service charges), 436 EUR on
# food, drink and tobacco, 361 EUR on transport, 142 EUR on information and communication and
# 260 EUR on leisure, sport and culture. The first line says so in the tape itself.
"$grind" calc --locale de-DE --save haushalt.fods --force \
    'hinweis = "Durchschnitt aller Haushalte, Destatis EVS 2023 - kein echter Haushalt"' \
    'einkommen = 4282 EUR' \
    'wohnen = 1137 EUR' \
    'lebensmittel = 436 EUR' \
    'verkehr = 361 EUR' \
    'kommunikation = 142 EUR' \
    'freizeit = 260 EUR' \
    'konsum = 3032 EUR' \
    'sonstiges = konsum - SUM(#3:#7)' \
    'einkommen - konsum' \
    'wohnanteil = wohnen / konsum'

# A weekend trip, split between friends.
"$grind" calc --save trip.fods --force \
    'fuel = 84.20 USD' \
    'cabin = 312 USD' \
    'groceries = 146.75 USD' \
    'tickets = 3 * 18.50 USD' \
    'total = SUM(#1:#4)' \
    'people = 3' \
    'total / people'

# A car loan: the monthly payment, and what the interest comes to.
"$grind" calc --save loan.fods --force \
    'price = 24900' \
    'deposit = 5000' \
    'principal = price - deposit' \
    'rate = 4.9%' \
    'years = 5' \
    'payment = PMT(rate / 12; years * 12; -principal)' \
    'payment * years * 12 - principal'

# A recipe for four, scaled to ten.
"$grind" calc --save recipe.fods --force \
    'serves = 4' \
    'want = 10' \
    'factor = want / serves' \
    'flour = 250 * factor' \
    'butter = 125 * factor' \
    'sugar = 90 * factor' \
    'eggs = ROUNDUP(2 * factor; 0)'
