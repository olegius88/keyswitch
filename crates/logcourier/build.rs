//! Embeds the application icon, the version information and the manifest (Common Controls 6, Per-Monitor-V2 DPI,
//! `asInvoker`) into `LogCourier.exe`. Only a Windows host builds the resources: `cargo test` and `cargo check --target
//! x86_64-pc-windows-msvc` on Linux compile the crate without them.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=windows/logcourier.manifest");
    println!("cargo:rerun-if-changed=../../packaging/windows/keyswitch.ico");
    #[cfg(windows)]
    embed();
}

#[cfg(windows)]
fn embed() {
    use std::path::PathBuf;

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let mut parts: Vec<u32> = version.split(['.', '-', '+']).map(|part| part.parse().unwrap_or(0)).collect();
    parts.resize(4, 0);
    let numeric = format!("{},{},{},{}", parts[0], parts[1], parts[2], parts[3]);
    let slash = |path: PathBuf| path.display().to_string().replace('\\', "/");
    let icon = slash(manifest_dir.join("../../packaging/windows/keyswitch.ico"));
    let manifest = slash(manifest_dir.join("windows/logcourier.manifest"));
    // Numbers instead of the SDK's names (RT_MANIFEST is 24, VS_VERSION_INFO is 1): no header is needed.
    let script = format!(
        r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "LogCourier"
            VALUE "FileDescription", "LogCourier"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "LogCourier"
            VALUE "LegalCopyright", "GNU GPL-3.0-or-later"
            VALUE "OriginalFilename", "LogCourier.exe"
            VALUE "ProductName", "LogCourier"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    );
    let path = out_dir.join("keyswitch.rc");
    std::fs::write(&path, script).expect("the resource script is written to OUT_DIR");
    embed_resource::compile(&path, embed_resource::NONE).manifest_required().expect("the resources are embedded");
}
