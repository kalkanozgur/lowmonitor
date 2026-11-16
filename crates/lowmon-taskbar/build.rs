// Örnek exe'ye manifest (MSVC link.exe); lowmon-tray aynı dosyayı kendi build.rs'iyle gömer.
fn main() {
    let m = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("app.manifest");
    println!("cargo:rerun-if-changed={}", m.display());
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-examples=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-examples=/MANIFESTINPUT:{}",
            m.display()
        );
    }
}
