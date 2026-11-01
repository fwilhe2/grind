// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: MIT
//
// A workbook from grype vulnerability scans (`grype -o json`), one scan per image.
//
//   grind build examples/cve/cve.rhai -o scan.ods             # the made-up sample beside it
//   examples/cve/cve.sh ~/scans/cve scan.ods                  # real scans, with filters
//
// `scans.json` names the scans to read, because a script cannot list a directory:
//
//   { "title": "Nightly images", "scans": [ { "name": "base", "file": "base.json" }, … ] }
//
// `cve.sh` writes that manifest for a directory of scans, builds, and then adds what a script
// cannot say: filter buttons and banding on every table, severity colours as conditional rules
// so the banding does not paint over them, and the helper columns hidden.
//
// | Sheet           | One row per                                                       |
// |-----------------|-------------------------------------------------------------------|
// | Summary         | image: findings by severity, and the same for what a filter shows |
// | Findings        | match: image × vulnerability × package, worst first               |
// | Vulnerabilities | vulnerability, counted once however many images have it           |
// | Packages        | package in an image, with its findings by severity                |
//
// Every count is a formula over Findings or Vulnerabilities, so the Summary agrees with the
// rows. Each table has a hidden `Shown` column (`SUBTOTAL(103;…)`, 1 while a filter leaves the
// row visible), which is what the Summary's "shown" rows count.

let source = json("scans.json");

const SEVERITIES = ["Critical", "High", "Medium", "Low", "Negligible", "Unknown"];

// --- helpers --------------------------------------------------------------------------------

/// Scanned text as a cell: a leading `=` or `'` would otherwise make it a formula or eat it.
fn txt(s) {
    if type_of(s) != "string" { return if s == () { "" } else { `${s}` }; }
    if s.starts_with("=") || s.starts_with("'") { "'" + s } else { s }
}

/// A JSON number as a float, or `()` when there is none.
fn num(x) {
    switch type_of(x) { "i64" => x.to_float(), "f64" => x, _ => () }
}

fn join(items, sep) {
    let out = "";
    for (x, i) in items { if i > 0 { out += sep; } out += x; }
    out
}

fn add_unique(x) {
    if x != "" && !(x in this) { this.push(x); }
}

fn rank(severity) {
    let i = global::SEVERITIES.index_of(severity);
    if i < 0 { 5 } else { i }
}

/// Grype's spelling, capitalised the way the Summary's headers are.
fn severity(s) {
    let s = if type_of(s) == "string" { s.to_lower() } else { "" };
    switch s {
        "critical" => "Critical", "high" => "High", "medium" => "Medium",
        "low" => "Low", "negligible" => "Negligible", _ => "Unknown",
    }
}

fn fix_state(state) {
    switch state { "fixed" => "yes", "not-fixed" => "no", "wont-fix" => "no", _ => "unknown" }
}

fn package_type(t) {
    switch t { "UnknownPackage" => "generic", () => "", _ => t }
}

/// The score to sort by: the newest CVSS version's highest base score.
fn best_cvss(list) {
    let best = #{ score: (), version: "" };
    let best_v = -1.0;
    for c in list {
        let score = num(c.metrics?.baseScore);
        if score == () { continue; }
        let v = parse_float(c.version ?? "0");
        if v > best_v || (v == best_v && score > best.score) {
            best_v = v;
            best = #{ score: score, version: c.version };
        }
    }
    best
}

/// Field `f` of the vulnerability, or of the first related one that has it.
fn first_list(m, f) {
    let own = m.vulnerability[f] ?? [];
    if own.len() > 0 { return own; }
    for r in m.relatedVulnerabilities ?? [] {
        let l = r[f] ?? [];
        if l.len() > 0 { return l; }
    }
    []
}

/// `2026-10-31T21:52:59.44Z` as `2026-10-31 21:52 UTC`; anything else as it is.
fn when(t) {
    if type_of(t) != "string" || t.len() < 16 || !t.ends_with("Z") { return txt(t); }
    t.sub_string(0, 10) + " " + t.sub_string(11, 5) + " UTC"
}

