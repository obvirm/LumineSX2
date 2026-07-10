slint::include_modules!();

pub mod pcsx2_capi;
pub mod host;
pub mod debug_backend;
pub mod debug_controller;
use pcsx2_capi::{Pcsx2Api, PCSX2_VMState};
use debug_controller::DebugController;

use slint::{SharedPixelBuffer, Rgba8Pixel, Image, ModelRc, Model};

/// Scan BIOS directory and detect valid BIOS files
fn scan_bios_dir(dir_path: &str) -> Vec<BiosEntry> {
    let dir = std::path::Path::new(dir_path);
    if !dir.exists() || !dir.is_dir() {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut bios_list = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let name_lower = name.to_lowercase();

        // Detect BIOS files by known patterns
        let (region, region_label, description) = if name_lower.contains("jap") || name_lower.contains("scph-1") || name_lower.contains("scph-0") {
            (0, "Japan", "PS2 BIOS - Japan")
        } else if name_lower.contains("usa") || name_lower.contains("scph-3") || name_lower.contains("scph-5") || name_lower.contains("scph-7001") {
            (1, "USA", "PS2 BIOS - USA")
        } else if name_lower.contains("eur") || name_lower.contains("scph-7002") || name_lower.contains("scph-77") || name_lower.contains("scph-9") {
            (2, "Europe", "PS2 BIOS - Europe")
        } else if name_lower.contains("oce") || name_lower.contains("scph-7003") {
            (3, "Oceania", "PS2 BIOS - Oceania")
        } else if name_lower.contains("asia") || name_lower.contains("scph-7006") {
            (4, "Asia", "PS2 BIOS - Asia")
        } else if name_lower.contains("rus") || name_lower.contains("scph-7008") {
            (5, "Russia", "PS2 BIOS - Russia")
        } else if name_lower.contains("chn") || name_lower.contains("scph-7009") {
            (6, "China", "PS2 BIOS - China")
        } else if name_lower.contains("mex") || name_lower.contains("scph-7004") {
            (7, "Mexico", "PS2 BIOS - Mexico")
        } else if name_lower == "rom0.bin" || name_lower == "rom1.bin" || name_lower == "erom.bin" || name_lower.ends_with(".bin") || name_lower.ends_with(".rom0") {
            (1, "USA", "PS2 BIOS")
        } else {
            continue;
        };

        bios_list.push(BiosEntry {
            filename: slint::SharedString::from(name),
            description: slint::SharedString::from(description),
            region,
            region_label: slint::SharedString::from(region_label),
            valid: true,
        });
    }

    bios_list.sort_by(|a, b| a.filename.as_str().cmp(b.filename.as_str()));
    bios_list
}

pub struct App {
    pub window: MainWindow,
    pub bios_path: std::sync::Arc<std::sync::Mutex<String>>,
    pub debug: DebugController,
}

