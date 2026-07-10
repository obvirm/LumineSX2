// pcsx2_capi.h — C API for Slint UI to call PCSX2 core
// Exposes VMManager, Host, Settings, Display functions

#pragma once

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

// ─── VM State ───
enum PCSX2_VMState {
    PCSX2_VMState_Invalid = 0,
    PCSX2_VMState_Initializing,
    PCSX2_VMState_Running,
    PCSX2_VMState_Paused,
    PCSX2_VMState_Stopping,
};

// ─── Boot Parameters ───
struct PCSX2_BootParams {
    const char* filename;       // ISO/ELF path
    const char* save_state;     // Optional save state path
    bool fast_boot;
    bool fullscreen;
    bool start_turbo;
    bool start_unlimited;
};

// ─── Game Info ───
struct PCSX2_GameInfo {
    const char* path;
    const char* serial;
    const char* title;
    const char* version;
    uint32_t crc;
};

// ─── Save State Info ───
struct PCSX2_SaveStateInfo {
    int32_t slot;
    bool exists;
    const char* filename;
    int64_t timestamp;
};

// ─── Callbacks (Slint → Core) ───
typedef void (*PCSX2_OnVMStarting)();
typedef void (*PCSX2_OnVMStarted)();
typedef void (*PCSX2_OnVMPaused)();
typedef void (*PCSX2_OnVMResumed)();
typedef void (*PCSX2_OnVMDestroyed)();
typedef void (*PCSX2_OnGameChanged)(const char* path, const char* serial, const char* title);
typedef void (*PCSX2_OnSaveStateLoading)(const char* path);
typedef void (*PCSX2_OnSaveStateLoaded)(const char* path, bool success);
typedef void (*PCSX2_OnSaveStateSaved)(const char* path, bool success, const char* error);
typedef void (*PCSX2_OnError)(const char* title, const char* message);
typedef void (*PCSX2_OnInfo)(const char* title, const char* message);
typedef void (*PCSX2_OnFrame)(const uint8_t* framebuffer, int32_t width, int32_t height, int32_t stride);

// ─── Callback Registration ───
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
    PCSX2_OnFrame on_frame
);

// ─── VM Lifecycle ───
bool pcsx2_initialize(const char* bios_dir);
bool pcsx2_set_bios_dir(const char* path);
const char* pcsx2_get_bios_dir();
bool pcsx2_boot(const PCSX2_BootParams* params);
bool pcsx2_boot_disc(const char* disc_path, bool fast_boot);
void pcsx2_shutdown();
void pcsx2_execute();
void pcsx2_pump_messages();
void pcsx2_set_render_parent(void* hwnd, int x, int y, int w, int h);
void pcsx2_resize_render(int x, int y, int w, int h);
void pcsx2_reset();
void pcsx2_set_state(PCSX2_VMState state);
void pcsx2_set_paused(bool paused);
PCSX2_VMState pcsx2_get_state();
bool pcsx2_has_valid_vm();

// ─── Game Info ───
PCSX2_GameInfo pcsx2_get_game_info();
const char* pcsx2_get_disc_path();
const char* pcsx2_get_disc_serial();
const char* pcsx2_get_title();

// ─── Save States ───
bool pcsx2_save_state(int32_t slot);
bool pcsx2_load_state(int32_t slot);
bool pcsx2_save_state_to_file(const char* path);
bool pcsx2_load_state_from_file(const char* path);
bool pcsx2_has_save_state(int32_t slot);
const char* pcsx2_get_save_state_filename(int32_t slot);

// ─── Settings ───
bool pcsx2_get_bool_setting(const char* section, const char* key, bool default_value);
int32_t pcsx2_get_int_setting(const char* section, const char* key, int32_t default_value);
float pcsx2_get_float_setting(const char* section, const char* key, float default_value);
const char* pcsx2_get_string_setting(const char* section, const char* key, const char* default_value);

void pcsx2_set_bool_setting(const char* section, const char* key, bool value);
void pcsx2_set_int_setting(const char* section, const char* key, int32_t value);
void pcsx2_set_float_setting(const char* section, const char* key, float value);
void pcsx2_set_string_setting(const char* section, const char* key, const char* value);

void pcsx2_commit_settings();
void pcsx2_apply_settings();
void pcsx2_reload_game_settings();

// ─── Display ───
void pcsx2_request_display_size(int32_t width, int32_t height);
void pcsx2_set_fullscreen(bool fullscreen);
bool pcsx2_is_fullscreen();

// ─── Disc ───
void pcsx2_change_disc(const char* path);

// ─── Input ───
void pcsx2_reload_input_bindings();

// ─── OSD ───
void pcsx2_osd_message(const char* message, float duration);
void pcsx2_osd_icon_message(const char* icon, const char* message, float duration);
void pcsx2_osd_clear();

// ─── Clipboard ───
bool pcsx2_copy_to_clipboard(const char* text);
const char* pcsx2_get_from_clipboard();

// ─── Game List ───
void pcsx2_refresh_game_list(bool invalidate_cache);
void pcsx2_cancel_game_list_refresh();

// ─── Limiter ───
int32_t pcsx2_get_limiter_mode();
void pcsx2_set_limiter_mode(int32_t mode);

// ─── Cleanup ───
void pcsx2_free_string(const char* str);

#ifdef __cplusplus
}
#endif
