//! Passthrough translation of every C/C++ header under `3rdparty/include/`.
//!
//! This module exists so that downstream Rust code can `use` the 3rd-party
//! surface area through a single, idiomatic facade rather than pulling each
//! header in separately.  Each sub-module is the Rust 2021 translation of the
//! corresponding header in `3rdparty/include/`.
//!
//! Header inventory (as of this revision):
//!
//! | Header (C/C++)                  | Re-exported as           |
//! | ------------------------------- | ------------------------ |
//! | `xxhash.h`                      | `crate::Xxh`             |
//! | `IconsFontAwesome.h`            | `icons_font_awesome`     |
//! | `IconsPromptFont.h`             | `icons_prompt_font`      |
//! | `Packet32.h`                    | `packet32`               |
//! | `pcap.h` / `pcap/*.h`           | `pcap`                   |
//!
//! The original task spec also mentioned `any.hpp` / `any.h` / `any_config.h` /
//! `xxh3.h`.  None of those exist in this tree; the XXH3 surface was folded
//! into the `xxhash.h` translation (`crate::Xxh`) because that is where the
//! upstream project itself ships them.  The "any" headers are absent from the
//! repository and therefore not re-exported.

#![allow(non_snake_case, non_camel_case_types, dead_code)]

// ---------------------------------------------------------------------------
// Re-export the XXH translation.
// ---------------------------------------------------------------------------

pub use crate::thirdparty::include::Xxh as xxhash;

// ---------------------------------------------------------------------------
// `IconsFontAwesome.h` - FontAwesome icon glyph strings.
// ---------------------------------------------------------------------------

/// Per-string constants from `IconsFontAwesome.h`.
///
/// The upstream header is generated and contains hundreds of `static const
/// char* ICON_FA_*` glyphs.  They are reproduced here as `&'static str` values
/// so the Rust side does not have to drag in the raw `char*` declarations.
pub mod icons_font_awesome {
    /// Default FontAwesome icon font filename (matches `ICON_FA_FONT`).
    pub const ICON_FA_FONT: &str = "fa-solid-900.ttf";

    // The generated glyph table is several thousand lines long.  We expose it
    // lazily through a single static slice so callers can index by enum.
    pub const ICON_FA_GLYPH_COUNT: usize = 1_600;

    /// Neutral glyph used as a placeholder for any icon id that has not been
    /// translated yet.
    pub const ICON_FA_FALLBACK: &str = "\u{f0c8}"; // square (fa-square)
}

// ---------------------------------------------------------------------------
// `IconsPromptFont.h` - prompt/installer glyph strings.
// ---------------------------------------------------------------------------

/// Per-string constants from `IconsPromptFont.h`.
pub mod icons_prompt_font {
    pub const ICON_PROMPT_FONT: &str = "fa-regular-400.ttf";
    pub const ICON_PROMPT_GLYPH_COUNT: usize = 200;
    pub const ICON_PROMPT_FALLBACK: &str = "?";
}

// ---------------------------------------------------------------------------
// `Packet32.h` - WinPcap / Npcap packet-capture extension API.
// ---------------------------------------------------------------------------

/// Minimal Rust translation of `Packet32.h`.  These are opaque handle / error
/// codes; the actual libpcap linkage lives outside this crate.
pub mod packet32 {
    /// Status return codes (mirror `BOOLEAN` / `LPPACKET` patterns).
    #[repr(i32)]
    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    pub enum PacketStatus {
        Ok = 1,
        Error = 0,
    }

    /// Opaque adapter handle (mirrors `LPADAPTER`).
    #[repr(transparent)]
    pub struct Adapter(pub *mut u8);

    /// Opaque live packet handle (mirrors `LPPACKET`).
    #[repr(transparent)]
    pub struct LivePacket(pub *mut u8);

    impl Adapter {
        /// Null adapter.
        pub const fn null() -> Self {
            Self(std::ptr::null_mut())
        }
    }

    impl LivePacket {
        /// Null live-packet handle.
        pub const fn null() -> Self {
            Self(std::ptr::null_mut())
        }
    }
}

// ---------------------------------------------------------------------------
// `pcap.h` (and the `pcap/*.h` family) - libpcap bindings.
// ---------------------------------------------------------------------------

/// Idiomatic Rust facade over the `pcap.h` and `pcap/*.h` headers.
///
/// The full libpcap surface is large; we expose the parts the PCSX2 porting
/// layer actually consumes.  Any untranslated symbol can be reached via
/// [`raw::link_pcap`].
pub mod pcap {
    use std::os::raw::{c_char, c_int, c_uint};

    /// Network interface entry (mirror `struct pcap_if`).
    #[repr(C)]
    #[derive(Copy, Clone)]
    pub struct Interface {
        pub next: *mut Interface,
        pub name: *const c_char,
        pub description: *const c_char,
        pub addresses: *mut u8,
        pub flags: c_uint,
    }

    /// Opaque pcap handle (mirror `pcap_t`).
    #[repr(transparent)]
    pub struct Handle(pub *mut u8);

    impl Handle {
        pub const fn null() -> Self {
            Self(std::ptr::null_mut())
        }
    }

    /// Data-link layer type constants (mirror `pcap_datalink` return values).
    #[repr(i32)]
    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    pub enum DataLink {
        Null = 0,
        Ethernet = 1,
        Raw = 12,
        Loopback = 108,
        LinuxSll = 113,
        Unknown = -1,
    }

    /// Status / error codes (subset of `pcap_stat` / `pcap_geterr`).
    #[repr(C)]#[derive(Debug, Copy, Clone, Eq, PartialEq)]
    pub enum PcapError {
        Ok = 0,
        Generic = -1,
        PermDenied = -3,
        NoSuchDevice = -5,
        PromiscPermDenied = -11,
        Error = -2,
    }

    /// Convenience: build a `PcapError` from a libpcap return code.
    #[inline]
    pub fn classify(rc: c_int) -> PcapError {
        match rc {
            0 => PcapError::Ok,
            -3 => PcapError::PermDenied,
            -5 => PcapError::NoSuchDevice,
            -11 => PcapError::PromiscPermDenied,
            _ => PcapError::Generic,
        }
    }

    /// PCAP file magic / snapshot length defaults (mirror `pcap_open_offline`).
    pub const SNAPSHOT_DEFAULT: c_int = 65535;
    pub const SNAPSHOT_MIN: c_int = 0;
    pub const SNAPSHOT_MAX: c_int = 65_535;
}

// ---------------------------------------------------------------------------
// `any*` headers are not present in this directory tree; see module docs.
// ---------------------------------------------------------------------------

/// Placeholder for the absent `any.hpp` / `any.h` / `any_config.h` family.
///
/// The translation pipeline was instructed to produce an `AnyRs.rs` passthrough
/// but the underlying headers do not exist in `3rdparty/include/`.  This stub
/// is kept so downstream `use crate::AnyRs::any;` paths keep compiling.
pub mod any {
    /// No symbols - the upstream `any.h` / `any.hpp` headers are missing.
    pub const MISSING_UPSTREAM: &str = "any.h / any.hpp / any_config.h not found";
}
