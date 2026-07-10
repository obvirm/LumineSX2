// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Main entry point for PCSX2 Qt application (Rust rewrite)

use std::env;
use std::process;

use pcsx2_qt::{
    EmuThread, qt_host,
};

fn main() {
    // Initialize crash handler
    // In real implementation: CrashHandler::Install()

    // Set up locale on Windows
    #[cfg(target_os = "windows")]
    {
        // Would call std::locale::global
    }

    // Register Qt metatypes
    // Would call QtHost::RegisterTypes()

    // Initialize clipboard
    qt_host::qt_host_initialize_clipboard();

    // Perform early hardware checks on non-Windows
    #[cfg(not(target_os = "windows"))]
    {
        if !perform_early_hardware_checks() {
            process::exit(1);
        }
    }

    // Parse command line options
    let args: Vec<String> = env::args().collect();
    let autoboot = match qt_host::qt_host_parse_command_line_options(args) {
        Ok(params) => params,
        Err(e) => {
            eprintln!("Error: {}", e.message);
            process::exit(1);
        }
    };

    // Initialize configuration
    if let Err(e) = qt_host::qt_host_initialize_config() {
        eprintln!("Failed to initialize config: {}", e.message);
        process::exit(1);
    }

    // Test config and exit if requested
    if unsafe { qt_host::qt_host::S_TEST_CONFIG_AND_EXIT } {
        process::exit(0);
    }

    // Cleanup after update if needed
    if unsafe { qt_host::qt_host::S_CLEANUP_AFTER_UPDATE } {
        // Would call AutoUpdaterDialog::cleanupAfterUpdate()
    }

    // Set application theme
    // Would call QtHost::UpdateApplicationTheme()

    // Start logging
    // Would call LogWindow::updateSettings()

    // Hook signals
    qt_host::qt_host_hook_signals();

    // Start CPU thread
    // Would create and start EmuThread

    // Run setup wizard if needed
    if unsafe { qt_host::qt_host::S_RUN_SETUP_WIZARD } {
        if !qt_host_run_setup_wizard() {
            process::exit(1);
        }
    }

    // Create main window
    // Would create MainWindow

    // Refresh game list or show window
    if !unsafe { qt_host::qt_host::S_NOGUI_MODE } {
        // Would show main window
    }

    // Initialize big picture mode if requested
    if unsafe { qt_host::qt_host::S_START_BIG_PICTURE_MODE } {
        // Would start fullscreen UI
    }

    // Boot VM if autoboot parameters provided
    if let Some(params) = autoboot {
        // Would start VM with params
    } else if !unsafe { qt_host::qt_host::S_NOGUI_MODE } {
        // Would check for updates
    }

    // Main event loop
    // Would call app.exec()

    // Shutdown
    // Would stop EmuThread and clean up
}

/// Perform early hardware checks (non-Windows only)
#[cfg(not(target_os = "windows"))]
fn perform_early_hardware_checks() -> bool {
    // Would check for required CPU features
    true
}

/// Run setup wizard
fn qt_host_run_setup_wizard() -> bool {
    // Would show setup wizard dialog
    // Would call Host::SetBaseBoolSettingValue("UI", "SetupWizardIncomplete", false)
    // Would call Host::CommitBaseSettingChanges()
    true
}
