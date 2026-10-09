//! Embeds the icon + version info (doomring.rc -> .res with llvm-rc, linked into the exe).
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=doomring.rc");
    println!("cargo:rerun-if-changed=doomring.ico");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("doomring.res");
    let rc = env::var("LLVM_RC").unwrap_or_else(|_| {
        let home = env::var("USERPROFILE").unwrap_or_default();
        format!("{home}/modtools/llvm/bin/llvm-rc.exe")
    });
    let ok = Command::new(&rc).args(["/fo"]).arg(&out).arg("doomring.rc").status().map(|s| s.success()).unwrap_or(false);
    if ok {
        println!("cargo:rustc-link-arg-bins={}", out.display());
    } else {
        println!("cargo:warning=llvm-rc not found - building without the icon");
    }
}
