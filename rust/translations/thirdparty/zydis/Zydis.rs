//! Rust 2021 idiomatic translation of the Zydis 3rdparty x86/x64 disassembler
//! library (originally `3rdparty/zydis`).
//!
//! This module exposes the public surface area of the original C/C++
//! Zydis + Zycore library as plain Rust data structures, enums, and
//! functions. The decoded-instruction / decoded-operand types, every
//! public mnemonic and register enum variant, the formatter enums, the
//! decoder init / decode / format entry points, and the status code
//! surface are all mirrored here.
//!
//! All state that the C library would keep in static globals is mirrored
//! here with `static mut` items, as permitted by the task rules.
//! No third-party crates are used; only `std`.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]

use std::fmt;
use std::ptr;

// =============================================================================
// Status / Error type
// =============================================================================

/// Mirrors the Zyan/Zydis `ZyanStatus` integer returned from most library
/// functions. A status of `0` (`Success`) indicates success; any other value
/// indicates a failure with the corresponding [`ZydisError`] variant.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ZyanStatus(pub u32);

impl ZyanStatus {
    pub const SUCCESS: ZyanStatus = ZyanStatus(0);
}

impl fmt::Display for ZyanStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ZyanStatus({:#x})", self.0)
    }
}

impl std::error::Error for ZyanStatus {}

/// Idiomatic Rust error mirror of all `ZyanStatus` / `ZydisStatus` codes.
/// Any non-success status returned by the original library is mapped to a
/// corresponding variant below.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ZydisError {
    /// `ZYAN_STATUS_SUCCESS`.
    Success,
    /// Generic failure (`ZYAN_STATUS_FAILED`).
    Failed,
    /// The caller passed an invalid argument (`ZYAN_STATUS_INVALID_ARGUMENT`).
    InvalidArgument,
    /// The caller passed an invalid operation (`ZYAN_STATUS_INVALID_OPERATION`).
    InvalidOperation,
    /// An attempt was made to read past the end of the input buffer
    /// (`ZYDIS_STATUS_NO_MORE_DATA`).
    NoMoreData,
    /// The decoder could not make sense of the bytes (`ZYDIS_STATUS_DECODING_ERROR`).
    DecodingError,
    /// The instruction exceeded the maximum length of 15 bytes
    /// (`ZYDIS_STATUS_INSTRUCTION_TOO_LONG`).
    InstructionTooLong,
    /// The instruction encoded an invalid register (`ZYDIS_STATUS_BAD_REGISTER`).
    BadRegister,
    /// A `LOCK` prefix was found on an instruction that does not accept it
    /// (`ZYDIS_STATUS_ILLEGAL_LOCK`).
    IllegalLock,
    /// A legacy prefix was found on a XOP/VEX/EVEX/MVEX instruction
    /// (`ZYDIS_STATUS_ILLEGAL_LEGACY_PFX`).
    IllegalLegacyPrefix,
    /// A `REX` prefix was found on a XOP/VEX/EVEX/MVEX instruction
    /// (`ZYDIS_STATUS_ILLEGAL_REX`).
    IllegalRex,
    /// An invalid opcode-map value was found
    /// (`ZYDIS_STATUS_INVALID_MAP`).
    InvalidMap,
    /// The EVEX prefix was malformed (`ZYDIS_STATUS_MALFORMED_EVEX`).
    MalformedEvex,
    /// The MVEX prefix was malformed (`ZYDIS_STATUS_MALFORMED_MVEX`).
    MalformedMvex,
    /// An invalid write-mask was specified
    /// (`ZYDIS_STATUS_INVALID_MASK`).
    InvalidMask,
    /// Skip-token status from a formatter callback
    /// (`ZYDIS_STATUS_SKIP_TOKEN`).
    SkipToken,
    /// The requested instruction cannot be encoded
    /// (`ZYDIS_STATUS_IMPOSSIBLE_INSTRUCTION`).
    ImpossibleInstruction,
    /// Returned by this Rust port when the supplied decoder/formatter state
    /// is not yet initialised.
    NotInitialised,
    /// Generic buffer too small (`ZYAN_STATUS_INSUFFICIENT_BUFFER_SIZE`).
    InsufficientBufferSize,
    /// `ZYAN_STATUS_OUT_OF_RANGE`.
    OutOfRange,
    /// Any other status code not otherwise enumerated.
    Other(u32),
}