fn header(s, cells) {
    s.push(row(cells).bold());
}

/// A column's letter, for building ranges like `Findings.$B$2:$B$40`.
fn col(s, c) {
    let a = s.at(0, c);
    a.sub_string(0, a.len() - 1)
}

/// `Sheet.$X$2:$X$last`, the data part of column `c`.
fn data_range(s, name, c, last) {
    let x = col(s, c);
    name + ".$" + x + "$2:$" + x + "$" + last
}

let colours = #{
    Critical: ["#b71c1c", "white"], High: ["#e65100", "white"], Medium: ["#f9a825", "black"],
    Low: ["#90caf9", "black"], Negligible: ["#e0e0e0", "black"], Unknown: ["#bdbdbd", "black"],
};
let heading = style().bold().background("#37474f").color("white").valign("middle");
// The tables' headers: `sheet format-table` puts its own background behind them.
let table_heading = style().bold().background("#cfd8dc").valign("middle");

// --- reading ------------------------------------------------------------------------------

let images = [];
let findings = [];
let tool = "";

for scan in source.scans {
    let data = json(scan.file);
    images.push(scan.name);
    if tool == "" && data.descriptor != () {
        tool = `${data.descriptor.name} ${data.descriptor.version}, scanned ${when(data.descriptor.timestamp)}`;
        let built = data.descriptor.db?.status?.built;
        if built != () { tool += `, database built ${when(built)}`; }
    }
    for m in data.matches {
        let v = m.vulnerability;
        let cve = if v.id.starts_with("CVE-") { v.id } else { "" };
        if cve == "" {
            for r in m.relatedVulnerabilities ?? [] {
                if r.id.starts_with("CVE-") { cve = r.id; break; }
            }
        }
        let cvss = best_cvss(first_list(m, "cvss"));
        let epss = first_list(m, "epss");
        let cwes = [];
        for c in first_list(m, "cwes") { cwes.add_unique(c.cwe ?? ""); }
        let fixed_in = [];
        for x in v.fix?.versions ?? [] { fixed_in.add_unique(txt(x)); }

        let sev = severity(v.severity);
        findings.push(#{
            image: scan.name,
            severity: sev,
            id: txt(v.id),
            key: if cve == "" { txt(v.id) } else { cve },
            cve: cve,
            pkg: txt(m.artifact.name),
            version: txt(m.artifact.version),
            kind: txt(package_type(m.artifact.type)),
            fix: fix_state(v.fix?.state ?? ""),
            fixed_in: join(fixed_in, ", "),
            cvss: cvss.score,
            cvss_version: cvss.version,
            epss: if epss.len() > 0 { num(epss[0].epss) } else { () },
            percentile: if epss.len() > 0 { num(epss[0].percentile) } else { () },
            risk: num(v.risk),
            cwe: join(cwes, ", "),
            link: txt(v.dataSource ?? ""),
            description: txt(v.description ?? ""),
            rank: rank(sev),
        });
    }
}

/// Worst first: severity, then grype's risk, then the CVSS score, then the name.
fn worse(a, b) {
    if a.rank != b.rank { return a.rank - b.rank; }
    let ar = a.risk ?? 0.0;
    let br = b.risk ?? 0.0;
    if ar != br { return if ar > br { -1 } else { 1 }; }
    let ac = a.cvss ?? 0.0;
    let bc = b.cvss ?? 0.0;
    if ac != bc { return if ac > bc { -1 } else { 1 }; }
    if a.key < b.key { -1 } else if a.key > b.key { 1 } else { 0 }
}
findings.sort(worse);

// --- Findings -----------------------------------------------------------------------------

