//! Rust 2021 idiomatic translation of the public surface of the
//! [`cpuinfo`](https://github.com/pytorch/cpuinfo) third-party C library.
//!
//! The original C library spans many translation units
//! (`api.c`, `init.c`, `x86/{isa.c,init.c,vendor.c,info.c,uarch.c,...}` and
//! `arm/linux/{aarch32-isa.c,aarch64-isa.c,...}`) and exposes a per-OS
//! `cpuinfo_initialize()` plus a large set of `cpuinfo_get_*` and
//! `cpuinfo_has_*` accessor functions.  This module condenses that surface
//! down to the parts that the rest of the project actually touches:
//!
//! * [`CpuInfo`] and [`CpuInfoProcess`] — public per-logical-processor and
//!   per-process structs mirroring the C `struct cpuinfo_processor` and the
//!   aggregated [`CpuInfo`] view.
//! * [`cpuinfo_init`] — initializes the module once and is idempotent.
//! * [`cpuinfo_get_processor`] / [`cpuinfo_get_processors`] — indexing into
//!   the cached processor table.
//! * `is_x86` / `is_arm` plus the targeted ISA predicates
//!   (`has_sse`, `has_avx`, `has_avx2`, `has_avx512`, `has_fma`,
//!    `has_popcnt`, `has_bmi1`, `has_bmi2`, `has_lzcnt`, `has_aes`,
//!    `has_sha`, `has_neon`, `has_sha1`, `has_sha2`, `has_crc32`,
//!    `has_fp16`).
//!
//! Implementation notes:
//! * ISA detection on x86/x86-64 uses the standard `__cpuid` / `__cpuid_count`
//!   intrinsics together with the `xgetbv` opcode to gate AVX/AVX-512 on the
//!   OS-enabled extended register state — exactly the same algorithm the C
//!   code in `src/x86/isa.c` performs.
//! * ISA detection on aarch64 reads `getauxval(AT_HWCAP|AT_HWCAP2)` through
//!   the Linux auxiliary vector, mirroring the Linux branch in
//!   `src/arm/linux/{aarch32,aarch64}-isa.c`.
//! * When the host is neither x86/x86-64 nor aarch64 (e.g. wasm), the
//!   corresponding predicates simply return `false`.
//! * Globals are kept in `static mut` items, as required by the task rules.

use std::sync::Once;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Vendor of the processor design.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum CpuVendor {
    Unknown = 0,
    Intel = 1,
    Amd = 2,
    Arm = 3,
    Qualcomm = 4,
    Apple = 5,
    Samsung = 6,
    Nvidia = 7,
    Via = 11,
    Cavium = 12,
    Broadcom = 13,
    Apm = 14,
    Huawei = 15,
    Hygon = 16,
    SiFive = 17,
}

/// Per-logical-processor view populated by [`cpuinfo_init`].
///
/// This is the Rust analogue of `struct cpuinfo_processor` from the original
/// C library, but flattened to the fields the rest of the codebase reads.
#[derive(Debug, Clone)]
pub struct CpuInfo {
    /// Vendor of the processor design (Intel, AMD, Apple, ...).
    pub vendor: CpuVendor,
    /// Human-readable brand string (e.g. `"AMD Ryzen 9 7950X 16-Core Processor"`).
    /// Up to 63 bytes plus a NUL terminator, matching `CPUINFO_PACKAGE_NAME_MAX`.
    pub brand: String,
    /// Effective CPU family (`base_family + extended_family`).
    pub family: u32,
    /// Effective CPU model (`base_model + (extended_model << 4)`).
    pub model: u32,
    /// Stepping (revision) field.
    pub stepping: u32,
    /// Aggregated ISA/feature flags.  See [`CpuFlags`] for the individual bits.
    pub flags: CpuFlags,
}

/// Per-process aggregate exposed for callers that only want a single value
/// describing the entire process (logical-processor count, etc.).
#[derive(Debug, Copy, Clone)]
pub struct CpuInfoProcess {
    /// Number of logical processors visible to the process.
    pub processor_count: u32,
    /// True when the process runs on an x86 or x86-64 host.
    pub is_x86: bool,
    /// True when the process runs on an ARM (32-bit) or AArch64 host.
    pub is_arm: bool,
}