impl App {
    pub fn new() -> Self {
        let window = MainWindow::new().unwrap();
        let bios_path = std::sync::Arc::new(std::sync::Mutex::new(String::from("bios")));
        let debug = DebugController::new();
        
        // ── BIOS Settings ──
        
        // Load BIOS dir from EmuFolders (set by main.rs or settings)
        #[cfg(feature = "pcsx2-core")]
        {
            let current_bios = Pcsx2Api::get_bios_dir();
            eprintln!("[BIOS] Current EmuFolders::Bios = {}", current_bios);
            if !current_bios.is_empty() {
                window.set_bios_dir_path(slint::SharedString::from(&current_bios));
                *bios_path.lock().unwrap() = current_bios;
            }
            // Load fast boot settings (like Qt: "EmuCore","EnableFastBoot")
            let fast_boot = Pcsx2Api::get_bool_setting("EmuCore", "EnableFastBoot", true);
            let fast_fwd = Pcsx2Api::get_bool_setting("EmuCore", "EnableFastBootFastForward", false);
            window.set_fast_boot(fast_boot);
            window.set_fast_forward_boot(fast_fwd);
        }

        // Browse BIOS folder (directory picker) — saves to settings
        window.on_browse_bios_folder({
            let handle_weak = window.as_weak();
            let bios_path_browse = bios_path.clone();
            move || {
                let Some(handle) = handle_weak.upgrade() else { return };
                let current = handle.get_bios_dir_path().to_string();
                let start_dir = if std::path::Path::new(&current).exists() { current } else { "bios".to_string() };
                let dialog = rfd::FileDialog::new()
                    .set_title("Pilih Folder BIOS PS2")
                    .set_directory(&start_dir)
                    .pick_folder();
                if let Some(path) = dialog {
                    let new_path = path.to_string_lossy().to_string();
                    handle.set_bios_dir_path(slint::SharedString::from(&new_path));
                    *bios_path_browse.lock().unwrap() = new_path.clone();
                    // Save to settings (like Qt: "Folders","Bios")
                    #[cfg(feature = "pcsx2-core")]
                    {
                        Pcsx2Api::set_string_setting("Folders", "Bios", &new_path);
                        Pcsx2Api::commit_settings();
                    }
                    eprintln!("[BIOS] Dir saved: {}", new_path);
                }
            }
        });

        // Reset BIOS folder to default — saves to settings
        window.on_reset_bios_folder({
            let handle_weak = window.as_weak();
            let bios_path_reset = bios_path.clone();
            move || {
                let Some(handle) = handle_weak.upgrade() else { return };
                let default = "bios".to_string();
                handle.set_bios_dir_path(slint::SharedString::from(&default));
                *bios_path_reset.lock().unwrap() = default.clone();
                #[cfg(feature = "pcsx2-core")]
                {
                    Pcsx2Api::set_string_setting("Folders", "Bios", &default);
                    Pcsx2Api::commit_settings();
                }
            }
        });

        // Open BIOS folder in file explorer
        window.on_open_bios_folder({
            let handle_weak = window.as_weak();
            move || {
                let Some(handle) = handle_weak.upgrade() else { return };
                let dir = std::path::PathBuf::from(handle.get_bios_dir_path().to_string());
                let dir = if dir.exists() { dir } else { std::path::PathBuf::from("bios") };
                #[cfg(target_os = "windows")]
                { let _ = std::process::Command::new("explorer").arg(dir.canonicalize().unwrap_or(dir)).spawn(); }
                #[cfg(target_os = "linux")]
                { let _ = std::process::Command::new("xdg-open").arg(&dir).spawn(); }
                #[cfg(target_os = "macos")]
                { let _ = std::process::Command::new("open").arg(&dir).spawn(); }
            }
        });

        // Refresh BIOS list — scan + auto-select from settings (like Qt)
        window.on_refresh_bios_list({
            let handle_weak = window.as_weak();
            move || {
                let Some(handle) = handle_weak.upgrade() else { return };
                let bios_dir = handle.get_bios_dir_path().to_string();
                eprintln!("[BIOS] Refresh: scanning '{}'", bios_dir);
                let entries = scan_bios_dir(&bios_dir);
                eprintln!("[BIOS] Found {} entries", entries.len());

                // Auto-select BIOS that matches settings (like Qt)
                #[cfg(feature = "pcsx2-core")]
                let selected_name = Pcsx2Api::get_string_setting("Filenames", "BIOS", "");
                #[cfg(not(feature = "pcsx2-core"))]
                let selected_name = String::new();

                let mut selected_idx: i32 = -1;
                if !selected_name.is_empty() {
                    for (i, entry) in entries.iter().enumerate() {
                        if entry.filename.to_string() == selected_name {
                            selected_idx = i as i32;
                            break;
                        }
                    }
                }
                eprintln!("[BIOS] Auto-select: '{}' -> idx={}", selected_name, selected_idx);

                handle.set_bios_entries(std::rc::Rc::new(slint::VecModel::from(entries)).into());
                handle.set_selected_bios(selected_idx);
            }
        });

        // Select BIOS — save filename to settings (like Qt: listItemChanged)
        window.on_select_bios({
            let handle_weak = window.as_weak();
            move |idx: i32| {
                let Some(handle) = handle_weak.upgrade() else { return };
                handle.set_selected_bios(idx);
                let entries = handle.get_bios_entries();
                if idx >= 0 && (idx as usize) < entries.row_count() {
                    if let Some(entry) = entries.row_data(idx as usize) {
                        let filename = entry.filename.to_string();
                        eprintln!("[BIOS] Selected: idx={} file={}", idx, filename);
                        // Save just filename to settings (like Qt: Host::SetBaseStringSettingValue)
                        #[cfg(feature = "pcsx2-core")]
                        {
                            Pcsx2Api::set_string_setting("Filenames", "BIOS", &filename);
                            Pcsx2Api::commit_settings();
                            // Don't apply_settings while VM running — disrupts Vulkan GS
                            if Pcsx2Api::get_state() != crate::pcsx2_capi::PCSX2_VMState::Running {
                                Pcsx2Api::apply_settings();
                            }
                            eprintln!("[BIOS] Saved: Filenames/BIOS={}", filename);
                        }
                    }
                }
            }
        });

        // ── Memory Card Callbacks ──
        
        // Browse folder
        window.on_memcard_browse_folder({
            let window_weak = window.as_weak();
            move || {
                let Some(win) = window_weak.upgrade() else { return };
                #[cfg(feature = "desktop")]
                {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        win.set_memcard_folder(path.to_string_lossy().to_string().into());
                    }
                }
            }
        });
        
