//! DOOM RING - Doom Eternal mechanics inside Elden Ring.
//!
//! Loaded by me3 next to erfps2 (first person camera). Everything runs from one recurring game task:
//! weapons, dash / double jump, glory kills, chainsaw, flame belch, armor, and the agent test bridge.

use std::{ffi::c_void, time::Duration};

use eldenring::{
    cs::{CSTaskGroupIndex, CSTaskImp},
    fd4::FD4TaskData,
};
use fromsoftware_shared::SharedTaskImpExt;
use windows::{
    Win32::{Foundation::HINSTANCE, System::SystemServices::DLL_PROCESS_ATTACH},
    core::BOOL,
};

mod audio;
mod autoloot;
mod bridge;
mod bullet;
mod config;
mod damage;
mod doomhud;
mod fx;
mod game;
mod gamepad;
mod hitter;
mod hud;
mod input;
mod logger;
mod params;
mod pickups;
mod program;
mod raycast;
mod remap;
mod rva;
mod settings_ui;
mod slayer;
mod viewmodel;
mod weapons;

fn main() {
    // The singleton table fills in during boot; early lookups report NotFound (as InvalidRva).
    let started = std::time::Instant::now();
    let cs_task = loop {
        match CSTaskImp::wait_for_instance(Duration::from_secs(5)) {
            Ok(t) => break t,
            Err(e) if started.elapsed() < Duration::from_secs(180) => {
                log::debug!("CSTaskImp not ready: {e}");
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                log::error!("CSTaskImp never became available: {e}");
                return;
            }
        }
    };
    log::info!("doomslayer: task runner ready after {:?}", started.elapsed());

    cs_task.run_recurring(
        |_: &FD4TaskData| {
            slayer::frame();
        },
        CSTaskGroupIndex::FrameBegin,
    );

    // Movement writes happen after physics so the proxy picks the new position up next step.
    cs_task.run_recurring(
        |_: &FD4TaskData| {
            slayer::post_physics();
        },
        CSTaskGroupIndex::ChrIns_PostPhysics,
    );
}

#[unsafe(no_mangle)]
unsafe extern "system" fn DllMain(module: HINSTANCE, reason: u32, _: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        config::set_module(module.0 as usize);
        logger::init();
        logger::set_panic_hook();
        // Wrong game version (a new ELDEN RING patch): one plain message, then the mod stays off
        // and the game runs without it - it used to crash with a panic naming build paths. (On top:
        // the game window covered it.)
        if let Err(found) = rva::check_game() {
            log::error!("unsupported game: {found} (this build: {})", rva::SUPPORTED_GAME);
            let msg = format!(
                "This version of DOOM RING needs {}.\nYour ELDEN RING is {found}.\n\nDOOM RING is turned off for this session; the game starts without it.\nGet the DOOM RING version for your game at github.com/stanleymov/DOOM-RING/releases",
                rva::SUPPORTED_GAME
            );
            std::thread::spawn(move || {
                use windows::{
                    Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW},
                    core::{PCWSTR, w},
                };
                let wide: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
                unsafe {
                    let _ = MessageBoxW(None, PCWSTR(wide.as_ptr()), w!("DOOM RING"), MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND);
                }
            });
            return true.into();
        }
        audio::init();
        audio::init_music();
        std::thread::spawn(main);
        hud::install(module.0 as usize);
    }
    true.into()
}
