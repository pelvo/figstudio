//! Reading archives piece by piece (`archive`, `open_lazy`) against reading them whole.

use std::io::{Cursor, Read, Write};
use std::path::PathBuf;

use figstudio_fig::{Document, archive};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(format!("{name}.fig"))
}

const FIXTURES: [&str; 7] = [
    "broken_images",
    "corners",
    "noise_and_grain",
    "sections",
    "stacks_wrap",
    "structure",
    "vector",
];

/// Every entry, as the zip crate reads it: (name, method, compressed size, size, contents).
fn zip_crate_entries(bytes: &[u8]) -> Vec<(String, u16, u64, u64, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..z.len())
        .map(|i| {
            let mut e = z.by_index(i).unwrap();
            let method = match e.compression() {
                zip::CompressionMethod::Stored => 0,
                zip::CompressionMethod::Deflated => 8,
                m => panic!("unexpected method {m:?}"),
            };
            let (name, cs, size) = (e.name().to_string(), e.compressed_size(), e.size());
            let mut data = Vec::new();
            e.read_to_end(&mut data).unwrap();
            (name, method, cs, size, data)
        })
        .collect()
}

fn ours(bytes: &[u8]) -> Vec<(String, u16, u64, u64, Vec<u8>)> {
    let mut r = Cursor::new(bytes);
    archive::read_entries(&mut r, bytes.len() as u64)
        .unwrap()
        .iter()
        .map(|e| {
            let data = archive::read_entry(&mut r, e).unwrap();
            (e.name.clone(), e.method, e.compressed_size, e.size, data)
        })
        .collect()
}

#[test]
fn entries_match_the_zip_crate_for_every_fixture() {
    for name in FIXTURES {
        let bytes = std::fs::read(fixture(name)).unwrap();
        assert_eq!(ours(&bytes), zip_crate_entries(&bytes), "{name}");
    }
}

#[test]
fn reads_deflated_zip64_and_commented_archives() {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let big = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("images/aa", big).unwrap();
    z.write_all(&[7u8; 5000]).unwrap();
    z.start_file("meta.json", stored).unwrap();
    z.write_all(br#"{"file_name":"x"}"#).unwrap();
    z.set_comment("a comment at the end of the archive")
        .unwrap();
    let bytes = z.finish().unwrap().into_inner();
    let got = ours(&bytes);
    assert_eq!(got, zip_crate_entries(&bytes));
    assert_eq!(got[0].1, 8, "first entry is deflated");
    assert_eq!(got[0].4, vec![7u8; 5000]);
}

#[test]
fn rejects_truncated_archives() {
    let bytes = std::fs::read(fixture("structure")).unwrap();
    let cut = &bytes[..bytes.len() - 10];
    assert!(archive::read_entries(&mut Cursor::new(cut), cut.len() as u64).is_err());
    assert!(archive::central_directory(b"not a zip", 9).is_err());
}

#[test]
fn open_lazy_matches_open_but_reads_no_images() {
    for name in FIXTURES {
        let path = fixture(name);
        let whole = Document::open(&path).unwrap();
        let lazy = Document::open_lazy(&path).unwrap();
        assert_eq!(whole.nodes.len(), lazy.nodes.len(), "{name}");
        for (a, b) in whole.nodes.iter().zip(&lazy.nodes) {
            assert_eq!(a.data, b.data, "{name}: node {}", a.id);
        }
        assert_eq!(whole.pages, lazy.pages, "{name}");
        assert_eq!(whole.blobs, lazy.blobs, "{name}");
        assert_eq!(whole.file_name, lazy.file_name, "{name}");
        assert_eq!(whole.thumbnail, lazy.thumbnail, "{name}");
        assert!(lazy.images.is_empty(), "{name}: images were read");
        let mut want: Vec<_> = whole.images.keys().cloned().collect();
        let mut got: Vec<_> = lazy.lazy_images.keys().cloned().collect();
        want.sort();
        got.sort();
        assert_eq!(got, want, "{name}");
        // Each listed entry reads back as the image `open` loaded.
        let mut file = std::fs::File::open(&path).unwrap();
        for (hash, entry) in &lazy.lazy_images {
            assert_eq!(
                archive::read_entry(&mut file, entry).unwrap(),
                whole.images[hash],
                "{name}: {hash}"
            );
        }
        assert!(
            lazy.to_bytes().is_err(),
            "{name}: a lazy document was written"
        );
    }
}
