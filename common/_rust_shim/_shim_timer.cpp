// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_timer.cpp
//
// C++ implementations of the `Common::Timer::` methods that are
// unresolved when the original C++ common/ sources are excluded.
// These weren't in the unresolved-symbols list (the original was
// Timer.cpp) but the linker still needs them when common is excluded.

#include "common/_rust_shim/_shim_common.h"

#include "common/Timer.h"

#include <chrono>

namespace Common
{
	// The Rust FFI exposes monotonic-tick primitives; the C++ class
	// `Timer` is a thin wrapper around the same concept. We forward
	// the tick queries through the Rust side and convert to seconds
	// in C++.

	Timer::Timer()
		: m_tvStartValue(GetCurrentValue())
	{
	}

	Timer::Timer(Value start_value)
		: m_tvStartValue(start_value)
	{
	}

	Timer::Value Timer::GetCurrentValue()
	{
		return static_cast<Value>(::pcsx2_timer_get_ticks());
	}

	double Timer::ConvertValueToSeconds(Value value)
	{
		const uint64_t freq = ::pcsx2_timer_get_tick_frequency();
		if (freq == 0)
			return 0.0;
		return static_cast<double>(value) / static_cast<double>(freq);
	}

	double Timer::ConvertValueToMilliseconds(Value value)
	{
		return ConvertValueToSeconds(value) * 1000.0;
	}

	double Timer::ConvertValueToNanoseconds(Value value)
	{
		return ConvertValueToSeconds(value) * 1'000'000'000.0;
	}

	Timer::Value Timer::ConvertSecondsToValue(double s)
	{
		const uint64_t freq = ::pcsx2_timer_get_tick_frequency();
		return static_cast<Value>(s * static_cast<double>(freq));
	}

	Timer::Value Timer::ConvertMillisecondsToValue(double s)
	{
		return ConvertSecondsToValue(s / 1000.0);
	}

	Timer::Value Timer::ConvertNanosecondsToValue(double ns)
	{
		return ConvertSecondsToValue(ns / 1'000'000'000.0);
	}

	void Timer::Reset()
	{
		m_tvStartValue = GetCurrentValue();
	}

	double Timer::GetTimeSeconds() const
	{
		const Value now = GetCurrentValue();
		return ConvertValueToSeconds(now - m_tvStartValue);
	}

	double Timer::GetTimeMilliseconds() const
	{
		return GetTimeSeconds() * 1000.0;
	}

	double Timer::GetTimeNanoseconds() const
	{
		return GetTimeSeconds() * 1'000'000'000.0;
	}

	double Timer::GetTimeSecondsAndReset()
	{
		const Value now = GetCurrentValue();
		const Value elapsed = now - m_tvStartValue;
		m_tvStartValue = now;
		return ConvertValueToSeconds(elapsed);
	}

	double Timer::GetTimeMillisecondsAndReset()
	{
		return GetTimeSecondsAndReset() * 1000.0;
	}

	double Timer::GetTimeNanosecondsAndReset()
	{
		return GetTimeSecondsAndReset() * 1'000'000'000.0;
	}

	bool Timer::ResetIfSecondsPassed(double s)
	{
		if (GetTimeSeconds() >= s)
		{
			Reset();
			return true;
		}
		return false;
	}

	bool Timer::ResetIfMillisecondsPassed(double s)
	{
		return ResetIfSecondsPassed(s / 1000.0);
	}

	bool Timer::ResetIfNanosecondsPassed(double s)
	{
		return ResetIfSecondsPassed(s / 1'000'000'000.0);
	}
} // namespace Common
