//! A reader for Blender's `.blend` files, enough to pull a material's packed textures out:
//! the file blocks, the SDNA (Blender's description of its own structs, stored in every
//! file) and typed field access by name. Handles zstd- and gzip-compressed files, the
//! classic header (`BLENDER-v405`, 24-byte block headers) and Blender 5's
//! (`BLENDER17-01v0500`, 32-byte block headers with 64-bit lengths).
//! Reference: <https://developer.blender.org/docs/features/core/blend_file/>

use oxide::assets::deps::{flate2, ruzstd};
use std::collections::HashMap;
use std::io::Read;

/// One field of an SDNA struct.
#[derive(Clone, Debug)]
pub struct Field {
    /// Its name without `*` or array brackets (`filepath`, `packedfile`).
    pub name: String,
    /// Its type name (`char`, `PackedFile`, `ListBase`).
    pub ty: String,
    pub pointer: bool,
    pub offset: usize,
    pub size: usize,
}

/// An SDNA struct: its name, size and fields.
#[derive(Clone, Debug)]
pub struct Struct {
    pub name: String,
    pub size: usize,
    pub fields: Vec<Field>,
}

impl Struct {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// A file block: its 4-character code, SDNA struct index, element count and data.
#[derive(Clone, Debug)]
pub struct Block {
    pub code: [u8; 4],
    pub sdna: usize,
    pub count: usize,
    pub old: u64,
    start: usize,
    len: usize,
}

/// A parsed `.blend` file.
pub struct BlendFile {
    data: Vec<u8>,
    pub version: u32,
    pub blocks: Vec<Block>,
    pub structs: Vec<Struct>,
    by_old: HashMap<u64, usize>,
    struct_by_name: HashMap<String, usize>,
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

fn u64_at(d: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(o..o + 8)?.try_into().ok()?))
}

/// Decompresses a zstd or gzip `.blend` (plain files pass through).
pub fn decompress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        // Blender writes many frames (and a skippable seek-table frame at the end).
        let mut out = Vec::new();
        let mut rest = bytes;
        while rest.len() >= 8 {
            let magic = u32_at(rest, 0).unwrap_or(0);
            if magic & 0xFFFF_FFF0 == 0x184D_2A50 {
                let n = u32_at(rest, 4).unwrap_or(0) as usize;
                rest = rest.get(8 + n..).unwrap_or(&[]);
                continue;
            }
            ruzstd::decoding::StreamingDecoder::new(&mut rest)
                .map_err(|e| format!("zstd: {e}"))?
                .read_to_end(&mut out)
                .map_err(|e| format!("zstd: {e}"))?;
        }
        Ok(out)
    } else if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut out)
            .map_err(|e| format!("gzip: {e}"))?;
        Ok(out)
    } else {
        Ok(bytes.to_vec())
    }
}

