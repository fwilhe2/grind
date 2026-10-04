// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which font file a run is set in (`doc/pdf-export.md`, "Fonts").
//!
//! Most of what makes a PDF look wrong is *matching*, not PDF bytes, so this is the part with
//! rules. A family is resolved in this order, and the answer says which step found it, because
//! a substitution is part of the output ([`crate::Report`]) rather than something to hide:
//!
//! 1. the family itself, if any face here has that name;
//! 2. a **metric-compatible** substitute: Liberation for Times New Roman, Arial and Courier New,
//!    Carlito for Calibri, Caladea for Cambria. Each pair is the substitute font's own published
//!    claim, and a substitute with the same advance widths breaks lines in the same places;
//! 3. the generic families ODF and CSS share (`serif`, `sans-serif`, `monospace`);
//! 4. Liberation Serif, Writer's own default face.
//!
//! The bundled set is Liberation Sans, Serif and Mono in four styles each (OFL-1.1, see
//! `REUSE.toml`). It is always present, so every family resolves to *something* and a test or a
//! browser with no fonts installed gets the same answer as a desktop with none of the named ones.

use std::sync::Arc;

/// A font file's bytes: compiled in, or read at run time and shared between every face of a
/// collection. Both are what the PDF writer can take without copying.
#[derive(Clone)]
pub enum Data {
    Static(&'static [u8]),
    Shared(Arc<Vec<u8>>),
}

impl Data {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Data::Static(bytes) => bytes,
            Data::Shared(bytes) => bytes,
        }
    }
}

/// Four megabytes of glyphs are not a useful thing to print.
impl std::fmt::Debug for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Data({} bytes)", self.bytes().len())
    }
}

/// One face: a font file, or one font of a collection, and what it calls itself.
#[derive(Clone, Debug)]
pub struct Face {
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    pub data: Data,
    /// Which font of a collection; 0 for a plain `.ttf`/`.otf`.
    pub index: u32,
}

impl Face {
    pub fn bytes(&self) -> &[u8] {
        self.data.bytes()
    }
}

/// A face, by its position in a [`Fonts`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FaceId(pub usize);

/// How a family was found — the steps in the module's own documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Match {
    /// The family asked for.
    Exact,
    /// A face with the same metrics under another name.
    Compatible,
    /// A generic family (`serif`, …) or nothing at all, set in the face it means.
    Generic,
    /// A named family nothing here has, and nothing is metric-compatible with.
    Fallback,
}

/// The answer to "set this family, this weight, this slant".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub face: FaceId,
    pub how: Match,
}

/// Every face this export may use.
#[derive(Clone, Debug, Default)]
pub struct Fonts {
    faces: Vec<Face>,
}

/// The bundled faces, compiled in.
const BUNDLED: [&[u8]; 12] = [
    include_bytes!("../fonts/LiberationSerif-Regular.ttf"),
    include_bytes!("../fonts/LiberationSerif-Bold.ttf"),
    include_bytes!("../fonts/LiberationSerif-Italic.ttf"),
    include_bytes!("../fonts/LiberationSerif-BoldItalic.ttf"),
    include_bytes!("../fonts/LiberationSans-Regular.ttf"),
    include_bytes!("../fonts/LiberationSans-Bold.ttf"),
    include_bytes!("../fonts/LiberationSans-Italic.ttf"),
    include_bytes!("../fonts/LiberationSans-BoldItalic.ttf"),
    include_bytes!("../fonts/LiberationMono-Regular.ttf"),
    include_bytes!("../fonts/LiberationMono-Bold.ttf"),
    include_bytes!("../fonts/LiberationMono-Italic.ttf"),
    include_bytes!("../fonts/LiberationMono-BoldItalic.ttf"),
];

/// Metric-compatible pairs: the family a document names, and the one with its advance widths.
/// Each is the substitute's own published claim — Liberation's for the first three, Carlito's
/// and Caladea's for the last two, which are found only when installed.
const COMPATIBLE: [(&str, &str); 5] = [
    ("times new roman", "Liberation Serif"),
    ("arial", "Liberation Sans"),
    ("courier new", "Liberation Mono"),
    ("calibri", "Carlito"),
    ("cambria", "Caladea"),
];

/// The generic families, and Writer's default face for a run that names none.
const GENERIC: [(&str, &str); 3] = [
    ("serif", "Liberation Serif"),
    ("sans-serif", "Liberation Sans"),
    ("monospace", "Liberation Mono"),
];
const DEFAULT: &str = "Liberation Serif";

impl Fonts {
    /// Only the bundled faces — what a test, a reproducible build and the browser use.
    pub fn bundled() -> Self {
        let mut fonts = Fonts::default();
        for bytes in BUNDLED {
            fonts.add_data(Data::Static(bytes));
        }
        fonts
    }

    /// Add every face in a font file or collection. Answers how many were added: nothing that
    /// does not parse as a font is an error, it is simply not a face.
    pub fn add(&mut self, data: Vec<u8>) -> usize {
        self.add_data(Data::Shared(Arc::new(data)))
    }

