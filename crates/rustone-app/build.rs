//! Gives rustone.exe its icon and version information on Windows, for
//! Explorer, the taskbar and the installers' shortcuts (`rustone.exe,0`).

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/icons/rustone.ico");
    // The target, not the host this build script runs on.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../packaging/icons/rustone.ico").set("ProductName", "RusTone").set("FileDescription", "RusTone");
        resource.compile().expect("compiling the Windows resources (needs the Windows SDK's rc.exe)");
    }
}
