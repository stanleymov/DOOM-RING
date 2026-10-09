//! "Setup DOOM RING.exe": starts the setup wizard with the bundled Python (runtime\pythonw.exe
//! setup\setup.pyw) - no console window. The icon gets embedded later (user makes it).
#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let py = dir.join("runtime").join("pythonw.exe");
    let script = dir.join("setup").join("setup.pyw");
    if Command::new(&py).arg(&script).current_dir(&dir).spawn().is_err() {
        // (no console to print to: show a message box through the shell)
        let _ = Command::new("mshta")
            .arg("javascript:alert('DOOM RING setup could not start: the runtime folder is missing. Please extract the whole download first.');close()")
            .spawn();
    }
}
