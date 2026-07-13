// pcsx2_capi.cpp — C API bridge for Rust/Slint UI → PCSX2 C++ core
// Includes CPUThreadInitialize fix for BIOS boot crash

#include "PrecompiledHeader.h"
#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <objbase.h>
#endif
#include <atomic>
#include "Achievements.h"
#include "ImGui/ImGuiManager.h"
#include "common/FileSystem.h"
#include "pcsx2_capi.h"
#include "VMManager.h"
#include "Host.h"
#include "Config.h"
#include "MTGS.h"
#include "CDVD/CDVDcommon.h"
#include "common/Error.h"
#include "common/ProgressCallback.h"
#include "common/Path.h"
#include "common/StringUtil.h"
#include "common/WindowInfo.h"
#include "common/Console.h"
#include "BuildVersion.h"
#include "Input/InputManager.h"
#include "INISettingsInterface.h"
#include "PerformanceMetrics.h"
#include "Memory.h"
#include "R5900.h"
#include "R3000A.h"
#include "SIO/Memcard/MemoryCardFile.h"
#include "ps2/BiosTools.h"
#include "GS.h"
#include "GS/Renderers/Common/GSDevice.h"
#include "SPU2/spu2.h"
#include "USB/USB.h"
#include "DEV9/DEV9.h"
#include "FW.h"
#include "Counters.h"
#include "DebugTools/DebugInterface.h"
#include "DebugTools/Breakpoints.h"
#include "fmt/format.h"
#include <cstring>
#include <string>
#include <vector>
#include <mutex>

#ifdef _WIN32
#include "common/RedtapeWindows.h"
#endif

// ═══════════════════════════════════════════════════════════════
// CALLBACK FUNCTION POINTERS
// ═══════════════════════════════════════════════════════════════

static PCSX2_OnVMStarting cb_vm_starting = nullptr;
static PCSX2_OnVMStarted cb_vm_started = nullptr;
static PCSX2_OnVMResumed cb_vm_resumed = nullptr;
static PCSX2_OnVMPaused cb_vm_paused = nullptr;
static PCSX2_OnVMDestroyed cb_vm_destroyed = nullptr;
static PCSX2_OnGameChanged cb_game_changed = nullptr;
static PCSX2_OnSaveStateLoading cb_save_state_loading = nullptr;
static PCSX2_OnSaveStateLoaded cb_save_state_loaded = nullptr;
static PCSX2_OnSaveStateSaved cb_save_state_saved = nullptr;
static PCSX2_OnError cb_error = nullptr;
static PCSX2_OnInfo cb_info = nullptr;
static PCSX2_OnFrame cb_frame = nullptr;

// ═══════════════════════════════════════════════════════════════
// GLOBALS
static bool s_cpu_initialized = false;
static INISettingsInterface* s_ini_settings = nullptr;
static HWND s_hwnd = nullptr;
static HWND s_parent_hwnd = nullptr;
static std::thread s_msg_pump_thread;
static std::atomic<bool> s_msg_pump_running{false};

// Frame capture buffer (shared between GS thread and Slint UI thread)
static std::vector<u8> s_frame_buffer;
static int s_frame_width = 0;
static int s_frame_height = 0;
static std::atomic<bool> s_frame_ready{false};

// ═══════════════════════════════════════════════════════════════
// HOST CALLBACKS — Called by PCSX2 core
// ═══════════════════════════════════════════════════════════════

void Host::OnVMStarting() { fprintf(stderr, "[Host] OnVMStarting\n"); if (cb_vm_starting) cb_vm_starting(); }
void Host::OnVMStarted() { fprintf(stderr, "[Host] OnVMStarted\n"); if (cb_vm_started) cb_vm_started(); }
void Host::OnVMDestroyed() { fprintf(stderr, "[Host] OnVMDestroyed\n"); if (cb_vm_destroyed) cb_vm_destroyed(); }
void Host::OnVMPaused() { fprintf(stderr, "[Host] OnVMPaused\n"); if (cb_vm_paused) cb_vm_paused(); }
void Host::OnVMResumed() { fprintf(stderr, "[Host] OnVMResumed\n"); if (cb_vm_resumed) cb_vm_resumed(); }

void Host::OnGameChanged(const std::string& title, const std::string& elf_override,
    const std::string& disc_path, const std::string& serial, u32 disc_crc, u32 crc)
{
    fprintf(stderr, "[Host] OnGameChanged: path=%s serial=%s\n", disc_path.c_str(), serial.c_str());
    if (cb_game_changed)
        cb_game_changed(disc_path.c_str(), serial.c_str(), title.c_str());
}

void Host::ReportErrorAsync(const std::string_view title, const std::string_view message)
{
    fprintf(stderr, "[Host] Error: %.*s — %.*s\n", (int)title.size(), title.data(), (int)message.size(), message.data());
    if (cb_error) cb_error(std::string(title).c_str(), std::string(message).c_str());
}

void Host::ReportInfoAsync(const std::string_view title, const std::string_view message)
{
    fprintf(stderr, "[Host] Info: %.*s — %.*s\n", (int)title.size(), title.data(), (int)message.size(), message.data());
    if (cb_info) cb_info(std::string(title).c_str(), std::string(message).c_str());
}

// OSD stubs removed — ImGuiManager.cpp in rlib provides real implementations

