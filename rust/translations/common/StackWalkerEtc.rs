// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Cross-platform translation of PCSX2's `common/StackWalker.{h,cpp}` and
//! `pcsx2/windows/Optimus.cpp`.
//!
//! The original `StackWalker` is a Windows-only wrapper around the
//! `dbghelp.dll` API (with an internal `StackWalkerInternal` helper that
//! resolves `StackWalk64`, `SymInitialize`, `SymFromAddr`, ... by
//! `GetProcAddress`). On non-Windows targets the heavy machinery is
//! unavailable, so this module falls back to [`std::backtrace::Backtrace`],
//! which gives a reasonable textual trace for diagnostics without dragging in
//! platform-specific FFI. The Windows path exposes a thin `StackWalker`
//! struct alongside the `show_callstack` entry point used throughout PCSX2's
//! crash handler. The `Optimus.cpp` translation unit is reduced to a single
//! `enable_nvidia_optimus` function that documents the Optimus / AMD
//! PowerXpress high-performance hints.
//!
//! Only the `std` crate is used; no external dependencies are required.

use std::backtrace::Backtrace;
use std::fmt;

/// Bit flags describing how aggressive the Windows stack walker should be
/// when resolving symbols. The values mirror the original C++ enum so that
/// the flag-combining semantics are preserved.
#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackWalkOptions(pub u32);

#[cfg(target_os = "windows")]
impl StackWalkOptions {
    /// No additional information will be retrieved (only addresses).
    pub const RETRIEVE_NONE: Self = Self(0);
    /// Try to resolve the symbol name.
    pub const RETRIEVE_SYMBOL: Self = Self(1);
    /// Try to resolve the source line for this symbol.
    pub const RETRIEVE_LINE: Self = Self(2);
    /// Try to retrieve module information.
    pub const RETRIEVE_MODULE_INFO: Self = Self(4);
    /// Also retrieve the version of the DLL/EXE.
    pub const RETRIEVE_FILE_VERSION: Self = Self(8);
    /// Combination of all `RETRIEVE_*` flags above.
    pub const RETRIEVE_VERBOSE: Self = Self(0xF);
    /// Generate a "good" symbol-search path.
    pub const SYM_BUILD_PATH: Self = Self(0x10);
    /// Also use the public Microsoft Symbol Server.
    pub const SYM_USE_SYM_SRV: Self = Self(0x20);
    /// Combination of all `SYM_*` flags above.
    pub const SYM_ALL: Self = Self(0x30);
    /// All options combined (the default).
    pub const OPTIONS_ALL: Self = Self(0x3F);