        // Open folder in explorer
        window.on_memcard_open_folder({
            let window_weak = window.as_weak();
            move || {
                let Some(win) = window_weak.upgrade() else { return };
                let dir = win.get_memcard_folder().to_string();
                let path = std::path::PathBuf::from(&dir);
                let path = if path.exists() { path } else { std::path::PathBuf::from("memcards") };
                #[cfg(target_os = "windows")]
                { let _ = std::process::Command::new("explorer").arg(path.canonicalize().unwrap_or(path)).spawn(); }
                #[cfg(target_os = "linux")]
                { let _ = std::process::Command::new("xdg-open").arg(&path).spawn(); }
                #[cfg(target_os = "macos")]
                { let _ = std::process::Command::new("open").arg(&path).spawn(); }
            }
        });
        
        // Reset folder to default
        window.on_memcard_reset_folder({
            let window_weak = window.as_weak();
            move || {
                let Some(win) = window_weak.upgrade() else { return };
                win.set_memcard_folder("memcards/".into());
            }
        });
        
        // Refresh card list
        window.on_memcard_refresh_list({
            let window_weak = window.as_weak();
            move || {
                let Some(win) = window_weak.upgrade() else { return };
                let dir = win.get_memcard_folder().to_string();
                let path = std::path::Path::new(dir.as_str());
                let _ = std::fs::create_dir_all(path);
                
                let mut cards = Vec::new();
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                        let ext = p.extension().unwrap_or_default().to_string_lossy().to_lowercase();
                        
                        let (card_type, is_folder) = if p.is_dir() {
                            ("PS2 (Folder)", true)
                        } else if ext == "ps2" {
                            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                            let size_mb = size / (1024 * 1024);
                            let type_str = match size_mb {
                                8 => "PS2 (8MB)",
                                16 => "PS2 (16MB)",
                                32 => "PS2 (32MB)",
                                64 => "PS2 (64MB)",
                                _ => "PS2 (Custom)",
                            };
                            (type_str, false)
                        } else if ext == "mcd" || ext == "mcr" || ext == "gme" || ext == "vgs" || ext == "mem" {
                            ("PS1 (128KB)", false)
                        } else {
                            continue;
                        };
                        
                        let modified = entry.metadata()
                            .and_then(|m| m.modified())
                            .map(|t| {
                                let dt: chrono::DateTime<chrono::Local> = t.into();
                                dt.format("%Y-%m-%d %H:%M").to_string()
                            })
                            .unwrap_or_else(|_| "Unknown".to_string());
                        
                        let in_use = name == win.get_memcard_slot1_card().to_string() 
                            || name == win.get_memcard_slot2_card().to_string();
                        
                        cards.push(MemCardInfo {
                            name: name.into(),
                            r#type: card_type.into(),
                            formatted: true,
                            last_modified: modified.into(),
                            in_use,
                            is_folder,
                        });
                    }
                }
                
                win.set_memcard_list(std::rc::Rc::new(slint::VecModel::from(cards)).into());
            }
        });
        
        // Create new memory card
        window.on_memcard_create({
            let window_weak = window.as_weak();
            move |name, size, _ntfs| {
                let Some(win) = window_weak.upgrade() else { return };
                let dir = win.get_memcard_folder().to_string();
                let path = std::path::Path::new(dir.as_str());
                let _ = std::fs::create_dir_all(path);
                
                let ext = if size == 5 { "mcd" } else { "ps2" };
                let filename = if name.ends_with(".ps2") || name.ends_with(".mcd") {
                    name.to_string()
                } else {
                    format!("{}.{}", name, ext)
                };
                
                let card_path = path.join(&filename);
                let size_bytes: usize = match size {
                    0 => 8 * 1024 * 1024,
                    1 => 16 * 1024 * 1024,
                    2 => 32 * 1024 * 1024,
                    3 => 64 * 1024 * 1024,
                    5 => 128 * 1024,
                    _ => 8 * 1024 * 1024,
                };
                
                if size == 4 {
                    // Folder type
                    let _ = std::fs::create_dir_all(&card_path);
                } else {
                    // File type - create with zeros
                    let data = vec![0u8; size_bytes];
                    let _ = std::fs::write(&card_path, &data);
                }
                
                // Refresh list
                win.invoke_memcard_refresh_list();
            }
        });
        
        // Rename memory card
        window.on_memcard_rename({
            let window_weak = window.as_weak();
            move |new_name| {
                let Some(win) = window_weak.upgrade() else { return };
                let dir = win.get_memcard_folder().to_string();
                // TODO: Get selected card name and rename
                eprintln!("[MemCard] Rename to: {}", new_name);
            }
        });
        
        // Delete memory card
        window.on_memcard_delete({
            let window_weak = window.as_weak();
            move |idx| {
                let Some(win) = window_weak.upgrade() else { return };
                let dir = win.get_memcard_folder().to_string();
                let cards = win.get_memcard_list();
                if idx >= 0 && (idx as usize) < cards.row_count() {
                    let card = cards.row_data(idx as usize).unwrap();
                    let card_path = std::path::Path::new(dir.as_str()).join(card.name.as_str());
                    if card_path.is_dir() {
                        let _ = std::fs::remove_dir_all(&card_path);
                    } else {
                        let _ = std::fs::remove_file(&card_path);
                    }
                    win.invoke_memcard_refresh_list();
                }
            }
        });
        
        // Convert memory card
        window.on_memcard_convert({
            let window_weak = window.as_weak();
            move |_idx, _target_type| {
                let Some(_win) = window_weak.upgrade() else { return };
                // TODO: Implement card conversion
                eprintln!("[MemCard] Convert not yet implemented");
            }
        });
        
        // Select card
        window.on_memcard_select({
            let window_weak = window.as_weak();
            move |idx| {
                // Selection is handled by the UI property directly
                let _ = &window_weak;
            }
        });
        
        // Assign card to slot
        window.on_memcard_assign_slot({
            let window_weak = window.as_weak();
            move |slot, card_idx| {
                let Some(win) = window_weak.upgrade() else { return };
                let cards = win.get_memcard_list();
                if card_idx >= 0 && (card_idx as usize) < cards.row_count() {
                    let card = cards.row_data(card_idx as usize).unwrap();
                    if slot == 0 {
                        win.set_memcard_slot1_card(card.name);
                        win.set_memcard_slot1_enabled(true);
                    } else {
                        win.set_memcard_slot2_card(card.name);
                        win.set_memcard_slot2_enabled(true);
                    }
                }
            }
        });
        
        // Eject card from slot
        window.on_memcard_eject({
            let window_weak = window.as_weak();
            move |slot| {
                let Some(win) = window_weak.upgrade() else { return };
                if slot == 0 {
                    win.set_memcard_slot1_card("".into());
                } else {
                    win.set_memcard_slot2_card("".into());
                }
            }
        });
        
        // Swap slots
        window.on_memcard_swap_slots({
            let window_weak = window.as_weak();
            move || {
                let Some(win) = window_weak.upgrade() else { return };
                let card1 = win.get_memcard_slot1_card();
                let card2 = win.get_memcard_slot2_card();
                win.set_memcard_slot1_card(card2);
                win.set_memcard_slot2_card(card1);
                
                let enabled1 = win.get_memcard_slot1_enabled();
                let enabled2 = win.get_memcard_slot2_enabled();
                win.set_memcard_slot1_enabled(enabled2);
                win.set_memcard_slot2_enabled(enabled1);
            }
        });

        // ── PCSX2 Core Callbacks ──
        
        // Boot BIOS
        window.on_boot_bios({
            let window_weak = window.as_weak();
            let bios_path = bios_path.clone();
            move || {
                let Some(win) = window_weak.upgrade() else { return };
                // Skip if VM already running
                if Pcsx2Api::get_state() == PCSX2_VMState::Running { return; }
                let bios_dir = bios_path.lock().unwrap().clone();
                // Get selected BIOS file
                let entries = win.get_bios_entries();
                let selected = win.get_selected_bios() as usize;
                let count = entries.row_count();
                let bios_file = if selected < count {
                    entries.row_data(selected).unwrap().filename.to_string()
                } else {
                    String::new()
                };
                eprintln!("[BIOS] Boot: dir={}, file={}", bios_dir, bios_file);
                // Set BIOS directory in PCSX2 config
                if !bios_dir.is_empty() && bios_dir != "bios" {
                    Pcsx2Api::set_bios_dir(&bios_dir);
                }
                // Save selected BIOS to settings (like Qt does)
                if !bios_file.is_empty() {
                    Pcsx2Api::set_string_setting("Filenames", "BIOS", &bios_file);
                    Pcsx2Api::commit_settings();
                    eprintln!("[BIOS] Saved to settings: Filenames/BIOS={}", bios_file);
                }
                // Save fast boot settings (like Qt: "EmuCore","EnableFastBoot")
                let fast_boot = win.get_fast_boot();
                let fast_fwd = win.get_fast_forward_boot();
                Pcsx2Api::set_bool_setting("EmuCore", "EnableFastBoot", fast_boot);
                Pcsx2Api::set_bool_setting("EmuCore", "EnableFastBootFastForward", fast_fwd);
                Pcsx2Api::commit_settings();
                // Boot with empty filename — PCSX2 reads BIOS from settings
                // Must run on fresh thread (Slint thread has COM initialized with different mode)
                std::thread::spawn(move || {
                    use crate::pcsx2_capi::PCSX2_VMState;
                    if Pcsx2Api::boot("", fast_boot) {
                        eprintln!("[MAIN] BIOS boot started (worker thread)");
                        loop {
                            match Pcsx2Api::get_state() {
                                PCSX2_VMState::Running => Pcsx2Api::execute(),
                                PCSX2_VMState::Paused => std::thread::sleep(std::time::Duration::from_millis(16)),
                                _ => break,
                            }
                        }
                    } else {
                        eprintln!("[MAIN] BIOS boot FAILED");
                    }
                });

                // Start frame polling thread (updates Slint framebuffer 60fps)
                let weak_for_frame = window_weak.clone();
                std::thread::spawn(move || {
                    use crate::pcsx2_capi::PCSX2_VMState;
                    loop {
                        // Check if VM is still running
                        let state = Pcsx2Api::get_state();
                        if state != PCSX2_VMState::Running && state != PCSX2_VMState::Paused {
                            break;
                        }
                        // Poll frame
                        if let Some((w, h, data)) = Pcsx2Api::get_frame() {
                            let w2 = weak_for_frame.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(win) = w2.upgrade() {
                                    let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w as u32, h as u32);
                                    let pixels = buffer.make_mut_slice();
                                    for (i, chunk) in data.chunks(4).enumerate() {
                                        if i < pixels.len() && chunk.len() >= 4 {
                                            pixels[i] = slint::Rgba8Pixel::new(chunk[0], chunk[1], chunk[2], chunk[3]);
                                        }
                                    }
                                    win.set_framebuffer(slint::Image::from_rgba8(buffer));
                                }
                            });
                        }
                        std::thread::sleep(std::time::Duration::from_millis(16));
                    }
                    eprintln!("[FRAME] Frame polling thread exited");
                });
                win.set_is_playing(true);
                win.set_status_message("Starting BIOS...".into());
            }
        });
        
        // Play game
        window.on_play_game({
            let window_weak = window.as_weak();
            move || {
                let Some(win) = window_weak.upgrade() else { return };
                // Get selected game from list
                let games = win.get_game_list();
                let selected = win.get_selected_game();
                if selected >= 0 && (selected as usize) < games.row_count() {
                    if let Some(game) = games.row_data(selected as usize) {
                        let path = game.path.to_string();
                        if !path.is_empty() {
                            // Save fast boot settings before boot
                            let fast_boot = win.get_fast_boot();
                            Pcsx2Api::set_bool_setting("EmuCore", "EnableFastBoot", fast_boot);
                            Pcsx2Api::commit_settings();
                            // Boot game
                            if Pcsx2Api::boot(&path, win.get_fast_boot()) {
                                win.set_is_playing(true);
                                win.set_status_message("Booting game...".into());
                            } else {
                                win.set_status_message("Failed to boot game".into());
                            }
                        }
                    }
                }
            }
        });
        
        // Pause/Resume
        window.on_pause_game({
            let window_weak = window.as_weak();
            move || {
                let Some(win) = window_weak.upgrade() else { return };
                let state = Pcsx2Api::get_state();
                if state == PCSX2_VMState::Running {
                    Pcsx2Api::set_paused(true);
                    win.set_status_message("Paused".into());
                } else if state == PCSX2_VMState::Paused {
                    Pcsx2Api::set_paused(false);
                    win.set_status_message("Resumed".into());
                }
            }
        });
        
        // Stop
        window.on_stop_game({
            let window_weak = window.as_weak();
            move || {
                let Some(win) = window_weak.upgrade() else { return };
                Pcsx2Api::shutdown();
                win.set_is_playing(false);
                win.set_status_message("Game stopped".into());
            }
        });
        
        // Reset
        window.on_reset_game({
            move || {
                Pcsx2Api::reset();
            }
        });
        
        // Save state
        window.on_save_state({
            let window_weak = window.as_weak();
            move |slot| {
                let Some(win) = window_weak.upgrade() else { return };
                if Pcsx2Api::save_state(slot) {
                    win.set_status_message(format!("Saving state to slot {}...", slot).into());
                }
            }
        });
        
        // Load state
        window.on_load_state({
            let window_weak = window.as_weak();
            move |slot| {
                let Some(win) = window_weak.upgrade() else { return };
                if Pcsx2Api::load_state(slot) {
                    win.set_status_message(format!("Loading state from slot {}...", slot).into());
                }
            }
        });

        window.on_add_game_folder({
            let window_weak = window.as_weak();
            move || {
                let win = window_weak.unwrap();

                let folder_path = {
                    #[cfg(feature = "desktop")]
                    {
                        rfd::FileDialog::new().pick_folder()
                    }
                    #[cfg(not(feature = "desktop"))]
                    {
                        let _ = &win;
                        None::<std::path::PathBuf>
                    }
                };

                let folder_path = match folder_path {
                    Some(p) => p,
                    None => return,
                };

                win.set_status_message("Scanning folder for PS2 games...".into());
                win.set_game_list(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::<GameInfo>::default())));

                let ww = window_weak.clone();
                std::thread::spawn(move || {
                    let window_weak = ww;
                    let mut scanned_games = Vec::new();

                    if let Ok(entries) = std::fs::read_dir(&folder_path) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if let Some(ext) = p.extension() {
                                let ext_lower = ext.to_string_lossy().to_lowercase();
                                if matches!(ext_lower.as_str(), "elf" | "iso" | "bin") {
                                    let path_str = p.to_string_lossy().to_string();
                                    if !is_valid_ps2_game(&path_str) {
                                        continue;
                                    }

                                    let title = p.file_stem()
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or_else(|| "Unknown Game".to_string());

                                    let size_str = match std::fs::metadata(&p) {
                                        Ok(meta) => format!("{:.1} GB", meta.len() as f64 / (1024.0 * 1024.0 * 1024.0)),
                                        Err(_) => "0.0 GB".to_string(),
                                    };

                                    let serial_opt = extract_ps2_serial(&path_str);
                                    let region = serial_opt.as_ref().map_or("Unknown", |s| get_region_from_serial(s));
                                    let code = serial_opt.clone().unwrap_or_else(|| "Unknown".to_string());

                                    let mut cover_path_str = None;
                                    if let Some(ref serial) = serial_opt {
                                        let cover_path = std::path::PathBuf::from("covers").join(format!("{}.jpg", serial));
                                        let success = if cover_path.exists() {
                                            true
                                        } else {
                                            let w = window_weak.clone();
                                            let t = title.clone();
                                            let _ = slint::invoke_from_event_loop(move || {
                                                w.unwrap().set_status_message(format!("Downloading cover for {}...", t).into());
                                            });
                                            download_cover(serial, &cover_path)
                                        };
                                        if success {
                                            cover_path_str = Some(cover_path.to_string_lossy().to_string());
                                        }
                                    }

                                    scanned_games.push((title, path_str.clone(), region.to_string(), code, size_str, cover_path_str));
                                }
                            }
                        }
                    }

                    let w = window_weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        let win = w.unwrap();
                        let card_colors = [
                            slint::Color::from_rgb_u8(234, 88, 12),
                            slint::Color::from_rgb_u8(185, 28, 28),
                            slint::Color::from_rgb_u8(51, 65, 85),
                            slint::Color::from_rgb_u8(37, 99, 235),
                            slint::Color::from_rgb_u8(71, 85, 105),
                            slint::Color::from_rgb_u8(217, 119, 6),
                        ];
                        let count = scanned_games.len();
                        let game_infos: Vec<GameInfo> = scanned_games.into_iter().enumerate().map(|(idx, (title, path, region, code, size, cover_path))| {
                            let cover = cover_path.as_ref().and_then(|p| slint::Image::load_from_path(std::path::Path::new(p)).ok()).unwrap_or_default();
                            GameInfo {
                                title: title.into(),
                                path: path.into(),
                                region: region.into(),
                                code: code.into(),
                                size: size.into(),
                                color: card_colors[idx % card_colors.len()],
                                favorite: false,
                                cover,
                            }
                        }).collect();
                        win.set_game_list(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(game_infos))));
                        win.set_status_message(format!("Scan complete! Found {} valid PS2 games.", count).into());
                    });
                });
            }
        });

        Self { window, bios_path, debug }
    }

    pub fn set_emu_state(&self, frame: u64, ee_pc: u32, iop_pc: u32, fps: f32, speed: i32) {
        self.window.set_emu_state(EmuState {
            frame: frame as i32,
            ee_pc: ee_pc as i32,
            iop_pc: iop_pc as i32,
            fps: fps as i32,
            speed: speed,
        });
    }

    pub fn set_framebuffer(&self, fb: &[u8], w: u32, h: u32) {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
        let pixels = buffer.make_mut_slice();
        for (i, chunk) in fb.chunks(4).enumerate() {
            if i < pixels.len() && chunk.len() >= 4 {
                pixels[i] = Rgba8Pixel::new(chunk[0], chunk[1], chunk[2], chunk[3]);
            }
        }
        self.window.set_framebuffer(Image::from_rgba8(buffer));
    }

    pub fn set_games(&self, games: &[GameInfo]) {
        self.window.set_game_list(ModelRc::from(games));
    }

    pub fn set_save_states(&self, states: &[SaveState]) {
        self.window.set_save_states(ModelRc::from(states));
    }

    pub fn set_bios_list(&self, bios: &[BiosInfo]) {
        self.window.set_bios_list(ModelRc::from(bios));
    }

    pub fn set_status(&self, msg: &str) {
        self.window.set_status_message(msg.into());
    }

    pub fn run(&self) {
        self.window.run().unwrap();
    }
}