// Required Host stubs
bool Host::InBatchMode() { return false; }
bool Host::InNoGUIMode() { return false; }
void Host::OpenURL(const std::string_view url) {}
bool Host::CopyTextToClipboard(const std::string_view text) { return false; }
std::string Host::GetTextFromClipboard() { return ""; }
bool Host::RequestResetSettings(bool folders, bool core, bool controllers, bool hotkeys, bool ui) { return false; }
void Host::RequestResizeHostDisplay(s32 width, s32 height) {}
void Host::RunOnCPUThread(std::function<void()> function, bool block) { function(); }
void Host::RunOnGSThread(std::function<void()> function) {}
void Host::RefreshGameListAsync(bool invalidate_cache) {}
void Host::CancelGameListRefresh() {}
void Host::RequestVMShutdown(bool allow_confirm, bool allow_save_state, bool default_save_state) {}
std::string Host::GetHTTPUserAgent() { return "lumine-sx2"; }
void Host::SetDefaultUISettings(SettingsInterface& si) {}
std::unique_ptr<ProgressCallback> Host::CreateHostProgressCallback() { return nullptr; }
int Host::LocaleSensitiveCompare(std::string_view lhs, std::string_view rhs) { return 0; }
std::string Host::TranslatePluralToString(const char* context, const char* msg, const char* disambiguation, int count) { return msg ? msg : ""; }
void Host::OnSaveStateLoading(const std::string_view filename) { if (cb_save_state_loading) cb_save_state_loading(std::string(filename).c_str()); }
void Host::OnSaveStateLoaded(const std::string_view filename, bool was_successful) { if (cb_save_state_loaded) cb_save_state_loaded(std::string(filename).c_str(), was_successful); }
void Host::OnSaveStateSaved(const std::string_view filename) { if (cb_save_state_saved) cb_save_state_saved(std::string(filename).c_str(), true, ""); }
void Host::OnCaptureStarted(const std::string& filename) {}
void Host::OnCaptureStopped() {}
void Host::CommitBaseSettingChanges() {
    fprintf(stderr, "[Host] CommitBaseSettingChanges\n");
    if (s_ini_settings) {
        Error error;
        if (!s_ini_settings->Save(&error))
            fprintf(stderr, "[Host] Failed to save settings: %s\n", error.GetDescription().c_str());
    }
}

// Forward declarations for Host:: functions not in headers but needed by rlib objects
namespace Host {
    void SetFullscreen(bool);
    bool IsFullscreen();
    void BeginPresentFrame();
    void RequestExitApplication(bool);
    void RequestExitBigPicture();
    bool LocaleCircleConfirm();
    void BeginTextInput();
    void EndTextInput();
    void OnInputDeviceConnected(std::string_view, std::string_view);
    void OnInputDeviceDisconnected(InputBindingKey, std::string_view);
    void SetMouseLock(bool);
    void SetMouseMode(bool, bool);
    bool ShouldPreferHostFileSelector();
    void OpenHostFileSelectorAsync(std::string_view, bool, std::function<void(const std::string&)>, std::vector<std::string>, std::string_view);
    void CheckForSettingsChanges(const Pcsx2Config&);
    void LoadSettings(SettingsInterface&, std::unique_lock<std::mutex>&);
    void PumpMessagesOnCPUThread();
    void OnPerformanceMetricsUpdated();
    void OnAchievementsLoginRequested(Achievements::LoginRequestReason);
    void OnAchievementsLoginSuccess(const char*, u32, u32, u32);
    void OnAchievementsRefreshed();
    void OnAchievementsHardcoreModeChanged(bool);
    void OnSaveStateLoading(std::string_view);
    void OnSaveStateLoaded(std::string_view, bool);
    void OnSaveStateSaved(std::string_view);
    void OnCaptureStarted(const std::string&);
    void OnCaptureStopped();
    void CommitBaseSettingChanges();
    namespace Internal { s32 GetTranslatedStringImpl(std::string_view, std::string_view, char*, size_t); }
}
namespace InputManager {
    std::optional<u32> ConvertHostKeyboardStringToCode(std::string_view);
    std::optional<std::string> ConvertHostKeyboardCodeToString(u32);
    const char* ConvertHostKeyboardCodeToIcon(u32);
}

// Implementations
void Host::SetFullscreen(bool) {}
bool Host::IsFullscreen() { return false; }

// Forward declaration
static void CaptureFrame();

