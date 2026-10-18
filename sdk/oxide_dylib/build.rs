// Points the compiler at the prebuilt engine library (`<install>/lib`), after checking that
// this compiler is the one the library was built with: Rust libraries only link with the
// exact compiler version that made them.
use std::path::PathBuf;

fn fail(msg: String) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let lib = manifest.join("../../lib");
    let lib = lib.canonicalize().unwrap_or(lib);
    let info_path = lib.join("sdk.txt");
    println!("cargo:rerun-if-changed={}", info_path.display());
    let info = std::fs::read_to_string(&info_path).unwrap_or_else(|e| {
        fail(format!(
            "{}: {e}. This Oxide install has no prebuilt engine library; reinstall it from the Oxide Hub.",
            info_path.display()
        ))
    });
    let value = |key: &str| {
        info.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let have = std::process::Command::new(&rustc)
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    let want = value("rustc");
    if have != want {
        fail(format!(
            "this Oxide editor was built with {want}, but the Scripts are being compiled with \
             {have}. Install that version (`rustup toolchain install {}`); the project's \
             rust-toolchain.toml selects it.",
            value("channel")
        ));
    }
    let target = std::env::var("TARGET").unwrap_or_default();
    if target != value("target") {
        fail(format!(
            "this Oxide editor is for {}, not {target}",
            value("target")
        ));
    }
    println!("cargo:rustc-link-search=all={}", lib.display());
    println!(
        "cargo:rustc-link-search=dependency={}",
        lib.join("deps").display()
    );
}
