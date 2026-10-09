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
        audio::init();
        audio::init_music();
        std::thread::spawn(main);
        hud::install(module.0 as usize);
    }
    true.into()
}