let f = sheet("Findings");
header(f, ["Image", "Severity", "Vulnerability", "CVE", "Package", "Version", "Type",
           "Fix available", "Fixed in", "CVSS", "CVSS version", "EPSS", "EPSS percentile",
           "Risk", "CWE", "Shown", "Source", "Description"]);
for x in findings {
    let at = f.push([x.image, x.severity, x.id, x.cve, x.pkg, x.version, x.kind, x.fix,
                     x.fixed_in, x.cvss, x.cvss_version, x.epss, x.percentile, x.risk, x.cwe,
                     (), x.link, x.description]);
    f.set(f.at(at, 15), formula("SUBTOTAL(103;" + f.at(at, 0) + ")"));
    let c = colours[x.severity];
    f.style(f.at(at, 1), style().background(c[0]).color(c[1]).bold());
}
let f_last = f.rows;
let f_image = data_range(f, "Findings", 0, f_last);
let f_sev = data_range(f, "Findings", 1, f_last);
let f_pkg = data_range(f, "Findings", 4, f_last);
let f_ver = data_range(f, "Findings", 5, f_last);
let f_fix = data_range(f, "Findings", 7, f_last);
let f_shown = data_range(f, "Findings", 15, f_last);

// --- Vulnerabilities ----------------------------------------------------------------------

// One entry per CVE (or advisory with none), merged across every image and package.
let by_key = #{};
let keys = [];
for x in findings {
    if !(x.key in by_key) {
        keys.push(x.key);
        by_key[x.key] = #{
            key: x.key, aliases: [], severity: x.severity, rank: x.rank, cvss: x.cvss,
            epss: x.epss, percentile: x.percentile, risk: x.risk, fix: x.fix, fixed_in: [],
            packages: [], images: [], link: x.link, description: x.description,
        };
    }
    let u = by_key[x.key];
    if x.id != x.key { u.aliases.add_unique(x.id); }
    if x.rank < u.rank { u.rank = x.rank; u.severity = x.severity; }
    if (x.cvss ?? -1.0) > (u.cvss ?? -1.0) { u.cvss = x.cvss; }
    if (x.epss ?? -1.0) > (u.epss ?? -1.0) { u.epss = x.epss; u.percentile = x.percentile; }
    if (x.risk ?? -1.0) > (u.risk ?? -1.0) { u.risk = x.risk; }
    if x.fix == "yes" || (x.fix == "no" && u.fix == "unknown") { u.fix = x.fix; }
    for fv in x.fixed_in.split(", ") { u.fixed_in.add_unique(fv); }
    u.packages.add_unique(x.pkg + " " + x.version);
    u.images.add_unique(x.image);
    if u.description == "" { u.description = x.description; }
    by_key[x.key] = u;
}
let vulns = [];
for k in keys { vulns.push(by_key[k]); }
vulns.sort(worse);

let v = sheet("Vulnerabilities");
let v_head = ["Vulnerability", "Also known as", "Severity", "CVSS", "EPSS", "EPSS percentile",
              "Fix available", "Fixed in", "Packages", "Images"];
let v_first_image = v_head.len();
for i in images { v_head.push(i); }
let v_shown_col = v_head.len();
v_head += ["Shown", "Source", "Description"];
header(v, v_head);
for u in vulns {
    let cells = [u.key, join(u.aliases, ", "), u.severity, u.cvss, u.epss, u.percentile, u.fix,
                 join(u.fixed_in, ", "), join(u.packages, ", "), ()];
    for i in images { cells.push(if i in u.images { "yes" } else { () }); }
    cells += [(), u.link, u.description];
    let at = v.push(cells);
    v.set(v.at(at, 9), formula("COUNTIF(" + v.at(at, v_first_image) + ":"
        + v.at(at, v_first_image + images.len() - 1) + ";\"yes\")"));
    v.set(v.at(at, v_shown_col), formula("SUBTOTAL(103;" + v.at(at, 0) + ")"));
    let c = colours[u.severity];
    v.style(v.at(at, 2), style().background(c[0]).color(c[1]).bold());
}
let v_last = v.rows;
let v_sev = data_range(v, "Vulnerabilities", 2, v_last);
let v_fix = data_range(v, "Vulnerabilities", 6, v_last);
let v_shown = data_range(v, "Vulnerabilities", v_shown_col, v_last);

