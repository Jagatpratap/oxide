# BlenderKit for Oxide

Search [BlenderKit](https://www.blenderkit.com) from the editor and import its assets.

**Install:** Window → Package Manager → *Packages: Built-in* → BlenderKit → Install.
The Scripts library recompiles, and the **BlenderKit** window opens as a tab next to the
Inspector. Window → BlenderKit reopens it after you close it.

| Kind | What you get | Button |
|---|---|---|
| Models | BlenderKit's glTF export for game engines (`.glb`, WebP textures) | **Add to Scene** (at the origin, selected) |
| Materials | The textures packed in the material's `.blend`, wired into a Lit `.mat` by following its Principled BSDF links (base color, normal, roughness + metallic packed into one map, AO, emission; height maps are saved alongside) | **Apply to Selection** |
| HDRIs | The EXR/HDR at the chosen resolution and a Panoramic skybox material | **Set as Skybox** |

**Import to Assets** downloads without touching the scene. Everything lands in
`Assets/BlenderKit/<Models|Materials|HDRIs>/<Name>/`, with a `BlenderKit.json` recording the
author and license. Assets the project already has aren't downloaded again.

## Account and Full Plan assets

Free assets need no account. To download Full Plan assets, open **👤 Sign in** in the window,
paste the API key from your [BlenderKit profile](https://www.blenderkit.com/profile/) and press
Save. The key is stored for your user only, in Oxide's settings folder
(`~/Library/Application Support/Oxide/BlenderKit.json` on macOS, readable only by you), never in
the project. `BLENDERKIT_API_KEY` overrides it.

## Limits

- Only models with BlenderKit's glTF export are listed (most are). Geometry-node and
  procedural materials without image textures import as their base color only.
- Height/displacement maps have no Lit input; they're saved next to the material.

## Writing your own package like this

`package.json` names the crate function that registers editor extensions:

```json
"oxide": { "editorExtensions": "register" }
```

`Cargo.toml` depends on the engine through the project workspace:

```toml
[dependencies]
oxide = { workspace = true }
```

and `register` adds windows, menu items, tools and drawers (`oxide::EditorExtensions`). A
window can dock next to a built-in one and open the first time a project has the package:

```rust
ext.window::<MyWindow>("My Tool", "Window/My Tool")
    .dock(oxide::WindowDock::Inspector)
    .open_by_default();
```

## Tests

```sh
cargo test -p oxide_blenderkit --test package               # offline
cargo test -p oxide_blenderkit --test package -- --ignored  # also downloads from blenderkit.com
```