/// Bit-set of ISA/feature flags detected at init time.
///
/// The bit positions are chosen so that the most common x86 SSE/AVX groups
/// live in `flags[0]` and the AVX-512 family lives in `flags[1]`, matching
/// roughly the layout of `struct cpuinfo_x86_isa` from the C library.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct CpuFlags(pub [u64; 4]);

impl CpuFlags {
    // x86/x86-64 ISA bits
    pub const SSE: u64 = 1 << 0;
    pub const SSE2: u64 = 1 << 1;
    pub const SSE3: u64 = 1 << 2;
    pub const SSSE3: u64 = 1 << 3;
    pub const SSE4_1: u64 = 1 << 4;
    pub const SSE4_2: u64 = 1 << 5;
    pub const AVX: u64 = 1 << 6;
    pub const AVX2: u64 = 1 << 7;
    pub const FMA: u64 = 1 << 8;
    pub const POPCNT: u64 = 1 << 9;
    pub const BMI1: u64 = 1 << 10;
    pub const BMI2: u64 = 1 << 11;
    pub const LZCNT: u64 = 1 << 12;
    pub const AES: u64 = 1 << 13;
    pub const SHA: u64 = 1 << 14;
    // AVX-512 family (in the second word)
    pub const AVX512F: u64 = 1 << 0;
    pub const AVX512BW: u64 = 1 << 1;
    pub const AVX512CD: u64 = 1 << 2;
    pub const AVX512DQ: u64 = 1 << 3;
    pub const AVX512VL: u64 = 1 << 4;
    // ARM / AArch64 bits
    pub const NEON: u64 = 1 << 16;
    pub const SHA1: u64 = 1 << 17;
    pub const SHA2: u64 = 1 << 18;
    pub const CRC32: u64 = 1 << 19;
    pub const FP16: u64 = 1 << 20;

    /// Returns `true` if the given bit (`flag`) is set.
    #[inline]
    pub fn has(&self, flag: u64) -> bool {
        if flag < 64 {
            (self.0[0] & flag) != 0
        } else if flag < 128 {
            (self.0[1] & (flag >> 64)) != 0
        } else if flag < 192 {
            (self.0[2] & (flag >> 128)) != 0
        } else {
            (self.0[3] & (flag >> 192)) != 0
        }
    }
}

/// Error returned by [`cpuinfo_init`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CpuInfoError {
    /// The platform is not supported (neither x86/x86-64 nor aarch64).
    UnsupportedPlatform,
    /// The init code was unable to allocate or read the auxiliary data it
    /// needs (e.g. CPUID failed on a hypervisor that denies it).
    DetectionFailed,
}

// ---------------------------------------------------------------------------
// Globals
// ---------------------------------------------------------------------------

static mut PROCESSORS: Vec<CpuInfo> = Vec::new();
static mut PROCESS_INFO: CpuInfoProcess = CpuInfoProcess {
    processor_count: 0,
    is_x86: false,
    is_arm: false,
};
static mut INITIALIZED: bool = false;
static INIT_ONCE: Once = Once::new();

// ---------------------------------------------------------------------------
// Architecture detection
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const HOST_IS_X86: bool = true;
#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
const HOST_IS_X86: bool = false;

#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
const HOST_IS_ARM: bool = true;
#[cfg(not(any(target_arch = "arm", target_arch = "aarch64")))]
const HOST_IS_ARM: bool = false;

/// Returns `true` when compiling for an x86 or x86-64 target.
#[inline]
pub fn is_x86() -> bool {
    HOST_IS_X86
}

/// Returns `true` when compiling for an ARM or AArch64 target.
#[inline]
pub fn is_arm() -> bool {
    HOST_IS_ARM
}

// ---------------------------------------------------------------------------
// x86 ISA predicates
// ---------------------------------------------------------------------------