// --- Packages -----------------------------------------------------------------------------

let pkgs = #{};
let pkg_keys = [];
for x in findings {
    let k = x.image + "\t" + x.pkg + "\t" + x.version;
    if !(k in pkgs) {
        pkg_keys.push(k);
        pkgs[k] = #{ image: x.image, pkg: x.pkg, version: x.version, kind: x.kind,
                     weight: 0, fixed_in: [] };
    }
    let p = pkgs[k];
    // A critical outweighs any number of highs, and so on down, for the order only.
    p.weight += 1000 ** (5 - x.rank);
    for fv in x.fixed_in.split(", ") { p.fixed_in.add_unique(fv); }
    pkgs[k] = p;
}
let packages = [];
for k in pkg_keys { packages.push(pkgs[k]); }
packages.sort(|a, b| if a.weight != b.weight { b.weight - a.weight }
                     else if a.pkg < b.pkg { -1 } else if a.pkg > b.pkg { 1 }
                     else if a.image < b.image { -1 } else { 1 });

let p = sheet("Packages");
let p_head = ["Image", "Package", "Version", "Type"];
p_head += SEVERITIES;
p_head += ["Total", "Fix available", "Fixed in", "Shown"];
header(p, p_head);
for x in packages {
    let at = p.push([x.image, x.pkg, x.version, x.kind]);
    let r = at + 1;
    let hit = "(" + f_image + "=A" + r + ")*(" + f_pkg + "=B" + r + ")*(" + f_ver + "=C" + r + ")";
    for (s, i) in SEVERITIES {
        p.set(p.at(at, 4 + i), formula("SUMPRODUCT(" + hit + "*(" + f_sev + "=\"" + s + "\"))"));
    }
    p.set(p.at(at, 10), formula("SUM(" + p.at(at, 4) + ":" + p.at(at, 9) + ")"));
    p.set(p.at(at, 11), formula("SUMPRODUCT(" + hit + "*(" + f_fix + "=\"yes\"))"));
    p.set(p.at(at, 12), join(x.fixed_in, ", "));
    p.set(p.at(at, 13), formula("SUBTOTAL(103;" + p.at(at, 0) + ")"));
}
let p_last = p.rows;

// --- Summary ------------------------------------------------------------------------------

let s = sheet("Summary");
s.push([txt(source.title ?? "Vulnerability scan")]);
s.push([txt(tool)]);
s.push([]);
let cols = ["Image"];
cols += SEVERITIES;
cols += ["Total", "Fix available", "Packages"];
let head_at = s.push(row(cols).bold());
let first = head_at + 2;
for img in images {
    let at = s.push([img]);
    let r = at + 1;
    for (sev, i) in SEVERITIES {
        s.set(s.at(at, 1 + i), formula("SUMPRODUCT((" + f_image + "=A" + r + ")*(" + f_sev + "=\"" + sev + "\"))"));
    }
    s.set(s.at(at, 7), formula("SUM(B" + r + ":G" + r + ")"));
    s.set(s.at(at, 8), formula("SUMPRODUCT((" + f_image + "=A" + r + ")*(" + f_fix + "=\"yes\"))"));
    s.set(s.at(at, 9), formula("COUNTIF(Packages.$A$2:$A$" + p_last + ";A" + r + ")"));
}
let last_image = s.rows;
let total = s.push(row(["All findings", sum_above(), sum_above(), sum_above(), sum_above(),
                        sum_above(), sum_above(), sum_above(), sum_above(), sum_above()]).bold());

