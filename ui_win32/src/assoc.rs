// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! File associations: **offered, never taken** (`doc/windows-shell.md`, "File associations").
//!
//! Windows lets an application say it *can* open a type without saying it is *the* program for
//! it, and this file says exactly the first thing. Per extension, a ProgID of ours is added
//! under `.ext\OpenWithProgids` — which is what puts Grind in Explorer's *Open with* menu and in
//! the "How do you want to open this file?" chooser — and the whole set is published as
//! `Capabilities` under `RegisteredApplications`, which is what lists Grind in Settings > Apps >
//! Default apps with every type it opens. **The extension's own default value is never
//! written**, and neither is anything under `UserChoice`: whatever opens a double-clicked
//! `.xlsx` today — Excel, LibreOffice, nothing — still does tomorrow, and switching to Grind is
//! the user's one click in a dialog Windows owns. Where nothing is registered for a type at all
//! (a `.fods` or a `.grind`, usually), Windows asks on the first double-click and offers Grind.
//!
//! Everything is per-user (`HKEY_CURRENT_USER\Software\…`): no elevation, no installer, and
//! nothing another account on the machine sees. The window writes it every time it starts from a
//! different place ([`needed`]), because this `.exe` is a file that can be copied anywhere —
//! off the test disc, out of a download — and an association naming where it *used* to be is a
//! double-click that fails. `--unregister` removes every key and value this file ever writes and
//! nothing else, which is why the list of them is data here rather than a sequence of calls.
//!
//! The portable half — which types, which ProgIDs, every value and where it goes — is tested on
//! Linux like the rest of this crate. The half that touches the registry is the `cfg(windows)`
//! block at the bottom, and it only walks the lists.

/// One type this shell opens, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileType {
    pub extension: &'static str,
    pub prog_id: &'static str,
    /// What the type is called, as Explorer would show it if Grind were its default.
    pub description: &'static str,
}

/// Every type the window opens: the three forms of both ODF document types, and the two Excel
/// workbooks the import filter reads (`doc/xlsx-import.md`, X6) — `.xlsm` included, since it is
/// the same XML with a macro that is counted and never run. `.xls` and `.xlsb` are not here
/// because the filter does not read them. The Excel rows are compiled out with the `xlsx`
/// feature, like the import itself: offering a type the binary then refuses would be worse
/// than not offering it.
pub const TYPES: &[FileType] = &[
    FileType {
        extension: ".fods",
        prog_id: "Grind.Spreadsheet",
        description: "OpenDocument Spreadsheet (flat)",
    },
    FileType {
        extension: ".ods",
        prog_id: "Grind.SpreadsheetPackage",
        description: "OpenDocument Spreadsheet",
    },
    FileType {
        extension: ".fodt",
        prog_id: "Grind.Document",
        description: "OpenDocument Text (flat)",
    },
    FileType {
        extension: ".odt",
        prog_id: "Grind.DocumentPackage",
        description: "OpenDocument Text",
    },
    FileType {
        extension: ".grind",
        prog_id: "Grind.Projection",
        description: "Grind Projection",
    },
    #[cfg(feature = "xlsx")]
    FileType {
        extension: ".xlsx",
        prog_id: "Grind.Workbook",
        description: "Excel Workbook (opened as a new ODF document)",
    },
    #[cfg(feature = "xlsx")]
    FileType {
        extension: ".xlsm",
        prog_id: "Grind.MacroWorkbook",
        description: "Excel Macro-Enabled Workbook (opened as a new ODF document)",
    },
];

/// The name the application goes by in *Open with* and in Default apps.
pub const APPLICATION: &str = "Grind";

/// What Default apps says under the name.
pub const DESCRIPTION: &str = "An ODF-native office suite. Opens OpenDocument spreadsheets and \
text documents, and Excel workbooks by importing them into a new ODF document — never writing \
one back.";

/// Where the capabilities live, relative to `HKEY_CURRENT_USER`.
const CAPABILITIES: &str = r"Software\Grind\Capabilities";