impl BlendFile {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let data = decompress(bytes)?;
        if !data.starts_with(b"BLENDER") {
            return Err("not a .blend file".into());
        }
        // Classic: BLENDER + '-'/'_' (pointer size) + 'v'/'V' (endianness) + "405".
        // Blender 5: BLENDER + "17" (header size) + "-01" (format) + 'v' + "0500".
        let (header, large, version) = if data[7].is_ascii_digit() {
            let h = std::str::from_utf8(&data[7..9])
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or("bad header")?;
            if data.get(9) != Some(&b'-') || data.get(12) != Some(&b'v') {
                return Err("unsupported .blend header (big-endian or 32-bit)".into());
            }
            let v = std::str::from_utf8(&data[13..h])
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            (h, true, v)
        } else {
            if data[7] != b'-' || data[8] != b'v' {
                return Err("unsupported .blend header (big-endian or 32-bit)".into());
            }
            let v = std::str::from_utf8(&data[9..12])
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            (12, false, v)
        };
        let mut blocks = Vec::new();
        let mut at = header;
        let mut dna = None;
        loop {
            let (code, sdna, old, len, count, head) = if large {
                // { code, SDNAnr, old, int64 len, int64 nr }
                (
                    data.get(at..at + 4).ok_or("truncated")?,
                    u32_at(&data, at + 4).ok_or("truncated")? as usize,
                    u64_at(&data, at + 8).ok_or("truncated")?,
                    u64_at(&data, at + 16).ok_or("truncated")? as usize,
                    u64_at(&data, at + 24).ok_or("truncated")? as usize,
                    32,
                )
            } else {
                // { code, len, old, SDNAnr, nr }
                (
                    data.get(at..at + 4).ok_or("truncated")?,
                    u32_at(&data, at + 16).ok_or("truncated")? as usize,
                    u64_at(&data, at + 8).ok_or("truncated")?,
                    u32_at(&data, at + 4).ok_or("truncated")? as usize,
                    u32_at(&data, at + 20).ok_or("truncated")? as usize,
                    24,
                )
            };
            let code: [u8; 4] = code.try_into().map_err(|_| "truncated")?;
            let start = at + head;
            if &code == b"ENDB" {
                break;
            }
            if start + len > data.len() {
                return Err("truncated block".into());
            }
            if &code == b"DNA1" {
                dna = Some((start, len));
            }
            blocks.push(Block {
                code,
                sdna,
                count,
                old,
                start,
                len,
            });
            at = start + len;
        }
        let (ds, dl) = dna.ok_or("no SDNA in the file")?;
        let structs = parse_sdna(&data[ds..ds + dl])?;
        let by_old = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.old != 0)
            .map(|(i, b)| (b.old, i))
            .collect();
        let struct_by_name = structs
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.clone(), i))
            .collect();
        Ok(Self {
            data,
            version,
            blocks,
            structs,
            by_old,
            struct_by_name,
        })
    }

    pub fn struct_named(&self, name: &str) -> Option<&Struct> {
        self.struct_by_name.get(name).map(|&i| &self.structs[i])
    }

    /// A block's bytes.
    pub fn bytes(&self, b: &Block) -> &[u8] {
        &self.data[b.start..b.start + b.len]
    }

    /// The block an old (in-file) pointer points at.
    pub fn deref(&self, ptr: u64) -> Option<&Block> {
        (ptr != 0).then(|| self.by_old.get(&ptr).map(|&i| &self.blocks[i]))?
    }

    /// A view of the struct named `ty` at `offset` in a block.
    pub fn view<'a>(&'a self, block: &'a Block, ty: &str, offset: usize) -> Option<View<'a>> {
        let s = self.struct_named(ty)?;
        let bytes = self.bytes(block).get(offset..offset + s.size)?;
        Some(View {
            file: self,
            s,
            bytes,
        })
    }

    /// The first element of a block, typed by its SDNA index.
    pub fn block_view<'a>(&'a self, block: &'a Block) -> Option<View<'a>> {
        let s = self.structs.get(block.sdna)?;
        let bytes = self.bytes(block).get(..s.size)?;
        Some(View {
            file: self,
            s,
            bytes,
        })
    }

    /// The elements of a `ListBase` (`first`/`last` pointers to `ty` structs).
    pub fn list<'a>(&'a self, list: Option<View<'a>>, ty: &str) -> Vec<View<'a>> {
        let mut out = Vec::new();
        let mut next = list.and_then(|l| l.ptr("first")).unwrap_or(0);
        while next != 0 && out.len() < 100_000 {
            let Some(v) = self.deref(next).and_then(|b| self.view(b, ty, 0)) else {
                break;
            };
            next = v.ptr("next").unwrap_or(0);
            out.push(v);
        }
        out
    }
}

/// A struct in a block, read by field name.
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub file: &'a BlendFile,
    pub s: &'a Struct,
    bytes: &'a [u8],
}

impl<'a> View<'a> {
    pub fn type_name(&self) -> &str {
        &self.s.name
    }

