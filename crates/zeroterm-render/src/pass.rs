//! One instanced render pass: the cell-buffer + uniform-buffer + bind-group
//! triple every draw (content, tab bar, status bar, scrollbar) needs, plus the
//! pass draw itself. Before this module each pass hand-rolled the same wgpu
//! resource wiring in BOTH `Renderer::new` and `Renderer::resize` — eight
//! copy-pasted blocks where a sizing change had to be made twice or the pass
//! silently drifted (the tab-bar buffer size and the content viewport famously
//! disagreed). One `Pass` owns the resources and the draw; adding a pass is a
//! single field + one call.

use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

use crate::renderer::{CellData, Uniforms};

pub(crate) struct Pass {
    buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    capacity: usize,
}

impl Pass {
    pub(crate) fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        label: &str,
        capacity: usize,
        initial_uniforms: Uniforms,
    ) -> Self {
        let cells = vec![CellData::zeroed(); capacity];
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label} Cell Buffer")),
            contents: bytemuck::cast_slice(&cells),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label} Uniform Buffer")),
            contents: bytemuck::cast_slice(&[initial_uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("{label} Bind Group")),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(
                        uniform_buffer.as_entire_buffer_binding(),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(buffer.as_entire_buffer_binding()),
                },
            ],
        });
        Self {
            buffer,
            uniform_buffer,
            bind_group,
            capacity,
        }
    }

    /// Recreate the GPU resources at a new capacity (no-op when unchanged).
    /// Every draw writes uniforms before drawing, so the zeroed initial is
    /// never observed.
    pub(crate) fn resize(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        label: &str,
        capacity: usize,
    ) {
        if capacity == self.capacity {
            return;
        }
        *self = Self::new(device, layout, label, capacity, Uniforms::zeroed());
    }

    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    pub(crate) fn write_cells(&self, queue: &wgpu::Queue, cells: &[CellData]) {
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(cells));
    }

    pub(crate) fn write_uniforms(&self, queue: &wgpu::Queue, u: &Uniforms) {
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(std::slice::from_ref(u)),
        );
    }

    /// Draw `instance_count` cell quads through this pass. `clear` is the
    /// initial load op for the color attachment: `Some(color)` clears (the
    /// first pass of a frame), `None` loads the previous contents.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        atlas_bind_group: &wgpu::BindGroup,
        quad_vertex_buffer: &wgpu::Buffer,
        view: &wgpu::TextureView,
        label: &str,
        instance_count: u32,
        clear: Option<wgpu::Color>,
    ) {
        let ops = match clear {
            Some(color) => wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
            None => wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        };
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, &self.bind_group, &[]);
        rp.set_bind_group(1, atlas_bind_group, &[]);
        rp.set_vertex_buffer(0, quad_vertex_buffer.slice(..));
        rp.draw(0..6, 0..instance_count);
    }
}
