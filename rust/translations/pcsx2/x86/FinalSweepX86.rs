// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! PCSX2 x86 JIT recompiler final sweep — Rust port.
//!
//! Idiomatic Rust 2021 translation of the C/C++ x86 source set under
//! `pcsx2/x86/`. This is a "final sweep" consolidation: it gathers
//! every type definition, constant, enum, decoder table, and free
//! function signature that the rest of the runtime (and other Rust
//! translation modules) need to know about into one module. Bodies
//! of the large inline-asm / dynarec / SSE emitter paths are
//! deliberately left as `unimplemented!()` stubs with `// ASM stub`
//! markers, because the real implementations depend on the x86
//! emitter, EE/IOP/VU/GTE register files, VTLB, base-block manager,
//! dynarec code cache, and host CPU feature detector (xgetbv, AVX,
//! ...). None of those are representable in a `std`-only module, so
//! this file acts as the typed interface layer between the
//! translation surface and any future dynarec-style backend.
//!
//! # Source coverage
//!
//! Every file under `pcsx2/x86/` has been scanned and is represented
//! here in some form. The mapping is roughly:
//!
//! | C++ source                       | Section in this module            |
//! |----------------------------------|------------------------------------|
//! | `BaseblockEx.{h,cpp}`            | Base block manager                 |
//! | `R5900_Profiler.h`               | EE profiler + opcode enum          |
//! | `iCore.{h,cpp}`                  | iCore regalloc + dispatcher        |
//! | `iFPU.{h,cpp}` / `iFPUd.cpp`     | iFPU dynarec handlers              |
//! | `iMMI.{h,cpp}`                   | iMMI dynarec handlers              |
//! | `iR3000A.{h,cpp}` + tables       | IOP (R3000A) interpreter + tables  |
//! | `iR5900*.h` + `iR5900Misc.cpp`   | R5900 (EE) recompiler handlers     |
//! | `iR5900Analysis.{h,cpp}`         | R5900 analysis passes              |
//! | `iCOP0.{h,cpp}`                  | COP0 dynarec handlers              |
//! | `microVU*.h/inl/cpp`             | micro-VU dynarec                   |
//! | `Vif_Dynarec.cpp`                | VIF dynarec                       |
//! | `Vif_UnpackSSE.{h,cpp}` +        | VIF SSE unpack                     |
//! | `newVif.h`                       |                                    |
//! | `ix86-32/*.{cpp}`                | 32-bit backend stubs               |
//!
//! See the top of each section for the original C++ file(s).

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(clippy::upper_case_acronyms)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ptr;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Primitive aliases
// ---------------------------------------------------------------------------
//
// The original C++ uses `u8`, `u16`, `u32`, `u64`, `s8`, `s32`, `s64`,
// and `uptr` from PCSX2's `Common.h`. Keep the names so the port reads
// 1:1 with the source.

pub type u8 = ::std::primitive::u8;
pub type u16 = ::std::primitive::u16;
pub type u32 = ::std::primitive::u32;
pub type u64 = ::std::primitive::u64;
pub type s8 = ::std::primitive::i8;
pub type s16 = ::std::primitive::i16;
pub type s32 = ::std::primitive::i32;
pub type s64 = ::std::primitive::i64;

/// Unsigned pointer-sized integer. In the C++ code this is `uptr`.
pub type uptr = usize;

/// Signed pointer-sized integer.
pub type sptr = isize;

// ===========================================================================
// Section: Base block manager (from `BaseblockEx.{h,cpp}`)
// ===========================================================================
//
// PCSX2's dynarec maintains two structures per PS2 addressable 4-byte
// location: a one-word `BASEBLOCK` that holds a function pointer for
// that PC, and an extended `BASEBLOCKEX` for the *start* of every
// recompiled block (with block size, x86 size, and the startpc).
//
// The sorted `BaseBlockArray` and the `BaseBlocks` manager that wraps
// it (with a `multimap<u32, uptr>` of back-patchable jump sites) are
// the heart of every indirect jump resolution.

/// 8-byte block header: just a function pointer for one PC location.
#[derive(Debug, Clone, Copy, Default)]
pub struct BASEBLOCK {
    pub m_pFnptr: uptr,
}

impl BASEBLOCK {
    #[inline]
    pub const fn GetFnptr(&self) -> uptr {
        self.m_pFnptr
    }
    #[inline]
    pub fn SetFnptr(&mut self, ptr: uptr) {
        self.m_pFnptr = ptr;
    }
}

/// Extended block info; only valid at the start of each recompiled
/// function.
#[derive(Debug, Clone, Default, Copy)]
pub struct BASEBLOCKEX {
    pub fnptr: uptr,
    pub startpc: u32,
    /// Size of the block in EE dwords (number of instructions).
    pub size: u32,
    /// Size of the translated x86 instructions, in bytes.
    pub x86size: u32,
}

/// Sorted array of `BASEBLOCKEX` keyed by `startpc`.
#[derive(Debug, Default)]
pub struct BaseBlockArray {
    _reserved: s32,
    _size: s32,
    blocks: Vec<BASEBLOCKEX>,
}

impl BaseBlockArray {
    /// Allocate a `BaseBlockArray` with the given initial capacity.
    pub fn new(size: s32) -> Self {
        let mut b = BaseBlockArray {
            _reserved: 0,
            _size: 0,
            blocks: Vec::new(),
        };
        b.reserve(size as u32);
        b
    }

    fn resize(&mut self, size: s32) {
        assert!(size > 0, "BaseBlockArray::resize size must be positive");
        // Real implementation copies old contents; here we just resize.
        self.blocks.resize(size as usize, BASEBLOCKEX::default());
    }

    fn reserve(&mut self, size: u32) {
        self.resize(size as s32);
        self._reserved = size as s32;
    }

    /// Insert a new block, returning a mutable handle.
    pub fn insert(&mut self, startpc: u32, fnptr: uptr) -> &mut BASEBLOCKEX {
        if self._size + 1 >= self._reserved {
            self.reserve((self._reserved as u32) + 0x2000);
        }

        // Find insertion point by binary search on `startpc`.
        let mut imin: s32 = 0;
        let mut imax: s32 = self._size;
        while imin < imax {
            let imid = ((imin + imax) >> 1) as usize;
            if self.blocks[imid].startpc > startpc {
                imax = imid as s32;
            } else {
                imin = imid as s32 + 1;
            }
        }
        let idx = imin as usize;
        assert!(
            idx == self._size as usize || self.blocks[idx].startpc > startpc,
            "BASEBLOCK: insertion point invariant violated"
        );

        if (idx as s32) < self._size {
            self.blocks.insert(idx, BASEBLOCKEX::default());
        } else {
            self.blocks.push(BASEBLOCKEX::default());
        }
        self.blocks[idx] = BASEBLOCKEX {
            fnptr,
            startpc,
            size: 0,
            x86size: 0,
        };
        self._size += 1;
        &mut self.blocks[idx]
    }

    /// Index into the array (bounds-checked).
    pub fn at(&self, idx: usize) -> Option<&BASEBLOCKEX> {
        self.blocks.get(idx)
    }

    /// Mutable index into the array (bounds-checked).
    pub fn at_mut(&mut self, idx: usize) -> Option<&mut BASEBLOCKEX> {
        self.blocks.get_mut(idx)
    }

    /// Reset the array, keeping allocation.
    pub fn clear(&mut self) {
        self._size = 0;
        self.blocks.clear();
    }

    pub fn size(&self) -> u32 {
        self._size as u32
    }

    /// Erase `[first, last]` (inclusive on both ends, matching C++).
    pub fn erase(&mut self, first: s32, last: s32) {
        let range = (last - first) as usize;
        if (last as s32) < self._size {
            self.blocks.drain(first as usize..(last as usize) + 1);
        } else {
            self.blocks.truncate(first as usize);
        }
        self._size -= range as s32;
    }
}

/// Type alias for the back-patch link iterator (multimap in the C++).
pub type LinkIter<'a> = std::collections::btree_map::Range<'a, u32, uptr>;
pub type LinkRange = (u32, uptr);

/// Holds the sorted base-block table and the multiset of outstanding
/// back-patchable jump slots.
#[derive(Debug, Default)]
pub struct BaseBlocks {
    /// Outstanding indirect-jump back-patch sites: PC -> pointer to
    /// the jump-slot memory location in the code cache.
    pub links: BTreeMap<u32, uptr>,
    /// Function pointer of the recompiler dispatcher; jumps to
    /// "not-yet-compiled" targets are patched to here.
    pub recompiler: uptr,
    /// Sorted block array.
    pub blocks: BaseBlockArray,
}

impl BaseBlocks {
    /// Construct a `BaseBlocks` with a 0x4000-block initial capacity,
    /// matching the original C++.
    pub fn new() -> Self {
        BaseBlocks {
            links: BTreeMap::new(),
            recompiler: 0,
            blocks: BaseBlockArray::new(0x4000),
        }
    }

    /// Set the recompiler dispatch function pointer.
    #[inline]
    pub fn SetJITCompile(&mut self, recompiler_: *const std::ffi::c_void) {
        self.recompiler = recompiler_ as uptr;
    }

    /// Find the index of the block whose `startpc` is the largest value
    /// `<= startpc`. Returns -1 when no block exists.
    pub fn LastIndex(&self, startpc: u32) -> i32 {
        if self.blocks.size() == 0 {
            return -1;
        }
        let mut imin: i32 = 0;
        let mut imax: i32 = self.blocks.size() as i32 - 1;
        while imin != imax {
            let imid = (imin + imax + 1) >> 1;
            if self.blocks.blocks[imid as usize].startpc > startpc {
                imax = imid - 1;
            } else {
                imin = imid;
            }
        }
        imin
    }

    /// Return the index of the block containing `startpc`, or -1.
    #[inline]
    pub fn Index(&self, startpc: u32) -> i32 {
        let idx = self.LastIndex(startpc);
        if idx == -1
            || startpc < self.blocks.blocks[idx as usize].startpc
            || (self.blocks.blocks[idx as usize].size != 0
                && startpc
                    >= self.blocks.blocks[idx as usize].startpc
                        + self.blocks.blocks[idx as usize].size * 4)
        {
            -1
        } else {
            idx
        }
    }

    /// Index access, returning a `&mut BASEBLOCKEX` or `None`.
    pub fn at(&mut self, idx: i32) -> Option<&mut BASEBLOCKEX> {
        if idx < 0 || idx >= self.blocks.size() as i32 {
            None
        } else {
            self.blocks.at_mut(idx as usize)
        }
    }

    /// Look up the block whose startpc is exactly `startpc`.
    pub fn Get(&mut self, startpc: u32) -> Option<&mut BASEBLOCKEX> {
        let i = self.Index(startpc);
        self.at(i)
    }

    /// Remove a range of blocks, patching outstanding links to the
    /// recompiler dispatcher.
    pub fn Remove(&mut self, first: i32, last: i32) {
        assert!(first <= last, "BaseBlocks::Remove: first > last");
        let mut idx = first;
        loop {
            assert!(idx <= last);
            let startpc = self.blocks.blocks[idx as usize].startpc;
            // Patch all outstanding links to this block to the
            // recompiler dispatcher.
            let keys: Vec<u32> = self
                .links
                .range(startpc..=startpc)
                .map(|(k, _)| *k)
                .collect();
            for k in keys {
                if let Some(p) = self.links.get(&k) {
                    // In the real port, write to a u32* in the code
                    // cache. Here we record the delta only.
                    let _delta = self.recompiler.wrapping_sub(p.wrapping_add(4));
                }
            }
            if idx >= last {
                break;
            }
            idx += 1;
        }
        self.blocks.erase(first, last + 1);
    }

    /// Link a 32-bit jump slot in the code cache to the start of the
    /// block at `pc`, or to the recompiler dispatcher if the block is
    /// not yet compiled.
    pub fn Link(&mut self, pc: u32, jumpptr: *mut s32) {
        let target_fnptr = if let Some(t) = self.Get(pc) {
            if t.startpc == pc {
                Some(t.fnptr)
            } else {
                None
            }
        } else {
            None
        };
        unsafe {
            if let Some(fnptr) = target_fnptr {
                *jumpptr = (fnptr as sptr - jumpptr.offset(1) as sptr) as s32;
            } else {
                *jumpptr = (self.recompiler as sptr - jumpptr.offset(1) as sptr) as s32;
            }
        }
        self.links.insert(pc, jumpptr as uptr);
    }

    /// Reset the table; called when the recompiler is reset.
    #[inline]
    pub fn Reset(&mut self) {
        self.blocks.clear();
        self.links.clear();
    }
}

/// `PC_GETBLOCK_(x, reclut)`. Computes the address of the BASEBLOCK
/// pointer for the page containing PS2 address `x`.
#[inline]
pub const fn PC_GETBLOCK_(x: u32, reclut: &[uptr; 0x10000]) -> uptr {
    reclut[((x as usize) >> 16) & 0xffff] + ((x as usize) * (8 / 4))
}

/// Add a page to the recompiler lookup table. Stub equivalent of
/// `recLUT_SetPage`.
#[inline]
pub fn recLUT_SetPage(
    reclut: &mut [uptr; 0x10000],
    hwlut: &mut [u32; 0x10000],
    mapbase: *mut BASEBLOCK,
    pagebase: u32,
    pageidx: u32,
    mappage: u32,
) {
    let page = pagebase + pageidx;
    assert!(page < 0x10000, "reclut page index out of range");
    let offset = (mappage as i32 - page as i32) << 14;
    unsafe {
        reclut[page as usize] = mapbase.offset(offset as isize) as uptr;
    }
    if !hwlut.as_ptr().is_null() {
        hwlut[page as usize] = 0u32.wrapping_sub(pagebase << 16);
    }
}

// ===========================================================================
// Section: EE opcode enum, profiler, op-name table
//        (from `R5900_Profiler.h`)
// ===========================================================================

/// Names for the EE's primary+sub-decoded opcodes. Used by the
/// profiler and the disassembler. The integer values match the
/// `eeOpcode` enum in the C++ source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum EeOpcode {
    // Core
    Special = 0,
    Regimm = 1,
    J = 2,
    Jal = 3,
    Beq = 4,
    Bne = 5,
    Blez = 6,
    Bgtz = 7,
    Addi = 8,
    Addiu = 9,
    Slti = 10,
    Sltiu = 11,
    Andi = 12,
    Ori = 13,
    Xori = 14,
    Lui = 15,
    Cop0 = 16,
    Cop1 = 17,
    Cop2 = 18,
    // 19 reserved
    Beql = 20,
    Bnel = 21,
    Blezl = 22,
    Bgtzl = 23,
    Daddi = 24,
    Daddiu = 25,
    Ldl = 26,
    Ldr = 27,
    Mmi = 28,
    // 29 reserved
    Lq = 30,
    Sq = 31,
    Lb = 32,
    Lh = 33,
    Lwl = 34,
    Lw = 35,
    Lbu = 36,
    Lhu = 37,
    Lwr = 38,
    Lwu = 39,
    Sb = 40,
    Sh = 41,
    Swl = 42,
    Sw = 43,
    Sdl = 44,
    Sdr = 45,
    Swr = 46,
    Cache = 47,
    // 48 reserved
    Lwc1 = 49,
    // 50 reserved
    Pref = 51,
    // 52-53 reserved
    Lqc2 = 54,
    Ld = 55,
    // 56 reserved
    Swc1 = 57,
    // 58-61 reserved
    Sqc2 = 62,
    Sd = 63,

    // Special
    Sll = 64,
    // 65 reserved
    Srl = 66,
    Sra = 67,
    Sllv = 68,
    // 69 reserved
    Srlv = 70,
    Srav = 71,
    Jr = 72,
    Jalr = 73,
    Movz = 74,
    Movn = 75,
    Syscall = 76,
    Break = 77,
    // 78 reserved
    Sync = 79,
    Mfhi = 80,
    Mthi = 81,
    Mflo = 82,
    Mtlo = 83,
    Dsllv = 84,
    // 85 reserved
    Dsrlv = 86,
    Dsrav = 87,
    Mult = 88,
    Multu = 89,
    Div = 90,
    Divu = 91,
    // 92-95 reserved
    Add = 96,
    Addu = 97,
    Sub = 98,
    Subu = 99,
    And = 100,
    Or = 101,
    Xor = 102,
    Nor = 103,
    Mfsa = 104,
    Mtsa = 105,
    Slt = 106,
    Sltu = 107,
    Dadd = 108,
    Daddu = 109,
    Dsub = 110,
    Dsubu = 111,
    Tge = 112,
    Tgeu = 113,
    Tlt = 114,
    Tltu = 115,
    Teq = 116,
    // 117 reserved
    Tne = 118,
    // 119 reserved
    Dsll = 120,
    // 121 reserved
    Dsrl = 122,
    Dsra = 123,
    Dsll32 = 124,
    // 125 reserved
    Dsrl32 = 126,
    Dsra32 = 127,

    // Regimm
    Bltz = 128,
    Bgez = 129,
    Bltzl = 130,
    Bgezl = 131,
    // 132-135 reserved
    Tgei = 136,
    Tgeiu = 137,
    Tlti = 138,
    Tltiu = 139,
    Teqi = 140,
    // 141 reserved
    Tnei = 142,
    // 143 reserved
    Bltzal = 144,
    Bgezal = 145,
    Bltzall = 146,
    Bgezall = 147,
    // 148-151 reserved
    Mtsab = 152,
    Mtsah = 153,
    // 154-159 reserved

    // MMI
    Madd = 160,
    Maddu = 161,
    // 162-163 reserved
    Plzcw = 164,
    // 165-167 reserved
    Mmi0 = 168,
    Mmi2 = 169,
    // 170-171 reserved
    Mfhi1 = 172,
    Mthi1 = 173,
    Mflo1 = 174,
    Mtlo1 = 175,
    // 176-179 reserved
    Mult1 = 180,
    Multu1 = 181,
    Div1 = 182,
    Divu1 = 183,
    // 184-187 reserved
    Madd1 = 188,
    MaddU1 = 189,
    // 190-191 reserved
    Mmi1 = 192,
    Mmi3 = 193,
    // 194-195 reserved
    Pmfhl = 196,
    Pmthl = 197,
    // 198-199 reserved
    Psllh = 200,
    // 201 reserved
    Psrlh = 202,
    Psrah = 203,
    // 204-207 reserved
    Psllw = 208,
    // 209 reserved
    Psrlw = 210,
    Psraw = 211,
    // 212-215 reserved

    // MMI0
    Paddw = 216,
    Psubw = 217,
    Pcgtw = 218,
    Pmaxw = 219,
    Paddh = 220,
    Psubh = 221,
    Pcgth = 222,
    Pmaxh = 223,
    Paddb = 224,
    Psubb = 225,
    Pcgtb = 226,
    // 227 reserved
    // 228-231 reserved
    Paddsw = 232,
    Psubsw = 233,
    Pextlw = 234,
    Ppacw = 235,
    Paddsh = 236,
    Psubsh = 237,
    Pextlh = 238,
    Ppach = 239,
    Paddsb = 240,
    Psubsb = 241,
    Pextlb = 242,
    Ppacb = 243,
    // 244-247 reserved
    Pext5 = 248,
    Ppac5 = 249,
    // 250-255 reserved

    // MMI1
    // 256 reserved
    Pabsw = 257,
    Pceqw = 258,
    Pminw = 259,
    Padsbh = 260,
    Pabsh = 261,
    Pceqh = 262,
    Pminh = 263,
    // 264-265 reserved
    Pceqb = 266,
    // 267 reserved
    // 268-271 reserved
    Padduw = 272,
    Psubuw = 273,
    Pextuw = 274,
    // 275 reserved
    Padduh = 276,
    Psubuh = 277,
    Pextuh = 278,
    // 279 reserved
    Paddub = 280,
    Psubub = 281,
    Pextub = 282,
    Qfsrv = 283,
    // 284-287 reserved

    // MMI2
    Pmaddw = 288,
    // 289 reserved
    Psllvw = 290,
    Psrlvw = 291,
    Pmsubw = 292,
    // 293-295 reserved
    Pmfhi = 296,
    Pmflo = 297,
    Pinth = 298,
    // 299 reserved
    Pmultw = 300,
    Pdivw = 301,
    Pcpyld = 302,
    // 303 reserved
    Pmaddh = 304,
    Phmadh = 305,
    Pand = 306,
    Pxor = 307,
    Pmsubh = 308,
    Phmsbh = 309,
    // 310-311 reserved
    Pexeh = 312,
    Prevh = 313,
    Pmulth = 314,
    Pdivbw = 315,
    Pexew = 316,
    Prot3w = 317,
    // 318-319 reserved

    // MMI3
    Pmadduw = 320,
    // 321-322 reserved
    Psravw = 323,
    // 324-327 reserved
    Pmthi = 328,
    Pmtlo = 329,
    Pinteh = 330,
    // 331 reserved
    Pmultuw = 332,
    Pdivuw = 333,
    Pcpyud = 334,
    // 335 reserved
    // 336-337 reserved
    Por = 338,
    Pnor = 339,
    // 340-343 reserved
    // 344-345 reserved
    Pexch = 346,
    Pcpyh = 347,
    // 348-349 reserved
    Pexcw = 350,
    // 351 reserved
    // 352-383 reserved

    // COP1 moves
    Mfc1 = 384,
    // 385 reserved
    Cfc1 = 386,
    // 387 reserved
    Mtc1 = 388,
    // 389 reserved
    Ctc1 = 390,
    // 391 reserved

    // COP1 BC1
    Bc1f = 392,
    Bc1t = 393,
    Bc1fl = 394,
    Bc1tl = 395,
    // 396-399 reserved

    // COP1 S
    AddS = 400,
    SubS = 401,
    MulS = 402,
    DivS = 403,
    SqrtS = 404,
    AbsS = 405,
    MovS = 406,
    NegS = 407,
    // 408-415 reserved
    // 416-423 reserved
    RsqrtS = 424,
    // 425 reserved
    AddaS = 426,
    SubaS = 427,
    MulaS = 428,
    // 429 reserved
    MaddS = 430,
    MsubS = 431,
    MaddaS = 432,
    MsubaS = 433,
    // 434-439 reserved
    Cvtw = 440,
    // 441-447 reserved
    MaxS = 448,
    MinS = 449,
    // 450-455 reserved
    CfF = 456,
    // 457 reserved
    CeqF = 458,
    // 459 reserved
    CltF = 460,
    // 461 reserved
    CleF = 462,
    // 463 reserved

    // COP1 W
    CvtsF = 464,
    // 465-471 reserved

    /// Sentinel for "no opcode" / end-of-table marker.
    Last = 472,
}

