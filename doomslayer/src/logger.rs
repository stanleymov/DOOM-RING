use std::{fs::File, iter, panic};

use log::LevelFilter;
use simplelog::{ConfigBuilder, WriteLogger};
use windows::{
    Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW},
    core::{PCWSTR, w},
};

use crate::config;

pub fn init() {
    let path = config::mod_dir().join("doomslayer.log");
    let Ok(file) = File::create(&path) else {
        return;
    };
    let cfg = ConfigBuilder::new().set_time_format_rfc3339().build();
    let _ = WriteLogger::init(LevelFilter::Info, cfg, file);
    log::info!("doomslayer {} loaded from {}", env!("CARGO_PKG_VERSION"), path.display());
}

pub fn set_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let mut msg = format!(
            "doomslayer panicked: {}",
            info.payload_as_str().unwrap_or("no panic message")
        );
        if let Some(l) = info.location() {
            msg += &format!("\n    {}:{}:{}", l.file(), l.line(), l.column());
        }
        log::error!("{msg}");
        log::logger().flush();
        let wide = msg.encode_utf16().chain(iter::once(0)).collect::<Vec<_>>();
        unsafe {
            let _ = MessageBoxW(None, PCWSTR(wide.as_ptr()), w!("doomslayer.dll"), MB_ICONERROR);
        }
    }));
}
