//! Default-handler integration (`specs/projects/xlsx-file-association/`, Phase 5).
//!
//! Packaging (Phase 1) registers FreeCell as a **candidate** handler for `.xlsx`/`.csv`. This
//! module adds the explicit, **user-initiated** step of making FreeCell the *default* handler for an
//! extension, plus a status query so the welcome screen can offer it only when we are not already
//! the default.
//!
//! The public surface is small and platform-agnostic — [`default_status`] / [`make_default`] (and
//! the [`is_default_for_xlsx`] / [`make_default_for_xlsx`] convenience wrappers) — so callers
//! (`shell::welcome`) never touch LaunchServices, COM, or `xdg-mime`. Every messy per-OS FFI lives
//! behind `cfg` in the private [`platform`] submodule; the cfg-agnostic pieces ([`file_type`],
//! [`identifiers_match`]) carry the automated coverage and run on the Linux CI.
//!
//! **Robustness contract.** These are best-effort convenience calls, never load-bearing: detection
//! returns [`None`] on *any* failure (unsupported OS, missing tool, OS error) rather than guessing,
//! and no path panics or blocks the UI. Detection always compares against our **own** runtime
//! identity (macOS bundle id / Linux `.desktop` id / Windows ProgId), never a hardcoded app string.
//!
//! Per-OS mechanism:
//! - **macOS** — LaunchServices `LSCopyDefaultRoleHandlerForContentType` vs `CFBundleGetIdentifier`;
//!   set via `LSSetDefaultRoleHandlerForContentType` (silent, no prompt / no admin).
//! - **Linux** — `xdg-mime query default <mime>` vs our `.desktop` id; set via `xdg-mime default …`
//!   (silent). `xdg-mime` absent ⇒ unknown.
//! - **Windows** — COM `IApplicationAssociationRegistration::QueryCurrentDefault` vs our NSIS ProgId;
//!   cannot set silently (the Win10/11 UserChoice hash), so it opens `ms-settings:defaultapps` as a
//!   guided hand-off.

/// The result of a [`make_default`] request.
///
/// `#[allow(dead_code)]`: each variant is produced only on a subset of platforms (`SetSilently` on
/// macOS/Linux, `OpenedSettings` on Windows, `Unavailable` everywhere), so on any single build
/// target one or two variants are never constructed. Suppressing here keeps the type's cross-platform
/// shape honest without scattering per-arm `cfg`s over the callers that `match` it.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakeDefaultOutcome {
    /// The default was changed silently (macOS / Linux). The caller should re-query the status.
    SetSilently,
    /// We handed off to the OS "Default apps" Settings UI (Windows, where an app cannot set the
    /// default itself). The user completes the change there.
    OpenedSettings,
    /// Nothing was attempted — the platform is unsupported, the extension is unknown, or an OS call
    /// failed. Treated exactly like "leave the UI as-is".
    Unavailable,
}

/// A spreadsheet file type we can register as a handler for, resolved from a bare extension by
/// [`file_type`]. Holds the identifier each OS needs; which fields are read depends on the build
/// target.
///
/// `#[allow(dead_code)]`: every field is consumed by exactly one platform arm (`uti` → macOS,
/// `mime` → Linux, `windows_progid` + `extension` → Windows), so on any single target the others are
/// unused. Suppressing here is cleaner than per-field `cfg`s and keeps the mapping table readable.
#[allow(dead_code)]
struct FileType {
    /// The lowercase extension without a leading dot, e.g. `"xlsx"`.
    extension: &'static str,
    /// macOS Uniform Type Identifier (the system UTI for the format).
    uti: &'static str,
    /// Linux / freedesktop MIME type.
    mime: &'static str,
    /// The ProgId cargo-packager's NSIS installer registers on Windows for this extension.
    windows_progid: &'static str,
}

