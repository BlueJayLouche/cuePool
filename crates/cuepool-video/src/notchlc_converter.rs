//! GPU-native NotchLC decode and conversion into CuePool's shared RGBA canvas.
//!
//! Mirrors [`crate::HapConverter`]: `upload` stages a frame, `encode` records
//! into the caller's encoder, and nothing here submits — the host keeps its own
//! submit schedule, which is what frame pacing depends on.
//!
//! The difference from HAP is one extra pass. HAP hands the GPU a BC texture it
//! can sample directly, so a single fit pass suffices. NotchLC has to be decoded
//! first, by a compute dispatch, into an RGBA texture the fit pass then samples.
//! Both go in the same encoder, so it is still one submit per frame.

use crate::frame::{FramePixels, VideoFrame};
use crate::yuv_converter::fit_rects;
use cuepool_core::CanvasFit;
use notchlc_rs::GpuDecoder;
use std::sync::Arc;
use wgpu::{Device, Queue, TextureFormat, TextureView};

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    dst_min: [f32; 2],
    dst_max: [f32; 2],
    src_min: [f32; 2],
    src_max: [f32; 2],
}

pub struct NotchLcConverter {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    decoder: Option<(GpuDecoder, (u32, u32))>,
    /// One bind group per texture in the decoder's ring, built on first sight.
    binds: Vec<(Arc<wgpu::Texture>, wgpu::BindGroup)>,
    staged: bool,
}

