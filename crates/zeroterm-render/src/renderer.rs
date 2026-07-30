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

use zeroterm_core::screen::Screen as CoreScreen;

#[derive(Debug, Clone, Copy, Default)]
pub struct Selection {
    pub start_row: usize,
    pub start_col: usize,
    pub end_row: usize,
    pub end_col: usize,
    pub active: bool,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn contains(&self, row: usize, col: usize) -> bool {
        if !self.active {
            return false;
        }
        let (sr, sc, er, ec) = if self.start_row < self.end_row
            || (self.start_row == self.end_row && self.start_col <= self.end_col)
        {
            (self.start_row, self.start_col, self.end_row, self.end_col)
        } else {
            (self.end_row, self.end_col, self.start_row, self.start_col)
        };
        (row > sr || (row == sr && col >= sc)) && (row < er || (row == er && col <= ec))
    }
}

const ATLAS_SIZE: u32 = 1024;

const ATTR_HAS_IMAGE: u32 = 0x400;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Uniforms {
    screen_size: [f32; 2],
    cell_size: [f32; 2],
    cursor_pos: [f32; 2],
    cursor_visible: u32,
    cols: u32,
    rows: u32,
    _padding: [u32; 1],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct CellData {
    glyph_uv_min: [f32; 2],
    glyph_uv_max: [f32; 2],
    glyph_size: [f32; 2],
    _pad0: [f32; 2],
    fg: [f32; 4],
    bg: [f32; 4],
    attrs: u32,
    _pad1: [u32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct QuadVertex {
    position: [f32; 2],
}

const QUAD_VERTS: [QuadVertex; 6] = [
    QuadVertex { position: [0.0, 0.0] },
    QuadVertex { position: [1.0, 0.0] },
    QuadVertex { position: [0.0, 1.0] },
    QuadVertex { position: [1.0, 0.0] },
    QuadVertex { position: [1.0, 1.0] },
    QuadVertex { position: [0.0, 1.0] },
];

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
    cell_width: f32,
    cell_height: f32,
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
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
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
            cell_width: font_size * 0.5,
            cell_height: font_size * 1.2,
        };

        // Compute cell metrics from font
        if let Some(font) = FontRef::from_index(&atlas.font_data, 0) {
            let metrics = font.metrics(&[]);
            let scale = atlas.font_size / metrics.units_per_em as f32;
            let ascent = metrics.ascent * scale;
            let descent = metrics.descent * scale;
            let leading = metrics.leading * scale;
            atlas.cell_height = (ascent + descent + leading).ceil();
            let mut scaler = atlas.scale_context.builder(font).size(atlas.font_size).build();
            let charmap = font.charmap();
            if let Some(img) = Render::new(&[Source::Outline])
                .format(swash::zeno::Format::Alpha)
                .render(&mut scaler, charmap.map(0x57u32))
            {
                atlas.cell_width = img.placement.width as f32;
            }
        }

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
    ) -> (f32, f32, f32, f32, f32, f32) {
        let key = ch as u32;
        if let Some(info) = self.glyph_cache.get(&key) {
            let uv = self.info_to_uv(info);
            return (uv.0, uv.1, uv.2, uv.3, info.width as f32, info.height as f32);
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
                let uv = self.info_to_uv(&info);
                (uv.0, uv.1, uv.2, uv.3, info.width as f32, info.height as f32)
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

    fn cell_metrics(&self) -> (f32, f32) {
        (self.cell_width, self.cell_height)
    }

    fn fallback_uv(&self) -> (f32, f32, f32, f32, f32, f32) {
        // 1x1 transparent pixel — alpha=0, bg shows through
        (0.0, 0.0, 1.0 / ATLAS_SIZE as f32, 1.0 / ATLAS_SIZE as f32, 0.0, 0.0)
    }
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    render_pipeline: wgpu::RenderPipeline,
    cell_buffer: wgpu::Buffer,
    quad_vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    atlas_bind_group: wgpu::BindGroup,
    glyph_atlas: GlyphAtlas,
    cell_size: [f32; 2],
    cell_buffer_capacity: usize,
    dirty_cells: Vec<(usize, usize)>,
    clear_color: [f64; 3],
    opacity: f64,
    atlas_bind_group_layout: wgpu::BindGroupLayout,
    image_texture: Option<wgpu::Texture>,
    image_view: Option<wgpu::TextureView>,
    has_image: bool,
}

impl Renderer {
    pub async fn new(window: Arc<Window>, font_size: f32, opacity: f64) -> Result<Self> {
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

        let alpha_mode = if surface_caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
            wgpu::CompositeAlphaMode::PostMultiplied
        } else {
            surface_caps.alpha_modes[0]
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Create shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ZeroTerm Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // Create glyph atlas first to get actual cell metrics from font
        let glyph_atlas = GlyphAtlas::new(&device, &queue, font_size)?;
        let (cell_width, cell_height) = glyph_atlas.cell_metrics();

        // Uniform buffer
        let cols = (size.width as f32 / cell_width).ceil() as u32;
        let rows = (size.height as f32 / cell_height).ceil() as u32;
        let uniforms = Uniforms {
            screen_size: [size.width as f32, size.height as f32],
            cell_size: [cell_width, cell_height],
            cursor_pos: [0.0, 0.0],
            cursor_visible: 0,
            cols,
            rows,
            _padding: [0],
        };
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let cell_buffer_capacity = (cols as usize) * (rows as usize);
        let cell_buffer = {
            let cell_data = vec![CellData::zeroed(); cell_buffer_capacity];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Cell Data Buffer"),
                contents: bytemuck::cast_slice(&cell_data),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
        };

        let quad_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Quad Vertex Buffer"),
            contents: bytemuck::cast_slice(&QUAD_VERTS),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Uniform + Storage Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uniform + Storage Bind Group"),
            layout: &uniform_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(uniform_buffer.as_entire_buffer_binding()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(cell_buffer.as_entire_buffer_binding()),
                },
            ],
        });

        // Atlas bind group layout with image texture binding
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });

        // Placeholder 1x1 texture for image_texture binding
        let placeholder_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Placeholder Image Texture"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let placeholder_view = placeholder_tex.create_view(&wgpu::TextureViewDescriptor::default());
        queue.write_texture(
            wgpu::ImageCopyTexture { texture: &placeholder_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &[0u8; 4],
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );

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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&placeholder_view),
                },
            ],
        });

        let vertex_attributes = [
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
        ];
        let vertex_buffer_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<QuadVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &vertex_attributes,
        };

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
                buffers: &[vertex_buffer_layout],
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

        log::info!("Renderer initialized: {}x{} (cell buffer capacity: {} cells)", size.width, size.height, cell_buffer_capacity);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            render_pipeline,
            cell_buffer,
            quad_vertex_buffer,
            uniform_buffer,
            uniform_bind_group,
            atlas_bind_group_layout,
            atlas_bind_group,
            glyph_atlas,
            cell_size: [cell_width, cell_height],
            cell_buffer_capacity,
            dirty_cells: Vec::new(),
            clear_color: [0.117, 0.117, 0.117],
            opacity,
            image_texture: Some(placeholder_tex),
            image_view: Some(placeholder_view),
            has_image: false,
        })
    }

    pub fn cell_size(&self) -> [f32; 2] {
        self.cell_size
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let new_cols = (width as f32 / self.cell_size[0]).ceil() as usize;
        let new_rows = (height as f32 / self.cell_size[1]).ceil() as usize;

        let needs_resize = new_cols * new_rows != self.cell_buffer_capacity;

        self.size = PhysicalSize::new(width, height);
        self.config.width = width;
        self.config.height = height;
        self.config.alpha_mode = wgpu::CompositeAlphaMode::PostMultiplied;
        self.surface.configure(&self.device, &self.config);

        if needs_resize {
            self.cell_buffer_capacity = new_cols * new_rows;
            self.cell_buffer = {
                let cell_data = vec![CellData::zeroed(); self.cell_buffer_capacity];
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Cell Data Buffer"),
                    contents: bytemuck::cast_slice(&cell_data),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                })
            };
            self.uniform_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Uniform + Storage Bind Group"),
                layout: &self.render_pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(self.uniform_buffer.as_entire_buffer_binding()),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(self.cell_buffer.as_entire_buffer_binding()),
                    },
                ],
            });
        }

        let uniforms = Uniforms {
            screen_size: [width as f32, height as f32],
            cell_size: self.cell_size,
            cursor_pos: [0.0, 0.0],
            cursor_visible: 0,
            cols: new_cols as u32,
            rows: new_rows as u32,
            _padding: [0],
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    pub fn render(&mut self, screen: &CoreScreen, scroll_offset: usize, selection: Option<Selection>) -> Result<()> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let buffer = screen.buffer();
        let visible_rows = buffer.len();
        let cols = if visible_rows > 0 { buffer[0].len() } else { 0 };

        // All cells dirty every frame — GPU handles <200KB writes trivially
        self.dirty_cells.clear();
        for row in 0..visible_rows {
            for col in 0..cols {
                self.dirty_cells.push((row, col));
            }
        }

        self.update_image_from_screen(screen);

        // Build and upload vertices for dirty cells only
        if !self.dirty_cells.is_empty() {
            self.update_cell_data(screen, scroll_offset, selection)?;
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
            cols: cols as u32,
            rows: visible_rows as u32,
            _padding: [0],
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
                            r: self.clear_color[0],
                            g: self.clear_color[1],
                            b: self.clear_color[2],
                            a: self.opacity,
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
            render_pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));

            let instance_count = (visible_rows * cols) as u32;
            render_pass.draw(0..6, 0..instance_count);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    fn update_cell_data(&mut self, screen: &CoreScreen, scroll_offset: usize, selection: Option<Selection>) -> Result<()> {
        let buffer = screen.buffer();
        let visible_rows = buffer.len();
        let cols = if visible_rows > 0 { buffer[0].len() } else { 0 };
        let cursor = screen.cursor();
        let cursor_col = cursor.col;
        let cursor_visible = cursor.visible;
        let cursor_shape = cursor.shape;

        let scrollback = screen.scrollback();
        let total_scrollback = scrollback.len();
        let total_rows = total_scrollback + visible_rows;

        let end = total_rows.saturating_sub(scroll_offset);
        let start = end.saturating_sub(visible_rows);

        let mut batch = vec![CellData::zeroed(); visible_rows * cols];
        for &(dirty_row, dirty_col) in &self.dirty_cells {
            if dirty_row >= visible_rows || dirty_col >= cols {
                continue;
            }

            let combined_idx = start + dirty_row;
            let line = if combined_idx < total_scrollback {
                &scrollback[total_scrollback - 1 - combined_idx]
            } else {
                &buffer[combined_idx - total_scrollback]
            };

            if dirty_col >= line.len() {
                continue;
            }

            let cell = &line[dirty_col];

            let fg = cell.fg;
            let bg = cell.bg;

            let is_cursor_cell = cursor_visible && scroll_offset == 0 && dirty_row == cursor.row && dirty_col == cursor_col;
            let (fg, bg, cell_attrs) = if is_cursor_cell {
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

            let is_selected = selection.is_some_and(|s| s.contains(combined_idx, dirty_col));

            let mut attrs = ((cell_attrs.bold as u32) << 0)
                | ((cell_attrs.italic as u32) << 1)
                | (((cell_attrs.underline != zeroterm_core::cell::UnderlineStyle::None) as u32) << 2)
                | ((cell_attrs.strikethrough as u32) << 3)
                | ((cell_attrs.dim as u32) << 4)
                | ((cell_attrs.blink as u32) << 5)
                | ((cell_attrs.reverse as u32) << 6)
                | ((cell_attrs.invisible as u32) << 7)
                | (if is_cursor_cell && matches!(cursor_shape, zeroterm_core::cell::CursorShape::Bar) { 0x100u32 } else { 0 })
                | (if is_selected { 0x200u32 } else { 0 });
            if screen.image_cells().contains_key(&(combined_idx, dirty_col)) {
                attrs |= ATTR_HAS_IMAGE;
            }

            let fg_color = [fg.r as f32 / 255.0, fg.g as f32 / 255.0, fg.b as f32 / 255.0, 1.0];
            let bg_color = [bg.r as f32 / 255.0, bg.g as f32 / 255.0, bg.b as f32 / 255.0, 1.0];

            let (u0, v0, u1, v1, gw, gh) = self
                .glyph_atlas
                .get_or_insert_glyph(cell.ch, &self.device, &self.queue);

            batch[dirty_row * cols + dirty_col] = CellData {
                glyph_uv_min: [u0, v0],
                glyph_uv_max: [u1, v1],
                glyph_size: [gw, gh],
                _pad0: [0.0, 0.0],
                fg: fg_color,
                bg: bg_color,
                attrs,
                _pad1: [0; 3],
            };
        }

        self.queue.write_buffer(&self.cell_buffer, 0, bytemuck::cast_slice(&batch));

        Ok(())
    }

    pub fn reload_config(&mut self, config: &zeroterm_config::Config) {
        if let Some((r, g, b)) = Self::parse_hex_color(&config.colors.background) {
            self.clear_color = [r, g, b];
        }
        self.opacity = config.window.opacity;
    }

    pub fn set_opacity(&mut self, opacity: f64) {
        self.opacity = opacity;
    }

    // ponytail: uploads only the latest image; multi-image needs texture array
    fn update_image_from_screen(&mut self, screen: &CoreScreen) {
        let reg = screen.image_registry();
        if reg.is_empty() || self.has_image {
            return;
        }
        let img = match reg.values().last() {
            Some(i) => i,
            None => return,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Kitty Image Texture"),
            size: wgpu::Extent3d { width: img.width, height: img.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.queue.write_texture(
            wgpu::ImageCopyTexture { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &img.rgba_data,
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(img.width * 4), rows_per_image: Some(img.height) },
            wgpu::Extent3d { width: img.width, height: img.height, depth_or_array_layers: 1 },
        );
        let new_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Atlas Bind Group"),
            layout: &self.atlas_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.glyph_atlas.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.glyph_atlas.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        self.image_texture = Some(texture);
        self.image_view = Some(view);
        self.atlas_bind_group = new_bind_group;
        self.has_image = true;
    }

    fn parse_hex_color(hex: &str) -> Option<(f64, f64, f64)> {
        let hex = hex.trim_start_matches('#');
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f64 / 255.0;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f64 / 255.0;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f64 / 255.0;
            Some((r, g, b))
        } else {
            None
        }
    }
}