    fn add_data(&mut self, data: Data) -> usize {
        use skrifa::MetadataProvider;
        use skrifa::raw::FileRef;
        use skrifa::string::StringId;
        let Ok(file) = FileRef::new(data.bytes()) else {
            return 0;
        };
        let mut added = Vec::new();
        for (index, font) in file.fonts().enumerate() {
            let Ok(font) = font else { continue };
            // The typographic family groups a family's weights under one name, where the legacy
            // one is split into "Foo Light" and so on; prefer it when a font has one.
            let family = [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME]
                .into_iter()
                .find_map(|id| font.localized_strings(id).english_or_first())
                .map(|name| name.to_string());
            let Some(family) = family else { continue };
            let attributes = font.attributes();
            added.push(Face {
                family,
                bold: attributes.weight.value() >= 600.0,
                italic: attributes.style != skrifa::attribute::Style::Normal,
                data: data.clone(),
                index: index as u32,
            });
        }
        let count = added.len();
        self.faces.extend(added);
        count
    }

    /// The face of `family` nearest this weight and slant, if any face has that family. A
    /// slant outranks a weight: an upright face set where italic was asked loses the emphasis,
    /// a regular one set for bold only some of it.
    fn nearest(&self, family: &str, bold: bool, italic: bool) -> Option<FaceId> {
        self.faces
            .iter()
            .enumerate()
            .filter(|(_, face)| face.family.eq_ignore_ascii_case(family))
            .min_by_key(|(_, face)| {
                2 * u8::from(face.italic != italic) + u8::from(face.bold != bold)
            })
            .map(|(at, _)| FaceId(at))
    }

    /// The face at `id`.
    pub fn face(&self, id: FaceId) -> &Face {
        &self.faces[id.0]
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// The face to set `family` in, at this weight and slant.
    pub fn resolve(&self, family: Option<&str>, bold: bool, italic: bool) -> Resolved {
        let asked = family.map(str::trim).filter(|f| !f.is_empty());
        let found = |name: &str, how: Match| {
            self.nearest(name, bold, italic)
                .map(|face| Resolved { face, how })
        };
        let lower = asked.map(str::to_ascii_lowercase);
        let lower = lower.as_deref();
        asked
            .and_then(|name| found(name, Match::Exact))
            .or_else(|| {
                let twin = COMPATIBLE.iter().find(|(name, _)| Some(*name) == lower)?.1;
                found(twin, Match::Compatible)
            })
            .or_else(|| {
                let generic = match lower {
                    None => DEFAULT,
                    Some(name) => GENERIC.iter().find(|(g, _)| *g == name)?.1,
                };
                found(generic, Match::Generic)
            })
            .or_else(|| found(DEFAULT, Match::Fallback))
            .unwrap_or(Resolved {
                // Only a database built by hand with no Liberation in it gets here; its first
                // face is as good an answer as any, and an empty one has none to give.
                face: FaceId(0),
                how: Match::Fallback,
            })
    }
}

/// The families worth looking for on the machine to set `named` in: each name itself and, when
/// there is one, its metric-compatible twin — so a document in Calibri finds an installed
/// Carlito even where Calibri is not installed.
pub fn wanted<'a>(named: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        if !out.iter().any(|seen| seen.eq_ignore_ascii_case(name)) {
            out.push(name.to_owned());
        }
    };
    for name in named {
        push(name);
        let lower = name.to_ascii_lowercase();
        if let Some((_, twin)) = COMPATIBLE.iter().find(|(asked, _)| *asked == lower) {
            push(twin);
        }
    }
    out
}

#[cfg(feature = "system-fonts")]
impl Fonts {
    /// The fonts installed on this machine, indexed but not read: only their names and where
    /// they are. Reading a face is [`Fonts::add_families`]'s, and only for the families asked for.
    pub fn system_database() -> fontdb::Database {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    }

