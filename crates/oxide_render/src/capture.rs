//! Renders a frame into an offscreen texture and reads it back (screenshots, MCP, tests).

use crate::Gpu;
use std::time::Duration;

/// Tightly packed 8-bit RGBA pixels, top row first.
pub struct CapturedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl CapturedImage {
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&self.rgba)
            .map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
        Ok(out)
    }

    /// RGBA of the pixel at `(x, y)` (origin top-left).
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }
}

/// Calls `draw` to render into a fresh `width`×`height` texture of `format`, then reads it
/// back. Supports 8-bit RGBA/BGRA formats (sRGB or not). Blocks until the GPU is done.
pub fn capture_frame(
    gpu: &Gpu,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
) -> Result<CapturedImage, String> {
    use wgpu::TextureFormat as F;
    let bgra = match format {
        F::Bgra8Unorm | F::Bgra8UnormSrgb => true,
        F::Rgba8Unorm | F::Rgba8UnormSrgb => false,
        other => return Err(format!("capture of {other:?} is not supported yet")),
    };
    if width == 0 || height == 0 {
        return Err("cannot capture an empty (minimized) frame".into());
    }

    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("capture target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let unpadded = width * 4;
    let padded =
        unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("capture readback"),
        size: (padded * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    draw(&mut encoder, &view);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let submission = gpu.queue.submit([encoder.finish()]);

    let (tx, rx) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |r| {
        let _ = tx.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(5)),
        })
        .map_err(|e| format!("GPU readback failed: {e}"))?;
    rx.recv()
        .map_err(|_| "GPU readback was cancelled".to_string())?
        .map_err(|e| format!("GPU readback failed: {e}"))?;

    let mut rgba = Vec::with_capacity((unpadded * height) as usize);
    {
        let mapped = buffer.get_mapped_range(..).map_err(|e| format!("{e:?}"))?;
        for row in mapped.chunks(padded as usize).take(height as usize) {
            rgba.extend_from_slice(&row[..unpadded as usize]);
        }
    }
    buffer.unmap();
    if bgra {
        for px in rgba.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
    }
    Ok(CapturedImage {
        width,
        height,
        rgba,
    })
}