    fn field(&self, name: &str) -> Option<&'a Field> {
        self.s.field(name)
    }

    pub fn ptr(&self, name: &str) -> Option<u64> {
        let f = self.field(name)?;
        f.pointer.then(|| u64_at(self.bytes, f.offset))?
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        let f = self.field(name)?;
        match f.size {
            1 => self.bytes.get(f.offset).map(|&b| b as i64),
            2 => u16_at(self.bytes, f.offset).map(|v| v as i16 as i64),
            4 => u32_at(self.bytes, f.offset).map(|v| v as i32 as i64),
            8 => u64_at(self.bytes, f.offset).map(|v| v as i64),
            _ => None,
        }
    }

    /// A `char[]` field up to its first NUL.
    pub fn string(&self, name: &str) -> Option<String> {
        let f = self.field(name)?;
        let b = self.bytes.get(f.offset..f.offset + f.size)?;
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        Some(String::from_utf8_lossy(&b[..end]).into_owned())
    }

    /// An embedded struct field.
    pub fn sub(&self, name: &str) -> Option<View<'a>> {
        let f = self.field(name)?;
        if f.pointer {
            return None;
        }
        let s = self.file.struct_named(&f.ty)?;
        Some(View {
            file: self.file,
            s,
            bytes: self.bytes.get(f.offset..f.offset + s.size)?,
        })
    }

    /// The struct a pointer field points at (typed by the field's type).
    pub fn follow(&self, name: &str) -> Option<View<'a>> {
        let f = self.field(name)?;
        let b = self.file.deref(self.ptr(name)?)?;
        // An `ID *` points at a whole datablock (Image, Material): use the block's own type.
        let s = self
            .file
            .structs
            .get(b.sdna)
            .filter(|s| s.name == f.ty || f.ty == "ID")
            .or_else(|| self.file.struct_named(&f.ty))?;
        Some(View {
            file: self.file,
            s,
            bytes: self.file.bytes(b).get(..s.size)?,
        })
    }

    /// The raw block a pointer field points at.
    pub fn follow_raw(&self, name: &str) -> Option<&'a [u8]> {
        let b = self.file.deref(self.ptr(name)?)?;
        Some(self.file.bytes(b))
    }

    /// An ID datablock's name without its type code (`IMoak.jpg` → `oak.jpg`).
    pub fn id_name(&self) -> Option<String> {
        // `ID.name` starts with the two-letter type code (`IM`, `MA`).
        let n = if self.s.name == "ID" {
            self.string("name")?
        } else {
            self.sub("id")?.string("name")?
        };
        Some(n.get(2..).unwrap_or("").to_owned())
    }
}