void Host::BeginPresentFrame() {
    // Debug: write to file to verify this is called
    static int frame_count = 0;
    frame_count++;
    if (frame_count == 1) {
        FILE* f = fopen("C:\\Users\\X\\Documents\\PCSX2\\logs\\beginpresent.log", "w");
        if (f) { fprintf(f, "BeginPresentFrame called!\n"); fclose(f); }
    }

    // Capture frame for Slint display
    CaptureFrame();
}
void Host::RequestExitApplication(bool) {}
void Host::RequestExitBigPicture() {}
bool Host::LocaleCircleConfirm() { return false; }
void Host::BeginTextInput() {}
void Host::EndTextInput() {}
void Host::OnInputDeviceConnected(std::string_view, std::string_view) {}
void Host::OnInputDeviceDisconnected(InputBindingKey, std::string_view) {}
void Host::SetMouseLock(bool) {}
void Host::SetMouseMode(bool, bool) {}
bool Host::ShouldPreferHostFileSelector() { return false; }
void Host::OpenHostFileSelectorAsync(std::string_view, bool, std::function<void(const std::string&)>, std::vector<std::string>, std::string_view) {}
void Host::CheckForSettingsChanges(const Pcsx2Config&) {}
void Host::LoadSettings(SettingsInterface&, std::unique_lock<std::mutex>&) {
    // Vulkan renderer (cross-platform, Android compatible)
    EmuConfig.GS.Renderer = GSRendererType::VK;
}
void Host::PumpMessagesOnCPUThread() {
    // Message pump runs on dedicated thread now
}
void Host::OnPerformanceMetricsUpdated() {}
void Host::OnAchievementsLoginRequested(Achievements::LoginRequestReason) {}
void Host::OnAchievementsLoginSuccess(const char*, u32, u32, u32) {}
void Host::OnAchievementsRefreshed() {}
void Host::OnAchievementsHardcoreModeChanged(bool) {}
s32 Host::Internal::GetTranslatedStringImpl(std::string_view, std::string_view, char* buf, size_t buf_len) { if (buf && buf_len > 0) buf[0] = 0; return 0; }
std::optional<u32> InputManager::ConvertHostKeyboardStringToCode(std::string_view) { return std::nullopt; }
std::optional<std::string> InputManager::ConvertHostKeyboardCodeToString(u32) { return std::nullopt; }
const char* InputManager::ConvertHostKeyboardCodeToIcon(u32) { return nullptr; }

// ═══════════════════════════════════════════════════════════════
// GS WINDOW (needed for rendering)
// ═══════════════════════════════════════════════════════════════

static int s_surface_width = 640;
static int s_surface_height = 480;

static LRESULT CALLBACK GSWndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
        case WM_CLOSE: return 0; // prevent closing
        case WM_DESTROY: PostQuitMessage(0); return 0;
    }
    return DefWindowProcA(hwnd, msg, wp, lp);
}

static void StartMessagePumpThread() {
    if (s_msg_pump_running.exchange(true)) return;
    s_msg_pump_thread = std::thread([]() {
        fprintf(stderr, "[GS] Message pump thread started\n");
        MSG msg;
        while (s_msg_pump_running && GetMessage(&msg, nullptr, 0, 0)) {
            TranslateMessage(&msg);
            DispatchMessage(&msg);
        }
        fprintf(stderr, "[GS] Message pump thread exited\n");
    });
}

void Host::ReleaseRenderWindow() {
    s_msg_pump_running = false;
    if (s_hwnd) PostMessage(s_hwnd, WM_CLOSE, 0, 0);
    if (s_msg_pump_thread.joinable()) s_msg_pump_thread.join();
    if (s_hwnd) { DestroyWindow(s_hwnd); s_hwnd = nullptr; }
}

std::optional<WindowInfo> Host::AcquireRenderWindow(bool recreate_window)
{
    fprintf(stderr, "[Host] AcquireRenderWindow\n");
    if (!s_hwnd)
    {
        WNDCLASSEXA wc = {};
        wc.cbSize = sizeof(wc);
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = GSWndProc;
        wc.hInstance = GetModuleHandleA(nullptr);
        wc.hCursor = LoadCursor(nullptr, IDC_ARROW);
        wc.hbrBackground = (HBRUSH)GetStockObject(BLACK_BRUSH);
        wc.lpszClassName = "RS-PS2-GS";
        RegisterClassExA(&wc);

        DWORD style = s_parent_hwnd ? (WS_CHILD | WS_VISIBLE) : WS_OVERLAPPEDWINDOW;
        HWND parent = s_parent_hwnd ? s_parent_hwnd : nullptr;
        s_hwnd = CreateWindowExA(0, wc.lpszClassName, "RS-PS2", style,
            0, 0, 640, 480, parent, nullptr, wc.hInstance, nullptr);
        if (s_hwnd) {
            if (!s_parent_hwnd) {
                // Standalone window: hide off-screen
                SetWindowPos(s_hwnd, nullptr, -32000, -32000, 640, 480, SWP_HIDEWINDOW);
            } else {
                // Child window: position in Slint UI
                ShowWindow(s_hwnd, SW_SHOW);
            }
            RECT rc_dbg;
            GetClientRect(s_hwnd, &rc_dbg);
            fprintf(stderr, "[Host] GS HWND: client=%dx%d child=%d\n",
                rc_dbg.right - rc_dbg.left, rc_dbg.bottom - rc_dbg.top, s_parent_hwnd ? 1 : 0);
        }
    }
    RECT rc;
    GetClientRect(s_hwnd, &rc);
    WindowInfo wi = {};
    wi.type = WindowInfo::Type::Win32;
    wi.window_handle = s_hwnd;
    wi.surface_width = rc.right - rc.left;
    wi.surface_height = rc.bottom - rc.top;
    return wi;
}

std::optional<WindowInfo> Host::GetTopLevelWindowInfo()
{
    if (s_hwnd)
    {
        RECT rc;
        GetClientRect(s_hwnd, &rc);
        WindowInfo wi = {};
        wi.type = WindowInfo::Type::Win32;
        wi.window_handle = s_hwnd;
        wi.surface_width = rc.right - rc.left;
        wi.surface_height = rc.bottom - rc.top;
        return wi;
    }
    return std::nullopt;
}

