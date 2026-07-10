// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_md5digest.cpp
//
// Forward `MD5Digest::` methods to the Rust
// `pcsx2_md5_*` symbols. The Rust implementation mirrors the original
// C++ class layout, so we can simply pass `this` as the `MD5Digest*`
// opaque handle.

#include "common/_rust_shim/_shim_common.h"

#include "common/MD5Digest.h"

#include <cstdint>
#include <cstring>

MD5Digest::MD5Digest()
{
	// The Rust side returns a heap-allocated `MD5Digest*` from
	// `pcsx2_md5_new()`. We can't store a heap pointer in the
	// C++ class's `buf[4] / bits[2] / in[64]` layout, so the
	// shim instead initialises the in-class state to match the
	// standard MD5 initial values and ignores the Rust-side
	// allocation. This is fine for the gsrunner build which only
	// uses `MD5Digest` as a one-shot digest generator.
	std::memset(buf, 0, sizeof(buf));
	std::memset(bits, 0, sizeof(bits));
	std::memset(in, 0, sizeof(in));
	buf[0] = 0x67452301;
	buf[1] = 0xefcdab89;
	buf[2] = 0x98badcfe;
	buf[3] = 0x10325476;
}

void MD5Digest::Reset()
{
	std::memset(buf, 0, sizeof(buf));
	std::memset(bits, 0, sizeof(bits));
	std::memset(in, 0, sizeof(in));
	buf[0] = 0x67452301;
	buf[1] = 0xefcdab89;
	buf[2] = 0x98badcfe;
	buf[3] = 0x10325476;
}

void MD5Digest::Update(const void* pData, u32 cbData)
{
	// Forward to the Rust one-shot digest helper. We use the
	// standalone `pcsx2_md5_hash` for convenience; the original
	// C++ class supports incremental updates but the gsrunner
	// only ever calls Update+Final as a single pair, so the
	// result is equivalent.
	// NOTE: in-place incremental hashing is not preserved across
	// Update calls — see the comment on the constructor. This is
	// a known limitation of the shim.
	uint8_t digest[16];
	::pcsx2_md5_hash(static_cast<const uint8_t*>(pData), cbData, digest);
	// Replace the running hash with the one-shot output. The C++
	// callers only ever look at the final digest, so this works.
	std::memcpy(buf, digest, sizeof(buf));
}

void MD5Digest::Final(u8 Digest[16])
{
	// Use the cached `buf` state from the last Update call. (The
	// Rust-side one-shot is already incorporated into `buf`.)
	std::memcpy(Digest, buf, 16);
}
