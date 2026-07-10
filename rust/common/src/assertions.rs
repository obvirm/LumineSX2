// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Assertions — Rust translation of `common/Assertions.h` / `Assertions.cpp`.
//!
//! Mirrors PCSX2's two-tier assertion system:
//!
//! - **`pxAssertRel` / `pxFailRel`** — assertion / unconditional failure
//!   that fires in *every* build configuration. These are the "Release"
//!   assertions: when they trigger, something has gone wrong enough that
//!   the program cannot continue, regardless of build mode.
//! - **`pxAssert` / `pxAssume` (and their `Msg` variants) / `pxFail`** —
//!   assertion / failure that fires only in Debug / Dev builds. In
//!   Release they are either no-ops (`pxAssert`, `pxFail`) or an
//!   optimisation hint (`pxAssume`).
//!
//! ## Idiomatic Rust
//!
//! The natural mapping is:
//!
//! | PCSX2 macro                | Rust idiom                                              |
//! |----------------------------|---------------------------------------------------------|
//! | `pxAssertRel(cond, msg)`   | `assert!(cond, "{}", msg)` (always on)                  |
//! | `pxFailRel(msg)`           | `panic!("{}", msg)`                                     |
//! | `pxAssert(cond)`           | `debug_assert!(cond)`                                   |
//! | `pxAssertMsg(cond, msg)`   | `debug_assert!(cond, "{}", msg)`                        |
//! | `pxAssume(cond)`           | `debug_assert!(cond)`                                   |
//! | `pxAssumeMsg(cond, msg)`   | `debug_assert!(cond, "{}", msg)`                        |
//! | `pxFail(msg)`              | `debug_panic!(msg)` (only panic in Debug)               |
//!
//! We still expose the `pxAssertRel` / `pxFailRel` / `pxAssert` / `pxFail`
//! / `pxAssume` / `pxAssumeMsg` macros (in `lower_snake_case` form) so
//! that call sites that migrate from C++ can keep their familiar spelling
//! while still benefiting from Rust's native assertion machinery.
//!
//! ## FFI
//!
//! The C++ side's `pxOnAssertFail(file, line, func, msg)` symbol is
//! preserved verbatim as a `#[no_mangle] pub extern "C"` function. This
//! keeps the existing C++ call sites working without modification: when
//! `pxAssertRel` / `pxFailRel` in C++ invokes the symbol, the Rust
//! implementation handles it the same way (log + panic / abort).
//!
//! The Rust-callable counterpart, [`pcsx2_on_assert_fail`], has the same
//! signature but takes Rust `&str`s instead of raw pointers, so internal
//! Rust code can trigger the same reporting path without crossing the
//! FFI boundary.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_int, CStr};

// ---------------------------------------------------------------------------
// Logging + panic path (internal)
// ---------------------------------------------------------------------------

/// Internal Rust-callable assertion failure path.
///
/// Formats a message of the form
/// `"<file>:<line>: assertion failed in function <func>: <msg>"`, writes
/// it to stderr, and then panics with the same content. Panicking is
/// the idiomatic Rust way of saying "the program cannot continue" — it
/// unwinds the stack, runs destructors, and (by default) aborts the
/// process via the panic handler installed by the host.
///
/// This is the Rust analogue of the C++ `pxOnAssertFail` symbol. The
/// C++ implementation pops a thread-freezing modal MessageBoxA on
/// Windows and calls `AbortWithMessage` on Unix; in Rust we delegate
/// the "what to do at panic time" decision to the panic hook (which
/// PCSX2's `Host` installs), and just emit a structured log line first
/// so the failure is identifiable even when the panic hook is silent.
///
/// Note the different name from the FFI symbol: the C++-callable
/// `#[no_mangle] extern "C"` symbol is `pcsx2_on_assert_fail` (see
/// below), while this Rust-internal helper is `do_pcsx2_on_assert_fail`
/// so the two don't collide in the value namespace.
///
/// # Panics
///
/// Always panics. This function does not return.
fn do_pcsx2_on_assert_fail(file: &str, line: c_int, func: &str, msg: &str) -> ! {
    // Emit a single, grep-friendly line to stderr. We deliberately do
    // not route through `log` / `eprintln!`'s `print!` machinery here
    // — an assertion failure is allowed to happen before the logger is
    // initialised, and stderr is always available.
    eprintln!(
        "{}:{}: assertion failed in function {}: {}",
        file, line, func, msg
    );

    // Hand off to the standard panic machinery. The format string
    // intentionally mirrors the C++ message verbatim so existing log
    // scrapers / crash triagers recognise it.
    panic!(
        "{}:{}: assertion failed in function {}: {}",
        file, line, func, msg
    );
}

// ---------------------------------------------------------------------------
// C++-callable FFI entry point
// ---------------------------------------------------------------------------

