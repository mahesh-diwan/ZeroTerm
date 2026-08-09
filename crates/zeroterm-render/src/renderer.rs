//! ZeroTerm Renderer - wgpu-based GPU renderer

use bytemuck::{Pod, Zeroable};
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use std::collections::HashMap;

use zeroterm_core::screen::Screen as CoreScreen;

use crate::atlas::GlyphAtlas;
use crate::cell_batch::{CellBatch, GlyphQuad, ATTR_DIM};
use crate::diag::Diag;
use crate::pass::Pass;

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
    #[error("font file could not be parsed: {0}")]
    FontInvalid(String),
    /// The surface could not present a frame (occluded, timed out, OOM). The
    /// caller must NOT treat this as success: no frame was acquired, so the
    /// redraw loop must stay alive and retry once the surface is presentable
    /// again. Previously this was swallowed (returning Ok), which froze the
    /// window on whatever it last presented — often a blank frame at startup.
    #[error("surface not presentable: {0}")]
    Surface(wgpu::SurfaceError),
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

/// Tab bar height in cell rows. draw_tab_bar and tab_bar_height() both use
/// this; the content viewport offset in main.rs derives from tab_bar_height(),
/// so a mismatch would draw the bar taller than the layout reserves.
/// Chrome rows reserved above the grid: tab bar plus status bar. These are
/// the single source of truth for spawn-time size estimates in the app crate
/// (cells_for_size) so they can never drift from this layout.
pub const TAB_BAR_ROWS: usize = 2;
pub const STATUS_BAR_ROWS: usize = 1;
/// Grid padding in physical pixels, ordered [left, right, top, bottom].
pub const PADDING: [f32; 4] = [16.0, 16.0, 16.0, 16.0];

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub(crate) struct Uniforms {
    screen_size: [f32; 2],
    cell_size: [f32; 2],
    viewport_origin: [f32; 2],
    cols: u32,
    rows: u32,
    /// 1 when the surface alpha mode is PreMultiplied: the fragment shader
    /// then premultiplies its output so translucent regions composite
    /// correctly instead of glowing too bright.
    premultiply: u32,
    /// Trailing pad: keeps the byte size at 40 so it matches the WGSL
    /// uniform struct's layout (align 8 → struct size rounded to 40).
    _pad: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub(crate) struct CellData {
    pub(crate) glyph_uv_min: [f32; 2],
    pub(crate) glyph_uv_max: [f32; 2],
    pub(crate) glyph_size: [f32; 2],
    /// Top-left of the glyph bitmap inside the cell, in cell pixels, y-down
    /// (placement.left, baseline - placement.top). See atlas::cell_offset.
    pub(crate) glyph_offset: [f32; 2],
    pub(crate) fg: [f32; 4],
    pub(crate) bg: [f32; 4],
    pub(crate) attrs: u32,
    pub(crate) _pad1: [u32; 3],
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


/// Generous max rows for the scrollbar overlay storage buffer (2 col cells each).
const SCROLLBAR_MAX_ROWS: usize = 512;

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    /// Surface alpha modes for the two states of window.opacity. The surface
    /// must be reconfigured with a compositor-capable mode when the window is
    /// translucent (Opaque discards alpha entirely).
    opaque_alpha_mode: wgpu::CompositeAlphaMode,
    transparent_alpha_mode: wgpu::CompositeAlphaMode,
    /// 1 while the surface uses PreMultiplied alpha (see Uniforms.premultiply).
    premultiply_output: u32,
    size: PhysicalSize<u32>,
    render_pipeline: wgpu::RenderPipeline,
    content_pass: Pass,
    quad_vertex_buffer: wgpu::Buffer,
    tab_bar_pass: Pass,
    status_bar_pass: Pass,
    scrollbar_pass: Pass,
    atlas_bind_group: wgpu::BindGroup,
    glyph_atlas: GlyphAtlas,
    cell_size: [f32; 2],
    cell_width: f32,
    cell_height: f32,
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
    /// Consecutive surface-acquire failures (see begin_frame). Reset to 0 on
    /// every successful acquire; used to rate-limit error logs and to emit a
    /// recovery notice so a blank-window bug is attributable from the log.
    surface_failures: u32,
    /// Env-gated diagnostics (ZTDIAG=1): frame dumps + probes for attributing
    /// blank-window reports to the framebuffer vs the compositor.
    diag: Diag,
}

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        font_size: f32,
        opacity: f64,
        font_path: Option<String>,
    ) -> Result<Self> {
        let size = window.inner_size();
        // Rasterize glyphs at the physical (device) pixel size. window.inner_size()
        // is in physical px, so cells must be too — otherwise text on a scale-2
        // display renders half the intended size (or gets upscaled into blocks).
        let font_size = font_size * window.scale_factor().max(0.5) as f32;

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

        let opaque_alpha_mode = if surface_caps
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
        // An Opaque surface makes the compositor ignore every alpha byte we
        // write, so window.opacity could never show the desktop through the
        // terminal. Pick a compositor-capable mode for the translucent case.
        let transparent_alpha_mode = [
            wgpu::CompositeAlphaMode::PostMultiplied,
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::Auto,
        ]
        .iter()
        .find(|m| surface_caps.alpha_modes.contains(m))
        .copied()
        .unwrap_or(opaque_alpha_mode);
        let alpha_mode = if opacity < 1.0 {
            transparent_alpha_mode
        } else {
            opaque_alpha_mode
        };
        if opacity < 1.0 && transparent_alpha_mode == opaque_alpha_mode {
            log::warn!(
                "this surface exposes no compositor-capable alpha mode; window.opacity < 1.0 \
                 will be ignored and the window stays opaque"
            );
        }
        // PreMultiplied surfaces expect the framebuffer's RGB to already be
        // scaled by alpha; we write straight colors, so the shader must do the
        // multiplication (Opaque/PostMultiplied never need it: a=1 or the
        // compositor multiplies).
        let premultiply_output = u32::from(alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied);

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
        let diag = Diag::new();
        diag.probe(
            "surface",
            &format!(
                "format={surface_format:?} alpha={alpha_mode:?} present={present_mode:?}"
            ),
        );

        let config = wgpu::SurfaceConfiguration {
            // COPY_SRC lets ZTDIAG=1 read the presented frame back to disk so
            // a blank-window report is attributable to the app's framebuffer
            // vs the compositor (see end_frame).
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
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
        log::info!(
            "Alpha mode: {:?} (opaque surface: {:?}, translucent surface: {:?}), glyph raster {:.1}px",
            alpha_mode,
            opaque_alpha_mode,
            transparent_alpha_mode,
            font_size
        );
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
            premultiply: premultiply_output,
            _pad: 0,
        };
        let cell_buffer_capacity = (cols as usize) * (rows as usize);

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

        let content_pass = Pass::new(
            &device,
            &uniform_bind_group_layout,
            "Content",
            cell_buffer_capacity,
            uniforms,
        );

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

        let tab_bar_pass = Pass::new(
            &device,
            &uniform_bind_group_layout,
            "Tab Bar",
            (cols as usize) * 2,
            uniforms,
        );

        let status_bar_pass = Pass::new(
            &device,
            &uniform_bind_group_layout,
            "Status Bar",
            512,
            uniforms,
        );

        let scrollbar_pass = Pass::new(
            &device,
            &uniform_bind_group_layout,
            "Scrollbar",
            SCROLLBAR_MAX_ROWS * 2,
            uniforms,
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            opaque_alpha_mode,
            transparent_alpha_mode,
            premultiply_output,
            size,
            render_pipeline,
            content_pass,
            quad_vertex_buffer,
            tab_bar_pass,
            status_bar_pass,
            scrollbar_pass,
            atlas_bind_group_layout,
            atlas_bind_group,
            glyph_atlas,
            cell_size: [cell_width, cell_height],
            cell_width,
            cell_height,
            dirty_cells: Vec::new(),
            viewport_origin: [0.0, 0.0],
            padding: PADDING,
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
            surface_failures: 0,
            diag,
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

        let needs_resize = new_cols * new_rows != self.content_pass.capacity();

        self.size = PhysicalSize::new(width, height);
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);

        if needs_resize {
            let layout = self.render_pipeline.get_bind_group_layout(0);
            self.content_pass
                .resize(&self.device, &layout, "Content", new_cols * new_rows);
            self.tab_bar_pass
                .resize(&self.device, &layout, "Tab Bar", new_cols * 2);
            self.status_bar_pass
                .resize(&self.device, &layout, "Status Bar", new_cols.max(512));
            self.scrollbar_pass
                .resize(&self.device, &layout, "Scrollbar", SCROLLBAR_MAX_ROWS * 2);
        }

        let uniforms = Uniforms {
            screen_size: [width as f32, height as f32],
            cell_size: [self.cell_width, self.cell_height],
            viewport_origin: [0.0, 0.0],
            cols: new_cols as u32,
            rows: new_rows as u32,
            premultiply: self.premultiply_output,
            _pad: 0,
        };
        self.content_pass.write_uniforms(&self.queue, &uniforms);
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
                Ok(frame) => {
                    if self.surface_failures > 0 {
                        log::info!(
                            "Surface recovered after {} failed acquire(s)",
                            self.surface_failures
                        );
                        self.surface_failures = 0;
                    }
                    break frame;
                }
                Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                    self.surface.configure(&self.device, &self.config);
                    // Surface was recreated; retry so a real frame is
                    // acquired instead of silently rendering nothing.
                    continue;
                }
                Err(e) => {
                    self.surface_failures += 1;
                    // Rate-limit: while the window is on another workspace the
                    // compositor sends no frame callbacks and every blink
                    // redraw can time out at ~2fps. Log the first couple, then
                    // every 60th, so the log stays readable.
                    if self.surface_failures <= 2 || self.surface_failures.is_multiple_of(60) {
                        log::warn!(
                            "Surface error (attempt {}): {}",
                            self.surface_failures,
                            e
                        );
                    }
                    // Never swallow: the caller must know no frame was
                    // acquired so it keeps the redraw loop alive and retries
                    // once the surface is presentable again.
                    return Err(RendererError::Surface(e));
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
            glyph_offset: [0.0, 0.0],
            fg: color,
            // Translucent when window.opacity < 1.0 so the desktop (or whatever
            // the compositor puts behind the window) shows through.
            bg: [color[0], color[1], color[2], self.opacity as f32],
            attrs: 0,
            _pad1: [0; 3],
        };
        self.tab_bar_pass.write_cells(&self.queue, std::slice::from_ref(&bg));

        let uniforms = Uniforms {
            screen_size: [self.size.width as f32, self.size.height as f32],
            cell_size: [self.size.width as f32, self.size.height as f32],
            viewport_origin: [0.0, 0.0],
            cols: 1,
            rows: 1,
            premultiply: self.premultiply_output,
            _pad: 0,
        };
        self.tab_bar_pass.write_uniforms(&self.queue, &uniforms);

        let view = self.current_view.as_ref();
        let Some(view) = view else {
            return Ok(());
        };
        let encoder = self.current_encoder.as_mut().unwrap();
        self.tab_bar_pass.draw(
            encoder,
            &self.render_pipeline,
            &self.atlas_bind_group,
            &self.quad_vertex_buffer,
            view,
            "Background Pass",
            1,
            Some(wgpu::Color {
                r: self.clear_color[0],
                g: self.clear_color[1],
                b: self.clear_color[2],
                a: self.opacity,
            }),
        );

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
        // GPU cell buffer is sized from the window (resize()). A resize race
        // or split-pane math can hand us a screen larger than that capacity —
        // clamp so the batch write and instance count never overrun the
        // storage buffer. Same rule as CellBatch::build (one source of truth).
        let capacity = self.content_pass.capacity();
        let (visible_rows, cols) =
            CellBatch::clamp_dims(buffer.len(), buffer.first().map_or(0, Vec::len), capacity);

        // Ground-truth probe (ZTDIAG=1): how many ink cells the screen holds
        // and what the last row says — distinguishes a parser/screen problem
        // from a GPU presentation problem.
        if self.diag.enabled() {
            let ink: usize = buffer
                .iter()
                .take(visible_rows)
                .map(|row| row.iter().filter(|c| c.ch != ' ').count())
                .sum();
            let last_text: String = buffer
                .get(visible_rows.saturating_sub(1))
                .map(|row| row.iter().take(70).map(|c| c.ch).collect())
                .unwrap_or_default();
            self.diag.probe(
                "render_screen",
                &format!(
                    "{visible_rows}x{cols} ink={ink} cap={capacity} size={}x{} cell={}x{} \
                     viewport={:?} last='{last_text}'",
                    self.size.width,
                    self.size.height,
                    self.cell_width,
                    self.cell_height,
                    self.viewport_origin,
                ),
            );
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
            premultiply: self.premultiply_output,
            _pad: 0,
        };
        self.content_pass.write_uniforms(&self.queue, &uniforms);

        let view = self.current_view.as_ref();
        let Some(view) = view else {
            return Ok(());
        };
        let encoder = self.current_encoder.as_mut().unwrap();
        let clear = if self.needs_clear {
            self.needs_clear = false;
            Some(wgpu::Color {
                r: self.clear_color[0],
                g: self.clear_color[1],
                b: self.clear_color[2],
                a: self.opacity,
            })
        } else {
            None
        };

        let instance_count = (visible_rows * cols) as u32;
        self.content_pass.draw(
            encoder,
            &self.render_pipeline,
            &self.atlas_bind_group,
            &self.quad_vertex_buffer,
            view,
            "Render Pass",
            instance_count,
            clear,
        );

        Ok(())
    }

    pub fn end_frame(&mut self) -> Result<()> {
        // ZTDIAG=1: queue a readback of this presented frame (inside the
        // encoder) so the dump is attributable to the framebuffer vs the
        // compositor. Finalized after submit.
        let mut readback: Option<crate::diag::DiagReadback> = None;
        if self.diag.should_dump_frame() {
            if let (Some(enc), Some(frame)) = (
                self.current_encoder.as_mut(),
                self.current_frame.as_ref(),
            ) {
                readback = Diag::queue_readback(
                    &self.device,
                    enc,
                    frame,
                    self.config.format,
                    self.config.width,
                    self.config.height,
                );
            }
        }
        if let Some(encoder) = self.current_encoder.take() {
            self.queue.submit(std::iter::once(encoder.finish()));
        }
        self.current_view = None;
        if let Some(rb) = readback {
            Diag::finalize_readback(&self.device, rb);
        }
        if let Some(frame) = self.current_frame.take() {
            frame.present();
        }
        Ok(())
    }

    /// Render a horizontal tab strip at the top of the window (two cell rows
    /// tall, ~40px — Ghostty/Kitty proportions). Runs as its own instanced
    /// pass after the pane(s); shader is unchanged. Row 0 carries the tab
    /// titles and close buttons; both rows carry the pill backgrounds so the
    /// bar reads as a solid strip with an active tab that stands out.
    pub fn draw_tab_bar(&mut self, tabs: &[TabInfo]) -> Result<()> {
        if self.current_view.is_none() || self.current_encoder.is_none() {
            return Ok(());
        }

        let t = self.theme;
        let bar_bg: [f32; 4] = [
            t.surface.r as f32 / 255.0,
            t.surface.g as f32 / 255.0,
            t.surface.b as f32 / 255.0,
            1.0,
        ];
        // Active tab pill: surface_highlight tinted ~40% toward accent so the
        // active tab clearly separates from the bar and from hover states.
        let active_bg: [f32; 4] = [
            (t.accent.r as f32 * 0.4 + t.surface_highlight.r as f32 * 0.6) / 255.0,
            (t.accent.g as f32 * 0.4 + t.surface_highlight.g as f32 * 0.6) / 255.0,
            (t.accent.b as f32 * 0.4 + t.surface_highlight.b as f32 * 0.6) / 255.0,
            1.0,
        ];
        // Hovered (inactive) tab pill: one step above the bar background.
        let hover_bg: [f32; 4] = [
            t.surface_highlight.r as f32 / 255.0,
            t.surface_highlight.g as f32 / 255.0,
            t.surface_highlight.b as f32 / 255.0,
            1.0,
        ];
        let accent_fg: [f32; 4] = [
            t.accent.r as f32 / 255.0,
            t.accent.g as f32 / 255.0,
            t.accent.b as f32 / 255.0,
            1.0,
        ];
        let fg: [f32; 4] = [
            t.fg.r as f32 / 255.0,
            t.fg.g as f32 / 255.0,
            t.fg.b as f32 / 255.0,
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

        let mut batch = vec![CellData::zeroed(); TAB_BAR_ROWS * cols];
        for cell in &mut batch {
            cell.bg = bar_bg;
            cell.fg = fg;
            let (u0, v0, u1, v1) = space.uv();
            cell.glyph_uv_min = [u0, v0];
            cell.glyph_uv_max = [u1, v1];
            cell.glyph_size = [space.width as f32, space.height as f32];
            cell.glyph_offset = [space.offset_x, space.offset_y];
        }

        // Same col layout as tab_at_point: starts at col 1, span = chars + 2,
        // col += span + 1, so hover and click land on the same cells.
        let mut col = 1usize;
        for tab in tabs {
            if col >= cols {
                break;
            }
            let title = truncate_title(&tab.title, 20);
            let span = tab_span(&tab.title, 20);
            let end = (col + span).min(cols);

            let (pill, title_fg, title_attrs) = if tab.active {
                (active_bg, accent_fg, 0)
            } else if tab.hovered {
                (hover_bg, fg, 0)
            } else {
                // Inactive titles: full fg passed to the shader, dimmed there
                // (ATTR_DIM = 0x10) so they recede without muddying colors.
                (bar_bg, fg, ATTR_DIM)
            };

            // Pill background. Row 0 carries the pill + title; row 1 is a
            // "connected" underline that only the active tab extends to the
            // bottom edge — the strip reads as ONE bar with an active
            // indicator instead of two stacked rows ("double tab bar").
            for r in 0..TAB_BAR_ROWS {
                if r == 1 && !tab.active {
                    continue;
                }
                let base = r * cols;
                for cell in batch.iter_mut().take(base + end).skip(base + col) {
                    cell.bg = pill;
                    cell.attrs = if r == 0 { title_attrs } else { 0 };
                }
            }

            // Title text on row 0.
            for (k, ch) in title.chars().enumerate() {
                let c = col + 1 + k;
                if c >= cols {
                    break;
                }
                let g = self.glyph_atlas.get_or_insert_glyph(ch, &self.device, &self.queue);
                let (u0, v0, u1, v1) = g.uv();
                let cell = &mut batch[c];
                cell.glyph_uv_min = [u0, v0];
                cell.glyph_uv_max = [u1, v1];
                cell.glyph_size = [g.width as f32, g.height as f32];
                cell.glyph_offset = [g.offset_x, g.offset_y];
                cell.fg = title_fg;
                cell.attrs = title_attrs;
            }

            // Close button in the right padding cell: always on the active
            // tab, on hover for the rest.
            if tab.active || tab.hovered {
                let close_c = col + span - 1;
                if close_c < cols {
                    let g = self.glyph_atlas.get_or_insert_glyph('×', &self.device, &self.queue);
                    let (u0, v0, u1, v1) = g.uv();
                    let cell = &mut batch[close_c];
                    cell.glyph_uv_min = [u0, v0];
                    cell.glyph_uv_max = [u1, v1];
                    cell.glyph_size = [g.width as f32, g.height as f32];
                    cell.glyph_offset = [g.offset_x, g.offset_y];
                    cell.fg = if tab.close_hovered { close_red } else { accent_fg };
                    cell.attrs = 0;
                }
            }
            col += span + 1;
        }

        self.tab_bar_pass.write_cells(&self.queue, &batch);

        let uniforms = Uniforms {
            screen_size: [self.size.width as f32, self.size.height as f32],
            cell_size: [self.cell_width, self.cell_height],
            viewport_origin: [0.0, 0.0],
            cols: cols as u32,
            rows: TAB_BAR_ROWS as u32,
            premultiply: self.premultiply_output,
            _pad: 0,
        };
        self.tab_bar_pass.write_uniforms(&self.queue, &uniforms);

        let view = self.current_view.as_ref();
        let Some(view) = view else {
            return Ok(());
        };
        let encoder = self.current_encoder.as_mut().unwrap();
        self.tab_bar_pass.draw(
            encoder,
            &self.render_pipeline,
            &self.atlas_bind_group,
            &self.quad_vertex_buffer,
            view,
            "Tab Bar Render Pass",
            (TAB_BAR_ROWS * cols) as u32,
            None,
        );

        Ok(())
    }

    /// Height of the tab bar in pixels (two cell rows). main.rs uses this for
    /// the content viewport offset; must stay in sync with draw_tab_bar.
    pub fn tab_bar_height(&self) -> f32 {
        TAB_BAR_ROWS as f32 * self.cell_height
    }

    /// Height of the status bar in pixels (one cell row).
    pub fn status_bar_height(&self) -> f32 {
        STATUS_BAR_ROWS as f32 * self.cell_height
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
            let (u0, v0, u1, v1) = space.uv();
            cell.glyph_uv_min = [u0, v0];
            cell.glyph_uv_max = [u1, v1];
            cell.glyph_size = [space.width as f32, space.height as f32];
            cell.glyph_offset = [space.offset_x, space.offset_y];
        }

        let title = truncate_title(left, cols.saturating_sub(2));
        for (k, ch) in title.chars().enumerate() {
            let c = 1 + k;
            if c >= cols {
                break;
            }
            let g = self.glyph_atlas.get_or_insert_glyph(ch, &self.device, &self.queue);
            let (u0, v0, u1, v1) = g.uv();
            batch[c].glyph_uv_min = [u0, v0];
            batch[c].glyph_uv_max = [u1, v1];
            batch[c].glyph_size = [g.width as f32, g.height as f32];
            batch[c].glyph_offset = [g.offset_x, g.offset_y];
        }
        let mut c = cols.saturating_sub(1);
        for ch in right.chars().rev() {
            let g = self.glyph_atlas.get_or_insert_glyph(ch, &self.device, &self.queue);
            let (u0, v0, u1, v1) = g.uv();
            batch[c].glyph_uv_min = [u0, v0];
            batch[c].glyph_uv_max = [u1, v1];
            batch[c].glyph_size = [g.width as f32, g.height as f32];
            batch[c].glyph_offset = [g.offset_x, g.offset_y];
            if c == 0 {
                break;
            }
            c -= 1;
        }

        self.status_bar_pass.write_cells(&self.queue, &batch);
        let uniforms = Uniforms {
            screen_size: [self.size.width as f32, self.size.height as f32],
            cell_size: [self.cell_width, self.cell_height],
            viewport_origin: [0.0, self.size.height as f32 - self.cell_height],
            cols: cols as u32,
            rows: 1,
            premultiply: self.premultiply_output,
            _pad: 0,
        };
        self.status_bar_pass.write_uniforms(&self.queue, &uniforms);

        let view = self.current_view.as_ref();
        let Some(view) = view else {
            return Ok(());
        };
        let encoder = self.current_encoder.as_mut().unwrap();
        self.status_bar_pass.draw(
            encoder,
            &self.render_pipeline,
            &self.atlas_bind_group,
            &self.quad_vertex_buffer,
            view,
            "Status Bar Render Pass",
            cols as u32,
            None,
        );
        Ok(())
    }


    /// Right-edge scrollbar overlay over the active pane. `fraction` is
    /// scroll_offset/max_scroll_offset (0.0 = oldest, 1.0 = newest) and
    /// `thumb_fraction` is visible_rows / total_rows; the thumb height is
    /// proportional to it so deep scrollback shows a small thumb.
    pub fn draw_scrollbar(
        &mut self,
        vx: f32,
        vy: f32,
        vw: f32,
        vh: f32,
        fraction: f32,
        thumb_fraction: f32,
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
        // Muted thumb: accent blended toward the surface so the bar reads as
        // a scrollbar, not a bright accent strip (a full-height bright-blue
        // bar on the right edge was the top complaint from screenshots).
        let thumb: [f32; 4] = [
            (t.accent.r as f32 * 0.45 + t.surface.r as f32 * 0.55) / 255.0,
            (t.accent.g as f32 * 0.45 + t.surface.g as f32 * 0.55) / 255.0,
            (t.accent.b as f32 * 0.45 + t.surface.b as f32 * 0.55) / 255.0,
            1.0,
        ];
        let space = self
            .glyph_atlas
            .get_or_insert_glyph(' ', &self.device, &self.queue);

        // 1 cell wide (was 2): reads as a slim scrollbar instead of a block.
        let bar_x = vx + vw - self.cell_width;
        let bar_y = vy + self.padding[0];
        let bar_h = (vh - self.padding[0] - self.padding[2]).max(0.0);
        // Clamp to the storage buffer's row capacity (never overrun).
        let rows = ((bar_h / self.cell_height).floor() as usize).min(SCROLLBAR_MAX_ROWS);
        if rows == 0 {
            return Ok(());
        }
        let mut batch = vec![CellData::zeroed(); rows];
        for cell in &mut batch {
            cell.bg = track;
            let (u0, v0, u1, v1) = space.uv();
            cell.glyph_uv_min = [u0, v0];
            cell.glyph_uv_max = [u1, v1];
            cell.glyph_size = [space.width as f32, space.height as f32];
        }

        // Thumb height follows the viewport's share of total content, and its
        // position follows the scroll fraction (0 = oldest, 1 = newest). The
        // old math was inverted: at the top of scrollback it sized the thumb
        // to the full track, painting a solid accent bar down the right edge.
        let (tstart, thumb_rows) = scrollbar_thumb(rows, thumb_fraction, fraction);
        for row in batch.iter_mut().take((tstart + thumb_rows).min(rows)).skip(tstart) {
            row.bg = thumb;
        }

        self.scrollbar_pass.write_cells(&self.queue, &batch);
        let uniforms = Uniforms {
            screen_size: [self.size.width as f32, self.size.height as f32],
            cell_size: [self.cell_width, self.cell_height],
            viewport_origin: [bar_x, bar_y],
            cols: 1,
            rows: rows as u32,
            premultiply: self.premultiply_output,
            _pad: 0,
        };
        self.scrollbar_pass.write_uniforms(&self.queue, &uniforms);

        let view = self.current_view.as_ref();
        let Some(view) = view else {
            return Ok(());
        };
        let encoder = self.current_encoder.as_mut().unwrap();
        self.scrollbar_pass.draw(
            encoder,
            &self.render_pipeline,
            &self.atlas_bind_group,
            &self.quad_vertex_buffer,
            view,
            "Scrollbar Render Pass",
            rows as u32,
            None,
        );
        Ok(())
    }

    fn update_cell_data(
        &mut self,
        screen: &CoreScreen,
        scroll_offset: usize,
        selection: Option<Selection>,
    ) -> Result<()> {
        // The screen -> CellData mapping (scrollback math, capacity clamp,
        // cursor/selection, syntax colors, block overlays) lives in the pure
        // CellBatch module so it's unit-testable without a GPU. Here we only
        // supply the glyph provider (the one GPU-touching dependency) and
        // upload the result.
        let device = &self.device;
        let queue = &self.queue;
        let atlas = &mut self.glyph_atlas;
        let mut batch = CellBatch::build(
            screen,
            scroll_offset,
            selection,
            self.content_pass.capacity(),
            &self.dirty_cells,
            self.blink_visible,
            self.opacity as f32,
            &self.theme,
            |ch| {
                let g = atlas.get_or_insert_glyph(ch, device, queue);
                let (u0, v0, u1, v1) = g.uv();
                GlyphQuad {
                    uv_min: [u0, v0],
                    uv_max: [u1, v1],
                    size: [g.width as f32, g.height as f32],
                    offset: [g.offset_x, g.offset_y],
                }
            },
        );

        // [ZTDIAG] Content-pass red-cell test: paint every cell bright red so a
        // framebuffer dump can tell whether the content pass draws at all.
        if std::env::var("ZTDIAG_CELLTEST").is_ok() {
            for cell in batch.iter_mut() {
                cell.bg = [1.0, 0.1, 0.1, 1.0];
                cell.glyph_size = [0.0, 0.0];
                cell.glyph_uv_min = [0.0, 0.0];
                cell.glyph_uv_max = [0.0, 0.0];
            }
        }
        if std::env::var("ZTDIAG_CELLDUMP").is_ok() {
            // Find batch rows that carry actual glyph ink (scrollback offset can
            // shift the prompt away from its screen row) and dump the first one.
            let cols = screen.buffer().first().map_or(0, Vec::len);
            let visible_rows = batch.len().checked_div(cols).unwrap_or(0);
            let mut rows_with_ink = Vec::new();
            for r in 0..visible_rows {
                let base = r * cols;
                if (0..cols).any(|c| batch.get(base + c).is_some_and(|cd| cd.glyph_size[0] > 0.0)) {
                    rows_with_ink.push(r);
                }
            }
            eprintln!("[ZTDIAG] batch rows-with-glyphs: {:?} (of {} rows)", rows_with_ink, visible_rows);
            if let Some(&r) = rows_with_ink.first() {
                let mut out = format!("[ZTDIAG] batch row {r} (cols={cols} rows={visible_rows}):\n");
                for c in 0..cols.min(40) {
                    if let Some(cd) = batch.get(r * cols + c) {
                        if cd.glyph_size[0] > 0.0 {
                            out.push_str(&format!(
                                "  c{c}: uv=({:.3},{:.3})-({:.3},{:.3}) sz=({:.1},{:.1}) off=({:.1},{:.1})\n",
                                cd.glyph_uv_min[0], cd.glyph_uv_min[1], cd.glyph_uv_max[0], cd.glyph_uv_max[1],
                                cd.glyph_size[0], cd.glyph_size[1], cd.glyph_offset[0], cd.glyph_offset[1]
                            ));
                        }
                    }
                }
                eprintln!("{out}");
            }
        }

        self.content_pass.write_cells(&self.queue, &batch);

        Ok(())
    }


