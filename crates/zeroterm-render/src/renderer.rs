//! ZeroTerm Renderer - wgpu-based GPU renderer

use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::sync::Arc;
use swash::scale::{Render, ScaleContext, Source};
use swash::FontRef;
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use zeroterm_core::cell::Cell;
use zeroterm_core::screen::Screen as CoreScreen;

const ATLAS_SIZE: u32 = 1024;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Uniforms {
    screen_size: [f32; 2],
    cell_size: [f32; 2],
    cursor_pos: [f32; 2],
    cursor_visible: u32,
    _padding: [u32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coord: [f32; 2],
    color: [f32; 4],
    bg_color: [f32; 4],
    cell_size: [f32; 2],
    attrs: u32,
}

impl Vertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 8]>() as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 12]>() as wgpu::BufferAddress,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 14]>() as wgpu::BufferAddress,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Uint32,
                },
            ],
        }
    }
}

#[derive(Clone, Copy)]
struct GlyphInfo {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[allow(dead_code)]
struct GlyphAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    glyph_cache: HashMap<u32, GlyphInfo>,
    font_data: Vec<u8>,
    scale_context: ScaleContext,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    font_size: f32,
}

impl GlyphAtlas {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, font_size: f32) -> Result<Self> {
        let font_data = Self::load_font()?;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Clear atlas to transparent
        let clear = vec![0u8; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize];
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &clear,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(ATLAS_SIZE * 4),
                rows_per_image: Some(ATLAS_SIZE),
            },
            wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
        );

        let mut atlas = Self {
            texture,
            view,
            sampler,
            glyph_cache: HashMap::new(),
            font_data,
            scale_context: ScaleContext::new(),
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
            font_size,
        };

        // Pre-pack ASCII printable characters
        for ch in 32u8..=126 {
            atlas.get_or_insert_glyph(ch as char, device, queue);
        }

        Ok(atlas)
    }

    fn load_font() -> Result<Vec<u8>> {
        let paths = [
            "/usr/share/fonts/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/GeistMonoVF.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        ];
        for path in &paths {
            if let Ok(data) = std::fs::read(path) {
                log::info!("Loaded font: {}", path);
                return Ok(data);
            }
        }
        anyhow::bail!("No monospace font found in search paths")
    }

    fn get_or_insert_glyph(
        &mut self,
        ch: char,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> (f32, f32, f32, f32) {
        let key = ch as u32;
        if let Some(info) = self.glyph_cache.get(&key) {
            return self.info_to_uv(info);
        }

        let font = match FontRef::from_index(&self.font_data, 0) {
            Some(f) => f,
            None => return self.fallback_uv(),
        };

        let charmap = font.charmap();
        let glyph_id = charmap.map(key);

        let mut scaler = self.scale_context.builder(font).size(self.font_size).build();

        let image = Render::new(&[Source::Outline])
            .format(swash::zeno::Format::Alpha)
            .render(&mut scaler, glyph_id);

        match image {
            Some(img) if img.placement.width > 0 && img.placement.height > 0 => {
                let info = self.pack_glyph(&img, device, queue);
                self.glyph_cache.insert(key, info);
                self.info_to_uv(&info)
            }
            _ => self.fallback_uv(),
        }
    }

    fn pack_glyph(
        &mut self,
        image: &swash::scale::image::Image,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> GlyphInfo {
        let w = image.placement.width;
        let h = image.placement.height;

        // Advance to next row if needed
        if self.cursor_x + w > ATLAS_SIZE {
            self.cursor_x = 0;
            self.cursor_y += self.row_height + 1;
            self.row_height = 0;
        }

        // Check if we've run out of space
        if self.cursor_y + h > ATLAS_SIZE {
            log::warn!("Glyph atlas full, dropping glyph");
            return GlyphInfo {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            };
        }

        let info = GlyphInfo {
            x: self.cursor_x,
            y: self.cursor_y,
            width: w,
            height: h,
        };

        // Convert alpha mask to RGBA (white RGB, alpha from mask)
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for &alpha in &image.data {
            rgba.push(255);
            rgba.push(255);
            rgba.push(255);
            rgba.push(alpha);
        }

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: self.cursor_x,
                    y: self.cursor_y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        self.cursor_x += w + 1;
        self.row_height = self.row_height.max(h);

        info
    }

    fn info_to_uv(&self, info: &GlyphInfo) -> (f32, f32, f32, f32) {
        let u0 = info.x as f32 / ATLAS_SIZE as f32;
        let v0 = info.y as f32 / ATLAS_SIZE as f32;
        let u1 = (info.x + info.width) as f32 / ATLAS_SIZE as f32;
        let v1 = (info.y + info.height) as f32 / ATLAS_SIZE as f32;
        (u0, v0, u1, v1)
    }

    fn fallback_uv(&self) -> (f32, f32, f32, f32) {
        // 1x1 transparent pixel — alpha=0, bg shows through
        (0.0, 0.0, 1.0 / ATLAS_SIZE as f32, 1.0 / ATLAS_SIZE as f32)
    }
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    atlas_bind_group: wgpu::BindGroup,
    glyph_atlas: GlyphAtlas,
    cell_size: [f32; 2],
    prev_buffer: Option<Vec<Vec<Cell>>>,
    vertex_buffer_capacity: usize,
    dirty_cells: Vec<(usize, usize)>,
}

impl Renderer {
    pub async fn new(window: Arc<Window>, font_size: f32) -> Result<Self> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone())?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| anyhow::anyhow!("No suitable adapter found"))?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("ZeroTerm Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Create shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ZeroTerm Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // Uniform buffer
        let cell_width = font_size * 0.6;
        let cell_height = font_size * 1.2;
        let uniforms = Uniforms {
            screen_size: [size.width as f32, size.height as f32],
            cell_size: [cell_width, cell_height],
            cursor_pos: [0.0, 0.0],
            cursor_visible: 0,
            _padding: [0, 0, 0],
        };
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Uniform Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uniform Bind Group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // Create glyph atlas
        let glyph_atlas = GlyphAtlas::new(&device, &queue, font_size)?;

        // Atlas bind group layout
        let atlas_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Atlas Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Atlas Bind Group"),
            layout: &atlas_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&glyph_atlas.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&glyph_atlas.sampler),
                },
            ],
        });

        // Render pipeline
        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&uniform_bind_group_layout, &atlas_bind_group_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Vertex buffer - pre-allocate for full screen (cols * rows * 6 vertices per cell)
        let cols = (size.width as f32 / cell_width).ceil() as usize;
        let rows = (size.height as f32 / cell_height).ceil() as usize;
        let vertex_buffer_capacity = cols * rows * 6;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Vertex Buffer"),
            size: (std::mem::size_of::<Vertex>() * vertex_buffer_capacity) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        log::info!("Renderer initialized: {}x{} (vertex buffer capacity: {} vertices)", size.width, size.height, vertex_buffer_capacity);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            render_pipeline,
            vertex_buffer,
            uniform_buffer,
            uniform_bind_group,
            atlas_bind_group,
            glyph_atlas,
            cell_size: [cell_width, cell_height],
            prev_buffer: None,
            vertex_buffer_capacity,
            dirty_cells: Vec::new(),
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let new_cols = (width as f32 / self.cell_size[0]).ceil() as usize;
        let new_rows = (height as f32 / self.cell_size[1]).ceil() as usize;

        let needs_resize = new_cols * new_rows * 6 != self.vertex_buffer_capacity;

        self.size = PhysicalSize::new(width, height);
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);

        if needs_resize {
            self.vertex_buffer_capacity = new_cols * new_rows * 6;
            self.vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Vertex Buffer"),
                size: (std::mem::size_of::<Vertex>() * self.vertex_buffer_capacity) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            // Reset prev_buffer on resize to force full rebuild
            self.prev_buffer = None;
        }

        let uniforms = Uniforms {
            screen_size: [width as f32, height as f32],
            cell_size: self.cell_size,
            cursor_pos: [0.0, 0.0],
            cursor_visible: 0,
            _padding: [0, 0, 0],
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    pub fn render(&mut self, screen: &CoreScreen, scroll_offset: usize) -> Result<()> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let buffer = screen.buffer();
        let visible_rows = buffer.len();
        let cols = if visible_rows > 0 { buffer[0].len() } else { 0 };

        // Dirty tracking: compare with previous frame
        self.dirty_cells.clear();
        let is_first_frame = self.prev_buffer.is_none();
        
        if is_first_frame {
            // First frame: all cells are dirty
            for row in 0..visible_rows {
                for col in 0..cols {
                    self.dirty_cells.push((row, col));
                }
            }
        } else if let Some(ref prev) = self.prev_buffer {
            // Compare current buffer with previous
            for row in 0..visible_rows.min(prev.len()) {
                let curr_row = &buffer[row];
                let prev_row = &prev[row];
                for col in 0..cols.min(prev_row.len()) {
                    if curr_row[col] != prev_row[col] {
                        self.dirty_cells.push((row, col));
                    }
                }
                // Handle case where cols changed
                if cols > prev_row.len() {
                    for col in prev_row.len()..cols {
                        self.dirty_cells.push((row, col));
                    }
                }
            }
            // Handle case where rows changed
            if visible_rows > prev.len() {
                for row in prev.len()..visible_rows {
                    for col in 0..cols {
                        self.dirty_cells.push((row, col));
                    }
                }
            }
        }

        // Build and upload vertices for dirty cells only
        if !self.dirty_cells.is_empty() {
            self.build_and_upload_dirty_vertices(screen, scroll_offset)?;
        }

        // Update uniforms
        let cursor = screen.cursor();
        let uniforms = Uniforms {
            screen_size: [self.size.width as f32, self.size.height as f32],
            cell_size: self.cell_size,
            cursor_pos: [
                cursor.col as f32 * self.cell_size[0],
                cursor.row as f32 * self.cell_size[1],
            ],
            cursor_visible: if cursor.visible { 1 } else { 0 },
            _padding: [0, 0, 0],
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        // Render
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.117,
                            g: 0.117,
                            b: 0.117,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            render_pass.set_bind_group(1, &self.atlas_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            
            // Draw all vertices (the buffer contains all cells, dirty ones were updated)
            let total_vertices = (visible_rows * cols * 6) as u32;
            render_pass.draw(0..6, 0..total_vertices);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        // Store current buffer for next frame
        self.prev_buffer = Some(buffer.iter().map(|row| row.to_vec()).collect());

        Ok(())
    }

    fn build_and_upload_dirty_vertices(&mut self, screen: &CoreScreen, scroll_offset: usize) -> Result<()> {
        let buffer = screen.buffer();
        let visible_rows = buffer.len();
        let cols = if visible_rows > 0 { buffer[0].len() } else { 0 };
        let cell_w = self.cell_size[0];
        let cell_h = self.cell_size[1];

        // Get cursor info
        let cursor = screen.cursor();
        let cursor_col = cursor.col;
        let cursor_row_global = cursor.row;
        let cursor_visible = cursor.visible;
        let cursor_shape = cursor.shape;

        // Build combined rows: scrollback (oldest first) + buffer
        let scrollback = screen.scrollback();
        let total_scrollback = scrollback.len();
        let total_rows = total_scrollback + visible_rows;

        // Visible window: [start..end) in the combined row space
        let end = total_rows.saturating_sub(scroll_offset);
        let start = end.saturating_sub(visible_rows);

        // Check if cursor is in visible range
        let cursor_in_range = cursor_row_global < total_rows && cursor_row_global >= start;

        // Build vertices for dirty cells only
        let mut dirty_vertices = Vec::new();
        let mut dirty_offsets = Vec::new(); // (offset_in_buffer, start_index_in_dirty_vertices, count)

        for &(dirty_row, dirty_col) in &self.dirty_cells {
            if dirty_row >= visible_rows || dirty_col >= cols {
                continue;
            }

            let combined_idx = start + dirty_row;
            let line = if combined_idx < total_scrollback {
                // Scrollback lines are stored newest-first, reverse for chronological
                &scrollback[total_scrollback - 1 - combined_idx]
            } else {
                &buffer[combined_idx - total_scrollback]
            };

            if dirty_col >= line.len() {
                continue;
            }

            let cell = &line[dirty_col];

            let x = dirty_col as f32 * cell_w;
            let y = dirty_row as f32 * cell_h;

            let fg = cell.fg;
            let bg = cell.bg;

            // Check if this is the cursor row
            let is_cursor_row = cursor_in_range && combined_idx == cursor_row_global;
            let is_cursor_cell = is_cursor_row && dirty_col == cursor_col;
            let (fg, bg, attrs) = if cursor_visible && is_cursor_cell {
                match cursor_shape {
                    zeroterm_core::cell::CursorShape::Block => (bg, fg, cell.attrs),
                    zeroterm_core::cell::CursorShape::Underline => {
                        let mut a = cell.attrs;
                        a.underline = zeroterm_core::cell::UnderlineStyle::Single;
                        (fg, bg, a)
                    }
                    zeroterm_core::cell::CursorShape::Bar => (fg, bg, cell.attrs),
                }
            } else {
                (fg, bg, cell.attrs)
            };

            let attrs = ((attrs.bold as u32) << 0)
                | ((attrs.italic as u32) << 1)
                | ((attrs.underline as u32) << 2)
                | ((attrs.strikethrough as u32) << 3)
                | ((attrs.dim as u32) << 4)
                | ((attrs.blink as u32) << 5)
                | ((attrs.reverse as u32) << 6)
                | ((attrs.invisible as u32) << 7);

            let fg_color = [
                fg.r as f32 / 255.0,
                fg.g as f32 / 255.0,
                fg.b as f32 / 255.0,
                1.0,
            ];
            let bg_color = [
                bg.r as f32 / 255.0,
                bg.g as f32 / 255.0,
                bg.b as f32 / 255.0,
                1.0,
            ];

            let (u0, v0, u1, v1) = self
                .glyph_atlas
                .get_or_insert_glyph(cell.ch, &self.device, &self.queue);

            let base_offset = (dirty_row * cols + dirty_col) * 6;
            let vertex_start = dirty_vertices.len();
            dirty_offsets.push((base_offset, vertex_start, 6));

            // Two triangles per cell (6 vertices)
            dirty_vertices.push(Vertex {
                position: [x, y],
                tex_coord: [u0, v0],
                color: fg_color,
                bg_color,
                cell_size: [0.0, 0.0],
                attrs,
            });
            dirty_vertices.push(Vertex {
                position: [x + cell_w, y],
                tex_coord: [u1, v0],
                color: fg_color,
                bg_color,
                cell_size: [1.0, 0.0],
                attrs,
            });
            dirty_vertices.push(Vertex {
                position: [x, y + cell_h],
                tex_coord: [u0, v1],
                color: fg_color,
                bg_color,
                cell_size: [0.0, 1.0],
                attrs,
            });
            dirty_vertices.push(Vertex {
                position: [x + cell_w, y],
                tex_coord: [u1, v0],
                color: fg_color,
                bg_color,
                cell_size: [1.0, 0.0],
                attrs,
            });
            dirty_vertices.push(Vertex {
                position: [x + cell_w, y + cell_h],
                tex_coord: [u1, v1],
                color: fg_color,
                bg_color,
                cell_size: [1.0, 1.0],
                attrs,
            });
            dirty_vertices.push(Vertex {
                position: [x, y + cell_h],
                tex_coord: [u0, v1],
                color: fg_color,
                bg_color,
                cell_size: [0.0, 1.0],
                attrs,
            });
        }

        // Upload dirty vertices to GPU buffer at their respective offsets
        for (offset, vertex_start, count) in dirty_offsets {
            let byte_offset = (offset * std::mem::size_of::<Vertex>()) as wgpu::BufferAddress;
            let vertex_slice = &dirty_vertices[vertex_start..vertex_start + count];
            self.queue.write_buffer(&self.vertex_buffer, byte_offset, bytemuck::cast_slice(vertex_slice));
        }

        Ok(())
    }
}
