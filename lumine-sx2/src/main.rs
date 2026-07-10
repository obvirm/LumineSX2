use slint::ComponentHandle;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has_boot = args.iter().any(|a| a == "--boot");
    let boot_path = args.iter().position(|a| a == "--boot").map(|i| args[i+1..].join(" ")).filter(|s| !s.is_empty());

    eprintln!("[MAIN] Initializing PCSX2 core...");
    #[cfg(feature = "pcsx2-core")]
    {
        lumine_sx2::pcsx2_capi::Pcsx2Api::register_default_callbacks();
        eprintln!("[MAIN] Callbacks registered");
        lumine_sx2::pcsx2_capi::Pcsx2Api::initialize("");
        eprintln!("[MAIN] PCSX2 core initialized");

        // Set BIOS directory
        let bios_dir = "E:\\project\\ps2\\bios";
        lumine_sx2::pcsx2_capi::Pcsx2Api::set_bios_dir(bios_dir);
        eprintln!("[MAIN] BIOS dir set to: {}", bios_dir);

        // Boot game if --boot argument provided (on worker thread to avoid COM clash)
        if has_boot {
            let path = boot_path.as_deref().unwrap_or("").to_string();
            eprintln!("[MAIN] Booting: {}", if path.is_empty() { "(BIOS)" } else { &path });
            std::thread::spawn(move || {
                if lumine_sx2::pcsx2_capi::Pcsx2Api::boot(&path, !path.is_empty()) {
                    eprintln!("[MAIN] Boot started (worker thread)");
                    // Main emulation loop (like Qt's EmuThread)
                    use lumine_sx2::pcsx2_capi::{Pcsx2Api, PCSX2_VMState};
                    loop {
                        match Pcsx2Api::get_state() {
                            PCSX2_VMState::Running => Pcsx2Api::execute(),
                            PCSX2_VMState::Paused => {
                                Pcsx2Api::pump_messages();
                                std::thread::sleep(std::time::Duration::from_millis(16));
                            }
                            _ => break,
                        }
                    }
                } else {
                    eprintln!("[MAIN] Boot FAILED (worker thread)");
                }
            });
        }
    }

    let app = lumine_sx2::App::new();

    // Frame update loop
    #[cfg(feature = "pcsx2-core")]
    {
        let window_weak = app.window.as_weak();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(16));
                if let Some((w, h, bgra)) = lumine_sx2::pcsx2_capi::Pcsx2Api::get_frame() {
                    let mut rgba = Vec::with_capacity(bgra.len());
                    for chunk in bgra.chunks_exact(4) {
                        rgba.push(chunk[2]);
                        rgba.push(chunk[1]);
                        rgba.push(chunk[0]);
                        rgba.push(chunk[3]);
                    }
                    let weak = window_weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = weak.upgrade() {
                            let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&rgba, w as u32, h as u32);
                            win.set_framebuffer(slint::Image::from_rgba8(buf));
                        }
                    });
                }
            }
        });
    }

    app.window.run().unwrap();
}
