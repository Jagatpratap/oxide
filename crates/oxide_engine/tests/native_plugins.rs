//! Native plugins: a C library under Assets/Plugins, called through `native_plugin!`.
//! Skipped when no C compiler (`cc`) is available.

use oxide_engine::native_plugins::NativePlugins;

oxide_engine::native_plugin! {
    unsafe "oxidemath" {
        fn oxm_add(a: i32, b: i32) -> i32;
        fn oxm_lerp(a: f32, b: f32, t: f32) -> f32;
    }
}

oxide_engine::native_plugin! {
    unsafe "not_there" {
        fn missing(x: i32) -> i32;
    }
}

#[test]
fn c_plugins_load_and_run() {
    let dir = std::env::temp_dir().join(format!("oxide_plugins_{}", std::process::id()));
    let plugins = dir.join("Assets/Plugins/Math");
    std::fs::create_dir_all(&plugins).unwrap();
    let src = dir.join("oxidemath.c");
    std::fs::write(
        &src,
        "int oxm_add(int a, int b) { return a + b; }\n\
         float oxm_lerp(float a, float b, float t) { return a + (b - a) * t; }\n",
    )
    .unwrap();
    let lib = plugins.join(format!(
        "{}oxidemath{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let built = std::process::Command::new("cc")
        .args(["-shared", "-fPIC", "-o"])
        .arg(&lib)
        .arg(&src)
        .status();
    if !built.is_ok_and(|s| s.success()) {
        eprintln!("no C compiler; skipped");
        return;
    }
    NativePlugins::set_search_root(dir.join("Assets"));
    assert_eq!(
        NativePlugins::find("oxidemath").as_deref(),
        Some(lib.as_path())
    );
    // [DllImport]-style calls.
    assert_eq!(oxm_add(2, 40), 42);
    assert_eq!(oxm_lerp(10.0, 20.0, 0.25), 12.5);
    assert!(NativePlugins::loaded().contains(&"oxidemath".to_owned()));
    // A missing plugin fails like Unity's DllNotFoundException.
    let err = std::panic::catch_unwind(|| missing(1)).unwrap_err();
    let msg = err.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(
        msg.contains("DllNotFoundException") && msg.contains("not_there"),
        "{msg}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plugin_importer_platforms() {
    use oxide_assets::BuildTarget as B;
    use oxide_assets::importers::PluginImporter;
    let mut imp = PluginImporter::default();
    assert!(imp.includes(B::Android, false) && imp.includes(B::Standalone, true));
    imp.any_platform = false;
    imp.android = false;
    imp.editor = false;
    assert!(!imp.includes(B::Android, false));
    assert!(imp.includes(B::Standalone, false));
    assert!(!imp.includes(B::Standalone, true), "editor off");
    assert!(oxide_assets::importers::is_native_plugin(
        "Assets/Plugins/x.DLL"
    ));
    assert!(!oxide_assets::importers::is_native_plugin("Assets/x.dl"));
}
