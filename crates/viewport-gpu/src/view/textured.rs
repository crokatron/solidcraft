//! Cached authored scan textures. CPU preparation belongs to the host worker; orbit only
//! updates the ordinary camera uniform, never decodes images or uploads triangle buffers.
use super::{Arc, Batch, GpuTarget, Tally};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// Position, UV, and straight sRGB RGBA8. UV seams remain independent triangle corners.
pub const SCAN_VERTEX_SIZE: usize = 24;
#[derive(Clone)]
pub struct ScanImage {
    pub size: [u32; 2],
    pub rgba: Arc<Vec<u8>>,
}
pub struct ScanMesh {
    pub vertices: Vec<u8>,
    pub image: Arc<ScanImage>,
}
#[derive(Clone)]
pub struct ScanPlacement {
    pub mesh: Arc<ScanMesh>,
    pub rows: [[f32; 4]; 3],
}
struct Resident {
    source: Arc<ScanMesh>,
    vertices: Batch,
    bind: wgpu::BindGroup,
}
struct Draw {
    mesh: Arc<Resident>,
    rows: [[f32; 4]; 3],
    instance: wgpu::Buffer,
}
#[derive(Default)]
pub(super) struct Batches {
    pub sources: Vec<ScanPlacement>,
    resident: HashMap<usize, Arc<Resident>>,
    draws: Vec<Draw>,
    images: HashMap<usize, (Arc<ScanImage>, wgpu::BindGroup)>,
}
impl Batches {
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler) {
        let mut old = std::mem::take(&mut self.draws).into_iter();
        for source in &self.sources {
            let key = Arc::as_ptr(&source.mesh) as usize;
            if let std::collections::hash_map::Entry::Vacant(entry) = self.resident.entry(key) {
                let image = &source.mesh.image;
                let [w, h] = image.size;
                let limit = device.limits().max_texture_dimension_2d;
                if w == 0 || h == 0 || w > limit || h > limit || image.rgba.len() as u64 != u64::from(w) * u64::from(h) * 4 {
                    continue;
                }
                let Some(vertices) = super::upload(device, "scan triangles", &source.mesh.vertices, SCAN_VERTEX_SIZE) else { continue };
                let image_key = Arc::as_ptr(image) as usize;
                let bind = if let Some((_, bind)) = self.images.get(&image_key) {
                    bind.clone()
                } else {
                    let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("scan photograph"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        &image.rgba,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
                        size,
                    );
                    let view = texture.create_view(&Default::default());
                    let made = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("scan photograph"),
                        layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                        ],
                    });
                    self.images.insert(image_key, (image.clone(), made.clone()));
                    made
                };
                entry.insert(Arc::new(Resident { source: source.mesh.clone(), vertices, bind }));
            }
            let Some(mesh) = self.resident.get(&key).cloned() else { continue };
            if let Some(draw) = old.next().filter(|d| Arc::ptr_eq(&d.mesh, &mesh) && d.rows == source.rows) {
                self.draws.push(draw);
            } else {
                let bytes: Vec<u8> = source.rows.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
                let instance = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("scan placement"),
                    contents: &bytes,
                    usage: wgpu::BufferUsages::VERTEX,
                });
                self.draws.push(Draw { mesh, instance, rows: source.rows });
            }
        }
        self.resident.retain(|_, mesh| self.sources.iter().any(|s| Arc::ptr_eq(&s.mesh, &mesh.source)));
        // Retain a bounded reserve across the brief empty scene while an edit is prepared.
        // Active photographs are never evicted; their geometry still uses the bind group.
        let mut bytes = self.images.values().map(|(image, _)| image.rgba.len()).sum::<usize>();
        let idle: Vec<_> = self
            .images
            .iter()
            .filter(|(_, (image, _))| !self.sources.iter().any(|s| Arc::ptr_eq(&s.mesh.image, image)))
            .map(|(key, _)| *key)
            .collect();
        for key in idle {
            if bytes <= 512 * 1024 * 1024 && self.images.len() <= 8 {
                break;
            }
            if let Some((image, _)) = self.images.remove(&key) {
                bytes = bytes.saturating_sub(image.rgba.len());
            }
        }
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'static>, pipeline: &wgpu::RenderPipeline, tally: &Tally) {
        for draw in &self.draws {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &draw.mesh.bind, &[]);
            pass.set_vertex_buffer(1, draw.instance.slice(..));
            for (buffer, count) in &draw.mesh.vertices.parts {
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..*count, 0..1);
                tally.triangles(*count, 1);
            }
        }
    }
}
pub(super) fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    target: GpuTarget,
    depth_stencil: Option<wgpu::DepthStencilState>,
) -> wgpu::RenderPipeline {
    const V: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Unorm8x4];
    const I: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("textured scans"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_scan"),
            buffers: &[
                Some(wgpu::VertexBufferLayout { array_stride: SCAN_VERTEX_SIZE as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &V }),
                Some(wgpu::VertexBufferLayout { array_stride: 48, step_mode: wgpu::VertexStepMode::Instance, attributes: &I }),
            ],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil,
        multisample: wgpu::MultisampleState { count: target.samples.max(1), mask: !0, alpha_to_coverage_enabled: false },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_scan"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target.format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}
