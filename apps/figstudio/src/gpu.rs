//! eframe's wgpu setup: egui-wgpu's defaults, but asking only for limits the adapter has.
//!
//! egui-wgpu always requests `max_texture_dimension_2d = 8192`, so an adapter that offers
//! less (WebGL2 or weak GL drivers often stop at 4096) fails to start. glow took whatever
//! the driver had; this keeps that behaviour.

use std::sync::Arc;

use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;

/// egui-wgpu's adapter selection and backends, with [`required_limits`] for the device.
pub fn setup() -> WgpuSetup {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.device_descriptor = Arc::new(|adapter| wgpu::DeviceDescriptor {
        label: Some("egui wgpu device"),
        required_limits: required_limits(
            adapter.get_info().backend == wgpu::Backend::Gl,
            &adapter.limits(),
        ),
        ..Default::default()
    });
    setup.into()
}

/// egui-wgpu's requested limits (WebGL2 downlevel on GL, else the WebGPU defaults, with
/// 8192 px textures for 4K+ screens), each lowered to what the adapter supports.
fn required_limits(gl: bool, adapter: &wgpu::Limits) -> wgpu::Limits {
    let base = if gl {
        wgpu::Limits::downlevel_webgl2_defaults()
    } else {
        wgpu::Limits::default()
    };
    wgpu::Limits {
        max_texture_dimension_2d: 8192,
        ..base
    }
    .or_worse_values_from(adapter)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter_with_side(side: u32) -> wgpu::Limits {
        wgpu::Limits {
            max_texture_dimension_2d: side,
            ..wgpu::Limits::downlevel_webgl2_defaults()
        }
    }

    #[test]
    fn a_4096_adapter_gets_a_4096_request() {
        let adapter = adapter_with_side(4096);
        let want = required_limits(true, &adapter);
        assert_eq!(want.max_texture_dimension_2d, 4096);
        assert!(
            want.check_limits(&adapter),
            "every limit is within the adapter's"
        );
    }

    #[test]
    fn a_large_adapter_keeps_egui_wgpus_8192() {
        let adapter = wgpu::Limits {
            max_texture_dimension_2d: 16384,
            ..wgpu::Limits::default()
        };
        assert_eq!(
            required_limits(false, &adapter).max_texture_dimension_2d,
            8192
        );
        assert_eq!(
            required_limits(true, &adapter).max_texture_dimension_2d,
            8192
        );
    }
}