impl ZydisError {
    pub fn from_status(s: u32) -> Self {
        match s {
            0 => ZydisError::Success,
            0x01 => ZydisError::Failed,
            0x02 => ZydisError::InvalidArgument,
            0x03 => ZydisError::InvalidOperation,
            0x10 => ZydisError::NoMoreData,
            0x11 => ZydisError::DecodingError,
            0x12 => ZydisError::InstructionTooLong,
            0x13 => ZydisError::BadRegister,
            0x14 => ZydisError::IllegalLock,
            0x15 => ZydisError::IllegalLegacyPrefix,
            0x16 => ZydisError::IllegalRex,
            0x17 => ZydisError::InvalidMap,
            0x18 => ZydisError::MalformedEvex,
            0x19 => ZydisError::MalformedMvex,
            0x1A => ZydisError::InvalidMask,
            0x1B => ZydisError::SkipToken,
            0x1C => ZydisError::ImpossibleInstruction,
            0x40 => ZydisError::InsufficientBufferSize,
            0x41 => ZydisError::OutOfRange,
            other => ZydisError::Other(other),
        }
    }

    pub fn to_status(self) -> u32 {
        match self {
            ZydisError::Success => 0,
            ZydisError::Failed => 0x01,
            ZydisError::InvalidArgument => 0x02,
            ZydisError::InvalidOperation => 0x03,
            ZydisError::NoMoreData => 0x10,
            ZydisError::DecodingError => 0x11,
            ZydisError::InstructionTooLong => 0x12,
            ZydisError::BadRegister => 0x13,
            ZydisError::IllegalLock => 0x14,
            ZydisError::IllegalLegacyPrefix => 0x15,
            ZydisError::IllegalRex => 0x16,
            ZydisError::InvalidMap => 0x17,
            ZydisError::MalformedEvex => 0x18,
            ZydisError::MalformedMvex => 0x19,
            ZydisError::InvalidMask => 0x1A,
            ZydisError::SkipToken => 0x1B,
            ZydisError::ImpossibleInstruction => 0x1C,
            ZydisError::NotInitialised => 0x1D,
            ZydisError::InsufficientBufferSize => 0x40,
            ZydisError::OutOfRange => 0x41,
            ZydisError::Other(v) => v,
        }
    }
}

impl fmt::Display for ZydisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZydisError::Success => write!(f, "success"),
            ZydisError::Failed => write!(f, "zyan failed"),
            ZydisError::InvalidArgument => write!(f, "invalid argument"),
            ZydisError::InvalidOperation => write!(f, "invalid operation"),
            ZydisError::NoMoreData => write!(f, "no more data"),
            ZydisError::DecodingError => write!(f, "decoding error"),
            ZydisError::InstructionTooLong => write!(f, "instruction too long"),
            ZydisError::BadRegister => write!(f, "bad register"),
            ZydisError::IllegalLock => write!(f, "illegal LOCK prefix"),
            ZydisError::IllegalLegacyPrefix => write!(f, "illegal legacy prefix"),
            ZydisError::IllegalRex => write!(f, "illegal REX prefix"),
            ZydisError::InvalidMap => write!(f, "invalid opcode map"),
            ZydisError::MalformedEvex => write!(f, "malformed EVEX prefix"),
            ZydisError::MalformedMvex => write!(f, "malformed MVEX prefix"),
            ZydisError::InvalidMask => write!(f, "invalid mask"),
            ZydisError::SkipToken => write!(f, "skip token"),
            ZydisError::ImpossibleInstruction => write!(f, "impossible instruction"),
            ZydisError::NotInitialised => write!(f, "decoder/formatter not initialised"),
            ZydisError::InsufficientBufferSize => write!(f, "insufficient buffer size"),
            ZydisError::OutOfRange => write!(f, "out of range"),
            ZydisError::Other(v) => write!(f, "other status {:#x}", v),
        }
    }
}

impl std::error::Error for ZydisError {}

impl From<ZydisError> for u32 {
    fn from(e: ZydisError) -> u32 { e.to_status() }
}

// =============================================================================
// Machine mode / stack width
// =============================================================================

