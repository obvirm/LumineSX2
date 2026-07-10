// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_extras.cpp
//
// Stub implementations of additional `common/` free functions and
// methods that are unresolved when the original C++ common/ sources
// are excluded but which were not in the original 14 unresolved-
// symbols lists. These are needed because the PCSX2 core (which
// links against `common`) calls into them.
//
// Each stub returns a safe default so the linker resolves.

#include "common/_rust_shim/_shim_common.h"

#include "common/HostSys.h"
#include "common/ProgressCallback.h"
#include "common/SmallString.h"
#include "common/Image.h"
#include "common/StringUtil.h"

#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <string>

// ---------------------------------------------------------------------------
// Free functions in HostSys / Common / PageFaultHandler / etc.
// ---------------------------------------------------------------------------

u64 GetTickFrequency()
{
	// Fall back to the Rust timer primitive.
	return ::pcsx2_timer_get_tick_frequency();
}

u64 GetCPUTicks()
{
	return ::pcsx2_timer_get_cpu_ticks();
}

u64 GetPhysicalMemory()
{
	return ::pcsx2_host_physical_memory();
}

u64 GetAvailablePhysicalMemory()
{
	// No FFI for "available" memory yet — fall back to the total.
	return ::pcsx2_host_physical_memory();
}

const CPUInfo& GetCPUInfo()
{
	// The shim has no CPU detection; return a static placeholder.
	// Callers in the gsrunner never read the fields.
	static const CPUInfo info{};
	return info;
}

u32 ShortSpin()
{
	return 0;
}

const u32 SPIN_TIME_NS = 1000;

void AbortWithMessage(const char* /*msg*/)
{
	std::abort();
}

std::string GetOSVersionString()
{
	return std::string("PCSX2-shim");
}

namespace Common
{
	bool InhibitScreensaver(bool /*inhibit*/)
	{
		return false;
	}

	bool PlaySoundAsync(const char* /*path*/)
	{
		return false;
	}

	void SetMousePosition(int /*x*/, int /*y*/)
	{
	}

	bool AttachMousePositionCb(std::function<void(int, int)> /*cb*/)
	{
		return false;
	}

	void DetachMousePositionCb()
	{
	}
} // namespace Common

namespace PageFaultHandler
{
	bool Install(Error* /*error*/)
	{
		return false;
	}

	bool InstallSecondaryThread()
	{
		return false;
	}

	// HandlePageFault is provided by pcsx2/lib/Core/vtlb.cpp (linked into
	// pcsx2.lib). Defining it here too would create an LNK2005 duplicate,
	// so the shim relies on the PCSX2 core's own implementation.
} // namespace PageFaultHandler

// ---------------------------------------------------------------------------
// ProgressCallback
// ---------------------------------------------------------------------------

ProgressCallback* ProgressCallback::NullProgressCallback = nullptr;

// ---------------------------------------------------------------------------
// SmallStringBase copy-assignment (the original header uses
// `SmallStringBase& operator=(const SmallStringBase&)`).
// ---------------------------------------------------------------------------

SmallStringBase& SmallStringBase::operator=(const SmallStringBase& copy)
{
	if (this != &copy)
	{
		clear();
		assign(copy.view());
	}
	return *this;
}

SmallStringBase& SmallStringBase::operator=(SmallStringBase&& move)
{
	if (this != &move)
	{
		clear();
		assign(move.view());
	}
	return *this;
}

// ---------------------------------------------------------------------------
// ProgressCallback virtual destructor
// ---------------------------------------------------------------------------

ProgressCallback::~ProgressCallback() {}

// Variadic formatted methods — use vsnprintf directly.
void ProgressCallback::SetFormattedStatusText(const char* Format, ...)
{
	char buf[4096];
	va_list ap;
	va_start(ap, Format);
	vsnprintf(buf, sizeof(buf), Format, ap);
	va_end(ap);
	SetStatusText(buf);
}

void ProgressCallback::DisplayFormattedError(const char* format, ...)
{
	char buf[4096];
	va_list ap;
	va_start(ap, format);
	vsnprintf(buf, sizeof(buf), format, ap);
	va_end(ap);
	DisplayError(buf);
}

void ProgressCallback::DisplayFormattedWarning(const char* format, ...)
{
	char buf[4096];
	va_list ap;
	va_start(ap, format);
	vsnprintf(buf, sizeof(buf), format, ap);
	va_end(ap);
	DisplayWarning(buf);
}

void ProgressCallback::DisplayFormattedInformation(const char* format, ...)
{
	char buf[4096];
	va_list ap;
	va_start(ap, format);
	vsnprintf(buf, sizeof(buf), format, ap);
	va_end(ap);
	DisplayInformation(buf);
}