/// Parses the `DNA1` block (`SDNA` `NAME` `TYPE` `TLEN` `STRC`).
fn parse_sdna(d: &[u8]) -> Result<Vec<Struct>, String> {
    fn align4(o: usize) -> usize {
        (o + 3) & !3
    }
    fn strings(d: &[u8], at: &mut usize, n: usize) -> Result<Vec<String>, String> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let rest = d.get(*at..).ok_or("SDNA truncated")?;
            let end = rest.iter().position(|&c| c == 0).ok_or("SDNA truncated")?;
            out.push(String::from_utf8_lossy(&rest[..end]).into_owned());
            *at += end + 1;
        }
        Ok(out)
    }
    let tag = |at: usize, t: &[u8]| -> Result<(), String> {
        if d.get(at..at + 4) == Some(t) {
            Ok(())
        } else {
            Err(format!("SDNA: expected {}", String::from_utf8_lossy(t)))
        }
    };
    tag(0, b"SDNA")?;
    tag(4, b"NAME")?;
    let n = u32_at(d, 8).ok_or("SDNA truncated")? as usize;
    let mut at = 12;
    let names = strings(d, &mut at, n)?;
    at = align4(at);
    tag(at, b"TYPE")?;
    let n = u32_at(d, at + 4).ok_or("SDNA truncated")? as usize;
    at += 8;
    let types = strings(d, &mut at, n)?;
    at = align4(at);
    tag(at, b"TLEN")?;
    at += 4;
    let lens: Vec<usize> = (0..types.len())
        .map(|i| u16_at(d, at + i * 2).map(|v| v as usize))
        .collect::<Option<_>>()
        .ok_or("SDNA truncated")?;
    at = align4(at + types.len() * 2);
    tag(at, b"STRC")?;
    let n = u32_at(d, at + 4).ok_or("SDNA truncated")? as usize;
    at += 8;
    let mut structs = Vec::with_capacity(n);
    for _ in 0..n {
        let ty = u16_at(d, at).ok_or("SDNA truncated")? as usize;
        let nf = u16_at(d, at + 2).ok_or("SDNA truncated")? as usize;
        at += 4;
        let mut fields = Vec::with_capacity(nf);
        let mut offset = 0;
        for _ in 0..nf {
            let fty = u16_at(d, at).ok_or("SDNA truncated")? as usize;
            let fname = names
                .get(u16_at(d, at + 2).ok_or("SDNA truncated")? as usize)
                .ok_or("SDNA: bad name index")?;
            at += 4;
            let pointer = fname.starts_with('*') || fname.starts_with("(*");
            // Array dimensions multiply: `name[2][3]`.
            let count: usize = fname
                .split('[')
                .skip(1)
                .filter_map(|p| p.trim_end_matches(']').parse::<usize>().ok())
                .product::<usize>()
                .max(1);
            let elem = if pointer {
                8
            } else {
                *lens.get(fty).ok_or("SDNA: bad type index")?
            };
            let size = elem * count;
            let clean: String = fname
                .trim_start_matches("(*")
                .trim_start_matches('*')
                .split(['[', ')'])
                .next()
                .unwrap_or("")
                .to_owned();
            fields.push(Field {
                name: clean,
                ty: types.get(fty).cloned().unwrap_or_default(),
                pointer,
                offset,
                size,
            });
            offset += size;
        }
        structs.push(Struct {
            name: types.get(ty).cloned().unwrap_or_default(),
            size: *lens.get(ty).unwrap_or(&0),
            fields,
        });
    }
    Ok(structs)
}

/// An image packed into a `.blend` file.
#[derive(Clone, Debug)]
pub struct PackedImage {
    /// The Image datablock's name (`oak_diff_1k.jpg`).
    pub name: String,
    /// Its original file path (`//textures/oak_diff_1k.jpg`).
    pub filepath: String,
    /// The file's bytes (PNG, JPEG, EXR, ...).
    pub data: Vec<u8>,
}

impl BlendFile {
    /// Every packed image (Image datablocks with packed file data).
    pub fn packed_images(&self) -> Vec<PackedImage> {
        let mut out = Vec::new();
        for b in self.blocks.iter().filter(|b| &b.code[..2] == b"IM") {
            let Some(img) = self.block_view(b) else {
                continue;
            };
            let name = img.id_name().unwrap_or_default();
            let filepath = img
                .string("filepath")
                .or_else(|| img.string("name"))
                .unwrap_or_default();
            // Blender 2.8+: a list of ImagePackedFile (one per view or UDIM tile); older
            // files have one `packedfile` pointer.
            let mut packed: Vec<View> = self
                .list(img.sub("packedfiles"), "ImagePackedFile")
                .into_iter()
                .filter_map(|ipf| ipf.follow("packedfile"))
                .collect();
            if packed.is_empty()
                && let Some(pf) = img.follow("packedfile")
            {
                packed.push(pf);
            }
            for pf in packed.into_iter().take(1) {
                let size = pf.int("size").unwrap_or(0).max(0) as usize;
                if let Some(raw) = pf.follow_raw("data") {
                    out.push(PackedImage {
                        name: name.clone(),
                        filepath: filepath.clone(),
                        data: raw[..size.min(raw.len())].to_vec(),
                    });
                }
            }
        }
        out
    }
}

/// What a texture drives on a Principled BSDF (found by following the node links).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureRole {
    BaseColor,
    Roughness,
    /// A gloss map (inverted roughness).
    Smoothness,
    Metallic,
    Normal,
    /// A height map through a Bump node (no Oxide equivalent; kept as a file).
    Height,
    Occlusion,
    Emission,
    Alpha,
}

