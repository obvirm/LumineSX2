// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_yaml.cpp
//
// C++ stub implementation of `ParseYAMLFromString` from common/YAML.h.
// The original is in common/YAML.cpp; with EXCLUDE_CPP_COMMON=ON we
// provide a minimal stub that returns std::nullopt. The gsrunner
// build does not parse YAML files at runtime in this configuration.

#include "common/_rust_shim/_shim_common.h"

#include "common/YAML.h"
#include "common/Error.h"

#include <ryml.hpp>
#include <ryml_std.hpp>

#include <optional>

std::optional<ryml::Tree> ParseYAMLFromString(ryml::csubstr /*yaml*/, ryml::csubstr /*file_name*/, Error* /*error*/)
{
	return std::nullopt;
}