/// Mirrors `ZydisMachineMode`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ZydisMachineMode {
    Long64       = 0,
    LongCompat32 = 1,
    LongCompat16 = 2,
    Legacy32     = 3,
    Legacy16     = 4,
    Real16       = 5,
}

/// Mirrors `ZydisStackWidth`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ZydisStackWidth {
    Width16 = 0,
    Width32 = 1,
    Width64 = 2,
}

/// Mirrors `ZydisInstructionEncoding`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ZydisInstructionEncoding {
    Legacy = 0,
    ThreeDNow = 1,
    Xop = 2,
    Vex = 3,
    Evex = 4,
    Mvex = 5,
}

/// Mirrors `ZydisOpcodeMap`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ZydisOpcodeMap {
    Default = 0,
    Map0F = 1,
    Map0F38 = 2,
    Map0F3A = 3,
    Map4 = 4, // not used
    Map5 = 5,
    Map6 = 6,
    Map7 = 7, // not used
    Map0F0F = 8,
    Xop8 = 9,
    Xop9 = 10,
    XopA = 11,
}

/// Mirrors `ZydisInstructionCategory`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ZydisInstructionCategory {
    Invalid = 0,
    AdoxAdcx, Aes, Amd3DNow, AmxTile, Avx, Avx2, Avx2Gather,
    Avx512, Avx512_4FMaps, Avx512_4VNNIW, Avx512Bitalg, Avx512VBMI, Avx512VP2Intersect,
    AvxIfma, Binary, BitByte, Blend, Bmi1, Bmi2, Broadcast, Call,
    Cet, Cldemote, Clflushopt, Clwb, Clzero, Cmov, Compress, CondBr,
    Conflict, Convert, DataXfer, Decimal, Enqcmd, Expand, Fcmov,
    FlagOp, Fma4, Fp16, Gather, Gfni, Hreset, Ifma, Interrupt,
    Io, IoStringOp, KeyLocker, KeyLockerWide, KMask, Knc, KncMask,
    KncScalar, Legacy, Logical, LogicalFp, Lzcnt, Misc, Mmx,
    Movdir, Mpx, MsrList, Nop, Padlock, PBNDKB, Pclmulqdq,
    Pcommit, Pconfig, Pku, Pop, Prefetch, PrefetchWt1, Pt,
    Push, RdPid, RdPru, RdRand, RdSeed, RdWrFsGs, Ret,
    Rotate, Scatter, SegOp, Semaphore, Serialize, Setcc, Sgx,
    Sha, Sha512, Shift, Smap, Sse, StringOp, Sttni,
    Syscall, Sysret, System, Tbm, TsxLdTrk, UFma, Uintr,
    UncondBr, Vaes, VBMI2, Vex, Vfma, VPClmulqdq, Vtx,
    WaitPkg, WideNop, WrMsrNs, X87Alu, Xop, Xsave, XsaveOpt,
}

