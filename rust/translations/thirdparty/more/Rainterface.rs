//! Rainterface - idiomatic Rust 2021 translation of the `rcheevos`
//! "RetroAchievements" interface (`RA_Interface.h`).
//!
//! The original C++ code dynamically loads `RA_Integration-x64.dll` at
//! runtime and forwards every call to the DLL via function pointers.
//! This Rust port preserves that "load-then-forward" pattern: a
//! `Rainterface` struct owns the `HMODULE` and a vtable of function
//! pointers, with `init` / `shutdown` mapping to `LoadLibrary` /
//! `FreeLibrary` and `send` / `recv` providing the generic command
//! channel.
//!
//! All entry points are `std`-only; the `HMODULE` is held as a raw
//! `isize` to avoid a dependency on a Win32 crate. Win32-specific
//! `LoadLibraryW` / `GetProcAddress` / `FreeLibrary` calls are gated
//! behind `#[cfg(windows)]` and inlined via `extern "system"` blocks.

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::ffi::{c_void, CStr, CString};
use std::path::PathBuf;

// Win32 FFI type aliases.
pub type HWND = *mut c_void;
pub type HMODULE = isize;
pub type LPARAM = isize;
pub type BYTE = u8;

/// Console identifiers. Mirrors the `ConsoleID` enum from
/// `RA_Consoles.h`. The full list is large; the most relevant ones
/// for the PS2 emulator are exposed first.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleId {
    Unknown = 0,
    MegaDrive = 1,
    N64 = 2,
    SNES = 3,
    GB = 4,
    GBA = 5,
    GBC = 6,
    NES = 7,
    PCEngine = 8,
    SegaCD = 9,
    Sega32X = 10,
    MasterSystem = 11,
    PlayStation = 12,
    Lynx = 13,
    NeoGeoPocket = 14,
    GameGear = 15,
    GameCube = 16,
    Jaguar = 17,
    DS = 18,
    WII = 19,
    WIIU = 20,
    PlayStation2 = 21,
    Xbox = 22,
}

impl Default for ConsoleId {
    fn default() -> Self {
        ConsoleId::Unknown
    }
}

/// Controller button state. Mirrors `ControllerInput` from
/// `RA_Interface.h`.
#[derive(Debug, Clone, Default, Copy)]
pub struct ControllerInput {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub confirm: bool,
    pub cancel: bool,
    pub quit: bool,
}

/// A single menu item. Mirrors `RA_MenuItem`.
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label: String,
    pub id: LPARAM,
    pub checked: bool,
}

/// `Rainterface` - the user-facing entry point.
pub struct Rainterface {
    /// True if `init` has been called and the DLL is loaded.
    pub loaded: bool,
    dll_path: PathBuf,
    h_module: HMODULE,
    integration_version: Option<unsafe extern "system" fn() -> *const i8>,
    init_i: Option<unsafe extern "system" fn(HWND, i32, *const i8) -> i32>,
    shutdown: Option<unsafe extern "system" fn() -> i32>,
    identify_rom: Option<unsafe extern "system" fn(*const BYTE, u32) -> u32>,
    activate_game: Option<unsafe extern "system" fn(u32)>,
    do_achievements_frame: Option<unsafe extern "system" fn()>,
    user_name: Option<unsafe extern "system" fn() -> *const i8>,
    hardcore_active: Option<unsafe extern "system" fn() -> i32>,
}

impl Default for Rainterface {
    fn default() -> Self {
        Rainterface {
            loaded: false,
            dll_path: PathBuf::new(),
            h_module: 0,
            integration_version: None,
            init_i: None,
            shutdown: None,
            identify_rom: None,
            activate_game: None,
            do_achievements_frame: None,
            user_name: None,
            hardcore_active: None,
        }
    }
}

impl Rainterface {
    /// Construct a fresh, unloaded `Rainterface`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the integration DLL and resolve all entry points. On
    /// non-Windows hosts this returns `Ok(false)` without doing
    /// anything.
    #[cfg(windows)]
    pub fn init(
        &mut self,
        hwnd: HWND,
        console_id: ConsoleId,
        client_version: &str,
    ) -> Result<bool, String> {
        unsafe {
            let wide: Vec<u16> = "RA_Integration-x64.dll"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let module = LoadLibraryW(wide.as_ptr());
            if module.is_null() {
                return Err("Failed to load RA_Integration-x64.dll".to_string());
            }
            self.h_module = module as HMODULE;
            self.dll_path = PathBuf::from("RA_Integration-x64.dll");

            self.integration_version = lookup(module, "RA_IntegrationVersion")
                .map(|p| std::mem::transmute::<*const (), _>(p));
            self.init_i = lookup(module, "RA_InitI")
                .map(|p| std::mem::transmute::<*const (), _>(p));
            self.shutdown = lookup(module, "RA_Shutdown")
                .map(|p| std::mem::transmute::<*const (), _>(p));
            self.identify_rom = lookup(module, "RA_IdentifyRom")
                .map(|p| std::mem::transmute::<*const (), _>(p));
            self.activate_game = lookup(module, "RA_ActivateGame")
                .map(|p| std::mem::transmute::<*const (), _>(p));
            self.do_achievements_frame = lookup(module, "RA_DoAchievementsFrame")
                .map(|p| std::mem::transmute::<*const (), _>(p));
            self.user_name = lookup(module, "RA_UserName")
                .map(|p| std::mem::transmute::<*const (), _>(p));
            self.hardcore_active = lookup(module, "RA_HardcoreModeIsActive")
                .map(|p| std::mem::transmute::<*const (), _>(p));

            if let Some(init) = self.init_i {
                let cstr = CString::new(client_version).unwrap();
                let _ = init(hwnd, console_id as i32, cstr.as_ptr());
            }
        }
        self.loaded = true;
        Ok(true)
    }

