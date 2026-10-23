//! Manual label hot-path benchmark: cargo test --profile fast -p figstudio
//! labels::bench::timing -- --ignored --nocapture
use super::tests::{at_y, document, node};
use super::*;
use std::time::Instant;

pub(super) fn workload() -> Document {
    let mut nodes = vec![node(1, 0, "CANVAS", "page", 0.0, 0.0)];
    for row in 0..50 {
        let section = 2 + row * 101;
        nodes.push(at_y(
            node(section, 1, "SECTION", "Section", 0.0, 80_000.0),
            f64::from(row) * 180.0,
        ));
        for column in 0..100 {
            nodes.push(at_y(
                node(
                    section + 1 + column,
                    section,
                    "FRAME",
                    "Synthetic frame label",
                    f64::from(column) * 800.0,
                    600.0,
                ),
                70.0,
            ));
        }
    }
    document(nodes)
}

#[test]
#[ignore = "manual timing; no wall-clock assertion"]
fn timing() {
    let doc = workload();
    let ctx = egui::Context::default();
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(12_000.0, 8_000.0));
    for zoom in [0.25, 0.5, 1.0] {
        let mut samples = Vec::new();
        let mut count = 0;
        let mut digest = 0_u64;
        for run in 0..6 {
            let mut elapsed = std::time::Duration::ZERO;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let start = Instant::now();
                for _ in 0..20 {
                    let ls = labels(
                        ui.painter(),
                        &doc,
                        Guid(9, 1),
                        View {
                            pan: Vec2::new(0.0, 100.0),
                            zoom,
                        },
                        Pos2::ZERO,
                        screen,
                    );
                    count = ls.len();
                    digest = ls
                        .iter()
                        .fold(0, |a, l| a.wrapping_mul(31).wrapping_add(u64::from(l.id.1)));
                    paint(ui.painter(), &ls);
                    for l in &ls {
                        std::hint::black_box(rect(ui.painter(), l));
                    }
                    std::hint::black_box(&ls);
                }
                elapsed = start.elapsed();
            });
            output.textures_delta.clear();
            if run > 0 {
                samples.push(elapsed.as_secs_f64() * 1000.0 / 20.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "zoom={zoom} labels={count} digest={digest} ms/call median={:.6} range={:.6}..{:.6} samples={samples:?}",
            samples[2], samples[0], samples[4]
        );
    }
}
