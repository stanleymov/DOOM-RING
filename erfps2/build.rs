use std::{
    env, fs,
    io::{self, Cursor, Seek, Write},
    process::Command,
};

fn main() -> io::Result<()> {
    bundle_tpf()?;
    compile_shaders()?;

    Ok(())
}

fn bundle_tpf() -> io::Result<()> {
    println!("cargo::rerun-if-changed=textures/MENU_Tuto_18949.dds");
    println!("cargo::rerun-if-changed=textures/MENU_Tuto_18949.tpf");

    let out = format!("{}/MENU_Tuto_18949.tpf", env::var("OUT_DIR").unwrap());

    let mut tpf = Cursor::new(fs::read("textures/MENU_Tuto_18949.tpf")?);
    tpf.seek(io::SeekFrom::End(0))?;

    let size = io::copy(
        &mut fs::File::open("textures/MENU_Tuto_18949.dds")?,
        &mut tpf,
    )?;

    let size = u32::try_from(size).map_err(io::Error::other)?.to_le_bytes();

    tpf.seek(io::SeekFrom::Start(4))?;
    tpf.write_all(&size)?;

    tpf.seek(io::SeekFrom::Start(20))?;
    tpf.write_all(&size)?;

    fs::write(out, tpf.get_ref())?;

    Ok(())
}

fn compile_shaders() -> io::Result<()> {
    println!("cargo::rerun-if-changed=shaders/ToneMap_PostHook.hlsl");

    let out = format!("{}/ToneMap_PostHook.ppo", env::var("OUT_DIR").unwrap());
    let dxc = env::var("DXC_PATH").unwrap_or_else(|_| "dxc".to_owned());

    Command::new(dxc)
        .args([
            "-E",
            "PSMain",
            "-T",
            "ps_6_0",
            "-Fo",
            &out,
            "shaders/ToneMap_PostHook.hlsl",
        ])
        .spawn()?
        .wait()?;

    Ok(())
}
