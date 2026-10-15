// A fresh id for every build of the engine library: scripts compiled against a different
// build than the running editor are refused instead of crashing.
fn main() {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    println!(
        "cargo:rustc-env=OXIDE_BUILD_ID={}-{t:x}",
        env!("CARGO_PKG_VERSION")
    );
}
