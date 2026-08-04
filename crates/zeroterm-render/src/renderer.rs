//! ZeroTerm Renderer - wgpu-based GPU renderer

use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::sync::Arc;
use swash::scale::{Render, ScaleContext, Source};
use swash::FontRef;
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use zeroterm_core::cell::Color;
use zeroterm_core::screen::Screen as CoreScreen;

pub type Result<T> = std::result::Result<T, RendererError>;

#[derive(Debug, thiserror::Error)]
pub enum RendererError {
    #[error("wgpu surface creation failed: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("wgpu device request failed: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("no suitable GPU adapter found; ZeroTerm requires Vulkan, Metal, DX11/12, or OpenGL")]
    NoAdapter,
    #[error("font not found: {0}")]
    FontNotFound(String),
}

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

#[derive(Debug, Clone)]
pub struct TabInfo {
    pub title: String,
    pub active: bool,
    pub hovered: bool,
    pub close_hovered: bool,
}

const ATLAS_SIZE: u32 = 1024;

const ATTR_HAS_IMAGE: u32 = 0x400;
const ATTR_BLOCK_DIVIDER: u32 = 0x800;
const ATTR_DIM: u32 = 0x10;

const COPY_MARKER: &str = "[copy]";

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Uniforms {
    screen_size: [f32; 2],
    cell_size: [f32; 2],
    viewport_origin: [f32; 2],
    cols: u32,
    rows: u32,
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
    QuadVertex {
        position: [0.0, 0.0],
    },
    QuadVertex {
        position: [1.0, 0.0],
    },
    QuadVertex {
        position: [0.0, 1.0],
    },
    QuadVertex {
        position: [1.0, 0.0],
    },
    QuadVertex {
        position: [1.0, 1.0],
    },
    QuadVertex {
        position: [0.0, 1.0],
    },
];

#[derive(Clone, Copy)]
struct GlyphInfo {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

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
    font_path: Option<String>,
}

