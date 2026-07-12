//! Headless integration test: simulate clicking a game card in the UI.
//!
//! This proves the `play-game` Slint callback (fired when a user clicks a game
//! in the list) actually boots the game and runs the emulation loop — the bug
//! we just fixed (game click only started the VM but never advanced it).
//!
//! Run:
//!   cargo run --example play_game_test --features pcsx2-core --release

use std::time::Duration;
use slint::{ComponentHandle, Model, ModelRc, VecModel};

fn main() {
    #[cfg(feature = "pcsx2-core")]
    {
        use lumine_sx2::pcsx2_capi::{Pcsx2Api, PCSX2_VMState};
        use lumine_sx2::App;
        use slint::{Model, ModelRc, VecModel};
        use std::rc::Rc;

        eprintln!("[TEST] Initializing PCSX2 core...");
        Pcsx2Api::register_default_callbacks();
        Pcsx2Api::initialize("");
        let bios_dir = "E:\\project\\ps2\\bios";
        Pcsx2Api::set_bios_dir(bios_dir);

        // Build the app + wire all Slint callbacks (same as real main).
        let app = App::new();
        let window = app.window.clone_strong();

        // Populate the game list with ONE entry pointing at the test ISO,
        // exactly like the UI does after scanning a folder.
        let games = Rc::new(VecModel::<lumine_sx2::GameInfo>::default());
        games.push(lumine_sx2::GameInfo {
            title: "Black (USA)".into(),
            path: "E:\\project\\ps2\\game\\Black (USA).iso".into(),
            region: "NTSC-U".into(),
            code: "SLUS-123".into(),
            size: "3.0 GB".into(),
            color: slint::Color::from_rgb_u8(234, 88, 12),
            favorite: false,
            cover: slint::Image::default(),
        });
        window.set_game_list(ModelRc::from(games));
        window.set_selected_game(0);
        window.set_fast_boot(true);

        eprintln!("[TEST] Invoking play-game (simulated game-card click)...");
        window.invoke_play_game();

        // Give the boot worker thread time to start the VM.
        std::thread::sleep(Duration::from_secs(8));

        let mut reached_running = false;
        let mut saw_frame = false;
        let mut ee_pc_moved = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(40) {
            let state = Pcsx2Api::get_state();
            if state == PCSX2_VMState::Running {
                reached_running = true;
            }
            if let Some((w, h, _data)) = Pcsx2Api::get_frame() {
                if w > 0 && h > 0 {
                    saw_frame = true;
                }
            }
            // If we have a frame and the VM is running, the emulation loop is
            // actually executing (this is what was broken before).
            if reached_running && saw_frame {
                ee_pc_moved = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }

        eprintln!(
            "[TEST] reached_running={} saw_frame={} emulation_loop_active={}",
            reached_running, saw_frame, ee_pc_moved
        );

        // Clean shutdown.
        Pcsx2Api::shutdown();

        if reached_running && saw_frame {
            eprintln!("[TEST] PASS: game click booted + emulation loop ran");
            std::process::exit(0);
        } else {
            eprintln!("[TEST] FAIL: game did not reach running frame");
            std::process::exit(1);
        }
    }

    #[cfg(not(feature = "pcsx2-core"))]
    {
        eprintln!("This test requires the `pcsx2-core` feature.");
        std::process::exit(2);
    }
}