fn is_valid_ps2_game(path: &str) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut f = file;
    
    // Check if it's a raw ELF first
    let mut header = [0u8; 4];
    if f.read_exact(&mut header).is_ok() {
        if header == [0x7f, 0x45, 0x4c, 0x46] { // \x7fELF
            return true;
        }
    }
    
    // Helper to check Primary Volume Descriptor
    let check_pvd = |f: &mut std::fs::File, offset: u64| -> bool {
        if f.seek(SeekFrom::Start(offset)).is_err() {
            return false;
        }
        let mut buf = [0u8; 40];
        if f.read_exact(&mut buf).is_err() {
            return false;
        }
        let type_val = buf[0];
        let standard_id = &buf[1..6];
        let system_id = &buf[8..40];
        
        if type_val == 0x01 && standard_id == b"CD001" {
            let sys_id_str = String::from_utf8_lossy(system_id);
            if sys_id_str.trim().contains("PLAYSTATION") {
                return true;
            }
        }
        false
    };
    
    // Check standard 2048-byte sector PVD or 2352-byte sector PVD
    if check_pvd(&mut f, 0x8000) || check_pvd(&mut f, 37656) {
        // Read first 4 MB to check for SYSTEM.CNF containing BOOT2 or BOOT
        if f.seek(SeekFrom::Start(0)).is_ok() {
            let mut buffer = vec![0u8; 4 * 1024 * 1024];
            if let Ok(bytes_read) = f.read(&mut buffer) {
                let chunk = &buffer[..bytes_read];
                let has_boot2 = chunk.windows(5).any(|w| w == b"BOOT2" || w == b"boot2");
                let has_system_cnf = chunk.windows(10).any(|w| w == b"SYSTEM.CNF" || w == b"system.cnf");
                if has_boot2 || has_system_cnf {
                    return true;
                }
            }
        }
    }
    false
}

