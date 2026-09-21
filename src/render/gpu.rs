use anyhow::{Context, Result};
use wgpu;

pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuContext {
    /// Default backend priority: platform-native rasterizers that render
    /// headless reliably (Metal on macOS, Vulkan/DX12 elsewhere).
    const DEFAULT_BACKENDS: wgpu::Backends =
        wgpu::Backends::METAL.union(wgpu::Backends::VULKAN).union(wgpu::Backends::DX12);

    pub fn new(backend: Option<&str>) -> Result<Self> {
        pollster::block_on(Self::init_async(backend))
    }

    fn parse_backends(spec: &str) -> Result<wgpu::Backends> {
        let mode = spec.to_ascii_lowercase();
        let backends = match mode.as_str() {
            "" => Self::DEFAULT_BACKENDS,
            "auto" => Self::DEFAULT_BACKENDS,
            "metal" => wgpu::Backends::METAL,
            "vulkan" => wgpu::Backends::VULKAN,
            "dx12" => wgpu::Backends::DX12,
            "gl" => wgpu::Backends::GL,
            "webgpu" => wgpu::Backends::BROWSER_WEBGPU,
            other => {
                anyhow::bail!(
                    "Unknown --backend '{other}'. Valid: auto, metal, vulkan, dx12, gl, webgpu"
                )
            }
        };

        Ok(backends)
    }

    async fn init_async(backend: Option<&str>) -> Result<Self> {
        let backend_spec = backend.unwrap_or("auto");
        let backends = Self::parse_backends(backend_spec)
            .with_context(|| format!("Invalid GPU backend '{}'", backend_spec))?;

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .with_context(|| {
                format!(
                    "No GPU adapter for --backend '{backend_spec}'. Try 'auto' or another backend."
                )
            })?;

        let info = adapter.get_info();
        log::info!("Using GPU: {} ({:?})", info.name, info.backend);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("sonica_device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await
            .context("Failed to create GPU device")?;

        Ok(Self { device, queue })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_parsing_is_strict() {
        assert_eq!(
            GpuContext::parse_backends("auto").unwrap(),
            GpuContext::DEFAULT_BACKENDS
        );
        assert_eq!(GpuContext::parse_backends("Metal").unwrap(), wgpu::Backends::METAL);
        assert_eq!(GpuContext::parse_backends("").unwrap(), GpuContext::DEFAULT_BACKENDS);
        assert_eq!(GpuContext::parse_backends("gl").unwrap(), wgpu::Backends::GL);
        assert!(GpuContext::parse_backends("cuda").is_err());
    }
}
