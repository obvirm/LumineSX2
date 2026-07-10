// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_memoryinterface.cpp
//
// C++ implementations of the `MemoryInterface::` non-pure methods
// (the IdempotentWrite overloads) that are unresolved (per
// `common/_rust_shim/MemoryInterface.txt`) when the original C++
// common/ sources are excluded.

#include "common/_rust_shim/_shim_common.h"

#include "common/MemoryInterface.h"

#include <cstdint>
#include <cstring>

bool MemoryInterface::IdempotentWrite8(u32 address, u8 value)
{
	return Write8(address, value);
}

bool MemoryInterface::IdempotentWrite16(u32 address, u16 value)
{
	return Write16(address, value);
}

bool MemoryInterface::IdempotentWrite32(u32 address, u32 value)
{
	return Write32(address, value);
}

bool MemoryInterface::IdempotentWrite64(u32 address, u64 value)
{
	return Write64(address, value);
}

bool MemoryInterface::IdempotentWrite128(u32 address, u128 value)
{
	// 128-bit write is exposed via the per-implementor
	// Write128 — the abstract base has no default implementation
	// in the FFI. Fall back to a pair of 64-bit writes. PCSX2's
	// `u128` union has explicit `lo`/`hi` fields of type `u64`.
	return Write64(address, value.lo) && Write64(address + 8, value.hi);
}

bool MemoryInterface::IdempotentWriteBytes(u32 address, void* src, u32 size)
{
	return WriteBytes(address, src, size);
}