fn extract_ps2_serial(path: &str) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buffer = vec![0u8; 4 * 1024 * 1024];
    let bytes_read = f.read(&mut buffer).ok()?;
    let data = &buffer[..bytes_read];

    let start_offset = if let Some(idx) = find_subsequence(data, b"BOOT2") {
        idx
    } else if let Some(idx) = find_subsequence(data, b"boot2") {
        idx
    } else if let Some(idx) = find_subsequence(data, b"BOOT") {
        idx
    } else {
        0
    };

    // We scan from start_offset
    let mut i = start_offset;
    while i + 9 < data.len() {
        // Try the 11-byte pattern (e.g. SLUS_209.46)
        if i + 10 < data.len() {
            let slice = &data[i..i+11];
            if slice[0..4].iter().all(|&b| b.is_ascii_alphabetic())
                && (slice[4] == b'_' || slice[4] == b'-' || slice[4] == b'.')
                && slice[5..8].iter().all(|&b| b.is_ascii_digit())
                && (slice[8] == b'.' || slice[8] == b'_' || slice[8] == b'-')
                && slice[9..11].iter().all(|&b| b.is_ascii_digit())
            {
                let letters = std::str::from_utf8(&slice[0..4]).ok()?;
                let digits1 = std::str::from_utf8(&slice[5..8]).ok()?;
                let digits2 = std::str::from_utf8(&slice[9..11]).ok()?;
                return Some(format!("{}-{}{}", letters.to_uppercase(), digits1, digits2));
            }
        }
        
        // Try the 10-byte pattern (e.g. SLUS-20946)
        let slice = &data[i..i+10];
        if slice[0..4].iter().all(|&b| b.is_ascii_alphabetic())
            && (slice[4] == b'_' || slice[4] == b'-' || slice[4] == b'.')
            && slice[5..10].iter().all(|&b| b.is_ascii_digit())
        {
            let letters = std::str::from_utf8(&slice[0..4]).ok()?;
            let digits = std::str::from_utf8(&slice[5..10]).ok()?;
            return Some(format!("{}-{}", letters.to_uppercase(), digits));
        }
        
        i += 1;
    }

    // Fallback if not found: try scan from 0 to start_offset
    if start_offset > 0 {
        let mut i = 0;
        while i + 9 < start_offset {
            if i + 10 < start_offset {
                let slice = &data[i..i+11];
                if slice[0..4].iter().all(|&b| b.is_ascii_alphabetic())
                    && (slice[4] == b'_' || slice[4] == b'-' || slice[4] == b'.')
                    && slice[5..8].iter().all(|&b| b.is_ascii_digit())
                    && (slice[8] == b'.' || slice[8] == b'_' || slice[8] == b'-')
                    && slice[9..11].iter().all(|&b| b.is_ascii_digit())
                {
                    let letters = std::str::from_utf8(&slice[0..4]).ok()?;
                    let digits1 = std::str::from_utf8(&slice[5..8]).ok()?;
                    let digits2 = std::str::from_utf8(&slice[9..11]).ok()?;
                    return Some(format!("{}-{}{}", letters.to_uppercase(), digits1, digits2));
                }
            }
            
            let slice = &data[i..i+10];
            if slice[0..4].iter().all(|&b| b.is_ascii_alphabetic())
                && (slice[4] == b'_' || slice[4] == b'-' || slice[4] == b'.')
                && slice[5..10].iter().all(|&b| b.is_ascii_digit())
            {
                let letters = std::str::from_utf8(&slice[0..4]).ok()?;
                let digits = std::str::from_utf8(&slice[5..10]).ok()?;
                return Some(format!("{}-{}", letters.to_uppercase(), digits));
            }
            i += 1;
        }
    }

    None
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn get_region_from_serial(serial: &str) -> &'static str {
    if serial.starts_with("SLUS") || serial.starts_with("SCUS") {
        "NTSC-U"
    } else if serial.starts_with("SLES") || serial.starts_with("SCES") {
        "PAL"
    } else if serial.starts_with("SLPS") || serial.starts_with("SLPM") || serial.starts_with("SCPS") || serial.starts_with("SLAJ") {
        "NTSC-J"
    } else if serial.starts_with("SLKA") {
        "NTSC-K"
    } else {
        "Unknown"
    }
}

fn download_cover(serial: &str, output_path: &std::path::Path) -> bool {
    let url = format!("https://raw.githubusercontent.com/xlenore/ps2-covers/main/covers/default/{}.jpg", serial);
    
    if let Some(parent) = output_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    
    let status = std::process::Command::new("curl")
        .args(&[
            "-L",
            "-f",
            "-o",
        ])
        .arg(output_path)
        .arg(&url)
        .status();
        
    match status {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

