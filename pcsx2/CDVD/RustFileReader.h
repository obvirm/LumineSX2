// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

#pragma once

#include "ThreadedFileReader.h"

#include <memory>

/// Create a Rust-backed file reader for ISO/raw disc images
/// Uses the `pcsx2_cdvd` Rust crate via FFI
std::unique_ptr<ThreadedFileReader> CreateRustFileReader();