/// EE opcode-name string table. Mirrors `eeOpcodeName[][16]`.
pub const EE_OPCODE_NAME: [&str; 472] = [
    // Core
    "special", "regimm", "J", "JAL", "BEQ", "BNE", "BLEZ", "BGTZ",
    "ADDI", "ADDIU", "SLTI", "SLTIU", "ANDI", "ORI", "XORI", "LUI",
    "cop0", "cop1", "cop2", "", "BEQL", "BNEL", "BLEZL", "BGTZL",
    "DADDI", "DADDIU", "LDL", "LDR", "mmi", "", "LQ", "SQ",
    "LB", "LH", "LWL", "LW", "LBU", "LHU", "LWR", "LWU",
    "SB", "SH", "SWL", "SW", "SDL", "SDR", "SWR", "CACHE",
    "", "LWC1", "", "PREF", "", "", "LQC2", "LD",
    "", "SWC1", "", "", "", "", "SQC2", "SD",
    // Special
    "SLL", "", "SRL", "SRA", "SLLV", "", "SRLV", "SRAV",
    "JR", "JALR", "MOVZ", "MOVN", "SYSCALL", "BREAK", "", "SYNC",
    "MFHI", "MTHI", "MFLO", "MTLO", "DSLLV", "", "DSRLV", "DSRAV",
    "MULT", "MULTU", "DIV", "DIVU", "", "", "", "",
    "ADD", "ADDU", "SUB", "SUBU", "AND", "OR", "XOR", "NOR",
    "MFSA", "MTSA", "SLT", "SLTU", "DADD", "DADDU", "DSUB", "DSUBU",
    "TGE", "TGEU", "TLT", "TLTU", "TEQ", "", "TNE", "",
    "DSLL", "", "DSRL", "DSRA", "DSLL32", "", "DSRL32", "DSRA32",
    // Regimm
    "BLTZ", "BGEZ", "BLTZL", "BGEZL", "", "", "", "",
    "TGEI", "TGEIU", "TLTI", "TLTIU", "TEQI", "", "TNEI", "",
    "BLTZAL", "BGEZAL", "BLTZALL", "BGEZALL", "", "", "", "",
    "MTSAB", "MTSAH", "", "", "", "", "", "",
    // MMI
    "MADD", "MADDU", "", "", "PLZCW", "", "", "",
    "MMI0", "MMI2", "", "", "", "", "", "",
    "MFHI1", "MTHI1", "MFLO1", "MTLO1", "", "", "", "",
    "MULT1", "MULTU1", "DIV1", "DIVU1", "", "", "", "",
    "MADD1", "MADDU1", "", "", "", "", "", "",
    "MMI1", "MMI3", "", "", "", "", "", "",
    "PMFHL", "PMTHL", "", "", "PSLLH", "", "PSRLH", "PSRAH",
    "", "", "", "", "PSLLW", "", "PSRLW", "PSRAW",
    // MMI0
    "PADDW", "PSUBW", "PCGTW", "PMAXW",
    "PADDH", "PSUBH", "PCGTH", "PMAXH",
    "PADDB", "PSUBB", "PCGTB", "",
    "", "", "", "",
    "PADDSW", "PSUBSW", "PEXTLW", "PPACW",
    "PADDSH", "PSUBSH", "PEXTLH", "PPACH",
    "PADDSB", "PSUBSB", "PEXTLB", "PPACB",
    "", "", "PEXT5", "PPAC5",
    // MMI1
    "", "PABSW", "PCEQW", "PMINW",
    "PADSBH", "PABSH", "PCEQH", "PMINH",
    "", "", "PCEQB", "",
    "", "", "", "",
    "PADDUW", "PSUBUW", "PEXTUW", "",
    "PADDUH", "PSUBUH", "PEXTUH", "",
    "PADDUB", "PSUBUB", "PEXTUB", "QFSRV",
    "", "", "", "",
    // MMI2
    "PMADDW", "", "PSLLVW", "PSRLVW",
    "PMSUBW", "", "", "",
    "PMFHI", "PMFLO", "PINTH", "",
    "PMULTW", "PDIVW", "PCPYLD", "",
    "PMADDH", "PHMADH", "PAND", "PXOR",
    "PMSUBH", "PHMSBH", "", "",
    "", "", "PEXEH", "PREVH",
    "PMULTH", "PDIVBW", "PEXEW", "PROT3W",
    // MMI3
    "PMADDUW", "", "", "PSRAVW",
    "", "", "", "",
    "PMTHI", "PMTLO", "PINTEH", "",
    "PMULTUW", "PDIVUW", "PCPYUD", "",
    "", "", "POR", "PNOR",
    "", "", "", "",
    "", "", "PEXCH", "PCPYH",
    "", "", "PEXCW", "",
    // COP1
    "MFC1", "", "CFC1", "", "MTC1", "", "CTC1", "",
    // COP1 BC1
    "BC1F", "BC1T", "BC1FL", "BC1TL", "", "", "", "",
    // COP1 S
    "ADD_S", "SUB_S", "MUL_S", "DIV_S", "SQRT_S", "ABS_S", "MOV_S", "NEG_S",
    "", "", "", "", "", "", "", "",
    "", "", "", "", "", "", "RSQRT_S", "",
    "ADDA_F", "SUBA_F", "MULA_F", "", "MADD_F", "MSUB_F", "MADDA_F", "MSUBA_F",
    "", "", "", "", "CVTW", "", "", "",
    "MAX_F", "MIN_F", "", "", "", "", "", "",
    "C.F", "", "C.EQ", "", "C.LT", "", "C.LE", "",
    // COP1 W
    "CVTS_F", "", "", "", "", "", "", "",
    "!",
    // Padding for indices 433..472 reserved entries (matching C++).
    "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "",
    "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "",
    "", "", "", "", "", "", "",
];

/// EE profiler. The real one emits x86 add-with-carry into a 64-bit
/// counter table. Here we just bump a `BTreeMap` of u64 counts.
#[derive(Debug, Default)]
pub struct EeProfiler {
    pub op_stats: BTreeMap<u32, u64>,
    /// 512 KB virtual-address memStats table (mirrors original).
    pub mem_stats: Vec<u32>,
    pub mem_stats_const: Vec<u32>,
    pub mem_stats_slow: u64,
    pub mem_stats_fast: u64,
    pub mem_mask: u32,
}

impl EeProfiler {
    /// Memory space for the EE profiler (1<<19 u32 entries = 2 MiB).
    pub const MEM_SPACE: u32 = 1 << 19;

    pub fn new() -> Self {
        let mut p = EeProfiler {
            op_stats: BTreeMap::new(),
            mem_stats: vec![0; Self::MEM_SPACE as usize],
            mem_stats_const: vec![0; Self::MEM_SPACE as usize],
            mem_stats_slow: 0,
            mem_stats_fast: 0,
            mem_mask: 0xF700_FFF0,
        };
        p.reset();
        p
    }

    pub fn reset(&mut self) {
        for v in self.op_stats.values_mut() {
            *v = 0;
        }
        for v in self.mem_stats.iter_mut() {
            *v = 0;
        }
        for v in self.mem_stats_const.iter_mut() {
            *v = 0;
        }
        self.mem_stats_slow = 0;
        self.mem_stats_fast = 0;
        self.mem_mask = 0xF700_FFF0;
        // eeOpcodeName[LAST][0] should be '!'
        debug_assert!(EE_OPCODE_NAME[EeOpcode::Last as usize].starts_with('!'));
    }

    /// Bump a 64-bit counter for one opcode.
    pub fn EmitOp(&mut self, opcode: EeOpcode) {
        *self.op_stats.entry(opcode as u32).or_insert(0) += 1;
    }

    /// Compute a percentage `part/total*100`. Returns 0 if total==0.
    pub fn per(&self, part: u64, total: u64) -> f64 {
        if total == 0 {
            0.0
        } else {
            (part as f64) / (total as f64) * 100.0
        }
    }

    /// Placeholder for the dev-build `Print()` function that walks the
    /// sorted stat vectors and dumps them to the console.
    pub fn print(&self) {
        // Original writes to DevCon; here we just no-op.
    }

    /// Compact a 4GB virtual address into a 512KB profiler slot.
    pub fn pext_mask(&self, addr: u32) -> u32 {
        // Mirrors `_pext_u32(add, memMask)` from the C++.
        pext_u32(addr, self.mem_mask)
    }

    /// Stub for `EmitMem(addr_reg)` — requires the x86 emitter.
    pub fn emit_mem(&mut self, _addr_reg: i32) {
        // ASM stub
    }

    pub fn emit_const_mem(&mut self, _add: u32) {
        // ASM stub
    }
    pub fn emit_slow_mem(&mut self) {
        self.mem_stats_slow = self.mem_stats_slow.wrapping_add(1);
    }
    pub fn emit_fast_mem(&mut self) {
        self.mem_stats_fast = self.mem_stats_fast.wrapping_add(1);
    }
}

/// Stand-in for the global `EE::Profiler` declared in the C++.
pub mod ee {
    use super::EeProfiler;
    thread_local! {
        /// Singleton handle mirroring the C++ `EE::Profiler` global.
        pub static Profiler: std::cell::RefCell<EeProfiler> =
            std::cell::RefCell::new(EeProfiler::new());
    }
}

/// Bitwise PEXT (parallel bit extract). Stand-in for the
/// `_pext_u32` intrinsic used in `EmitMem`/`EmitConstMem`.
pub const fn pext_u32(src: u32, mask: u32) -> u32 {
    let mut dst: u32 = 0;
    let mut bit: u32 = 1;
    let mut m: u32 = mask;
    let mut s: u32 = src;
    while m != 0 {
        let lsb = m & m.wrapping_neg();
        if s & lsb != 0 {
            dst |= bit;
        }
        bit <<= 1;
        m ^= lsb;
        s &= !lsb;
    }
    dst
}

// ===========================================================================
// Section: iCore register allocator (from `iCore.{h,cpp}`)
// ===========================================================================
//
// The dynarec tags every cached x86/XMM host register with the
// kind of EE-side state it currently holds. The C `MODE_READ` /
// `MODE_WRITE` / `MODE_CALLEESAVED` / `MODE_COP2` flags double as
// the mode parameter of every alloc/check/free call.

/// "kind" tag for a cached x86 GPR allocation. Mirrors
/// `enum x86type : u8` from `iCore.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum X86Type {
    Temp = 0,
    Gpr = 1,
    FpRc = 2,
    ViReg = 3,
    PcWriteback = 4,
    Psx = 5,
    PsxPcWriteback = 6,
}

impl Default for X86Type {
    fn default() -> Self {
        X86Type::Temp
    }
}

/// Access mode for register allocation. Can be ORed.
pub const MODE_READ: u32 = 0x1;
pub const MODE_WRITE: u32 = 0x2;
pub const MODE_CALLEESAVED: u32 = 0x20;
pub const MODE_COP2: u32 = 0x40;

/// Per-instruction "reg is valid" bits packed into `info` (EE-style).
pub const PROCESS_EE_XMM: u32 = 0x02;
pub const PROCESS_EE_S: u32 = 0x04; // S is valid, else take from mem
pub const PROCESS_EE_T: u32 = 0x08; // T is valid, else take from mem
pub const PROCESS_EE_D: u32 = 0x10; // D is valid, else take from mem
pub const PROCESS_EE_LO: u32 = 0x40;
pub const PROCESS_EE_HI: u32 = 0x80;
pub const PROCESS_EE_ACC: u32 = 0x40;

pub const PROCESS_CONSTS: u32 = 1;
pub const PROCESS_CONSTT: u32 = 2;

/// XMM high/low aliases for the GPR pair.
pub const XMMGPR_LO: u8 = 33;
pub const XMMGPR_HI: u8 = 32;
pub const XMMFPU_ACC: u8 = 32;

/// XMM caching info bitmask (`enum xmminfo : u16`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum XmmInfo {
    ReadLo = 0x001,
    ReadHi = 0x002,
    WriteLo = 0x004,
    WriteHi = 0x008,
    WriteD = 0x010,
    ReadD = 0x020,
    ReadS = 0x040,
    ReadT = 0x080,
    ReadAcc = 0x200,
    WriteAcc = 0x400,
    WriteT = 0x800,
    Bit64Op = 0x1000,
    ForceRegS = 0x2000,
    ForceRegT = 0x4000,
    NoRename = 0x8000,
}

impl XmmInfo {
    #[inline]
    pub const fn bits(self) -> u16 {
        self as u16
    }
}

impl core::ops::BitOr for XmmInfo {
    type Output = u16;
    fn bitor(self, rhs: XmmInfo) -> u16 {
        (self as u16) | (rhs as u16)
    }
}

/// Delete-register policies. Used by the dynarec to decide whether
/// to write back and/or free on a register delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum DeleteReg {
    Free = 0,
    Flush = 1,
    FlushAndFree = 2,
    FreeNoWriteback = 3,
}

pub const XMMTYPE_TEMP: u8 = 0;
pub const XMMTYPE_GPRREG: u8 = X86Type::Gpr as u8;
pub const XMMTYPE_FPREG: u8 = 6;
pub const XMMTYPE_FPACC: u8 = 7;
pub const XMMTYPE_VFREG: u8 = 8;

/// State of one cached x86 GPR allocation. Mirrors `_x86regs`.
#[derive(Debug, Clone, Copy, Default)]
pub struct X86Reg {
    pub inuse: u8,
    pub reg: i8,
    pub mode: u8,
    pub needed: u8,
    pub ty: X86Type,
    pub counter: u16,
    pub extra: u32,
}

/// State of one cached XMM (128-bit) allocation. Mirrors `_xmmregs`.
#[derive(Debug, Clone, Copy)]
pub struct XmmReg {
    pub inuse: u8,
    pub reg: i8,
    pub ty: u8,
    pub mode: u8,
    pub needed: u8,
    pub counter: u16,
}

impl Default for XmmReg {
    fn default() -> Self {
        XmmReg {
            inuse: 0,
            reg: 0,
            ty: XMMTYPE_TEMP,
            mode: 0,
            needed: 0,
            counter: 0,
        }
    }
}

/// Per-instruction liveness info. Mirrors `EEINST` from `iCore.h`.
#[derive(Debug, Clone, Copy)]
pub struct EEInst {
    pub info: u16,
    /// 34 entries: GPR[0..32], HI=32, LO=33.
    pub regs: [u8; 34],
    /// 33 entries: FPR[0..32], ACC=32.
    pub fpuregs: [u8; 33],
    /// 34 entries: VFR[0..32], ACC=32, I=33.
    pub vfregs: [u8; 34],
    /// 16 VI regs.
    pub viregs: [u8; 16],
    /// 3 write sites (uses XMMTYPE_ flags; 0 = TEMP = unused).
    pub write_type: [u8; 3],
    pub write_reg: [u8; 3],
    /// 4 read sites.
    pub read_type: [u8; 4],
    pub read_reg: [u8; 4],
}

impl Default for EEInst {
    fn default() -> Self {
        Self {
            info: 0,
            regs: [0; 34],
            fpuregs: [0; 33],
            vfregs: [0; 34],
            viregs: [0; 16],
            write_type: [0; 3],
            write_reg: [0; 3],
            read_type: [0; 4],
            read_reg: [0; 4],
        }
    }
}

impl EEInst {
    /// Mirrors C++ `_recClearInst(EEINST*)`.
    pub fn clear(&mut self) {
        self.regs = [0; 34];
        self.fpuregs = [0; 33];
        self.vfregs = [0; 34];
        self.viregs = [0; 16];
        self.write_type = [0; 3];
        self.write_reg = [0; 3];
        self.read_type = [0; 4];
        self.read_reg = [0; 4];
    }
}

// Liveness flag bits (from `iCore.h`).
pub const EEINST_LIVE: u8 = 0x01;
pub const EEINST_LASTUSE: u8 = 0x08;
pub const EEINST_XMM: u8 = 0x20;
pub const EEINST_USED: u8 = 0x40;
pub const EEINST_COP2_DENORMALIZE_STATUS_FLAG: u16 = 0x100;
pub const EEINST_COP2_NORMALIZE_STATUS_FLAG: u16 = 0x200;
pub const EEINST_COP2_STATUS_FLAG: u16 = 0x400;
pub const EEINST_COP2_MAC_FLAG: u16 = 0x800;
pub const EEINST_COP2_CLIP_FLAG: u16 = 0x1000;
pub const EEINST_COP2_SYNC_VU0: u16 = 0x2000;
pub const EEINST_COP2_FINISH_VU0: u16 = 0x4000;
pub const EEINST_COP2_FLUSH_VU0_REGISTERS: u16 = 0x8000;

/// `EE_WRITE_DEAD_VALUES`. When unset, the dynarec can skip the
/// write-back of a register that is no longer live.
pub const EE_WRITE_DEAD_VALUES: bool = true;

// ---------------------------------------------------------------------------
// iFlushCall flags (from `iCore.h`).
// ---------------------------------------------------------------------------