/// Maps a file extension to the per-OS identifiers FreeCell registers for it, or `None` for an
/// extension we don't handle. Accepts an extension with or without a leading dot and in any case
/// (`"xlsx"`, `".XLSX"`).
///
/// Kept generic over both registered extensions (`xlsx`, `csv`) even though the welcome screen only
/// wires `xlsx` today — so adding the `.csv` button later is a one-line change.
fn file_type(extension: &str) -> Option<FileType> {
    match extension
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "xlsx" => Some(FileType {
            extension: "xlsx",
            uti: "org.openxmlformats.spreadsheetml.sheet",
            mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            // cargo-packager's NSIS installer writes `Software\Classes\.xlsx` (default) = the
            // `FileAssociation.nsh` `APP_ASSOCIATE` FILECLASS, which its `installer.nsi` template
            // fills as `{{or association.name ext}}` — i.e. our packaging `name`. Our xlsx entry's
            // `name` is "Excel Workbook" (crate `Cargo.toml` file-associations), so that literal is
            // our registered ProgId. This is the one genuinely uncertain identifier of the feature
            // (see the project spec) — the comparison below is case-insensitive to stay robust.
            windows_progid: "Excel Workbook",
        }),
        "csv" => Some(FileType {
            extension: "csv",
            uti: "public.comma-separated-values-text",
            mime: "text/csv",
            windows_progid: "CSV Document",
        }),
        _ => None,
    }
}

/// Compares two handler identifiers (macOS bundle ids, Linux `.desktop` ids, Windows ProgIds) for
/// equality, ignoring ASCII case and surrounding whitespace. Empty on either side is never a match
/// (an unset / unreadable identifier is not "us").
fn identifiers_match(ours: &str, theirs: &str) -> bool {
    let ours = ours.trim();
    let theirs = theirs.trim();
    !ours.is_empty() && !theirs.is_empty() && ours.eq_ignore_ascii_case(theirs)
}

/// Whether FreeCell is the current OS default handler for `extension`: `Some(true)` = yes,
/// `Some(false)` = definitely not, `None` = unknown / unsupported / an OS call failed.
pub fn default_status(extension: &str) -> Option<bool> {
    platform::default_status(&file_type(extension)?)
}

/// Requests that FreeCell become the OS default handler for `extension`. See [`MakeDefaultOutcome`]
/// for how the result differs per platform.
pub fn make_default(extension: &str) -> MakeDefaultOutcome {
    match file_type(extension) {
        Some(file_type) => platform::make_default(&file_type),
        None => MakeDefaultOutcome::Unavailable,
    }
}

/// [`default_status`] for `.xlsx` — the extension the welcome screen surfaces.
pub fn is_default_for_xlsx() -> Option<bool> {
    default_status("xlsx")
}

/// [`make_default`] for `.xlsx` — the extension the welcome screen surfaces.
pub fn make_default_for_xlsx() -> MakeDefaultOutcome {
    make_default("xlsx")
}

// ---------------------------------------------------------------------------------------------
// macOS — LaunchServices via raw CoreFoundation / CoreServices FFI (no extra crate; the frameworks
// ship with the OS). Detection compares the default UTI handler to our own bundle identifier.
// ---------------------------------------------------------------------------------------------
#[cfg(target_os = "macos")]
mod platform {
    use super::{identifiers_match, FileType, MakeDefaultOutcome};
    use std::os::raw::{c_char, c_void};

    // Opaque CoreFoundation handles — we treat every CF object as a pointer and manage its lifetime
    // explicitly (release the +1 results of the "Copy"/"Create" rule; borrow the "Get" rule).
    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFBundleRef = *const c_void;
    type CFAllocatorRef = *const c_void;
    type Boolean = u8;
    type CFIndex = isize;
    type OSStatus = i32;
    type LSRolesMask = u32;

