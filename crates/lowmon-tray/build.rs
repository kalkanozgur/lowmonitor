// Manifest (supportedOS Win10/11): taskbar göstergesi katmanlı alt pencere (ADR-0032). MSVC link.exe.
fn main() {
    let m = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../lowmon-taskbar/app.manifest");
    println!("cargo:rerun-if-changed={}", m.display());
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", m.display());
    }
}