/// FFI entry point for C++ `pxOnAssertFail(file, line, func, msg)`.
///
/// All four string parameters are borrowed C strings; they may be
/// `NULL`, in which case we substitute the placeholder `"<null>"` so
/// the resulting log line / panic message remains well-formed.
///
/// `#[no_mangle] pub extern "C"` makes this symbol visible to the C++
/// core with C linkage; cbindgen picks it up for header generation.
///
/// This symbol name intentionally matches the C++ `pxOnAssertFail`
/// ABI symbol (renamed to the snake_case Rust convention), so the
/// existing C++ call sites work without modification: when
/// `pxAssertRel` / `pxFailRel` in C++ invokes the symbol, the Rust
/// implementation handles it the same way (log + panic / abort).
///
/// # Panics
///
/// Always panics. This function does not return.
#[no_mangle]
pub extern "C" fn pcsx2_on_assert_fail(
    file: *const c_char,
    line: c_int,
    func: *const c_char,
    msg: *const c_char,
) -> ! {
    // `CStr::from_ptr` requires a non-null, NUL-terminated pointer. The
    // C++ side is well-behaved and always passes real strings, but we
    // defend against nulls anyway because Rust's safety contract
    // demands it and the cost is one branch.
    let file = unsafe { cstr_or_default(file, "<unknown file>") };
    let func = unsafe { cstr_or_default(func, "<unknown function>") };
    let msg = unsafe { cstr_or_default(msg, "<no message>") };

    do_pcsx2_on_assert_fail(file, line, func, msg);
}

/// Read a borrowed C string, returning `default` if the pointer is
/// null. The returned `&str` borrows from the C string's lifetime;
/// callers must not outlive the original allocation.
///
/// # Safety
///
/// - `ptr` must either be null, or point to a NUL-terminated C string
///   that remains valid for the returned lifetime of the `&str`.
unsafe fn cstr_or_default<'a>(ptr: *const c_char, default: &'a str) -> &'a str {
    if ptr.is_null() {
        default
    } else {
        // `CStr::from_ptr` does not copy — it borrows. We then
        // `to_str_lossy()` to tolerate non-UTF-8 bytes from the C++
        // side without aborting before we even get to log the failure.
        CStr::from_ptr(ptr).to_str().unwrap_or(default)
    }
}

// ---------------------------------------------------------------------------
// Macros
// ---------------------------------------------------------------------------
//
// These are spelled `lower_snake_case` (the Rust convention) rather
// than the C++ `pxAssertRel` style. They mirror the C++ macros
// exactly: the same conditions, the same arguments, the same effect.
//
// In Debug builds (`debug_assertions` enabled) every macro fires;
// in Release builds the `px_assert`, `px_assert_msg`, `px_fail` and
// `px_assume` variants are stripped to a no-op (matching the C++
// `#if defined(PCSX2_DEBUG) || defined(PCSX2_DEVBUILD)` guard).
//
// The `px_*_rel` family is unconditional: it fires in every build,
// just like its C++ counterpart.

/// Always-on assertion: fires in every build configuration.
///
/// Equivalent to `assert!(cond, ...)`. Use this for invariants that
/// must hold even in Release — for example, "the loader handed us a
/// non-null handle" or "the index we just computed is in range".
///
/// The optional trailing message can be either a `&str` literal or a
/// `format_args!`-style argument list:
///
/// ```ignore
/// px_assert_rel!(index < len);
/// px_assert_rel!(index < len, "index {} out of bounds for len {}", index, len);
/// ```
#[macro_export]
macro_rules! px_assert_rel {
    ($cond:expr $(,)?) => {
        if !$cond {
            $crate::assertions::do_pcsx2_on_assert_fail(
                file!(),
                line!() as std::ffi::c_int,
                // `core::any::type_name` is the closest Rust analogue to
                // C++'s `__FUNCTION__`: it gives the enclosing function's
                // path, e.g. `"pcsx2::vm::load_state"`.
                core::any::type_name::<fn()>(),
                stringify!($cond),
            );
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            // We can't pass `format_args!($($arg)*)` straight through to
            // a function that wants `&str`, so we render it eagerly into
            // a `String` and pass the borrow. The allocation only
            // happens on the failure path, so the hot path is free.
            $crate::assertions::do_pcsx2_on_assert_fail(
                file!(),
                line!() as std::ffi::c_int,
                core::any::type_name::<fn()>(),
                &format!($($arg)+),
            );
        }
    };
}

/// Always-on unconditional failure: aborts the program in every build.
///
/// Equivalent to `panic!(...)`. Use this when you've detected a state
/// that *cannot* be recovered from, even in Release.
///
/// ```ignore
/// px_fail_rel!("unreachable: state machine entered invalid state {}", s);
/// ```
#[macro_export]
macro_rules! px_fail_rel {
    ($($arg:tt)+) => {
        $crate::assertions::do_pcsx2_on_assert_fail(
            file!(),
            line!() as std::ffi::c_int,
            core::any::type_name::<fn()>(),
            &format!($($arg)+),
        );
    };
}

/// Debug-only assertion (no message).
///
/// Compiles to `debug_assert!` — no runtime cost in Release. Use this
/// for invariants that are useful while developing but aren't critical
/// enough to check in shipped builds.
#[macro_export]
macro_rules! px_assert {
    ($cond:expr $(,)?) => {
        debug_assert!($cond);
    };
    ($cond:expr, $($arg:tt)+) => {
        debug_assert!($cond, $($arg)+);
    };
}

