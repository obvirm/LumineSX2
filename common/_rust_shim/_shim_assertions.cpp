// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_assertions.cpp
//
// Forward `pxOnAssertFail` to the Rust `pcsx2_on_assert_fail` symbol.
// The Rust side handles logging and panicking exactly the way the
// original C++ `Assertions.cpp` did, so the call site
// (`pxAssertRel(...)` in the C++ core) needs no change.

#include "common/_rust_shim/_shim_common.h"
#include "common/_rust_shim/_shim_pcsx2_assertions.h"

// bumped

// NOTE: do NOT wrap in `extern "C"` here. The original C++ symbol is
// a free function with C++ linkage (name `?pxOnAssertFail@@YAXPEBDH00@Z`),
// so the shim must keep the same linkage for the linker to match.
void pxOnAssertFail(const char* file, int line, const char* func, const char* msg)
{
	// Forward to Rust. The Rust function is documented as never
	// returning (it panics / aborts), matching the C++
	// `[[noreturn]]` semantics of the original implementation.
	::pcsx2_on_assert_fail(file, line, func, msg);
}