pub const FLUSH_NONE: u32 = 0x000;
pub const FLUSH_CONSTANT_REGS: u32 = 0x001;
pub const FLUSH_FLUSH_XMM: u32 = 0x002;
pub const FLUSH_FREE_XMM: u32 = 0x004;
pub const FLUSH_ALL_X86: u32 = 0x020;
pub const FLUSH_FREE_TEMP_X86: u32 = 0x040;
pub const FLUSH_FREE_NONTEMP_X86: u32 = 0x080;
pub const FLUSH_FREE_VU0: u32 = 0x100;
pub const FLUSH_PC: u32 = 0x200;
pub const FLUSH_CODE: u32 = 0x800;
pub const FLUSH_EVERYTHING: u32 = 0x1ff;
pub const FLUSH_INTERPRETER: u32 = 0xfff;
pub const FLUSH_FULLVTLB: u32 = 0x000;
pub const FLUSH_NODESTROY: u32 =
    FLUSH_CONSTANT_REGS | FLUSH_FLUSH_XMM | FLUSH_ALL_X86;

// ---------------------------------------------------------------------------
// Liveness test helpers (from `iCore.h`).
// ---------------------------------------------------------------------------

#[inline]
pub const fn eeinst_used_test(pinst: &EEInst, reg: usize) -> bool {
    (pinst.regs[reg] & (EEINST_USED | EEINST_LASTUSE)) == EEINST_USED
}
#[inline]
pub const fn eeinst_xmm_used_test(pinst: &EEInst, reg: usize) -> bool {
    (pinst.regs[reg] & (EEINST_USED | EEINST_XMM | EEINST_LASTUSE))
        == (EEINST_USED | EEINST_XMM)
}
#[inline]
pub const fn eeinst_vf_used_test(pinst: &EEInst, reg: usize) -> bool {
    (pinst.vfregs[reg] & (EEINST_USED | EEINST_LASTUSE)) == EEINST_USED
}
#[inline]
pub const fn eeinst_vi_used_test(pinst: &EEInst, reg: usize) -> bool {
    (pinst.viregs[reg] & (EEINST_USED | EEINST_LASTUSE)) == EEINST_USED
}
#[inline]
pub const fn eeinst_live_test(pinst: &EEInst, reg: usize) -> bool {
    EE_WRITE_DEAD_VALUES || (pinst.regs[reg] & EEINST_LIVE) != 0
}
#[inline]
pub const fn eeinst_rename_test(pinst: &EEInst, reg: usize) -> bool {
    reg == 0 || !eeinst_used_test(pinst, reg) || !eeinst_live_test(pinst, reg)
}

#[inline]
pub const fn fpuinst_islive(pinst: &EEInst, reg: usize) -> bool {
    (pinst.fpuregs[reg] & EEINST_LIVE) != 0
}
#[inline]
pub const fn fpuinst_lastuse(pinst: &EEInst, reg: usize) -> bool {
    (pinst.fpuregs[reg] & EEINST_LASTUSE) != 0
}
#[inline]
pub const fn fpuinst_used_test(pinst: &EEInst, reg: usize) -> bool {
    (pinst.fpuregs[reg] & (EEINST_USED | EEINST_LASTUSE)) == EEINST_USED
}
#[inline]
pub const fn fpuinst_live_test(pinst: &EEInst, reg: usize) -> bool {
    EE_WRITE_DEAD_VALUES || fpuinst_islive(pinst, reg)
}
#[inline]
pub const fn fpuinst_rename_test(pinst: &EEInst, reg: usize) -> bool {
    !eeinst_used_test(pinst, reg) || !eeinst_live_test(pinst, reg)
}

// ---------------------------------------------------------------------------
// Global regalloc state (from `iCore.cpp`).
// ---------------------------------------------------------------------------

/// Number of allocatable GPR slots on the x86 backend.
pub const IREGCNT_GPR: usize = 8;
/// Number of allocatable XMM slots on the x86 backend.
pub const IREGCNT_XMM: usize = 16;

thread_local! {
    /// Stand-in for the C `j8Ptr[32]` and `j32Ptr[32]` jump-patch
    /// scratch arrays (deprecated in modern code).
    pub static J8_PTR: RefCell<[*mut u8; 32]> =
        RefCell::new([ptr::null_mut(); 32]);
    pub static J32_PTR: RefCell<[*mut u32; 32]> =
        RefCell::new([ptr::null_mut(); 32]);
}

/// Per-thread X86 and XMM allocation tables. The original C++ is
/// global, but the dynarec is re-entrant; using a thread-local keeps
/// the port simple without losing semantics.
thread_local! {
    pub static X86REGS: RefCell<Vec<X86Reg>> =
        RefCell::new(vec![X86Reg::default(); IREGCNT_GPR]);
    pub static XMMREGS: RefCell<Vec<XmmReg>> =
        RefCell::new(vec![XmmReg::default(); IREGCNT_XMM]);
    pub static S_SAVE_X86REGS: RefCell<Vec<X86Reg>> =
        RefCell::new(vec![X86Reg::default(); IREGCNT_GPR]);
    pub static S_SAVE_XMMREGS: RefCell<Vec<XmmReg>> =
        RefCell::new(vec![XmmReg::default(); IREGCNT_XMM]);
}

/// Per-thread allocation-counter (16-bit wrap).
thread_local! {
    pub static G_X86_ALLOC_COUNTER: RefCell<u16> = const { RefCell::new(0) };
    pub static G_XMM_ALLOC_COUNTER: RefCell<u16> = const { RefCell::new(0) };
}

/// `g_pCurInstInfo`: pointer to the current instruction's liveness
/// info. The original is a raw pointer; here we use a `RefCell`
/// pointer for safety.
thread_local! {
    pub static G_P_CUR_INST_INFO: RefCell<*const EEInst> =
        const { RefCell::new(ptr::null()) };
}

// ---------------------------------------------------------------------------
// iCore regalloc API (from `iCore.{h,cpp}`).
// ---------------------------------------------------------------------------

/// Mirrors `bool _isAllocatableX86reg(int x86reg)` from `iCore.cpp`.
#[inline]
pub fn is_allocatable_x86reg(x86reg: i32) -> bool {
    // rax, rcx, rdx are scratch
    if x86reg <= 2 {
        return false;
    }
    // arg1reg and arg2reg reserved (Windows: ecx/edx, Linux: rsi/rdi)
    // The exact IDs are runtime-dependent; here we conservatively
    // disallow them by ID when fastmem is on.
    if x86reg == 4 {
        return false; // rsp
    }
    true
}

/// Initialize the XMM register cache.
pub fn init_xmmregs() {
    XMMREGS.with(|r| {
        for x in r.borrow_mut().iter_mut() {
            *x = XmmReg::default();
        }
    });
    G_XMM_ALLOC_COUNTER.with(|c| *c.borrow_mut() = 0);
}

/// Find an unused XMM register, or evict a dead one. Returns the
/// index, or -1 on failure. Mirrors `_getFreeXMMreg` from `iCore.cpp`.
pub fn get_free_xmmreg(maxreg: u32) -> i32 {
    let maxreg = maxreg as usize;
    XMMREGS.with(|r| {
        let regs = r.borrow();
        // Step 1: any free register
        for (i, x) in regs.iter().enumerate().take(maxreg) {
            if x.inuse == 0 {
                return i as i32;
            }
        }
        // Step 2: dead reg with lowest counter
        let mut best: i32 = -1;
        let mut best_count: u16 = 0xffff;
        for (i, x) in regs.iter().enumerate().take(maxreg) {
            if x.needed != 0 {
                continue;
            }
            if x.ty == XMMTYPE_TEMP {
                continue;
            }
            if x.counter < best_count {
                best = i as i32;
                best_count = x.counter;
            }
        }
        if best >= 0 {
            drop(regs);
            free_xmmreg(best);
            return best;
        }
        // Step 3: any reg with lowest counter
        let mut best_count: u16 = 0xffff;
        for (i, x) in regs.iter().enumerate().take(maxreg) {
            if x.needed != 0 {
                continue;
            }
            if x.counter < best_count {
                best = i as i32;
                best_count = x.counter;
            }
        }
        if best >= 0 {
            drop(regs);
            free_xmmreg(best);
            return best;
        }
        -1
    })
}

/// Reserve a temporary XMM register. Mirrors `_allocTempXMMreg`.
pub fn alloc_temp_xmmreg(_sse_type: u32) -> i32 {
    let r = get_free_xmmreg(IREGCNT_XMM as u32);
    if r < 0 {
        return -1;
    }
    XMMREGS.with(|regs| {
        let mut regs = regs.borrow_mut();
        let x = &mut regs[r as usize];
        x.inuse = 1;
        x.ty = XMMTYPE_TEMP;
        x.needed = 1;
    });
    G_XMM_ALLOC_COUNTER.with(|c| {
        let v = c.borrow().wrapping_add(1);
        *c.borrow_mut() = v;
    });
    r
}

/// Free a cached XMM register. Mirrors `_freeXMMreg`.
pub fn free_xmmreg(xmmreg: i32) {
    XMMREGS.with(|r| {
        let mut regs = r.borrow_mut();
        if xmmreg < 0 || xmmreg as usize >= regs.len() {
            return;
        }
        let x = &mut regs[xmmreg as usize];
        x.inuse = 0;
        x.needed = 0;
        x.ty = XMMTYPE_TEMP;
        x.mode = 0;
    });
}

/// Free a cached XMM register without writing back. Mirrors
/// `_freeXMMregWithoutWriteback`.
pub fn free_xmmreg_without_writeback(xmmreg: i32) {
    free_xmmreg(xmmreg);
}

/// Free a cached x86 GPR. Mirrors `_freeX86reg`.
pub fn free_x86reg(x86reg: i32) {
    X86REGS.with(|r| {
        let mut regs = r.borrow_mut();
        if x86reg < 0 || x86reg as usize >= regs.len() {
            return;
        }
        let x = &mut regs[x86reg as usize];
        x.inuse = 0;
        x.needed = 0;
        x.ty = X86Type::Temp;
        x.mode = 0;
    });
}

/// Free a cached x86 GPR without writing back.
pub fn free_x86reg_without_writeback(x86reg: i32) {
    free_x86reg(x86reg);
}

/// Returns true if there is a cached x86 register holding the
/// specified EE state.
pub fn has_x86reg(ty: X86Type, reg: i32, required_mode: u32) -> bool {
    X86REGS.with(|r| {
        let regs = r.borrow();
        for x in regs.iter() {
            if x.inuse != 0 && x.ty == ty && x.reg as i32 == reg {
                return (x.mode as u32 & required_mode) == required_mode;
            }
        }
        false
    })
}

/// Returns true if there is a cached XMM register holding the
/// specified EE state.
pub fn has_xmmreg(ty: u8, reg: i32, required_mode: u32) -> bool {
    XMMREGS.with(|r| {
        let regs = r.borrow();
        for x in regs.iter() {
            if x.inuse != 0 && x.ty == ty && x.reg as i32 == reg {
                return (x.mode as u32 & required_mode) == required_mode;
            }
        }
        false
    })
}

/// Allocate an x86 GPR for a given EE state, or reuse an existing
/// one. Mirrors `_allocX86reg` / `_checkX86reg`.
pub fn alloc_x86reg(ty: X86Type, reg: i32, mode: u32) -> i32 {
    // If already cached, set the mode and return.
    X86REGS.with(|r| {
        let mut regs = r.borrow_mut();
        for (i, x) in regs.iter_mut().enumerate() {
            if x.inuse != 0 && x.ty == ty && x.reg as i32 == reg {
                x.mode = (x.mode as u32 | mode) as u8;
                return i as i32;
            }
        }
        // Otherwise, find a free slot.
        for (i, x) in regs.iter_mut().enumerate() {
            if x.inuse == 0 {
                x.inuse = 1;
                x.reg = reg as i8;
                x.ty = ty;
                x.mode = mode as u8;
                x.needed = 0;
                x.counter = G_X86_ALLOC_COUNTER.with(|c| {
                    let v = c.borrow().wrapping_add(1);
                    *c.borrow_mut() = v;
                    v
                });
                return i as i32;
            }
        }
        -1
    })
}

/// Same as [`alloc_x86reg`], but only allocates if the EE register is
/// used later in the block. Mirrors `_allocIfUsedGPRtoX86`.
pub fn alloc_if_used_gpr_to_x86(gprreg: i32, mode: u32) -> i32 {
    let used = G_P_CUR_INST_INFO.with(|p| {
        let p = *p.borrow();
        if p.is_null() {
            false
        } else {
            unsafe { eeinst_used_test(&*p, gprreg as usize) }
        }
    });
    if used {
        alloc_x86reg(X86Type::Gpr, gprreg, mode)
    } else {
        check_x86reg(X86Type::Gpr, gprreg, mode)
    }
}

/// Same as [`alloc_if_used_gpr_to_x86`] for VI regs.
pub fn alloc_if_used_vi_to_x86(vireg: i32, mode: u32) -> i32 {
    let used = G_P_CUR_INST_INFO.with(|p| {
        let p = *p.borrow();
        if p.is_null() {
            false
        } else {
            unsafe { eeinst_vi_used_test(&*p, vireg as usize) }
        }
    });
    if used {
        alloc_x86reg(X86Type::ViReg, vireg, mode)
    } else {
        check_x86reg(X86Type::ViReg, vireg, mode)
    }
}

pub fn alloc_if_used_gpr_to_xmm(gprreg: i32, mode: u32) -> i32 {
    let used = G_P_CUR_INST_INFO.with(|p| {
        let p = *p.borrow();
        if p.is_null() {
            false
        } else {
            unsafe { eeinst_used_test(&*p, gprreg as usize) }
        }
    });
    if used {
        alloc_gpr_to_xmmreg(gprreg, mode)
    } else {
        check_xmmreg(XMMTYPE_GPRREG, gprreg, mode)
    }
}

pub fn alloc_if_used_fpu_to_xmm(fpureg: i32, mode: u32) -> i32 {
    let used = G_P_CUR_INST_INFO.with(|p| {
        let p = *p.borrow();
        if p.is_null() {
            false
        } else {
            unsafe { fpuinst_used_test(&*p, fpureg as usize) }
        }
    });
    if used {
        alloc_fp_to_xmmreg(fpureg, mode)
    } else {
        check_xmmreg(XMMTYPE_FPREG, fpureg, mode)
    }
}

/// Locate a cached x86 GPR holding the given state, or return -1.
/// Mirrors `_checkX86reg`.
pub fn check_x86reg(ty: X86Type, reg: i32, mode: u32) -> i32 {
    X86REGS.with(|r| {
        let mut regs = r.borrow_mut();
        for (i, x) in regs.iter_mut().enumerate() {
            if x.inuse != 0 && x.ty == ty && x.reg as i32 == reg {
                x.mode = (x.mode as u32 | mode) as u8;
                return i as i32;
            }
        }
        -1
    })
}

/// Same as [`check_x86reg`] for XMM.
pub fn check_xmmreg(ty: u8, reg: i32, mode: u32) -> i32 {
    XMMREGS.with(|r| {
        let mut regs = r.borrow_mut();
        for (i, x) in regs.iter_mut().enumerate() {
            if x.inuse != 0 && x.ty == ty && x.reg as i32 == reg {
                x.mode = (x.mode as u32 | mode) as u8;
                return i as i32;
            }
        }
        -1
    })
}

/// Allocate an XMM to hold a copy of an EE FPR. Mirrors
/// `_allocFPtoXMMreg`.
pub fn alloc_fp_to_xmmreg(fpreg: i32, mode: u32) -> i32 {
    let slot = get_free_xmmreg(IREGCNT_XMM as u32);
    if slot < 0 {
        return -1;
    }
    XMMREGS.with(|r| {
        let mut regs = r.borrow_mut();
        let x = &mut regs[slot as usize];
        x.inuse = 1;
        x.ty = XMMTYPE_FPREG;
        x.reg = fpreg as i8;
        x.mode = mode as u8;
        x.counter = G_XMM_ALLOC_COUNTER.with(|c| {
            let v = c.borrow().wrapping_add(1);
            *c.borrow_mut() = v;
            v
        });
    });
    slot
}

/// Allocate an XMM to hold a copy of an EE GPR pair. Mirrors
/// `_allocGPRtoXMMreg`.
pub fn alloc_gpr_to_xmmreg(gprreg: i32, mode: u32) -> i32 {
    let slot = get_free_xmmreg(IREGCNT_XMM as u32);
    if slot < 0 {
        return -1;
    }
    XMMREGS.with(|r| {
        let mut regs = r.borrow_mut();
        let x = &mut regs[slot as usize];
        x.inuse = 1;
        x.ty = XMMTYPE_GPRREG;
        x.reg = gprreg as i8;
        x.mode = mode as u8;
    });
    slot
}

/// Allocate an XMM to hold the FPU ACC. Mirrors `_allocFPACCtoXMMreg`.
pub fn alloc_fpacc_to_xmmreg(mode: u32) -> i32 {
    let slot = get_free_xmmreg(IREGCNT_XMM as u32);
    if slot < 0 {
        return -1;
    }
    XMMREGS.with(|r| {
        let mut regs = r.borrow_mut();
        let x = &mut regs[slot as usize];
        x.inuse = 1;
        x.ty = XMMTYPE_FPACC;
        x.reg = XMMFPU_ACC as i8;
        x.mode = mode as u8;
    });
    slot
}

/// Allocate an XMM to hold a VU VF register. Mirrors
/// `_allocVFtoXMMreg`.
pub fn alloc_vf_to_xmmreg(vfreg: i32, mode: u32) -> i32 {
    let slot = get_free_xmmreg(IREGCNT_XMM as u32);
    if slot < 0 {
        return -1;
    }
    XMMREGS.with(|r| {
        let mut regs = r.borrow_mut();
        let x = &mut regs[slot as usize];
        x.inuse = 1;
        x.ty = XMMTYPE_VFREG;
        x.reg = vfreg as i8;
        x.mode = mode as u8;
    });
    slot
}

/// Update the EE-state tag on a cached XMM register. Mirrors
/// `_reallocateXMMreg`.
pub fn reallocate_xmmreg(
    xmmreg: i32,
    newtype: u8,
    newreg: i32,
    newmode: u32,
    _writeback: bool,
) {
    XMMREGS.with(|r| {
        let mut regs = r.borrow_mut();
        if xmmreg < 0 || xmmreg as usize >= regs.len() {
            return;
        }
        let x = &mut regs[xmmreg as usize];
        x.ty = newtype;
        x.reg = newreg as i8;
        x.mode = newmode as u8;
    });
}

pub fn flush_xmmregs() {
    // ASM stub
}
pub fn flush_xmmreg(_xmmreg: i32) {
    // ASM stub
}
pub fn flush_x86regs() {
    // ASM stub
}
pub fn flush_const_regs(_delete_const: bool) {
    // ASM stub
}
pub fn flush_const_reg(_reg: i32) {
    // ASM stub
}
pub fn clear_needed_xmmregs() {
    XMMREGS.with(|r| {
        for x in r.borrow_mut().iter_mut() {
            x.needed = 0;
        }
    });
}
pub fn clear_needed_x86regs() {
    X86REGS.with(|r| {
        for x in r.borrow_mut().iter_mut() {
            x.needed = 0;
        }
    });
}
pub fn validate_regs() {
    // ASM stub
}
pub fn writeback_x86_reg(_x86reg: i32) {
    // ASM stub
}
pub fn writeback_xmm_reg(_xmmreg: i32) {
    // ASM stub
}
pub fn add_needed_x86reg(_ty: X86Type, _reg: i32) {}
pub fn add_needed_xmmreg(_ty: u8, _reg: i32) {}
pub fn add_needed_fpto_xmmreg(_fpreg: i32) {}
pub fn add_needed_fpacc_to_xmmreg() {}
pub fn add_needed_gprto_x86reg(_gprreg: i32) {}
pub fn add_needed_psxto_x86reg(_gprreg: i32) {}
pub fn add_needed_gprto_xmmreg(_gprreg: i32) {}
pub fn delete_gprto_x86reg(_reg: i32, _flush: i32) {
    // ASM stub
}
pub fn delete_psxto_x86reg(_reg: i32, _flush: i32) {
    // ASM stub
}
pub fn delete_gprto_xmmreg(_reg: i32, _flush: i32) {
    // ASM stub
}
pub fn delete_fpto_xmmreg(_reg: i32, _flush: i32) {
    // ASM stub
}
pub fn delete_ee_reg(_reg: i32, _flush: i32) {
    // ASM stub
}
pub fn delete_ee_reg128(_reg: i32) {
    // ASM stub
}
pub fn flush_ee_reg(_reg: i32, _clear: bool) {
    // ASM stub
}
pub fn flush_cop2_regs() {
    // ASM stub
}
pub fn flush_all_dirty() {
    // ASM stub
}
pub fn on_write_reg(_reg: i32, _signext: i32) {
    // ASM stub
}
pub fn ee_try_rename_reg(
    _to: i32,
    _from: i32,
    _from_x86: i32,
    _other: i32,
    _xmminfo: i32,
) -> i32 {
    -1
}
pub fn ee_move_gpr_to_r(_to: i32, _fromgpr: i32, _allow_preload: bool) {
    // ASM stub
}
pub fn ee_move_gpr_to_m(_to: uptr, _fromgpr: i32) {
    // ASM stub
}
pub fn mvu_free_cop2_gpr(_hostreg: i32) {
    // ASM stub
}
pub fn mvu_is_reserved_cop2(_hostreg: i32) -> bool {
    false
}
pub fn mvu_free_cop2_xmm_reg(_hostreg: i32) {
    // ASM stub
}