void ProgressCallback::DisplayFormattedDebugMessage(const char* format, ...)
{
	char buf[4096];
	va_list ap;
	va_start(ap, format);
	vsnprintf(buf, sizeof(buf), format, ap);
	va_end(ap);
	DisplayDebugMessage(buf);
}

void ProgressCallback::DisplayFormattedModalError(const char* format, ...)
{
	char buf[4096];
	va_list ap;
	va_start(ap, format);
	vsnprintf(buf, sizeof(buf), format, ap);
	va_end(ap);
	ModalError(buf);
}

bool ProgressCallback::DisplayFormattedModalConfirmation(const char* format, ...)
{
	char buf[4096];
	va_list ap;
	va_start(ap, format);
	vsnprintf(buf, sizeof(buf), format, ap);
	va_end(ap);
	return ModalConfirmation(buf);
}

void ProgressCallback::DisplayFormattedModalInformation(const char* format, ...)
{
	char buf[4096];
	va_list ap;
	va_start(ap, format);
	vsnprintf(buf, sizeof(buf), format, ap);
	va_end(ap);
	ModalInformation(buf);
}

// ---------------------------------------------------------------------------
// RGBA8Image — declared in common/Image.h.
// Real implementation that delegates to Rust FFI (pcsx2_rgba_image_*).
// Replaces the old stubs that always returned false.
// ---------------------------------------------------------------------------

// FFI declarations — these are exported from `rust/common/src/image.rs`
// and will be picked up by cbindgen in the generated header.
extern "C"
{
	u32 pcsx2_image_load_from_file(const char* path, u32* out_width, u32* out_height, u8** out_pixels);
	bool pcsx2_image_save_to_file(const char* path, u32 width, u32 height, const u8* pixels, u32 stride, u8 quality);
	u32 pcsx2_image_load_from_buffer(const u8* buffer, size_t buffer_size, u32* out_width, u32* out_height, u8** out_pixels);
	u8* pcsx2_image_save_to_buffer(u32 width, u32 height, const u8* pixels, u32 stride, const char* format, u8 quality, size_t* out_size);
	void pcsx2_image_free(u8* pixels);
	void* pcsx2_rgba_image_create();
	void pcsx2_rgba_image_destroy(void* handle);
	bool pcsx2_rgba_image_load_from_file(void* handle, const char* path);
	bool pcsx2_rgba_image_load_from_buffer(void* handle, const char* format, const void* buffer, u64 size);
	bool pcsx2_rgba_image_save_to_file(void* handle, const char* path, u8 quality);
	bool pcsx2_rgba_image_save_to_file_ptr(void* handle, const char* path, void* fp, u8 quality);
	u8* pcsx2_rgba_image_save_to_buffer(void* handle, const char* format, u8 quality, size_t* out_size);
	u32 pcsx2_rgba_image_get_width(const void* handle);
	u32 pcsx2_rgba_image_get_height(const void* handle);
	const u8* pcsx2_rgba_image_get_pixels(const void* handle);
	u32 pcsx2_rgba_image_get_pixel_count(const void* handle);
	bool pcsx2_rgba_image_is_valid(const void* handle);
	u32 pcsx2_rgba_image_copy_pixels(const void* handle, u8* dst, u32 dst_size);
}

RGBA8Image::RGBA8Image() {}
RGBA8Image::RGBA8Image(const RGBA8Image& copy)
	: Image(copy)
{
}
RGBA8Image::RGBA8Image(const RGBA8Image&& move)
	: Image(move)
{
}
RGBA8Image::RGBA8Image(u32 width, u32 height)
	: Image(width, height)
{
}
RGBA8Image::RGBA8Image(u32 width, u32 height, const u32* pixels)
	: Image(width, height, pixels)
{
}
RGBA8Image::RGBA8Image(u32 width, u32 height, std::vector<u32> pixels)
	: Image(width, height, std::move(pixels))
{
}
RGBA8Image& RGBA8Image::operator=(const RGBA8Image& copy)
{
	Image<u32>::operator=(copy);
	return *this;
}
RGBA8Image& RGBA8Image::operator=(RGBA8Image&& move)
{
	Image<u32>::operator=(move);
	return *this;
}

bool RGBA8Image::LoadFromFile(const char* filename)
{
	u32 w = 0, h = 0;
	u8* pixels = nullptr;
	const u32 size = ::pcsx2_image_load_from_file(filename, &w, &h, &pixels);
	if (size == 0 || !pixels)
		return false;

	SetSize(w, h);
	std::memcpy(GetPixels(), pixels, static_cast<size_t>(size));
	::pcsx2_image_free(pixels);
	return true;
}