#[inline]
pub fn has_sse() -> bool {
    flag(CpuFlags::SSE)
}
#[inline]
pub fn has_sse2() -> bool {
    flag(CpuFlags::SSE2)
}
#[inline]
pub fn has_sse3() -> bool {
    flag(CpuFlags::SSE3)
}
#[inline]
pub fn has_sse4_1() -> bool {
    flag(CpuFlags::SSE4_1)
}
#[inline]
pub fn has_sse4_2() -> bool {
    flag(CpuFlags::SSE4_2)
}
#[inline]
pub fn has_avx() -> bool {
    flag(CpuFlags::AVX)
}
#[inline]
pub fn has_avx2() -> bool {
    flag(CpuFlags::AVX2)
}
#[inline]
pub fn has_avx512() -> bool {
    flag(CpuFlags::AVX512F)
}
#[inline]
pub fn has_fma() -> bool {
    flag(CpuFlags::FMA)
}
#[inline]
pub fn has_popcnt() -> bool {
    flag(CpuFlags::POPCNT)
}
#[inline]
pub fn has_bmi1() -> bool {
    flag(CpuFlags::BMI1)
}
#[inline]
pub fn has_bmi2() -> bool {
    flag(CpuFlags::BMI2)
}
#[inline]
pub fn has_lzcnt() -> bool {
    flag(CpuFlags::LZCNT)
}
#[inline]
pub fn has_aes() -> bool {
    flag(CpuFlags::AES)
}
#[inline]
pub fn has_sha() -> bool {
    flag(CpuFlags::SHA)
}

// ---------------------------------------------------------------------------
// ARM ISA predicates
// ---------------------------------------------------------------------------

#[inline]
pub fn has_neon() -> bool {
    flag(CpuFlags::NEON)
}
#[inline]
pub fn has_sha1() -> bool {
    flag(CpuFlags::SHA1)
}
#[inline]
pub fn has_sha2() -> bool {
    flag(CpuFlags::SHA2)
}
#[inline]
pub fn has_crc32() -> bool {
    flag(CpuFlags::CRC32)
}
#[inline]
pub fn has_fp16() -> bool {
    flag(CpuFlags::FP16)
}

#[inline]
fn flag(mask: u64) -> bool {
    // SAFETY: the only mutator is `cpuinfo_init` behind a `Once`; readers
    // racing with the first init call see either the default empty state
    // (all flags false) or the populated state.
    unsafe {
        if !INITIALIZED {
            return false;
        }
        PROCESSORS
            .first()
            .map(|c| c.flags.has(mask))
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the module.  Safe to call multiple times — only the first call
/// performs detection work.
pub fn cpuinfo_init() -> Result<(), CpuInfoError> {
    let mut result: Result<(), CpuInfoError> = Ok(());
    INIT_ONCE.call_once(|| {
        let r = detect();
        // SAFETY: `INIT_ONCE` guarantees this closure runs at most once.
        unsafe {
            if let Ok((processors, info)) = r {
                PROCESSORS = processors;
                PROCESS_INFO = info;
                INITIALIZED = true;
            } else {
                INITIALIZED = false;
                result = Err(CpuInfoError::DetectionFailed);
            }
        }
    });
    result
}

/// Returns the logical-processor at `index`, or `None` if `index` is out of
/// range (mirrors `cpuinfo_get_processor`, which returns `NULL` for an
/// out-of-range index).
pub fn cpuinfo_get_processor(index: usize) -> Option<&'static CpuInfo> {
    // SAFETY: `PROCESSORS` is only mutated by `cpuinfo_init` behind a `Once`,
    // and `INIT_ONCE` has returned at this point.
    unsafe {
        if !INITIALIZED {
            None
        } else {
            PROCESSORS.get(index)
        }
    }
}

/// Returns the entire cached processor table.
pub fn cpuinfo_get_processors() -> &'static [CpuInfo] {
    // SAFETY: see `cpuinfo_get_processor`.
    unsafe {
        if INITIALIZED {
            &PROCESSORS[..]
        } else {
            &[]
        }
    }
}