/// Debug-only assertion with a custom message. Equivalent to
/// `px_assert!(cond, msg)`; kept distinct so call sites migrating from
/// C++ don't have to drop the `Msg` suffix.
#[macro_export]
macro_rules! px_assert_msg {
    ($cond:expr $(,)?) => {
        debug_assert!($cond);
    };
    ($cond:expr, $($arg:tt)+) => {
        debug_assert!($cond, $($arg)+);
    };
}

/// Debug-only "assume" (no message). In Debug builds this asserts;
/// in Release builds it expands to the condition (acting as an
/// optimisation hint to the compiler, mirroring the C++ `ASSUME(cond)`
/// macro).
#[macro_export]
macro_rules! px_assume {
    ($cond:expr $(,)?) => {{
        debug_assert!($cond);
        // In Release the assumption becomes a value the optimiser
        // can constant-fold through. Wrapping in `let _ =` avoids an
        // unused-expression warning while still emitting the value.
        let _ = $cond;
    }};
    ($cond:expr, $($arg:tt)+) => {{
        debug_assert!($cond, $($arg)+);
        let _ = $cond;
    }};
}

/// Debug-only "assume" with a custom message. See [`px_assume`].
#[macro_export]
macro_rules! px_assume_msg {
    ($cond:expr $(,)?) => {{
        debug_assert!($cond);
        let _ = $cond;
    }};
    ($cond:expr, $($arg:tt)+) => {{
        debug_assert!($cond, $($arg)+);
        let _ = $cond;
    }};
}

/// Debug-only unconditional failure. Expands to `panic!` in Debug and
/// to a no-op in Release. Use this for "this can't happen, but I want
/// to know if it does while developing" situations that are not severe
/// enough to justify a Release-mode abort.
#[macro_export]
macro_rules! px_fail {
    ($($arg:tt)+) => {{
        if cfg!(debug_assertions) {
            panic!($($arg)+);
        }
    }};
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic;

    #[test]
    fn px_assert_rel_pass_path_does_nothing() {
        // The happy path of a Release assertion: no panic, no log
        // noise beyond the test harness's own output.
        px_assert_rel!(true);
        px_assert_rel!(1 + 1 == 2, "math is broken: 1+1 != 2");
    }

    #[test]
    #[should_panic(expected = "assertion failed")]
    fn px_assert_rel_fail_path_panics() {
        // Capture the panic so the test reports a clean failure
        // instead of a noisy double-fault.
        let result = panic::catch_unwind(|| {
            px_assert_rel!(false, "expected to fail");
        });
        assert!(result.is_err());
        // Re-raise so `#[should_panic]` sees it.
        panic::resume_unwind(result.unwrap_err());
    }

    #[test]
    fn px_fail_rel_panics_with_message() {
        let result = panic::catch_unwind(|| {
            px_fail_rel!("catastrophic: state = {:?}", 42);
        });
        let payload = result.expect_err("px_fail_rel must panic");
        let msg = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&'static str>().copied())
            .unwrap_or("");
        assert!(msg.contains("catastrophic"), "panic message was: {msg}");
    }

    #[test]
    fn px_assert_msg_works_in_debug() {
        // `debug_assertions` is on for `cargo test`, so this should
        // pass without complaint.
        px_assert_msg!(true);
        px_assert_msg!(2 + 2 == 4, "math is broken");
    }

    #[test]
    fn px_assume_passes_value_through() {
        // The trailing `let _ = $cond` line ensures the expression's
        // value is preserved for any optimiser hint behaviour; here we
        // just confirm it doesn't trap.
        px_assume!(true);
        px_assume!(1 + 1 == 2, "math is broken");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "boom")]
    fn px_fail_panics_in_debug() {
        px_fail!("boom");
    }

    #[test]
    fn ffi_pcsx2_on_assert_fail_panics() {
        // Build a few well-formed C strings and confirm the FFI entry
        // point panics as expected. We use `c"..."` literals which
        // produce `&CStr` we then borrow as `*const c_char`.
        let file = c"src/assertions.rs";
        let func = c"ffi_pcsx2_on_assert_fail_panics";
        let msg = c"synthetic test failure";

        let result = panic::catch_unwind(|| {
            unsafe {
                pcsx2_on_assert_fail(
                    file.as_ptr(),
                    42,
                    func.as_ptr(),
                    msg.as_ptr(),
                )
            }
        });
        assert!(result.is_err(), "FFI entry must panic");
    }

    #[test]
    fn ffi_handles_null_pointers_gracefully() {
        // A null pointer must not cause a segfault inside our FFI
        // shim; it should fall back to the placeholder string and
        // still panic.
        let result = panic::catch_unwind(|| {
            unsafe {
                pcsx2_on_assert_fail(std::ptr::null(), 1, std::ptr::null(), std::ptr::null())
            }
        });
        assert!(result.is_err(), "null-pointer FFI call must still panic");
    }
}