/// One image texture node of the material and what it feeds.
#[derive(Clone, Debug)]
pub struct MaterialTexture {
    /// The Image datablock name.
    pub image: String,
    pub role: Option<TextureRole>,
}

/// A material's base values and textures, read from its node tree.
#[derive(Clone, Debug, Default)]
pub struct MaterialInfo {
    pub name: String,
    pub base_color: Option<[f32; 4]>,
    pub metallic: Option<f32>,
    pub roughness: Option<f32>,
    pub textures: Vec<MaterialTexture>,
}

impl BlendFile {
    /// Materials with their Principled BSDF inputs traced back to image textures.
    pub fn materials(&self) -> Vec<MaterialInfo> {
        let mut out = Vec::new();
        for b in self.blocks.iter().filter(|b| &b.code[..2] == b"MA") {
            let Some(ma) = self.block_view(b) else {
                continue;
            };
            let mut info = MaterialInfo {
                name: ma.id_name().unwrap_or_default(),
                ..Default::default()
            };
            if let Some(tree) = ma.follow("nodetree") {
                self.trace_tree(&tree, &mut info);
            }
            out.push(info);
        }
        out
    }

    fn trace_tree(&self, tree: &View, info: &mut MaterialInfo) {
        let nodes = self.list(tree.sub("nodes"), "bNode");
        let links = self.list(tree.sub("links"), "bNodeLink");
        // Nodes and sockets by old pointer.
        let node_ptr = |v: &View| self.ptr_of(v);
        let node_ptrs: Vec<u64> = nodes.iter().map(node_ptr).collect();
        let idname = |n: &View| n.string("idname").unwrap_or_default();
        let socket_name = |p: u64| {
            self.deref(p)
                .and_then(|b| self.view(b, "bNodeSocket", 0))
                .and_then(|s| s.string("name"))
                .unwrap_or_default()
        };
        // Downstream (to_node, to_socket name) of each node.
        let mut down: HashMap<u64, Vec<(u64, String)>> = HashMap::new();
        for l in &links {
            let (Some(f), Some(t), Some(ts)) =
                (l.ptr("fromnode"), l.ptr("tonode"), l.ptr("tosock"))
            else {
                continue;
            };
            down.entry(f).or_default().push((t, socket_name(ts)));
        }
        let node_of = |p: u64| node_ptrs.iter().position(|&q| q == p).map(|i| nodes[i]);
        // The Principled BSDF's unlinked values.
        for n in &nodes {
            if idname(n) != "ShaderNodeBsdfPrincipled" {
                continue;
            }
            for s in self.list(n.sub("inputs"), "bNodeSocket") {
                let name = s.string("name").unwrap_or_default();
                let block = s.ptr("default_value").and_then(|p| self.deref(p));
                // bNodeSocketValueRGBA { float value[4] } / bNodeSocketValueFloat
                // { int subtype; float value; ... }.
                let ty = if name == "Base Color" {
                    "bNodeSocketValueRGBA"
                } else {
                    "bNodeSocketValueFloat"
                };
                let value = block.and_then(|b| {
                    let off = self.struct_named(ty)?.field("value")?.offset;
                    self.bytes(b).get(off..)
                });
                let f = |i: usize| {
                    value
                        .and_then(|v| v.get(i * 4..i * 4 + 4))
                        .map(|x| f32::from_le_bytes(x.try_into().unwrap_or([0; 4])))
                };
                match name.as_str() {
                    "Base Color" => {
                        if let (Some(r), Some(g), Some(b), Some(a)) = (f(0), f(1), f(2), f(3)) {
                            info.base_color = Some([r, g, b, a]);
                        }
                    }
                    "Metallic" => info.metallic = f(0),
                    "Roughness" => info.roughness = f(0),
                    _ => {}
                }
            }
        }
        for n in &nodes {
            if idname(n) != "ShaderNodeTexImage" {
                continue;
            }
            let Some(image) = n.follow("id").and_then(|i| i.id_name()) else {
                continue;
            };
            // Walk downstream (through Normal Map, Bump, Invert, Separate, Math, Mix nodes)
            // to the Principled BSDF.
            let mut role = None;
            let mut frontier = vec![(self.ptr_of(n), false, false, 0)];
            while let Some((p, inverted, bump, depth)) = frontier.pop() {
                if depth > 6 || role.is_some() {
                    break;
                }
                for (to, sock) in down.get(&p).into_iter().flatten() {
                    let Some(tn) = node_of(*to) else { continue };
                    let id = idname(&tn);
                    if id == "ShaderNodeBsdfPrincipled" {
                        role = match sock.as_str() {
                            "Base Color" => Some(TextureRole::BaseColor),
                            "Roughness" if inverted => Some(TextureRole::Smoothness),
                            "Roughness" => Some(TextureRole::Roughness),
                            "Metallic" => Some(TextureRole::Metallic),
                            "Normal" if bump => Some(TextureRole::Height),
                            "Normal" => Some(TextureRole::Normal),
                            "Alpha" => Some(TextureRole::Alpha),
                            "Emission" | "Emission Color" => Some(TextureRole::Emission),
                            _ => None,
                        };
                        if role.is_some() {
                            break;
                        }
                    } else if id == "ShaderNodeOutputMaterial" && sock == "Displacement" {
                        role = Some(TextureRole::Height);
                        break;
                    } else {
                        let inv = inverted ^ (id == "ShaderNodeInvert");
                        let bump = bump || id == "ShaderNodeBump" || id == "ShaderNodeDisplacement";
                        // An AO map multiplied into the base color.
                        if (id == "ShaderNodeMix" || id == "ShaderNodeMixRGB")
                            && guess_role(&image) == Some(TextureRole::Occlusion)
                        {
                            role = Some(TextureRole::Occlusion);
                            break;
                        }
                        frontier.push((*to, inv, bump, depth + 1));
                    }
                }
            }
            info.textures.push(MaterialTexture {
                role: role.or_else(|| guess_role(&image)),
                image,
            });
        }
    }