/// A row of counts by severity over one table: `extra` is `*(…)` to narrow it, or "".
fn count_row(s, label, sev, fix, extra, packages) {
    let at = s.push([label]);
    let r = at + 1;
    for (name, i) in global::SEVERITIES {
        s.set(s.at(at, 1 + i), formula("SUMPRODUCT((" + sev + "=\"" + name + "\")" + extra + ")"));
    }
    s.set(s.at(at, 7), formula("SUM(B" + r + ":G" + r + ")"));
    s.set(s.at(at, 8), formula("SUMPRODUCT((" + fix + "=\"yes\")" + extra + ")"));
    if packages != "" { s.set(s.at(at, 9), formula(packages)); }
    at
}
let distinct = count_row(s, "Distinct vulnerabilities", v_sev, v_fix, "", "");
s.push([]);
let shown_head = s.push(row(["Shown by the current filter"]).italic());
count_row(s, "Findings", f_sev, f_fix, "*(" + f_shown + "=1)", "");
count_row(s, "Vulnerabilities", v_sev, v_fix, "*(" + v_shown + "=1)", "");
let s_last = s.rows;

s.style("A1", style().bold().size("16pt"));
s.style("A2", style().italic().color("#616161"));
s.style("A" + (head_at + 1) + ":J" + (head_at + 1), heading);
for (sev, i) in SEVERITIES {
    let c = colours[sev];
    s.style(s.at(head_at, 1 + i), style().background(c[0]).color(c[1]).align("center"));
}
s.style("B" + (head_at + 1) + ":J" + (head_at + 1), style().align("center"));
s.style("A" + (total + 1) + ":J" + (distinct + 1), style().background("#eceff1"));
s.style("A" + (total + 1) + ":J" + (total + 1), style().border("0.5pt solid #37474f"));
s.style("A" + (shown_head + 2) + ":J" + s_last, style().background("#f5f5f5"));
s.width("A:A", "5.5cm");
s.width("B:J", "2.4cm");
s.height("1:" + s_last, "6mm");
s.height("1:1", "9mm");

s.chart(chart("bar").title("Findings by severity")
    .categories("A" + first + ":A" + last_image)
    .series("B" + first + ":B" + last_image, "B" + (head_at + 1))
    .series("C" + first + ":C" + last_image, "C" + (head_at + 1))
    .series("D" + first + ":D" + last_image, "D" + (head_at + 1))
    .series("E" + first + ":E" + last_image, "E" + (head_at + 1))
    .colors([colours.Critical[0], colours.High[0], colours.Medium[0], colours.Low[0]])
    .y_label("Findings").gridlines()
    .position("0cm", `${(s_last + 1) * 6 + 3}mm`).size("18cm", "9cm"));

// --- the other sheets' look ---------------------------------------------------------------

f.style("A1:R1", table_heading);
f.format("J2:J" + f_last, format("number").decimals(1));
f.format("L2:L" + f_last, format("percent").decimals(2));
f.format("M2:M" + f_last, format("percent").decimals(0));
f.format("N2:N" + f_last, format("number").decimals(2));
f.width("A:A", "3cm");
f.width("B:B", "2.2cm");
f.width("C:D", "4.2cm");
f.width("E:E", "5cm");
f.width("F:F", "2.8cm");
f.width("G:G", "2.4cm");
f.width("H:H", "2.8cm");
f.width("I:I", "3cm");
f.width("J:L", "2.2cm");
f.width("M:M", "3.2cm");
f.width("N:N", "2.2cm");
f.width("O:O", "3cm");
f.width("P:P", "1.6cm");
f.width("Q:Q", "6cm");
f.width("R:R", "20cm");

