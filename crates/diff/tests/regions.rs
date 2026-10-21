use figstudio_diff::{MAX_REGIONS, find_regions};

const W: usize = 600;
const H: usize = 400;

fn mask(blobs: &[(usize, usize, usize, usize)]) -> Vec<bool> {
    let mut m = vec![false; W * H];
    for &(x, y, w, h) in blobs {
        for yy in y..y + h {
            for xx in x..x + w {
                m[yy * W + xx] = true;
            }
        }
    }
    m
}

#[test]
fn separate_blobs_are_separate_regions_largest_first() {
    let r = find_regions(&mask(&[(10, 10, 10, 10), (300, 200, 20, 20)]), W, H);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].pixels_changed, 400);
    assert_eq!((r[0].x, r[0].y), (276, 176));
    assert_eq!((r[0].width, r[0].height), (68, 68));
    assert_eq!((r[1].x, r[1].y, r[1].pixels_changed), (0, 0, 100));
}

#[test]
fn blobs_within_padding_merge() {
    let r = find_regions(&mask(&[(100, 100, 10, 10), (130, 100, 10, 10)]), W, H);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].pixels_changed, 200);
    assert_eq!((r[0].x, r[0].width), (76, 88));
}

#[test]
fn specks_under_the_noise_floor_are_dropped() {
    assert!(find_regions(&mask(&[(50, 50, 7, 9)]), W, H).is_empty());
    assert!(find_regions(&[], 0, 0).is_empty());
    assert!(find_regions(&[], usize::MAX, 2).is_empty());
}

#[test]
fn at_most_twelve_regions_deterministically() {
    let blobs: Vec<_> = (0..14)
        .map(|i| (10 + (i % 7) * 80, 10 + (i / 7) * 150, 10, 10))
        .collect();
    let m = mask(&blobs);
    let r = find_regions(&m, W, H);
    assert_eq!(r.len(), MAX_REGIONS);
    assert_eq!(r, find_regions(&m, W, H));
}
