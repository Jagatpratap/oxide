//! The package's tests. The crate links the engine dylib and libtest only exists as a static
//! library, so this is a plain program (`harness = false`), run explicitly (see Cargo.toml):
//! `cargo test -p oxide_blenderkit --test package` runs the offline tests, `-- --ignored`
//! also the ones that talk to blenderkit.com, and `BLEND_FILE=path` dumps a .blend.

#![allow(clippy::unwrap_used)]

use oxide::assets::AssetServer;
use oxide::assets::deps::{image, serde_json};
use oxide::scene::{MeshRenderer, RenderSettings, World};
use oxide_blenderkit::api::*;
use oxide_blenderkit::blend::*;
use oxide_blenderkit::import::*;
use oxide_blenderkit::settings::*;
use serde_json::json;

fn builds_search_urls() {
    let u = search_url(AssetType::Model, "office chair", PriceFilter::Free, 24);
    assert_eq!(
        u,
        "https://www.blenderkit.com/api/v1/search/?query=office+chair+asset_type:model+last_gltf_godot_upload_gte:2000-01-01+is_free:true+order:_score&dict_parameters=1&page_size=24"
    );
    let u = search_url(AssetType::Hdr, "", PriceFilter::All, 10);
    assert!(
        u.contains("query=asset_type:hdr+order:-last_blend_upload&"),
        "{u}"
    );
}

fn parses_search_results() {
    let v = json!({
        "count": 2,
        "next": "http://www.blenderkit.com/api/v1/search/?page=2",
        "results": [{
            "id": "a1", "assetBaseId": "b1", "name": "chair", "displayName": "Chair",
            "assetType": "material", "isFree": true, "license": "cc_zero",
            "author": {"fullName": "Ann"}, "tags": ["wood"],
            "thumbnailSmallUrl": "https://t/s.jpg",
            "ratingsAverage": {"quality": 4.5},
            "files": [
                {"fileType": "blend", "downloadUrl": "https://d/1/", "filename": "x/blend_1.blend", "fileUploadSize": 10},
                {"fileType": "resolution_2K", "downloadUrl": "https://d/2/", "filename": "x/r.blend"},
                {"fileType": "resolution_0_5K", "downloadUrl": "https://d/3/", "filename": "x/r.blend"}
            ]
        }]
    });
    let p = parse_search(&v);
    assert_eq!(p.count, 2);
    assert_eq!(
        p.next.as_deref(),
        Some("https://www.blenderkit.com/api/v1/search/?page=2")
    );
    let a = &p.results[0];
    assert_eq!(
        (a.name.as_str(), a.author.as_str(), a.license_label()),
        ("Chair", "Ann", "CC0")
    );
    assert_eq!(a.quality, Some(4.5));
    assert_eq!(a.resolutions(), vec!["0_5K", "2K"]);
    assert_eq!(
        a.import_file(Some("2K")).unwrap().download_url,
        "https://d/2/"
    );
    assert_eq!(a.import_file(Some("8K")).unwrap().file_type, "blend");
    assert_eq!(a.files[0].extension, "blend");
}

fn parses_profiles() {
    let p = parse_profile(&json!({"fullName": "Ann B", "email": "a@b", "currentPlanName": "Full"}));
    assert!(p.has_full_plan());
    assert!(!parse_profile(&json!({"currentPlanName": "Free"})).has_full_plan());
}

fn live_search_and_free_download_link() {
    let op = search(
        &search_url(AssetType::Model, "chair", PriceFilter::Free, 3),
        "",
    );
    op.block();
    assert_eq!(error_of(&op), None);
    let page = parse_search(&op.json().unwrap());
    assert!(page.count > 0);
    let a = &page.results[0];
    assert!(a.is_free);
    let f = a.import_file(None).expect("glTF");
    assert_eq!(f.extension, "glb");
    let link = download_link(f, "", "00000000-0000-0000-0000-000000000001");
    link.block();
    assert_eq!(error_of(&link), None);
    assert!(
        parse_download_link(&link.json().unwrap())
            .unwrap()
            .starts_with("https://")
    );
    // A Full Plan asset without a key is refused.
    let op = search(&search_url(AssetType::Model, "", PriceFilter::Full, 1), "");
    op.block();
    let a = &parse_search(&op.json().unwrap()).results[0];
    let link = download_link(
        a.import_file(None).unwrap(),
        "",
        "00000000-0000-0000-0000-000000000001",
    );
    link.block();
    assert!(error_of(&link).unwrap().contains("Full Plan"));
}