impl GlyphAtlas {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_size: f32,
        font_path: Option<String>,
    ) -> Result<Self> {
        let font_data = Self::load_font(font_path.clone())?;

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
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // ponytail: no full-atlas clear. Uninitialized texels are only sampled
        // at 0.5-texel Linear bleed into *written* glyph neighbors, never far
        // from a glyph, so skipping the 4MB write_texture saves ~30ms of boot.
        let mut atlas = Self {
            texture,
            view,
            sampler,
            glyph_cache: HashMap::new(),
            font_data,
            font_path,
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
            let mut scaler = atlas
                .scale_context
                .builder(font)
                .size(atlas.font_size)
                .build();
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

    fn load_font(font_path: Option<String>) -> Result<Vec<u8>> {
        if let Some(path) = font_path {
            return std::fs::read(&path).map_err(|_| RendererError::FontNotFound(path));
        }
        let paths = [
            "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf",
            "/usr/share/fonts/truetype/JetBrainsMono/JetBrainsMonoNerdFont-Regular.ttf",
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
        log::info!("No system font found, using embedded DejaVu Sans Mono fallback");
        Ok(include_bytes!("../DejaVuSansMono.ttf").to_vec())
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
            return (
                uv.0,
                uv.1,
                uv.2,
                uv.3,
                info.width as f32,
                info.height as f32,
            );
        }

        let font = match FontRef::from_index(&self.font_data, 0) {
            Some(f) => f,
            None => return self.fallback_uv(),
        };

        let charmap = font.charmap();
        let glyph_id = charmap.map(key);

        let mut scaler = self
            .scale_context
            .builder(font)
            .size(self.font_size)
            .build();

        let image = Render::new(&[Source::Outline])
            .format(swash::zeno::Format::Alpha)
            .render(&mut scaler, glyph_id);

        match image {
            Some(img) if img.placement.width > 0 && img.placement.height > 0 => {
                let info = self.pack_glyph(&img, device, queue);
                self.glyph_cache.insert(key, info);
                let uv = self.info_to_uv(&info);
                (
                    uv.0,
                    uv.1,
                    uv.2,
                    uv.3,
                    info.width as f32,
                    info.height as f32,
                )
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
            log::warn!("Glyph atlas full, clearing");
            self.clear_atlas(queue);
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
        (
            0.0,
            0.0,
            1.0 / ATLAS_SIZE as f32,
            1.0 / ATLAS_SIZE as f32,
            0.0,
            0.0,
        )
    }

    fn clear_atlas(&mut self, queue: &wgpu::Queue) {
        let clear = vec![0u8; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize];
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
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
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.row_height = 0;
        self.glyph_cache.clear();
    }

    fn repack_ascii(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.clear_atlas(queue);
        for ch in 32u8..=126 {
            self.get_or_insert_glyph(ch as char, device, queue);
        }
    }
}

/// Generous max rows for the scrollbar overlay storage buffer (2 col cells each).
const SCROLLBAR_MAX_ROWS: usize = 512;

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
    tab_bar_buffer: wgpu::Buffer,
    tab_bar_uniform_buffer: wgpu::Buffer,
    tab_bar_bind_group: wgpu::BindGroup,
    status_bar_buffer: wgpu::Buffer,
    status_bar_uniform_buffer: wgpu::Buffer,
    status_bar_bind_group: wgpu::BindGroup,
    scrollbar_buffer: wgpu::Buffer,
    scrollbar_uniform_buffer: wgpu::Buffer,
    scrollbar_bind_group: wgpu::BindGroup,
    atlas_bind_group: wgpu::BindGroup,
    glyph_atlas: GlyphAtlas,
    cell_size: [f32; 2],
    cell_width: f32,
    cell_height: f32,
    cell_buffer_capacity: usize,
    dirty_cells: Vec<(usize, usize)>,
    viewport_origin: [f32; 2],
    padding: [f32; 4],
    current_frame: Option<wgpu::SurfaceTexture>,
    current_view: Option<wgpu::TextureView>,
    current_encoder: Option<wgpu::CommandEncoder>,
    needs_clear: bool,
    clear_color: [f64; 3],
    opacity: f64,
    theme: crate::theme::Theme,
    atlas_bind_group_layout: wgpu::BindGroupLayout,
    image_texture: Option<wgpu::Texture>,
    image_view: Option<wgpu::TextureView>,
    uploaded_image_id: Option<u32>,
    image_frames: Vec<(Vec<u8>, u32, u32, u64)>,
    anim_frame_index: usize,
    anim_last_swap: std::time::Instant,
    is_animated: bool,
    cursor_blink: bool,
    blink_visible: bool,
    blink_last_toggle: std::time::Instant,
    blink_interval: std::time::Duration,
    cursor_blink_enabled: bool,
}

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        font_size: f32,
        opacity: f64,
        font_path: Option<String>,
    ) -> Result<Self> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            // Native backend only: enumerating every backend (GL/D3D/Metal)
            // on startup costs ~170ms of adapter probing.
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone())?;

        let adapter = {
            let mut options = wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            };
            let mut found = instance.request_adapter(&options).await;
            if found.is_none() {
                log::warn!("No high-performance GPU found, trying low-power (iGPU)");
                options.power_preference = wgpu::PowerPreference::LowPower;
                found = instance.request_adapter(&options).await;
            }
            if found.is_none() {
                log::warn!(
                    "No hardware GPU found, trying software fallback (lavapipe/WARP/SwiftShader)"
                );
                options.force_fallback_adapter = true;
                found = instance.request_adapter(&options).await;
            }
            found
        }
        .ok_or(RendererError::NoAdapter)?;
        let supports_pipeline_cache = adapter.features().contains(wgpu::Features::PIPELINE_CACHE);

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("ZeroTerm Device"),
                    required_features: if supports_pipeline_cache {
                        wgpu::Features::PIPELINE_CACHE
                    } else {
                        wgpu::Features::empty()
                    },
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await?;

        // Pipeline cache (wgpu 23 API): reuse compiled shader state across launches.
        // Vulkan-only in this wgpu version; on other backends this degrades to no caching.
        let pipeline_cache = if supports_pipeline_cache {
            let cache_file = wgpu::util::pipeline_cache_key(&adapter.get_info())
                .and_then(|key| dirs::cache_dir().map(|dir| dir.join("zeroterm").join(key)));
            let cache_data = cache_file.as_ref().and_then(|p| std::fs::read(p).ok());
            let cache = unsafe {
                // SAFETY: cache_data (if any) was produced by PipelineCache::get_data in a
                // prior run; fallback:true makes invalid/outdated data degrade to an empty cache.
                device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("ZeroTerm Pipeline Cache"),
                    data: cache_data.as_deref(),
                    fallback: true,
                })
            };
            Some((cache, cache_file))
        } else {
            None
        };

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let alpha_mode = if surface_caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::Auto)
        {
            wgpu::CompositeAlphaMode::Auto
        } else if surface_caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::Opaque)
        {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            surface_caps.alpha_modes[0]
        };

        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Fifo)
        {
            wgpu::PresentMode::Fifo
        } else {
            surface_caps.present_modes[0]
        };
        log::info!(
            "Using present mode: {:?}, alpha mode: {:?}, surface format: {:?}",
            present_mode,
            alpha_mode,
            surface_format
        );

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode,
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
        let glyph_atlas = GlyphAtlas::new(&device, &queue, font_size, font_path)?;
        let (cell_width, cell_height) = glyph_atlas.cell_metrics();

        // Uniform buffer
        let cols = (size.width as f32 / cell_width).ceil() as u32;
        let rows = (size.height as f32 / cell_height).ceil() as u32;
        let uniforms = Uniforms {
            screen_size: [size.width as f32, size.height as f32],
            cell_size: [cell_width, cell_height],
            viewport_origin: [0.0, 0.0],
            cols,
            rows,
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
                    resource: wgpu::BindingResource::Buffer(
                        uniform_buffer.as_entire_buffer_binding(),
                    ),
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
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let placeholder_view = placeholder_tex.create_view(&wgpu::TextureViewDescriptor::default());
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &placeholder_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[0u8; 4],
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
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

        let vertex_attributes = [wgpu::VertexAttribute {
            offset: 0,
            shader_location: 0,
            format: wgpu::VertexFormat::Float32x2,
        }];
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
            cache: pipeline_cache.as_ref().map(|(cache, _)| cache),
        });

        // Persist cache results for the next launch (atomic write, per wgpu docs).
        if let Some((cache, Some(file))) = pipeline_cache.as_ref() {
            if let Some(data) = cache.get_data() {
                if let Some(parent) = file.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let tmp = file.with_extension("tmp");
                if std::fs::write(&tmp, &data).is_ok() {
                    let _ = std::fs::rename(&tmp, file);
                }
            }
        }

        log::info!(
            "Renderer initialized: {}x{} (cell buffer capacity: {} cells)",
            size.width,
            size.height,
            cell_buffer_capacity
        );

        let tab_bar_buffer = {
            let cells = vec![CellData::zeroed(); cols as usize];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Tab Bar Cell Buffer"),
                contents: bytemuck::cast_slice(&cells),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
        };
        let tab_bar_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Tab Bar Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let tab_bar_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Tab Bar Bind Group"),
            layout: &render_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(
                        tab_bar_uniform_buffer.as_entire_buffer_binding(),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(
                        tab_bar_buffer.as_entire_buffer_binding(),
                    ),
                },
            ],
        });

        let status_bar_buffer = {
            let cells = vec![CellData::zeroed(); 512];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Status Bar Cell Buffer"),
                contents: bytemuck::cast_slice(&cells),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
        };
        let status_bar_uniform_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Status Bar Uniform Buffer"),
                contents: bytemuck::cast_slice(&[uniforms]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let status_bar_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Status Bar Bind Group"),
            layout: &render_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(
                        status_bar_uniform_buffer.as_entire_buffer_binding(),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(
                        status_bar_buffer.as_entire_buffer_binding(),
                    ),
                },
            ],
        });

        let scrollbar_buffer = {
            let cells = vec![CellData::zeroed(); SCROLLBAR_MAX_ROWS * 2];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Scrollbar Cell Buffer"),
                contents: bytemuck::cast_slice(&cells),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
        };
        let scrollbar_uniform_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Scrollbar Uniform Buffer"),
                contents: bytemuck::cast_slice(&[uniforms]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let scrollbar_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scrollbar Bind Group"),
            layout: &render_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(
                        scrollbar_uniform_buffer.as_entire_buffer_binding(),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(
                        scrollbar_buffer.as_entire_buffer_binding(),
                    ),
                },
            ],
        });

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
            tab_bar_buffer,
            tab_bar_uniform_buffer,
            tab_bar_bind_group,
            status_bar_buffer,
            status_bar_uniform_buffer,
            status_bar_bind_group,
            scrollbar_buffer,
            scrollbar_uniform_buffer,
            scrollbar_bind_group,
            atlas_bind_group_layout,
            atlas_bind_group,
            glyph_atlas,
            cell_size: [cell_width, cell_height],
            cell_width,
            cell_height,
            cell_buffer_capacity,
            dirty_cells: Vec::new(),
            viewport_origin: [0.0, 0.0],
            padding: [16.0, 16.0, 16.0, 16.0],
            current_frame: None,
            current_view: None,
            current_encoder: None,
            needs_clear: false,
            clear_color: [0.102, 0.106, 0.149],
            opacity,
            theme: crate::theme::Theme::tokyo_night(),
            image_texture: Some(placeholder_tex),
            image_view: Some(placeholder_view),
            uploaded_image_id: None,
            image_frames: Vec::new(),
            anim_frame_index: 0,
            anim_last_swap: std::time::Instant::now(),
            is_animated: false,
            cursor_blink: false,
            blink_visible: true,
            blink_last_toggle: std::time::Instant::now(),
            blink_interval: std::time::Duration::from_millis(530),
            cursor_blink_enabled: true,
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
        self.surface.configure(&self.device, &self.config);

        if needs_resize {
            self.cell_buffer_capacity = new_cols * new_rows;
            self.cell_buffer = {
                let cell_data = vec![CellData::zeroed(); self.cell_buffer_capacity];
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
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
                        resource: wgpu::BindingResource::Buffer(
                            self.uniform_buffer.as_entire_buffer_binding(),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(
                            self.cell_buffer.as_entire_buffer_binding(),
                        ),
                    },
                ],
            });
            self.tab_bar_buffer = {
                let cells = vec![CellData::zeroed(); new_cols];
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Tab Bar Cell Buffer"),
                        contents: bytemuck::cast_slice(&cells),
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    })
            };
            self.tab_bar_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Tab Bar Bind Group"),
                layout: &self.render_pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(
                            self.tab_bar_uniform_buffer.as_entire_buffer_binding(),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(
                            self.tab_bar_buffer.as_entire_buffer_binding(),
                        ),
                    },
                ],
            });
            self.status_bar_buffer = {
                let cells = vec![CellData::zeroed(); new_cols];
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Status Bar Cell Buffer"),
                        contents: bytemuck::cast_slice(&cells),
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    })
            };
            self.status_bar_bind_group =
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Status Bar Bind Group"),
                    layout: &self.render_pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(
                                self.status_bar_uniform_buffer.as_entire_buffer_binding(),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Buffer(
                                self.status_bar_buffer.as_entire_buffer_binding(),
                            ),
                        },
                    ],
                });
            self.scrollbar_buffer = {
                let cells = vec![CellData::zeroed(); SCROLLBAR_MAX_ROWS * 2];
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Scrollbar Cell Buffer"),
                        contents: bytemuck::cast_slice(&cells),
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    })
            };
            self.scrollbar_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Scrollbar Bind Group"),
                layout: &self.render_pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(
                            self.scrollbar_uniform_buffer.as_entire_buffer_binding(),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(
                            self.scrollbar_buffer.as_entire_buffer_binding(),
                        ),
                    },
                ],
            });
        }

        let uniforms = Uniforms {
            screen_size: [width as f32, height as f32],
            cell_size: [self.cell_width, self.cell_height],
            viewport_origin: [0.0, 0.0],
            cols: new_cols as u32,
            rows: new_rows as u32,
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    /// Offset added to every cell position in the shader (pixels).
    /// Applied when render_screen writes uniforms; no separate write_buffer.
    pub fn set_viewport(&mut self, x: f32, y: f32) {
        self.viewport_origin = [x, y];
    }

    pub fn cols_for(&self, width: f32) -> usize {
        let usable = width - self.padding[1] - self.padding[3];
        (usable / self.cell_size[0]).floor().max(1.0) as usize
    }

    pub fn rows_for(&self, height: f32) -> usize {
        let usable = height - self.padding[0] - self.padding[2];
        (usable / self.cell_size[1]).floor().max(1.0) as usize
    }

    pub fn begin_frame(&mut self) -> Result<()> {
        let frame = loop {
            match self.surface.get_current_texture() {
                Ok(frame) => break frame,
                Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                    self.surface.configure(&self.device, &self.config);
                    // Surface was recreated; retry so a real frame is
                    // acquired instead of silently rendering nothing.
                    continue;
                }
                Err(e) => {
                    log::warn!("Surface error: {}", e);
                    return Ok(());
                }
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });
        self.current_frame = Some(frame);
        self.current_view = Some(view);
        self.current_encoder = Some(encoder);
        self.needs_clear = true;
        Ok(())
    }

    /// Fill the entire window with a solid background so hairline gaps
    /// between cell quads (fractional device-pixel rounding) don't show the
    /// clear color underneath. Must be called after begin_frame() and before
    /// any render_screen()/draw_tab_bar() pass.
    pub fn draw_background(&mut self, color: [f32; 4]) -> Result<()> {
        if self.current_view.is_none() || self.current_encoder.is_none() {
            return Ok(());
        }
        let bg = CellData {
            glyph_uv_min: [0.0, 0.0],
            glyph_uv_max: [0.0, 0.0],
            glyph_size: [0.0, 0.0],
            _pad0: [0.0, 0.0],
            fg: color,
            bg: color,
            attrs: 0,
            _pad1: [0; 3],
        };
        self.queue
            .write_buffer(&self.tab_bar_buffer, 0, bytemuck::cast_slice(&[bg]));

        let uniforms = Uniforms {
            screen_size: [self.size.width as f32, self.size.height as f32],
            cell_size: [self.size.width as f32, self.size.height as f32],
            viewport_origin: [0.0, 0.0],
            cols: 1,
            rows: 1,
        };
        self.queue.write_buffer(
            &self.tab_bar_uniform_buffer,
            0,
            bytemuck::cast_slice(&[uniforms]),
        );

        let view = self.current_view.as_ref().unwrap();
        let encoder = self.current_encoder.as_mut().unwrap();
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Background Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
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
        render_pass.set_bind_group(0, &self.tab_bar_bind_group, &[]);
        render_pass.set_bind_group(1, &self.atlas_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        render_pass.draw(0..6, 0..1);

        self.needs_clear = false;
        Ok(())
    }

    pub fn render_screen(
        &mut self,
        screen: &CoreScreen,
        scroll_offset: usize,
        selection: Option<Selection>,
    ) -> Result<()> {
        if self.current_view.is_none() || self.current_encoder.is_none() {
            return Ok(());
        }

        let buffer = screen.buffer();
        let mut visible_rows = buffer.len();
        let mut cols = if visible_rows > 0 { buffer[0].len() } else { 0 };

        // GPU cell buffer is sized from the window (resize()). A resize race
        // or split-pane math can hand us a screen larger than that capacity —
        // clamp so the batch write and instance count never overrun the
        // storage buffer.
        let capacity = self.cell_buffer_capacity.max(1);
        if visible_rows.saturating_mul(cols) > capacity {
            visible_rows = (capacity / cols.max(1)).min(visible_rows);
            cols = (capacity / visible_rows.max(1)).min(cols);
        }

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
        let uniforms = Uniforms {
            screen_size: [self.size.width as f32, self.size.height as f32],
            cell_size: [self.cell_width, self.cell_height],
            viewport_origin: [
                self.viewport_origin[0] + self.padding[3],
                self.viewport_origin[1] + self.padding[0],
            ],
            cols: cols as u32,
            rows: visible_rows as u32,
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        let view = self.current_view.as_ref().unwrap();
        let encoder = self.current_encoder.as_mut().unwrap();
        let load = if self.needs_clear {
            self.needs_clear = false;
            wgpu::LoadOp::Clear(wgpu::Color {
                r: self.clear_color[0],
                g: self.clear_color[1],
                b: self.clear_color[2],
                a: self.opacity,
            })
        } else {
            wgpu::LoadOp::Load
        };

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
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

        Ok(())
    }

    pub fn end_frame(&mut self) -> Result<()> {
        if let Some(encoder) = self.current_encoder.take() {
            self.queue.submit(std::iter::once(encoder.finish()));
        }
        self.current_view = None;
        if let Some(frame) = self.current_frame.take() {
            frame.present();
        }
        Ok(())
    }

    /// Render a horizontal tab strip at the top of the window (one cell tall).
    /// Runs as its own instanced pass after the pane(s); shader is unchanged.
    pub fn draw_tab_bar(&mut self, tabs: &[TabInfo]) -> Result<()> {
        if self.current_view.is_none() || self.current_encoder.is_none() {
            return Ok(());
        }

        let t = self.theme;
        let inactive_bg: [f32; 4] = [
            t.surface.r as f32 / 255.0,
            t.surface.g as f32 / 255.0,
            t.surface.b as f32 / 255.0,
            1.0,
        ];
        // Active tab bg: surface_highlight tinted ~30% toward accent (the pill).
        let active_bg: [f32; 4] = [
            (t.accent.r as f32 * 0.3 + t.surface_highlight.r as f32 * 0.7) / 255.0,
            (t.accent.g as f32 * 0.3 + t.surface_highlight.g as f32 * 0.7) / 255.0,
            (t.accent.b as f32 * 0.3 + t.surface_highlight.b as f32 * 0.7) / 255.0,
            1.0,
        ];
        let fg: [f32; 4] = [
            t.fg.r as f32 / 255.0,
            t.fg.g as f32 / 255.0,
            t.fg.b as f32 / 255.0,
            1.0,
        ];
        let fg_dim: [f32; 4] = [
            t.border.r as f32 / 255.0,
            t.border.g as f32 / 255.0,
            t.border.b as f32 / 255.0,
            1.0,
        ];
        let accent_fg: [f32; 4] = [
            t.accent.r as f32 / 255.0,
            t.accent.g as f32 / 255.0,
            t.accent.b as f32 / 255.0,
            1.0,
        ];
        let hover_bg: [f32; 4] = [
            t.border.r as f32 / 255.0,
            t.border.g as f32 / 255.0,
            t.border.b as f32 / 255.0,
            1.0,
        ];
        let close_red: [f32; 4] = [
            t.ansi[1].r as f32 / 255.0,
            t.ansi[1].g as f32 / 255.0,
            t.ansi[1].b as f32 / 255.0,
            1.0,
        ];

        let cols = (self.size.width as f32 / self.cell_size[0])
            .floor()
            .max(1.0) as usize;
        let space = self
            .glyph_atlas
            .get_or_insert_glyph(' ', &self.device, &self.queue);

        let mut batch = vec![CellData::zeroed(); cols];
        for cell in &mut batch {
            cell.bg = inactive_bg;
            cell.fg = fg_dim;
            cell.glyph_uv_min = [space.0, space.1];
            cell.glyph_uv_max = [space.2, space.3];
            cell.glyph_size = [space.4, space.5];
        }

        let mut col = 1usize;
        for tab in tabs {
            if col >= cols {
                break;
            }
            let title = truncate_title(&tab.title, 20);
            let span = tab_span(&tab.title, 20);
            let (bg, fgc) = if tab.active {
                (active_bg, accent_fg)
            } else if tab.hovered {
                (hover_bg, fg)
            } else {
                (inactive_bg, fg_dim)
            };
            let end = (col + span).min(cols);
            for cell in batch.iter_mut().take(end).skip(col) {
                cell.bg = bg;
                cell.fg = fgc;
            }
            // 1-cell accent line under the active tab (left padding cell).
            if tab.active && col < end {
                batch[col].bg = accent_fg;
            }
            for (k, ch) in title.chars().enumerate() {
                let c = col + 1 + k;
                if c >= cols {
                    break;
                }
                let (u0, v0, u1, v1, gw, gh) =
                    self.glyph_atlas
                        .get_or_insert_glyph(ch, &self.device, &self.queue);
                batch[c].glyph_uv_min = [u0, v0];
                batch[c].glyph_uv_max = [u1, v1];
                batch[c].glyph_size = [gw, gh];
            }
            // Close button occupies the right padding cell; only drawn while hovered.
            if tab.hovered {
                let close_c = col + span - 1;
                if close_c < cols {
                    let (u0, v0, u1, v1, gw, gh) =
                        self.glyph_atlas
                            .get_or_insert_glyph('×', &self.device, &self.queue);
                    batch[close_c].glyph_uv_min = [u0, v0];
                    batch[close_c].glyph_uv_max = [u1, v1];
                    batch[close_c].glyph_size = [gw, gh];
                    batch[close_c].fg = if tab.close_hovered {
                        close_red
                    } else {
                        accent_fg
                    };
                }
            }
            col += span + 1;
        }

        self.queue
            .write_buffer(&self.tab_bar_buffer, 0, bytemuck::cast_slice(&batch));

        let uniforms = Uniforms {
            screen_size: [self.size.width as f32, self.size.height as f32],
            cell_size: [self.cell_width, self.cell_height],
            viewport_origin: [0.0, 0.0],
            cols: cols as u32,
            rows: 1,
        };
        self.queue.write_buffer(
            &self.tab_bar_uniform_buffer,
            0,
            bytemuck::cast_slice(&[uniforms]),
        );

        let view = self.current_view.as_ref().unwrap();
        let encoder = self.current_encoder.as_mut().unwrap();
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Tab Bar Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, &self.tab_bar_bind_group, &[]);
        render_pass.set_bind_group(1, &self.atlas_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        render_pass.draw(0..6, 0..cols as u32);

        Ok(())
    }

    /// Height of the status bar in pixels (one cell row).
    pub fn status_bar_height(&self) -> f32 {
        self.cell_height
    }

    /// Render a one-cell-tall status bar across the bottom of the window,
    /// with the active pane title left and a scroll indicator right.
    pub fn draw_status_bar(&mut self, left: &str, right: &str) -> Result<()> {
        if self.current_view.is_none() || self.current_encoder.is_none() {
            return Ok(());
        }
        let t = self.theme;
        let bg: [f32; 4] = [
            t.surface.r as f32 / 255.0,
            t.surface.g as f32 / 255.0,
            t.surface.b as f32 / 255.0,
            1.0,
        ];
        let fg: [f32; 4] = [
            t.fg.r as f32 / 255.0,
            t.fg.g as f32 / 255.0,
            t.fg.b as f32 / 255.0,
            1.0,
        ];

        let cols = (self.size.width as f32 / self.cell_width).floor().max(1.0) as usize;
        let space = self
            .glyph_atlas
            .get_or_insert_glyph(' ', &self.device, &self.queue);
        let mut batch = vec![CellData::zeroed(); cols];
        for cell in &mut batch {
            cell.bg = bg;
            cell.fg = fg;
            cell.glyph_uv_min = [space.0, space.1];
            cell.glyph_uv_max = [space.2, space.3];
            cell.glyph_size = [space.4, space.5];
        }

        let title = truncate_title(left, cols.saturating_sub(2));
        for (k, ch) in title.chars().enumerate() {
            let c = 1 + k;
            if c >= cols {
                break;
            }
            let (u0, v0, u1, v1, gw, gh) =
                self.glyph_atlas
                    .get_or_insert_glyph(ch, &self.device, &self.queue);
            batch[c].glyph_uv_min = [u0, v0];
            batch[c].glyph_uv_max = [u1, v1];
            batch[c].glyph_size = [gw, gh];
        }
        let mut c = cols.saturating_sub(1);
        for ch in right.chars().rev() {
            let (u0, v0, u1, v1, gw, gh) =
                self.glyph_atlas
                    .get_or_insert_glyph(ch, &self.device, &self.queue);
            batch[c].glyph_uv_min = [u0, v0];
            batch[c].glyph_uv_max = [u1, v1];
            batch[c].glyph_size = [gw, gh];
            if c == 0 {
                break;
            }
            c -= 1;
        }

        self.queue
            .write_buffer(&self.status_bar_buffer, 0, bytemuck::cast_slice(&batch));
        let uniforms = Uniforms {
            screen_size: [self.size.width as f32, self.size.height as f32],
            cell_size: [self.cell_width, self.cell_height],
            viewport_origin: [0.0, self.size.height as f32 - self.cell_height],
            cols: cols as u32,
            rows: 1,
        };
        self.queue.write_buffer(
            &self.status_bar_uniform_buffer,
            0,
            bytemuck::cast_slice(&[uniforms]),
        );

        let view = self.current_view.as_ref().unwrap();
        let encoder = self.current_encoder.as_mut().unwrap();
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Status Bar Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, &self.status_bar_bind_group, &[]);
        render_pass.set_bind_group(1, &self.atlas_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        render_pass.draw(0..6, 0..cols as u32);
        Ok(())
    }

    /// Right-edge scrollbar overlay over the active pane. `fraction` is
    /// scroll_offset/max_scroll_offset, so 0.0 = at the bottom (thumb at the
    /// bottom). Track fills the pane's 2 rightmost cells, thumb is accent.
    pub fn draw_scrollbar(
        &mut self,
        vx: f32,
        vy: f32,
        vw: f32,
        vh: f32,
        fraction: f32,
    ) -> Result<()> {
        if self.current_view.is_none() || self.current_encoder.is_none() {
            return Ok(());
        }
        let t = self.theme;
        let track: [f32; 4] = [
            t.surface.r as f32 / 255.0,
            t.surface.g as f32 / 255.0,
            t.surface.b as f32 / 255.0,
            1.0,
        ];
        let thumb: [f32; 4] = [
            t.accent.r as f32 / 255.0,
            t.accent.g as f32 / 255.0,
            t.accent.b as f32 / 255.0,
            1.0,
        ];
        let space = self
            .glyph_atlas
            .get_or_insert_glyph(' ', &self.device, &self.queue);

        let bar_x = vx + vw - 2.0 * self.cell_width;
        let bar_y = vy + self.padding[0];
        let bar_h = (vh - self.padding[0] - self.padding[2]).max(0.0);
        // Clamp to the storage buffer's row capacity (never overrun).
        let rows = ((bar_h / self.cell_height).floor() as usize).min(SCROLLBAR_MAX_ROWS);
        if rows == 0 {
            return Ok(());
        }
        let batch_len = 2 * rows;
        let mut batch = vec![CellData::zeroed(); batch_len];
        for cell in &mut batch {
            cell.bg = track;
            cell.glyph_uv_min = [space.0, space.1];
            cell.glyph_uv_max = [space.2, space.3];
            cell.glyph_size = [space.4, space.5];
        }

        let visible_fraction = 1.0 - fraction;
        let mut thumb_rows = (rows as f32 * visible_fraction)
            .ceil()
            .max(2.0)
            .min(rows as f32) as usize;
        if thumb_rows < 1 {
            thumb_rows = 1;
        }
        let max_start = rows.saturating_sub(thumb_rows);
        let tstart =
            (rows as f32 - thumb_rows as f32 - fraction * (rows as f32 - thumb_rows as f32))
                .round()
                .clamp(0.0, max_start as f32) as usize;
        for r in tstart..(tstart + thumb_rows).min(rows) {
            let base = 2 * r;
            batch[base].bg = thumb;
            batch[base + 1].bg = thumb;
        }

        self.queue
            .write_buffer(&self.scrollbar_buffer, 0, bytemuck::cast_slice(&batch));
        let uniforms = Uniforms {
            screen_size: [self.size.width as f32, self.size.height as f32],
            cell_size: [self.cell_width, self.cell_height],
            viewport_origin: [bar_x, bar_y],
            cols: 2,
            rows: rows as u32,
        };
        self.queue.write_buffer(
            &self.scrollbar_uniform_buffer,
            0,
            bytemuck::cast_slice(&[uniforms]),
        );

        let view = self.current_view.as_ref().unwrap();
        let encoder = self.current_encoder.as_mut().unwrap();
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Scrollbar Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, &self.scrollbar_bind_group, &[]);
        render_pass.set_bind_group(1, &self.atlas_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        render_pass.draw(0..6, 0..batch_len as u32);
        Ok(())
    }

    fn update_cell_data(
        &mut self,
        screen: &CoreScreen,
        scroll_offset: usize,
        selection: Option<Selection>,
    ) -> Result<()> {
        let buffer = screen.buffer();
        let mut visible_rows = buffer.len();
        let mut cols = if visible_rows > 0 { buffer[0].len() } else { 0 };

        // Same clamp as render_screen: the GPU cell buffer is window-sized;
        // never build a batch that overruns it.
        let capacity = self.cell_buffer_capacity.max(1);
        if visible_rows.saturating_mul(cols) > capacity {
            visible_rows = (capacity / cols.max(1)).min(visible_rows);
            cols = (capacity / visible_rows.max(1)).min(cols);
        }

        let cursor = screen.cursor();
        let cursor_col = cursor.col;
        let cursor_visible = cursor.visible;
        let cursor_shape = cursor.shape;

        let scrollback = screen.scrollback();
        let total_scrollback = scrollback.len();
        let total_rows = total_scrollback + visible_rows;

        let end = total_rows.saturating_sub(scroll_offset);
        let start = end.saturating_sub(visible_rows);

        // ponytail: block start_line is buffer-local; divider rows only line up
        // with view rows while scroll_offset == 0. Scrolled dividers are skipped.
        let mut divider_rows = std::collections::HashSet::new();
        let mut divider_meta: std::collections::HashMap<usize, Vec<char>> =
            std::collections::HashMap::new();
        for block in screen.blocks() {
            divider_rows.insert(block.start_line);
            divider_meta.insert(
                block.start_line,
                screen.block_metadata(block).chars().collect(),
            );
        }

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

            let mut fg = self.theme.map_cell_color(cell.fg);
            let bg = self.theme.map_cell_color(cell.bg);

            // Syntax classes are tagged into cells at write time (see Screen),
            // so scrollback rows carry their colors too — no scroll_offset gate.
            let mut cell_attrs = cell.attrs;
            if cell.syntax_color != 0 {
                if let Some(c) = Self::highlight_color(cell.syntax_color, &self.theme) {
                    fg = c;
                }
                if cell.syntax_color == zeroterm_core::highlight::HL_URL {
                    cell_attrs.underline = zeroterm_core::cell::UnderlineStyle::Single;
                }
            }

            let is_cursor_cell = cursor_visible
                && self.blink_visible
                && scroll_offset == 0
                && dirty_row == cursor.row
                && dirty_col == cursor_col;
            let (fg, bg, cell_attrs) = if is_cursor_cell {
                match cursor_shape {
                    zeroterm_core::cell::CursorShape::Block => (bg, fg, cell_attrs),
                    zeroterm_core::cell::CursorShape::Underline => {
                        let mut a = cell_attrs;
                        a.underline = zeroterm_core::cell::UnderlineStyle::Single;
                        (fg, bg, a)
                    }
                    zeroterm_core::cell::CursorShape::Bar => (fg, bg, cell_attrs),
                }
            } else {
                (fg, bg, cell_attrs)
            };

            let is_selected = selection.is_some_and(|s| s.contains(combined_idx, dirty_col));

            let mut attrs = (cell_attrs.bold as u32)
                | ((cell_attrs.italic as u32) << 1)
                | (((cell_attrs.underline != zeroterm_core::cell::UnderlineStyle::None) as u32)
                    << 2)
                | ((cell_attrs.strikethrough as u32) << 3)
                | ((cell_attrs.dim as u32) << 4)
                | ((cell_attrs.blink as u32) << 5)
                | ((cell_attrs.reverse as u32) << 6)
                | ((cell_attrs.invisible as u32) << 7)
                | (if is_cursor_cell
                    && matches!(cursor_shape, zeroterm_core::cell::CursorShape::Bar)
                {
                    0x100u32
                } else {
                    0
                })
                | (if is_selected { 0x200u32 } else { 0 });
            if screen
                .image_cells()
                .contains_key(&(combined_idx, dirty_col))
            {
                attrs |= ATTR_HAS_IMAGE;
            }

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

            let mut ch = cell.ch;
            if divider_rows.contains(&dirty_row) {
                attrs |= ATTR_BLOCK_DIVIDER;
                let meta = divider_meta.get(&dirty_row);
                let meta_len = meta.map_or(0, Vec::len);
                let copy_start = cols.saturating_sub(COPY_MARKER.len());
                let meta_start = copy_start.saturating_sub(meta_len);
                let overlay = if dirty_col >= copy_start {
                    COPY_MARKER
                        .as_bytes()
                        .get(dirty_col - copy_start)
                        .map(|&b| b as char)
                } else if dirty_col >= meta_start {
                    meta.and_then(|m| m.get(dirty_col - meta_start)).copied()
                } else {
                    None
                };
                if let Some(c) = overlay {
                    ch = c;
                    attrs |= ATTR_DIM;
                }
            }

            let (u0, v0, u1, v1, gw, gh) =
                self.glyph_atlas
                    .get_or_insert_glyph(ch, &self.device, &self.queue);

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

        self.queue
            .write_buffer(&self.cell_buffer, 0, bytemuck::cast_slice(&batch));

        Ok(())
    }

    pub fn reload_config(&mut self, config: &zeroterm_config::Config) {
        self.theme = crate::theme::Theme::by_name(&config.colors.theme);
        let (r, g, b) = (
            self.theme.bg.r as f64 / 255.0,
            self.theme.bg.g as f64 / 255.0,
            self.theme.bg.b as f64 / 255.0,
        );
        self.clear_color = [r, g, b];
        self.opacity = config.window.opacity;
        self.set_cursor_blink(config.cursor.blink, config.cursor.blink_interval_ms);
    }

    pub fn theme_bg(&self) -> [f32; 4] {
        [
            self.theme.bg.r as f32 / 255.0,
            self.theme.bg.g as f32 / 255.0,
            self.theme.bg.b as f32 / 255.0,
            1.0,
        ]
    }

    pub fn set_theme(&mut self, name: &str) {
        self.theme = crate::theme::Theme::by_name(name);
        let (r, g, b) = (
            self.theme.bg.r as f64 / 255.0,
            self.theme.bg.g as f64 / 255.0,
            self.theme.bg.b as f64 / 255.0,
        );
        self.clear_color = [r, g, b];
    }

    pub fn set_opacity(&mut self, opacity: f64) {
        self.opacity = opacity;
    }

    fn update_image_from_screen(&mut self, screen: &CoreScreen) {
        let img = match latest_new_image(screen.image_registry(), self.uploaded_image_id) {
            Some(img) => img,
            None => return,
        };
        self.image_frames.clear();
        if img.frames.is_empty() {
            self.image_frames
                .push((img.rgba_data.clone(), img.width, img.height, 0));
        } else {
            for f in &img.frames {
                self.image_frames
                    .push((f.rgba.clone(), f.width, f.height, f.delay_ms));
            }
        }
        let (rgba, width, height, _) = self.image_frames[0].clone();
        self.upload_image_texture(&rgba, width, height);
        self.is_animated = self.image_frames.len() > 1;
        self.anim_frame_index = 0;
        self.anim_last_swap = std::time::Instant::now();
        self.uploaded_image_id = Some(img.id);
    }

    /// Upload RGBA bytes into the image texture (recreating it when the size
    /// changes) and re-point the atlas bind group's image binding at it.
    fn upload_image_texture(&mut self, rgba: &[u8], width: u32, height: u32) {
        let size_mismatch = match &self.image_texture {
            Some(t) => t.width() != width || t.height() != height,
            None => true,
        };
        if size_mismatch {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Image Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
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
        }
        let texture = self.image_texture.as_ref().expect("image texture set");
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Progress the animation if the current frame's delay has elapsed and
    /// return how long until the next swap, or `None` for static images.
    /// The caller should request a redraw whenever this returns `Some`.
    pub fn next_frame_delay(&mut self) -> Option<std::time::Duration> {
        if !self.is_animated {
            return None;
        }
        let (_, _, _, delay_ms) = self.image_frames.get(self.anim_frame_index)?;
        let delay = std::time::Duration::from_millis((*delay_ms).max(1));
        if self.anim_last_swap.elapsed() >= delay {
            self.anim_frame_index = (self.anim_frame_index + 1) % self.image_frames.len();
            self.anim_last_swap = std::time::Instant::now();
            if let Some((rgba, width, height, _)) =
                self.image_frames.get(self.anim_frame_index).cloned()
            {
                self.upload_image_texture(&rgba, width, height);
            }
            Some(delay)
        } else {
            Some(delay.saturating_sub(self.anim_last_swap.elapsed()))
        }
    }

    pub fn set_cursor_blink(&mut self, enabled: bool, interval_ms: u64) {
        self.cursor_blink_enabled = enabled;
        self.cursor_blink = enabled;
        self.blink_interval = std::time::Duration::from_millis(interval_ms);
    }

    /// Progress the cursor blink phase if its interval has elapsed and return
    /// how long until the next toggle, or `None` when blinking is disabled.
    /// The caller should request a redraw whenever this returns `Some`.
    pub fn cursor_blink_tick(&mut self) -> Option<std::time::Duration> {
        if !self.cursor_blink_enabled {
            if self.cursor_blink {
                self.cursor_blink = false;
            }
            self.blink_visible = true;
            return None;
        }
        let elapsed = self.blink_last_toggle.elapsed();
        if elapsed >= self.blink_interval {
            self.blink_visible = !self.blink_visible;
            self.cursor_blink = self.blink_visible;
            self.blink_last_toggle = std::time::Instant::now();
            Some(self.blink_interval)
        } else {
            Some(self.blink_interval.saturating_sub(elapsed))
        }
    }

    pub fn reload_font(&mut self, font_path: Option<String>) {
        self.glyph_atlas.font_path = font_path.clone();
        if let Ok(data) = GlyphAtlas::load_font(font_path) {
            self.glyph_atlas.font_data = data;
            self.glyph_atlas.repack_ascii(&self.device, &self.queue);
        }
    }

    /// Palette for `highlight` classes, mapped through the active theme's ANSI colors.
    fn highlight_color(idx: u8, theme: &crate::theme::Theme) -> Option<Color> {
        match idx {
            zeroterm_core::highlight::HL_KEYWORD => Some(theme.ansi[6]),
            zeroterm_core::highlight::HL_STRING => Some(theme.ansi[3]),
            zeroterm_core::highlight::HL_NUMBER => Some(theme.ansi[5]),
            zeroterm_core::highlight::HL_COMMENT => Some(theme.ansi[8]),
            zeroterm_core::highlight::HL_URL => Some(theme.accent),
            _ => None,
        }
    }
}

fn truncate_title(title: &str, max: usize) -> String {
    if title.chars().count() <= max {
        title.to_string()
    } else {
        let mut s: String = title.chars().take(max).collect();
        s.push('\u{2026}');
        s
    }
}

/// Width of a tab in cells: truncated title + one cell of padding each side.
/// Must be identical wherever a tab's hit region is computed (draw_tab_bar and
/// the app's tab_at_point), otherwise clicks land on the wrong tab.
pub fn tab_span(title: &str, max_title: usize) -> usize {
    truncate_title(title, max_title)
        .chars()
        .count()
        .saturating_add(2)
}

/// The newest image in the registry that has not been uploaded yet, or `None`
/// when the registry is empty or the newest id is already uploaded. The old
/// code gated on a sticky `has_image` flag, so once any image was shown every
/// later image rendered as the first one (and `clear` never reset it).
fn latest_new_image(
    reg: &HashMap<u32, zeroterm_core::screen::ImageData>,
    uploaded: Option<u32>,
) -> Option<&zeroterm_core::screen::ImageData> {
    let img = reg.values().max_by_key(|img| img.id)?;
    if uploaded == Some(img.id) {
        None
    } else {
        Some(img)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroterm_core::screen::ImageData;

    fn image(id: u32) -> ImageData {
        ImageData {
            id,
            width: 8,
            height: 8,
            rgba_data: vec![0u8; 8 * 8 * 4],
            frames: vec![],
        }
    }

    #[test]
    fn image_update_tracks_newest_id_not_sticky_flag() {
        let mut reg = HashMap::new();
        reg.insert(0, image(0));
        // Nothing uploaded yet -> picks the newest image.
        assert_eq!(latest_new_image(&reg, None).unwrap().id, 0);
        // Same registry, id 0 already uploaded -> no re-upload.
        assert!(latest_new_image(&reg, Some(0)).is_none());
        // A later image must be picked even though one was already shown.
        // (Old code gated on a sticky has_image flag and kept rendering image 0.)
        reg.insert(1, image(1));
        assert_eq!(latest_new_image(&reg, Some(0)).unwrap().id, 1);
        // Eviction only removes the oldest; newest stays stable.
        reg.remove(&0);
        assert!(latest_new_image(&reg, Some(1)).is_none());
    }

    #[test]
    fn image_update_ignores_empty_registry() {
        let reg = HashMap::new();
        assert!(latest_new_image(&reg, None).is_none());
        assert!(latest_new_image(&reg, Some(0)).is_none());
    }

    #[test]
    fn tab_span_matches_drawn_width() {
        // Short title: len + 2 padding cells.
        assert_eq!(tab_span("short", 20), 7);
        // Long title: truncated to 20 chars + ellipsis (1) + 2 padding.
        assert_eq!(tab_span(&"x".repeat(30), 20), 23);
        // Empty title: 2 padding cells only.
        assert_eq!(tab_span("", 20), 2);
    }
}