// ═══════════════════════════════════════════════════════════════
// FRAME CAPTURE
// ═══════════════════════════════════════════════════════════════

static void CaptureFrame()
{
    if (!g_gs_device) return;

    // Rate-limit to ~60fps
    static u64 s_last_capture = 0;
    u64 now = GetTickCount64();
    if (now - s_last_capture < 15) return;
    s_last_capture = now;

    // Capture via PCSX2's built-in GPU readback
    u32 w = 0, h = 0;
    std::vector<u32> pixels;
    if (!GSSaveSnapshotToMemory(0, 0, true, false, &w, &h, &pixels) || w == 0 || h == 0)
        return;

    // Copy to shared buffer (converting BGRA/u32 to RGBA bytes)
    s_frame_width = static_cast<int>(w);
    s_frame_height = static_cast<int>(h);
    s_frame_buffer.resize(w * h * 4);
    {
        u8* dst = s_frame_buffer.data();
        const u32* src = pixels.data();
        for (u32 i = 0; i < w * h; i++)
        {
            u32 c = src[i];
            // ABGR (host byte order) -> RGBA
            dst[i * 4 + 0] = (c >> 16) & 0xFF;  // R (from B)
            dst[i * 4 + 1] = (c >> 8) & 0xFF;   // G
            dst[i * 4 + 2] = c & 0xFF;           // B (from R)
            dst[i * 4 + 3] = (c >> 24) & 0xFF;  // A
        }
    }
    s_frame_ready = true;
}

// ═══════════════════════════════════════════════════════════════
// C API — CALLBACK REGISTRATION
// ═══════════════════════════════════════════════════════════════

