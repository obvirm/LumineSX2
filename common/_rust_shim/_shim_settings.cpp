// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_settings.cpp
//
// Stub implementations of the SettingsInterface family of classes
// (MemorySettingsInterface, SettingsLoadWrapper, SettingsSaveWrapper)
// plus the ProgressCallback::CreateNullProgressCallback factory and
// the SharedMemoryMappingArea class — all unresolved when the original
// C++ common/ sources are excluded.

#include "common/_rust_shim/_shim_common.h"

#include "common/MemorySettingsInterface.h"
#include "common/ProgressCallback.h"
#include "common/SettingsWrapper.h"
#include "common/SmallString.h"
#include "common/HostSys.h"

#include <memory>
#include <string_view>

// ---------------------------------------------------------------------------
// MemorySettingsInterface
// ---------------------------------------------------------------------------

MemorySettingsInterface::MemorySettingsInterface() = default;
MemorySettingsInterface::~MemorySettingsInterface() = default;

bool MemorySettingsInterface::Save(Error* /*error*/) { return true; }
void MemorySettingsInterface::Clear() {}
bool MemorySettingsInterface::IsEmpty() { return true; }

bool MemorySettingsInterface::GetIntValue(const char*, const char*, s32*) const { return false; }
bool MemorySettingsInterface::GetUIntValue(const char*, const char*, u32*) const { return false; }
bool MemorySettingsInterface::GetFloatValue(const char*, const char*, float*) const { return false; }
bool MemorySettingsInterface::GetDoubleValue(const char*, const char*, double*) const { return false; }
bool MemorySettingsInterface::GetBoolValue(const char*, const char*, bool*) const { return false; }
bool MemorySettingsInterface::GetStringValue(const char*, const char*, std::string*) const { return false; }
bool MemorySettingsInterface::GetStringValue(const char*, const char*, SmallStringBase*) const { return false; }

void MemorySettingsInterface::SetIntValue(const char*, const char*, s32) {}
void MemorySettingsInterface::SetUIntValue(const char*, const char*, u32) {}
void MemorySettingsInterface::SetFloatValue(const char*, const char*, float) {}
void MemorySettingsInterface::SetDoubleValue(const char*, const char*, double) {}
void MemorySettingsInterface::SetBoolValue(const char*, const char*, bool) {}
void MemorySettingsInterface::SetStringValue(const char*, const char*, const char*) {}

std::vector<std::pair<std::string, std::string>> MemorySettingsInterface::GetKeyValueList(const char*) const { return {}; }
void MemorySettingsInterface::SetKeyValueList(const char*, const std::vector<std::pair<std::string, std::string>>&) {}

bool MemorySettingsInterface::ContainsValue(const char*, const char*) const { return false; }
void MemorySettingsInterface::DeleteValue(const char*, const char*) {}
void MemorySettingsInterface::ClearSection(const char*) {}
void MemorySettingsInterface::RemoveSection(const char*) {}
void MemorySettingsInterface::RemoveEmptySections() {}

std::vector<std::string> MemorySettingsInterface::GetStringList(const char*, const char*) const { return {}; }
void MemorySettingsInterface::SetStringList(const char*, const char*, const std::vector<std::string>&) {}
bool MemorySettingsInterface::RemoveFromStringList(const char*, const char*, const char*) { return false; }
bool MemorySettingsInterface::AddToStringList(const char*, const char*, const char*) { return false; }

// ---------------------------------------------------------------------------
// SettingsWrapper family
// ---------------------------------------------------------------------------

SettingsWrapper::SettingsWrapper(SettingsInterface& si) : m_si(si) {}