    /// The old pointer of a view that starts a block (nodes and other list elements).
    fn ptr_of(&self, v: &View) -> u64 {
        let start = v.bytes.as_ptr() as usize;
        self.blocks
            .iter()
            .find(|b| self.data[b.start..].as_ptr() as usize == start)
            .map_or(0, |b| b.old)
    }
}

/// A texture's role from its name (`oak_rough_1k.jpg`, `Metal_Normal.png`), for textures
/// whose node links don't say.
pub fn guess_role(name: &str) -> Option<TextureRole> {
    let n = name.to_ascii_lowercase();
    let words: Vec<&str> = n
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |keys: &[&str]| words.iter().any(|w| keys.contains(w));
    if has(&["normal", "nor", "nrm", "norm", "normalgl", "nor_gl", "gl"]) || n.contains("normal") {
        Some(TextureRole::Normal)
    } else if has(&["rough", "roughness", "rgh"]) || n.contains("rough") {
        Some(TextureRole::Roughness)
    } else if has(&["gloss", "glossiness", "smoothness"]) {
        Some(TextureRole::Smoothness)
    } else if has(&["metal", "metallic", "metalness", "mtl"]) || n.contains("metallic") {
        Some(TextureRole::Metallic)
    } else if has(&["ao", "occlusion", "ambientocclusion"]) {
        Some(TextureRole::Occlusion)
    } else if has(&["disp", "displacement", "height", "bump"]) {
        Some(TextureRole::Height)
    } else if has(&["emission", "emissive", "emit"]) {
        Some(TextureRole::Emission)
    } else if has(&["opacity", "alpha", "mask"]) {
        Some(TextureRole::Alpha)
    } else if has(&[
        "diff",
        "diffuse",
        "albedo",
        "color",
        "col",
        "basecolor",
        "base",
        "basecolour",
    ]) || n.contains("albedo")
        || n.contains("basecolor")
    {
        Some(TextureRole::BaseColor)
    } else {
        None
    }
}
