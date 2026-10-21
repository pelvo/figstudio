use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Source {
    calls: AtomicUsize,
    transparent: bool,
    side: u16,
}

impl EmojiSource for Source {
    fn raster(&self, _: &str, _: u16) -> Option<EmojiRaster> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        let side = self.side.max(1);
        EmojiRaster::from_rgba(
            [
                100,
                200,
                40,
                if self.transparent && call == 0 {
                    0
                } else {
                    128
                },
            ]
            .repeat(usize::from(side).pow(2)),
            side,
            side,
            0.0,
            -1.0,
        )
    }
}

#[test]
fn transparent_first_raster_does_not_permanently_hide_the_cluster() {
    let source = Arc::new(Source {
        transparent: true,
        ..Source::default()
    });
    let mut cache = Cache::new(Some(source.clone()));
    assert!(cache.raster("❤", 40).is_none());
    assert!(cache.raster("❤", 40).is_some());
    assert!(cache.raster("❤", 40).is_some());
    assert_eq!(source.calls.load(Ordering::Relaxed), 2);
}

#[test]
fn reduced_platform_ppem_is_preserved_for_draw_scaling() {
    struct Reduced;
    impl EmojiSource for Reduced {
        fn raster(&self, _: &str, _: u16) -> Option<EmojiRaster> {
            EmojiRaster::from_rgba(vec![255; 16], 2, 2, -1.0, -3.0)?.with_ppem(16)
        }
    }
    let mut cache = Cache::new(Some(Arc::new(Reduced)));
    for requested in [40, 4096, u16::MAX] {
        let Some((crate::emoji::Glyph::Bitmap { ppem, x, y, .. }, _)) =
            cache.raster("❤", requested)
        else {
            panic!("must rasterize")
        };
        assert_eq!(ppem, 16);
        assert_eq!((x, y), (-1.0, 1.0));
    }
}

#[test]
fn rgba_is_premultiplied_once_before_recording() {
    let source = Source::default();
    let image = source.raster("❤", 40).unwrap().image;
    assert_eq!(image.data_as_u8_slice(), &[50, 100, 20, 128]);
}

#[test]
fn cache_pressure_preserves_recent_clusters_and_rejects_oversized_keys() {
    for (side, count) in [(1, 256), (512, 16)] {
        let source = Arc::new(Source {
            side,
            ..Source::default()
        });
        let mut cache = Cache::new(Some(source.clone()));
        for i in 0..count {
            assert!(cache.raster(&i.to_string(), 40).is_some());
        }
        assert!(cache.raster("0", 40).is_some()); // refresh the oldest entry
        assert!(cache.raster("new", 40).is_some());
        assert!(cache.raster("0", 40).is_some());
        assert_eq!(
            source.calls.load(Ordering::Relaxed),
            count + 1,
            "recent entry retained"
        );
        assert!(cache.raster("1", 40).is_some());
        assert_eq!(
            source.calls.load(Ordering::Relaxed),
            count + 2,
            "oldest entry evicted"
        );
        assert!(cache.raster(&"x".repeat(1025), 40).is_none());
        assert_eq!(source.calls.load(Ordering::Relaxed), count + 2);
    }
}