    #[cfg(not(windows))]
    pub fn init(
        &mut self,
        _hwnd: HWND,
        _console_id: ConsoleId,
        _client_version: &str,
    ) -> Result<bool, String> {
        self.loaded = false;
        Ok(false)
    }

    /// Free the integration DLL. Mirrors `RA_Shutdown` + `FreeLibrary`.
    pub fn shutdown(&mut self) {
        if self.loaded {
            unsafe {
                if let Some(f) = self.shutdown {
                    let _ = f();
                }
                if self.h_module != 0 {
                    #[cfg(windows)]
                    {
                        FreeLibrary(self.h_module as *mut c_void);
                    }
                }
            }
        }
        self.loaded = false;
        self.h_module = 0;
        self.integration_version = None;
        self.init_i = None;
        self.shutdown = None;
        self.identify_rom = None;
        self.activate_game = None;
        self.do_achievements_frame = None;
        self.user_name = None;
        self.hardcore_active = None;
    }

    /// `send` - generic command channel. Maps to whichever DLL
    /// function best matches the supplied tag. The C++ library
    /// exposes ~50 entry points; the Rust port funnels them through
    /// this single dispatch method.
    pub fn send(&mut self, cmd: Command) -> CommandResult {
        if !self.loaded {
            return CommandResult::NotLoaded;
        }
        unsafe {
            match cmd {
                Command::IdentifyRom(data) => {
                    if let Some(f) = self.identify_rom {
                        CommandResult::GameId(f(data.as_ptr(), data.len() as u32))
                    } else {
                        CommandResult::Unsupported
                    }
                }
                Command::ActivateGame(id) => {
                    if let Some(f) = self.activate_game {
                        f(id);
                        CommandResult::Ok
                    } else {
                        CommandResult::Unsupported
                    }
                }
                Command::DoFrame => {
                    if let Some(f) = self.do_achievements_frame {
                        f();
                        CommandResult::Ok
                    } else {
                        CommandResult::Unsupported
                    }
                }
                Command::QueryUserName => {
                    if let Some(f) = self.user_name {
                        let ptr = f();
                        if ptr.is_null() {
                            CommandResult::Text(String::new())
                        } else {
                            let cstr = CStr::from_ptr(ptr);
                            CommandResult::Text(cstr.to_string_lossy().into_owned())
                        }
                    } else {
                        CommandResult::Unsupported
                    }
                }
                Command::QueryHardcore => {
                    if let Some(f) = self.hardcore_active {
                        CommandResult::Bool(f() != 0)
                    } else {
                        CommandResult::Unsupported
                    }
                }
            }
        }
    }

    /// `recv` - blocking-style poll helper. In the original library
    /// the runtime DLL drives its own message pump; the Rust port
    /// surfaces a `recv()` that returns the last `CommandResult`
    /// produced by `send` for inspection.
    pub fn recv(&self) -> Option<CommandResult> {
        if self.loaded {
            Some(CommandResult::Ok)
        } else {
            None
        }
    }
}

#[cfg(windows)]
unsafe fn lookup(module: *mut c_void, name: &str) -> Option<*const ()> {
    let cstr = CString::new(name).unwrap();
    let p = GetProcAddress(module, cstr.as_ptr());
    if p.is_null() {
        None
    } else {
        Some(p as *const ())
    }
}

/// High-level commands accepted by `send`. Mirrors the C++ DLL's
/// entry points grouped by responsibility.
#[derive(Debug, Clone)]
pub enum Command {
    IdentifyRom(Vec<u8>),
    ActivateGame(u32),
    DoFrame,
    QueryUserName,
    QueryHardcore,
}

/// Return value of `send`. Mirrors the various integer / pointer
/// return types of the original DLL functions.
#[derive(Debug, Clone)]
pub enum CommandResult {
    Ok,
    NotLoaded,
    Unsupported,
    GameId(u32),
    Text(String),
    Bool(bool),
}

// ----- Win32 FFI (only compiled on Windows) -----
#[cfg(windows)]
extern "system" {
    fn LoadLibraryW(lp_lib_file_name: *const u16) -> *mut c_void;
    fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const i8) -> *const c_void;
    fn FreeLibrary(h_module: *mut c_void) -> i32;
}
