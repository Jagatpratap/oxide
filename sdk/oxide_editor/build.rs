// A release puts the engine and Rust standard libraries in `<install>/lib`, next to `bin/`.
fn main() {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path/../lib"),
        Ok("linux") => println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN/../lib"),
        _ => {}
    }
}
