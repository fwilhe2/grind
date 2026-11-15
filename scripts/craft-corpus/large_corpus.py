#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Large, plausible Office files built with the Craft CLIs, for a Grind compatibility matrix.

Two kinds of file come out of this:

* probes: one small document per feature, each carrying a unique token, so a matrix can tell
  which feature a loss belongs to (``probes/xlsx``, ``probes/docx``);
* scale tiers: one enterprise workbook and one operating playbook, modelled on the Northstar
  fixtures, at growing sizes (``scale/xlsx``, ``scale/docx``).

Every probe and tier is listed in ``manifest.json`` with what to look for in the Office file
(did Craft write the feature at all?) and in Grind's ODF output (did it survive?).
"""

import argparse
import base64
import json
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib
from datetime import date, timedelta
from pathlib import Path

WORDCRAFT = shutil.which("wordcraft-cli")
GRIDCRAFT = shutil.which("gridcraft-cli")
os.environ.setdefault("WORDCRAFT_REGION", "en-US")


def cmd(name, params=None):
    item = {"command": name}
    if params is not None:
        item["params"] = params
    return item


def png(width=96, height=48, rgb=(46, 117, 182)):
    """A solid-colour PNG, written by hand so no imaging library is needed."""
    row = b"\x00" + bytes(rgb) * width
    raw = row * height

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


PNG_B64 = base64.b64encode(png()).decode()


def run_wordcraft(path, commands):
    with tempfile.NamedTemporaryFile("w", suffix=".json", encoding="utf-8", delete=False) as script:
        json.dump({"template": "blank", "commands": commands}, script)
    try:
        subprocess.run([WORDCRAFT, "run", "--script", script.name, "--save", str(path), "--author", "Compatibility Test Fixture"],
                       check=True, stdout=subprocess.DEVNULL)
    finally:
        os.unlink(script.name)


def run_gridcraft(path, commands):
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", encoding="utf-8", delete=False) as script:
        for item in commands:
            script.write(json.dumps(item) + "\n")
    try:
        subprocess.run([GRIDCRAFT, "run", "--script", script.name, "--out", str(path), "--quiet"], check=True, stdout=subprocess.DEVNULL)
    finally:
        os.unlink(script.name)


def compose_wordcraft(block, copies):
    """A runner that writes `block` appended to itself until there are `copies` of it."""
    def run(path, _commands):
        append = [a for _ in range(copies - 1) for a in ("--cmd", "caret.docEnd", "--cmd", "insert.textFromFile=" + json.dumps({"path": str(block)}))]
        subprocess.run([WORDCRAFT, "run", "--file", str(block), *append, "--save", str(path)], check=True, stdout=subprocess.DEVNULL)
    return run


# --------------------------------------------------------------------------------------------
# Spreadsheet probes. `src` is a regex over the xlsx's parts, `odf` one over Grind's output;
# both are searched in every XML part of the package, concatenated.

def xbase(*extra):
    return [
        cmd("file.new", {}),
        cmd("sheet.rename", {"name": "Probe"}),
        cmd("range.setValues", {"range": "A1:D6", "values": [
            ["FICTIONAL TEST DATA", "Units", "Price", "Region"],
            ["Desk lamp", 12, 42.5, "North"],
            ["Notebook set", 35, 18, "South"],
            ["Storage box", 18, 27.25, "East"],
            ["Cable organizer", 42, 8.75, "West"],
            ["Monitor riser", 14, 54, "North"],
        ]}),
        *extra,
    ]


def xvalues(rng, rows):
    return cmd("range.setValues", {"range": rng, "values": rows})


XLSX_PROBES = [
    # values and formulas
    ("values", "Text, numbers, booleans", xbase(xvalues("F1:F3", [["XTOKVAL"], [True], [-0.0001]])),
     r"XTOKVAL", r"XTOKVAL.*?office:boolean-value=\"true\"|office:boolean-value=\"true\".*?XTOKVAL"),
    ("unicode", "Unicode, emoji, RTL, CJK text", xbase(xvalues("F1:F3", [["Grüße Ünïcödé 😀"], ["مرحبا"], ["日本語テキスト"]])),
     r"日本語", r"Grüße Ünïcödé 😀.*مرحبا.*日本語テキスト"),
    ("long-text", "Text near the 32,767-character cell limit", xbase(xvalues("F1", [["XTOKLONG" + "x" * 32000]])),
     r"XTOKLONGx{31000}", r"XTOKLONGx{31000}"),
    ("leading-apostrophe", "Numeric-looking text kept as text", xbase(cmd("range.setValues", {"range": "F1:F2", "values": [["00123"], ["1-2"]], "parse": False})),
     r"00123", r'office:value-type="string"[^>]*>\s*<text:p>00123'),
    ("dates", "Dates and times", xbase(xvalues("F1:F3", [["2026-11-15"], ["14:30"], ["2026-11-15 08:15"]])),
     r"<v>46\d{3}(\.\d+)?</v>", r'office:date-value="2026-11-15"'),
    ("formula-basic", "Arithmetic and SUM", xbase(xvalues("E2:E7", [["=B2*C2"], ["=B3*C3"], ["=B4*C4"], ["=B5*C5"], ["=B6*C6"], ["=SUM(E2:E6)"]])),
     r"<f>SUM\(E2:E6\)</f>", r'table:formula="(of:)?=SUM\(\[\.E2:\.E6\]\)"'),
    ("formula-cross-sheet", "Formulas referencing another sheet", xbase(cmd("home.insertSheet", {"name": "Summary"}), xvalues("A1:B2", [["Total units", "=SUM(Probe!B2:B6)"], ["Max price", "=MAX(Probe!C2:C6)"]])),
     r"<f>SUM\(Probe!B2:B6\)</f>", r"=SUM\(\[\$?'?Probe'?\.B2:\.B6\]\)"),
    ("formula-conditional", "SUMIFS, COUNTIFS, AVERAGEIFS", xbase(xvalues("F1:F3", [['=SUMIFS(B2:B6,D2:D6,"North")'], ['=COUNTIFS(D2:D6,"North",B2:B6,">10")'], ['=AVERAGEIFS(C2:C6,D2:D6,"North")']])),
     r"<f>SUMIFS", r"=SUMIFS.*=COUNTIFS.*=AVERAGEIFS"),
    ("formula-lookup", "VLOOKUP, INDEX/MATCH, XLOOKUP", xbase(xvalues("F1:F3", [['=VLOOKUP("Storage box",A2:C6,3,FALSE)'], ['=INDEX(C2:C6,MATCH("Notebook set",A2:A6,0))'], ['=XLOOKUP("Monitor riser",A2:A6,C2:C6)']])),
     r"XLOOKUP", r"=VLOOKUP.*=INDEX.*XLOOKUP"),
    ("formula-text-date", "Text and date functions", xbase(xvalues("F1:F4", [['=TEXT(DATE(2026,11,15),"yyyy-mm-dd")'], ['=CONCATENATE(A2," / ",D2)'], ['=EOMONTH(DATE(2026,2,10),0)'], ['=NETWORKDAYS(DATE(2026,11,2),DATE(2026,11,27))']])),
     r"NETWORKDAYS", r"=TEXT.*=CONCATENATE.*=EOMONTH.*=NETWORKDAYS"),
    ("formula-iferror", "IFERROR and error values", xbase(xvalues("F1:F3", [["=1/0"], ["=IFERROR(1/0,\"n/a\")"], ["=NA()"]])),
     r"#DIV/0!", r"=IFERROR.*=NA\(\)"),
    ("formula-dynamic", "Dynamic arrays: UNIQUE, FILTER, SORT", xbase(xvalues("H1", [["=UNIQUE(D2:D6)"]]), xvalues("I1", [['=FILTER(A2:A6,B2:B6>15)']]), xvalues("J1", [["=SORT(C2:C6)"]])),
     r"UNIQUE", r"UNIQUE.*FILTER.*SORT"),
    ("formula-array", "Legacy CSE array formula", xbase(cmd("cell.set", {"cell": "F1", "input": "=SUM(B2:B6*C2:C6)", "array": True})),
     r't="array"', r"table:number-matrix-columns-spanned"),
    ("formula-structured", "Structured table references", xbase(cmd("insert.table", {"range": "A1:D6", "header": True, "name": "Probes"}), xvalues("F1", [["=SUM(Probes[Units])"]])),
     r"Probes\[Units\]|SUM\(B2:B6\)", r"=SUM\(\[\.\$?B\$?2:\.\$?B\$?6\]\)"),
    ("defined-name", "Workbook defined name", xbase(cmd("formulas.defineName", {"name": "XTOKNAME", "refersTo": "=Probe!$B$2:$B$6"}), xvalues("F1", [["=SUM(XTOKNAME)"]])),
     r"XTOKNAME", r'(?i)table:name="XTOKNAME"', r'table:name="XTOKNAME"'),
    ("defined-name-sheet", "Sheet-scoped defined name", xbase(cmd("formulas.defineName", {"name": "XTOKLOCAL", "refersTo": "=Probe!$C$2:$C$6", "scope": "Probe"})),
     r"XTOKLOCAL", r'(?i)table:name="XTOKLOCAL"', r'<table:table [^>]*>(?:(?!</table:table>).)*table:name="XTOKLOCAL"'),
    ("calc-manual", "Manual calculation mode", xbase(cmd("formulas.calculationOptions", {"mode": "manual"})),
     r'calcMode="manual"', r"AutoCalculate.{0,40}false|table:automatic-find-labels|calculation-settings[^>]*manual"),
    # cell formatting
    ("number-currency", "Currency number format", xbase(cmd("home.numberFormat", {"range": "C2:C6", "format": "Currency"})),
     r"numFmt[^>]*formatCode=\"[^\"]*[$€]", r"number:currency-style|number:currency-symbol|<number:text>\$</number:text>"),
    ("number-percent", "Percentage format", xbase(xvalues("F1", [[0.1292]]), cmd("home.numberFormat", {"range": "F1", "format": "Percentage"})),
     r'numFmtId="1[09]"|formatCode="0(\.0+)?%"', r"number:percentage-style"),
    ("number-custom", "Custom format with colour and sections", xbase(xvalues("F1:F2", [[-1234.5], [1234.5]]), cmd("home.numberFormat", {"range": "F1:F2", "code": "#,##0.00;[Red]-#,##0.00"})),
     r"\[Red\]", r'fo:color="#ff0000"'),
    ("number-date", "Long date format", xbase(xvalues("F1", [["2026-11-15"]]), cmd("home.numberFormat", {"range": "F1", "format": "Long Date"})),
     r"numFmt", r"number:date-style.*number:month[^>]*number:textual=\"true\""),
    ("font", "Font name, size, bold, italic, colour", xbase(cmd("home.fontName", {"range": "A1:D1", "name": "Georgia"}), cmd("home.fontSize", {"range": "A1:D1", "size": 14}), cmd("home.bold", {"range": "A1:D1", "on": True}), cmd("home.italic", {"range": "A2"}), cmd("home.fontColor", {"range": "B2", "color": "#C00000"})),
     r"Georgia", r'fo:font-weight="bold".*|fo:font-size="14pt"'),
    ("font-family", "Non-default font family carried", xbase(cmd("home.fontName", {"range": "A1:D6", "name": "Georgia"})),
     r"Georgia", r"Georgia"),
    ("fill-borders", "Fill colour and borders", xbase(cmd("home.fillColor", {"range": "A1:D1", "color": "#1F4E79"}), cmd("home.borders", {"range": "A1:D6", "preset": "all", "style": "thin"})),
     r"1F4E79", r'fo:background-color="#1f4e79".*|fo:border'),
    ("alignment", "Alignment, wrap, rotation, indent", xbase(cmd("home.alignCenter", {"range": "B1:D1"}), cmd("home.wrapText", {"range": "A1", "on": True}), cmd("home.orientation", {"range": "D1", "angle": 45}), cmd("home.increaseIndent", {"range": "A2"})),
     r'textRotation="45"', r'style:rotation-angle="45"'),
    ("merge", "Merged cells", xbase(cmd("home.mergeCenter", {"range": "A8:D8"}), xvalues("A8", [["XTOKMERGE"]])),
     r"mergeCell ref=\"A8:D8\"", r'table:number-columns-spanned="4"'),
    ("cell-style", "Named cell styles (Good, Bad, Heading 1)", xbase(cmd("home.cellStyle", {"range": "A1:D1", "name": "Heading 1"}), cmd("home.cellStyle", {"range": "B2", "name": "Good"}), cmd("home.cellStyle", {"range": "B3", "name": "Bad"})),
     r'cellStyle[^>]*name="Good"', r'style:name="Good"|style:display-name="Good"'),
    # rows, columns, sheets
    ("col-width-row-height", "Column widths and row heights", xbase(cmd("home.columnWidth", {"cols": "A:A", "chars": 38}), cmd("home.rowHeight", {"rows": "1:1", "points": 32})),
     r'customHeight="1"', r'style:row-height="32pt"'),
    ("hidden-rows-cols", "Hidden rows and columns", xbase(cmd("home.hideRows", {"rows": "3:3"}), cmd("home.hideColumns", {"cols": "C:C"})),
     r'hidden="1"', r'table:visibility="collapse"'),
    ("grouping", "Row and column outline groups", xbase(cmd("data.group", {"rows": "2:5"}), cmd("data.group", {"cols": "B:C"})),
     r"outlineLevel=\"1\"", r"table:table-row-group.*|table:table-column-group"),
    ("sheet-hidden", "Hidden sheet", xbase(cmd("home.insertSheet", {"name": "XTOKHIDDEN"}), cmd("sheet.hide", {"sheet": "XTOKHIDDEN"})),
     r'state="hidden"', r'table:display="false"'),
    ("tab-color", "Sheet tab colour", xbase(cmd("sheet.tabColor", {"sheet": "Probe", "color": "#C00000"})),
     r"tabColor", r"tab-color|TabColor"),
    ("many-sheets", "Forty sheets", [cmd("file.new", {})] + [c for i in range(2, 41) for c in (cmd("home.insertSheet", {"name": f"S{i:02d}"}), xvalues("A1", [[f"Sheet {i}"]]))],
     r'name="S40"', r'table:name="S40"'),
    ("sheet-protection", "Protected sheet", xbase(cmd("review.protectSheet", {})),
     r"sheetProtection", r'table:protected="true"'),
    ("workbook-protection", "Protected workbook structure", xbase(cmd("review.protectWorkbook", {"on": True})),
     r"workbookProtection", r'table:structure-protected="true"'),
    # view and page
    ("freeze-panes", "Frozen panes", xbase(cmd("view.freezePanes", {"cell": "B2"})),
     r'state="frozen"', r'"HorizontalSplitMode"[^>]*>2'),
    ("zoom-gridlines", "Zoom and hidden gridlines", xbase(cmd("view.zoom", {"percent": 85}), cmd("view.gridlines", {"on": False})),
     r'zoomScale="85"', r'"ZoomValue"[^>]*>85'),
    ("page-setup", "Landscape, margins, scale to fit", xbase(cmd("pageLayout.orientation", {"orientation": "landscape"}), cmd("pageLayout.margins", {"preset": "narrow"}), cmd("pageLayout.scaleToFit", {"width": 1, "height": 0})),
     r'orientation="landscape"', r'style:print-orientation="landscape"'),
    ("header-footer", "Header and footer with fields", xbase(cmd("pageLayout.headerFooter", {"header": "&CXTOKHEAD", "footer": "&CPage &P of &N"})),
     r"XTOKHEAD", r"<style:header>.*?XTOKHEAD"),
    ("print-area-titles", "Print area and print titles", xbase(cmd("pageLayout.printArea", {"range": "A1:D6"}), cmd("pageLayout.printTitles", {"rows": "1:1"})),
     r"_xlnm.Print_Area", r"table:print-ranges"),
    ("page-breaks", "Manual page break", xbase(cmd("pageLayout.breaks", {"insert": True, "cell": "A4"})),
     r"rowBreaks", r'fo:break-before="page"'),
    # data features
    ("table", "Excel table (ListObject) with style", xbase(cmd("insert.table", {"range": "A1:D6", "header": True, "style": "TableStyleMedium2", "name": "XTOKTABLE"})),
     r"XTOKTABLE", r'table:database-range[^>]*table:name="XTOKTABLE"'),
    ("table-total-row", "Table total row", xbase(cmd("insert.table", {"range": "A1:D6", "header": True, "name": "Totals"}), cmd("table.totalRow", {"table": "Totals", "on": True})),
     r'totalsRowCount="1"|totalsRowShown', r"=SUBTOTAL\(109"),
    ("autofilter", "AutoFilter with active criterion", xbase(cmd("selection.set", {"range": "A1"}), cmd("data.filter", {"range": "A1:D6"}), cmd("data.filterBy", {"column": "D", "values": ["North"]})),
     r"<filterColumn", r"table:filter-condition"),
    ("sort-state", "Sorted range", xbase(cmd("data.sort", {"range": "A1:D6", "header": True, "keys": [{"column": "B", "order": "desc"}]})),
     r"sortState|<v>42</v>", r"Cable organizer"),
    ("validation-list", "List data validation", xbase(cmd("data.validation", {"range": "D2:D6", "type": "list", "formula1": '"North,South,East,West"'})),
     r"<dataValidation", r"table:content-validation"),
    ("validation-whole", "Whole-number validation with error message", xbase(cmd("data.validation", {"range": "B2:B6", "type": "whole", "operator": "greaterOrEqual", "formula1": "0", "errorTitle": "XTOKERR", "errorMessage": "Enter a non-negative whole number."})),
     r"XTOKERR", r"table:content-validation.*XTOKERR"),
    ("cf-cellis", "Conditional format: cell value", xbase(cmd("home.conditionalFormat", {"range": "B2:B6", "rule": {"type": "cellIs", "operator": "less", "value": 20, "preset": "lightRedFill"}})),
     r'cfRule[^>]*type="cellIs"', r"calcext:conditional-format|style:map"),
    ("cf-expression", "Conditional format: formula", xbase(cmd("home.conditionalFormat", {"range": "A2:D6", "rule": {"type": "expression", "formula": "=$B2>30", "style": {"fill": "#FFEB9C"}}})),
     r'type="expression"', r"calcext:condition[^>]*formula-is|calcext:conditional-format|style:map"),
    ("cf-colorscale", "Conditional format: colour scale", xbase(cmd("home.conditionalFormat", {"range": "C2:C6", "rule": {"type": "colorScale", "colors": ["#F8696B", "#FFEB84", "#63BE7B"]}})),
     r"colorScale", r"calcext:color-scale"),
    ("cf-databar", "Conditional format: data bar", xbase(cmd("home.conditionalFormat", {"range": "B2:B6", "rule": {"type": "dataBar", "color": "#5B9BD5"}})),
     r"dataBar", r"calcext:data-bar"),
    ("cf-iconset", "Conditional format: icon set", xbase(cmd("home.conditionalFormat", {"range": "B2:B6", "rule": {"type": "iconSet", "set": "3TrafficLights1"}})),
     r"iconSet", r"calcext:icon-set"),
    ("cf-top10-dup", "Conditional format: top 10 and duplicates", xbase(cmd("home.conditionalFormat", {"range": "B2:B6", "rule": {"type": "top10", "rank": 2, "preset": "greenFill"}}), cmd("home.conditionalFormat", {"range": "D2:D6", "rule": {"type": "duplicate", "preset": "yellowFill"}})),
     r'type="top10"', r"top-elements|duplicate"),
    ("hyperlink", "Hyperlink to a URL", xbase(cmd("insert.link", {"cell": "F1", "target": "https://portal.example.test/xtoklink", "text": "Portal"})),
     r"xtoklink", r'xlink:href="https://portal\.example\.test/xtoklink"'),
    ("hyperlink-internal", "Hyperlink to a cell in another sheet", xbase(cmd("home.insertSheet", {"name": "Target"}), cmd("sheet.activate", {"sheet": "Probe"}), cmd("insert.link", {"cell": "F1", "target": "Target!A1", "text": "XTOKJUMP"})),
     r'location="Target!A1"', r'xlink:href="#Target\.A1"'),
    ("note", "Legacy note", xbase(cmd("review.newNote", {"cell": "B2", "text": "XTOKNOTE checked against the ledger", "author": "R. Example"})),
     r"XTOKNOTE", r"<office:annotation.*?XTOKNOTE"),
    ("threaded-comment", "Threaded comment with reply", xbase(cmd("review.newComment", {"cell": "C2", "text": "XTOKTHREAD please confirm", "author": "R. Example"}), cmd("review.replyComment", {"cell": "C2", "text": "XTOKREPLY confirmed", "author": "T. Sample"})),
     r"XTOKTHREAD", r"<office:annotation.*?XTOKTHREAD", r"XTOKTHREAD.*XTOKREPLY"),
    # objects
    ("chart-column", "Column chart", xbase(cmd("insert.chart", {"range": "A1:B6", "type": "column", "title": "XTOKCHART", "at": "F2"})),
     r"<c:barChart", r"chart:class=\"chart:bar\"|draw:object"),
    ("chart-line-pie", "Line and pie charts", xbase(cmd("insert.chart", {"range": "A1:C6", "type": "line", "title": "Trend", "at": "F2"}), cmd("insert.chart", {"range": "A1:B6", "type": "pie", "title": "Share", "at": "F20"})),
     r"<c:pieChart", r"chart:pie|draw:object"),
    ("chart-combo-modern", "Waterfall chart (chartex)", xbase(cmd("insert.chart", {"range": "A1:B6", "type": "waterfall", "title": "Bridge", "at": "F2"})),
     r"chartex|cx:chart", r"draw:object"),
    ("sparkline", "Sparklines", xbase(cmd("insert.sparkline", {"range": "B2:C2", "location": "E2", "type": "line"})),
     r"x14:sparklineGroup", r"calcext:sparkline"),
    ("picture", "Embedded picture", xbase(cmd("insert.picture", {"base64": PNG_B64, "mime": "image/png", "at": "F2", "alt": "XTOKPIC"})),
     r"<xdr:pic", r"<draw:image"),
    ("shape-textbox", "Shape and text box", xbase(cmd("insert.shape", {"kind": "roundedRectangle", "at": "F2", "text": "XTOKSHAPE"}), cmd("insert.textBox", {"at": "F10", "text": "XTOKTEXTBOX"})),
     r"XTOKSHAPE", r"XTOKSHAPE.*XTOKTEXTBOX", r"<draw:(custom-shape|frame)\b.*XTOKSHAPE"),
    ("pivot", "Pivot table", xbase(cmd("insert.pivotTable", {"source": "Probe!$A$1:$D$6", "destination": "Probe!H3", "rows": ["Region"], "values": [{"field": "Units", "func": "sum"}], "name": "XTOKPIVOT"})),
     r"pivotTableDefinition", r"table:data-pilot-table"),
    ("slicer", "Table slicer", xbase(cmd("insert.table", {"range": "A1:D6", "header": True, "name": "Sliced"}), cmd("insert.slicer", {"table": "Sliced", "column": "Region", "at": "H2"})),
     r"slicer", r"slicer"),
    ("checkbox", "Cell checkboxes", xbase(xvalues("E2:E3", [[True], [False]]), cmd("insert.checkbox", {"range": "E2:E3"})),
     r"checkbox|featurePropertyBag|<v>1</v>", r'office:boolean-value="true"'),
    ("properties", "Document properties", xbase(cmd("file.properties", {"title": "XTOKTITLE", "author": "Compatibility Test Fixture", "company": "Example Works Ltd.", "keywords": "fictional"})),
     r"XTOKTITLE", r"<dc:title>XTOKTITLE</dc:title>"),
]


# --------------------------------------------------------------------------------------------
# Text probes.

def para(commands, value, style=None):
    if style:
        commands.append(cmd("para.style", {"style": style}))
    commands.extend((cmd("text.insert", {"text": value}), cmd("text.newParagraph")))
    if style:
        commands.append(cmd("para.style", {"style": "Normal"}))


def dbase(*extra):
    c = []
    para(c, "FICTIONAL TEST DOCUMENT - Compatibility probe", "Title")
    para(c, "Example Works Ltd. | All people, dates and figures in this file are fabricated.")
    c.extend(extra)
    return c


def table_cmds(rows, style="Table Grid", header_fill=None, repeat=False):
    c = [cmd("insert.table", {"rows": len(rows), "cols": len(rows[0]), "style": style})]
    if header_fill or repeat:
        c.append(cmd("table.selectRow"))
        if header_fill:
            c.append(cmd("table.shading", {"color": header_fill}))
        if repeat:
            c.append(cmd("table.repeatHeader"))
        c.append(cmd("select.collapse"))
    for r, row in enumerate(rows):
        for k, value in enumerate(row):
            c.append(cmd("text.insert", {"text": str(value)}))
            if r != len(rows) - 1 or k != len(row) - 1:
                c.append(cmd("text.tab"))
    c.append(cmd("caret.docEnd"))
    return c


def fmt_run(props, text):
    """Type `text` with character formatting on, then switch it off again."""
    on = [cmd(k, v) for k, v in props]
    off = [cmd(k, {"value": False}) for k, v in props if k in ("format.bold", "format.italic")]
    return [*on, cmd("text.insert", {"text": text}), *off, cmd("format.clear"), cmd("text.insert", {"text": " plain text after."}), cmd("text.newParagraph")]


def dtext(text):
    return [cmd("text.insert", {"text": text}), cmd("text.newParagraph")]


SAMPLE_ROWS = [["Workstream", "Owner", "Status", "Due"],
               ["Support", "R. Example", "On track", "20 Nov"],
               ["Training", "J. Placeholder", "At risk", "24 Nov"],
               ["Facilities", "A. Fiction", "Done", "26 Nov"]]

DOCX_PROBES = [
    ("paragraphs", "Plain paragraphs", dbase(*dtext("XTOKPARA plain body text.")),
     r"XTOKPARA", r"<text:p[^>]*>XTOKPARA"),
    ("headings", "Heading 1–3 with outline levels", dbase(*(c for lvl in (1, 2, 3) for c in (cmd("para.style", {"style": f"Heading {lvl}"}), *dtext(f"XTOKH{lvl} heading"), cmd("para.style", {"style": "Normal"})))),
     r'pStyle w:val="Heading3"', r'<text:h[^>]*text:outline-level="3"[^>]*>XTOKH3'),
    ("title-subtitle", "Title and Subtitle styles", dbase(cmd("para.style", {"style": "Subtitle"}), *dtext("XTOKSUB")),
     r'w:val="Subtitle"', r"Subtitle.*XTOKSUB|XTOKSUB"),
    ("bold-italic", "Bold, italic, underline", dbase(*fmt_run([("format.bold", {"value": True}), ("format.italic", {"value": True}), ("format.underline", {})], "XTOKBIU")),
     r"<w:b/>", r'fo:font-weight="bold".*fo:font-style="italic"'),
    ("char-color-size", "Font, size and colour", dbase(*fmt_run([("format.font", {"name": "Georgia"}), ("format.size", {"size": 16}), ("format.color", {"color": "C00000"})], "XTOKFONT")),
     r"Georgia", r'fo:color="#c00000"'),
    ("highlight", "Highlight and shading", dbase(*fmt_run([("format.highlight", {"color": "yellow"})], "XTOKHI")),
     r'w:highlight w:val="yellow"', r'fo:background-color="#ffff00"'),
    ("strike-caps", "Strikethrough, small caps, all caps", dbase(*fmt_run([("format.strikethrough", {}), ("format.smallCaps", {})], "XTOKSC"), *fmt_run([("format.allCaps", {})], "xtokcaps")),
     r"<w:smallCaps/>", r'fo:font-variant="small-caps".*|style:text-line-through-style'),
    ("super-sub", "Superscript and subscript", dbase(*fmt_run([("format.superscript", {})], "XTOKSUP"), *fmt_run([("format.subscript", {})], "XTOKSUB")),
     r'vertAlign w:val="superscript"', r"style:text-position=\"super"),
    ("alignment", "Centred and justified paragraphs", dbase(cmd("para.alignCenter"), *dtext("XTOKCENTER"), cmd("para.justify"), *dtext("XTOKJUST " * 30)),
     r'w:jc w:val="center"', r'fo:text-align="center"'),
    ("spacing", "Line spacing and space before/after", dbase(cmd("para.lineSpacing", {"value": 1.5}), cmd("para.spacing", {"before": 12, "after": 18}), *dtext("XTOKSPACE")),
     r'w:after="360"', r'fo:margin-bottom="(0\.25in|0\.635cm|18pt)"'),
    ("indents", "Left, right and hanging indents", dbase(cmd("para.indents", {"left": 36, "right": 18, "firstLine": -18}), *dtext("XTOKINDENT " * 20)),
     r'w:hanging="360"', r'fo:text-indent="-'),
    ("para-borders-shading", "Paragraph borders and shading", dbase(cmd("para.borders", {"kind": "outside", "color": "1F4E79"}), cmd("para.shading", {"color": "DEEAF6"}), *dtext("XTOKBOX")),
     r"<w:pBdr>", r'fo:border[^=]*="[^"]*#1f4e79".*|fo:background-color="#deeaf6"'),
    ("tabs", "Tab stops with leaders", dbase(cmd("para.tabs", {"tabs": [{"pos": 216, "align": "right", "leader": "dot"}]}), cmd("text.insert", {"text": "XTOKTAB"}), cmd("text.tab"), *dtext("42")),
     r'w:leader="dot"', r"style:tab-stop[^>]*style:leader-text=\"\.\"|style:tab-stop[^>]*leader"),
    ("keep-pagebreak", "Keep with next and page break before", dbase(cmd("para.keepNext", {"value": True}), *dtext("XTOKKEEP"), cmd("para.keepNext", {"value": False}), cmd("para.pageBreakBefore", {"value": True}), *dtext("XTOKBREAKBEFORE")),
     r"<w:pageBreakBefore/>", r'fo:break-before="page"'),
    ("page-break", "Manual page break", dbase(*dtext("Before the break."), cmd("text.pageBreak"), *dtext("XTOKAFTERBREAK")),
     r'w:br w:type="page"', r'fo:break-before="page"'),
    ("line-break", "Manual line break", dbase(cmd("text.insert", {"text": "XTOKLINE1"}), cmd("text.lineBreak"), *dtext("XTOKLINE2")),
     r"<w:br/>", r"XTOKLINE1<text:line-break/>XTOKLINE2"),
    ("bullets", "Bulleted list", dbase(cmd("para.bullets", {"kind": "bullet"}), *dtext("XTOKBUL1"), *dtext("XTOKBUL2"), cmd("para.bullets", {"off": True})),
     r"<w:numPr>", r"<text:list\b.*XTOKBUL1"),
    ("numbering", "Numbered list", dbase(cmd("para.numbering", {"kind": "numbered"}), *dtext("XTOKNUM1"), *dtext("XTOKNUM2"), cmd("para.numbering", {"off": True})),
     r'w:numFmt w:val="decimal"', r"text:list-level-style-number.*XTOKNUM1|XTOKNUM1"),
    ("multilevel", "Multilevel list", dbase(cmd("para.multilevel", {"kind": "outline"}), *dtext("XTOKML1"), cmd("para.listLevel", {"level": 1}), *dtext("XTOKML2"), cmd("para.listLevel", {"level": 2}), *dtext("XTOKML3"), cmd("para.numbering", {"off": True})),
     r'w:ilvl w:val="2"', r"<text:list\b.*<text:list\b.*<text:list\b.*XTOKML3"),
    ("quote-styles", "Quote, Intense Quote and No Spacing styles", dbase(cmd("para.style", {"style": "Quote"}), *dtext("XTOKQUOTE"), cmd("para.style", {"style": "Intense Quote"}), *dtext("XTOKIQ"), cmd("para.style", {"style": "No Spacing"}), *dtext("XTOKNOSP"), cmd("para.style", {"style": "Normal"})),
     r'w:val="IntenseQuote"', r'style:display-name="Intense Quote"|style:name="Intense_20_Quote"'),
    ("table", "Simple table", dbase(*table_cmds(SAMPLE_ROWS)),
     r"<w:tbl>", r"<table:table\b.*At risk"),
    ("table-styled", "Table style, header shading, repeat header", dbase(*table_cmds(SAMPLE_ROWS, "Grid Table 4 – Blue", "1F4E79", True)),
     r"<w:tblHeader/>", r"<table:table-header-rows>"),
    ("table-merge", "Merged table cells", dbase(cmd("insert.table", {"rows": 3, "cols": 3, "style": "Table Grid"}), cmd("table.selectRow"), cmd("table.merge"), cmd("text.insert", {"text": "XTOKMERGED"}), cmd("caret.docEnd")),
     r"<w:gridSpan w:val=\"3\"/>", r'table:number-columns-spanned="3"'),
    ("table-large", "Table with 500 rows", dbase(*table_cmds([["ID", "Name", "Value"]] + [[f"R{i:04d}", f"Item {i}", str(i * 7)] for i in range(1, 501)])),
     r"R0500", r"R0500"),
    ("table-nested", "Text before, between and after tables", dbase(*table_cmds(SAMPLE_ROWS), *dtext("XTOKBETWEEN"), *table_cmds(SAMPLE_ROWS)),
     r"XTOKBETWEEN", r"</table:table>\s*<text:p[^>]*>XTOKBETWEEN</text:p>\s*<table:table"),
    ("hyperlink", "External hyperlink", dbase(cmd("text.insert", {"text": "See "}), cmd("insert.link", {"url": "https://portal.example.test/xtoklink", "text": "the guide"}), cmd("text.newParagraph")),
     r"xtoklink", r'xlink:href="https://portal\.example\.test/xtoklink"'),
    ("bookmark-xref", "Bookmark and cross-reference", dbase(cmd("text.insert", {"text": "XTOKTARGET"}), cmd("insert.bookmark", {"name": "XTOKBM"}), cmd("text.newParagraph"), cmd("text.insert", {"text": "See "}), cmd("insert.crossReference", {"to": "bookmark", "target": "XTOKBM"}), cmd("text.newParagraph")),
     r'w:name="XTOKBM"', r'text:bookmark(-start)? text:name="XTOKBM"'),
    ("internal-link", "Link to a bookmark", dbase(cmd("text.insert", {"text": "XTOKANCHOR"}), cmd("insert.bookmark", {"name": "XTOKANCH"}), cmd("text.newParagraph"), cmd("insert.link", {"bookmark": "XTOKANCH", "text": "jump"}), cmd("text.newParagraph")),
     r'w:anchor="XTOKANCH"', r'xlink:href="#XTOKANCH"'),
    ("header-footer", "Header and footer", dbase(cmd("insert.header", {"text": "XTOKHEADER"}), cmd("insert.footer", {"text": "XTOKFOOTER"})),
     r"XTOKHEADER", r"<style:header>.*XTOKHEADER.*<style:footer>.*XTOKFOOTER|XTOKHEADER.*XTOKFOOTER"),
    ("page-number", "Page X of Y in footer", dbase(*dtext("Body"), cmd("insert.pageNumber", {"position": "bottom", "align": "right", "format": "x of y"})),
     r"NUMPAGES", r"<text:page-number\b.*<text:page-count"),
    ("first-page-header", "Different first page header", dbase(cmd("insert.header", {"text": "XTOKODDHEAD"}), cmd("layout.differentFirstPage")),
     r"<w:titlePg/>", r"style:header-first|XTOKODDHEAD"),
    ("fields-date", "Date and author fields", dbase(cmd("insert.dateField"), cmd("text.newParagraph"), cmd("insert.field", {"instr": "AUTHOR", "result": "Compatibility Test Fixture"}), cmd("text.newParagraph")),
     r"DATE|AUTHOR", r"<text:date\b|<text:author|<text:initial-creator"),
    ("toc", "Table of contents", dbase(cmd("references.toc", {"levels": 3, "title": "Contents"}), cmd("caret.docEnd"), cmd("para.style", {"style": "Heading 1"}), *dtext("XTOKTOC1"), cmd("para.style", {"style": "Heading 2"}), *dtext("XTOKTOC2"), cmd("references.updateToc")),
     r"TOC \\o", r"<text:table-of-content\b"),
    ("footnote", "Footnote", dbase(cmd("text.insert", {"text": "A claim."}), cmd("references.footnote", {"text": "XTOKFOOT source note"}), cmd("insert.closeHeader"), cmd("text.newParagraph")),
     r"XTOKFOOT", r'<text:note text:id="[^"]*" text:note-class="footnote">.*?XTOKFOOT'),
    ("endnote", "Endnote", dbase(cmd("text.insert", {"text": "A claim."}), cmd("references.endnote", {"text": "XTOKEND source note"}), cmd("insert.closeHeader"), cmd("text.newParagraph")),
     r"XTOKEND", r'text:note-class="endnote">.*?XTOKEND'),
    ("caption", "Table and figure captions", dbase(*table_cmds(SAMPLE_ROWS), cmd("references.caption", {"label": "Table", "text": " XTOKCAPTION"})),
     r"SEQ Table", r"<text:sequence\b"),
    ("comment", "Comment", dbase(*dtext("The audit group verified 14 records."), cmd("select.text", {"text": "14 records"}), cmd("review.newComment", {"text": "XTOKCOMMENT synthetic count"})),
     r"XTOKCOMMENT", r"<office:annotation\b.*?XTOKCOMMENT"),
    ("comment-reply", "Comment reply thread", dbase(*dtext("The audit group verified 14 records."), cmd("select.text", {"text": "audit group"}), cmd("review.newComment", {"text": "XTOKROOT question"}), cmd("review.reply", {"id": 0, "text": "XTOKREPLY answer"})),
     r"XTOKREPLY", r"XTOKROOT", r"XTOKROOT.*XTOKREPLY"),
    ("track-insert", "Tracked insertion", dbase(*dtext("The sample audit group met."), cmd("review.trackChanges", {"value": True}), cmd("select.text", {"text": "met"}), cmd("text.insert", {"text": "XTOKINS reconvened"})),
     r"<w:ins\b", r"<text:tracked-changes\b.*<text:insertion"),
    ("track-delete", "Tracked deletion", dbase(*dtext("The XTOKDEL sample group met."), cmd("review.trackChanges", {"value": True}), cmd("select.text", {"text": "XTOKDEL "}), cmd("text.delete")),
     r"<w:del\b", r"<text:deletion"),
    ("track-format", "Tracked formatting change", dbase(*dtext("XTOKFMTCHG sample."), cmd("review.trackChanges", {"value": True}), cmd("select.text", {"text": "XTOKFMTCHG"}), cmd("format.bold", {"value": True})),
     r"<w:rPrChange\b", r'<text:span text:style-name="[^"]+">XTOKFMTCHG', r"<text:format-change"),
    ("landscape-section", "Landscape section between portrait ones", dbase(*dtext("Portrait page."), cmd("layout.break", {"kind": "nextPage"}), cmd("layout.orientation", {"value": "landscape"}), *dtext("XTOKLANDSCAPE wide table page."), cmd("layout.break", {"kind": "nextPage"}), cmd("layout.orientation", {"value": "portrait"}), *dtext("Portrait again.")),
     r'w:orient="landscape"', r'style:print-orientation="landscape"'),
    ("paper-a4-margins", "A4 paper and custom margins", dbase(cmd("layout.size", {"name": "A4"}), cmd("layout.margins", {"top": 54, "bottom": 54, "left": 72, "right": 72})),
     r'w:w="11906"', r'fo:page-width="(595\.\d+pt|8\.26|21(\.0)?cm|210)'),
    ("columns", "Two-column section", dbase(cmd("layout.columns", {"count": 2, "space": 18, "separator": True}), *dtext("XTOKCOL " * 200)),
     r'w:num="2"', r'fo:column-count="2"'),
    ("line-numbers", "Line numbering", dbase(cmd("layout.lineNumbers", {"value": "continuous"}), *dtext("XTOKLN " * 50)),
     r"<w:lnNumType\b", r'text:number-lines="true"'),
    ("picture", "Inline picture", dbase(cmd("insert.picture", {"data": PNG_B64, "width": 144, "alt": "XTOKPIC"}), cmd("text.newParagraph")),
     r"<pic:pic\b", r"<draw:image\b"),
    ("text-box", "Floating text box", dbase(cmd("insert.textBox", {"text": "XTOKTEXTBOX", "width": 144, "height": 72})),
     r"XTOKTEXTBOX", r"XTOKTEXTBOX", r"<draw:text-box\b.*XTOKTEXTBOX"),
    ("shape", "Drawing shape", dbase(cmd("insert.shape", {"kind": "roundedRectangle", "width": 144, "height": 72, "fill": "1F4E79"})),
     r"<wps:wsp\b", r"<draw:custom-shape\b|<draw:rect\b"),
    ("chart", "Embedded chart", dbase(cmd("insert.chart", {"type": "column", "title": "XTOKCHART", "data": {"categories": ["Q1", "Q2", "Q3"], "series": [{"name": "Plan", "values": [10, 12, 14]}]}})),
     r"<c:chart\b", r"<draw:object\b|chart:chart"),
    ("equation", "Office Math equation", dbase(cmd("insert.equation", {"builtin": "quadratic", "edit": False}), cmd("text.newParagraph")),
     r"<m:oMath", r"<math:math\b|<draw:object\b"),
    ("watermark", "Watermark", dbase(cmd("design.watermark", {"text": "XTOKDRAFT"})),
     r"XTOKDRAFT", r"XTOKDRAFT"),
    ("drop-cap", "Drop cap", dbase(*dtext("XTOKDROP once upon a fictional time " * 6), cmd("caret.up"), cmd("insert.dropCap", {"lines": 3})),
     r"w:dropCap", r"<style:drop-cap\b"),
    ("custom-style", "Custom paragraph style", dbase(cmd("text.insert", {"text": "XTOKSTYLED"}), cmd("format.color", {"color": "7030A0"}), cmd("styles.create", {"name": "Callout Text", "fromSelection": True}), cmd("text.newParagraph")),
     r'w:styleId="CalloutText"', r'style:display-name="Callout Text"|style:name="Callout_20_Text"'),
    ("content-controls", "Checkbox and drop-down content controls", dbase(cmd("developer.checkBox", {"title": "Approved", "checked": True}), cmd("text.newParagraph"), cmd("developer.dropDown", {"title": "Status", "items": ["Draft", "Final"]}), cmd("text.newParagraph")),
     r"<w:sdt\b", r"☒|Draft", r"<form:|<draw:control\b|<text:drop-down\b"),
    ("symbols", "Symbols, special characters, unicode", dbase(*dtext("XTOKSYM © ® ™ — … € ½ Grüße 😀 مرحبا 日本語"), cmd("text.nbsp"), cmd("text.nbHyphen")),
     r"XTOKSYM", r"XTOKSYM © ® ™ — … € ½ Grüße 😀 مرحبا 日本語"),
    ("properties", "Document properties", dbase(cmd("file.properties", {"title": "XTOKTITLE", "subject": "Compatibility", "keywords": "fictional", "custom": {"Department": "Example Works"}})),
     r"XTOKTITLE", r"<dc:title>XTOKTITLE</dc:title>"),
]


# --------------------------------------------------------------------------------------------
# Scale tiers, modelled on the Northstar workbook and playbook.

REGIONS = ["North", "South", "East", "West", "Central", "Imaginary Isles"]
UNITS = ["Advisory", "Devices", "Cloud Services", "Managed Ops", "Training"]
CHANNELS = ["Direct", "Partner", "Online", "Reseller"]
STATUSES = ["Not started", "In progress", "At risk", "On hold", "Done"]
PRIORITIES = ["Low", "Medium", "High", "Critical"]
CATEGORIES = ["Compliance", "Delivery", "Financial", "People", "Technology", "Vendor"]
DEPARTMENTS = ["Finance", "Operations", "Platform", "Sales", "Service", "People"]
EPOCH = date(1899, 12, 30)


def serial(d):
    return (d - EPOCH).days


def xlsx_tier(rows, seed=2026):
    rnd = random.Random(seed)
    projects, risks, people = max(60, rows // 20), max(40, rows // 30), max(120, rows // 10)
    last = rows + 4
    c = [cmd("file.new", {}), cmd("file.properties", {"title": f"Northstar-style enterprise workbook ({rows} rows)", "author": "Compatibility Test Fixture", "company": "Example Works Ltd. (fictional)"})]

    # Transactions: the big table.
    c += [cmd("sheet.rename", {"name": "Transactions"}),
          xvalues("A1:A2", [["TRANSACTION LEDGER"], [f"{rows:,} synthetic line items • FICTIONAL TEST DATA • FCU = fictional currency units"]])]
    header = ["Transaction ID", "Date", "Region", "Business Unit", "Customer Code", "Product Code", "Channel", "Units", "Unit Price (FCU)", "Gross Value (FCU)", "Discount %", "Net Value (FCU)"]
    data = [header]
    start = date(2025, 1, 1)
    for i in range(rows):
        r = i + 5
        data.append([f"TX-{i + 1:06d}", serial(start + timedelta(days=rnd.randrange(730))), rnd.choice(REGIONS), rnd.choice(UNITS),
                     f"CUST-{rnd.randrange(1, 900):04d}", f"SKU-{rnd.randrange(1, 120):04d}", rnd.choice(CHANNELS), rnd.randrange(1, 120),
                     round(rnd.uniform(4, 7000), 2), f"=H{r}*I{r}", round(rnd.uniform(0, 0.25), 4), f"=J{r}*(1-K{r})"])
    chunk = 5000
    for at in range(0, len(data), chunk):
        part = data[at:at + chunk]
        c.append(cmd("range.setValues", {"range": f"A{4 + at}:L{4 + at + len(part) - 1}", "values": part, "parse": False}))
    c += [cmd("insert.table", {"range": f"A4:L{last}", "header": True, "style": "TableStyleMedium2", "name": "TransactionLedger"}),
          cmd("home.numberFormat", {"range": f"B5:B{last}", "format": "Short Date"}),
          cmd("home.numberFormat", {"range": f"I5:J{last}", "code": "#,##0.00"}),
          cmd("home.numberFormat", {"range": f"L5:L{last}", "code": "#,##0.00"}),
          cmd("home.numberFormat", {"range": f"K5:K{last}", "format": "Percentage"}),
          cmd("home.mergeCells", {"range": "A1:L1"}), cmd("home.mergeCells", {"range": "A2:L2"}),
          cmd("home.fontSize", {"range": "A1", "size": 18}), cmd("home.bold", {"range": "A1", "on": True}),
          cmd("home.columnWidth", {"cols": "A:L", "chars": 16}),
          cmd("home.conditionalFormat", {"range": f"L5:L{last}", "rule": {"type": "dataBar", "color": "#5B9BD5"}}),
          cmd("view.freezePanes", {"cell": "A5"}), cmd("view.gridlines", {"on": False}), cmd("view.zoom", {"percent": 90}),
          cmd("pageLayout.orientation", {"orientation": "landscape"}), cmd("pageLayout.printTitles", {"rows": "4:4"}),
          cmd("pageLayout.headerFooter", {"header": "&C&BNORTHSTAR DYNAMICS — SYNTHETIC TEST WORKBOOK", "footer": "&CPage &P of &N  |  Fictional data only"})]

    # Project portfolio.
    c.append(cmd("home.insertSheet", {"name": "Project Portfolio"}))
    data = [["Project ID", "Project Name", "Sponsor", "Workstream", "Region", "Status", "Priority", "Start Date", "End Date", "Budget (FCU)", "Actual (FCU)", "Variance (FCU)", "Risk Score"]]
    for i in range(projects):
        r = i + 5
        s = start + timedelta(days=rnd.randrange(500))
        budget = rnd.randrange(20, 900) * 1000
        data.append([f"PRJ-{i + 1:04d}", f"Project {i + 1:03d} — Sample Initiative", f"Sponsor {rnd.randrange(1, 12)}", rnd.choice(UNITS), rnd.choice(REGIONS), rnd.choice(STATUSES), rnd.choice(PRIORITIES),
                     serial(s), serial(s + timedelta(days=rnd.randrange(60, 400))), budget, round(budget * rnd.uniform(0.6, 1.3), 2), f"=J{r}-K{r}", rnd.randrange(1, 26)])
    c += [xvalues("A1:A2", [["PROJECT PORTFOLIO REGISTER"], ["Initiative-level records with statuses, dates, budget, risk and ownership"]]),
          cmd("range.setValues", {"range": f"A4:M{projects + 4}", "values": data, "parse": False}),
          cmd("insert.table", {"range": f"A4:M{projects + 4}", "header": True, "style": "TableStyleMedium9", "name": "Portfolio"}),
          cmd("home.numberFormat", {"range": f"H5:I{projects + 4}", "format": "Short Date"}),
          cmd("home.numberFormat", {"range": f"J5:L{projects + 4}", "format": "Currency"}),
          cmd("home.conditionalFormat", {"range": f"L5:L{projects + 4}", "rule": {"type": "colorScale", "colors": ["#F8696B", "#FFEB84", "#63BE7B"]}}),
          cmd("home.conditionalFormat", {"range": f"M5:M{projects + 4}", "rule": {"type": "iconSet", "set": "3TrafficLights1"}}),
          cmd("data.validation", {"range": f"F5:F{projects + 4}", "type": "list", "formula1": '"' + ",".join(STATUSES) + '"'}),
          cmd("data.validation", {"range": f"G5:G{projects + 4}", "type": "list", "formula1": '"' + ",".join(PRIORITIES) + '"'}),
          cmd("view.freezePanes", {"cell": "C5"}),
          cmd("formulas.defineName", {"name": "PortfolioBudget", "refersTo": f"='Project Portfolio'!$J$5:$J${projects + 4}"})]

    # Monthly financials, 36 months, with a chart and sparklines.
    c.append(cmd("home.insertSheet", {"name": "Monthly Financials"}))
    data = [["Month", "Revenue Plan", "Revenue Actual", "Revenue Forecast", "Cost Plan", "Cost Actual", "Gross Margin Plan", "Gross Margin Actual", "Variance", "Commentary"]]
    for m in range(36):
        r = m + 5
        plan = 1_000_000 + m * 9_500 + rnd.randrange(-40_000, 40_000)
        cost = int(plan * rnd.uniform(0.42, 0.5))
        data.append([serial(date(2025 + m // 12, m % 12 + 1, 1)), plan, round(plan * rnd.uniform(0.92, 1.06), 2), round(plan * rnd.uniform(0.95, 1.03), 2), cost,
                     round(cost * rnd.uniform(0.9, 1.08), 2), f"=B{r}-E{r}", f"=C{r}-F{r}", f"=C{r}-B{r}", rnd.choice(["On plan", "Seasonal dip", "Large renewal", "FX effect (fictional)", "Pipeline slip"])])
    data.append(["Total", "=SUM(B5:B40)", "=SUM(C5:C40)", "=SUM(D5:D40)", "=SUM(E5:E40)", "=SUM(F5:F40)", "=SUM(G5:G40)", "=SUM(H5:H40)", "=SUM(I5:I40)", None])
    c += [xvalues("A1:A2", [["MONTHLY FINANCIAL PLAN"], ["36 months of synthetic plan / actual / forecast data in FCU"]]),
          cmd("range.setValues", {"range": "A4:J41", "values": data, "parse": False}),
          cmd("home.numberFormat", {"range": "A5:A40", "code": "mmm yyyy"}),
          cmd("home.numberFormat", {"range": "B5:I41", "code": "#,##0;[Red]-#,##0"}),
          cmd("home.bold", {"range": "A41:I41", "on": True}), cmd("home.borders", {"range": "A41:I41", "preset": "topDoubleBottom"}),
          cmd("insert.chart", {"range": "A4:C40", "type": "line", "title": "Revenue plan vs actual", "at": "L4"}),
          cmd("insert.sparkline", {"range": "B5:B40", "location": "B42", "type": "line"}),
          cmd("data.group", {"rows": "5:16"}), cmd("data.group", {"rows": "17:28"})]

    # Risk register.
    c.append(cmd("home.insertSheet", {"name": "Risk Register"}))
    data = [["Risk ID", "Category", "Risk Statement", "Likelihood (1-5)", "Impact (1-5)", "Inherent Score", "Response", "Control Owner", "Due Date", "Residual Score", "Evidence Status"]]
    for i in range(risks):
        r = i + 5
        data.append([f"RSK-{i + 1:04d}", rnd.choice(CATEGORIES), f"Fictional scenario {i + 1}: possible disruption to a synthetic business objective.", rnd.randrange(1, 6), rnd.randrange(1, 6),
                     f"=D{r}*E{r}", rnd.choice(["Mitigate", "Accept", "Transfer", "Avoid"]), f"Owner {chr(65 + rnd.randrange(26))}. Example", serial(date(2026, 1, 1) + timedelta(days=rnd.randrange(540))),
                     f"=ROUND(F{r}*0.6,0)", rnd.choice(["Complete", "Partial", "Missing"])])
    c += [xvalues("A1:A2", [["ENTERPRISE RISK & CONTROL REGISTER"], ["Synthetic risk scenarios; likelihood × impact; control and mitigation evidence"]]),
          cmd("range.setValues", {"range": f"A4:K{risks + 4}", "values": data, "parse": False}),
          cmd("insert.table", {"range": f"A4:K{risks + 4}", "header": True, "style": "TableStyleLight9", "name": "Risks"}),
          cmd("home.wrapText", {"range": f"C5:C{risks + 4}", "on": True}), cmd("home.columnWidth", {"cols": "C:C", "chars": 50}),
          cmd("home.numberFormat", {"range": f"I5:I{risks + 4}", "format": "Short Date"}),
          cmd("home.conditionalFormat", {"range": f"F5:F{risks + 4}", "rule": {"type": "cellIs", "operator": "greaterOrEqual", "value": 15, "preset": "lightRedFillDarkRedText"}}),
          cmd("home.conditionalFormat", {"range": f"K5:K{risks + 4}", "rule": {"type": "containsText", "text": "Missing", "preset": "redText"}}),
          cmd("review.newNote", {"cell": "F4", "text": "Inherent score = likelihood × impact (fictional method).", "author": "Enterprise Planning Group"})]

    # People and capacity.
    c.append(cmd("home.insertSheet", {"name": "People & Capacity"}))
    data = [["Person Key", "Display Name", "Department", "Role Family", "Location", "FTE", "Annual Cost (FCU)", "Start Date", "Manager Key", "Training Status"]]
    for i in range(people):
        data.append([f"EMP-TEST-{i + 1:05d}", f"Test Person {i + 1:05d}", rnd.choice(DEPARTMENTS), rnd.choice(["Analyst", "Engineer", "Manager", "Specialist", "Consultant"]),
                     rnd.choice(["Example City", "Sampletown", "Fiction Bay", "Placeholder Park"]), rnd.choice([1, 1, 1, 0.8, 0.6, 0.5]), rnd.randrange(42, 160) * 1000,
                     serial(date(2012, 1, 1) + timedelta(days=rnd.randrange(5000))), f"EMP-TEST-{rnd.randrange(1, max(2, people // 12)):05d}", rnd.choice(["Complete", "In progress", "Overdue"])])
    c += [xvalues("A1:A2", [["PEOPLE & CAPACITY MODEL"], ["Fabricated employee-like records; no real people or identifiers"]]),
          cmd("range.setValues", {"range": f"A4:J{people + 4}", "values": data, "parse": False}),
          cmd("insert.table", {"range": f"A4:J{people + 4}", "header": True, "style": "TableStyleMedium4", "name": "People"}),
          cmd("home.numberFormat", {"range": f"G5:G{people + 4}", "format": "Currency"}),
          cmd("home.numberFormat", {"range": f"H5:H{people + 4}", "format": "Short Date"}),
          cmd("data.filter", {"range": f"A4:J{people + 4}"}),
          cmd("view.freezePanes", {"cell": "C5"})]

    # Reference data with edge cases.
    c.append(cmd("home.insertSheet", {"name": "Reference Data"}))
    ref = [["Domain", "Value", "Description", "Expected type", "Example"]]
    ref += [["Region", v, "Synthetic region classification", "Text", v] for v in REGIONS]
    ref += [["Status", v, "Delivery status", "Text", v] for v in STATUSES]
    ref += [["Edge case", "Leading zeros", "Must stay text", "Text", "000123"],
            ["Edge case", "Long text", "Wrapped paragraph in a cell", "Text", "Lorem-like fictional text. " * 20],
            ["Edge case", "Unicode", "Accents, CJK and emoji", "Text", "Grüße · 東京 · 😀 · مرحبا"],
            ["Edge case", "Negative zero", "Signed zero", "Number", "=-0"],
            ["Edge case", "Error", "Division by zero", "Error", "=1/0"],
            ["Edge case", "Boolean", "Logical value", "Boolean", "TRUE"],
            ["Edge case", "Formula text", "Looks like a formula", "Text", "'=not a formula"],
            ["Contact", "finance@example.test", "Fictional mailbox", "Text", "finance@example.test"]]
    c += [xvalues("A1:A2", [["REFERENCE DATA & TEST NOTES"], ["Controlled vocabularies, data dictionary and intentional edge cases"]]),
          xvalues(f"A4:E{len(ref) + 3}", ref),
          cmd("insert.link", {"cell": f"E{len(ref) + 3}", "target": "mailto:finance@example.test", "text": "finance@example.test"})]

    # The dashboard comes first and reads every other sheet.
    c += [cmd("home.insertSheet", {"name": "Executive Dashboard"}), cmd("sheet.move", {"sheet": "Executive Dashboard", "to": 0})]
    dash = [["NORTHSTAR DYNAMICS | PERFORMANCE & PORTFOLIO DASHBOARD", None, None, None, None, None],
            ["FICTIONAL DATA • FCU = fictional currency units • Reporting period: FY2026", None, None, None, None, None],
            [None] * 6,
            ["Portfolio metric", "Current", "Target", "Variance", "RAG", "Owner"]]
    metrics = [
        ("Net sales (FCU)", f"=SUM(Transactions!L5:L{last})", 4.0e8, "Finance"),
        ("Transactions", f"=COUNTA(Transactions!A5:A{last})", rows, "Sales Ops"),
        ("Average discount", f"=AVERAGE(Transactions!K5:K{last})", 0.12, "Pricing"),
        ("Net sales, North (FCU)", f'=SUMIFS(Transactions!L5:L{last},Transactions!C5:C{last},"North")', 6.0e7, "Region North"),
        ("Online orders", f'=COUNTIFS(Transactions!G5:G{last},"Online")', rows // 4, "Digital"),
        ("Largest order (FCU)", f"=MAX(Transactions!L5:L{last})", 500000, "Finance"),
        ("Projects at risk", f'=COUNTIF(\'Project Portfolio\'!F5:F{projects + 4},"At risk")', projects // 10, "PMO"),
        ("Portfolio budget (FCU)", "=SUM(PortfolioBudget)", 1.0e8, "PMO"),
        ("High risks", f'=COUNTIF(\'Risk Register\'!F5:F{risks + 4},">=15")', risks // 10, "Risk"),
        ("Headcount (FTE)", f"=SUM('People & Capacity'!F5:F{people + 4})", people * 0.9, "People"),
        ("Revenue actual, 36 months", "='Monthly Financials'!C41", 3.6e7, "Finance"),
        ("Lookup: first project", f"=XLOOKUP(\"PRJ-0001\",'Project Portfolio'!A5:A{projects + 4},'Project Portfolio'!B5:B{projects + 4})", None, "PMO"),
    ]
    for k, (label, formula, target, owner) in enumerate(metrics):
        r = k + 5
        dash.append([label, formula, target, f'=IF(ISNUMBER(C{r}),B{r}-C{r},"")', f'=IF(ISNUMBER(C{r}),IF(B{r}>=C{r},"GREEN","AMBER"),"n/a")', owner])
    c += [xvalues(f"A1:F{len(dash)}", dash),
          cmd("home.mergeCells", {"range": "A1:F1"}), cmd("home.fontSize", {"range": "A1", "size": 16}), cmd("home.bold", {"range": "A1:F1", "on": True}),
          cmd("home.fillColor", {"range": "A4:F4", "color": "#1F4E79"}), cmd("home.fontColor", {"range": "A4:F4", "color": "#FFFFFF"}), cmd("home.bold", {"range": "A4:F4", "on": True}),
          cmd("home.columnWidth", {"cols": "A:A", "chars": 38}), cmd("home.columnWidth", {"cols": "B:F", "chars": 16}),
          cmd("home.numberFormat", {"range": f"B5:D{len(dash)}", "code": "#,##0"}), cmd("home.numberFormat", {"range": "B7:D7", "format": "Percentage"}),
          cmd("home.conditionalFormat", {"range": f"E5:E{len(dash)}", "rule": {"type": "containsText", "text": "GREEN", "preset": "greenFill"}}),
          cmd("home.conditionalFormat", {"range": f"E5:E{len(dash)}", "rule": {"type": "containsText", "text": "AMBER", "preset": "yellowFill"}}),
          cmd("insert.chart", {"range": "A5:B6", "type": "column", "title": "Headline metrics", "at": "H4"}),
          cmd("review.newComment", {"cell": "B5", "text": "Net sales from the full ledger; recalculated on open (fictional).", "author": "Enterprise Planning Group"}),
          cmd("view.gridlines", {"on": False}), cmd("sheet.tabColor", {"sheet": "Executive Dashboard", "color": "#1F4E79"}), cmd("sheet.activate", {"sheet": "Executive Dashboard"})]
    return c


LOREM = ("Illustrative management commentary: the team should validate scope, confirm accountable ownership, and reconcile the reported position against the "
         "latest approved baseline. Any variance above the agreed tolerance should be supported by a quantified explanation, a named action owner, and a dated recovery milestone.")
CHAPTERS = ["Executive overview", "Strategic context", "Operating model", "Portfolio and delivery governance", "Financial plan", "People and organization",
            "Technology architecture", "Risk and controls", "Performance scorecard", "Implementation roadmap", "Communications", "Appendices and data dictionary"]


def docx_tier(chapters, seed=2026):
    rnd = random.Random(seed)
    c = [cmd("file.properties", {"title": f"Northstar-style operating playbook ({chapters} chapters)", "subject": "Compatibility test fixture", "keywords": "fictional; synthetic"}),
         cmd("layout.size", {"name": "A4"})]
    para(c, "NORTHSTAR DYNAMICS", "Title")
    para(c, "Enterprise Operating Model & Transformation Playbook", "Subtitle")
    para(c, "FY2026–FY2028 | Version 4.2 | Prepared 15 November 2026")
    c += [cmd("para.borders", {"kind": "outside", "color": "C00000"}), cmd("para.shading", {"color": "FDECEA"}), cmd("format.bold", {"value": True}),
          cmd("text.insert", {"text": "TEST FIXTURE NOTICE "}), cmd("format.bold", {"value": False}),
          cmd("text.insert", {"text": "This document is entirely fictional. All organizations, people, products, metrics, dates and financial values are synthetic."}),
          cmd("text.newParagraph"), cmd("para.borders", {"kind": "none"}), cmd("para.shading", {"color": None})]
    para(c, "Document control", "Heading 1")
    c += table_cmds([["Field", "Value", "Field", "Value"], ["Document ID", "NSD-STRAT-00042", "Classification", "Internal / Synthetic"],
                     ["Business owner", "Office of Transformation", "Author", "Enterprise Planning Group"], ["Version", "4.2", "Review cycle", "Quarterly"],
                     ["Approval status", "Illustrative — not approved", "Next review", "15 February 2027"]], "Grid Table 4 – Blue", "1F4E79")
    para(c, "Revision history", "Heading 2")
    c += table_cmds([["Version", "Date", "Owner", "Change summary", "Status"], ["1.0", "2025-04-01", "A. Example", "Initial operating model", "Archived"],
                     ["2.0", "2025-10-15", "B. Sample", "Added portfolio governance", "Archived"], ["3.1", "2026-04-30", "C. Placeholder", "Updated financial scenarios", "Superseded"],
                     ["4.2", "2026-11-15", "D. Testperson", "Expanded performance and compatibility fixtures", "Current test version"]], "Grid Table 4 – Blue", "1F4E79", True)
    c += [cmd("text.pageBreak"), cmd("references.toc", {"levels": 2, "title": "Contents"}), cmd("caret.docEnd"), cmd("text.pageBreak")]
    for n in range(chapters):
        name = CHAPTERS[n % len(CHAPTERS)]
        if n >= len(CHAPTERS):
            name += f" (part {n // len(CHAPTERS) + 1})"
        para(c, f"{n + 1}. {name}", "Heading 1")
        c += [cmd("text.insert", {"text": f"Chapter {n + 1} sets out how the fictional enterprise approaches {name.lower()}. "}),
              cmd("references.footnote", {"text": f"Synthetic note {n + 1}: figures are fabricated for compatibility testing."}),
              cmd("insert.closeHeader"),  # a new note leaves the caret in it
              cmd("text.insert", {"text": "The figures are deliberately synthetic and should never be interpreted as guidance. Further reading: "}),
              cmd("insert.link", {"url": f"https://intranet.example.test/playbook/ch{n + 1}", "text": "chapter resources"}),
              cmd("text.insert", {"text": "."}), cmd("text.newParagraph")]
        para(c, "Objectives and design principles", "Heading 2")
        c.append(cmd("para.bullets", {"kind": "bullet"}))
        c += dtext("Establish one accountable owner for every material outcome and decision.")
        c += dtext("Use consistent definitions across planning, delivery, finance and reporting.")
        c += dtext("Prefer measurable acceptance criteria over activity-based completion statements.")
        c.append(cmd("para.bullets", {"off": True}))
        para(c, "Key activities", "Heading 2")
        c.append(cmd("para.numbering", {"kind": "numbered"}))
        c += dtext("Confirm baseline, scope boundaries, assumptions and source-of-truth records.")
        c += dtext("Validate dependencies with adjacent teams and document unresolved decisions.")
        c += dtext("Publish a concise monthly status pack with exceptions and corrective actions.")
        c.append(cmd("para.numbering", {"off": True}))
        para(c, "Illustrative workstream register", "Heading 2")
        rows = [["Workstream ID", "Workstream", "Status", "Accountable owner", "Target", "Priority"]]
        for k in range(rnd.randrange(6, 14)):
            rows.append([f"WS-{n + 1:02d}-{k + 1:02d}", rnd.choice(UNITS), rnd.choice(STATUSES), f"Owner {chr(65 + rnd.randrange(6))}. Placeholder", f"2027-Q{rnd.randrange(1, 5)}", rnd.choice(PRIORITIES)])
        c += table_cmds(rows, "Grid Table 4 – Blue", "1F4E79", True)
        c += [cmd("references.caption", {"label": "Table", "text": f" Workstream register, chapter {n + 1}"}), cmd("caret.docEnd")]
        para(c, "Management commentary", "Heading 2")
        c += [cmd("para.justify"), cmd("text.insert", {"text": "\n".join([LOREM] * rnd.randrange(4, 9)), "paragraphs": True}), cmd("text.newParagraph"), cmd("para.alignLeft")]
        if n % 3 == 0:
            c += [cmd("select.text", {"text": f"Chapter {n + 1} sets out"}), cmd("review.newComment", {"text": f"Reviewer note {n + 1}: confirm wording with the fictional PMO."}), cmd("caret.docEnd")]
        if n % 4 == 1:
            c += [cmd("insert.picture", {"data": PNG_B64, "width": 288, "alt": f"Synthetic chart placeholder {n + 1}"}), cmd("text.newParagraph"),
                  cmd("references.caption", {"label": "Figure", "text": f" Synthetic placeholder figure {n + 1}"}), cmd("caret.docEnd")]
        if n % 6 == 5:
            c += [cmd("insert.chart", {"type": "column", "title": f"Plan vs actual, chapter {n + 1}", "data": {"categories": ["Q1", "Q2", "Q3", "Q4"],
                  "series": [{"name": "Plan", "values": [rnd.randrange(50, 100) for _ in range(4)]}, {"name": "Actual", "values": [rnd.randrange(50, 100) for _ in range(4)]}]}}),
                  cmd("caret.docEnd"), cmd("text.newParagraph")]
        c.append(cmd("text.pageBreak"))
    para(c, "Appendix A. Data dictionary", "Heading 1")
    rows = [["Field", "Type", "Description", "Example"]] + [[f"field_{i:04d}", rnd.choice(["Text", "Number", "Date", "Code"]), f"Synthetic attribute {i} of the fictional data model.", f"EX-{i:05d}"] for i in range(chapters * 15)]
    c += table_cmds(rows, "Table Grid", "D9E2F3", True)
    c += [cmd("references.updateToc"), cmd("insert.header", {"text": "NORTHSTAR DYNAMICS | ENTERPRISE PLAYBOOK | FICTIONAL"}),
          cmd("insert.pageNumber", {"position": "bottom", "align": "right", "format": "x of y"}), cmd("layout.differentFirstPage")]
    return c


XLSX_TIERS = [1_000, 12_000, 50_000, 100_000, 200_000]
DOCX_TIERS = [12, 48, 150, 450]
# WordCraft slows down superlinearly with script length and refuses scripts over 4 MiB, so a
# tier past this many chapters is this tier's file appended to itself.
DOCX_BLOCK = 150


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("out", nargs="?", default="target/craft-corpus")
    parser.add_argument("--only", choices=["probes", "scale"], help="build one half only")
    parser.add_argument("--xlsx-tiers", type=lambda s: [int(x) for x in s.split(",") if x], default=XLSX_TIERS)
    parser.add_argument("--docx-tiers", type=lambda s: [int(x) for x in s.split(",") if x], default=DOCX_TIERS)
    parser.add_argument("--filter", default="", help="build only probes whose id contains this")
    args = parser.parse_args()
    if not WORDCRAFT or not GRIDCRAFT:
        raise SystemExit("wordcraft-cli and gridcraft-cli must be on PATH")
    out = Path(args.out)
    manifest_path = out / "manifest.json"
    manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {"probes": [], "scale": []}
    failures = []

    if args.only in (None, "probes"):
        for kind, probes, runner in (("xlsx", XLSX_PROBES, run_gridcraft), ("docx", DOCX_PROBES, run_wordcraft)):
            (out / "probes" / kind).mkdir(parents=True, exist_ok=True)
            for pid, label, commands, src, odf, *exact in probes:
                if args.filter not in pid:
                    continue
                path = out / "probes" / kind / f"{pid}.{kind}"
                entry = {"id": f"{kind}/{pid}", "kind": kind, "feature": label, "path": str(path.relative_to(out)), "src": src, "odf": odf, **({"exact": exact[0]} if exact else {})}
                try:
                    runner(path, commands)
                except subprocess.CalledProcessError as error:
                    entry["craft_error"] = str(error)
                    failures.append(entry["id"])
                manifest["probes"] = [p for p in manifest["probes"] if p["id"] != entry["id"]] + [entry]
                print(f"probe {entry['id']}", file=sys.stderr)

    if args.only in (None, "scale"):
        for kind, tiers in (("xlsx", args.xlsx_tiers), ("docx", args.docx_tiers)):
            (out / "scale" / kind).mkdir(parents=True, exist_ok=True)
            for size in tiers:
                if kind == "xlsx":
                    name, runner, commands = f"northstar-workbook-{size // 1000}k.xlsx", run_gridcraft, xlsx_tier(size)
                else:
                    name, runner, commands = f"northstar-playbook-{size}ch.docx", run_wordcraft, docx_tier(size)
                    if size > DOCX_BLOCK:
                        block = (out / "scale" / kind / f"northstar-playbook-{DOCX_BLOCK}ch.docx").resolve()
                        if not block.exists():
                            run_wordcraft(block, docx_tier(DOCX_BLOCK))
                        runner = compose_wordcraft(block, -(-size // DOCX_BLOCK))
                path = out / "scale" / kind / name
                entry = {"id": f"{kind}/{path.stem}", "kind": kind, "size": size, "path": str(path.relative_to(out))}
                try:
                    runner(path, commands)
                except subprocess.CalledProcessError as error:
                    entry["craft_error"] = str(error)
                    failures.append(entry["id"])
                manifest["scale"] = [s for s in manifest["scale"] if s["id"] != entry["id"]] + [entry]
                print(f"scale {entry['id']}", file=sys.stderr)

    manifest["probes"].sort(key=lambda p: p["id"])
    manifest["scale"].sort(key=lambda s: (s["kind"], s["size"]))
    manifest_path.write_text(json.dumps(manifest, indent=1, ensure_ascii=False))
    if failures:
        print("Craft refused: " + ", ".join(failures), file=sys.stderr)


if __name__ == "__main__":
    main()
