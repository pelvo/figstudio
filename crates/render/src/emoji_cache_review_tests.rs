use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Capped(AtomicUsize);
impl EmojiSource for Capped {
    fn raster_size(&self, requested: u16) -> u16 {
        requested.min(32)
    }
    fn raster(&self, _: &str, requested: u16) -> Option<EmojiRaster> {
        self.0.fetch_add(1, Ordering::Relaxed);
        EmojiRaster::from_rgba(vec![255; 16], 2, 2, 0.0, -2.0)?.with_ppem(requested.min(32))
    }
}

#[test]
fn capped_source_reuses_one_raster_across_every_larger_zoom() {
    let source = Arc::new(Capped(AtomicUsize::new(0)));
    let mut cache = Cache::new(Some(source.clone()));
    for pixels in 33..133 {
        assert!(cache.raster("❤", pixels).is_some());
    }
    let calls = source.0.load(Ordering::Relaxed);
    eprintln!("100 zoom requests above cap: {calls} rasters");
    assert_eq!(calls, 1);
}

#[test]
fn permanent_misses_are_bounded_but_still_periodically_retried() {
    struct Missing(AtomicUsize);
    impl EmojiSource for Missing {
        fn raster(&self, _: &str, _: u16) -> Option<EmojiRaster> {
            self.0.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
    let source = Arc::new(Missing(AtomicUsize::new(0)));
    let mut cache = Cache::new(Some(source.clone()));
    for _ in 0..1000 {
        assert!(cache.raster("❤", 40).is_none());
    }
    let calls = source.0.load(Ordering::Relaxed);
    eprintln!("1000 permanent-miss requests: {calls} rasters");
    assert!(
        (2..=18).contains(&calls),
        "misses must back off, not become permanent or busy-loop"
    );
}

#[test]
fn oversized_raster_draws_uncached_without_evicting_hot_entries() {
    struct Sized(AtomicUsize);
    impl EmojiSource for Sized {
        fn raster(&self, cluster: &str, _: u16) -> Option<EmojiRaster> {
            self.0.fetch_add(1, Ordering::Relaxed);
            let side = if cluster == "large" { 2 } else { 1 };
            EmojiRaster::from_rgba(
                vec![255; usize::from(side).pow(2) * 4],
                side,
                side,
                0.0,
                -1.0,
            )
        }
    }
    let source = Arc::new(Sized(AtomicUsize::new(0)));
    let mut cache = Cache::new(Some(source.clone()));
    cache.byte_budget = 4;
    assert!(cache.raster("small", 40).is_some());
    for _ in 0..2 {
        assert!(
            cache.raster("large", 40).is_some(),
            "draw oversize uncached"
        );
    }
    assert!(cache.raster("small", 40).is_some());
    assert_eq!(source.0.load(Ordering::Relaxed), 3);
}
