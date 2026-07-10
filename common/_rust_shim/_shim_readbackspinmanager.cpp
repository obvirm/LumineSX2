// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_readbackspinmanager.cpp
//
// C++ stub implementations of the `ReadbackSpinManager` methods that
// are unresolved when the original C++ common/ sources are excluded.
// The Rust port does not yet implement the readback spin manager; the
// shim returns safe defaults so the linker resolves.

#include "common/_rust_shim/_shim_common.h"

#include "common/ReadbackSpinManager.h"

ReadbackSpinManager::DrawSubmittedReturn ReadbackSpinManager::DrawSubmitted(u64 /*draw_nr*/)
{
	return DrawSubmittedReturn{0, 0};
}

void ReadbackSpinManager::DrawCompleted(u32 /*fv*/, u32 /*start*/, u32 /*end*/)
{
}

void ReadbackSpinManager::SpinCompleted(u32 /*fv*/, u32 /*start*/, u32 /*end*/)
{
}

void ReadbackSpinManager::NextFrame()
{
}

void ReadbackSpinManager::ReadbackRequested()
{
}
