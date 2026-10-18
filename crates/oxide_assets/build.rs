fn main() {
    // intel_tex_2's prebuilt ISPC kernels contain C++ objects but don't ask for the C++ runtime,
    // which Linux doesn't link by default (macOS and MSVC do).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=stdc++");
    }
}
