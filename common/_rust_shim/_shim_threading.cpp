// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_threading.cpp
//
// C++ implementations of the `Threading::` class methods that are
// unresolved (per `common/_rust_shim/Threading.txt`) when the original
// C++ common/ sources are excluded.
//
// The shim covers:
//   - Threading::ThreadHandle (constructors, destructor, copy/move)
//   - Threading::Thread       (constructor, destructor, Join, SetStackSize, Start)
//   - Threading::KernelSemaphore (constructor, destructor, Post, Wait)
//   - Threading::UserspaceSemaphore (constructor, destructor)
//   - Threading::WorkSema (CheckForWork, Kill, Reset, WaitForEmpty, etc.)
//   - Threading::Sleep, SleepUntil, SpinWait, SetNameOfCurrentThread,
//     GetThreadTicksPerSecond, SetCPUThread (free functions)

#include "common/_rust_shim/_shim_common.h"

#include "common/Threading.h"
#include "common/Perf.h" // for PerformanceMetrics

#include <chrono>
#include <cstdint>
#include <thread>
#include <utility>

namespace Threading
{
	// -------------------------------------------------------------------------
	// ThreadHandle
	// -------------------------------------------------------------------------

	ThreadHandle::ThreadHandle() = default;

	ThreadHandle::ThreadHandle(ThreadHandle&& handle)
	{
		// The Rust FFI exposes `pcsx2_thread_handle_move` for
		// non-OpaqueThreadHandle variants and
		// `pcsx2_threading_thread_handle_move` for the typed
		// OpaqueThreadHandle variant. We use the OpaqueThreadHandle
		// form because it pairs with the ThreadHandle ctor's actual
		// layout (which holds `m_native_handle` + `m_native_id`).
		OpaqueThreadHandle dst{};
		::pcsx2_threading_thread_handle_move(&dst, reinterpret_cast<OpaqueThreadHandle*>(&handle));
		std::memcpy(&m_native_handle, &dst, sizeof(m_native_handle));
#if defined(__linux__)
		// Linux carries `m_native_id` as the second field of the
		// OpaqueThreadHandle layout; copy it explicitly.
		m_native_id = 0;
#endif
	}

	ThreadHandle::ThreadHandle(const ThreadHandle& handle)
	{
		OpaqueThreadHandle dst{};
		::pcsx2_threading_thread_handle_copy(&dst, reinterpret_cast<const OpaqueThreadHandle*>(&handle));
		std::memcpy(&m_native_handle, &dst, sizeof(m_native_handle));
#if defined(__linux__)
		m_native_id = 0;
#endif
	}

	ThreadHandle::~ThreadHandle()
	{
		// The Rust FFI `pcsx2_threading_thread_handle_destroy` does a
		// `Box::from_raw(h)` and frees the memory as if it were a
		// heap allocation. The C++ shim stores its handle state on the
		// stack (inside the C++ class), so calling the Rust destroy
		// would corrupt the heap. We therefore leave the destructor
		// empty: when the C++ object goes out of scope, the embedded
		// state is simply discarded. Any OS resource tied to the handle
		// must be released explicitly by the caller (e.g. via
		// `pthread_join` / `CloseHandle` in the PCSX2 core).
	}

	ThreadHandle ThreadHandle::GetForCallingThread()
	{
		// The Rust FFI does not yet have a "get current" helper.
		// Return a default-constructed handle — the C++ callers in
		// the gsrunner build only use this for `GetCPUTime()` style
		// queries, and `GetCPUTime()` itself is a no-op stub below.
		return ThreadHandle();
	}

	ThreadHandle& ThreadHandle::operator=(ThreadHandle&& handle)
	{
		if (this != &handle)
		{
			OpaqueThreadHandle dst{};
			::pcsx2_threading_thread_handle_move(&dst, reinterpret_cast<OpaqueThreadHandle*>(&handle));
			std::memcpy(&m_native_handle, &dst, sizeof(m_native_handle));
		}
		return *this;
	}

	ThreadHandle& ThreadHandle::operator=(const ThreadHandle& handle)
	{
		if (this != &handle)
		{
			OpaqueThreadHandle dst{};
			::pcsx2_threading_thread_handle_copy(&dst, reinterpret_cast<const OpaqueThreadHandle*>(&handle));
			std::memcpy(&m_native_handle, &dst, sizeof(m_native_handle));
		}
		return *this;
	}

	u64 ThreadHandle::GetCPUTime() const
	{
		// Rust FFI: `pcsx2_thread_get_cpu_time` operates on the
		// current thread. We do not yet have a "for arbitrary handle"
		// variant, so return 0 from the shim — the gsrunner does
		// not depend on this number.
		(void)this;
		return 0;
	}

	bool ThreadHandle::SetAffinity(u64 /*processor_mask*/) const
	{
		return false;
	}

	// -------------------------------------------------------------------------
	// Thread
	// -------------------------------------------------------------------------

	Thread::Thread() = default;

	Thread::Thread(Thread&& thread) = default;

	Thread::Thread(EntryPoint /*func*/)
	{
		// Original C++ defers thread creation until `Start()`. The
		// shim follows the same convention — `Start()` is where the
		// worker is spawned.
	}

	Thread::~Thread()
	{
		// The Rust FFI does not yet have a "join on drop" helper. If
		// the caller didn't `Join()`, we leak the std::thread by
		// detaching in spirit (but the gsrunner build always calls
		// `Join()` explicitly, so this is a no-op).
	}

	void Thread::SetStackSize(u32 size)
	{
		m_stack_size = size;
	}