/// What a registry value holds. `None` is `REG_NONE` with no data, which is how an
/// `OpenWithProgids` entry is spelled: the value's *name* is the whole message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Data {
    Text(String),
    None,
}

/// One value to write, under `HKEY_CURRENT_USER`. `name` of `None` is the key's default value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value {
    pub key: String,
    pub name: Option<String>,
    pub data: Data,
}

/// One thing `--unregister` removes: a whole key of ours, or a single value of ours inside a
/// key that is somebody else's (`.xlsx\OpenWithProgids` belongs to everyone who opens one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Removal {
    Tree(String),
    Value { key: String, name: String },
}

fn value(key: impl Into<String>, name: Option<&str>, data: Data) -> Value {
    Value {
        key: key.into(),
        name: name.map(str::to_owned),
        data,
    }
}

fn text(s: impl Into<String>) -> Data {
    Data::Text(s.into())
}

/// The command a double-click runs: the executable, and the file quoted, since both paths may
/// hold spaces (`C:\Program Files\…`, `Quarterly Sales Report.fods`).
pub fn command(exe: &str) -> String {
    format!("\"{exe}\" \"%1\"")
}

/// The file name `Applications\…` is keyed by — whatever this copy of the executable is called.
fn file_name(exe: &str) -> &str {
    exe.rsplit(['\\', '/']).next().unwrap_or(exe)
}

/// Every value that offers this executable, at `exe`, for every type in [`TYPES`].
pub fn register(exe: &str) -> Vec<Value> {
    let open = command(exe);
    let icon = format!("\"{exe}\",0");
    let application = format!(r"Software\Classes\Applications\{}", file_name(exe));
    let mut values = Vec::new();

    for ty in TYPES {
        let prog = format!(r"Software\Classes\{}", ty.prog_id);
        values.push(value(&prog, None, text(ty.description)));
        values.push(value(&prog, Some("FriendlyTypeName"), text(ty.description)));
        values.push(value(format!(r"{prog}\DefaultIcon"), None, text(&icon)));
        values.push(value(
            format!(r"{prog}\Application"),
            Some("ApplicationName"),
            text(APPLICATION),
        ));
        values.push(value(
            format!(r"{prog}\shell\open\command"),
            None,
            text(&open),
        ));
        // The offer itself. Note what is *not* here: `.ext`'s own default value, which is the
        // one that would make Grind the handler.
        values.push(value(
            format!(r"Software\Classes\{}\OpenWithProgids", ty.extension),
            Some(ty.prog_id),
            Data::None,
        ));
        values.push(value(
            format!(r"{application}\SupportedTypes"),
            Some(ty.extension),
            text(""),
        ));
        values.push(value(
            format!(r"{CAPABILITIES}\FileAssociations"),
            Some(ty.extension),
            text(ty.prog_id),
        ));
    }

    values.push(value(
        &application,
        Some("FriendlyAppName"),
        text(APPLICATION),
    ));
    values.push(value(
        format!(r"{application}\shell\open\command"),
        None,
        text(&open),
    ));
    values.push(value(
        CAPABILITIES,
        Some("ApplicationName"),
        text(APPLICATION),
    ));
    values.push(value(
        CAPABILITIES,
        Some("ApplicationDescription"),
        text(DESCRIPTION),
    ));
    values.push(value(
        r"Software\RegisteredApplications",
        Some(APPLICATION),
        text(CAPABILITIES),
    ));
    values
}

/// Everything [`register`] wrote, as what to remove — whole keys where the key is ours, single
/// values where it is shared. `exe` names the `Applications\…` key, which is per file name.
pub fn unregister(exe: &str) -> Vec<Removal> {
    let mut removals: Vec<Removal> = TYPES
        .iter()
        .map(|ty| Removal::Tree(format!(r"Software\Classes\{}", ty.prog_id)))
        .collect();
    removals.extend(TYPES.iter().map(|ty| Removal::Value {
        key: format!(r"Software\Classes\{}\OpenWithProgids", ty.extension),
        name: ty.prog_id.to_owned(),
    }));
    removals.push(Removal::Tree(format!(
        r"Software\Classes\Applications\{}",
        file_name(exe)
    )));
    removals.push(Removal::Tree(r"Software\Grind".to_owned()));
    removals.push(Removal::Value {
        key: r"Software\RegisteredApplications".to_owned(),
        name: APPLICATION.to_owned(),
    });
    removals
}