SettingsLoadWrapper::SettingsLoadWrapper(SettingsInterface& si) : SettingsWrapper(si) {}
bool SettingsLoadWrapper::IsLoading() const { return true; }
bool SettingsLoadWrapper::IsSaving() const { return false; }
void SettingsLoadWrapper::Entry(const char*, const char*, int& v, int) { v = 0; }
void SettingsLoadWrapper::Entry(const char*, const char*, uint& v, uint) { v = 0; }
void SettingsLoadWrapper::Entry(const char*, const char*, bool& v, bool) { v = false; }
void SettingsLoadWrapper::Entry(const char*, const char*, float& v, float) { v = 0.0f; }
void SettingsLoadWrapper::Entry(const char*, const char*, std::string& v, const std::string&) { v.clear(); }
void SettingsLoadWrapper::Entry(const char*, const char*, SmallStringBase& v, std::string_view) { v.clear(); }
bool SettingsLoadWrapper::EntryBitBool(const char*, const char*, bool v, bool) { return v; }
int SettingsLoadWrapper::EntryBitfield(const char*, const char*, int v, int) { return v; }
void SettingsLoadWrapper::_EnumEntry(const char*, const char*, int& v, const char* const*, int) { v = 0; }

SettingsSaveWrapper::SettingsSaveWrapper(SettingsInterface& si) : SettingsWrapper(si) {}
bool SettingsSaveWrapper::IsLoading() const { return false; }
bool SettingsSaveWrapper::IsSaving() const { return true; }
void SettingsSaveWrapper::Entry(const char*, const char*, int& v, int) { (void)v; }
void SettingsSaveWrapper::Entry(const char*, const char*, uint& v, uint) { (void)v; }
void SettingsSaveWrapper::Entry(const char*, const char*, bool& v, bool) { (void)v; }
void SettingsSaveWrapper::Entry(const char*, const char*, float& v, float) { (void)v; }
void SettingsSaveWrapper::Entry(const char*, const char*, std::string& v, const std::string&) { (void)v; }
void SettingsSaveWrapper::Entry(const char*, const char*, SmallStringBase& v, std::string_view) { (void)v; }
bool SettingsSaveWrapper::EntryBitBool(const char*, const char*, bool v, bool) { return v; }
int SettingsSaveWrapper::EntryBitfield(const char*, const char*, int v, int) { return v; }
void SettingsSaveWrapper::_EnumEntry(const char*, const char*, int& v, const char* const*, int) { (void)v; }

SettingsClearWrapper::SettingsClearWrapper(SettingsInterface& si) : SettingsWrapper(si) {}
bool SettingsClearWrapper::IsLoading() const { return false; }
bool SettingsClearWrapper::IsSaving() const { return false; }
void SettingsClearWrapper::Entry(const char*, const char*, int& v, int) { v = 0; }
void SettingsClearWrapper::Entry(const char*, const char*, uint& v, uint) { v = 0; }
void SettingsClearWrapper::Entry(const char*, const char*, bool& v, bool) { v = false; }
void SettingsClearWrapper::Entry(const char*, const char*, float& v, float) { v = 0.0f; }
void SettingsClearWrapper::Entry(const char*, const char*, std::string& v, const std::string&) { v.clear(); }
void SettingsClearWrapper::Entry(const char*, const char*, SmallStringBase& v, std::string_view) { v.clear(); }
bool SettingsClearWrapper::EntryBitBool(const char*, const char*, bool v, bool) { return v; }
int SettingsClearWrapper::EntryBitfield(const char*, const char*, int v, int) { return v; }
void SettingsClearWrapper::_EnumEntry(const char*, const char*, int& v, const char* const*, int) { v = 0; }

// ---------------------------------------------------------------------------
// ProgressCallback
// ---------------------------------------------------------------------------

std::unique_ptr<ProgressCallback> ProgressCallback::CreateNullProgressCallback()
{
	return nullptr;
}

// ---------------------------------------------------------------------------
// SharedMemoryMappingArea
// ---------------------------------------------------------------------------

SharedMemoryMappingArea::SharedMemoryMappingArea(u8*, size_t, size_t) {}
SharedMemoryMappingArea::~SharedMemoryMappingArea() {}

std::unique_ptr<SharedMemoryMappingArea> SharedMemoryMappingArea::Create(u64, bool)
{
	return nullptr;
}

u8* SharedMemoryMappingArea::Map(void*, u64, void*, u64, const PageProtectionMode&)
{
	return nullptr;
}

bool SharedMemoryMappingArea::Unmap(void*, u64, bool)
{
	return false;
}