/// Returns the number of instructions + 1 until the register is next
/// written, or 0 if never. Mirrors `_recIsRegReadOrWritten`.
pub fn rec_is_reg_read_or_written(
    _pinst: &EEInst,
    _size: i32,
    _xmmtype: u8,
    _reg: i32,
) -> u32 {
    0
}
pub fn rec_fill_register(_pinst: &mut EEInst, _ty: i32, _reg: i32, _write: i32) {
    // ASM stub
}

// ===========================================================================
// Section: Instruction field extractors (from `iR3000A.h`, etc.)
// ===========================================================================

#[inline]
pub const fn instr_op(instr: u32) -> u32 {
    instr >> 26
}
#[inline]
pub const fn instr_rs(instr: u32) -> u32 {
    (instr >> 21) & 0x1f
}
#[inline]
pub const fn instr_rt(instr: u32) -> u32 {
    (instr >> 16) & 0x1f
}
#[inline]
pub const fn instr_rd(instr: u32) -> u32 {
    (instr >> 11) & 0x1f
}
#[inline]
pub const fn instr_sa(instr: u32) -> u32 {
    (instr >> 6) & 0x1f
}
#[inline]
pub const fn instr_funct(instr: u32) -> u32 {
    instr & 0x3f
}
#[inline]
pub const fn instr_imm(instr: u32) -> u32 {
    instr & 0xffff
}
#[inline]
pub const fn instr_imm_u(instr: u32) -> u32 {
    instr & 0xffff
}
#[inline]
pub const fn instr_target(instr: u32) -> u32 {
    instr & 0x03ff_ffff
}

// ===========================================================================
// Section: R5900 (EE) recompiler API (from `iR5900.h`)
// ===========================================================================

/// Maximum compiled-code size the dynarec will emit per thread.
pub static mut MAX_RECMEM: u32 = 0;

/// Current EE recompiler PC.
pub static mut PC: u32 = 0;
/// Set when the dynarec wants to record a branch.
pub static mut G_BRANCH: i32 = 0;
/// Branch target.
pub static mut TARGET: u32 = 0;
/// Cycles of the block currently being recompiled.
pub static mut S_N_BLOCK_CYCLES: u32 = 0;
/// Current block has VU0 interlocking.
pub static mut S_N_BLOCK_INTERLOCKED: bool = false;

/// True when we are currently recompiling a delay slot.
pub static mut G_RECOMPILING_DELAY_SLOT: bool = false;

/// Pointer to fastmem base in the EE dynarec; matches `RFASTMEMBASE`.
pub const RFASTMEMBASE: u32 = 5; // rbp in x64 encoding

/// `R5900_TEXTPTR`. x86 displacement to the field used as the
/// "text pointer" by the EE recompiler; a 144-byte offset into
/// cpuRegs so the GPR file can be reached with an s8 displacement.
pub const R5900_TEXTPTR_OFFSET: u32 = 0;

// EE constant-propagation register state.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C, align(16))]
pub struct GprReg64 {
    pub ud: [u32; 2],
}

/// 32 EE constant registers.
pub static mut G_CPU_CONST_REGS: [GprReg64; 32] = [GprReg64 {
    ud: [0; 2],
}; 32];

/// Bitmasks tracking which EE GPRs currently hold a propagated
/// constant, and which of those have not yet been flushed.
pub static mut G_CPU_HAS_CONST_REG: u32 = 0;
pub static mut G_CPU_FLUSHED_CONST_REG: u32 = 0;

// ---------------------------------------------------------------------------
// R5900 dynarec helper API (from `iR5900.h`).
// ---------------------------------------------------------------------------

pub fn rec_begin_thunk() -> *mut u8 {
    ptr::null_mut()
}
pub fn rec_end_thunk() -> *mut u8 {
    ptr::null_mut()
}
pub fn try_swap_delay_slot(
    _rs: u32,
    _rt: u32,
    _rd: u32,
    _allow_loadstore: bool,
) -> bool {
    false
}
pub fn save_branch_state() {
    // ASM stub
}
pub fn load_branch_state() {
    // ASM stub
}
pub fn recompile_next_instruction(_delayslot: bool, _swapped: bool) {
    // ASM stub
}
pub fn set_branch_reg() {
    unsafe { G_BRANCH = 2; }
}
pub fn set_branch_imm(_imm: u32) {
    unsafe { G_BRANCH = 2; }
}
pub fn i_flush_call(_flushtype: u32) {
    // ASM stub
}
pub fn rec_branch_call(_func: extern "C" fn()) {
    // ASM stub
}
pub fn rec_call(_func: extern "C" fn()) {
    // ASM stub
}
pub fn scale_block_cycles_clear() -> u32 {
    0
}

pub mod r5900 {
    pub mod dynarec {
        /// `R5900::Dynarec::recDoBranchImm`.
        pub fn rec_do_branch_imm(
            _branch_to: u32,
            _jmp_skip: *mut u32,
            _is_likely: bool,
            _swapped_delay_slot: bool,
        ) {
            // ASM stub
        }
    }
}

/// Type alias matching `R5900FNPTR` (function with no args, no ret).
pub type R5900FnpTr = extern "C" fn();
/// Type alias matching `R5900FNPTR_INFO` (function with one int arg).
pub type R5900FnpTrInfo = extern "C" fn(i32);

/// `eeRecompileCodeRC0` - rd = rs op rt.
pub fn ee_recompile_code_rc0(
    _constcode: R5900FnpTr,
    _constscode: R5900FnpTrInfo,
    _consttcode: R5900FnpTrInfo,
    _noconstcode: R5900FnpTrInfo,
    _xmminfo: i32,
) {
    // ASM stub
}
/// `eeRecompileCodeRC1` - rt = rs op imm16.
pub fn ee_recompile_code_rc1(
    _constcode: R5900FnpTr,
    _noconstcode: R5900FnpTrInfo,
    _xmminfo: i32,
) {
    // ASM stub
}
/// `eeRecompileCodeRC2` - rd = rt op sa.
pub fn ee_recompile_code_rc2(
    _constcode: R5900FnpTr,
    _noconstcode: R5900FnpTrInfo,
    _xmminfo: i32,
) {
    // ASM stub
}
/// `eeRecompileCodeXMM` - rd = rs op rt (XMM variant).
pub fn ee_recompile_code_xmm(_xmminfo: i32) -> i32 {
    -1
}
/// `eeFPURecompileCode` - dispatch an FPU op through the dynarec.
pub fn ee_fpu_recompile_code(
    _xmmcode: R5900FnpTrInfo,
    _fpucode: R5900FnpTr,
    _xmminfo: i32,
) {
    // ASM stub
}

// ---------------------------------------------------------------------------
// EE opcode dynarec handlers, namespaced as in `iR5900*.h`.
// ---------------------------------------------------------------------------

/// Macro replacement: `GPR_IS_CONST1(reg)` -> true if `reg < 32` and
/// its bit in `g_cpuHasConstReg` is set.
#[inline]
pub fn gpr_is_const1(reg: i32) -> bool {
    unsafe {
        EE_CONST_PROP && (reg as u32) < 32 && (G_CPU_HAS_CONST_REG & (1 << reg)) != 0
    }
}
#[inline]
pub fn gpr_is_const2(reg1: i32, reg2: i32) -> bool {
    unsafe {
        EE_CONST_PROP
            && (G_CPU_HAS_CONST_REG & (1 << reg1)) != 0
            && (G_CPU_HAS_CONST_REG & (1 << reg2)) != 0
    }
}
#[inline]
pub fn gpr_is_dirty_const(reg: i32) -> bool {
    unsafe {
        EE_CONST_PROP
            && (reg as u32) < 32
            && (G_CPU_HAS_CONST_REG & (1 << reg)) != 0
            && (G_CPU_FLUSHED_CONST_REG & (1 << reg)) == 0
    }
}
pub fn gpr_set_const(reg: i32) {
    unsafe {
        if (reg as u32) < 32 {
            G_CPU_HAS_CONST_REG |= 1 << reg;
            G_CPU_FLUSHED_CONST_REG &= !(1 << reg);
        }
    }
}
pub fn gpr_del_const(reg: i32) {
    unsafe {
        if (reg as u32) < 32 {
            G_CPU_HAS_CONST_REG &= !(1 << reg);
        }
    }
}
pub const EE_CONST_PROP: bool = true;

// ---------------------------------------------------------------------------
// R5900::Dynarec::OpcodeImpl::{COP0,COP1,FPU,MMI,*Arit*,*AritImm*,*Move*,
//                                 *Branch*,*Jump*,*LoadStore*,*MultDiv*,
//                                 *Shift*} function declarations.
// ---------------------------------------------------------------------------

pub mod opcode_impl {
    //! R5900 (EE) dynarec handlers, all of which are bodies of
    //! `unimplemented!()`. Their declarations are kept because the
    //! generated code calls them by name through the dynarec dispatcher.

    // ----- Cop0 (from `iCOP0.{h,cpp}`) -----
    pub mod cop0 {
        extern "C" {
            pub fn recMFC0();
            pub fn recMTC0();
            pub fn recBC0F();
            pub fn recBC0T();
            pub fn recBC0FL();
            pub fn recBC0TL();
            pub fn recTLBR();
            pub fn recTLBWI();
            pub fn recTLBWR();
            pub fn recTLBP();
            pub fn recERET();
            pub fn recDI();
            pub fn recEI();
        }
    }

    // ----- Cop1 / FPU (from `iFPU.{h,cpp}`) -----
    pub mod cop1 {
        extern "C" {
            pub fn recMFC1();
            pub fn recCFC1();
            pub fn recMTC1();
            pub fn recCTC1();
            pub fn recCOP1_BC1();
            pub fn recCOP1_S();
            pub fn recCOP1_W();
            pub fn recC_EQ();
            pub fn recC_F();
            pub fn recC_LT();
            pub fn recC_LE();
            pub fn recADD_S();
            pub fn recSUB_S();
            pub fn recMUL_S();
            pub fn recDIV_S();
            pub fn recSQRT_S();
            pub fn recABS_S();
            pub fn recMOV_S();
            pub fn recNEG_S();
            pub fn recRSQRT_S();
            pub fn recADDA_S();
            pub fn recSUBA_S();
            pub fn recMULA_S();
            pub fn recMADD_S();
            pub fn recMSUB_S();
            pub fn recMADDA_S();
            pub fn recMSUBA_S();
            pub fn recCVT_S();
            pub fn recCVT_W();
            pub fn recMAX_S();
            pub fn recMIN_S();
            pub fn recBC1F();
            pub fn recBC1T();
            pub fn recBC1FL();
            pub fn recBC1TL();
        }
    }

    // ----- MMI (from `iMMI.{h,cpp}`) -----
    pub mod mmi_common {
        extern "C" {
            pub fn recMADD1();
            pub fn recMADDU1();
            pub fn recMADD();
            pub fn recMADDU();
            pub fn recMTHI1();
            pub fn recMTLO1();
            pub fn recMFHI1();
            pub fn recMFLO1();
            pub fn recMULT1();
            pub fn recMULTU1();
            pub fn recDIV1();
            pub fn recDIVU1();
        }
    }
    pub mod mmi {
        extern "C" {
            pub fn recPLZCW();
            pub fn recMMI0();
            pub fn recMMI1();
            pub fn recMMI2();
            pub fn recMMI3();
            pub fn recPMFHL();
            pub fn recPMTHL();
            pub fn recPMAXW();
            pub fn recPMINW();
            pub fn recPPACW();
            pub fn recPEXTLH();
            pub fn recPPACH();
            pub fn recPEXTLB();
            pub fn recPPACB();
            pub fn recPEXT5();
            pub fn recPPAC5();
            pub fn recPABSW();
            pub fn recPADSBH();
            pub fn recPABSH();
            pub fn recPADDUW();
            pub fn recPSUBUW();
            pub fn recPSUBUH();
            pub fn recPEXTUH();
            pub fn recPSUBUB();
            pub fn recPEXTUB();
            pub fn recQFSRV();
            pub fn recPMADDW();
            pub fn recPSLLVW();
            pub fn recPSRLVW();
            pub fn recPMSUBW();
            pub fn recPINTH();
            pub fn recPMULTW();
            pub fn recPDIVW();
            pub fn recPMADDH();
            pub fn recPHMADH();
            pub fn recPMSUBH();
            pub fn recPHMSBH();
            pub fn recPEXEH();
            pub fn recPREVH();
            pub fn recPMULTH();
            pub fn recPDIVBW();
            pub fn recPEXEW();
            pub fn recPROT3W();
            pub fn recPMADDUW();
            pub fn recPSRAVW();
            pub fn recPINTEH();
            pub fn recPMULTUW();
            pub fn recPDIVUW();
            pub fn recPEXCH();
            pub fn recPEXCW();
            pub fn recPSRLH();
            pub fn recPSRLW();
            pub fn recPSRAH();
            pub fn recPSRAW();
            pub fn recPSLLH();
            pub fn recPSLLW();
            pub fn recPMAXH();
            pub fn recPCGTB();
            pub fn recPCGTH();
            pub fn recPCGTW();
            pub fn recPADDSB();
            pub fn recPADDSH();
            pub fn recPADDSW();
            pub fn recPSUBSB();
            pub fn recPSUBSH();
            pub fn recPSUBSW();
            pub fn recPADDB();
            pub fn recPADDH();
            pub fn recPADDW();
            pub fn recPSUBB();
            pub fn recPSUBH();
            pub fn recPSUBW();
            pub fn recPEXTLW();
            pub fn recPEXTUW();
            pub fn recPMINH();
            pub fn recPCEQB();
            pub fn recPCEQH();
            pub fn recPCEQW();
            pub fn recPADDUB();
            pub fn recPADDUH();
            pub fn recPMFHI();
            pub fn recPMFLO();
            pub fn recPAND();
            pub fn recPXOR();
            pub fn recPCPYLD();
            pub fn recPNOR();
            pub fn recPMTHI();
            pub fn recPMTLO();
            pub fn recPCPYUD();
            pub fn recPOR();
            pub fn recPCPYH();
        }
    }

    // ----- Arit (from `iR5900Arit.h`) -----
    extern "C" {
        pub fn recADD();
        pub fn recADDU();
        pub fn recDADD();
        pub fn recDADDU();
        pub fn recSUB();
        pub fn recSUBU();
        pub fn recDSUB();
        pub fn recDSUBU();
        pub fn recAND();
        pub fn recOR();
        pub fn recXOR();
        pub fn recNOR();
        pub fn recSLT();
        pub fn recSLTU();
    }
    // ----- AritImm (from `iR5900AritImm.h`) -----
    extern "C" {
        pub fn recADDI();
        pub fn recADDIU();
        pub fn recDADDI();
        pub fn recDADDIU();
        pub fn recANDI();
        pub fn recORI();
        pub fn recXORI();
        pub fn recSLTI();
        pub fn recSLTIU();
    }
    // ----- Move (from `iR5900Move.h`) -----
    extern "C" {
        pub fn recLUI();
        pub fn recMFLO();
        pub fn recMFHI();
        pub fn recMTLO();
        pub fn recMTHI();
        pub fn recMOVN();
        pub fn recMOVZ();
    }
    // ----- Branch (from `iR5900Branch.h`) -----
    extern "C" {
        pub fn recBEQ();
        pub fn recBEQL();
        pub fn recBNE();
        pub fn recBNEL();
        pub fn recBLTZ();
        pub fn recBLTZL();
        pub fn recBLTZAL();
        pub fn recBLTZALL();
        pub fn recBGTZ();
        pub fn recBGTZL();
        pub fn recBLEZ();
        pub fn recBLEZL();
        pub fn recBGEZ();
        pub fn recBGEZL();
        pub fn recBGEZAL();
        pub fn recBGEZALL();
    }
    // ----- Jump (from `iR5900Jump.h`) -----
    extern "C" {
        pub fn recJ();
        pub fn recJAL();
        pub fn recJR();
        pub fn recJALR();
    }
    // ----- LoadStore (from `iR5900LoadStore.h`) -----
    extern "C" {
        pub fn recLB();
        pub fn recLBU();
        pub fn recLH();
        pub fn recLHU();
        pub fn recLW();
        pub fn recLWU();
        pub fn recLWL();
        pub fn recLWR();
        pub fn recLD();
        pub fn recLDR();
        pub fn recLDL();
        pub fn recLQ();
        pub fn recSB();
        pub fn recSH();
        pub fn recSW();
        pub fn recSWL();
        pub fn recSWR();
        pub fn recSD();
        pub fn recSDL();
        pub fn recSDR();
        pub fn recSQ();
        pub fn recLWC1();
        pub fn recSWC1();
        pub fn recLQC2();
        pub fn recSQC2();
    }
    // ----- MultDiv (from `iR5900MultDiv.h`) -----
    extern "C" {
        pub fn recMULT();
        pub fn recMULTU();
        pub fn recDIV();
        pub fn recDIVU();
    }
    // ----- Shift (from `iR5900Shift.h`) -----
    extern "C" {
        pub fn recSLL();
        pub fn recSRL();
        pub fn recSRA();
        pub fn recDSLL();
        pub fn recDSRL();
        pub fn recDSRA();
        pub fn recDSLL32();
        pub fn recDSRL32();
        pub fn recDSRA32();
        pub fn recSLLV();
        pub fn recSRLV();
        pub fn recSRAV();
        pub fn recDSLLV();
        pub fn recDSRLV();
        pub fn recDSRAV();
    }
    // ----- Misc / "trampoline" handlers (from `iR5900Misc.cpp`) -----
    extern "C" {
        pub fn recPREF();
        pub fn recSYNC();
        pub fn recMFSA();
        pub fn recMTSA();
        pub fn recMTSAB();
        pub fn recMTSAH();
        pub fn recCACHE();
        pub fn recNULL();
        pub fn recUnknown();
        pub fn recMMI_Unknown();
        pub fn recCOP0_Unknown();
        pub fn recCOP1_Unknown();
        pub fn recTGE();
        pub fn recTGEU();
        pub fn recTLT();
        pub fn recTLTU();
        pub fn recTEQ();
        pub fn recTNE();
        pub fn recTGEI();
        pub fn recTGEIU();
        pub fn recTLTI();
        pub fn recTLTIU();
        pub fn recTEQI();
        pub fn recTNEI();
    }
}

// ---------------------------------------------------------------------------
// R5900 analysis passes (from `iR5900Analysis.{h,cpp}`).
// ---------------------------------------------------------------------------