fn dump_blend() {
    let Ok(path) = std::env::var("BLEND_FILE") else {
        return;
    };
    let f = BlendFile::parse(&std::fs::read(path).unwrap()).unwrap();
    println!(
        "version {} blocks {} structs {}",
        f.version,
        f.blocks.len(),
        f.structs.len()
    );
    for i in f.packed_images() {
        println!(
            "image {:?} {:?} {} bytes {:?}",
            i.name,
            i.filepath,
            i.data.len(),
            &i.data[..4.min(i.data.len())]
        );
    }
    for m in f.materials() {
        println!("material {:?}", m);
    }
}

fn guesses_roles_from_names() {
    use TextureRole::*;
    for (n, r) in [
        ("oak_diff_1k.jpg", Some(BaseColor)),
        ("Wood_BaseColor.png", Some(BaseColor)),
        ("oak_nor_gl_1k.png", Some(Normal)),
        ("Metal_Roughness.png", Some(Roughness)),
        ("metal_metallic.jpg", Some(Metallic)),
        ("rock_ao.png", Some(Occlusion)),
        ("rock_disp_2k.png", Some(Height)),
        ("whatever.png", None),
    ] {
        assert_eq!(guess_role(n), r, "{n}");
    }
}

fn reads_packed_images_from_a_synthetic_file() {
    let mut sdna = Vec::new();
    sdna.extend(b"SDNANAME");
    let names = [
        "name[66]",
        "*first",
        "*last",
        "*packedfile",
        "*next",
        "*prev",
        "size",
        "seek",
        "*data",
        "id",
        "packedfiles",
        "filepath[16]",
    ];
    sdna.extend((names.len() as u32).to_le_bytes());
    for n in names {
        sdna.extend(n.as_bytes());
        sdna.push(0);
    }
    while sdna.len() % 4 != 0 {
        sdna.push(0);
    }
    let types = [
        "char",
        "int",
        "void",
        "ID",
        "ListBase",
        "Image",
        "ImagePackedFile",
        "PackedFile",
    ];
    sdna.extend(b"TYPE");
    sdna.extend((types.len() as u32).to_le_bytes());
    for t in types {
        sdna.extend(t.as_bytes());
        sdna.push(0);
    }
    while sdna.len() % 4 != 0 {
        sdna.push(0);
    }
    sdna.extend(b"TLEN");
    // ID = char name[66] (pad to 72 is fine: just 66 here), ListBase 16, Image 66+16+16.
    let lens: [u16; 8] = [1, 4, 0, 66, 16, 98, 24, 16];
    for l in lens {
        sdna.extend(l.to_le_bytes());
    }
    while sdna.len() % 4 != 0 {
        sdna.push(0);
    }
    sdna.extend(b"STRC");
    let structs: Vec<(u16, Vec<(u16, u16)>)> = vec![
        (3, vec![(0, 0)]),                   // ID { char name[66] }
        (4, vec![(2, 1), (2, 2)]),           // ListBase { void *first, *last }
        (5, vec![(3, 9), (4, 10), (0, 11)]), // Image { ID id; ListBase packedfiles; char filepath[16] }
        (6, vec![(6, 4), (6, 5), (7, 3)]), // ImagePackedFile { *next, *prev, PackedFile *packedfile }
        (7, vec![(1, 6), (1, 7), (2, 8)]), // PackedFile { int size, seek; void *data }
    ];
    sdna.extend((structs.len() as u32).to_le_bytes());
    for (t, fs) in &structs {
        sdna.extend(t.to_le_bytes());
        sdna.extend((fs.len() as u16).to_le_bytes());
        for (ft, fname) in fs {
            sdna.extend(ft.to_le_bytes());
            sdna.extend(fname.to_le_bytes());
        }
    }
    let mut file = b"BLENDER-v405".to_vec();
    let mut block = |code: &[u8; 4], old: u64, sdna_nr: u32, data: &[u8]| {
        file.extend(code);
        file.extend((data.len() as u32).to_le_bytes());
        file.extend(old.to_le_bytes());
        file.extend(sdna_nr.to_le_bytes());
        file.extend(1u32.to_le_bytes());
        file.extend(data);
    };
    let png = b"\x89PNG fake image bytes";
    let mut image = vec![0u8; 98];
    image[..10].copy_from_slice(b"IMwood_nor");
    image[66..74].copy_from_slice(&0x2000u64.to_le_bytes()); // packedfiles.first
    image[74..82].copy_from_slice(&0x2000u64.to_le_bytes());
    image[82..90].copy_from_slice(b"//n.png\0");
    block(b"IM\0\0", 0x1000, 2, &image);
    let mut ipf = vec![0u8; 24];
    ipf[16..24].copy_from_slice(&0x3000u64.to_le_bytes());
    block(b"DATA", 0x2000, 3, &ipf);
    let mut pf = vec![0u8; 16];
    pf[..4].copy_from_slice(&(png.len() as u32).to_le_bytes());
    pf[8..16].copy_from_slice(&0x4000u64.to_le_bytes());
    block(b"DATA", 0x3000, 4, &pf);
    block(b"DATA", 0x4000, 0, png);
    block(b"DNA1", 0, 0, &sdna);
    file.extend(b"ENDB");
    file.extend([0u8; 20]);

    let f = BlendFile::parse(&file).unwrap();
    assert_eq!(f.version, 405);
    let imgs = f.packed_images();
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].name, "wood_nor");
    assert_eq!(imgs[0].filepath, "//n.png");
    assert_eq!(imgs[0].data, png);
}

