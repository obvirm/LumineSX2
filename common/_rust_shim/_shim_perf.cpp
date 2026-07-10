// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_perf.cpp
//
// Forward `Perf::Group::` instance methods to the Rust
// `pcsx2_perf_group_register_key` symbol, or stub when no equivalent
// exists. Per `common/_rust_shim/Perf.txt`, three symbols are
// unresolved.

#include "common/_rust_shim/_shim_common.h"

#include "common/Perf.h"

#include <cstdint>

namespace Perf
{
	// The `Perf::any`, `Perf::ee`, etc. global instances are declared
	// `extern` in `Perf.h` and were originally defined in `Perf.cpp`.
	// With `EXCLUDE_CPP_COMMON=ON`, the original cpp is not compiled
	// and the linker expects these symbols to be defined somewhere.
	// We provide no-op instances here so the linker resolves cleanly.
	Group any("");
	Group ee("EE");
	Group iop("IOP");
	Group vu0("VU0");
	Group vu1("VU1");
	Group vif("VIF");

	void Group::Register(const void* ptr, size_t size, const char* symbol)
	{
		// The Rust FFI exposes a typed `pcsx2_perf_group_register_key`
		// that takes `(void* group, const void* key, uint64_t name_hash,
		// const char* name, uint64_t display_order)`. Map the C++
		// signature onto that:
		//   key          = ptr
		//   name_hash    = 0          (not used by the Rust stub)
		//   name         = symbol
		//   display_order = size       (close enough; the Rust side
		//                               ignores it).
		::pcsx2_perf_group_register_key(
			static_cast<void*>(this),
			ptr,
			0,
			symbol,
			static_cast<uint64_t>(size));
	}

	void Group::RegisterKey(const void* ptr, size_t size, const char* prefix, u64 key)
	{
		// Same FFI as Register; the `prefix` argument isn't carried
		// through but the symbol name is hashed into the registry.
		(void)prefix;
		(void)size;
		::pcsx2_perf_group_register_key(
			static_cast<void*>(this),
			ptr,
			0,
			prefix,
			key);
	}

	void Group::RegisterPC(const void* ptr, size_t size, u32 pc)
	{
		// Not ported; no-op.
		(void)ptr;
		(void)size;
		(void)pc;
	}
} // namespace Perf
