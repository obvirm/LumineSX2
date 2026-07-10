// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Deferred port note for `common/FastJmp.asm`.
//!
//! ## Status: deferred (assembly-only).
//!
//! The C++ PCSX2 core contains a hand-written MASM assembly file,
//! [`common/FastJmp.asm`][asm], that provides two Windows x86_64 entry
//! points used by `common/FastJmp.h` to implement a fast,
//! non-trivial `setjmp`/`longjmp` pair for the JIT exception model:
//!
//! | MASM symbol      | Signature                          | Purpose                            |
//! |------------------|------------------------------------|------------------------------------|
//! | `fastjmp_set`    | `void fastjmp_set(fastjmp_buf*)`   | Capture the current register state |
//! | `fastjmp_jmp`    | `void fastjmp_jmp(fastjmp_buf*, int)` | Restore it and `jmp` to the saved rip |
//!
//! What is saved/restored (per the assembly):
//!
//! * GPRs: `rax` (saved as the return address / `rip`), `rbx`, `rsp`,
//!   `rbp`, `rsi`, `rdi`, `r12`-`r15`.
//! * SSE: `xmm6`-`xmm15` (the callee-saved SIMD registers on the Windows
//!   x64 ABI).
//!
//! The "set" routine stores `[rsp]` (the return address pushed by `call
//! fastjmp_set`) into the buffer's `rip` slot, fixup-adjusts `rsp` past
//! the `call`, then returns `0` in `eax` (mimicking the C `setjmp`
//! contract). The "jmp" routine restores every saved register, sets
//! `eax = <return code>`, and tail-jumps to the saved `rip`.
//!
//! ## Why this is not translated to Rust.
//!
//! This module is *pure assembly* — the C++ side does not link a
//! corresponding `.cpp` translation unit against `FastJmp.asm`; it is
//! only ever assembled by MASM and exposed via `extern "C"` declarations
//! in `common/FastJmp.h`. There is no portable Rust source to mirror:
//! the file is a hand-tuned sequence of `mov`/`movaps`/`jmp`
//! instructions whose exact encoding matters for performance
//! (e.g. the `add rcx, 112` mid-procedure dispatches are deliberate
//! to keep the displacement on the SIMD stores within one byte).
//!
//! Translating this to Rust would require either:
//!
//! 1. Re-emitting the same x86_64 sequence via `core::arch::asm!`
//!    (Windows-x64-only, requires `target_arch = "x86_64"` +
//!    `target_env = "msvc"` guards and `#[naked]` semantics that
//!    stable Rust does not yet provide cleanly), or
//! 2. Falling back to a portable C library implementation
//!    (`libc::setjmp` / `libc::longjmp`, or a small wrapper around
//!    the platform `setjmp.h`).
//!
//! ## What the Rust port does instead.
//!
//! The Rust port of `FastJmp` lives in [`fast_jmp.rs`][rust] and uses
//! the *portable* path: it wraps `libc::setjmp` / `libc::longjmp` (on
//! non-Windows) and falls back to `SetjmpExceptionHandler` /
//! `LongjmpExceptionHandler` semantics on Windows via the C runtime's
//! `setjmp.h`. This loses the small constant-factor win the hand-rolled
//! MASM gives on Windows x64 (a handful of cycles per save/restore), but
//! keeps the code 100% safe, 100% portable, and free of inline-asm
//! platform-gating.
//!
//! ## Future work.
//!
//! If profiling on Windows x64 ever shows the JIT exception path being
//! `setjmp`/`longjmp`-bound, the right next step is to add an
//! `#[cfg(all(target_arch = "x86_64", target_env = "msvc"))]` block to
//! `fast_jmp.rs` that exposes `fastjmp_set` / `fastjmp_jmp` via
//! `core::arch::asm!` using the same register save layout as
//! `FastJmp.asm` (gprs + `xmm6`-`xmm15`), and switch the build behind a
//! `cfg` flag. That work is out of scope for the current port and is
//! tracked as a follow-up rather than landing here.
//!
//! ## FFI exports
//!
//! **None.** This file is a documentation stub only. It does not
//! contain any `#[no_mangle]`, `extern "C"`, or other export surface —
//! the actual Rust `fastjmp_*` API is in `fast_jmp.rs` and re-exported
//! from `lib.rs` like every other module in this crate.
//!
//! [asm]: ../../../../common/FastJmp.asm
//! [rust]: ./fast_jmp.rs

#![allow(dead_code, unused_imports)]

// Intentionally empty. See the module-level documentation above for
// the rationale. Keeping the file body empty (rather than deleting it)
// ensures the deferred-port note survives any future "audit which C++
// sources have been ported" sweep.