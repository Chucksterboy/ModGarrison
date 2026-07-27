fn main() {
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons/taskbar-icon.png");
    println!("cargo:rerun-if-changed=icons/tray-icon.png");
    tauri_build::build()
}
