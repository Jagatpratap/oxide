fn main() {
    // The download the Hub picks from the release list.
    println!(
        "cargo:rustc-env=OXIDE_HUB_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
    // The program icon on Windows (macOS and Linux get theirs from the app bundle and the
    // .desktop file; every platform also sets the window icon at run time).
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.compile().expect("cannot embed the Windows icon");
    }
}
