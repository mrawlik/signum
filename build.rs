fn main() {
    // Embed the icon into signum.exe so Explorer and the taskbar show it.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("packaging/windows/signum.ico")
            .compile()
            .expect("failed to embed Windows icon");
    }
    println!("cargo:rerun-if-changed=packaging/windows/signum.ico");
}
