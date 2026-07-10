// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_texturedecompress.cpp
//
// C++ stub implementations of the texture-decompression free
// functions that are unresolved when the original C++ common/ sources
// are excluded. The Rust port does not yet implement BC1/BC2/BC3/BC7
// decompression; the shim is a no-op.

#include "common/_rust_shim/_shim_common.h"

#include "common/TextureDecompress.h"

#include <cstdint>

void DecompressBlockBC1(u32 /*col0*/, u32 /*col1*/, u32 /*xy_565*/, const u8* /*src*/, u8* /*dst*/)
{
}

void DecompressBlockBC2(u32 /*col0*/, u32 /*col1*/, u32 /*xy_565*/, const u8* /*src*/, u8* /*dst*/)
{
}

void DecompressBlockBC3(u32 /*col0*/, u32 /*col1*/, u32 /*xy_565*/, const u8* /*src*/, u8* /*dst*/)
{
}

namespace bc7decomp
{
	bool unpack_bc7(const void* /*input*/, color_rgba* /*output*/)
	{
		return false;
	}
} // namespace bc7decomp
