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

/// egui-wgpu's requested limits (WebGL2 downlevel on GL, else the WebGPU defaults), each
/// lowered to what the adapter supports, but with the adapter's full texture size (egui-wgpu
/// asks for 8192), since the window surface can be no larger than that.
fn required_limits(gl: bool, adapter: &wgpu::Limits) -> wgpu::Limits {
    let base = if gl {
        wgpu::Limits::downlevel_webgl2_defaults()
    } else {
        wgpu::Limits::default()
    };
    wgpu::Limits {
        max_texture_dimension_2d: adapter.max_texture_dimension_2d,
        ..base
    }
    .or_worse_values_from(adapter)
}

/// The largest window or canvas side, in points (CSS px on the web), whose size in pixels at
/// `ppp` pixels per point stays within the GPU's `max_side`. eframe configures the surface at
/// the window's pixel size, and wgpu panics on a surface larger than the device allows.
pub fn max_points(max_side: usize, ppp: f32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "texture sides are far below 2^24"
    )]
    let side = max_side as f32;
    (side / ppp.max(f32::MIN_POSITIVE)).floor()
}

/// Desktop: caps the window's inner size so its pixels stay within the GPU limit. Re-sent when
/// the limit or pixels per point (screen scale, egui zoom) change; `applied` remembers the last
/// cap. egui-winit turns the points into physical pixels with the same pixels per point
/// (egui-winit 0.36 `process_viewport_command`, `ViewportCommand::MaxInnerSize`).
#[cfg(not(target_arch = "wasm32"))]
pub fn cap_window(ctx: &egui::Context, applied: &mut Option<f32>) {
    let (side, ppp) = ctx.input(|i| (i.max_texture_side, i.pixels_per_point));
    let max = max_points(side, ppp);
    if *applied != Some(max) {
        *applied = Some(max);
        ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(egui::vec2(max, max)));
    }
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
    fn a_large_adapter_gets_its_full_texture_size() {
        let adapter = wgpu::Limits {
            max_texture_dimension_2d: 16384,
            ..wgpu::Limits::default()
        };
        for gl in [false, true] {
            let want = required_limits(gl, &adapter);
            assert_eq!(want.max_texture_dimension_2d, 16384);
            assert!(want.check_limits(&adapter));
        }
    }

    #[test]
    fn the_window_in_pixels_stays_within_the_texture_limit() {
        for ppp in [1.0, 1.25, 1.5, 2.0, 2.625, 3.0, 4.0] {
            let points = max_points(4096, ppp);
            let pixels = (points * ppp).round();
            assert!(pixels <= 4096.0, "ppp {ppp}: {points} pt is {pixels} px");
            assert!(
                pixels > 4096.0 - 2.0 * ppp,
                "ppp {ppp}: cap {points} pt is too tight"
            );
        }
    }
}
