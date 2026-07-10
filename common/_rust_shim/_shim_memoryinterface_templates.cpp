// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_memoryinterface_templates.cpp
//
// Out-of-line definitions for the `MemoryInterface::IdempotentWrite<T>`
// template specialisations declared in `common/MemoryInterface.h`.
// With `EXCLUDE_CPP_COMMON=ON`, the original `MemoryInterface.cpp`
// is not compiled; the linker therefore expects these template
// instantiations here.

#include "common/_rust_shim/_shim_common.h"

#include "common/MemoryInterface.h"

#include <cstdint>

// `MemoryInterface::IdempotentWrite<Value>(address, value)` simply
// dispatches to the type-specific `IdempotentWriteN` non-template
// method.

template <>
bool MemoryInterface::IdempotentWrite<u8>(u32 address, u8 value)
{
	return IdempotentWrite8(address, value);
}

template <>
bool MemoryInterface::IdempotentWrite<u16>(u32 address, u16 value)
{
	return IdempotentWrite16(address, value);
}

template <>
bool MemoryInterface::IdempotentWrite<u32>(u32 address, u32 value)
{
	return IdempotentWrite32(address, value);
}

template <>
bool MemoryInterface::IdempotentWrite<u64>(u32 address, u64 value)
{
	return IdempotentWrite64(address, value);
}
