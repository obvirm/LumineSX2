// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust rewrite of QtHost.h using unsafe FFI to interface with C++ PCSX2 code.

pub mod qt_host;

// Re-export main types
pub use qt_host::{
    EmuThread, Error, ProgressCallback, VMBootParameters,
    qt_host_is_vm_valid, qt_host_is_vm_paused, qt_host_is_on_ui_thread,
    qt_host_should_show_advanced_settings, qt_host_run_on_ui_thread,
    qt_host_get_app_name_and_version, qt_host_get_app_config_suffix,
    qt_host_get_runtime_downloaded_resource_url, qt_host_save_game_settings,
    qt_host_lock_vm_with_dialog, qt_host_unlock_vm_with_dialog,
    qt_host_initialize_config, qt_host_save_settings, qt_host_initialize_clipboard,
    qt_host_parse_command_line_options, qt_host_print_command_line_version,
    qt_host_hook_signals, qt_host_initialize_early_console,
    host_set_default_ui_settings, host_in_batch_mode, host_in_no_gui_mode,
    host_request_exit_application, host_request_vm_shutdown,
    host_is_fullscreen, host_set_fullscreen,
    host_on_vm_starting, host_on_vm_started, host_on_vm_destroyed,
    host_on_vm_paused, host_on_vm_resumed,
    host_on_performance_metrics_updated,
    host_on_save_state_loading, host_on_save_state_loaded, host_on_save_state_saved,
    host_report_error_async, host_report_info_async,
    host_open_url, host_copy_text_to_clipboard, host_get_text_from_clipboard,
    host_on_input_device_connected, host_on_input_device_disconnected,
    host_set_mouse_mode, host_set_mouse_lock,
    host_on_capture_started, host_on_capture_stopped,
    host_on_game_changed, host_create_host_progress_callback,
};