/// Theme background in LINEAR color space, for wgpu `LoadOp::Clear`.
/// Theme colors are sRGB byte values (`0..255`) and the Bgra8UnormSrgb
/// surface re-encodes anything cleared with them, so passing the sRGB
/// floats straight through yields a lighter shade than the background quad
/// (faint horizontal bands between cell rows on screen). Clearing with the
/// true linear value makes every region identical to the painted background.
fn theme_linear_bg(theme: &crate::theme::Theme) -> [f64; 3] {
    let srgb = |c: u8| f64::from(c) / 255.0;
    let to_linear = |c: f64| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [
        to_linear(srgb(theme.bg.r)),
        to_linear(srgb(theme.bg.g)),
        to_linear(srgb(theme.bg.b)),
    ]
}

    pub fn reload_config(&mut self, config: &zeroterm_config::Config) {
        self.theme = crate::theme::Theme::by_name(&config.colors.theme);
        self.clear_color = Self::theme_linear_bg(&self.theme);
        // Go through set_opacity (not a field write): crossing the 1.0 boundary
        // must reconfigure the surface alpha mode, or window.opacity < 1.0
        // keeps rendering on an Opaque surface and never shows the desktop.
        self.set_opacity(config.window.opacity);
        if config.window.blur {
            static BLUR_NOTE: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !BLUR_NOTE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!(
                    "background blur is a compositor-side feature (KDE 'Blur' effect, Hyprland \
                     'windowrule=blur,class:.*'); ZeroTerm renders a transparent window and \
                     lets the compositor blur whatever is actually behind it"
                );
            }
        }
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
        self.clear_color = Self::theme_linear_bg(&self.theme);
    }

    pub fn set_opacity(&mut self, opacity: f64) {
        let prev = self.opacity;
        self.opacity = opacity;
        // Crossing the fully-opaque boundary requires reconfiguring the
        // surface: an Opaque alpha mode silently drops alpha bytes.
        if (prev < 1.0) != (opacity < 1.0) {
            self.config.alpha_mode = if opacity < 1.0 {
                self.transparent_alpha_mode
            } else {
                self.opaque_alpha_mode
            };
            self.premultiply_output =
                u32::from(self.config.alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied);
            self.surface.configure(&self.device, &self.config);
        }
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
    /// Returns the instant the cursor should next toggle visibility. Call
    /// once per frame; when that instant has passed, toggles the blink phase
    /// and returns now. The caller redraws at that cadence (2/s) and sleeps
    /// until it — this replaces a busy loop that requested a redraw on every
    /// vsync (60fps of full-cell GPU writes while idle).
    pub fn blink_next(&mut self) -> std::time::Instant {
        if !self.cursor_blink_enabled {
            self.blink_visible = true;
            self.cursor_blink = false;
            // Far future: no blink-driven redraws; events alone drive frames.
            return std::time::Instant::now() + std::time::Duration::from_secs(3600);
        }
        let elapsed = self.blink_last_toggle.elapsed();
        if elapsed >= self.blink_interval {
            self.blink_visible = !self.blink_visible;
            self.cursor_blink = self.blink_visible;
            self.blink_last_toggle = std::time::Instant::now();
            std::time::Instant::now()
        } else {
            self.blink_last_toggle + self.blink_interval
        }
    }

    /// Apply a config font change (file and/or size) at runtime. Cell metrics
    /// are recomputed from the atlas, the GPU cell grid is rebuilt for the new
    /// cell size, and the caller re-lays-out panes and redraws. Returns the
    /// error on a failed load/parse so the caller keeps its font_path/size
    /// bookkeeping in sync with what actually rendered (a rejected font must
    /// not trigger a pane re-layout at metrics that were never applied).
    pub fn reload_font(
        &mut self,
        font_path: Option<String>,
        font_size: f32,
    ) -> Result<()> {
        self.glyph_atlas
            .set_font(&self.device, &self.queue, font_path, font_size)?;
        let (cw, ch) = self.glyph_atlas.cell_metrics();
        self.cell_width = cw;
        self.cell_height = ch;
        self.cell_size = [cw, ch];
        // The GPU cell grid (content_pass capacity + shader cols/rows) was
        // sized from the OLD cell metrics; rebuild it for the new size so
        // render_screen never clamps a grid whose shape just changed.
        self.resize(self.size.width, self.size.height);
        Ok(())
    }

}

