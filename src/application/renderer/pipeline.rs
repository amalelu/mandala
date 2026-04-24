//! One-time device / surface / shader / pipeline-layout setup.
//! Lifted verbatim from `renderer/mod.rs` so the frame-hot code
//! isn't interleaved with startup scaffolding. Every item here runs
//! once per process (adapter + device + queue + compiled shader
//! modules), never per frame.

use std::borrow::Cow;

use log::debug;
use rustc_hash::FxHashMap;
use wgpu::{
    Adapter, Device, Instance, MultisampleState, PipelineLayout, Queue, RenderPipeline,
    ShaderModule, Surface, SurfaceCapabilities, SurfaceConfiguration, TextureFormat,
};
use winit::dpi::PhysicalSize;

use baumhard::shaders::shaders::SHADERS;

use super::Renderer;

impl Renderer {
    pub(super) fn create_surface_config(
        texture_format: TextureFormat,
        surface_capabilities: &SurfaceCapabilities,
        surface_size: PhysicalSize<u32>,
    ) -> SurfaceConfiguration {
        SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: texture_format,
            width: surface_size.width,
            height: surface_size.height,
            present_mode: wgpu::PresentMode::Fifo,
            // Interactive-first latency budget. wgpu's default is 2,
            // which at 60Hz bakes ~33ms of input-to-photon queueing
            // into every frame (and ~12ms at 165Hz — the user-visible
            // asymmetry that made rapid drags feel laggy on 60Hz
            // monitors while feeling fine on 165Hz). A single queued
            // frame still lets the GPU overlap with the CPU but caps
            // the backlog at one refresh interval.
            desired_maximum_frame_latency: 1,
            alpha_mode: surface_capabilities.alpha_modes[0],
            view_formats: vec![],
        }
    }

    #[inline]
    pub(super) fn create_render_pipeline(
        device: &Device,
        shader: &ShaderModule,
        pipeline_layout: &PipelineLayout,
        texture_format: TextureFormat,
    ) -> RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(texture_format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    #[inline]
    pub(super) fn load_shaders(device: &Device, shaders: &mut FxHashMap<&'static str, ShaderModule>) {
        assert!(SHADERS.len() > 0, "No shaders defined!");
        for i in 0..SHADERS.len() {
            let (name, source) = SHADERS[i].clone();
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
            });
            shaders.insert(name, shader);
            debug!("Loaded a shader");
        }
    }

    #[inline]
    pub(super) fn create_pipeline_layout(device: &Device) -> PipelineLayout {
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[],
            immediate_size: 0,
        })
    }

    #[inline]
    pub(super) async fn get_device(adapter: &Adapter) -> (Device, Queue) {
        adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults()
                        .using_resolution(adapter.limits()),
                    memory_hints: Default::default(),
                    trace: Default::default(),
                    experimental_features: Default::default(),
                },
            )
            .await
            .expect("Failed to create device")
    }

    #[inline]
    pub(super) async fn get_adapter(instance: &Instance, surface: &Surface<'static>) -> Adapter {
        instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .expect("Failed to find an appropriate adapter")
    }
}
