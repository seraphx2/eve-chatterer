fn main() {
    // tauri-build embeds icons/icon.ico as a Windows resource but does not
    // reliably rebuild when the icon changes, which shipped a stale icon once.
    println!("cargo:rerun-if-changed=icons");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    tauri_build::build()
}