    /// Read every face of these families that `db` knows of and add it, unless a face of that
    /// family is already here (the bundled Liberation is not replaced by an installed copy).
    /// Answers how many faces were added.
    pub fn add_families(&mut self, db: &fontdb::Database, families: &[String]) -> usize {
        let missing: Vec<&String> = families
            .iter()
            .filter(|family| {
                !self
                    .faces
                    .iter()
                    .any(|face| face.family.eq_ignore_ascii_case(family))
            })
            .collect();
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        for info in db.faces() {
            let named = info.families.iter().any(|(name, _)| {
                missing
                    .iter()
                    .any(|family| family.eq_ignore_ascii_case(name))
            });
            if let (true, fontdb::Source::File(path)) = (named, &info.source)
                && !files.contains(path)
            {
                files.push(path.clone());
            }
        }
        let before = self.faces.len();
        for path in files {
            if let Ok(bytes) = std::fs::read(&path) {
                self.add_data(Data::Shared(Arc::new(bytes)));
            }
        }
        // A collection holds other families as well; keep only what was asked for.
        let mut at = before;
        while at < self.faces.len() {
            if missing
                .iter()
                .any(|family| family.eq_ignore_ascii_case(&self.faces[at].family))
            {
                at += 1;
            } else {
                self.faces.remove(at);
            }
        }
        self.faces.len() - before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(fonts: &Fonts, r: &Resolved) -> (String, bool, bool) {
        let face = fonts.face(r.face);
        (face.family.clone(), face.bold, face.italic)
    }

    #[test]
    fn the_bundled_set_is_three_families_in_four_styles() {
        let fonts = Fonts::bundled();
        assert_eq!(fonts.len(), 12);
        for family in ["Liberation Serif", "Liberation Sans", "Liberation Mono"] {
            for bold in [false, true] {
                for italic in [false, true] {
                    let r = fonts.resolve(Some(family), bold, italic);
                    assert_eq!(r.how, Match::Exact, "{family} {bold} {italic}");
                    assert_eq!(named(&fonts, &r), (family.to_owned(), bold, italic));
                }
            }
        }
    }

    #[test]
    fn a_family_is_matched_whatever_its_case() {
        let fonts = Fonts::bundled();
        let r = fonts.resolve(Some("liberation sans"), false, false);
        assert_eq!(r.how, Match::Exact);
        assert_eq!(named(&fonts, &r).0, "Liberation Sans");
    }

    #[test]
    fn the_office_fonts_are_set_in_their_metric_compatible_twins() {
        let fonts = Fonts::bundled();
        for (asked, twin) in [
            ("Times New Roman", "Liberation Serif"),
            ("Arial", "Liberation Sans"),
            ("Courier New", "Liberation Mono"),
        ] {
            let r = fonts.resolve(Some(asked), true, false);
            assert_eq!(r.how, Match::Compatible, "{asked}");
            assert_eq!(named(&fonts, &r), (twin.to_owned(), true, false), "{asked}");
        }
    }

    #[test]
    fn the_generic_families_mean_the_bundled_faces() {
        let fonts = Fonts::bundled();
        for (asked, face) in [
            (Some("serif"), "Liberation Serif"),
            (Some("sans-serif"), "Liberation Sans"),
            (Some("monospace"), "Liberation Mono"),
            (None, "Liberation Serif"),
        ] {
            let r = fonts.resolve(asked, false, false);
            assert_eq!(r.how, Match::Generic, "{asked:?}");
            assert_eq!(named(&fonts, &r).0, face, "{asked:?}");
        }
    }

    #[test]
    fn a_family_nobody_has_falls_back_to_writers_default_and_says_so() {
        let fonts = Fonts::bundled();
        let r = fonts.resolve(Some("Comic Neue Angular"), false, true);
        assert_eq!(r.how, Match::Fallback);
        assert_eq!(
            named(&fonts, &r),
            ("Liberation Serif".to_owned(), false, true)
        );
    }

    #[test]
    fn a_missing_style_takes_the_nearest_face_of_the_family() {
        let mut fonts = Fonts::default();
        let regular = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fonts/LiberationSans-Regular.ttf"
        ))
        .unwrap();
        assert_eq!(fonts.add(regular), 1);
        let r = fonts.resolve(Some("Liberation Sans"), true, true);
        assert_eq!(r.how, Match::Exact);
        assert_eq!(
            named(&fonts, &r),
            ("Liberation Sans".to_owned(), false, false)
        );
    }

    #[test]
    fn a_family_is_looked_for_with_its_twin() {
        assert_eq!(
            wanted(["Calibri", "Georgia", "arial", "Calibri"]),
            vec!["Calibri", "Carlito", "Georgia", "arial", "Liberation Sans"]
        );
    }

    #[cfg(feature = "system-fonts")]
    #[test]
    fn only_the_families_asked_for_are_read_from_the_machine() {
        let mut db = fontdb::Database::new();
        db.load_fonts_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts"));
        assert_eq!(
            db.faces().count(),
            12,
            "a directory standing in for the machine"
        );
        let mut fonts = Fonts::default();
        assert_eq!(fonts.add_families(&db, &["liberation mono".to_owned()]), 4);
        let r = fonts.resolve(Some("Liberation Mono"), true, true);
        assert_eq!(
            (r.how.clone(), named(&fonts, &r)),
            (Match::Exact, ("Liberation Mono".to_owned(), true, true))
        );
        assert!(
            fonts
                .faces
                .iter()
                .all(|face| face.family == "Liberation Mono")
        );
        assert_eq!(
            fonts.add_families(&db, &["Liberation Mono".to_owned()]),
            0,
            "already here"
        );
        assert_eq!(fonts.add_families(&db, &["Nowhere Sans".to_owned()]), 0);
    }

    #[test]
    fn bytes_that_are_not_a_font_add_nothing() {
        let mut fonts = Fonts::default();
        assert_eq!(fonts.add(b"not a font".to_vec()), 0);
        assert!(fonts.is_empty());
    }
}