    /// `kCFStringEncodingUTF8`.
    const UTF8: u32 = 0x0800_0100;
    /// `kLSRolesAll` — match a handler registered under any role (viewer/editor/…).
    const ROLES_ALL: LSRolesMask = 0xFFFF_FFFF;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFBundleGetMainBundle() -> CFBundleRef;
        fn CFBundleGetIdentifier(bundle: CFBundleRef) -> CFStringRef;
        fn CFStringCreateWithBytes(
            alloc: CFAllocatorRef,
            bytes: *const u8,
            num_bytes: CFIndex,
            encoding: u32,
            is_external_representation: Boolean,
        ) -> CFStringRef;
        fn CFStringGetLength(the_string: CFStringRef) -> CFIndex;
        fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
        fn CFStringGetCString(
            the_string: CFStringRef,
            buffer: *mut c_char,
            buffer_size: CFIndex,
            encoding: u32,
        ) -> Boolean;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn LSCopyDefaultRoleHandlerForContentType(
            content_type: CFStringRef,
            role: LSRolesMask,
        ) -> CFStringRef;
        fn LSSetDefaultRoleHandlerForContentType(
            content_type: CFStringRef,
            role: LSRolesMask,
            handler_bundle_id: CFStringRef,
        ) -> OSStatus;
    }

    /// Owns a CoreFoundation string created under the "Create"/"Copy" rule and releases it exactly
    /// once on drop.
    struct CfString(CFStringRef);

    impl CfString {
        /// Builds an immutable CF string from Rust text, or `None` on allocation failure.
        fn new(text: &str) -> Option<Self> {
            // SAFETY: `text` is a valid UTF-8 byte slice; `CFStringCreateWithBytes` copies it, so the
            // borrow need not outlive the call. A null return (allocation failure) becomes `None`.
            let handle = unsafe {
                CFStringCreateWithBytes(
                    std::ptr::null(),
                    text.as_ptr(),
                    text.len() as CFIndex,
                    UTF8,
                    0,
                )
            };
            (!handle.is_null()).then_some(Self(handle))
        }
    }

    impl Drop for CfString {
        fn drop(&mut self) {
            // SAFETY: `self.0` is a non-null +1 reference we own (Create/Copy rule); release once.
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    /// Reads a borrowed (not owned) CFString handle into an owned Rust `String`.
    fn cfstring_to_string(handle: CFStringRef) -> Option<String> {
        if handle.is_null() {
            return None;
        }
        // SAFETY: `handle` is a valid CFString for the duration of this call. We size the buffer with
        // `CFStringGetMaximumSizeForEncoding` (+1 for the NUL) so `CFStringGetCString` cannot
        // overflow it, then read the written C string back through `CStr`.
        unsafe {
            let length = CFStringGetLength(handle);
            let capacity = CFStringGetMaximumSizeForEncoding(length, UTF8) + 1;
            if capacity <= 0 {
                return None;
            }
            let mut buffer = vec![0 as c_char; capacity as usize];
            if CFStringGetCString(handle, buffer.as_mut_ptr(), capacity, UTF8) == 0 {
                return None;
            }
            std::ffi::CStr::from_ptr(buffer.as_ptr())
                .to_str()
                .ok()
                .map(str::to_owned)
        }
    }

    /// FreeCell's own bundle identifier, or `None` when running unbundled (e.g. `cargo run`), where
    /// Launch Services has no identity to compare against.
    fn our_bundle_id() -> Option<String> {
        // SAFETY: both are "Get" rule calls — we borrow, we do not release. A missing main bundle or
        // identifier (unbundled binary) yields a null handle, which `cfstring_to_string` maps to
        // `None`.
        unsafe {
            let bundle = CFBundleGetMainBundle();
            if bundle.is_null() {
                return None;
            }
            cfstring_to_string(CFBundleGetIdentifier(bundle))
        }
    }

    pub(super) fn default_status(file_type: &FileType) -> Option<bool> {
        let ours = our_bundle_id()?;
        let uti = CfString::new(file_type.uti)?;
        // SAFETY: `uti.0` is a valid CFString; the returned handler id is a +1 "Copy" result we take
        // ownership of via `CfString` so it is released on scope exit.
        let handler = unsafe { LSCopyDefaultRoleHandlerForContentType(uti.0, ROLES_ALL) };
        if handler.is_null() {
            // No handler registered for this type at all → we are certainly not the default.
            return Some(false);
        }
        let handler = CfString(handler);
        let theirs = cfstring_to_string(handler.0)?;
        Some(identifiers_match(&ours, &theirs))
    }

    pub(super) fn make_default(file_type: &FileType) -> MakeDefaultOutcome {
        let Some(ours) = our_bundle_id() else {
            return MakeDefaultOutcome::Unavailable;
        };
        let (Some(uti), Some(bundle)) = (CfString::new(file_type.uti), CfString::new(&ours)) else {
            return MakeDefaultOutcome::Unavailable;
        };
        // SAFETY: both args are valid CFStrings owned by `uti` / `bundle` for the call's duration.
        // This sets the default silently (no prompt, no admin) — `noErr` (0) means success.
        let status = unsafe { LSSetDefaultRoleHandlerForContentType(uti.0, ROLES_ALL, bundle.0) };
        if status == 0 {
            MakeDefaultOutcome::SetSilently
        } else {
            MakeDefaultOutcome::Unavailable
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Linux — shell out to `xdg-mime`. Detection compares the MIME's default `.desktop` id to ours.
// ---------------------------------------------------------------------------------------------
#[cfg(target_os = "linux")]
mod platform {
    use super::{identifiers_match, FileType, MakeDefaultOutcome};
    use std::process::Command;

    /// The `.desktop` id cargo-packager's `.deb` installs for FreeCell. cargo-packager names the
    /// entry `{main_binary_name}.desktop` under `usr/share/applications/`, and our bin target is
    /// `freecell` (crate `Cargo.toml` `[[bin]] name = "freecell"`), so the installed entry — and the
    /// id `xdg-mime` reports handlers by — is `freecell.desktop`.
    const OUR_DESKTOP_ID: &str = "freecell.desktop";

    pub(super) fn default_status(file_type: &FileType) -> Option<bool> {
        let output = Command::new("xdg-mime")
            .args(["query", "default", file_type.mime])
            .output()
            .ok()?; // `xdg-mime` missing → unknown
        if !output.status.success() {
            return None;
        }
        let current = String::from_utf8_lossy(&output.stdout);
        let current = current.trim();
        if current.is_empty() {
            // No default registered for this MIME yet → we are definitely not it.
            return Some(false);
        }
        Some(identifiers_match(OUR_DESKTOP_ID, current))
    }

    pub(super) fn make_default(file_type: &FileType) -> MakeDefaultOutcome {
        match Command::new("xdg-mime")
            .args(["default", OUR_DESKTOP_ID, file_type.mime])
            .status()
        {
            Ok(status) if status.success() => MakeDefaultOutcome::SetSilently,
            _ => MakeDefaultOutcome::Unavailable,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Windows — COM `IApplicationAssociationRegistration` for detection; a guided Settings hand-off for
// make-default (Win10/11 protect the default with a per-user UserChoice hash an app can't forge).
// ---------------------------------------------------------------------------------------------
#[cfg(windows)]
mod platform {
    use super::{identifiers_match, FileType, MakeDefaultOutcome};
    use std::ffi::c_void;
    use std::process::Command;
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        ApplicationAssociationRegistration, IApplicationAssociationRegistration, AL_EFFECTIVE,
        AT_FILEEXTENSION,
    };

    pub(super) fn default_status(file_type: &FileType) -> Option<bool> {
        let current = current_default_progid(file_type.extension)?;
        Some(identifiers_match(file_type.windows_progid, &current))
    }

    /// Reads the ProgId Windows treats as the effective default for `.<extension>`, or `None` on any
    /// COM failure (degrade to "unknown", never panic).
    fn current_default_progid(extension: &str) -> Option<String> {
        // `.<ext>` as a NUL-terminated UTF-16 string backing the `PCWSTR` query arg; kept alive for
        // the whole call.
        let mut query: Vec<u16> = format!(".{extension}").encode_utf16().collect();
        query.push(0);

        // SAFETY: COM interop. gpui already runs the UI thread as an STA, so `CoInitializeEx` here is
        // redundant (returns `S_FALSE` / `RPC_E_CHANGED_MODE`, both harmless) — we call it only to be
        // robust if invoked before that init, and never `CoUninitialize` (that is gpui's apartment,
        // not ours). Every fallible result maps to `None`. The returned `PWSTR` is a
        // `CoTaskMemAlloc` buffer we read before freeing with `CoTaskMemFree`.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let registration: IApplicationAssociationRegistration = CoCreateInstance(
                &ApplicationAssociationRegistration,
                None,
                CLSCTX_INPROC_SERVER,
            )
            .ok()?;
            let handler = registration
                .QueryCurrentDefault(PCWSTR(query.as_ptr()), AT_FILEEXTENSION, AL_EFFECTIVE)
                .ok()?;
            if handler.is_null() {
                return None;
            }
            let progid = handler.to_string().ok();
            CoTaskMemFree(Some(handler.0 as *const c_void));
            progid
        }
    }

    pub(super) fn make_default(_file_type: &FileType) -> MakeDefaultOutcome {
        // No silent path on Win10/11: the default handler is guarded by a per-user UserChoice hash an
        // app cannot compute. The supported experience is to open Settings → "Default apps" and let
        // the user confirm. `explorer.exe <uri>` launches the `ms-settings:` page with no console
        // flash; we use it rather than `ShellExecuteW` because that function's exact `windows` 0.61
        // signature can't be compile-checked on the Linux dev host, and a URI launch is equivalent
        // and certain. (`explorer` can return a non-zero exit even on success, so we only check that
        // the spawn itself succeeded.)
        match Command::new("explorer.exe")
            .arg("ms-settings:defaultapps")
            .spawn()
        {
            Ok(_) => MakeDefaultOutcome::OpenedSettings,
            Err(_) => MakeDefaultOutcome::Unavailable,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Any other target — no-op so the crate still builds. Detection is always "unknown".
// ---------------------------------------------------------------------------------------------
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod platform {
    use super::{FileType, MakeDefaultOutcome};

    pub(super) fn default_status(_file_type: &FileType) -> Option<bool> {
        None
    }

    pub(super) fn make_default(_file_type: &FileType) -> MakeDefaultOutcome {
        MakeDefaultOutcome::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::{file_type, identifiers_match};

    #[test]
    fn file_type_maps_known_extensions() {
        let xlsx = file_type("xlsx").expect("xlsx is registered");
        assert_eq!(xlsx.extension, "xlsx");
        assert_eq!(xlsx.uti, "org.openxmlformats.spreadsheetml.sheet");
        assert_eq!(
            xlsx.mime,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
        assert_eq!(xlsx.windows_progid, "Excel Workbook");

        let csv = file_type("csv").expect("csv is registered");
        assert_eq!(csv.mime, "text/csv");
        assert_eq!(csv.windows_progid, "CSV Document");
    }

    #[test]
    fn file_type_normalizes_dot_and_case() {
        // A leading dot and any casing resolve to the same entry.
        assert_eq!(file_type(".xlsx").expect("with dot").extension, "xlsx");
        assert_eq!(file_type("XLSX").expect("uppercase").extension, "xlsx");
        assert_eq!(file_type(".CsV").expect("mixed").extension, "csv");
    }

    #[test]
    fn file_type_rejects_unknown_extensions() {
        assert!(file_type("txt").is_none());
        assert!(file_type("xls").is_none());
        assert!(file_type("").is_none());
    }

    #[test]
    fn identifiers_match_is_case_insensitive_and_trimmed() {
        assert!(identifiers_match("freecell.desktop", "freecell.desktop"));
        // Launch Services / xdg report ids in varying case + with surrounding whitespace.
        assert!(identifiers_match("freecell.desktop", " FreeCell.Desktop\n"));
        assert!(identifiers_match(
            "net.scosman.freecell",
            "NET.SCOSMAN.FREECELL"
        ));
    }

    #[test]
    fn identifiers_match_rejects_mismatches_and_empties() {
        assert!(!identifiers_match(
            "freecell.desktop",
            "libreoffice-calc.desktop"
        ));
        assert!(!identifiers_match("", "freecell.desktop"));
        assert!(!identifiers_match("freecell.desktop", ""));
        assert!(!identifiers_match("   ", "freecell.desktop"));
    }
}