/// Whether the offer has to be (re)written: when any type's command is missing, or runs a
/// different copy of the executable than this one. `registered` is what each of
/// [`witnesses`] holds, in that order.
pub fn needed(registered: &[Option<String>], exe: &str) -> bool {
    let open = command(exe);
    registered.len() != TYPES.len() || registered.iter().any(|r| r.as_deref() != Some(&open))
}

/// The keys [`needed`] reads: every ProgID's open command, one per type.
pub fn witnesses() -> Vec<String> {
    TYPES
        .iter()
        .map(|ty| format!(r"Software\Classes\{}\shell\open\command", ty.prog_id))
        .collect()
}

/// What `--register` and `--unregister` tell the user they did.
pub fn registered_message() -> String {
    let extensions: Vec<&str> = TYPES.iter().map(|ty| ty.extension).collect();
    format!(
        "Grind is now offered for {} — in Explorer's \"Open with\" menu and in Settings > Apps > \
         Default apps.\n\nYour default apps are unchanged: whatever opened these files before \
         still does.",
        extensions.join(", ")
    )
}

pub const UNREGISTERED_MESSAGE: &str = "Grind is no longer offered for any file type. Nothing \
else was changed.\n\nStarting grind-win32 again offers it again; delete the executable after \
this if you are removing it.";

#[cfg(windows)]
pub use registry::{offer, register_self, unregister_self};