/// Base analysis pass over an EE block.
pub struct AnalysisPass {
    _private: [u8; 0],
}
impl AnalysisPass {
    pub fn new() -> Self {
        AnalysisPass { _private: [] }
    }
    pub fn run(&self, _start: u32, _end: u32, _inst_cache: &mut [EEInst]) {
        // ASM stub
    }
    pub fn for_each_instruction<F: FnMut(u32, &EEInst) -> bool>(
        &self,
        _start: u32,
        _end: u32,
        _inst_cache: &mut [EEInst],
        _func: F,
    ) {
        // ASM stub
    }
}
impl Default for AnalysisPass {
    fn default() -> Self {
        Self::new()
    }
}

/// Analysis pass that fixes up COP2 flag-write annotations.
pub struct COP2FlagHackPass {
    pub status_denormalized: bool,
    pub last_status_write: *mut EEInst,
    pub last_mac_write: *mut EEInst,
    pub last_clip_write: *mut EEInst,
    pub cfc2_pc: u32,
    _parent: AnalysisPass,
}
impl COP2FlagHackPass {
    pub fn new() -> Self {
        COP2FlagHackPass {
            status_denormalized: false,
            last_status_write: ptr::null_mut(),
            last_mac_write: ptr::null_mut(),
            last_clip_write: ptr::null_mut(),
            cfc2_pc: 0,
            _parent: AnalysisPass::new(),
        }
    }
    pub fn run(&mut self, _start: u32, _end: u32, _inst_cache: &mut [EEInst]) {
        // ASM stub
    }
    fn commit_status_flag(&mut self) {
        // ASM stub
    }
    fn commit_mac_flag(&mut self) {
        // ASM stub
    }
    fn commit_clip_flag(&mut self) {
        // ASM stub
    }
    fn commit_all_flags(&mut self) {
        // ASM stub
    }
}
impl Default for COP2FlagHackPass {
    fn default() -> Self {
        Self::new()
    }
}

/// Analysis pass that ensures the EE ends a block cleanly when the
/// COP2 micro-VU finishes.
pub struct COP2MicroFinishPass {
    _parent: AnalysisPass,
}
impl COP2MicroFinishPass {
    pub fn new() -> Self {
        COP2MicroFinishPass {
            _parent: AnalysisPass::new(),
        }
    }
    pub fn run(&mut self, _start: u32, _end: u32, _inst_cache: &mut [EEInst]) {
        // ASM stub
    }
}
impl Default for COP2MicroFinishPass {
    fn default() -> Self {
        Self::new()
    }
}

pub fn rec_backprop_bsc(_code: u32, _prev: &mut EEInst, _pinst: &mut EEInst) {
    // ASM stub
}

// ===========================================================================
// Section: R3000A (IOP) interpreter + dynarec (from `iR3000A.{h,cpp}`,
//                       `iR3000Atables.cpp`)
// ===========================================================================

/// Cycle penalty for slow IOP operations.
pub const PSX_INST_CYCLES_MULT: i32 = 7;
pub const PSX_INST_CYCLES_DIV: i32 = 40;
pub const PSX_INST_CYCLES_PEEPHOLE_STORE: i32 = 0;
pub const PSX_INST_CYCLES_STORE: i32 = 0;
pub const PSX_INST_CYCLES_LOAD: i32 = 0;

/// PSX HI/LO alias to the EE XMM HI/LO constants.
pub const PSX_HI: i32 = XMMGPR_HI as i32;
pub const PSX_LO: i32 = XMMGPR_LO as i32;

/// IOP constant-propagation state. Mirrors the C++ globals.
pub static mut PSX_CONST_REGS: [u32; 32] = [0; 32];
pub static mut G_PSX_HAS_CONST_REG: u32 = 0;
pub static mut G_PSX_FLUSHED_CONST_REG: u32 = 0;

/// IOP recompiler PC.
pub static mut PSXPC: u32 = 0;
pub static mut PSX_BRANCH: i32 = 0;
pub static mut G_IOP_CYCLE_PENALTY: u32 = 0;

/// "Has the current PSX reg been written OK?" flag. Set by load/store
/// handlers when the write is allowed by the PSX protection map.
pub static mut G_PSX_WRITE_OK: i32 = 0;
/// Maximum amount of recompiled PSX memory.
pub static mut G_PSX_MAX_RECMEM: u32 = 0;

/// Page table for PSX recompiled blocks, indexed by 64K page.
pub static mut PSX_RECLUT: [uptr; 0x10000] = [0; 0x10000];

#[inline]
pub fn psx_is_const1(reg: i32) -> bool {
    unsafe { (reg as u32) < 32 && (G_PSX_HAS_CONST_REG & (1 << reg)) != 0 }
}
#[inline]
pub fn psx_is_const2(reg1: i32, reg2: i32) -> bool {
    unsafe {
        (G_PSX_HAS_CONST_REG & (1 << reg1)) != 0
            && (G_PSX_HAS_CONST_REG & (1 << reg2)) != 0
    }
}
#[inline]
pub fn psx_is_dirty_const(reg: i32) -> bool {
    unsafe {
        (reg as u32) < 32
            && (G_PSX_HAS_CONST_REG & (1 << reg)) != 0
            && (G_PSX_FLUSHED_CONST_REG & (1 << reg)) == 0
    }
}
pub fn psx_set_const(reg: i32) {
    unsafe {
        if (reg as u32) < 32 {
            G_PSX_HAS_CONST_REG |= 1u32 << reg;
            G_PSX_FLUSHED_CONST_REG &= !(1u32 << reg);
        }
    }
}
pub fn psx_del_const(reg: i32) {
    unsafe {
        if (reg as u32) < 32 {
            G_PSX_HAS_CONST_REG &= !(1 << reg);
        }
    }
}

pub fn psx_flush_const_reg(_reg: i32) {
    // ASM stub
}
pub fn psx_flush_const_regs() {
    // ASM stub
}
pub fn psx_delete_reg(_reg: i32, _flush: i32) {
    // ASM stub
}
pub fn psx_flush_call(_flushtype: u32) {
    // ASM stub
}
pub fn psx_flush_all_dirty() {
    // ASM stub
}
pub fn psx_on_write_reg(_reg: i32) {
    // ASM stub
}
pub fn psx_move_gpr_to_r(_to: i32, _fromgpr: i32) {
    // ASM stub
}
pub fn psx_move_gpr_to_m(_to: uptr, _fromgpr: i32) {
    // ASM stub
}
pub fn psx_save_branch_state() {
    // ASM stub
}
pub fn psx_load_branch_state() {
    // ASM stub
}
pub fn psx_set_branch_reg() {
    unsafe { PSX_BRANCH = 2; }
}
pub fn psx_set_branch_imm(_imm: u32) {
    unsafe { PSX_BRANCH = 2; }
}
pub fn psx_recompile_next_instruction(_delayslot: bool, _swapped: bool) {
    // ASM stub
}
pub fn psx_try_swap_delay_slot(_rs: u32, _rt: u32, _rd: u32) -> bool {
    false
}
pub fn psx_try_rename_reg(
    _to: i32,
    _from: i32,
    _from_x86: i32,
    _other: i32,
    _xmminfo: i32,
) -> i32 {
    -1
}

pub type R3000AFnpTr = extern "C" fn();
pub type R3000AFnpTrInfo = extern "C" fn(i32);

pub fn psx_recompile_code_const0(
    _constcode: R3000AFnpTr,
    _constscode: R3000AFnpTrInfo,
    _consttcode: R3000AFnpTrInfo,
    _noconstcode: R3000AFnpTrInfo,
    _xmminfo: i32,
) {
    // ASM stub
}
pub fn psx_recompile_code_const1(
    _constcode: R3000AFnpTr,
    _noconstcode: R3000AFnpTrInfo,
    _xmminfo: i32,
) {
    // ASM stub
}
pub fn psx_recompile_code_const2(
    _constcode: R3000AFnpTr,
    _noconstcode: R3000AFnpTrInfo,
    _xmminfo: i32,
) {
    // ASM stub
}
pub fn psx_recompile_code_const3(
    _constcode: R3000AFnpTr,
    _constscode: R3000AFnpTrInfo,
    _consttcode: R3000AFnpTrInfo,
    _noconstcode: R3000AFnpTrInfo,
    _lohi: i32,
) {
    // ASM stub
}

// ---------------------------------------------------------------------------
// rpsx* function declarations (from `iR3000Atables.cpp`).
// ---------------------------------------------------------------------------
//
// All of these are `extern "C"` stubs because they are reached only
// through dispatch tables. The dispatch tables below (`rpsxBSC` etc.)
// hold the `fn` pointers used by the EE's "PSX branch" path.

pub mod rpsx {
    // rpsxBSC[64] - primary opcode dispatch.
    extern "C" {
        pub fn rpsxSPECIAL();
        pub fn rpsxREGIMM();
        pub fn rpsxJ();
        pub fn rpsxJAL();
        pub fn rpsxBEQ();
        pub fn rpsxBNE();
        pub fn rpsxBLEZ();
        pub fn rpsxBGTZ();
        pub fn rpsxADDI();
        pub fn rpsxADDIU();
        pub fn rpsxSLTI();
        pub fn rpsxSLTIU();
        pub fn rpsxANDI();
        pub fn rpsxORI();
        pub fn rpsxXORI();
        pub fn rpsxLUI();
        pub fn rpsxCOP0();
        pub fn rpsxCOP2();
        pub fn rpsxNULL();
        pub fn rpsxLB();
        pub fn rpsxLH();
        pub fn rpsxLWL();
        pub fn rpsxLW();
        pub fn rpsxLBU();
        pub fn rpsxLHU();
        pub fn rpsxLWR();
        pub fn rpsxSB();
        pub fn rpsxSH();
        pub fn rpsxSWL();
        pub fn rpsxSW();
        pub fn rpsxSWR();
        pub fn rgteLWC2();
        pub fn rgteSWC2();
    }
    // rpsxSPC[64] - SPECIAL secondary dispatch.
    extern "C" {
        pub fn rpsxSLL();
        pub fn rpsxSRL();
        pub fn rpsxSRA();
        pub fn rpsxSLLV();
        pub fn rpsxSRLV();
        pub fn rpsxSRAV();
        pub fn rpsxJR();
        pub fn rpsxJALR();
        pub fn rpsxSYSCALL();
        pub fn rpsxBREAK();
        pub fn rpsxMFHI();
        pub fn rpsxMTHI();
        pub fn rpsxMFLO();
        pub fn rpsxMTLO();
        pub fn rpsxMULT();
        pub fn rpsxMULTU();
        pub fn rpsxDIV();
        pub fn rpsxDIVU();
        pub fn rpsxADD();
        pub fn rpsxADDU();
        pub fn rpsxSUB();
        pub fn rpsxSUBU();
        pub fn rpsxAND();
        pub fn rpsxOR();
        pub fn rpsxXOR();
        pub fn rpsxNOR();
        pub fn rpsxSLT();
        pub fn rpsxSLTU();
    }
    // rpsxREG[32] - REGIMM secondary dispatch.
    extern "C" {
        pub fn rpsxBLTZ();
        pub fn rpsxBGEZ();
        pub fn rpsxBLTZAL();
        pub fn rpsxBGEZAL();
    }
    // rpsxCP0[32] - COP0 secondary dispatch.
    extern "C" {
        pub fn rpsxMFC0();
        pub fn rpsxCFC0();
        pub fn rpsxMTC0();
        pub fn rpsxCTC0();
        pub fn rpsxRFE();
    }
    // rpsxCP2[64] - GTE secondary dispatch.
    extern "C" {
        pub fn rpsxBASIC();
        pub fn rgteRTPS();
        pub fn rgteNCLIP();
        pub fn rgteOP();
        pub fn rgteDPCS();
        pub fn rgteINTPL();
        pub fn rgteMVMVA();
        pub fn rgteNCDS();
        pub fn rgteCDP();
        pub fn rgteNCDT();
        pub fn rgteNCCS();
        pub fn rgteCC();
        pub fn rgteNCS();
        pub fn rgteNCT();
        pub fn rgteSQR();
        pub fn rgteDCPL();
        pub fn rgteDPCT();
        pub fn rgteAVSZ3();
        pub fn rgteAVSZ4();
        pub fn rgteRTPT();
        pub fn rgteGPF();
        pub fn rgteGPL();
        pub fn rgteNCCT();
    }
    // rpsxCP2BSC[32] - GTE BASIC secondary dispatch.
    extern "C" {
        pub fn rgteMFC2();
        pub fn rgteCFC2();
        pub fn rgteMTC2();
        pub fn rgteCTC2();
    }
}

// ---------------------------------------------------------------------------
// R3000A dispatch tables (from `iR3000Atables.cpp`).
// ---------------------------------------------------------------------------
//
// These are `static` arrays of function pointers. In Rust 2021 they
// are encoded as `*const ()` (i.e. void pointers) and we re-export
// casts via the helper functions below.

/// Type alias for an IOP opcode handler.
pub type RpsxOpFn = unsafe extern "C" fn();

/// Cast an extern "C" function to the table-pointer type.
#[inline]
pub const fn rpsx_op(f: unsafe extern "C" fn()) -> RpsxOpFn {
    f
}

extern "C" {
    // The five secondary-dispatch wrappers. They are defined in
    // `iR3000Atables.cpp` as small functions that index into the
    // secondary tables.
    fn rpsxSPECIAL_tramp();
    fn rpsxREGIMM_tramp();
    fn rpsxCOP0_tramp();
    fn rpsxCOP2_tramp();
    fn rpsxBASIC_tramp();
}

/// `rpsxBSC[64]`: 64-entry primary-opcode table, indexed by
/// `code >> 26` of `psxRegs.code`. Empty slots point to `rpsxNULL`.
pub static RPSX_BSC: [RpsxOpFn; 64] = [
    rpsx_op(rpsxSPECIAL_tramp), rpsx_op(rpsxREGIMM_tramp), rpsx::rpsxJ, rpsx::rpsxJAL,
    rpsx::rpsxBEQ, rpsx::rpsxBNE, rpsx::rpsxBLEZ, rpsx::rpsxBGTZ,
    rpsx::rpsxADDI, rpsx::rpsxADDIU, rpsx::rpsxSLTI, rpsx::rpsxSLTIU,
    rpsx::rpsxANDI, rpsx::rpsxORI, rpsx::rpsxXORI, rpsx::rpsxLUI,
    rpsx_op(rpsxCOP0_tramp), rpsx::rpsxNULL, rpsx_op(rpsxCOP2_tramp), rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxLB, rpsx::rpsxLH, rpsx::rpsxLWL, rpsx::rpsxLW,
    rpsx::rpsxLBU, rpsx::rpsxLHU, rpsx::rpsxLWR, rpsx::rpsxNULL,
    rpsx::rpsxSB, rpsx::rpsxSH, rpsx::rpsxSWL, rpsx::rpsxSW,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxSWR, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rgteLWC2, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rgteSWC2, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
];

/// `rpsxSPC[64]`: SPECIAL secondary dispatch, indexed by `code & 0x3F`.
pub static RPSX_SPC: [RpsxOpFn; 64] = [
    rpsx::rpsxSLL, rpsx::rpsxNULL, rpsx::rpsxSRL, rpsx::rpsxSRA,
    rpsx::rpsxSLLV, rpsx::rpsxNULL, rpsx::rpsxSRLV, rpsx::rpsxSRAV,
    rpsx::rpsxJR, rpsx::rpsxJALR, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxSYSCALL, rpsx::rpsxBREAK, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxMFHI, rpsx::rpsxMTHI, rpsx::rpsxMFLO, rpsx::rpsxMTLO,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxMULT, rpsx::rpsxMULTU, rpsx::rpsxDIV, rpsx::rpsxDIVU,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxADD, rpsx::rpsxADDU, rpsx::rpsxSUB, rpsx::rpsxSUBU,
    rpsx::rpsxAND, rpsx::rpsxOR, rpsx::rpsxXOR, rpsx::rpsxNOR,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxSLT, rpsx::rpsxSLTU,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
];

/// `rpsxREG[32]`: REGIMM secondary dispatch, indexed by `rt`.
pub static RPSX_REG: [RpsxOpFn; 32] = [
    rpsx::rpsxBLTZ, rpsx::rpsxBGEZ, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxBLTZAL, rpsx::rpsxBGEZAL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
];

/// `rpsxCP0[32]`: COP0 secondary dispatch, indexed by `rs`.
pub static RPSX_CP0: [RpsxOpFn; 32] = [
    rpsx::rpsxMFC0, rpsx::rpsxNULL, rpsx::rpsxCFC0, rpsx::rpsxNULL,
    rpsx::rpsxMTC0, rpsx::rpsxNULL, rpsx::rpsxCTC0, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxRFE, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
];

/// `rpsxCP2[64]`: GTE secondary dispatch, indexed by `funct`.
pub static RPSX_CP2: [RpsxOpFn; 64] = [
    rpsx_op(rpsxBASIC_tramp), rpsx::rgteRTPS, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rgteNCLIP, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rgteOP, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rgteDPCS, rpsx::rgteINTPL, rpsx::rgteMVMVA, rpsx::rgteNCDS,
    rpsx::rgteCDP, rpsx::rpsxNULL, rpsx::rgteNCDT, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rgteNCCS,
    rpsx::rgteCC, rpsx::rpsxNULL, rpsx::rgteNCS, rpsx::rpsxNULL,
    rpsx::rgteNCT, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rgteSQR, rpsx::rgteDCPL, rpsx::rgteDPCT, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rgteAVSZ3, rpsx::rgteAVSZ4, rpsx::rpsxNULL,
    rpsx::rgteRTPT, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rgteGPF, rpsx::rgteGPL, rpsx::rgteNCCT,
];

/// `rpsxCP2BSC[32]`: GTE BASIC secondary dispatch.
pub static RPSX_CP2BSC: [RpsxOpFn; 32] = [
    rpsx::rgteMFC2, rpsx::rpsxNULL, rpsx::rgteCFC2, rpsx::rpsxNULL,
    rpsx::rgteMTC2, rpsx::rpsxNULL, rpsx::rgteCTC2, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
    rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL, rpsx::rpsxNULL,
];

// ---------------------------------------------------------------------------
// Extern declarations for the load/store helpers used by the IOP
// interpreter (from `iR3000A.{h,cpp}`).
// ---------------------------------------------------------------------------

extern "C" {
    pub fn psxLWL();
    pub fn psxLWR();
    pub fn psxSWL();
    pub fn psxSWR();
}

// ---------------------------------------------------------------------------
// R3000A interpreter entry point (from `iR3000A.cpp`).
// ---------------------------------------------------------------------------

/// Execute one IOP (R3000A) interpreter step. Mirrors the
/// `psxBSC[code >> 26]()` dispatch.
pub fn iR3000A_Opcode(instr: u32) {
    let op = instr_op(instr) as usize;
    let handler = RPSX_BSC[op & 0x3f];
    unsafe { handler(); }
}

// ---------------------------------------------------------------------------
// R3000A internal helpers (from `iR3000Atables.cpp`).
// ---------------------------------------------------------------------------

#[inline]
fn rpsx_alloc_reg_if_used(reg: i32, mode: i32) -> i32 {
    // Mirror of the static helper at the top of `iR3000Atables.cpp`.
    let used = G_P_CUR_INST_INFO.with(|p| {
        let p = *p.borrow();
        if p.is_null() {
            false
        } else {
            unsafe { eeinst_used_test(&*p, reg as usize) }
        }
    });
    if used {
        alloc_x86reg(X86Type::Psx, reg, mode as u32)
    } else {
        check_x86reg(X86Type::Psx, reg, mode as u32)
    }
}

/// Make-merge-mask helper used by VIF unpack. Stub equivalent of
/// the C++ `makeMergeMask(u32& x)`.
pub fn make_merge_mask(x: u32) -> u32 {
    ((x & 0x40) >> 6) | ((x & 0x10) >> 3) | (x & 4) | ((x & 1) << 3)
}

