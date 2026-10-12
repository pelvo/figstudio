//! Closing a document while its image read is in flight stores nothing for it.

use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use super::*;
use crate::source::ImageSource;

/// Answers each read only when the test sends the bytes.
struct Gate(Mutex<Receiver<Vec<u8>>>, SyncSender<()>);

impl ImageSource for Gate {
    fn read(&self, _: &figstudio_fig::archive::Entry) -> Result<Vec<u8>, String> {
        let _ = self.1.send(());
        let rx = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        rx.recv().map_err(|e| e.to_string())
    }
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn request(instance: u64) -> ImageRequest {
    ImageRequest {
        hash: "ab".into(),
        key: format!("{instance:x}/ab"),
        level: 0,
        entry: figstudio_fig::archive::Entry {
            name: "images/ab".into(),
            method: 0,
            compressed_size: 0,
            size: 0,
            header_offset: 0,
        },
    }
}

fn settle(workers: &Workers) {
    let start = Instant::now();
    while workers.in_flight.load(Ordering::Relaxed) > 0 {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "read never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn an_image_read_finishing_after_its_document_closed_is_not_stored() {
    let workers = Workers::new(&egui::Context::default());
    let (bytes_tx, bytes_rx) = sync_channel(1);
    let (started_tx, started_rx) = sync_channel(1);
    let source: Source = Arc::new(Gate(Mutex::new(bytes_rx), started_tx));
    // Control: an open document stores the image.
    workers.queue.push(vec![request(0x51)], &source);
    started_rx.recv().unwrap();
    bytes_tx.send(png()).unwrap();
    settle(&workers);
    assert!(workers.images().bytes() > 0, "control image not stored");
    workers.close_document(0x51);
    assert_eq!(workers.images().bytes(), 0);
    // Closed while its read is in flight: nothing comes back.
    workers.queue.push(vec![request(0x52)], &source);
    started_rx.recv().unwrap();
    workers.close_document(0x52);
    bytes_tx.send(png()).unwrap();
    settle(&workers);
    assert_eq!(
        workers.images().bytes(),
        0,
        "a closed document's image was stored"
    );
    assert_eq!(workers.images().in_flight(), 0);
}
