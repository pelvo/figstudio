//! Closing a document while its image read is in flight stores nothing for it.

use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use super::*;
use crate::source::ImageSource;

/// Answers each read only when the test sends the bytes.
struct Gate(Mutex<Receiver<Vec<u8>>>, SyncSender<()>);

impl ImageSource for Gate {
    fn read(&self, _: &figstudio_fig::archive::Entry) -> anyhow::Result<Vec<u8>> {
        let _ = self.1.send(());
        let rx = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(rx.recv()?)
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
            crc32: 0,
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

/// Blocks inside shader evaluation so the test can queue `Msg::Shaders` and `close_document`
/// before the render worker drains the channel.
struct BlockingShader {
    started: SyncSender<()>,
    release: Mutex<Receiver<()>>,
}

impl figstudio_render::shader::ShaderFills for BlockingShader {
    fn fill(
        &self,
        _: &figstudio_fig::Document,
        _: &figstudio_fig::Val,
        _: u16,
        _: u16,
        _: f64,
    ) -> figstudio_render::shader::Shaded {
        let _ = self.started.send(());
        let rx = self.release.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = rx.recv();
        figstudio_render::shader::Shaded::Unavailable
    }

    fn has_effect(&self, _: &figstudio_fig::Document, _: &figstudio_fig::Val) -> bool {
        false
    }

    fn effect(
        &self,
        _: &figstudio_fig::Document,
        _: &figstudio_fig::Val,
        _: &vello_gpu::Pixmap,
        _: f64,
    ) -> figstudio_render::shader::Shaded {
        figstudio_render::shader::Shaded::Unavailable
    }
}

fn shader_fixture() -> Arc<Document> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/structure.fig"
    );
    let mut doc = Document::open(path).unwrap();
    let page = doc.pages[0];
    let mut nodes = doc.nodes.clone();
    let target = doc.children(page)[0];
    let rect = nodes.iter_mut().find(|n| n.id == target).unwrap();
    let mut paint = figstudio_fig::Obj::new();
    paint.insert("type", figstudio_fig::Val::Str("CUSTOM".into()));
    paint.insert("visible", figstudio_fig::Val::Bool(true));
    rect.data.set(
        "fillPaints",
        figstudio_fig::Val::Arr(vec![figstudio_fig::Val::Obj(paint)]),
    );
    doc.set_nodes(nodes).unwrap();
    Arc::new(doc)
}

#[test]
fn a_queued_shader_host_after_close_is_not_stored() {
    let mut workers = Workers::new(&egui::Context::default());
    let blocker = shader_fixture();
    let (started_tx, started_rx) = sync_channel(1);
    let (release_tx, release_rx) = sync_channel(1);
    workers.images().set_shaders(
        blocker.lineage,
        Arc::new(BlockingShader {
            started: started_tx,
            release: Mutex::new(release_rx),
        }),
    );
    let bounds = figstudio_render::content_bounds(&blocker, blocker.pages[0]).unwrap();
    let key = tiles::Range::covering(0, bounds).by_distance(blocker.instance, blocker.pages[0])[0];
    workers
        .render(RenderJob {
            tab: 1,
            doc: blocker.clone(),
            source: None,
            root: blocker.pages[0],
            background: None,
            images: 0,
            tiles: VecDeque::from([key]),
            prototype: Default::default(),
            layers: Vec::new(),
            received: Vec::new(),
        })
        .unwrap();
    started_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("render worker never entered blocking shader");

    // Queue a shader host for another document, then close it before the worker drains.
    let closed_lineage = 0x77;
    let host: Arc<dyn figstudio_render::shader::ShaderFills> =
        Arc::new(figstudio_shader::Fills::new(Vec::new()));
    workers.set_shaders(closed_lineage, host.clone());
    workers.close_document(closed_lineage);

    release_tx.send(()).unwrap();
    workers
        .render_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("blocking tile never finished");
    // Render one tiny non-blocking tile so the worker has definitely drained all prior messages.
    let flush_doc = Arc::new(
        Document::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/structure.fig"
        ))
        .unwrap(),
    );
    let flush_key = tiles::Key {
        doc: flush_doc.instance,
        page: flush_doc.pages[0],
        level: 0,
        x: 0,
        y: 0,
    };
    workers
        .render(RenderJob {
            tab: 2,
            doc: flush_doc.clone(),
            source: None,
            root: flush_doc.pages[0],
            background: None,
            images: 0,
            tiles: VecDeque::from([flush_key]),
            prototype: Default::default(),
            layers: Vec::new(),
            received: Vec::new(),
        })
        .unwrap();
    workers
        .render_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("flush tile never finished");

    assert_eq!(
        Arc::strong_count(&host),
        1,
        "queued Msg::Shaders resurrected a shader host after close_document"
    );
}