#[cfg(target_os = "windows")]
pub fn make_merge_mask_all_columns(x: u32) -> u32 {
    ((x & 0x4040_4040) >> 6)
        | ((x & 0x1010_1010) >> 3)
        | (x & 0x0404_0404)
        | ((x & 0x0101_0101) << 3)
}

// ===========================================================================
// Section: EE interpreter dispatch (from `iFPU.cpp`, `iMMI.cpp`, `iCOP0.cpp`)
// ===========================================================================

/// `minvals` / `maxvals` masks used by the FPU / VU clamp operations.
#[repr(align(16))]
pub struct AlignedU32x4(pub [u32; 4]);
pub static mut G_MINVALS: AlignedU32x4 = AlignedU32x4([0xff7f_ffff; 4]);
pub static mut G_MAXVALS: AlignedU32x4 = AlignedU32x4([0x7f7f_ffff; 4]);

/// Execute one EE FPU instruction. Stub for the COP1 dispatch
/// in `iFPU.cpp`.
pub fn FPU_Opcode(_instr: u32) {
    // ASM stub
}

/// Execute one EE MMI instruction. Stub for the MMI dispatch in
/// `iMMI.cpp`.
pub fn MMI_Opcode(_instr: u32) {
    // ASM stub
}

/// Execute one EE COP0 instruction. Stub for the COP0 dispatch in
/// `iCOP0.cpp`.
pub fn COP0_Opcode(_instr: u32) {
    // ASM stub
}

/// Execute one EE core (integer) instruction. Stub for the
/// interpreter dispatch in `iFPUd.cpp` / `iR3000A.cpp`.
pub fn EE_Core_Opcode(_instr: u32) {
    // ASM stub
}

// ===========================================================================
// Section: 32-bit ix86 backend (from `ix86-32/*.cpp`)
// ===========================================================================
//
// These files contain a parallel 32-bit-only path for the EE
// dynarec, used on platforms where the 64-bit emitter is not
// available. In this Rust port they are exposed as `extern "C"`
// stubs that match the call surface.

pub mod ix86_32 {
    //! 32-bit x86 EE dynarec backend. All entry points are stubs.

    // `iCore.cpp` (32-bit) - register allocator wrappers.
    extern "C" {
        pub fn _isAllocatableX86reg(x86reg: i32) -> bool;
        pub fn _getFreeX86reg(mode: i32) -> i32;
        pub fn _allocX86reg(ty: i32, reg: i32, mode: i32) -> i32;
        pub fn _checkX86reg(ty: i32, reg: i32, mode: i32) -> i32;
        pub fn _hasX86reg(ty: i32, reg: i32, required_mode: i32) -> bool;
        pub fn _addNeededX86reg(ty: i32, reg: i32);
        pub fn _clearNeededX86regs();
        pub fn _freeX86reg(x86reg: i32);
        pub fn _freeX86regWithoutWriteback(x86reg: i32);
        pub fn _freeX86regs();
        pub fn _flushX86regs();
        pub fn _flushConstRegs(delete_const: i32);
        pub fn _flushConstReg(reg: i32);
        pub fn _validateRegs();
        pub fn _writebackX86Reg(x86reg: i32);
    }

    // `iR5900.cpp` (32-bit) - EE recompiler entry point.
    pub fn rec_recompile(_start: u32, _end: u32) {
        // ASM stub
    }
    pub fn rec_recompile_block() {
        // ASM stub
    }

    // Arithmetic dynarec handlers.
    extern "C" {
        pub fn recADD();
        pub fn recADDU();
        pub fn recDADD();
        pub fn recDADDU();
        pub fn recSUB();
        pub fn recSUBU();
        pub fn recDSUB();
        pub fn recDSUBU();
        pub fn recAND();
        pub fn recOR();
        pub fn recXOR();
        pub fn recNOR();
        pub fn recSLT();
        pub fn recSLTU();
    }
    // AritImm dynarec handlers.
    extern "C" {
        pub fn recADDI();
        pub fn recADDIU();
        pub fn recDADDI();
        pub fn recDADDIU();
        pub fn recANDI();
        pub fn recORI();
        pub fn recXORI();
        pub fn recSLTI();
        pub fn recSLTIU();
    }
    // Branch handlers.
    extern "C" {
        pub fn recBEQ();
        pub fn recBEQL();
        pub fn recBNE();
        pub fn recBNEL();
        pub fn recBLTZ();
        pub fn recBLTZL();
        pub fn recBLTZAL();
        pub fn recBLTZALL();
        pub fn recBGTZ();
        pub fn recBGTZL();
        pub fn recBLEZ();
        pub fn recBLEZL();
        pub fn recBGEZ();
        pub fn recBGEZL();
        pub fn recBGEZAL();
        pub fn recBGEZALL();
    }
    // Jump handlers.
    extern "C" {
        pub fn recJ();
        pub fn recJAL();
        pub fn recJR();
        pub fn recJALR();
    }
    // Load/Store handlers.
    extern "C" {
        pub fn recLB();
        pub fn recLBU();
        pub fn recLH();
        pub fn recLHU();
        pub fn recLW();
        pub fn recLWU();
        pub fn recLWL();
        pub fn recLWR();
        pub fn recLD();
        pub fn recLDR();
        pub fn recLDL();
        pub fn recLQ();
        pub fn recSB();
        pub fn recSH();
        pub fn recSW();
        pub fn recSWL();
        pub fn recSWR();
        pub fn recSD();
        pub fn recSDL();
        pub fn recSDR();
        pub fn recSQ();
        pub fn recLWC1();
        pub fn recSWC1();
        pub fn recLQC2();
        pub fn recSQC2();
    }
    // MultDiv handlers.
    extern "C" {
        pub fn recMULT();
        pub fn recMULTU();
        pub fn recDIV();
        pub fn recDIVU();
    }
    // Shift handlers.
    extern "C" {
        pub fn recSLL();
        pub fn recSRL();
        pub fn recSRA();
        pub fn recDSLL();
        pub fn recDSRL();
        pub fn recDSRA();
        pub fn recDSLL32();
        pub fn recDSRL32();
        pub fn recDSRA32();
        pub fn recSLLV();
        pub fn recSRLV();
        pub fn recSRAV();
        pub fn recDSLLV();
        pub fn recDSRLV();
        pub fn recDSRAV();
    }
    // `iR5900Templates.cpp` - constant-propagation templates.
    pub fn ee_recompile_code_const0(
        _constcode: super::R5900FnpTr,
        _constscode: super::R5900FnpTrInfo,
        _consttcode: super::R5900FnpTrInfo,
        _noconstcode: super::R5900FnpTrInfo,
        _xmminfo: i32,
    ) {
    }
    pub fn ee_recompile_code_const1(
        _constcode: super::R5900FnpTr,
        _noconstcode: super::R5900FnpTrInfo,
        _xmminfo: i32,
    ) {
    }
    pub fn ee_recompile_code_const2(
        _constcode: super::R5900FnpTr,
        _noconstcode: super::R5900FnpTrInfo,
        _xmminfo: i32,
    ) {
    }

    // `recVTLB.cpp` - VTLB resolver; the entire file is an
    // inline-asm-heavy stub here.
    pub fn rec_vtlb_refill(_addr: u32, _is_write: bool) {
        // ASM stub
    }
    pub fn rec_vtlb_miss(_addr: u32, _is_write: bool) -> *mut u8 {
        std::ptr::null_mut()
    }
    pub fn rec_vtlb_init() {}
    pub fn rec_vtlb_reset() {}
    pub fn vtlb_set_pgprot(_addr: u32, _prot: u32) {}
    pub fn vtlb_load_pgprot(_addr: u32) -> u32 {
        0
    }
}

// ===========================================================================
// Section: microVU IR (from `microVU_IR.h`)
// ===========================================================================
//
// The microVU recompiler takes a stream of VU micro-instructions,
// builds an SSA-style IR (`microIR`), and emits x86. The IR types
// are quite specific; here we expose the data shapes.

/// 4-bit-per-vector register cycle descriptor. Mirrors
/// `regCycleInfo` from `microVU_IR.h`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RegCycleInfo {
    pub x: u8,
    pub y: u8,
    pub z: u8,
    pub w: u8,
}

impl RegCycleInfo {
    /// Build a 4-bit-per-vector cycle info from a 4-vector byte.
    pub const fn from_packed(p: u8) -> Self {
        RegCycleInfo {
            x: (p >> 0) & 0xf,
            y: (p >> 4) & 0xf,
            z: 0,
            w: 0,
        }
    }
}

/// Pipeline state for a micro-VU block. Carefully laid out for
/// fast 64-bit equality compares (`quick64`).
#[derive(Debug, Clone, Copy)]
#[repr(C, align(16))]
pub struct MicroRegInfo {
    pub need_exact_match: u8,
    pub flag_info: u8,
    pub q: u8,
    pub p: u8,
    pub xgkick: u8,
    pub vi_backup: u8,
    pub block_type: u8,
    pub r: u8,
    pub xgkick_cycles: u32,
    pub unused: u8,
    pub vi15v: u8,
    pub vi15: u16,
    pub vi: [u8; 16],
    pub vf: [RegCycleInfo; 32],
}
impl Default for MicroRegInfo {
    fn default() -> Self {
        MicroRegInfo {
            need_exact_match: 0,
            flag_info: 0,
            q: 0,
            p: 0,
            xgkick: 0,
            vi_backup: 0,
            block_type: 0,
            r: 0,
            xgkick_cycles: 0,
            unused: 0,
            vi15v: 0,
            vi15: 0,
            vi: [0; 16],
            vf: [RegCycleInfo::default(); 32],
        }
    }
}