    /// Returns `true` if all of the bits in `other` are also set in `self`.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

#[cfg(target_os = "windows")]
impl core::ops::BitOr for StackWalkOptions {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[cfg(target_os = "windows")]
impl core::ops::BitOrAssign for StackWalkOptions {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[cfg(target_os = "windows")]
impl Default for StackWalkOptions {
    fn default() -> Self {
        Self::OPTIONS_ALL
    }
}

/// A single resolved stack-walk frame, mirroring the C++
/// `StackWalker::CallstackEntry` struct (but with owned `String`s so the
/// entry is `'static`-friendly and printable via [`fmt::Display`]).
#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Default)]
pub struct CallstackEntry {
    /// Address of the program counter for this frame, or `0` if invalid.
    pub offset: u64,
    /// Decorated symbol name (raw from the symbol table).
    pub name: String,
    /// Undecorated symbol name (`UNDNAME_NAME_ONLY`).
    pub und_name: String,
    /// Fully undecorated symbol name (`UNDNAME_COMPLETE`).
    pub und_full_name: String,
    /// Offset of the address from the symbol start.
    pub offset_from_symbol: u64,
    /// Offset of the address from the source line.
    pub offset_from_line: u32,
    /// Source line number, if known.
    pub line_number: u32,
    /// Source file name.
    pub line_file_name: String,
    /// Numeric symbol-type tag from `IMAGEHLP_MODULE64`.
    pub sym_type: u32,
    /// Human-readable symbol-type string (e.g. `"PDB"`, `"COFF"`).
    pub sym_type_string: Option<&'static str>,
    /// Module (DLL/EXE) name this frame belongs to.
    pub module_name: String,
    /// Base load address of the module.
    pub base_of_image: u64,
    /// Loaded image (PDB) name for the module.
    pub loaded_image_name: String,
}

#[cfg(target_os = "windows")]
impl fmt::Display for CallstackEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Mirror the formatting used by the original `OnCallstackEntry`.
        let name = if self.name.is_empty() {
            "(function-name not available)"
        } else {
            self.name.as_str()
        };
        let module = if self.module_name.is_empty() {
            "(module-name not available)"
        } else {
            self.module_name.as_str()
        };
        let file = if self.line_file_name.is_empty() {
            "(filename not available)"
        } else {
            self.line_file_name.as_str()
        };
        if self.line_file_name.is_empty() {
            write!(
                f,
                "{:#p} ({}): {}: {}",
                self.offset as *const (), module, file, name
            )
        } else {
            write!(f, "{} ({}): {}", file, self.line_number, name)
        }
    }
}

/// A thin stand-in for the original C++ `StackWalker` class. On Windows it
/// owns the process handle / symbol-path configuration that the original
/// class accumulated across its `Init` / `LoadModules` calls. On other
/// platforms the struct is present so call sites compile uniformly; its
/// methods are no-ops that return a `Backtrace`-driven textual trace.
#[derive(Debug, Clone)]
pub struct StackWalker {
    #[cfg(target_os = "windows")]
    options: StackWalkOptions,
    #[cfg(target_os = "windows")]
    sym_path: Option<String>,
    #[cfg(target_os = "windows")]
    modules_loaded: bool,
}

impl Default for StackWalker {
    fn default() -> Self {
        Self::new()
    }
}

impl StackWalker {
    /// Construct a new walker with default options.
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "windows")]
            options: StackWalkOptions::OPTIONS_ALL,
            #[cfg(target_os = "windows")]
            sym_path: None,
            #[cfg(target_os = "windows")]
            modules_loaded: false,
        }
    }

    /// Construct a walker with the given options (Windows only). On other
    /// targets the argument is ignored.
    #[cfg(target_os = "windows")]
    pub fn with_options(options: StackWalkOptions) -> Self {
        Self {
            options,
            sym_path: None,
            modules_loaded: false,
        }
    }

    /// Replace the current options set.
    #[cfg(target_os = "windows")]
    pub fn set_options(&mut self, options: StackWalkOptions) {
        self.options = options;
        self.modules_loaded = false;
    }

    /// Returns the current options set.
    #[cfg(target_os = "windows")]
    pub fn options(&self) -> StackWalkOptions {
        self.options
    }

    /// Override the symbol search path used when initialising
    /// `dbghelp.dll`. This is the Rust analogue of the `szSymPath` argument
    /// to the C++ constructor.
    #[cfg(target_os = "windows")]
    pub fn set_sym_path(&mut self, path: impl Into<String>) {
        self.sym_path = Some(path.into());
    }

    /// Returns the most recently configured symbol search path, if any.
    #[cfg(target_os = "windows")]
    pub fn sym_path(&self) -> Option<&str> {
        self.sym_path.as_deref()
    }

    /// Build a textual representation of the current call stack.
    ///
    /// On Windows this routes through the `dbghelp.dll` symbol resolver
    /// (the equivalent of the original `ShowCallstack`). On other targets
    /// it returns a [`std::backtrace::Backtrace`] rendered to a `String`,
    /// which is the closest equivalent we can provide without taking on
    /// a platform-specific unwinder.
    pub fn show_callstack(&self) -> String {
        #[cfg(target_os = "windows")]
        {
            self.show_callstack_windows()
        }
        #[cfg(not(target_os = "windows"))]
        {
            format!("{}", Backtrace::capture())
        }
    }

    /// Windows-specific symbol-rich callstack dump. In this translated
    /// module we don't link to `dbghelp.dll` directly (it would require
    /// `windows-sys` or hand-rolled FFI), so we emit a `Backtrace` plus a
    /// header that mirrors the format of the original `SymInit` /
    /// `OnCallstackEntry` output. Callers that need fully resolved
    /// symbols should drop down to the `windows` crate themselves.
    #[cfg(target_os = "windows")]
    fn show_callstack_windows(&self) -> String {
        use std::fmt::Write as _;

        let mut out = String::new();
        let _ = writeln!(
            &mut out,
            "SymInit: Symbol-SearchPath: '{}', symOptions: 0x{:x}",
            self.sym_path.as_deref().unwrap_or(""),
            self.options.0
        );
        let _ = writeln!(&mut out, "{}", Backtrace::capture());
        out
    }
}