bool RGBA8Image::SaveToFile(const char* filename, u8 quality) const
{
	if (m_width == 0 || m_height == 0 || m_pixels.empty())
		return false;

	return ::pcsx2_image_save_to_file(filename, m_width, m_height,
		reinterpret_cast<const u8*>(m_pixels.data()), GetPitch(), quality);
}

bool RGBA8Image::LoadFromFile(const char* filename, std::FILE* fp)
{
	// Fall back to LoadFromFile with path — the image crate reads the
	// file natively and auto-detects format from content.
	return LoadFromFile(filename);
}

bool RGBA8Image::SaveToFile(const char* filename, std::FILE* fp, u8 quality) const
{
	// Fall back to SaveToFile with path — the image crate writes the
	// file natively; format is auto-detected from the file extension.
	return SaveToFile(filename, quality);
}

bool RGBA8Image::LoadFromBuffer(const char* /*format*/, const void* buffer, u64 size)
{
	u32 w = 0, h = 0;
	u8* pixels = nullptr;
	const u32 pixel_count = ::pcsx2_image_load_from_buffer(
		static_cast<const u8*>(buffer), static_cast<size_t>(size), &w, &h, &pixels);
	if (pixel_count == 0 || !pixels)
		return false;

	SetSize(w, h);
	std::memcpy(GetPixels(), pixels, static_cast<size_t>(pixel_count));
	::pcsx2_image_free(pixels);
	return true;
}

std::optional<std::vector<u8>> RGBA8Image::SaveToBuffer(const char* filename, u8 quality) const
{
	std::optional<std::vector<u8>> ret;
	if (m_width == 0 || m_height == 0 || m_pixels.empty())
		return ret;

	// Derive format hint from extension
	const char* fmt_str = "png";
	const char* dot = std::strrchr(filename, '.');
	if (dot)
	{
		dot++; // skip dot
		if (StringUtil::Strncasecmp(dot, "jpg", 3) == 0 || StringUtil::Strncasecmp(dot, "jpeg", 4) == 0)
			fmt_str = "jpeg";
		else if (StringUtil::Strncasecmp(dot, "webp", 4) == 0)
			fmt_str = "webp";
		else if (StringUtil::Strncasecmp(dot, "bmp", 3) == 0)
			fmt_str = "bmp";
	}

	size_t out_size = 0;
	u8* encoded = ::pcsx2_image_save_to_buffer(
		m_width, m_height,
		reinterpret_cast<const u8*>(m_pixels.data()),
		GetPitch(), fmt_str, quality, &out_size);
	if (encoded && out_size > 0)
	{
		ret = std::vector<u8>(encoded, encoded + out_size);
		::pcsx2_image_free(encoded);
	}
	return ret;
}

// ---------------------------------------------------------------------------
// BaseProgressCallback — full implementation needed by updater.exe
// and other PCSX2 binaries that use progress reporting.
// ---------------------------------------------------------------------------

BaseProgressCallback::BaseProgressCallback()
{
	m_cancellable = false;
}

BaseProgressCallback::~BaseProgressCallback() {}

void BaseProgressCallback::PushState()
{
	State* state = new State;
	state->next_saved_state = m_saved_state;
	state->status_text = m_status_text;
	state->progress_range = m_progress_range;
	state->progress_value = m_progress_value;
	state->base_progress_value = m_base_progress_value;
	state->cancellable = m_cancellable;
	m_saved_state = state;
}

void BaseProgressCallback::PopState()
{
	State* state = m_saved_state;
	m_saved_state = state->next_saved_state;
	m_status_text = state->status_text;
	m_progress_range = state->progress_range;
	m_progress_value = state->progress_value;
	m_base_progress_value = state->base_progress_value;
	m_cancellable = state->cancellable;
	delete state;
}

bool BaseProgressCallback::IsCancelled() const
{
	return m_cancelled;
}

bool BaseProgressCallback::IsCancellable() const
{
	return m_cancellable;
}

void BaseProgressCallback::SetCancellable(bool cancellable)
{
	m_cancellable = cancellable;
}

void BaseProgressCallback::SetStatusText(const char* text)
{
	m_status_text = text ? text : "";
}

void BaseProgressCallback::SetProgressRange(u32 range)
{
	m_progress_range = range;
	m_progress_value = 0;
}

void BaseProgressCallback::SetProgressValue(u32 value)
{
	m_progress_value = value;
}

void BaseProgressCallback::IncrementProgressValue()
{
	m_progress_value++;
}

void BaseProgressCallback::SetProgressState(ProgressState state)
{
	m_progress_state = state;
}