/// Per-process aggregate information (logical-processor count, host ISA).
pub fn cpuinfo_get_process_info() -> CpuInfoProcess {
    // SAFETY: see `cpuinfo_get_processor`.
    unsafe {
        if INITIALIZED {
            PROCESS_INFO
        } else {
            CpuInfoProcess {
                processor_count: 0,
                is_x86: HOST_IS_X86,
                is_arm: HOST_IS_ARM,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detect() -> Result<(Vec<CpuInfo>, CpuInfoProcess), ()> {
    let leaf0 = cpuid(0);
    let max_base = leaf0.eax;
    let vendor = decode_vendor(leaf0.ebx, leaf0.ecx, leaf0.edx);

    let leaf_ext0 = cpuid(0x8000_0000);
    let max_ext = if leaf_ext0.eax >= 0x8000_0000 {
        leaf_ext0.eax
    } else {
        0
    };

    let leaf1 = if max_base >= 1 { cpuid(1) } else { Cpuid { eax: 0, ebx: 0, ecx: 0, edx: 0 } };
    let leaf0x8000_0001 = if max_ext >= 0x8000_0001 {
        cpuid(0x8000_0001)
    } else {
        Cpuid { eax: 0, ebx: 0, ecx: 0, edx: 0 }
    };

    let model = decode_model_info(leaf1.eax);
    let avx_regs = os_avx_enabled(leaf1.ecx);
    let avx512_regs = os_avx512_enabled(leaf1.ecx);
    let leaf7 = if max_base >= 7 { cpuid_count(7, 0) } else { Cpuid { eax: 0, ebx: 0, ecx: 0, edx: 0 } };
    let leaf7_1 = if max_base >= 7 { cpuid_count(7, 1) } else { Cpuid { eax: 0, ebx: 0, ecx: 0, edx: 0 } };

    let flags = detect_x86_flags(leaf1, leaf0x8000_0001, leaf7, leaf7_1, avx_regs, avx512_regs);
    let brand = detect_brand(max_ext);

    let info = CpuInfo {
        vendor,
        brand,
        family: model.family,
        model: model.model,
        stepping: model.stepping,
        flags,
    };

    let processor_count = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    let proc = CpuInfoProcess {
        processor_count,
        is_x86: true,
        is_arm: false,
    };
    Ok((vec![info], proc))
}

#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
fn detect() -> Result<(Vec<CpuInfo>, CpuInfoProcess), ()> {
    use std::io::Read;

    let (hwcap, hwcap2) = read_hwcap();
    let vendor = detect_arm_vendor();
    let flags = detect_arm_flags(hwcap, hwcap2);
    let brand = detect_arm_brand();

    let info = CpuInfo {
        vendor,
        brand,
        family: 0,
        model: 0,
        stepping: 0,
        flags,
    };

    let processor_count = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let proc = CpuInfoProcess {
        processor_count,
        is_x86: false,
        is_arm: true,
    };
    Ok((vec![info], proc))
}

#[cfg(not(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "arm",
    target_arch = "aarch64"
)))]
fn detect() -> Result<(Vec<CpuInfo>, CpuInfoProcess), ()> {
    let proc = CpuInfoProcess {
        processor_count: 1,
        is_x86: false,
        is_arm: false,
    };
    Ok((Vec::new(), proc))
}