/// The half that touches the registry: walk the lists above, and nothing else.
#[cfg(windows)]
mod registry {
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_NONE, REG_SZ, REG_VALUE_TYPE,
        RegCloseKey, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetKeyValueW,
    };
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    use windows::core::PCWSTR;

    use super::{Data, Removal, Value};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// This executable's own path, as a double-click must spell it.
    fn exe() -> Result<String, String> {
        std::env::current_exe()
            .map_err(|e| format!("where is this executable? {e}"))
            .map(|path| path.display().to_string())
    }

    /// Write every value in `values`, then tell Explorer the associations changed so the
    /// *Open with* menu reflects them without a sign-out.
    fn apply(values: &[Value]) -> Result<(), String> {
        for v in values {
            let key = wide(&v.key);
            let name = v.name.as_deref().map(wide);
            let name_ptr = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
            let (kind, bytes): (REG_VALUE_TYPE, Vec<u8>) = match &v.data {
                Data::Text(s) => (
                    REG_SZ,
                    wide(s).iter().flat_map(|unit| unit.to_le_bytes()).collect(),
                ),
                Data::None => (REG_NONE, Vec::new()),
            };
            // `RegSetKeyValueW` creates the subkey when it is missing, so one call is one value.
            // SAFETY: the key and name are NUL-terminated and, like the data, outlive the call;
            // the data pointer is `None` exactly when its length is zero.
            let set = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    PCWSTR(key.as_ptr()),
                    name_ptr,
                    kind.0,
                    (!bytes.is_empty()).then_some(bytes.as_ptr().cast()),
                    bytes.len() as u32,
                )
            };
            if set.is_err() {
                return Err(format!("could not write HKCU\\{}", v.key));
            }
        }
        notify();
        Ok(())
    }

    /// Remove every key and value in `removals`. One that is already gone is not an error:
    /// `--unregister` twice is the same as once.
    fn remove(removals: &[Removal]) -> Result<(), String> {
        for removal in removals {
            match removal {
                Removal::Tree(key) => {
                    let name = wide(key);
                    // SAFETY: `name` is NUL-terminated and outlives the call.
                    let result =
                        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(name.as_ptr())) };
                    if result.is_err() && result != ERROR_FILE_NOT_FOUND {
                        return Err(format!("could not remove HKCU\\{key}"));
                    }
                }
                Removal::Value { key, name } => {
                    let key_name = wide(key);
                    let mut open = HKEY::default();
                    // SAFETY: as above; `open` is closed on the one path that opened it.
                    if unsafe {
                        RegOpenKeyExW(
                            HKEY_CURRENT_USER,
                            PCWSTR(key_name.as_ptr()),
                            None,
                            KEY_SET_VALUE,
                            &mut open,
                        )
                    }
                    .is_err()
                    {
                        continue;
                    }
                    let value = wide(name);
                    // SAFETY: `open` is open for setting values and `value` outlives the call.
                    let result = unsafe { RegDeleteValueW(open, PCWSTR(value.as_ptr())) };
                    // SAFETY: `open` is not used again.
                    let _ = unsafe { RegCloseKey(open) };
                    if result.is_err() && result != ERROR_FILE_NOT_FOUND {
                        return Err(format!("could not remove {name} from HKCU\\{key}"));
                    }
                }
            }
        }
        notify();
        Ok(())
    }

    /// A key's default value as a string, or `None` when there is none.
    fn read_default(key: &str) -> Option<String> {
        let name = wide(key);
        let mut open = HKEY::default();
        // SAFETY: as above.
        unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(name.as_ptr()),
                None,
                KEY_READ,
                &mut open,
            )
        }
        .ok()
        .ok()?;
        let mut buffer = [0u16; 1024];
        let mut size = (buffer.len() * 2) as u32;
        let mut kind = REG_VALUE_TYPE::default();
        // SAFETY: `buffer` is `size` bytes long and outlives the call.
        let result = unsafe {
            RegQueryValueExW(
                open,
                PCWSTR::null(),
                None,
                Some(&mut kind),
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        // SAFETY: `open` is not used again.
        let _ = unsafe { RegCloseKey(open) };
        if result.is_err() || kind != REG_SZ {
            return None;
        }
        let units = (size as usize / 2).min(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..units]);
        Some(text.trim_end_matches('\0').to_owned())
    }

    fn notify() {
        // SAFETY: SHCNE_ASSOCCHANGED takes no items.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }

    /// What the window does on every start: write the offer when it is missing or names another
    /// copy of the executable, and otherwise nothing — so an ordinary launch touches no key and
    /// sends Explorer no notification. Best effort: a registry this account may not write is not
    /// a reason to refuse to open a document.
    pub fn offer() {
        let Ok(exe) = exe() else { return };
        let registered: Vec<Option<String>> = super::witnesses()
            .iter()
            .map(|key| read_default(key))
            .collect();
        if super::needed(&registered, &exe) {
            let _ = apply(&super::register(&exe));
        }
    }

    /// `--register`.
    pub fn register_self() -> Result<(), String> {
        apply(&super::register(&exe()?))
    }

    /// `--unregister`.
    pub fn unregister_self() -> Result<(), String> {
        remove(&super::unregister(&exe()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Program Files\Grind\grind-win32.exe";

    fn written(key: &str, name: Option<&str>) -> Option<Data> {
        register(EXE)
            .into_iter()
            .find(|v| v.key == key && v.name.as_deref() == name)
            .map(|v| v.data)
    }

    /// The whole point: an extension's own default value — the one that says which program
    /// *is* its handler — is never among the values written, for any type. What is written
    /// under the extension is one `OpenWithProgids` entry, which only adds Grind to the list.
    #[test]
    fn no_extension_is_taken_only_offered() {
        let values = register(EXE);
        for ty in TYPES {
            let own = format!(r"Software\Classes\{}", ty.extension);
            assert!(
                !values.iter().any(|v| v.key == own),
                "{} has a value written on the extension's own key — that would make Grind \
                 its default",
                ty.extension
            );
            assert!(
                values
                    .iter()
                    .filter(|v| v.key.starts_with(&format!("{own}\\")))
                    .all(|v| v.key == format!(r"{own}\OpenWithProgids")),
                "only OpenWithProgids is touched under {}",
                ty.extension
            );
            assert_eq!(
                written(&format!(r"{own}\OpenWithProgids"), Some(ty.prog_id)),
                Some(Data::None)
            );
        }
        assert!(
            !values.iter().any(|v| v.key.contains("UserChoice")),
            "UserChoice is the user's, and Windows guards it for that reason"
        );
    }

    #[cfg(feature = "xlsx")]
    #[test]
    fn excel_workbooks_are_offered_both_extensions() {
        for (ext, prog) in [
            (".xlsx", "Grind.Workbook"),
            (".xlsm", "Grind.MacroWorkbook"),
        ] {
            assert_eq!(
                written(
                    &format!(r"Software\Classes\{prog}\shell\open\command"),
                    None
                ),
                Some(Data::Text(format!("\"{EXE}\" \"%1\""))),
                "{ext}"
            );
            assert_eq!(
                written(r"Software\Grind\Capabilities\FileAssociations", Some(ext)),
                Some(Data::Text(prog.to_owned())),
                "{ext} is listed in Default apps"
            );
        }
    }

    #[test]
    fn default_apps_can_find_the_capabilities() {
        assert_eq!(
            written(r"Software\RegisteredApplications", Some("Grind")),
            Some(Data::Text(r"Software\Grind\Capabilities".to_owned()))
        );
        assert_eq!(
            written(
                r"Software\Classes\Applications\grind-win32.exe",
                Some("FriendlyAppName")
            ),
            Some(Data::Text("Grind".to_owned()))
        );
    }

    /// Every key `register` creates is ours or holds exactly one value of ours, and
    /// `unregister` removes each: a tree for what is wholly ours, a value where the key is
    /// shared. Nothing written survives an unregister, and nothing else is removed.
    #[test]
    fn unregister_removes_exactly_what_register_wrote() {
        let removals = unregister(EXE);
        let covered = |v: &Value| {
            removals.iter().any(|r| match r {
                Removal::Tree(tree) => v.key == *tree || v.key.starts_with(&format!("{tree}\\")),
                Removal::Value { key, name } => v.key == *key && v.name.as_deref() == Some(name),
            })
        };
        for v in register(EXE) {
            assert!(covered(&v), "{v:?} would survive --unregister");
        }
        for r in &removals {
            if let Removal::Tree(tree) = r {
                assert!(
                    !TYPES
                        .iter()
                        .any(|ty| tree.ends_with(&format!(r"\{}", ty.extension))),
                    "{tree} is an extension's key, which is not ours to delete"
                );
            }
        }
    }

    #[test]
    fn a_moved_or_damaged_offer_is_written_again_and_an_intact_one_is_not() {
        let all = |exe: &str| vec![Some(command(exe)); TYPES.len()];
        assert!(
            needed(&vec![None; TYPES.len()], EXE),
            "nothing registered yet"
        );
        assert!(!needed(&all(EXE), EXE));
        assert!(
            needed(&all(r"D:\grind-win32.exe"), EXE),
            "registered from the test disc, now run from somewhere else"
        );
        let mut one_gone = all(EXE);
        one_gone[0] = None;
        assert!(needed(&one_gone, EXE), "one ProgID deleted by hand");
        assert_eq!(witnesses().len(), TYPES.len());
        for key in witnesses() {
            assert!(
                register(EXE)
                    .iter()
                    .any(|v| v.key == key && v.name.is_none()),
                "{key} is read but never written"
            );
        }
    }

    #[test]
    fn the_command_quotes_both_paths() {
        assert_eq!(
            command(EXE),
            r#""C:\Program Files\Grind\grind-win32.exe" "%1""#
        );
        assert_eq!(file_name(EXE), "grind-win32.exe");
        assert_eq!(file_name("grind.exe"), "grind.exe");
    }
}
