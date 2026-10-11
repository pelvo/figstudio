//! GPU tiles registered with eframe's egui renderer (desktop): each finished tile texture gets
//! an egui id on the UI thread, freed with its texture when the tile cache drops the tile.

use std::sync::Arc;

use eframe::egui_wgpu::RenderState;
use eframe::wgpu;

/// Frees a registered tile from egui's renderer.
pub type Free = Arc<dyn Fn(egui::TextureId) + Send + Sync>;

/// A GPU tile registered with egui; dropping it (the tile cache evicts it) frees its egui id
/// and its texture.
pub struct NativeTexture {
    pub id: egui::TextureId,
    free: Free,
    /// Held until the egui id is freed (the bind group refers to it).
    _texture: Option<wgpu::Texture>,
}

impl Drop for NativeTexture {
    fn drop(&mut self) {
        (self.free)(self.id);
    }
}

/// Registers finished GPU tiles with eframe's egui renderer (UI thread).
pub struct Registry {
    device: wgpu::Device,
    renderer: Arc<egui::mutex::RwLock<eframe::egui_wgpu::Renderer>>,
    free: Free,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry").finish_non_exhaustive()
    }
}

impl Registry {
    pub fn new(state: &RenderState) -> Self {
        let renderer = state.renderer.clone();
        let weak = Arc::downgrade(&renderer);
        Registry {
            device: state.device.clone(),
            renderer,
            free: Arc::new(move |id| {
                if let Some(r) = weak.upgrade() {
                    r.write().free_texture(&id);
                }
            }),
        }
    }

    /// Registers `texture` for egui, sampled like the CPU tiles (linear).
    pub fn register(&self, texture: wgpu::Texture) -> NativeTexture {
        let view = texture.create_view(&Default::default());
        let id = self.renderer.write().register_native_texture(
            &self.device,
            &view,
            wgpu::FilterMode::Linear,
        );
        NativeTexture {
            id,
            free: self.free.clone(),
            _texture: Some(texture),
        }
    }
}

#[cfg(test)]
impl NativeTexture {
    /// A tile whose freeing calls `free` (bookkeeping tests, no GPU).
    pub fn fake(id: u64, free: Free) -> Self {
        NativeTexture {
            id: egui::TextureId::User(id),
            free,
            _texture: None,
        }
    }
}