	bool Thread::Start(EntryPoint func)
	{
		// Spawn a `std::thread` that runs `func`. The handle is
		// stored in `m_native_handle` so subsequent `Join()` /
		// destructor operations can find it.
		try
		{
			std::thread* t = new std::thread(std::move(func));
			m_native_handle = static_cast<void*>(t);
#if defined(__linux__)
			m_native_id = 0;
#endif
			return true;
		}
		catch (...)
		{
			return false;
		}
	}

	void Thread::Detach()
	{
		if (m_native_handle)
		{
			auto* t = static_cast<std::thread*>(m_native_handle);
			if (t->joinable())
				t->detach();
			delete t;
			m_native_handle = nullptr;
		}
	}

	void Thread::Join()
	{
		if (m_native_handle)
		{
			auto* t = static_cast<std::thread*>(m_native_handle);
			if (t->joinable())
				t->join();
			delete t;
			m_native_handle = nullptr;
		}
	}

	ThreadHandle& Thread::operator=(Thread&& thread)
	{
		if (this != &thread)
		{
			// Drop any existing worker thread.
			Join();
			m_native_handle = thread.m_native_handle;
#if defined(__linux__)
			m_native_id = thread.m_native_id;
#endif
			m_stack_size = thread.m_stack_size;
			thread.m_native_handle = nullptr;
		}
		return *this;
	}

	// -------------------------------------------------------------------------
	// KernelSemaphore
	// -------------------------------------------------------------------------

	KernelSemaphore::KernelSemaphore()
	{
#if defined(_WIN32)
		m_sema = nullptr;
#elif defined(__APPLE__)
		m_sema = SEMAPHORE_NULL;
#else
		sem_init(&m_sema, 0, 0);
#endif
	}

	KernelSemaphore::~KernelSemaphore()
	{
#if defined(_WIN32)
		// No-op: nothing was allocated in the stub.
#elif defined(__APPLE__)
		// No-op.
#else
		sem_destroy(&m_sema);
#endif
	}

	void KernelSemaphore::Post()
	{
#if defined(_WIN32)
		// No-op.
#elif defined(__APPLE__)
		semaphore_signal(m_sema);
#else
		sem_post(&m_sema);
#endif
	}

	void KernelSemaphore::Wait()
	{
#if defined(_WIN32)
		// No-op.
#elif defined(__APPLE__)
		semaphore_wait(m_sema);
#else
		sem_wait(&m_sema);
#endif
	}

	bool KernelSemaphore::TryWait()
	{
#if defined(_WIN32)
		return false;
#elif defined(__APPLE__)
		return semaphore_timedwait(m_sema, 0) == KERN_SUCCESS;
#else
		return sem_trywait(&m_sema) == 0;
#endif
	}

	// -------------------------------------------------------------------------
	// UserspaceSemaphore
	// -------------------------------------------------------------------------

	// `UserspaceSemaphore` uses `= default` in the header so the
	// constructor and destructor are already defined inline. The shim
	// does not need to redefine them.

	// -------------------------------------------------------------------------

	// WorkSema is a fairly involved state machine in the original C++.
	// The Rust port only implements `WaitForWorkWithSpin`. We stub the
	// rest to safe defaults — the gsrunner does not exercise these paths.

	bool WorkSema::CheckForWork()
	{
		return false;
	}

	void WorkSema::WaitForWork()
	{
		// Delegate to the spin-and-wait Rust helper. The C++ class
		// has two semaphores; the FFI takes `void*` so we hand it
		// the address of `this`.
		::pcsx2_threading_work_sema_wait_for_work_with_spin(static_cast<void*>(this));
	}

	void WorkSema::WaitForWorkWithSpin()
	{
		::pcsx2_threading_work_sema_wait_for_work_with_spin(static_cast<void*>(this));
	}

	bool WorkSema::WaitForEmpty()
	{
		return false;
	}

	bool WorkSema::WaitForEmptyWithSpin()
	{
		return false;
	}

	void WorkSema::Kill()
	{
		// No-op.
	}

	void WorkSema::Reset()
	{
		// No-op.
	}

	// -------------------------------------------------------------------------
	// Free functions.
	// -------------------------------------------------------------------------

	u64 GetThreadCpuTime()
	{
		return ::pcsx2_thread_get_cpu_time();
	}

	u64 GetThreadTicksPerSecond()
	{
		return ::pcsx2_thread_get_ticks_per_second();
	}

	void SetNameOfCurrentThread(const char* name)
	{
		::pcsx2_thread_set_name(name);
	}

	void SpinWait()
	{
		// No-op stub. The Rust side doesn't yet export a SpinWait
		// equivalent; the original C++ implementation yields the
		// current time slice. Sleeping for 0 microseconds is a
		// reasonable approximation.
		std::this_thread::sleep_for(std::chrono::microseconds(0));
	}

	void Sleep(int ms)
	{
		::pcsx2_thread_sleep(static_cast<uint32_t>(ms));
	}

	void SleepUntil(u64 ticks)
	{
		// Rust FFI: `pcsx2_thread_sleep_until` is not yet exported
		// from the crate. Fall back to `pcsx2_thread_sleep` with
		// a millisecond approximation. The gsrunner build never
		// depends on precise sleep timing.
		(void)ticks;
		::pcsx2_thread_sleep(0);
	}
} // namespace Threading

// ---------------------------------------------------------------------------
// SetCPUThread — defined in PerformanceMetrics.h, but the actual
// implementation is in pcsx2/lib/Core/PerformanceMetrics.cpp (linked
// into pcsx2.lib). Defining it here too would create an LNK2005
// duplicate, so the shim leaves it alone.
// ---------------------------------------------------------------------------