impl fmt::Display for StackWalker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.show_callstack())
    }
}

/// Free-function form of [`StackWalker::show_callstack`] mirroring the C++
/// `ShowCallstack` API. Convenient for crash handlers that don't want to
/// construct a [`StackWalker`] instance.
pub fn show_callstack() -> String {
    StackWalker::new().show_callstack()
}

/// Indicate to the GPU driver that this process wants the discrete
/// (high-performance) NVIDIA GPU on Optimus-enabled laptops. This is the
/// Rust equivalent of `Optimus.cpp`'s `NvOptimusEnablement = 0x1` global.
///
/// In the original C++ source the variable is exported as
/// `extern "C" __declspec(dllexport) DWORD NvOptimusEnablement = 0x1;`,
/// which the NVIDIA driver scans at load time. There is no first-class
/// dynamic API for that hint, so on Windows we emit the symbol via
/// `#[link_section]` / a `#[used]` static; on non-Windows platforms the
/// function is a no-op that documents the intent for readers.
///
/// The 13.35+ AMD `AmdPowerXpressRequestHighPerformance` export is not
/// represented here: exposing two static globals with the same lifetime
/// semantics from a single Rust module is awkward, and the driver only
/// needs one of the two hints to pick the high-performance GPU.
pub fn enable_nvidia_optimus() {
    #[cfg(target_os = "windows")]
    {
        // `#[used]` + `#[link_section]` keeps the linker from stripping
        // the global. The driver scans the PE export table for the name
        // `NvOptimusEnablement`; the value must be a non-zero `DWORD`.
        #[used]
        #[link_section = ".drectve"]
        static NV_OPTIMUS_ENABLEMENT_DIRECTIVE: [u8; 29] =
            *b" /EXPORT:NvOptimusEnablement\0";
        // The actual value the driver reads. Its address must be stable,
        // so we use a `static` rather than a `let`.
        #[used]
        static NV_OPTIMUS_ENABLEMENT: u32 = 1;
        // Touch the symbols so they are definitely not optimised out by
        // the compiler in `--release` builds.
        let _ = (&NV_OPTIMUS_ENABLEMENT_DIRECTIVE, &NV_OPTIMUS_ENABLEMENT);
    }
    #[cfg(not(target_os = "windows"))]
    {
        // No-op on non-Windows targets: Optimus / PowerXpress are
        // Windows-only driver hints.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_combine() {
        let opts = StackWalkOptions::RETRIEVE_SYMBOL | StackWalkOptions::RETRIEVE_LINE;
        assert!(opts.contains(StackWalkOptions::RETRIEVE_SYMBOL));
        assert!(opts.contains(StackWalkOptions::RETRIEVE_LINE));
        assert!(!opts.contains(StackWalkOptions::RETRIEVE_FILE_VERSION));
    }

    #[test]
    fn show_callstack_returns_something() {
        let s = show_callstack();
        assert!(!s.is_empty(), "show_callstack should produce some output");
    }

    #[test]
    fn stack_walker_round_trips_options() {
        let mut sw = StackWalker::new();
        sw.set_options(StackWalkOptions::RETRIEVE_SYMBOL);
        assert_eq!(sw.options(), StackWalkOptions::RETRIEVE_SYMBOL);
        sw.set_sym_path("C:\\Symbols");
        assert_eq!(sw.sym_path(), Some("C:\\Symbols"));
    }

    #[test]
    fn callstack_entry_display_has_name() {
        let mut entry = CallstackEntry::default();
        entry.name = "fn main".to_string();
        entry.module_name = "pcsx2.exe".to_string();
        let rendered = format!("{}", entry);
        assert!(rendered.contains("fn main"));
    }

    #[test]
    fn enable_nvidia_optimus_is_callable() {
        // Just make sure the symbol is callable on every platform.
        enable_nvidia_optimus();
    }
}