let v_end = col(v, v_head.len() - 1);
v.style("A1:" + v_end + "1", table_heading);
v.format("D2:D" + v_last, format("number").decimals(1));
v.format("E2:E" + v_last, format("percent").decimals(2));
v.format("F2:F" + v_last, format("percent").decimals(0));
v.width("A:B", "4.2cm");
v.width("C:E", "2.4cm");
v.width("F:F", "3.2cm");
v.width("G:G", "2.8cm");
v.width("H:H", "3cm");
v.width("I:I", "7cm");
v.width("J:J", "1.8cm");
v.width(col(v, v_first_image) + ":" + col(v, v_shown_col), "2.4cm");
v.width(col(v, v_shown_col + 1), "6cm");
v.width(col(v, v_shown_col + 2), "20cm");
if images.len() > 0 {
    v.style(col(v, v_first_image) + "2:" + col(v, v_first_image + images.len() - 1) + v_last,
            style().align("center"));
}

p.style("A1:N1", table_heading);
p.width("A:A", "3cm");
p.width("B:B", "6cm");
p.width("C:C", "2.8cm");
p.width("D:D", "2.4cm");
p.width("E:K", "2.4cm");
p.width("L:L", "2.8cm");
p.width("M:M", "4cm");
p.width("N:N", "1.6cm");
for (sev, i) in SEVERITIES {
    let c = colours[sev];
    p.style(p.at(0, 4 + i), style().background(c[0]).color(c[1]));
}

let book = spreadsheet();
book.push(s);
book.push(f);
book.push(v);
book.push(p);
book;

// --- tests, against the sample beside this script ----------------------------------------

fn test_the_summary_counts_the_sample(d) {
    // base: one critical, one medium, one low; app: the same critical, a high and an unknown.
    assert_eq(d.cell("Summary.A5").value, "base");
    assert_eq(d.cell("Summary.B5").value, 1);
    assert_eq(d.cell("Summary.D5").value, 1);
    assert_eq(d.cell("Summary.E5").value, 1);
    assert_eq(d.cell("Summary.H5").value, 3);
    assert_eq(d.cell("Summary.I5").value, 1);
    assert_eq(d.cell("Summary.J5").value, 2);
    assert_eq(d.cell("Summary.H6").value, 3);
    assert_eq(d.cell("Summary.G6").value, 1);
}

fn test_a_vulnerability_in_two_images_counts_once(d) {
    assert_eq(d.cell("Summary.A7").value, "All findings");
    assert_eq(d.cell("Summary.B7").value, 2);
    assert_eq(d.cell("Summary.H7").value, 6);
    assert_eq(d.cell("Summary.A8").value, "Distinct vulnerabilities");
    assert_eq(d.cell("Summary.B8").value, 1);
    assert_eq(d.cell("Summary.H8").value, 5);
    assert_eq(d.cell("Vulnerabilities.A2").value, "CVE-2099-1001");
    assert_eq(d.cell("Vulnerabilities.J2").value, 2);
}

fn test_with_no_filter_everything_is_shown(d) {
    assert_eq(d.cell("Summary.H11").value, d.cell("Summary.H7").value);
    assert_eq(d.cell("Summary.H12").value, d.cell("Summary.H8").value);
}

fn test_the_worst_comes_first(d) {
    assert_eq(d.cell("Findings.B2").value, "Critical");
    assert_eq(d.cell("Findings.B7").value, "Unknown");
    assert_eq(d.cell("Packages.B2").value, "libsquish");
}

fn test_an_advisory_is_filed_under_its_cve(d) {
    assert_eq(d.cell("Vulnerabilities.A3").value, "CVE-2099-2001");
    assert_eq(d.cell("Vulnerabilities.B3").value, "GHSA-xxxx-yyyy-zzzz");
    assert_eq(d.cell("Vulnerabilities.D3").value, 8.7);
}

fn test_scanned_text_never_becomes_a_formula(d) {
    // base.json's CVE-2099-1002 has a description that begins with `=`.
    for r in 2..8 {
        let c = d.cell("Findings.C" + r);
        if c.value == "CVE-2099-1002" {
            let desc = d.cell("Findings.R" + r);
            assert_eq(desc.formula, ());
            assert(desc.value.starts_with("="), desc.value);
            return;
        }
    }
    throw "CVE-2099-1002 is not in Findings";
}
