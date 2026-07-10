// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_pcsx2_assertions.h
//
// Declaration of the C++ `pxOnAssertFail` symbol, which the existing
// PCSX2 C++ `pxAssertRel` / `pxFailRel` macros in `common/Assertions.h`
// expect to find as `extern void pxOnAssertFail(const char* file,
// int line, const char* func, const char* msg)`.
//
// The implementation lives in `_shim_assertions.cpp` and forwards to the
// Rust `pcsx2_on_assert_fail` symbol from `pcsx2_common_rs`.

#pragma once

// The original C++ symbol is a free function with C++ linkage (name
// `?pxOnAssertFail@@YAXPEBDH00@Z`). Do NOT mark it `extern "C"` here —
// the shim's C++ definition must match the linkage the PCSX2 core
// expects, otherwise the linker won't resolve the call sites that
// reference the mangled name.
void pxOnAssertFail(const char* file, int line, const char* func, const char* msg);