/// Compute the scrollbar thumb geometry: returns `(start_row, row_count)`
/// for a track of `rows` rows. `thumb_fraction` is the viewport's share of
/// the total content (visible_rows / total_rows); the thumb height is
/// proportional to it, so deep scrollback shows a small thumb. `fraction`
/// is the scroll position (0 = oldest content, 1 = newest); the thumb
/// starts at `fraction * (rows - thumb_rows)` so it tracks the scrollbar.
fn scrollbar_thumb(rows: usize, thumb_fraction: f32, fraction: f32) -> (usize, usize) {
    if rows == 0 {
        return (0, 0);
    }
    let fraction = fraction.clamp(0.0, 1.0);
    let min_thumb = rows.min(2);
    let thumb_rows = ((rows as f32 * thumb_fraction.clamp(0.0, 1.0)).round() as usize)
        .clamp(min_thumb, rows);
    let max_start = rows.saturating_sub(thumb_rows);
    let tstart = ((fraction * max_start as f32).round() as usize).min(max_start);
    (tstart, thumb_rows)
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

    #[test]
    fn scrollbar_thumb_never_paints_a_full_height_bar_at_the_top() {
        // 40-row track, viewport is the whole content (thumb_fraction 1.0).
        // A full-height thumb is the old inverted-math bug; height must be
        // viewport-proportional, clamped to the track.
        let (start, len) = scrollbar_thumb(40, 1.0, 0.0);
        assert_eq!(start, 0);
        assert_eq!(len, 40);

        // Deep scrollback: viewport is 1/4 of content -> thumb 1/4 of track,
        // positioned at the top when at the oldest content (fraction 0.0).
        let (start, len) = scrollbar_thumb(40, 0.25, 0.0);
        assert_eq!(len, 10);
        assert_eq!(start, 0);

        // Same deep scrollback, scrolled to the newest content (fraction 1.0)
        // -> thumb at the bottom of the track.
        let (start, len) = scrollbar_thumb(40, 0.25, 1.0);
        assert_eq!(len, 10);
        assert_eq!(start, 30);

        // Mid-scroll: thumb starts partway down, never overflowing the track.
        let (start, len) = scrollbar_thumb(40, 0.25, 0.5);
        assert_eq!(len, 10);
        assert_eq!(start, 15);

        // Tiny thumb is clamped to a minimum of 2 rows so it stays visible.
        let (start, len) = scrollbar_thumb(40, 0.001, 0.0);
        assert_eq!(len, 2);
        assert_eq!(start, 0);

        // Track too small to hold a 2-row min: full track is fine.
        let (start, len) = scrollbar_thumb(1, 0.25, 0.0);
        assert_eq!(len, 1);
        assert_eq!(start, 0);

        // Fraction and thumb_fraction are clamped; no panics, no overrun.
        let (start, len) = scrollbar_thumb(10, 2.0, 2.0);
        assert_eq!(len, 10);
        assert_eq!(start, 0);
        let (start, len) = scrollbar_thumb(10, 0.5, -1.0);
        assert_eq!(len, 5);
        assert_eq!(start, 0);
    }
}
