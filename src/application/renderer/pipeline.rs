// SPDX-License-Identifier: MPL-2.0

//! Once-per-process device / surface setup.

use wgpu::{
    Adapter, Device, Instance, Queue, Surface, SurfaceCapabilities, SurfaceConfiguration, TextureFormat,
};
use winit::dpi::PhysicalSize;

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
            // `Auto` is wgpu 30's default and reproduces the color
            // space every earlier wgpu picked implicitly for the
            // chosen format: sRGB for the sRGB format `Renderer::new`
            // prefers, and the historical choice for its `formats[0]`
            // fallback. The canvas renders as it did before the field
            // existed.
            color_space: wgpu::SurfaceColorSpace::Auto,
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
    pub(super) async fn get_device(adapter: &Adapter) -> (Device, Queue) {
        adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                memory_hints: Default::default(),
                trace: Default::default(),
                experimental_features: Default::default(),
            })
            .await
            .expect("Failed to create device")
    }

    #[inline]
    pub(super) async fn get_adapter(instance: &Instance, surface: &Surface<'static>) -> Adapter {
        instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(surface),
                // Limit bucketing is a fingerprinting defence for
                // hosts that expose wgpu to untrusted content. This
                // app runs only its own code, so it keeps the
                // adapter's real limits, as before the option existed.
                apply_limit_buckets: false,
            })
            .await
            .expect("Failed to find an appropriate adapter")
    }
}