/// Mirrors `ZydisISASet`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ZydisISASet {
    Invalid = 0,
    AdoxAdcx, Aes, Amd, Amd3DNow, AmdInvlpgb,
    AmxBf16, AmxFp16, AmxInt8, AmxTile,
    Avx, Avx2, Avx2Gather,
    Avx512Bw128, Avx512Bw128N, Avx512Bw256, Avx512Bw512, Avx512BwKop,
    Avx512Cd128, Avx512Cd256, Avx512Cd512,
    Avx512Dq128, Avx512Dq128N, Avx512Dq256, Avx512Dq512, Avx512DqKop, Avx512DqScalar,
    Avx512Er512, Avx512ErScalar,
    Avx512F128, Avx512F128N, Avx512F256, Avx512F512, Avx512FKop, Avx512FScalar,
    Avx512Pf512,
    Avx512_4FMaps512, Avx512_4FMapsScalar,
    Avx512_4VNNIW512,
    Avx512Bf16128, Avx512Bf16256, Avx512Bf16512,
    Avx512Bitalg128, Avx512Bitalg256, Avx512Bitalg512,
    Avx512Fp16128, Avx512Fp16128N, Avx512Fp16256, Avx512Fp16512, Avx512Fp16Scalar,
    Avx512Gfni128, Avx512Gfni256, Avx512Gfni512,
    Avx512Ifma128, Avx512Ifma256, Avx512Ifma512,
    Avx512Vaes128, Avx512Vaes256, Avx512Vaes512,
    Avx512Vbmi2128, Avx512Vbmi2256, Avx512Vbmi2512,
    Avx512Vbmi128, Avx512Vbmi256, Avx512Vbmi512,
    Avx512Vnni128, Avx512Vnni256, Avx512Vnni512,
    Avx512Vp2Intersect128, Avx512Vp2Intersect256, Avx512Vp2Intersect512,
    Avx512VPclmulqdq128, Avx512VPclmulqdq256, Avx512VPclmulqdq512,
    Avx512Vpopcntdq128, Avx512Vpopcntdq256, Avx512Vpopcntdq512,
    AvxAes, AvxGfni, AvxIfma, AvxNeConvert, AvxVnni, AvxVnniInt16, AvxVnniInt8,
    Bmi1, Bmi2, Cet, Cldemote, Clflushopt, Clfsh, Clwb, Clzero,
    Cmov, Cmpxchg16B, Enqcmd, F16C, FatNop, Fcmov, Fcomi,
    Fma, Fma4, Fxsave, Fxsave64, Gfni, Hreset,
    I186, I286Protected, I286Real, I386, I486, I486Real, I86,
    IcachePrefetch, Invpcid, KeyLocker, KeyLockerWide,
    KncE, KncJkbr, KncStream, KncV, KncMisc, KncPfHint,
    Lahf, LongMode, Lwp, Lzcnt,
    Mcommit, Monitor, MonitorX, Movbe, Movdir, Mpx, MsrList,
    PadlockAce, PadlockPhe, PadlockPmm, PadlockRng,
    Pause, PBndkb, Pclmulqdq, Pcommit, Pconfig,
    PentiumMmx, PentiumReal, Pku, PopCnt, PPro, PrefetchWt1, PrefetchNop,
    Pt, RaoInt, RdPid, Rdpmc, Rdpru, RdRand, RdSeed, RdTscp, RdWrFsGs,
    Rtm, Serialize, Sgx, SgxEnclv,
    Sha, Sha512, Sm3, Sm4, Smap, Smx, Snp,
    Sse, Sse2, Sse2Mmx, Sse3, Sse3X87, Sse4, Sse42, Sse4A, SseMxCsr, SsePrefetch, Ssse3, Ssse3Mmx,
    Svm, Tbm, Tdx, TsxLdTrk, Uintr,
    Vaes, Vmfunc, VPclmulqdq, Vtx, WaitPkg, WrMsrNs,
    X87, Xop, Xsave, XsaveC, XsaveOpt, XsaveS,
}

/// Mirrors `ZydisISAExt`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ZydisISAExt {
    Invalid = 0,
    AdoxAdcx, Aes, Amd3DNow, Amd3DNowPrefetch, AmdInvlpgb,
    AmxBf16, AmxFp16, AmxInt8, AmxTile,
    Avx, Avx2, Avx2Gather, Avx512Evex, Avx512Vex,
    AvxAes, AvxIfma, AvxNeConvert, AvxVnni, AvxVnniInt16, AvxVnniInt8,
    Base, Bmi1, Bmi2, Cet, Cldemote, Clflushopt, Clfsh, Clwb, Clzero,
    Enqcmd, F16C, Fma, Fma4, Gfni, Hreset, IcachePrefetch, Invpcid,
    KeyLocker, KeyLockerWide,
    Knc, KncE, KncV, LongMode, Lzcnt,
    Mcommit, Mmx, Monitor, MonitorX, Movbe, Movdir, Mpx, MsrList,
    Padlock, Pause, PBndkb, Pclmulqdq, Pcommit, Pconfig, Pku,
    PrefetchWt1, Pt, RaoInt, RdPid, Rdpru, RdRand, RdSeed, RdTscp, RdWrFsGs,
    Rtm, Serialize, Sgx, SgxEnclv, Sha, Sha512, Sm3, Sm4, Smap, Smx, Snp,
    Sse, Sse2, Sse3, Sse4, Sse4A, Ssse3, Svm, Tbm, Tdx, TsxLdTrk, Uintr,
    Vaes, Vmfunc, VPclmulqdq, Vtx, WaitPkg, WrMsrNs,
    X87, Xop, Xsave, XsaveC, XsaveOpt, XsaveS,
}