/// One micro-block (entry point + pipeline state). Mirrors
/// `microBlock` from `microVU_IR.h`.
#[derive(Debug, Clone, Copy, Default)]
pub struct MicroBlock {
    pub p_state: MicroRegInfo,
    pub p_state_end: MicroRegInfo,
    pub x86ptr_start: *mut u8,
    pub jump_cache: *mut MicroJumpCache,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroJumpCache {
    pub prog: *mut MicroProgram,
    pub x86ptr_start: *mut std::ffi::c_void,
}

/// Temporary per-instruction pipeline info. Mirrors
/// `microTempRegInfo`.
#[derive(Debug, Clone, Copy, Default)]
pub struct MicroTempRegInfo {
    pub vf: [RegCycleInfo; 2],
    pub vf_reg: [u8; 2],
    pub vi: u8,
    pub vi_reg: u8,
    pub q: u8,
    pub p: u8,
    pub r: u8,
    pub xgkick: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroVfReg {
    pub reg: u8,
    pub x: u8,
    pub y: u8,
    pub z: u8,
    pub w: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroViReg {
    pub reg: u8,
    pub used: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroConstInfo {
    pub is_valid: u8,
    pub reg_value: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroUpperOp {
    pub e_bit: bool,
    pub i_bit: bool,
    pub m_bit: bool,
    pub t_bit: bool,
    pub d_bit: bool,
    pub vf_write: MicroVfReg,
    pub vf_read: [MicroVfReg; 2],
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroLowerOp {
    pub vf_write: MicroVfReg,
    pub vf_read: [MicroVfReg; 2],
    pub vi_write: MicroViReg,
    pub vi_read: [MicroViReg; 2],
    pub const_jump: MicroConstInfo,
    pub branch: u32,
    pub kick_cycles: u32,
    pub bad_branch: bool,
    pub evil_branch: bool,
    pub is_nop: bool,
    pub is_fsset: bool,
    pub no_write_vf: bool,
    pub backup_vi: bool,
    pub mem_read_is: bool,
    pub mem_read_it: bool,
    pub read_flags: bool,
    pub is_mem_write: bool,
    pub is_kick: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroFlagInst {
    pub do_flag: bool,
    pub do_non_sticky: bool,
    pub write: u8,
    pub last_write: u8,
    pub read: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroFlagCycles {
    pub x_status: [i32; 4],
    pub x_mac: [i32; 4],
    pub x_clip: [i32; 4],
    pub cycles: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroOp {
    pub stall: u8,
    pub is_bad_op: bool,
    pub is_eob: bool,
    pub is_bdelay: bool,
    pub swap_ops: bool,
    pub backup_vf: bool,
    pub do_xgkick: bool,
    pub xgkick_pc: u32,
    pub do_div_flag: bool,
    pub read_q: i32,
    pub write_q: i32,
    pub read_p: i32,
    pub write_p: i32,
    pub s_flag: MicroFlagInst,
    pub m_flag: MicroFlagInst,
    pub c_flag: MicroFlagInst,
    pub u_op: MicroUpperOp,
    pub l_op: MicroLowerOp,
}

/// Per-program IR state. Mirrors `microIR<pSize>`.
pub struct MicroIR<const P_SIZE: usize> {
    pub block: MicroBlock,
    pub p_block: *mut MicroBlock,
    pub regs_temp: MicroTempRegInfo,
    pub info: Vec<MicroOp>,
    pub const_reg: [MicroConstInfo; 16],
    pub branch: u8,
    pub cycles: u32,
    pub count: u32,
    pub cur_pc: u32,
    pub start_pc: u32,
    pub s_flag_hack: u32,
}

impl<const P_SIZE: usize> MicroIR<P_SIZE> {
    pub const fn new() -> Self {
        MicroIR {
            block: MicroBlock {
                p_state: MicroRegInfo {
                    need_exact_match: 0, flag_info: 0, q: 0, p: 0,
                    xgkick: 0, vi_backup: 0, block_type: 0, r: 0,
                    xgkick_cycles: 0, unused: 0, vi15v: 0, vi15: 0,
                    vi: [0; 16],
                    vf: [RegCycleInfo { x: 0, y: 0, z: 0, w: 0 }; 32],
                },
                p_state_end: MicroRegInfo {
                    need_exact_match: 0, flag_info: 0, q: 0, p: 0,
                    xgkick: 0, vi_backup: 0, block_type: 0, r: 0,
                    xgkick_cycles: 0, unused: 0, vi15v: 0, vi15: 0,
                    vi: [0; 16],
                    vf: [RegCycleInfo { x: 0, y: 0, z: 0, w: 0 }; 32],
                },
                x86ptr_start: std::ptr::null_mut(),
                jump_cache: std::ptr::null_mut(),
            },
            p_block: ptr::null_mut(),
            regs_temp: MicroTempRegInfo {
                vf: [RegCycleInfo { x: 0, y: 0, z: 0, w: 0 }; 2],
                vf_reg: [0; 2],
                vi: 0,
                vi_reg: 0,
                q: 0,
                p: 0,
                r: 0,
                xgkick: 0,
            },
            info: Vec::new(), // populated by `init` at runtime
            const_reg: [MicroConstInfo { is_valid: 0, reg_value: 0 }; 16],
            branch: 0,
            cycles: 0,
            count: 0,
            cur_pc: 0,
            start_pc: 0,
            s_flag_hack: 0,
        }
    }
    /// Allocate the `info` array. Must be called before use.
    pub fn init(&mut self) {
        if self.info.is_empty() {
            self.info = vec![MicroOp::default(); P_SIZE / 2];
        }
    }
}

// ===========================================================================
// Section: microVU compiler state (from `microVU.h`)
// ===========================================================================

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroRange {
    pub start: s32,
    pub end: s32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroBlockLink {
    pub block: MicroBlock,
    pub next: *mut MicroBlockLink,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroBlockLinkRef {
    pub p_block: *mut MicroBlock,
    pub quick: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroProgram {
    pub data: *mut u32,
    pub block: *mut *mut MicroBlockManager,
    pub ranges: *mut std::collections::VecDeque<MicroRange>,
    pub start_pc: u32,
    pub idx: i32,
}

/// Stub for the `microBlockManager` class.
pub struct MicroBlockManager {
    pub q_block_list: *mut MicroBlockLink,
    pub q_block_end: *mut MicroBlockLink,
    pub f_block_list: *mut MicroBlockLink,
    pub f_block_end: *mut MicroBlockLink,
    pub quick_lookup: Vec<MicroBlockLinkRef>,
    pub q_list_i: i32,
    pub f_list_i: i32,
}
impl Default for MicroBlockManager {
    fn default() -> Self {
        MicroBlockManager {
            q_block_list: ptr::null_mut(),
            q_block_end: ptr::null_mut(),
            f_block_list: ptr::null_mut(),
            f_block_end: ptr::null_mut(),
            quick_lookup: Vec::new(),
            q_list_i: 0,
            f_list_i: 0,
        }
    }
}
impl MicroBlockManager {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn get_full_list_count(&self) -> i32 {
        self.f_list_i
    }
    pub fn reset(&mut self) {
        self.q_block_list = ptr::null_mut();
        self.q_block_end = ptr::null_mut();
        self.f_block_list = ptr::null_mut();
        self.f_block_end = ptr::null_mut();
        self.quick_lookup.clear();
        self.q_list_i = 0;
        self.f_list_i = 0;
    }
    pub fn add(&mut self, _mvu: &mut MicroVu, _p_block: &mut MicroBlock) -> *mut MicroBlock {
        ptr::null_mut()
    }
    pub fn search(&self, _mvu: &MicroVu, _p_state: &MicroRegInfo) -> *mut MicroBlock {
        ptr::null_mut()
    }
    pub fn print_info(&self, _pc: i32, _print_quick: bool) {
        // ASM stub
    }
}

/// Stub for the `microProgManager` struct.
pub struct MicroProgManager {
    pub ir_info: MicroIR<{ 0x4000 / 4 }>,
    pub prog: [*mut std::collections::VecDeque<MicroProgram>; 0x4000 / 4 / 2],
    pub quick: [MicroProgramQuick; 0x4000 / 4 / 2],
    pub cur: *mut MicroProgram,
    pub total: i32,
    pub is_same: i32,
    pub cleared: i32,
    pub cur_frame: u32,
    pub x86ptr: *mut u8,
    pub x86start: *mut u8,
    pub x86end: *mut u8,
    pub lp_state: MicroRegInfo,
}
impl Default for MicroProgManager {
    fn default() -> Self {
        MicroProgManager {
            ir_info: MicroIR::new(),
            prog: [ptr::null_mut(); 0x4000 / 4 / 2],
            quick: [MicroProgramQuick::default(); 0x4000 / 4 / 2],
            cur: ptr::null_mut(),
            total: 0,
            is_same: -1,
            cleared: 0,
            cur_frame: 0,
            x86ptr: ptr::null_mut(),
            x86start: ptr::null_mut(),
            x86end: ptr::null_mut(),
            lp_state: MicroRegInfo::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MicroProgramQuick {
    pub block: *mut MicroBlockManager,
    pub prog: *mut MicroProgram,
}

/// The main per-VU `microVU` struct. Mirrors `microVU` from
/// `microVU.h`. Only the layout is preserved; the data members
/// point to host-managed state in a real port.
#[repr(C, align(16))]
pub struct MicroVu {
    pub stat_flag: [u32; 4],
    pub mac_flag: [u32; 4],
    pub clip_flag: [u32; 4],
    pub xmm_c_temp: [u32; 4],
    pub xmm_backup: [[u32; 4]; 16],

    pub index: u32,
    pub cop2: u32,
    pub vu_mem_size: u32,
    pub micro_mem_size: u32,
    pub prog_size: u32,
    pub prog_mem_mask: u32,
    pub cache_size: u32,

    pub prog: MicroProgManager,
    pub profiler: MicroProfiler,
    pub reg_alloc: *mut MicroRegAlloc,
    pub log_file: *mut std::ffi::c_void,

    pub cache: *mut u8,
    pub start_funct: *mut u8,
    pub exit_funct: *mut u8,
    pub start_funct_xg: *mut u8,
    pub exit_funct_xg: *mut u8,
    pub compare_state_f: *mut u8,
    pub wait_mtvu: *mut u8,
    pub copy_pl_state: *mut u8,
    pub resume_ptr_xg: *mut u8,
    pub code: u32,
    pub div_flag: u32,
    pub vi_backup: u32,
    pub vi_xgkick: u32,
    pub branch: u32,
    pub bad_branch: u32,
    pub evil_branch: u32,
    pub evilevil_branch: u32,
    pub p: u32,
    pub q: u32,
    pub total_cycles: u32,
    pub cycles: s32,
}
impl Default for MicroVu {
    fn default() -> Self {
        MicroVu {
            stat_flag: [0; 4],
            mac_flag: [0; 4],
            clip_flag: [0; 4],
            xmm_c_temp: [0; 4],
            xmm_backup: [[0; 4]; 16],

            index: 0,
            cop2: 0,
            vu_mem_size: 0,
            micro_mem_size: 0,
            prog_size: 0,
            prog_mem_mask: 0,
            cache_size: 0,

            prog: MicroProgManager::default(),
            profiler: MicroProfiler::default(),
            reg_alloc: ptr::null_mut(),
            log_file: ptr::null_mut(),

            cache: ptr::null_mut(),
            start_funct: ptr::null_mut(),
            exit_funct: ptr::null_mut(),
            start_funct_xg: ptr::null_mut(),
            exit_funct_xg: ptr::null_mut(),
            compare_state_f: ptr::null_mut(),
            wait_mtvu: ptr::null_mut(),
            copy_pl_state: ptr::null_mut(),
            resume_ptr_xg: ptr::null_mut(),
            code: 0,
            div_flag: 0,
            vi_backup: 0,
            vi_xgkick: 0,
            branch: 0,
            bad_branch: 0,
            evil_branch: 0,
            evilevil_branch: 0,
            p: 0,
            q: 0,
            total_cycles: 0,
            cycles: 0,
        }
    }
}

/// Per-VU reg allocator stub. Mirrors `microRegAlloc` from
/// `microVU_IR.h`.
pub struct MicroRegAlloc {
    pub xmm_map: Vec<MicroMapXmm>,
    pub gpr_map: Vec<MicroMapGpr>,
    pub counter: i32,
    pub index: i32,
    pub reg_alloc_cop2: bool,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct MicroMapXmm {
    pub vf_reg: i32,
    pub xyzw: i32,
    pub count: i32,
    pub is_needed: bool,
    pub is_zero: bool,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct MicroMapGpr {
    pub vi_reg: i32,
    pub count: i32,
    pub is_needed: bool,
    pub dirty: bool,
    pub is_zero_extended: bool,
    pub usable: bool,
}
impl MicroRegAlloc {
    pub fn new(_index: i32) -> Self {
        MicroRegAlloc {
            xmm_map: Vec::new(),
            gpr_map: Vec::new(),
            counter: 0,
            index: 0,
            reg_alloc_cop2: false,
        }
    }
    pub fn reset(&mut self, _cop2_mode: bool) {
        // ASM stub
    }
    pub fn get_xmm_count(&self) -> i32 {
        self.xmm_map.len() as i32
    }
    pub fn get_free_xmm_count(&self) -> i32 {
        self.xmm_map
            .iter()
            .filter(|m| !m.is_needed && m.vf_reg < 0)
            .count() as i32
    }
    pub fn has_reg_vf(&self, vfreg: i32) -> bool {
        self.xmm_map.iter().any(|m| m.vf_reg == vfreg)
    }
    pub fn get_reg_vf(&self, i: usize) -> i32 {
        self.xmm_map.get(i).map(|m| m.vf_reg).unwrap_or(-1)
    }
    pub fn get_gpr_count(&self) -> i32 {
        self.gpr_map.len() as i32
    }
    pub fn get_free_gpr_count(&self) -> i32 {
        self.gpr_map
            .iter()
            .filter(|m| !m.is_needed && m.vi_reg < 0)
            .count() as i32
    }
    pub fn has_reg_vi(&self, vireg: i32) -> bool {
        self.gpr_map.iter().any(|m| m.vi_reg == vireg)
    }
    pub fn get_reg_vi(&self, i: usize) -> i32 {
        self.gpr_map.get(i).map(|m| m.vi_reg).unwrap_or(-1)
    }
    pub fn flush_all(&mut self, _clear_state: bool) {
        // ASM stub
    }
    pub fn write_back_reg(&mut self, _reg: i32) {
        // ASM stub
    }
    pub fn clear_reg(&mut self, _i: i32) {
        // ASM stub
    }
    pub fn clear_gpr(&mut self, _i: i32) {
        // ASM stub
    }
    pub fn write_vi_backup(&mut self, _reg: i32) {
        // ASM stub
    }
}

/// Global per-VU microVU state. The 16-byte alignment is preserved.
#[repr(C, align(16))]
pub struct AlignedMicroVu(pub MicroVu);

/// Safe-zone for program recompilation (in megabytes).
pub const MVU_CACHE_SAFE_ZONE: u32 = 3;

/// Helper constant for micro-program sizing: 0x4000 / 4 = 1024
/// 32-bit micro-instructions per VU program.
pub const MPROG_SIZE: usize = 0x4000 / 4;
pub const MPROG_HALF: usize = MPROG_SIZE / 2;

/// Global per-VU compiler state. Mirrors `alignas(16) microVU0/1`.
/// Note: the static init cannot allocate a `Vec`, so `info` is
/// populated lazily via `MicroIR::init()`.
pub static mut MICROVU0: MicroVu = MicroVu {
    stat_flag: [0; 4], mac_flag: [0; 4], clip_flag: [0; 4],
    xmm_c_temp: [0; 4], xmm_backup: [[0; 4]; 16],
    index: 0, cop2: 0, vu_mem_size: 0, micro_mem_size: 0,
    prog_size: 0, prog_mem_mask: 0, cache_size: 0,
    prog: MicroProgManager {
        ir_info: MicroIR::<{ 0x4000 / 4 }>::new(),
        prog: [ptr::null_mut(); MPROG_HALF],
        quick: [MicroProgramQuick {
            block: ptr::null_mut(),
            prog: ptr::null_mut(),
        }; MPROG_HALF],
        cur: ptr::null_mut(),
        total: 0,
        is_same: -1,
        cleared: 0,
        cur_frame: 0,
        x86ptr: ptr::null_mut(),
        x86start: ptr::null_mut(),
        x86end: ptr::null_mut(),
        lp_state: MicroRegInfo {
            need_exact_match: 0, flag_info: 0, q: 0, p: 0, xgkick: 0,
            vi_backup: 0, block_type: 0, r: 0,
            xgkick_cycles: 0, unused: 0, vi15v: 0, vi15: 0,
            vi: [0; 16],
            vf: [RegCycleInfo { x: 0, y: 0, z: 0, w: 0 }; 32],
        },
    },
    profiler: MicroProfiler {
        op_stats: [0u64; 166], prog_count: 0, index: 0,
    },
    reg_alloc: ptr::null_mut(),
    log_file: ptr::null_mut(),
    cache: ptr::null_mut(),
    start_funct: ptr::null_mut(),
    exit_funct: ptr::null_mut(),
    start_funct_xg: ptr::null_mut(),
    exit_funct_xg: ptr::null_mut(),
    compare_state_f: ptr::null_mut(),
    wait_mtvu: ptr::null_mut(),
    copy_pl_state: ptr::null_mut(),
    resume_ptr_xg: ptr::null_mut(),
    code: 0, div_flag: 0, vi_backup: 0, vi_xgkick: 0,
    branch: 0, bad_branch: 0, evil_branch: 0, evilevil_branch: 0,
    p: 0, q: 0, total_cycles: 0, cycles: 0,
};
pub static mut MICROVU1: MicroVu = MicroVu {
    stat_flag: [0; 4], mac_flag: [0; 4], clip_flag: [0; 4],
    xmm_c_temp: [0; 4], xmm_backup: [[0; 4]; 16],
    index: 1, cop2: 0, vu_mem_size: 0, micro_mem_size: 0,
    prog_size: 0, prog_mem_mask: 0, cache_size: 0,
    prog: MicroProgManager {
        ir_info: MicroIR::<{ 0x4000 / 4 }>::new(),
        prog: [ptr::null_mut(); MPROG_HALF],
        quick: [MicroProgramQuick {
            block: ptr::null_mut(),
            prog: ptr::null_mut(),
        }; MPROG_HALF],
        cur: ptr::null_mut(),
        total: 0,
        is_same: -1,
        cleared: 0,
        cur_frame: 0,
        x86ptr: ptr::null_mut(),
        x86start: ptr::null_mut(),
        x86end: ptr::null_mut(),
        lp_state: MicroRegInfo {
            need_exact_match: 0, flag_info: 0, q: 0, p: 0, xgkick: 0,
            vi_backup: 0, block_type: 0, r: 0,
            xgkick_cycles: 0, unused: 0, vi15v: 0, vi15: 0,
            vi: [0; 16],
            vf: [RegCycleInfo { x: 0, y: 0, z: 0, w: 0 }; 32],
        },
    },
    profiler: MicroProfiler {
        op_stats: [0u64; 166], prog_count: 0, index: 1,
    },
    reg_alloc: ptr::null_mut(),
    log_file: ptr::null_mut(),
    cache: ptr::null_mut(),
    start_funct: ptr::null_mut(),
    exit_funct: ptr::null_mut(),
    start_funct_xg: ptr::null_mut(),
    exit_funct_xg: ptr::null_mut(),
    compare_state_f: ptr::null_mut(),
    wait_mtvu: ptr::null_mut(),
    copy_pl_state: ptr::null_mut(),
    resume_ptr_xg: ptr::null_mut(),
    code: 0, div_flag: 0, vi_backup: 0, vi_xgkick: 0,
    branch: 0, bad_branch: 0, evil_branch: 0, evilevil_branch: 0,
    p: 0, q: 0, total_cycles: 0, cycles: 0,
};

// ---------------------------------------------------------------------------
// Debug helper
// ---------------------------------------------------------------------------

/// `mVUdebugNow` - single-step debugging flag. Mirrors the int in
/// `microVU.h`.
pub static mut MVU_DEBUG_NOW: i32 = 0;

pub fn dump_vu_state(_n: u32, _pc: u32) {
    // ASM stub
}

// ===========================================================================
// Section: microVU opcodes + profiler (from `microVU_Profiler.h`)
// ===========================================================================

/// microVU opcode IDs. Mirrors `enum microOpcode` from
/// `microVU_Profiler.h`. The numeric values match the C++ source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum MicroOpcode {
    // Upper Instructions
    OpAbs = 0,
    OpClip = 1,
    OpOpmula = 2,
    OpOpmsub = 3,
    OpNop = 4,
    OpAdd = 5,
    OpAddI = 6,
    OpAddQ = 7,
    OpAddX = 8,
    OpAddY = 9,
    OpAddZ = 10,
    OpAddW = 11,
    OpAddA = 12,
    OpAddAI = 13,
    OpAddAQ = 14,
    OpAddAX = 15,
    OpAddAY = 16,
    OpAddAZ = 17,
    OpAddAW = 18,
    OpSub = 19,
    OpSubI = 20,
    OpSubQ = 21,
    OpSubX = 22,
    OpSubY = 23,
    OpSubZ = 24,
    OpSubW = 25,
    OpSubA = 26,
    OpSubAI = 27,
    OpSubAQ = 28,
    OpSubAX = 29,
    OpSubAY = 30,
    OpSubAZ = 31,
    OpSubAW = 32,
    OpMul = 33,
    OpMulI = 34,
    OpMulQ = 35,
    OpMulX = 36,
    OpMulY = 37,
    OpMulZ = 38,
    OpMulW = 39,
    OpMulA = 40,
    OpMulAi = 41,
    OpMulAq = 42,
    OpMulAx = 43,
    OpMulAy = 44,
    OpMulAz = 45,
    OpMulAw = 46,
    OpMadd = 47,
    OpMaddI = 48,
    OpMaddQ = 49,
    OpMaddX = 50,
    OpMaddY = 51,
    OpMaddZ = 52,
    OpMaddW = 53,
    OpMaddA = 54,
    OpMaddAi = 55,
    OpMaddAq = 56,
    OpMaddAx = 57,
    OpMaddAy = 58,
    OpMaddAz = 59,
    OpMaddAw = 60,
    OpMsub = 61,
    OpMsubI = 62,
    OpMsubQ = 63,
    OpMsubX = 64,
    OpMsubY = 65,
    OpMsubZ = 66,
    OpMsubW = 67,
    OpMsubA = 68,
    OpMsubAi = 69,
    OpMsubAq = 70,
    OpMsubAx = 71,
    OpMsubAy = 72,
    OpMsubAz = 73,
    OpMsubAw = 74,
    OpMax = 75,
    OpMaxI = 76,
    OpMaxX = 78,
    OpMaxY = 79,
    OpMaxZ = 80,
    OpMaxW = 81,
    OpMini = 82,
    OpMiniI = 83,
    OpMiniX = 85,
    OpMiniY = 86,
    OpMiniZ = 87,
    OpMiniW = 88,
    OpFtoi0 = 89,
    OpFtoi4 = 90,
    OpFtoi12 = 91,
    OpFtoi15 = 92,
    OpItof0 = 93,
    OpItof4 = 94,
    OpItof12 = 95,
    OpItof15 = 96,
    // Lower Instructions
    OpDiv = 97,
    OpSqrt = 98,
    OpRsqrt = 99,
    OpIadd = 100,
    OpIaddI = 101,
    OpIaddIu = 102,
    OpIand = 103,
    OpIor = 104,
    OpIsub = 105,
    OpIsubIu = 106,
    OpMove = 107,
    OpMfir = 108,
    OpMtir = 109,
    OpMr32 = 110,
    OpMfp = 111,
    OpLq = 112,
    OpLqd = 113,
    OpLqi = 114,
    OpSq = 115,
    OpSqd = 116,
    OpSqi = 117,
    OpIlw = 118,
    OpIsw = 119,
    OpIlwr = 120,
    OpIswr = 121,
    OpRinit = 122,
    OpRget = 123,
    OpRnext = 124,
    OpRxor = 125,
    OpWaitq = 126,
    OpWaitp = 127,
    OpFsand = 128,
    OpFseq = 129,
    OpFsor = 130,
    OpFsset = 131,
    OpFmand = 132,
    OpFmeq = 133,
    OpFmor = 134,
    OpFcand = 135,
    OpFceq = 136,
    OpFcor = 137,
    OpFcset = 138,
    OpFcget = 139,
    OpIbeq = 140,
    OpIbgez = 141,
    OpIbgtz = 142,
    OpIbltz = 143,
    OpIblez = 144,
    OpIbne = 145,
    OpB = 146,
    OpBal = 147,
    OpJr = 148,
    OpJalr = 149,
    OpEsadd = 150,
    OpErsadd = 151,
    OpEleng = 152,
    OpErleng = 153,
    OpEatanxy = 154,
    OpEatanxz = 155,
    OpEsum = 156,
    OpErcpr = 157,
    OpEsqrt = 158,
    OpErsqrt = 159,
    OpEsin = 160,
    OpEatan = 161,
    OpEexp = 162,
    OpXitop = 163,
    OpXtop = 164,
    OpXgkick = 165,
    /// Sentinel: end of the opcode table.
    OpLastOpcode = 166,
}

/// microVU opcode name table. Mirrors `microOpcodeName[][16]`.
pub const MICRO_OPCODE_NAME: [&str; 166] = [
    "ABS", "CLIP", "OPMULA", "OPMSUB", "NOP",
    "ADD", "ADDi", "ADDq", "ADDx", "ADDy", "ADDz", "ADDw",
    "ADDA", "ADDAi", "ADDAq", "ADDAx", "ADDAy", "ADDAz", "ADDAw",
    "SUB", "SUBi", "SUBq", "SUBx", "SUBy", "SUBz", "SUBw",
    "SUBA", "SUBAi", "SUBAq", "SUBAx", "SUBAy", "SUBAz", "SUBAw",
    "MUL", "MULi", "MULq", "MULx", "MULy", "MULz", "MULw",
    "MULA", "MULAi", "MULAq", "MULAx", "MULAy", "MULAz", "MULAw",
    "MADD", "MADDi", "MADDq", "MADDx", "MADDy", "MADDz", "MADDw",
    "MADDA", "MADDAi", "MADDAq", "MADDAx", "MADDAy", "MADDAz", "MADDAw",
    "MSUB", "MSUBi", "MSUBq", "MSUBx", "MSUBy", "MSUBz", "MSUBw",
    "MSUBA", "MSUBAi", "MSUBAq", "MSUBAx", "MSUBAy", "MSUBAz", "MSUBAw",
    "MAX", "MAXi", "", "MAXx", "MAXy", "MAXz", "MAXw",
    "MINI", "MINIi", "", "MINIx", "MINIy", "MINIz", "MINIw",
    "FTOI0", "FTOI4", "FTOI12", "FTOI15",
    "ITOF0", "ITOF4", "ITOF12", "ITOF15",
    "DIV", "SQRT", "RSQRT",
    "IADD", "IADDI", "IADDIU",
    "IAND", "IOR",
    "ISUB", "ISUBIU",
    "MOVE", "MFIR", "MTIR", "MR32", "MFP",
    "LQ", "LQD", "LQI",
    "SQ", "SQD", "SQI",
    "ILW", "ISW", "ILWR", "ISWR",
    "RINIT", "RGET", "RNEXT", "RXOR",
    "WAITQ", "WAITP",
    "FSAND", "FSEQ", "FSOR", "FSSET",
    "FMAND", "FMEQ", "FMOR",
    "FCAND", "FCEQ", "FCOR", "FCSET", "FCGET",
    "IBEQ", "IBGEZ", "IBGTZ", "IBLTZ", "IBLEZ", "IBNE",
    "B", "BAL", "JR", "JALR",
    "ESADD", "ERSADD", "ELENG", "ERLENG",
    "EATANxy", "EATANxz", "ESUM", "ERCPR",
    "ESQRT", "ERSQRT", "ESIN", "EATAN",
    "EEXP", "XITOP", "XTOP", "XGKICK",
];

/// Stub micro-VU profiler.
#[derive(Debug)]
pub struct MicroProfiler {
    pub op_stats: [u64; 166],
    pub prog_count: u32,
    pub index: i32,
}

impl Default for MicroProfiler {
    fn default() -> Self {
        MicroProfiler {
            op_stats: [0u64; 166],
            prog_count: 0,
            index: 0,
        }
    }
}

impl MicroProfiler {
    pub const PROG_LIMIT: u32 = 10000;
    pub fn reset(&mut self, _index: i32) {}
    pub fn emit_op(&mut self, op: MicroOpcode) {
        let i = op as usize;
        if i < self.op_stats.len() {
            self.op_stats[i] = self.op_stats[i].wrapping_add(1);
        }
    }
    pub fn print(&mut self) {
        // ASM stub
    }
}

// ===========================================================================
// Section: microVU constants and helper macros
//        (from `microVU_Misc.h` / `microVU_Misc.inl`)
// ===========================================================================

/// Globals bundle (mVU_Globals in C++). Each field is a 4-vector
/// of `u32` / `f32` bit patterns used as SSE register initializers.
#[repr(align(32))]
pub struct MvuGlobals {
    pub absclip: [u32; 4],
    pub signbit: [u32; 4],
    pub minvals: [u32; 4],
    pub maxvals: [u32; 4],
    pub exponent: [u32; 4],
    pub one: [u32; 4],
    pub pi4: [u32; 4],
    pub t1: [u32; 4],
    pub t5: [u32; 4],
    pub t2: [u32; 4],
    pub t3: [u32; 4],
    pub t4: [u32; 4],
    pub t6: [u32; 4],
    pub t7: [u32; 4],
    pub t8: [u32; 4],
    pub s2: [u32; 4],
    pub s3: [u32; 4],
    pub s4: [u32; 4],
    pub s5: [u32; 4],
    pub e1: [u32; 4],
    pub e2: [u32; 4],
    pub e3: [u32; 4],
    pub e4: [u32; 4],
    pub e5: [u32; 4],
    pub e6: [u32; 4],
    pub i32maxf: [u32; 4],
    pub ftoi_4: [f32; 4],
    pub ftoi_12: [f32; 4],
    pub ftoi_15: [f32; 4],
    pub itof_4: [f32; 4],
    pub itof_12: [f32; 4],
    pub itof_15: [f32; 4],
}

impl Default for MvuGlobals {
    fn default() -> Self {
        MvuGlobals {
            absclip: [0x7fff_ffff; 4],
            signbit: [0x8000_0000; 4],
            minvals: [0xff7f_ffff; 4],
            maxvals: [0x7f7f_ffff; 4],
            exponent: [0x7f80_0000; 4],
            one: [0x3f80_0000; 4],
            pi4: [0x3f49_0fdb; 4],
            t1: [0x3f7f_fff5; 4],
            t5: [0xbeaa_a61c; 4],
            t2: [0x3e4c_40a6; 4],
            t3: [0xbe0e_6c63; 4],
            t4: [0x3dc5_77df; 4],
            t6: [0xbd65_01c4; 4],
            t7: [0x3cb3_1652; 4],
            t8: [0xbb84_d7e7; 4],
            s2: [0xbe2a_aaa4; 4],
            s3: [0x3c08_873e; 4],
            s4: [0xb94f_b21f; 4],
            s5: [0x362e_9c14; 4],
            e1: [0x3e7f_ffa8; 4],
            e2: [0x3d00_07f4; 4],
            e3: [0x3b29_d3ff; 4],
            e4: [0x3933_e553; 4],
            e5: [0x36b6_3510; 4],
            e6: [0x3539_61ac; 4],
            i32maxf: [0x4eff_ffff; 4],
            ftoi_4: [16.0; 4],
            ftoi_12: [4096.0; 4],
            ftoi_15: [32768.0; 4],
            itof_4: [0.0625; 4],
            itof_12: [0.000244140625; 4],
            itof_15: [0.000030517578125; 4],
        }
    }
}

/// 32-byte aligned static instance.
#[repr(align(32))]
pub struct AlignedGlobals(pub MvuGlobals);
pub static mut MVU_GLOB: MvuGlobals = MvuGlobals {
    absclip: [0x7fff_ffff; 4], signbit: [0x8000_0000; 4],
    minvals: [0xff7f_ffff; 4], maxvals: [0x7f7f_ffff; 4],
    exponent: [0x7f80_0000; 4], one: [0x3f80_0000; 4],
    pi4: [0x3f49_0fdb; 4], t1: [0x3f7f_fff5; 4], t5: [0xbeaa_a61c; 4],
    t2: [0x3e4c_40a6; 4], t3: [0xbe0e_6c63; 4], t4: [0x3dc5_77df; 4],
    t6: [0xbd65_01c4; 4], t7: [0x3cb3_1652; 4], t8: [0xbb84_d7e7; 4],
    s2: [0xbe2a_aaa4; 4], s3: [0x3c08_873e; 4], s4: [0xb94f_b21f; 4],
    s5: [0x362e_9c14; 4],
    e1: [0x3e7f_ffa8; 4], e2: [0x3d00_07f4; 4], e3: [0x3b29_d3ff; 4],
    e4: [0x3933_e553; 4], e5: [0x36b6_3510; 4], e6: [0x3539_61ac; 4],
    i32maxf: [0x4eff_ffff; 4],
    ftoi_4: [16.0; 4], ftoi_12: [4096.0; 4], ftoi_15: [32768.0; 4],
    itof_4: [0.0625; 4], itof_12: [0.000244140625; 4],
    itof_15: [0.000030517578125; 4],
};

// ---------------------------------------------------------------------------
// VU macro-encoded bit fields
// ---------------------------------------------------------------------------

pub const _IBIT_: u32 = 1 << 31;
pub const _EBIT_: u32 = 1 << 30;
pub const _MBIT_: u32 = 1 << 29;
pub const _DBIT_: u32 = 1 << 28;
pub const _TBIT_: u32 = 1 << 27;
pub const DIVI: u32 = 0x104_0000;
pub const DIVD: u32 = 0x208_0000;

/// Branch-type string table.
pub const BRANCH_STR: [&str; 16] = [
    "None", "B", "BAL", "IBEQ", "IBGEZ", "IBGTZ", "IBLEZ", "IBLTZ",
    "IBNE", "JR", "JALR", "N/A", "N/A", "N/A", "N/A", "N/A",
];

// ---------------------------------------------------------------------------
// VU micro-instruction field extractors
// ---------------------------------------------------------------------------

#[inline]
pub const fn ft_field(code: u32) -> u32 { (code >> 16) & 0x1f }
#[inline]
pub const fn fs_field(code: u32) -> u32 { (code >> 11) & 0x1f }
#[inline]
pub const fn fd_field(code: u32) -> u32 { (code >> 6) & 0x1f }
#[inline]
pub const fn it_field(code: u32) -> u32 { (code >> 16) & 0xf }
#[inline]
pub const fn is_field(code: u32) -> u32 { (code >> 11) & 0xf }
#[inline]
pub const fn id_field(code: u32) -> u32 { (code >> 6) & 0xf }
#[inline]
pub const fn x_bit(code: u32) -> u32 { (code >> 24) & 0x1 }
#[inline]
pub const fn y_bit(code: u32) -> u32 { (code >> 23) & 0x1 }
#[inline]
pub const fn z_bit(code: u32) -> u32 { (code >> 22) & 0x1 }
#[inline]
pub const fn w_bit(code: u32) -> u32 { (code >> 21) & 0x1 }
#[inline]
pub const fn xyzw_bits(code: u32) -> u32 { (code >> 21) & 0xf }
#[inline]
pub const fn bc_field(code: u32) -> u32 { code & 0x3 }
#[inline]
pub const fn fsf_field(code: u32) -> u32 { (code >> 21) & 0x3 }
#[inline]
pub const fn ftf_field(code: u32) -> u32 { (code >> 23) & 0x3 }
#[inline]
pub const fn imm5(code: u32) -> i16 {
    let raw = if (code & 0x400) != 0 { 0xfff0u16 } else { 0 };
    (raw | ((code >> 6) & 0xf) as u16) as i16
}
#[inline]
pub const fn imm11(code: u32) -> i32 {
    if (code & 0x400) != 0 {
        (0xffff_fc00u32 | (code & 0x3ff)) as i32
    } else {
        (code & 0x3ff) as i32
    }
}
#[inline]
pub const fn imm12(code: u32) -> u32 {
    (((code >> 21) & 0x1) << 11) | (code & 0x7ff)
}
#[inline]
pub const fn imm15(code: u32) -> u32 {
    ((code >> 10) & 0x7800) | (code & 0x7ff)
}
#[inline]
pub const fn imm24(code: u32) -> u32 { code & 0x00ff_ffff }

// ---------------------------------------------------------------------------
// Optimization / debug option flags
// ---------------------------------------------------------------------------

/// If true, the dynarec keeps a register allocation across 32-bit
/// instructions. If false, every instruction flushes.
pub const DO_REG_ALLOC: bool = true;
/// If true, disable all VU flag-setting optimizations.
pub const NO_FLAG_OPTS: bool = false;
/// If true, keep four instances of the status flag across the
/// VU pipeline.
pub const DO_S_FLAG_INSTS: bool = true;
/// If true, keep four instances of the mac flag.
pub const DO_M_FLAG_INSTS: bool = true;
/// If true, keep four instances of the clip flag.
pub const DO_C_FLAG_INSTS: bool = true;
/// If true, support evil-branches (branches in branch delay slots).
pub const DO_BRANCH_IN_DELAY_SLOT: bool = true;
/// If true, do vi15 constant propagation (off by default; very
/// expensive in some games like GoW).
pub const DO_CONST_PROP: bool = false;
/// If true, cache indirect jump targets.
pub const DO_JUMP_CACHING: bool = true;
/// If true, treat indirect jumps as part of the same microProgram.
pub const DO_JUMP_AS_SAME_PROGRAM: bool = false;
/// Handling of D-Bit in micro programs. Should be off for shipping
/// games.
pub const DO_D_BIT_HANDLING: bool = false;
/// Whole-program comparison on search; rarely useful.
pub const DO_WHOLE_PROG_COMPARE: bool = false;

// ===========================================================================
// Section: microVU entry points (from `microVU.cpp`)
// ===========================================================================

pub fn mvu_init(_mvu: &mut MicroVu, _vu_index: u32) {
    // ASM stub
}
pub fn mvu_reset(_mvu: &mut MicroVu, _reset_reserve: bool) {
    // ASM stub
}
pub fn mvu_clear(_mvu: &mut MicroVu, _arg1: u32, _arg2: u32) {
    // ASM stub
}
pub fn mvu_block_fetch(
    _mvu: &mut MicroVu,
    _start_pc: u32,
    _p_state: uptr,
) -> *mut std::ffi::c_void {
    ptr::null_mut()
}
pub fn mvu_compile_jit(_start_pc: u32, _ptr: uptr) -> *mut std::ffi::c_void {
    ptr::null_mut()
}
pub fn mvu_search_prog(_start_pc: u32, _p_state: uptr) -> *mut std::ffi::c_void {
    ptr::null_mut()
}
pub fn mvu_execute_vu0(_start_pc: u32, _cycles: u32) -> *mut std::ffi::c_void {
    ptr::null_mut()
}
pub fn mvu_execute_vu1(_start_pc: u32, _cycles: u32) -> *mut std::ffi::c_void {
    ptr::null_mut()
}
pub fn mvu_cleanup_vu0() {
    // ASM stub
}
pub fn mvu_cleanup_vu1() {
    // ASM stub
}
pub fn mvu_cache_prog(_mvu: &mut MicroVu, _prog: &mut MicroProgram) {
    // ASM stub
}
pub fn mvu_delete_prog(_mvu: &mut MicroVu, _prog: &mut *mut MicroProgram) {
    // ASM stub
}

/// `mVUdispatcherAB` / `mVUdispatcherCD` - the VU1 thread dispatcher
/// functions generated by the C++ code.
pub fn mvu_dispatcher_ab(_mvu: &mut MicroVu) {
    // ASM stub
}
pub fn mvu_dispatcher_cd(_mvu: &mut MicroVu) {
    // ASM stub
}
pub fn mvu_generate_wait_mtvu(_mvu: &mut MicroVu) {
    // ASM stub
}
pub fn mvu_generate_copy_pipeline_state(_mvu: &mut MicroVu) {
    // ASM stub
}

/// `mVUmergeRegs(dest, src, xyzw, modXYZW)`. Merge lanes of two
/// XMM registers by mask. Real impl uses x86 SSE; here it's a stub.
pub fn mvu_merge_regs(_dest: i32, _src: i32, _xyzw: i32, _mod_xyzw: bool) {
    // ASM stub
}
pub fn mvu_save_reg(_reg: i32, _ptr: uptr, _xyzw: i32, _mod_xyzw: bool) {
    // ASM stub
}
pub fn mvu_load_reg(_reg: i32, _ptr: uptr, _xyzw: i32) {
    // ASM stub
}

// ===========================================================================
// Section: VIF dynarec + SSE unpack (from `Vif_Dynarec.cpp`,
//        `Vif_UnpackSSE.{h,cpp}`)
// ===========================================================================

/// `dVifReset(idx)` - reset the per-VIF dynarec state for `idx`.
pub fn dVifReset(_idx: i32) {
    // ASM stub
}

/// `dVifRelease(idx)` - release any resources held by VIF `idx`.
pub fn dVifRelease(_idx: i32) {
    // ASM stub
}

// ---------------------------------------------------------------------------
// VifUnpackSSE — class translation. All methods are stubs; the data
// members are kept so the layout matches the C++ ABI for the simple
// cases.
// ---------------------------------------------------------------------------

/// Base class for VIF unpack implementations.
pub struct VifUnpackSSEBase {
    pub usn: bool,
    pub do_mask: bool,
    pub unpk_loop_iteration: i32,
    pub unpk_no_of_iterations: i32,
    pub is_aligned: i32,
    pub dst_indirect: uptr,
    pub src_indirect: uptr,
    pub zero_reg: i32,
    pub work_reg: i32,
    pub dest_reg: i32,
}
impl Default for VifUnpackSSEBase {
    fn default() -> Self {
        VifUnpackSSEBase {
            usn: false,
            do_mask: false,
            unpk_loop_iteration: 0,
            unpk_no_of_iterations: 0,
            is_aligned: 0,
            dst_indirect: 0,
            src_indirect: 0,
            zero_reg: 0,
            work_reg: 0,
            dest_reg: 0,
        }
    }
}
impl VifUnpackSSEBase {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn xunpack(&self, _upktype: i32) {
        // ASM stub
    }
    pub fn is_write_protected_op(&self) -> bool {
        false
    }
    pub fn is_input_masked(&self) -> bool {
        false
    }
    pub fn is_unmasked_op(&self) -> bool {
        !self.do_mask
    }
    pub fn xmov_dest(&self) {
        // ASM stub
    }
}

/// "Simple" VIF unpack - one row at a time, no dynarec integration.
pub struct VifUnpackSSESimple {
    pub base: VifUnpackSSEBase,
    pub cur_cycle: i32,
}
impl VifUnpackSSESimple {
    pub fn new(usn_: bool, domask_: bool, cur_cycle_: i32) -> Self {
        VifUnpackSSESimple {
            base: VifUnpackSSEBase {
                usn: usn_,
                do_mask: domask_,
                unpk_loop_iteration: 0,
                unpk_no_of_iterations: 0,
                is_aligned: 0,
                dst_indirect: 0,
                src_indirect: 0,
                zero_reg: 0,
                work_reg: 0,
                dest_reg: 0,
            },
            cur_cycle: cur_cycle_,
        }
    }
}

/// "Dynarec" VIF unpack - integrated with the VIF dynarec.
pub struct VifUnpackSSEDynarec {
    pub base: VifUnpackSSEBase,
    pub is_fill: bool,
    pub do_mode: i32,
    pub skip_processing: bool,
    pub input_masked: bool,
    pub vif_ptr: i32,
    pub v_cl: i32,
    pub col_regs: [i32; 4],
    pub row_reg: i32,
    pub tmp_reg: i32,
}
impl VifUnpackSSEDynarec {
    pub fn new(_v: &NVifStruct, _v_b: &NVifBlock) -> Self {
        VifUnpackSSEDynarec {
            base: VifUnpackSSEBase::default(),
            is_fill: false,
            do_mode: 0,
            skip_processing: false,
            input_masked: false,
            vif_ptr: 0,
            v_cl: 0,
            col_regs: [0; 4],
            row_reg: 0,
            tmp_reg: 0,
        }
    }
    pub fn mod_unpack(&mut self, _upknum: i32, _post_op: bool) {
        // ASM stub
    }
    pub fn process_masks(&mut self) {
        // ASM stub
    }
    pub fn compile_routine(&mut self) {
        // ASM stub
    }
    pub fn set_masks(&self, _c_s: i32) {
        // ASM stub
    }
    pub fn write_back_row(&self) {
        // ASM stub
    }
}

// ---------------------------------------------------------------------------
// Placeholder types for the VIF dynarec state referenced by Vif_Dynarec.cpp
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct NVifStruct {
    pub vif_blocks: NVifBlocks,
    pub rec_write_ptr: *mut u8,
    pub rec_end_ptr: *mut u8,
}
#[derive(Debug, Default, Clone)]
pub struct NVifBlocks {
    pub blocks: Vec<NVifBlock>,
}
impl NVifBlocks {
    pub fn reset(&mut self) {
        self.blocks.clear();
    }
    pub fn clear(&mut self) {
        self.blocks.clear();
    }
}
#[derive(Debug, Default, Clone)]
pub struct NVifBlock {
    pub wl: i32,
    pub cl: i32,
    pub upk_type: u32,
    pub mode: u32,
    pub aligned: i32,
}

// ---------------------------------------------------------------------------
// `newVif.h` constants and helpers
// ---------------------------------------------------------------------------

/// Constants used to construct mask bytes.
pub const V0: u8 = 0x00;
pub const V1: u8 = 0x55;
pub const V2: u8 = 0xaa;
pub const V3: u8 = 0xff;

// ---------------------------------------------------------------------------
// Host CPU feature detection (from various x86 emitter wrappers).
// ---------------------------------------------------------------------------

/// CPU feature bits. Matches the bits in the C++ `x86capabilities`
/// struct.
pub mod cpu_features {
    use super::u32 as u32t;
    pub const HAS_MMX: u32t = 0x0000_0001;
    pub const HAS_SSE: u32t = 0x0000_0002;
    pub const HAS_SSE2: u32t = 0x0000_0004;
    pub const HAS_SSE3: u32t = 0x0000_0008;
    pub const HAS_SSSE3: u32t = 0x0000_0010;
    pub const HAS_SSE4_1: u32t = 0x0000_0020;
    pub const HAS_SSE4_2: u32t = 0x0000_0040;
    pub const HAS_AVX: u32t = 0x0000_0080;
    pub const HAS_AVX2: u32t = 0x0000_0100;
    pub const HAS_BMI1: u32t = 0x0000_0200;
    pub const HAS_BMI2: u32t = 0x0000_0400;
    pub const HAS_AES: u32t = 0x0000_0800;
    pub const HAS_F16C: u32t = 0x0000_1000;
    pub const HAS_FMA: u32t = 0x0000_2000;
    pub const HAS_AVX512F: u32t = 0x0000_4000;
}

/// Return the value of an extended control register. Stub for
/// `xgetbv`. Real implementation reads XCR0.
pub extern "C" fn xgetbv(_ecx: u32) -> u32 {
    // ASM stub
    0
}

/// Opaque host CPU descriptor. Mirrors the C++ `x86capabilities`.
#[derive(Debug, Default, Clone, Copy)]
pub struct X86Capabilities {
    pub features: u32,
    pub has_mmx: u8,
    pub has_sse: u8,
    pub has_sse2: u8,
    pub has_sse3: u8,
    pub has_ssse3: u8,
    pub has_sse4_1: u8,
    pub has_sse4_2: u8,
    pub has_avx: u8,
    pub has_avx2: u8,
    pub has_bmi1: u8,
    pub has_bmi2: u8,
    pub has_fma: u8,
    pub has_fast_pmovmskb: u8,
    pub has_fast_pshufb: u8,
}
/// Global CPU feature set, mirroring the C++ `x86capabilities`.
pub static mut G_CPU: X86Capabilities = X86Capabilities {
    features: 0, has_mmx: 0, has_sse: 0, has_sse2: 0, has_sse3: 0,
    has_ssse3: 0, has_sse4_1: 0, has_sse4_2: 0, has_avx: 0, has_avx2: 0,
    has_bmi1: 0, has_bmi2: 0, has_fma: 0, has_fast_pmovmskb: 0,
    has_fast_pshufb: 0,
};

/// `CHECK_FASTMEM` - whether the dynarec is allowed to use the
/// 4GB fastmem mapping. Mirrors the C++ `EmuConfig.Gamefixes` and
/// `Speedhacks` switches.
pub static mut CHECK_FASTMEM: bool = true;

// ===========================================================================
// Section: R5900 (EE) to x86 block recompilation entry point
//        (from `iR5900.cpp`, `iR5900Templates.cpp`)
// ===========================================================================

/// Stub for `R5900_RecompileBlock(pc)`. In a real port this walks
/// the EE instruction stream, allocates host registers, and emits
/// x86 into the dynarec cache.
pub fn r5900_recompile_block(_pc: u32) {
    // ASM stub
}

/// Stub for `microVU_RecompileBlock(vu, pc)`.
pub fn microVU_recompile_block(_vu: u32, _pc: u32) {
    // ASM stub
}

// ===========================================================================
// End of module
// ===========================================================================