impl NotchLcConverter {
    pub fn new(device: &Device, target_format: TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("notchlc-fit-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("notchlc-fit-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("notchlc-fit-pl"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("notchlc-fit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("notchlc-fit-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("notchlc-fit-uniform"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            layout,
            sampler,
            uniform,
            decoder: None,
            binds: Vec::new(),
            staged: false,
        }
    }

    /// Stage one frame: the payload and its block offsets go to the GPU, and
    /// the fit rectangles are written. No work is recorded or submitted here.
    pub fn upload(
        &mut self,
        device: &Device,
        queue: &Queue,
        frame: &VideoFrame,
        canvas_size: [u32; 2],
        fit: CanvasFit,
    ) -> Result<(), String> {
        let FramePixels::NotchLc {
            payload,
            header,
            bit_offsets,
        } = &frame.pixels
        else {
            return Err("NotchLC converter received a non-NotchLC frame".into());
        };
        if frame.width == 0 || frame.height == 0 {
            return Err("NotchLC frame has zero dimensions".into());
        }
        if (header.width, header.height) != (frame.width, frame.height) {
            return Err(format!(
                "NotchLC header is {}x{} but the frame claims {}x{}",
                header.width, header.height, frame.width, frame.height
            ));
        }
        let limit = device.limits().max_texture_dimension_2d;
        if frame.width > limit || frame.height > limit {
            return Err(format!(
                "dimensions {}x{} exceed the device texture limit {limit}",
                frame.width, frame.height
            ));
        }

        // A dimension change means new textures, so the cached bind groups no
        // longer refer to anything this converter will draw.
        let dimensions = (frame.width, frame.height);
        if self.decoder.as_ref().is_none_or(|(_, dims)| *dims != dimensions) {
            self.decoder = Some((GpuDecoder::new(device, frame.width, frame.height), dimensions));
            self.binds.clear();
        }
        let (decoder, _) = self.decoder.as_mut().expect("decoder just set");
        decoder.prepare(queue, payload, header, bit_offsets);

        let (src_min, src_max, dst_min, dst_max) =
            fit_rects(frame.width, frame.height, canvas_size[0], canvas_size[1], fit);
        // Unlike HAP there is no block padding, so the source rect needs no
        // rescaling — the decoded texture is exactly the logical frame.
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&Uniforms {
                dst_min,
                dst_max,
                src_min,
                src_max,
            }),
        );
        self.staged = true;
        Ok(())
    }

    /// Record the decode dispatch and the fit pass into `encoder`.
    ///
    /// Takes `&mut self` where [`crate::HapConverter::encode`] takes `&self`,
    /// because the decode picks the next texture in its ring.
    pub fn encode(&mut self, device: &Device, encoder: &mut wgpu::CommandEncoder, canvas_view: &TextureView) {
        if !self.staged {
            return;
        }
        let Some((decoder, _)) = self.decoder.as_mut() else {
            return;
        };
        let texture = decoder.record(encoder);

        let bind = match self.binds.iter().find(|(t, _)| Arc::ptr_eq(t, &texture)) {
            Some((_, bind)) => bind,
            None => {
                let view = texture.create_view(&Default::default());
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("notchlc-fit-bg"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.uniform.as_entire_binding(),
                        },
                    ],
                });
                self.binds.push((Arc::clone(&texture), bind));
                &self.binds.last().expect("just pushed").1
            }
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("notchlc-fit-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: canvas_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

const SHADER: &str = r#"
struct Uniforms {
  dst_min: vec2<f32>,
  dst_max: vec2<f32>,
  src_min: vec2<f32>,
  src_max: vec2<f32>,
};
@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var<uniform> u: Uniforms;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
  let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
  var o: VsOut;
  o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
  o.uv = vec2<f32>(p.x, 1.0 - p.y);
  return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  if (in.uv.x < u.dst_min.x || in.uv.x > u.dst_max.x ||
      in.uv.y < u.dst_min.y || in.uv.y > u.dst_max.y) {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
  }
  let t = (in.uv - u.dst_min) / (u.dst_max - u.dst_min);
  let texel = 0.5 / vec2<f32>(textureDimensions(tex));
  let uv = clamp(
    u.src_min + t * (u.src_max - u.src_min),
    texel,
    vec2<f32>(1.0, 1.0) - texel,
  );
  return textureSampleLevel(tex, samp, uv, 0.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use notchlc_rs::{bit_offsets, decompress_packet, parse_header, FrameEncoder};

    /// 8-bit -> 12-bit the way the codec expands stored endpoints.
    fn to12(v: u8) -> u16 {
        ((v as u16) << 4) | ((v as u16) >> 4)
    }

    /// Encode a solid colour, then run it through the converter into a canvas
    /// and read the canvas back.
    fn render_solid(rgb: [u8; 3], size: u32, canvas_size: [u32; 2]) -> Option<(Vec<u8>, u32)> {
        let _gpu = crate::gpu_test_lock();
        let (device, queue) = crate::test_device_queue(wgpu::Features::empty())?;

        let count = (size * size) as usize;
        // Planes are GBR identity: Y carries G, U carries B, V carries R.
        let packet = FrameEncoder::new(size, size).encode_packet(
            &vec![to12(rgb[1]); count],
            &vec![to12(rgb[2]); count],
            &vec![to12(rgb[0]); count],
        );
        let payload = decompress_packet(&packet).unwrap();
        let header = parse_header(&payload).unwrap();
        let offsets = bit_offsets(&payload, &header).unwrap();
        let frame = VideoFrame::notchlc(size, size, 0.0, payload, header, offsets);

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("notchlc-test-target"),
            size: wgpu::Extent3d {
                width: canvas_size[0],
                height: canvas_size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let mut converter = NotchLcConverter::new(&device, TextureFormat::Rgba8Unorm);
        converter
            .upload(&device, &queue, &frame, canvas_size, CanvasFit::Stretch)
            .unwrap();

        let bytes_per_row =
            (canvas_size[0] * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("notchlc-test-readback"),
            size: u64::from(bytes_per_row * canvas_size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("notchlc-test-encoder"),
        });
        // Decode dispatch and fit pass both land in this one encoder.
        converter.encode(&device, &mut encoder, &target.create_view(&Default::default()));
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(canvas_size[1]),
                },
            },
            wgpu::Extent3d {
                width: canvas_size[0],
                height: canvas_size[1],
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = slice.get_mapped_range().expect("mapped range").to_vec();
        readback.unmap();
        Some((data, bytes_per_row))
    }

    #[test]
    fn decodes_notchlc_into_the_canvas_with_the_right_channels() {
        // Deliberately asymmetric so a swapped plane mapping cannot pass: the
        // codec stores GBR, and getting that backwards is the failure mode.
        let source = [200u8, 40, 20];
        let Some((rendered, stride)) = render_solid(source, 32, [32, 32]) else {
            return; // no usable adapter in this environment
        };
        let offset = (16 * stride + 16 * 4) as usize;
        let pixel = &rendered[offset..offset + 4];
        for (actual, expected) in pixel[..3].iter().zip(source) {
            assert!(
                actual.abs_diff(expected) <= 2,
                "expected ~{source:?}, got {:?}",
                &pixel[..3]
            );
        }
        assert_eq!(pixel[3], 255, "solid frame should be opaque");
    }

    #[test]
    fn rejects_a_frame_that_is_not_notchlc() {
        let Some((device, queue)) = crate::test_device_queue(wgpu::Features::empty()) else {
            return;
        };
        let mut converter = NotchLcConverter::new(&device, TextureFormat::Rgba8Unorm);
        let frame = VideoFrame::new(4, 4, vec![0; 4 * 4 * 4], 0.0);
        let error = converter
            .upload(&device, &queue, &frame, [4, 4], CanvasFit::Stretch)
            .unwrap_err();
        assert!(error.contains("non-NotchLC"), "{error}");
    }
}