// ---------------------------------------------------------------------------
// x86 helpers
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[derive(Copy, Clone, Default)]
struct Cpuid {
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[derive(Copy, Clone, Default)]
struct ModelInfo {
    family: u32,
    model: u32,
    stepping: u32,
    base_family: u32,
    base_model: u32,
    extended_family: u32,
    extended_model: u32,
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
fn cpuid(eax: u32) -> Cpuid {
    // SAFETY: the `cpuid` instruction is always available on the targets
    // guarded above; it does not touch memory and has no side effects beyond
    // filling the four output registers.
    unsafe {
        let r = core::arch::x86_64::__cpuid(eax);
        Cpuid { eax: r.eax, ebx: r.ebx, ecx: r.ecx, edx: r.edx }
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
fn cpuid_count(eax: u32, ecx: u32) -> Cpuid {
    // SAFETY: see `cpuid`.  `__cpuid_count` is supported by rustc on both
    // x86 and x86_64 targets.
    unsafe {
        let r = core::arch::x86_64::__cpuid_count(eax, ecx);
        Cpuid { eax: r.eax, ebx: r.ebx, ecx: r.ecx, edx: r.edx }
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
fn xgetbv(index: u32) -> u64 {
    // SAFETY: `xgetbv` is a non-faulting, side-effect-free instruction as
    // long as `index` is a valid extended control register index (0 is the
    // only one supported by user-mode code).
    unsafe { core::arch::x86_64::_xgetbv(index) }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn os_avx_enabled(leaf1_ecx: u32) -> bool {
    // XSAVE & OSXSAVE bits must both be set for AVX state to be usable.
    const MASK: u32 = 0x0C00_0000;
    (leaf1_ecx & MASK) == MASK && os_xcr0_avx_enabled()
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn os_xcr0_avx_enabled() -> bool {
    let xcr0 = xgetbv(0);
    // bits 1 (SSE) and 2 (AVX) of XCR0 must be set.
    (xcr0 & 0b110) == 0b110
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn os_avx512_enabled(leaf1_ecx: u32) -> bool {
    // XSAVE + OSXSAVE required, plus bits 1, 2, 5, 6, 7 of XCR0 set.
    const MASK: u32 = 0x0C00_0000;
    if (leaf1_ecx & MASK) != MASK {
        return false;
    }
    let xcr0 = xgetbv(0);
    const AVX512_MASK: u64 = 0xE6;
    (xcr0 & AVX512_MASK) == AVX512_MASK
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn decode_model_info(eax: u32) -> ModelInfo {
    let stepping = eax & 0xF;
    let base_model = (eax >> 4) & 0xF;
    let base_family = (eax >> 8) & 0xF;
    let processor_type = (eax >> 12) & 0x3;
    let extended_model = (eax >> 16) & 0xF;
    let extended_family = (eax >> 20) & 0xFF;

    ModelInfo {
        family: base_family + extended_family,
        model: base_model + (extended_model << 4),
        stepping,
        base_family,
        base_model,
        extended_family,
        extended_model,
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn decode_vendor(ebx: u32, ecx: u32, edx: u32) -> CpuVendor {
    // GenuineIntel: EBX=0x756E6547, ECX=0x6C65746E, EDX=0x49656E69
    if ebx == 0x756E_6547 && ecx == 0x6C65_746E && edx == 0x4965_6E69 {
        return CpuVendor::Intel;
    }
    // AuthenticAMD: EBX=0x68747541, ECX=0x444D4163, EDX=0x69746E65
    if ebx == 0x6874_7541 && ecx == 0x444D_4163 && edx == 0x6974_6E65 {
        return CpuVendor::Amd;
    }
    // CentaurHauls / VIA VIA VIA
    if ebx == 0x746E_6543 && ecx == 0x736C_7561 && edx == 0x4872_7561 {
        return CpuVendor::Via;
    }
    // HygonGenuine: EBX=0x6F677948, ECX=0x656E6975, EDX=0x6E65476E
    if ebx == 0x6F67_7948 && ecx == 0x656E_6975 && edx == 0x6E65_476E {
        return CpuVendor::Hygon;
    }
    CpuVendor::Unknown
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detect_x86_flags(
    leaf1: Cpuid,
    leaf0x8000_0001: Cpuid,
    leaf7: Cpuid,
    leaf7_1: Cpuid,
    avx_regs: bool,
    avx512_regs: bool,
) -> CpuFlags {
    let mut flags = [0u64; 4];

    // SSE / SSE2 (mandatory on x86_64; gated by CPUID on x86).
    #[cfg(target_arch = "x86_64")]
    {
        flags[0] |= CpuFlags::SSE | CpuFlags::SSE2;
    }
    if leaf1.edx & (1 << 25) != 0 {
        flags[0] |= CpuFlags::SSE;
    }
    if leaf1.edx & (1 << 26) != 0 {
        flags[0] |= CpuFlags::SSE2;
    }
    if leaf1.ecx & (1 << 0) != 0 {
        flags[0] |= CpuFlags::SSE3;
    }
    if leaf1.ecx & (1 << 9) != 0 {
        flags[0] |= CpuFlags::SSSE3;
    }
    if leaf1.ecx & (1 << 19) != 0 {
        flags[0] |= CpuFlags::SSE4_1;
    }
    if leaf1.ecx & (1 << 20) != 0 {
        flags[0] |= CpuFlags::SSE4_2;
    }
    // AVX / AVX2 / FMA — gated on the OS enabling XSAVE state.
    if avx_regs && (leaf1.ecx & (1 << 28)) != 0 {
        flags[0] |= CpuFlags::AVX;
    }
    if avx_regs && (leaf1.ecx & (1 << 12)) != 0 {
        flags[0] |= CpuFlags::FMA;
    }
    if avx_regs && (leaf7.ebx & (1 << 5)) != 0 {
        flags[0] |= CpuFlags::AVX2;
    }
    // POPCNT / LZCNT
    if leaf1.ecx & (1 << 23) != 0 {
        flags[0] |= CpuFlags::POPCNT;
    }
    if leaf0x8000_0001.ecx & (1 << 5) != 0 {
        flags[0] |= CpuFlags::LZCNT;
    }
    // BMI1 / BMI2
    if leaf7.ebx & (1 << 3) != 0 {
        flags[0] |= CpuFlags::BMI1;
    }
    if leaf7.ebx & (1 << 8) != 0 {
        flags[0] |= CpuFlags::BMI2;
    }
    // AES / SHA
    if leaf1.ecx & (1 << 25) != 0 {
        flags[0] |= CpuFlags::AES;
    }
    if leaf7.ebx & (1 << 29) != 0 {
        flags[0] |= CpuFlags::SHA;
    }
    // AVX-512 Foundation (covers the AVX-512 family)
    if avx512_regs && (leaf7.ebx & (1 << 16)) != 0 {
        flags[1] |= CpuFlags::AVX512F;
    }
    if avx512_regs && (leaf7.ebx & (1 << 17)) != 0 {
        flags[1] |= CpuFlags::AVX512DQ;
    }
    if avx512_regs && (leaf7.ebx & (1 << 28)) != 0 {
        flags[1] |= CpuFlags::AVX512CD;
    }
    if avx512_regs && (leaf7.ebx & (1 << 30)) != 0 {
        flags[1] |= CpuFlags::AVX512BW;
    }
    if avx512_regs && (leaf7.ebx & (1 << 31)) != 0 {
        flags[1] |= CpuFlags::AVX512VL;
    }
    // AVX-VNNI is reported in leaf 7 sub-leaf 1 on recent CPUs.
    if avx_regs && (leaf7_1.eax & (1 << 4)) != 0 {
        // Mark AVX-VNNI alongside AVX so it stays visible in the same word.
        // (Original C library carries this in a separate field of
        //  struct cpuinfo_x86_isa; the bit-set here is sufficient for the
        //  has_*() queries in this module.)
        flags[0] |= CpuFlags::AVX;
    }

    let _ = leaf1; // keep the parameter warning-free if all the uses above go away.
    let _ = leaf0x8000_0001;
    CpuFlags(flags)
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detect_brand(max_ext: u32) -> String {
    if max_ext < 0x8000_0004 {
        return String::new();
    }
    let mut buf = [0u8; 48];
    for i in 0..3u32 {
        let r = cpuid(0x8000_0002 + i);
        let regs = [
            r.eax.to_le_bytes(),
            r.ebx.to_le_bytes(),
            r.ecx.to_le_bytes(),
            r.edx.to_le_bytes(),
        ];
        let offset = (i as usize) * 16;
        for (j, chunk) in regs.iter().enumerate() {
            buf[offset + j * 4..offset + j * 4 + 4].copy_from_slice(chunk);
        }
    }
    // Trim at the first NUL, mirroring C's %48s printing.
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..len]).trim().to_string()
}

// ---------------------------------------------------------------------------
// ARM helpers
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
fn read_hwcap() -> (u64, u64) {
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        let mut buf = [0u8; 4096];
        let mut file = match std::fs::File::open("/proc/self/auxv") {
            Ok(f) => f,
            Err(_) => return (0, 0),
        };
        if file.read(&mut buf).is_err() {
            return (0, 0);
        }
        parse_auxv(&buf)
    }
    #[cfg(not(target_os = "linux"))]
    {
        (0, 0)
    }
}

#[cfg(all(any(target_arch = "arm", target_arch = "aarch64"), target_os = "linux"))]
fn parse_auxv(buf: &[u8]) -> (u64, u64) {
    const AT_HWCAP: u64 = 16;
    const AT_HWCAP2: u64 = 26;
    let mut hwcap = 0u64;
    let mut hwcap2 = 0u64;
    let mut i = 0;
    while i + 16 <= buf.len() {
        let key = u64::from_ne_bytes(buf[i..i + 8].try_into().unwrap_or([0u8; 8]));
        let val = u64::from_ne_bytes(buf[i + 8..i + 16].try_into().unwrap_or([0u8; 8]));
        if key == 0 {
            break;
        }
        match key {
            AT_HWCAP => hwcap = val,
            AT_HWCAP2 => hwcap2 = val,
            _ => {}
        }
        i += 16;
    }
    (hwcap, hwcap2)
}

#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
fn detect_arm_vendor() -> CpuVendor {
    #[cfg(target_os = "macos")]
    {
        // Apple Silicon is the only macOS arm64 we expect to see.
        return CpuVendor::Apple;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let midr = read_midr();
        let implementer = (midr >> 24) & 0xFF;
        let part = (midr >> 4) & 0xFFF;
        match implementer {
            0x41 => CpuVendor::Arm,
            0x51 => CpuVendor::Qualcomm,
            0x53 => CpuVendor::Qualcomm,
            0x42 | 0x43 => CpuVendor::Broadcom,
            0x4D => {
                // 0x4D = Freescale (now NXP), grouped with Broadcom.
                let _ = part;
                CpuVendor::Broadcom
            }
            0x48 => CpuVendor::Huawei,
            0x49 => {
                // 0x49 = Infineon -> TriCore, not relevant here.
                let _ = part;
                CpuVendor::Unknown
            }
            _ => CpuVendor::Unknown,
        }
    }
}

#[cfg(all(any(target_arch = "arm", target_arch = "aarch64"), not(target_os = "macos")))]
fn read_midr() -> u32 {
    // MIDR is exposed at the same path on Linux: /proc/cpuinfo.
    let s = match std::fs::read_to_string("/proc/cpuinfo") {
        Ok(s) => s,
        Err(_) => return 0,
    };
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("CPU implementer") {
            // Look for the next line that contains "CPU part".
            // Simple state machine would be overkill; parse the file twice
            // because cpuinfo on Linux is small.
            let _ = rest;
        }
    }
    // Fall back to a linear scan that re-reads the file.
    let s = match std::fs::read_to_string("/proc/cpuinfo") {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let mut implementer: Option<u32> = None;
    let mut part: Option<u32> = None;
    for line in s.lines() {
        if let Some(rest) = line.split(':').nth(1) {
            let value = rest.trim();
            if let Some(suffix) = line.strip_prefix("CPU implementer") {
                let _ = suffix;
                implementer = u32::from_str_radix(value, 16).ok();
            } else if line.starts_with("CPU part") {
                part = u32::from_str_radix(value, 16).ok();
            }
        }
    }
    match (implementer, part) {
        (Some(impl_), Some(part)) => (impl_ << 24) | (part << 4),
        _ => 0,
    }
}

#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
fn detect_arm_flags(hwcap: u64, hwcap2: u64) -> CpuFlags {
    let mut flags = [0u64; 4];

    // HWCAP / HWCAP2 bit positions follow the Linux definitions from
    // <asm/hwcap.h>.  The values used here match those found on
    // aarch64 (and arm where applicable).
    #[cfg(target_arch = "aarch64")]
    const HWCAP_ASIMD: u64 = 1 << 1; // NEON/ASIMD
    #[cfg(target_arch = "aarch64")]
    const HWCAP_AES: u64 = 1 << 3;
    #[cfg(target_arch = "aarch64")]
    const HWCAP_SHA1: u64 = 1 << 4;
    #[cfg(target_arch = "aarch64")]
    const HWCAP_SHA2: u64 = 1 << 6;
    #[cfg(target_arch = "aarch64")]
    const HWCAP_CRC32: u64 = 1 << 7;
    #[cfg(target_arch = "aarch64")]
    const HWCAP_FPHP: u64 = 1 << 9;
    #[cfg(target_arch = "aarch64")]
    const HWCAP_ASIMDHP: u64 = 1 << 10;

    #[cfg(target_arch = "arm")]
    const HWCAP_NEON: u64 = 1 << 12;
    #[cfg(target_arch = "arm")]
    const HWCAP_AES: u64 = 1 << 28;
    #[cfg(target_arch = "arm")]
    const HWCAP_SHA1: u64 = 1 << 29;
    #[cfg(target_arch = "arm")]
    const HWCAP_SHA2: u64 = 1 << 30;
    #[cfg(target_arch = "arm")]
    const HWCAP_CRC32: u64 = 1 << 31;

    #[cfg(target_arch = "aarch64")]
    {
        if hwcap & HWCAP_ASIMD != 0 {
            flags[0] |= CpuFlags::NEON;
        }
        if hwcap & HWCAP_AES != 0 {
            flags[0] |= CpuFlags::AES;
        }
        if hwcap & HWCAP_SHA1 != 0 {
            flags[0] |= CpuFlags::SHA1;
        }
        if hwcap & HWCAP_SHA2 != 0 {
            flags[0] |= CpuFlags::SHA2;
        }
        if hwcap & HWCAP_CRC32 != 0 {
            flags[0] |= CpuFlags::CRC32;
        }
        if (hwcap & (HWCAP_FPHP | HWCAP_ASIMDHP)) == (HWCAP_FPHP | HWCAP_ASIMDHP) {
            flags[0] |= CpuFlags::FP16;
        }
    }
    #[cfg(target_arch = "arm")]
    {
        if hwcap & HWCAP_NEON != 0 {
            flags[0] |= CpuFlags::NEON;
        }
        if hwcap & HWCAP_AES != 0 {
            flags[0] |= CpuFlags::AES;
        }
        if hwcap & HWCAP_SHA1 != 0 {
            flags[0] |= CpuFlags::SHA1;
        }
        if hwcap & HWCAP_SHA2 != 0 {
            flags[0] |= CpuFlags::SHA2;
        }
        if hwcap & HWCAP_CRC32 != 0 {
            flags[0] |= CpuFlags::CRC32;
        }
        let _ = hwcap2;
    }

    let _ = hwcap;
    CpuFlags(flags)
}

#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
fn detect_arm_brand() -> String {
    #[cfg(target_os = "macos")]
    {
        // Use `sysctlbyname` equivalent via std::process for simplicity.
        // On Apple Silicon, `sysctl -n machdep.cpu.brand_string` returns
        // the marketing name (e.g. "Apple M2 Pro").
        let out = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output();
        if let Ok(o) = out {
            if o.status.success() {
                return String::from_utf8_lossy(&o.stdout).trim().to_string();
            }
        }
        return String::from("Apple");
    }
    #[cfg(not(target_os = "macos"))]
    {
        let s = match std::fs::read_to_string("/proc/cpuinfo") {
            Ok(s) => s,
            Err(_) => return String::new(),
        };
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("Hardware\t: ") {
                return rest.trim().to_string();
            }
            if let Some(rest) = line.strip_prefix("model name\t: ") {
                return rest.trim().to_string();
            }
            if let Some(rest) = line.strip_prefix("Processor\t: ") {
                return rest.trim().to_string();
            }
        }
        String::new()
    }
}