fn safe_names() {
    assert_eq!(safe_name("Oak Wood #2"), "Oak_Wood_2");
    assert_eq!(safe_name("  ../x  "), "x");
    assert_eq!(safe_name("???"), "Asset");
}

fn packs_roughness_and_metallic() {
    let mut png = Vec::new();
    image::GrayImage::from_pixel(2, 2, image::Luma([200u8]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let rough = PackedImage {
        name: "r".into(),
        filepath: String::new(),
        data: png,
    };
    let out = pack_metal_rough(Some(&rough), None, None, 0.5, 0.25).unwrap();
    let img = image::load_from_memory(&out).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (2, 2));
    assert_eq!(img.get_pixel(0, 0).0, [255, 200, 64, 255]);
}

fn live_import_free_assets() {
    let root = std::env::temp_dir().join(format!("oxide_bk_import_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Assets")).unwrap();
    let mut world = World::new();
    oxide::init_world(&mut world, root.clone());
    let uuid = "00000000-0000-0000-0000-000000000001";
    let fetch = |kind: AssetType, q: &str, res: Option<&str>| -> (Asset, Vec<u8>, String) {
        let op = search(&search_url(kind, q, PriceFilter::Free, 5), "");
        op.block();
        let page = parse_search(&op.json().unwrap());
        let asset = page.results.into_iter().next().expect("a result");
        let file = asset.import_file(res).expect("a file").clone();
        let link = download_link(&file, "", uuid);
        link.block();
        assert_eq!(error_of(&link), None);
        let url = parse_download_link(&link.json().unwrap()).unwrap();
        let dl = get(&url, "");
        dl.block();
        assert_eq!(error_of(&dl), None);
        (asset, dl.data(), file.extension)
    };
    let (a, glb, ext) = fetch(AssetType::Model, "chair", None);
    assert_eq!(ext, "glb");
    let path = import_model(&mut world, &a, &glb).unwrap();
    let root_id = place_model(&mut world, &path, oxide::math::Vector3::ZERO).unwrap();
    let renderers = world
        .descendants(root_id)
        .into_iter()
        .filter(|&id| world.get_component::<MeshRenderer>(id).is_some())
        .count();
    println!("model {} -> {path}, {renderers} renderers", a.name);
    assert!(renderers > 0);

    let (a, blend, _) = fetch(AssetType::Material, "wood", Some("1K"));
    let m = import_material(&mut world, &a, &blend).unwrap();
    println!("material {} -> {:?}", a.name, m);
    assert!(m.textures.iter().any(|(r, _)| r == "BaseColor"), "{m:?}");
    let n = apply_material(&mut world, &[root_id], &m.path).unwrap();
    assert_eq!(n, renderers);

    let (a, hdr, ext) = fetch(AssetType::Hdr, "sky", Some("1K"));
    let sky = import_hdri(&mut world, &a, &hdr, &ext, "1K").unwrap();
    set_skybox(&mut world, &sky).unwrap();
    println!("hdri {} -> {sky}", a.name);
    let assets = world.resource::<AssetServer>();
    let mat = assets
        .materials
        .get(
            world
                .resource::<RenderSettings>()
                .skybox_material
                .as_ref()
                .unwrap(),
        )
        .unwrap();
    let tex = assets.textures.get(mat.base_map.as_ref().unwrap()).unwrap();
    assert!(tex.width >= 1024, "{}", tex.width);
    let _ = std::fs::remove_dir_all(&root);
}

fn api_key_round_trip_is_private() {
    let dir = std::env::temp_dir().join(format!("oxide_bk_settings_{}", std::process::id()));
    let file = dir.join("BlenderKit.json");
    save_api_key(&file, " abc123 ").unwrap();
    if std::env::var("BLENDERKIT_API_KEY").is_err() {
        assert_eq!(load_api_key(&file), "abc123");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    save_api_key(&file, "").unwrap();
    assert!(!std::fs::read_to_string(&file).unwrap().contains("abc123"));
    let id = project_uuid(&dir);
    assert_eq!(id.len(), 36);
    assert_eq!(&id[14..15], "4");
    assert_eq!(project_uuid(&dir), id);
    let _ = std::fs::remove_dir_all(&dir);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ignored = args
        .iter()
        .any(|a| a == "--ignored" || a == "--include-ignored");
    let filter: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let tests: &[(&str, bool, fn())] = &[
        ("api::builds_search_urls", false, builds_search_urls as fn()),
        (
            "api::parses_search_results",
            false,
            parses_search_results as fn(),
        ),
        ("api::parses_profiles", false, parses_profiles as fn()),
        (
            "api::live_search_and_free_download_link",
            true,
            live_search_and_free_download_link as fn(),
        ),
        ("blend::dump_blend", true, dump_blend as fn()),
        (
            "blend::guesses_roles_from_names",
            false,
            guesses_roles_from_names as fn(),
        ),
        (
            "blend::reads_packed_images_from_a_synthetic_file",
            false,
            reads_packed_images_from_a_synthetic_file as fn(),
        ),
        ("import::safe_names", false, safe_names as fn()),
        (
            "import::packs_roughness_and_metallic",
            false,
            packs_roughness_and_metallic as fn(),
        ),
        (
            "import::live_import_free_assets",
            true,
            live_import_free_assets as fn(),
        ),
        (
            "settings::api_key_round_trip_is_private",
            false,
            api_key_round_trip_is_private as fn(),
        ),
    ];
    let (mut passed, mut failed, mut skipped) = (0, 0, 0);
    for (name, is_ignored, f) in tests {
        if !filter.is_empty() && !filter.iter().any(|q| name.contains(q.as_str())) {
            continue;
        }
        if *is_ignored && !ignored {
            skipped += 1;
            continue;
        }
        match std::panic::catch_unwind(f) {
            Ok(()) => {
                println!("test {name} ... ok");
                passed += 1;
            }
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    println!(
        "\ntest result: {}. {passed} passed; {failed} failed; {skipped} ignored",
        if failed == 0 { "ok" } else { "FAILED" }
    );
    if failed > 0 {
        std::process::exit(101);
    }
}