extern "C" {

void pcsx2_register_callbacks(
    PCSX2_OnVMStarting on_vm_starting,
    PCSX2_OnVMStarted on_vm_started,
    PCSX2_OnVMPaused on_vm_paused,
    PCSX2_OnVMResumed on_vm_resumed,
    PCSX2_OnVMDestroyed on_vm_destroyed,
    PCSX2_OnGameChanged on_game_changed,
    PCSX2_OnSaveStateLoading on_save_state_loading,
    PCSX2_OnSaveStateLoaded on_save_state_loaded,
    PCSX2_OnSaveStateSaved on_save_state_saved,
    PCSX2_OnError on_error,
    PCSX2_OnInfo on_info,
    PCSX2_OnFrame on_frame)
{
    fprintf(stderr, "[CAPI] register_callbacks\n");
    cb_vm_starting = on_vm_starting;
    cb_vm_started = on_vm_started;
    cb_vm_paused = on_vm_paused;
    cb_vm_resumed = on_vm_resumed;
    cb_vm_destroyed = on_vm_destroyed;
    cb_game_changed = on_game_changed;
    cb_save_state_loading = on_save_state_loading;
    cb_save_state_loaded = on_save_state_loaded;
    cb_save_state_saved = on_save_state_saved;
    cb_error = on_error;
    cb_info = on_info;
    cb_frame = on_frame;
}

// ═══════════════════════════════════════════════════════════════
// INITIALIZATION & BOOT — with CPUThreadInitialize fix
// ═══════════════════════════════════════════════════════════════



static void EnsureCPUThreadInit()
{
    if (s_cpu_initialized) return;
    fprintf(stderr, "[CAPI] Calling CPUThreadInitialize...\n");
#ifdef _WIN32
    // Pre-init COM on this worker thread with COINIT_MULTITHREADED
    // so CPUThreadInitialize's CoInitializeEx succeeds
    HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    fprintf(stderr, "[CAPI] CoInitializeEx: %08X\n", (u32)hr);
    // S_OK=success, S_FALSE=already init, RPC_E_CHANGED_MODE=mode conflict (non-fatal)
#endif
    if (!VMManager::Internal::CPUThreadInitialize())
    {
        fprintf(stderr, "[CAPI] *** CPUThreadInitialize FAILED (non-fatal, continuing) ***\n");
        // Don't return — PS2 memory may not be allocated, but try anyway
    }
    s_cpu_initialized = true;
    fprintf(stderr, "[CAPI] CPUThreadInitialize done\n");
}

bool pcsx2_initialize(const char* bios_dir)
{
    fprintf(stderr, "[CAPI] pcsx2_initialize(bios_dir=%s)\n", bios_dir ? bios_dir : "(null)");
    EmuFolders::SetAppRoot();
    // Set resources directory to PCSX2 bin/resources (for shaders)
    EmuFolders::Resources = "E:\\project\\pcsx2\\bin\\resources";
    fprintf(stderr, "[CAPI] Resources=%s\n", EmuFolders::Resources.c_str());

    // Set up default ImGui fonts (required for GS init)
    {
        static std::vector<u8> s_font_data;
        static std::vector<ImGuiManager::FontInfo> s_fonts;
        std::string font_path = EmuFolders::Resources + "\\fonts\\Roboto-Regular.ttf";
        std::optional<std::vector<u8>> font_data = FileSystem::ReadBinaryFile(font_path.c_str());
        if (font_data.has_value()) {
            s_font_data = std::move(font_data.value());
            ImGuiManager::FontInfo fi;
            fi.data = s_font_data;
            fi.face_name = "Roboto";
            fi.is_emoji_font = false;
            s_fonts.push_back(fi);
            ImGuiManager::SetFonts(std::move(s_fonts));
            fprintf(stderr, "[CAPI] Fonts initialized\n");
        } else {
            fprintf(stderr, "[CAPI] WARNING: Could not load font from %s\n", font_path.c_str());
        }
    }

    Error error;
    EmuFolders::SetDataDirectory(&error);
    fprintf(stderr, "[CAPI] DataRoot=%s\n", EmuFolders::DataRoot.c_str());

    std::string ini_path = Path::Combine(EmuFolders::Settings, "PCSX2.ini");
    auto* sif = new INISettingsInterface(std::move(ini_path));
    s_ini_settings = sif; // save for CommitBaseSettingChanges
    sif->Load();
    Host::Internal::SetBaseSettingsLayer(sif);
    EmuFolders::LoadConfig(*sif);
    EmuFolders::EnsureFoldersExist();

    if (bios_dir && bios_dir[0])
        EmuFolders::Bios = bios_dir;

    fprintf(stderr, "[CAPI] Init done. Bios=%s\n", EmuFolders::Bios.c_str());
    return true;
}

bool pcsx2_set_bios_dir(const char* path)
{
    if (!path || !path[0]) return false;
    EmuFolders::Bios = path;
    return true;
}

const char* pcsx2_get_bios_dir() { return EmuFolders::Bios.c_str(); }

bool pcsx2_boot(const PCSX2_BootParams* params)
{
    fprintf(stderr, "[CAPI] ================= pcsx2_boot =================\n");

    VMBootParameters boot_params;
    if (params && params->filename && params->filename[0])
        boot_params.filename = params->filename;
    if (params)
        boot_params.fast_boot = params->fast_boot;

    // Force Null GS in INI so LoadConfig picks it up
    Host::SetBaseIntSettingValue("EmuCore/GS", "Renderer", (int)GSRendererType::VK);
    fprintf(stderr, "[CAPI] SetBaseIntSettingValue EmuCore/GS/Renderer=%d (VK)\n", (int)GSRendererType::VK);
    Host::CommitBaseSettingChanges();
    EmuConfig.GS.Renderer = GSRendererType::VK;

    // Verify INI was updated
    {
        int val = Host::GetBaseIntSettingValue("EmuCore/GS", "Renderer", -999);
        fprintf(stderr, "[CAPI] Verified EmuCore/GS/Renderer=%d (expected %d)\n", val, (int)GSRendererType::Null);
    }

    // CRITICAL: Must call CPUThreadInitialize BEFORE InitializeAsync!
    EnsureCPUThreadInit();

    fprintf(stderr, "[CAPI] Calling InitializeAsync...\n");

    auto hardcore_disable = [](std::string reason, VMBootRestartCallback restart) {
        fprintf(stderr, "[CAPI] Hardcore disable: %s\n", reason.c_str());
        restart();
    };

    auto done_callback = [](VMBootResult result, const Error& err) {
        if (result == VMBootResult::StartupSuccess) {
            fprintf(stderr, "[CAPI] *** BOOT SUCCESS ***\n");
            VMManager::SetState(VMState::Running);
        } else {
            fprintf(stderr, "[CAPI] *** BOOT FAILED (result=%d) ***\n", (int)result);
        }
    };

    VMManager::InitializeAsync(boot_params, std::move(hardcore_disable), std::move(done_callback));

    fprintf(stderr, "[CAPI] InitializeAsync done\n");
    fprintf(stderr, "[CAPI] InitializeAsync returned\n");
    return true;
}

bool pcsx2_boot_disc(const char* disc_path, bool fast_boot)
{
    PCSX2_BootParams params = {};
    params.filename = disc_path;
    params.fast_boot = fast_boot;
    return pcsx2_boot(&params);
}

void pcsx2_shutdown()
{
    fprintf(stderr, "[CAPI] pcsx2_shutdown\n");
    VMManager::SetState(VMState::Stopping);
}

void pcsx2_reset() { VMManager::Reset(); }
void pcsx2_set_state(PCSX2_VMState state) { VMManager::SetState(static_cast<VMState>(state)); }
void pcsx2_set_paused(bool paused) { VMManager::SetPaused(paused); }
PCSX2_VMState pcsx2_get_state() { return static_cast<PCSX2_VMState>(VMManager::GetState()); }
bool pcsx2_has_valid_vm() { return VMManager::HasValidVM(); }

// ═══════════════════════════════════════════════════════════════
// SAVE STATES
// ═══════════════════════════════════════════════════════════════

bool pcsx2_save_state_to_file(const char* filename) {
    if (!filename || !VMManager::HasValidVM()) return false;
    VMManager::SaveState(filename, true, true, nullptr);
    return true;
}

bool pcsx2_load_state_from_file(const char* filename) {
    if (!filename || !VMManager::HasValidVM()) return false;
    return VMManager::LoadState(filename);
}

bool pcsx2_save_state_to_slot(int32_t slot) {
    if (!VMManager::HasValidVM()) return false;
    VMManager::SaveStateToSlot(slot, true, nullptr);
    return true;
}

bool pcsx2_load_state_from_slot(int32_t slot) {
    if (!VMManager::HasValidVM()) return false;
    return VMManager::LoadStateFromSlot(slot);
}

// Aliases matching Rust FFI expectations
bool pcsx2_save_state(int32_t slot) { return pcsx2_save_state_to_slot(slot); }
bool pcsx2_load_state(int32_t slot) { return pcsx2_load_state_from_slot(slot); }
bool pcsx2_has_save_state(int32_t slot) {
    // HasSaveStateInSlot needs serial+crc, stub for now
    return false;
}

// ═══════════════════════════════════════════════════════════════
// DISC
// ═══════════════════════════════════════════════════════════════

void pcsx2_change_disc(const char* path) {
    if (path && path[0]) VMManager::ChangeDisc(CDVD_SourceType::Iso, path);
}

const char* pcsx2_get_disc_serial() {
    static thread_local std::string s;
    s = VMManager::GetDiscSerial();
    return s.c_str();
}

const char* pcsx2_get_disc_path() {
    static thread_local std::string s;
    s = VMManager::GetDiscPath();
    return s.c_str();
}

const char* pcsx2_get_title() {
    static thread_local std::string s;
    s = VMManager::GetTitle(false);
    return s.c_str();
}

bool pcsx2_copy_to_clipboard(const char* text) { return Host::CopyTextToClipboard(text ? text : ""); }
const char* pcsx2_get_from_clipboard() {
    static thread_local std::string s;
    s = Host::GetTextFromClipboard();
    return s.c_str();
}
void pcsx2_refresh_game_list(bool invalidate) { Host::RefreshGameListAsync(invalidate); }
void pcsx2_cancel_game_list_refresh() { Host::CancelGameListRefresh(); }
int pcsx2_get_limiter_mode() { return static_cast<int>(VMManager::GetLimiterMode()); }
void pcsx2_set_limiter_mode(int mode) { VMManager::SetLimiterMode(static_cast<LimiterModeType>(mode)); }
void pcsx2_free_string(const char* s) {} // strings are thread_local statics

// ═══════════════════════════════════════════════════════════════
// SETTINGS
// ═══════════════════════════════════════════════════════════════

bool pcsx2_get_bool_setting(const char* s, const char* k, bool d) { return Host::GetBaseBoolSettingValue(s, k, d); }
void pcsx2_set_bool_setting(const char* s, const char* k, bool v) { Host::SetBaseBoolSettingValue(s, k, v); }
int pcsx2_get_int_setting(const char* s, const char* k, int d) { return Host::GetBaseIntSettingValue(s, k, d); }
void pcsx2_set_int_setting(const char* s, const char* k, int v) { Host::SetBaseIntSettingValue(s, k, v); }
float pcsx2_get_float_setting(const char* s, const char* k, float d) { return Host::GetBaseFloatSettingValue(s, k, d); }
void pcsx2_set_float_setting(const char* s, const char* k, float v) { Host::SetBaseFloatSettingValue(s, k, v); }

const char* pcsx2_get_string_setting(const char* s, const char* k, const char* d) {
    static thread_local std::string r;
    r = Host::GetBaseStringSettingValue(s, k, d ? d : "");
    return r.c_str();
}
void pcsx2_set_string_setting(const char* s, const char* k, const char* v) { Host::SetBaseStringSettingValue(s, k, v ? v : ""); }

void pcsx2_commit_settings() { Host::CommitBaseSettingChanges(); }
void pcsx2_apply_settings() { VMManager::ApplySettings(); }
void pcsx2_reload_game_settings() { VMManager::ReloadGameSettings(); }
void pcsx2_reload_input_bindings() { VMManager::ReloadInputBindings(); }
bool pcsx2_contains_setting(const char* s, const char* k) { return Host::ContainsBaseSettingValue(s, k); }
void pcsx2_remove_setting(const char* s, const char* k) { Host::RemoveBaseSettingValue(s, k); }
void pcsx2_reload_input_devices() { InputManager::ReloadDevices(); }

// ═══════════════════════════════════════════════════════════════
// OSD & PERFORMANCE
// ═══════════════════════════════════════════════════════════════

void pcsx2_osd_message(const char* msg, float dur) {}
void pcsx2_osd_clear() {}
float pcsx2_get_current_fps() { return PerformanceMetrics::GetFPS(); }
float pcsx2_get_current_speed() { return PerformanceMetrics::GetSpeed(); }

// ═══════════════════════════════════════════════════════════════
// HOTKEYS
// ═══════════════════════════════════════════════════════════════

const HotkeyInfo g_host_hotkeys[1] = {};

// Hotkey capture state (for "press a key" UI).
// When a capture is active, InputManager routes the next event to our hook.
static std::atomic<bool> s_capturing(false);
static std::string s_captured_key;
static std::mutex s_capture_mutex;

// InputInterceptHook callback — returns the captured key string and stops capture.
static InputInterceptHook::CallbackResult HotkeyCaptureHook(InputBindingKey key, float value)
{
	if (value <= 0.0f)
		return InputInterceptHook::CallbackResult::ContinueProcessingEvent;

	// Convert the key to a human-readable binding string and store it.
	std::string str = InputManager::ConvertInputBindingKeyToString(InputBindingInfo::Type::Button, key);
	if (!str.empty())
	{
		std::unique_lock lock(s_capture_mutex);
		s_captured_key = std::move(str);
	}
	s_capturing.store(false);
	InputManager::RemoveHook();
	return InputInterceptHook::CallbackResult::RemoveHookAndStopProcessingEvent;
}

const char* pcsx2_get_hotkey_list()
{
	static thread_local std::string s;
	const std::vector<const HotkeyInfo*> hotkeys(InputManager::GetHotkeyList());
	s.clear();
	for (const HotkeyInfo* hk : hotkeys)
	{
		if (!s.empty())
			s += "\n";
		s += hk->name;
		s += "|";
		s += hk->category ? hk->category : "";
		s += "|";
		s += hk->display_name ? hk->display_name : "";
	}
	return s.c_str();
}

const char* pcsx2_get_hotkey_binding(const char* name)
{
	static thread_local std::string s;
	s = Host::GetBaseStringListSetting("Hotkeys", name).empty()
			? ""
			: Host::GetBaseStringListSetting("Hotkeys", name).front();
	return s.c_str();
}

void pcsx2_set_hotkey_binding(const char* name, const char* binding)
{
	if (!name || !binding)
		return;
	std::vector<std::string> list = Host::GetBaseStringListSetting("Hotkeys", name);
	list.clear();
	list.push_back(binding);
	Host::SetBaseStringListSettingValue("Hotkeys", name, list);
}

void pcsx2_clear_hotkey_binding(const char* name)
{
	if (!name)
		return;
	std::vector<std::string> empty;
	Host::SetBaseStringListSettingValue("Hotkeys", name, empty);
}

void pcsx2_capture_hotkey_begin()
{
	s_capturing.store(true);
	{
		std::unique_lock lock(s_capture_mutex);
		s_captured_key.clear();
	}
	InputManager::SetHook(HotkeyCaptureHook);
}

bool pcsx2_capture_hotkey_poll(char* out, int32_t size)
{
	if (!s_capturing.load())
	{
		std::unique_lock lock(s_capture_mutex);
		if (s_captured_key.empty())
			return false;
		const int n = std::min<int>(static_cast<int>(s_captured_key.size()), size - 1);
		std::memcpy(out, s_captured_key.c_str(), n);
		out[n] = '\0';
		s_captured_key.clear();
		return true;
	}
	return false;
}

void pcsx2_capture_hotkey_cancel()
{
	s_capturing.store(false);
	InputManager::RemoveHook();
	std::unique_lock lock(s_capture_mutex);
	s_captured_key.clear();
}

// ═══════════════════════════════════════════════════════════════
// DEBUG INTERFACE
// ═══════════════════════════════════════════════════════════════

static DebugInterface* GetDI(int cpu_type) { return &DebugInterface::get(static_cast<BreakPointCpu>(cpu_type)); }

bool DebugInterface_isAlive(int t) { return GetDI(t)->isAlive(); }
u32 DebugInterface_getPC(int t) { return GetDI(t)->getPC(); }
void DebugInterface_setPC(int t, u32 pc) { GetDI(t)->setPc(pc); }
int DebugInterface_getRegisterCount(int t, int cat) { return GetDI(t)->getRegisterCount(cat); }

const char* DebugInterface_getRegisterName(int t, int cat, int idx) {
    static thread_local std::string s;
    s = GetDI(t)->getRegisterName(cat, idx);
    return s.c_str();
}

u32 DebugInterface_getRegister(int t, int cat, int idx) { return GetDI(t)->getRegister(cat, idx).lo; }
void DebugInterface_setRegister(int t, int cat, int idx, u32 v) { GetDI(t)->setRegister(cat, idx, u128::From32(v)); }

void DebugInterface_getRegister128(int t, int cat, int idx, u8 out[16]) {
    u128 val = GetDI(t)->getRegister(cat, idx);
    memcpy(out, &val, 16);
}

void DebugInterface_setRegister128(int t, int cat, int idx, const u8 data[16]) {
    u128 val; memcpy(&val, data, 16);
    GetDI(t)->setRegister(cat, idx, val);
}

void pcsx2_execute() {
    // Pump Android/Win32 messages inside execute to avoid deadlock
    // when GS needs window messages to complete swap chain presentation
    MSG msg;
    while (PeekMessage(&msg, s_hwnd, 0, 0, PM_REMOVE)) {
        TranslateMessage(&msg);
        DispatchMessage(&msg);
    }
    VMManager::Execute();
}

void pcsx2_pump_messages() {
    MSG msg;
    while (PeekMessage(&msg, s_hwnd, 0, 0, PM_REMOVE)) {
        TranslateMessage(&msg);
        DispatchMessage(&msg);
    }
}

void pcsx2_set_render_parent(void* parent, int x, int y, int w, int h) {
    s_parent_hwnd = reinterpret_cast<HWND>(parent);
    if (s_hwnd) {
        SetParent(s_hwnd, reinterpret_cast<HWND>(parent));
        SetWindowPos(s_hwnd, nullptr, x, y, w, h, SWP_SHOWWINDOW);
    }
}

void pcsx2_resize_render(int x, int y, int w, int h) {
    if (s_hwnd) {
        SetWindowPos(s_hwnd, nullptr, x, y, w, h, SWP_NOZORDER);
    }
}

int DebugInterface_getRegisterSize(int t, int cat) { return GetDI(t)->getRegisterSize(cat); }
int DebugInterface_getRegisterCategoryCount(int t) { return GetDI(t)->getRegisterCategoryCount(); }

const char* DebugInterface_getRegisterCategoryName(int t, int idx) {
    static thread_local std::string s;
    s = GetDI(t)->getRegisterCategoryName(idx);
    return s.c_str();
}

u8 DebugInterface_Read8(int t, u32 a) { return GetDI(t)->Read8(a); }
u16 DebugInterface_Read16(int t, u32 a) { return GetDI(t)->Read16(a); }
u32 DebugInterface_Read32(int t, u32 a) { return GetDI(t)->Read32(a); }
u64 DebugInterface_Read64(int t, u32 a) { return GetDI(t)->Read64(a); }
void DebugInterface_Read128(int t, u32 a, u8 out[16]) { u128 v = GetDI(t)->Read128(a); memcpy(out, &v, 16); }
void DebugInterface_Write8(int t, u32 a, u8 v) { GetDI(t)->Write8(a, v); }
void DebugInterface_Write16(int t, u32 a, u16 v) { GetDI(t)->Write16(a, v); }
void DebugInterface_Write32(int t, u32 a, u32 v) { GetDI(t)->Write32(a, v); }
void DebugInterface_Write64(int t, u32 a, u64 v) { GetDI(t)->Write64(a, v); }
void DebugInterface_Write128(int t, u32 a, const u8 data[16]) { u128 v; memcpy(&v, data, 16); GetDI(t)->Write128(a, v); }

bool CBreakPoints_IsAddressBreakPoint(int t, u32 a, bool* en) { return CBreakPoints::IsAddressBreakPoint(static_cast<BreakPointCpu>(t), a, en); }
void CBreakPoints_AddBreakPoint(int t, u32 a, bool en) { CBreakPoints::AddBreakPoint(static_cast<BreakPointCpu>(t), a, false, en, false); }
void CBreakPoints_RemoveBreakPoint(int t, u32 a) { CBreakPoints::RemoveBreakPoint(static_cast<BreakPointCpu>(t), a); }

void CBreakPoints_SwitchBreakPoint(int t, u32 a) {
    bool en = false;
    if (CBreakPoints::IsAddressBreakPoint(static_cast<BreakPointCpu>(t), a, &en))
        CBreakPoints::ChangeBreakPoint(static_cast<BreakPointCpu>(t), a, !en);
    else
        CBreakPoints::AddBreakPoint(static_cast<BreakPointCpu>(t), a, false, true, false);
}

u32 CBreakPoints_GetBreakpointCount(int t) { return static_cast<u32>(CBreakPoints::GetNumBreakpoints()); }

void CBreakPoints_GetBreakpointInfo(int t, int idx, u32* out_addr, bool* out_en, const char** out_cond) {
    auto bps = CBreakPoints::GetBreakpoints(static_cast<BreakPointCpu>(t), false);
    if (idx >= 0 && idx < (int)bps.size()) {
        if (out_addr) *out_addr = bps[idx].addr;
        if (out_en) *out_en = bps[idx].enabled;
        if (out_cond) *out_cond = "";
    }
}

void DebugInterface_resumeCpu(int t) { GetDI(t)->resumeCpu(); }
void DebugInterface_pauseCpu(int t) { GetDI(t)->pauseCpu(); }
void DebugInterface_stepInto(int t) {}
void DebugInterface_stepOver(int t) {}
void DebugInterface_stepOut(int t) {}
void DebugInterface_FreeString(const char* s) {}
int DebugInterface_GetSymbolCount(int t) { return 0; }
void DebugInterface_GetSymbolInfo(int t, int idx, const char** n, u32* a, int* ty) {}
int DebugInterface_GetThreadCount(int t) { return 0; }
void DebugInterface_GetThreadInfo(int t, int idx, int* id, u32* pc, u32* entry, int* pri, const char** st, const char** wt, const char** wi) {}
int DebugInterface_GetModuleCount(int t) { return 0; }
void DebugInterface_GetModuleInfo(int t, int idx, const char** n, const char** v, u32* e, u32* gp, u32* text, u32* data, u32* bss) {}

// Frame capture API
int pcsx2_frame_width() { return s_frame_width; }
int pcsx2_frame_height() { return s_frame_height; }
bool pcsx2_frame_ready() { return s_frame_ready; }
const u8* pcsx2_frame_data() { return s_frame_buffer.data(); }
int pcsx2_frame_size() { return (int)s_frame_buffer.size(); }
void pcsx2_frame_consumed() { s_frame_ready = false; }

// ═══════════════════════════════════════════════════════════════
// LOG STREAMING + VERSION (for Slint About / Log viewer)
// ═══════════════════════════════════════════════════════════════

static PCSX2_OnLog s_on_log = nullptr;

// Console host-output sink: forwards every core log line to the Slint callback.
static void HostLogSink(LOGLEVEL level, ConsoleColors color, std::string_view message)
{
    if (s_on_log)
        s_on_log(static_cast<int32_t>(level), static_cast<int32_t>(color), std::string(message).c_str());
}

void pcsx2_register_log_callback(PCSX2_OnLog on_log)
{
    s_on_log = on_log;
    // Mirror core logs to the Slint UI. Level 3 = INFO (show info+errors).
    Log::SetHostOutputLevel(LOGLEVEL_INFO, &HostLogSink);
    fprintf(stderr, "[CAPI] Log host output enabled\n");
}

static thread_local std::string s_version_buf;
const char* pcsx2_get_version_string()
{
    s_version_buf = std::string("PCSX2 ") + BuildVersion::GitRev +
        " (" + BuildVersion::GitDate + ")";
    return s_version_buf.c_str();
}

} // extern "C"
