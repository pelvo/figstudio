//! Regression tests for lazy `.fig` archive saving (desktop streaming and browser zero-copy
//! planning): preserving untouched entries without decompressing or materializing all images.

use std::io::{Cursor, Read, Write};
use std::path::PathBuf;

use figstudio_fig::edit::{Txn, execute_import_image};
use figstudio_fig::{Document, Val};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// Builds a synthetic `.fig` archive larger than `archive::TAIL_LEN` with:
/// - `canvas.fig`, `meta.json`, `thumbnail.png` from `structure.fig`
/// - a deflated image entry (`images/<deflated_hash>`)
/// - a stored image entry (`images/<stored_hash>`)
/// - an empty 0-byte image entry (`images/<empty_hash>`)
/// - a custom non-image entry (`custom/extra.txt`)
fn synthetic_archive_bytes() -> (Vec<u8>, String, Vec<u8>, String, Vec<u8>, String) {
    let base = Document::open(fixture("structure.fig")).expect("open structure.fig");
    let canvas = base.encode_canvas().expect("encode canvas");
    let thumb = base.thumbnail.clone().expect("structure.fig has thumbnail");

    let deflated_data: Vec<u8> = b"DEFLATED-IMAGE-PAYLOAD-0123456789abcdef"
        .iter()
        .copied()
        .cycle()
        .take(96 * 1024)
        .collect();
    let deflated_hash = figstudio_fig::sha1::sha1_hex(&deflated_data);

    let stored_data = vec![0x5Au8; 80 * 1024];
    let stored_hash = figstudio_fig::sha1::sha1_hex(&stored_data);

    let empty_hash = "00".repeat(20);

    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    z.start_file("canvas.fig", stored).unwrap();
    z.write_all(&canvas).unwrap();
    z.start_file("meta.json", stored).unwrap();
    z.write_all(br#"{"file_name":"Synthetic Lazy"}"#).unwrap();
    z.start_file("thumbnail.png", stored).unwrap();
    z.write_all(&thumb).unwrap();
    z.start_file("custom/extra.txt", deflated).unwrap();
    z.write_all(b"preserve custom archive metadata verbatim")
        .unwrap();
    z.add_directory("images/", stored).unwrap();
    z.start_file(format!("images/{deflated_hash}"), deflated)
        .unwrap();
    z.write_all(&deflated_data).unwrap();
    z.start_file(format!("images/{stored_hash}"), stored)
        .unwrap();
    z.write_all(&stored_data).unwrap();
    z.start_file(format!("images/{empty_hash}"), stored)
        .unwrap();

    let bytes = z.finish().unwrap().into_inner();
    assert!(
        bytes.len() as u64 > figstudio_fig::archive::TAIL_LEN,
        "archive must exceed TAIL_LEN so tail does not cover early entry payloads"
    );
    (
        bytes,
        deflated_hash,
        deflated_data,
        stored_hash,
        stored_data,
        empty_hash,
    )
}

#[test]
fn desktop_lazy_save_preserves_deflated_entries_custom_files_empty_images_and_overrides() {
    let (bytes, deflated_hash, deflated_data, stored_hash, _stored_data, empty_hash) =
        synthetic_archive_bytes();
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("source.fig");
    std::fs::write(&src, &bytes).unwrap();

    let mut doc = Document::open_lazy(&src).expect("open_lazy");
    assert!(doc.is_lazy());
    assert!(doc.images.is_empty());

    // Edit a node, override an existing image hash, and import a brand-new image.
    let page = doc.pages[0];
    let target = doc.children(page)[0];
    let mut txn = Txn::new("rename");
    txn.set_field(target, "name", Val::Str("Lazy Saved Layer".into()));
    doc.apply_txn(&txn).unwrap();

    let replacement_bytes = b"replaced-stored-image".to_vec();
    doc.images
        .insert(stored_hash.clone(), replacement_bytes.clone());

    let mut history = figstudio_fig::edit::History::new(4);
    let new_img_bytes = b"brand-new-imported-image".to_vec();
    let (new_hash, _) =
        execute_import_image(&mut doc, &mut history, new_img_bytes.clone()).unwrap();

    let dst = dir.path().join("copy.fig");
    doc.save_copy_checked(Some(&src), &dst, false, true)
        .expect("save_copy_checked on lazy doc");

    // Verify ZIP-level invariants: untouched deflated entry stayed Deflated (not inflated and
    // re-stored uncompressed), custom entry and empty image survived, no duplicates.
    let saved_file = std::fs::File::open(&dst).unwrap();
    let mut zip = zip::ZipArchive::new(saved_file).unwrap();
    assert!(
        !zip.has_duplicate_names(),
        "saved archive must not contain duplicate entry names"
    );
    {
        let mut entry = zip
            .by_name(&format!("images/{deflated_hash}"))
            .expect("deflated image preserved");
        assert_eq!(
            entry.compression(),
            zip::CompressionMethod::Deflated,
            "untouched deflated image must stay compressed rather than being materialized uncompressed"
        );
        let mut out = Vec::new();
        entry.read_to_end(&mut out).unwrap();
        assert_eq!(out, deflated_data);
    }
    {
        let mut entry = zip
            .by_name("custom/extra.txt")
            .expect("custom non-image entry preserved");
        let mut out = Vec::new();
        entry.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"preserve custom archive metadata verbatim");
    }

    let reopened = Document::open(&dst).expect("reopen saved copy");
    assert_eq!(reopened.node(target).unwrap().name, "Lazy Saved Layer");
    assert_eq!(reopened.file_name.as_deref(), Some("Synthetic Lazy"));
    assert_eq!(reopened.images.get(&deflated_hash), Some(&deflated_data));
    assert_eq!(reopened.images.get(&stored_hash), Some(&replacement_bytes));
    assert_eq!(
        reopened.images.get(&empty_hash).map(Vec::len),
        Some(0),
        "0-byte image entry must survive lazy save"
    );
    assert_eq!(reopened.images.get(&new_hash), Some(&new_img_bytes));
}

/// Assembles `ArchivePart`s by copying `Bytes` and slicing `SourceRange` out of `source`
/// (the exact operation browser `Blob` / `FileSystemWritableFileStream` performs).
fn assemble_parts(source: &[u8], parts: &[figstudio_fig::lazy_save::ArchivePart]) -> Vec<u8> {
    use figstudio_fig::lazy_save::ArchivePart;
    let mut out = Vec::new();
    for part in parts {
        match part {
            ArchivePart::Bytes(b) => out.extend_from_slice(b),
            ArchivePart::SourceRange { offset, len } => {
                let start = usize::try_from(*offset).unwrap();
                let end = start + usize::try_from(*len).unwrap();
                out.extend_from_slice(&source[start..end]);
            }
        }
    }
    out
}

/// Opens `bytes` through the exact browser lazy path (`archive::central_directory` +
/// `archive::entries` + `EAGER_ENTRIES` + `Document::from_lazy`).
fn open_browser_lazy(bytes: &[u8]) -> Document {
    use figstudio_fig::archive;
    let file_len = bytes.len() as u64;
    let tail_len = archive::TAIL_LEN.min(file_len) as usize;
    let tail = &bytes[bytes.len() - tail_len..];
    let (cd_offset, cd_size) = archive::central_directory(tail, file_len).unwrap();
    let cd = &bytes[cd_offset as usize..(cd_offset + cd_size) as usize];
    let entries = archive::entries(cd).unwrap();
    let mut cursor = Cursor::new(bytes);
    let mut raw = std::collections::HashMap::new();
    for e in entries
        .iter()
        .filter(|e| archive::EAGER_ENTRIES.contains(&e.name.as_str()))
    {
        raw.insert(e.name.clone(), archive::read_raw(&mut cursor, e).unwrap());
    }
    let eager = archive::decode_raw(&entries, raw).unwrap();
    Document::from_lazy(entries, eager).expect("from_lazy")
}

/// Runs the metadata-only browser lazy plan without ever passing untouched entry payloads.
fn plan_browser_lazy(
    source: &[u8],
    doc: &Document,
) -> Result<Vec<figstudio_fig::lazy_save::ArchivePart>, figstudio_fig::FigError> {
    use figstudio_fig::archive;
    use figstudio_fig::lazy_save::LazyArchiveMeta;

    let file_len = source.len() as u64;
    let tail_len = archive::TAIL_LEN.min(file_len) as usize;
    let tail = source[source.len() - tail_len..].to_vec();
    let (cd_offset, cd_size) = archive::central_directory(&tail, file_len)?;
    let cd = source[cd_offset as usize..(cd_offset + cd_size) as usize].to_vec();

    let meta = LazyArchiveMeta::from_directory(file_len, tail, cd_offset, cd)?;
    let offsets = meta.needed_local_headers(doc)?;
    let mut headers = Vec::with_capacity(offsets.len());
    for off in offsets {
        let start = usize::try_from(off).unwrap();
        let hdr: [u8; 30] = source[start..start + 30].try_into().unwrap();
        headers.push((off, hdr));
    }
    meta.plan(doc, &headers)
}

#[test]
fn browser_lazy_plan_rebuilds_archive_without_reading_or_decompressing_payloads() {
    use figstudio_fig::lazy_save::ArchivePart;

    let (source, deflated_hash, deflated_data, stored_hash, stored_data, empty_hash) =
        synthetic_archive_bytes();
    let mut doc = open_browser_lazy(&source);
    assert!(doc.is_lazy());
    assert!(doc.images.is_empty());

    // Edit a node, keep both untouched images (one deflated, one stored) plus the 0-byte image
    // and custom file, and import one brand-new image.
    let page = doc.pages[0];
    let target = doc.children(page)[0];
    let mut txn = Txn::new("browser edit");
    txn.set_field(target, "name", Val::Str("Browser Lazy Edited".into()));
    doc.apply_txn(&txn).unwrap();

    let mut history = figstudio_fig::edit::History::new(4);
    let new_img_bytes = b"browser-imported-image-bytes".to_vec();
    let (new_hash, _) =
        execute_import_image(&mut doc, &mut history, new_img_bytes.clone()).unwrap();

    // Even `to_bytes()` remains rejected for lazy documents; the browser path uses `LazyArchiveMeta`.
    assert!(doc.to_bytes().is_err());

    let parts = plan_browser_lazy(&source, &doc).expect("plan lazy archive");
    let source_ranges: Vec<(u64, u64)> = parts
        .iter()
        .filter_map(|p| match p {
            ArchivePart::SourceRange { offset, len } => Some((*offset, *len)),
            ArchivePart::Bytes(_) => None,
        })
        .collect();
    assert!(
        source_ranges.len() >= 3,
        "expected SourceRange parts for meta.json, custom file, deflated image, and stored image; got {source_ranges:?}"
    );
    let materialized_bytes: usize = parts
        .iter()
        .filter_map(|p| match p {
            ArchivePart::Bytes(b) => Some(b.len()),
            ArchivePart::SourceRange { .. } => None,
        })
        .sum();
    assert!(
        materialized_bytes < stored_data.len(),
        "untouched 80 KiB image must stay a SourceRange, not materialized in Bytes ({materialized_bytes} B)"
    );

    let rebuilt = assemble_parts(&source, &parts);
    let mut zip = zip::ZipArchive::new(Cursor::new(&rebuilt)).expect("valid rebuilt zip");
    assert!(!zip.has_duplicate_names());
    {
        let mut entry = zip
            .by_name(&format!("images/{deflated_hash}"))
            .expect("deflated image");
        assert_eq!(entry.compression(), zip::CompressionMethod::Deflated);
        let mut got = Vec::new();
        entry.read_to_end(&mut got).unwrap();
        assert_eq!(got, deflated_data);
    }
    {
        let mut entry = zip.by_name("custom/extra.txt").expect("custom entry");
        let mut got = Vec::new();
        entry.read_to_end(&mut got).unwrap();
        assert_eq!(got, b"preserve custom archive metadata verbatim");
    }

    let reopened = Document::from_bytes(&rebuilt).expect("reopen rebuilt archive");
    assert_eq!(reopened.node(target).unwrap().name, "Browser Lazy Edited");
    assert_eq!(reopened.file_name.as_deref(), Some("Synthetic Lazy"));
    assert_eq!(reopened.images.get(&deflated_hash), Some(&deflated_data));
    assert_eq!(reopened.images.get(&stored_hash), Some(&stored_data));
    assert_eq!(reopened.images.get(&empty_hash).map(Vec::len), Some(0));
    assert_eq!(reopened.images.get(&new_hash), Some(&new_img_bytes));
}

#[test]
fn browser_lazy_plan_handles_small_fixtures_and_zip64_commented_archives() {
    // 1. Small fixture where `tail` covers the entire archive: armed payload ranges must still
    //    produce `SourceRange` entries and round-trip cleanly.
    let small_src = std::fs::read(fixture("structure.fig")).unwrap();
    let mut small_doc = open_browser_lazy(&small_src);
    let expected_images = Document::from_bytes(&small_src).unwrap().images;
    let parts = plan_browser_lazy(&small_src, &small_doc).expect("plan small fixture");
    let rebuilt = assemble_parts(&small_src, &parts);
    let reopened = Document::from_bytes(&rebuilt).expect("reopen small fixture");
    assert_eq!(reopened.images, expected_images);
    assert_eq!(reopened.thumbnail, small_doc.thumbnail);

    // 2. ZIP64 (`large_file(true)`) archive with an archive comment and an image override.
    let canvas = small_doc.encode_canvas().unwrap();
    let img_a = vec![0xABu8; 4096];
    let hash_a = figstudio_fig::sha1::sha1_hex(&img_a);
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let z64_deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let z64_stored = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    z.start_file("canvas.fig", z64_stored).unwrap();
    z.write_all(&canvas).unwrap();
    z.start_file(format!("images/{hash_a}"), z64_deflated)
        .unwrap();
    z.write_all(&img_a).unwrap();
    z.set_comment("commented zip64 fixture").unwrap();
    let z64_bytes = z.finish().unwrap().into_inner();

    let mut z64_doc = open_browser_lazy(&z64_bytes);
    let z64_parts = plan_browser_lazy(&z64_bytes, &z64_doc).expect("plan zip64");
    let z64_rebuilt = assemble_parts(&z64_bytes, &z64_parts);
    let z64_reopened = Document::from_bytes(&z64_rebuilt).expect("reopen zip64");
    assert_eq!(z64_reopened.images.get(&hash_a), Some(&img_a));

    // Overriding `hash_a` in `z64_doc.images` replaces the old entry without duplicates.
    let override_bytes = b"overridden-z64-image".to_vec();
    z64_doc
        .images
        .insert(hash_a.clone(), override_bytes.clone());
    let parts2 = plan_browser_lazy(&z64_bytes, &z64_doc).expect("plan zip64 override");
    let rebuilt2 = assemble_parts(&z64_bytes, &parts2);
    let zip2 = zip::ZipArchive::new(Cursor::new(&rebuilt2)).unwrap();
    assert!(!zip2.has_duplicate_names());
    let reopened2 = Document::from_bytes(&rebuilt2).unwrap();
    assert_eq!(reopened2.images.get(&hash_a), Some(&override_bytes));
    small_doc.images.clear();
}

#[test]
fn browser_lazy_plan_rejects_bad_local_headers_and_truncated_source() {
    use figstudio_fig::archive;
    use figstudio_fig::lazy_save::LazyArchiveMeta;

    let (source, ..) = synthetic_archive_bytes();
    let doc = open_browser_lazy(&source);
    let file_len = source.len() as u64;
    let tail_len = archive::TAIL_LEN.min(file_len) as usize;
    let tail = source[source.len() - tail_len..].to_vec();
    let (cd_offset, cd_size) = archive::central_directory(&tail, file_len).unwrap();
    let cd = source[cd_offset as usize..(cd_offset + cd_size) as usize].to_vec();

    let meta = LazyArchiveMeta::from_directory(file_len, tail.clone(), cd_offset, cd.clone())
        .expect("valid directory");
    let offsets = meta.needed_local_headers(&doc).unwrap();
    assert!(!offsets.is_empty());

    // Missing local headers must fail cleanly, not panic or emit empty slices.
    assert!(meta.plan(&doc, &[]).is_err());

    // Corrupted local header magic must fail cleanly.
    let bad_headers: Vec<(u64, [u8; 30])> = offsets.iter().map(|&o| (o, [0u8; 30])).collect();
    assert!(meta.plan(&doc, &bad_headers).is_err());

    // A local header claiming extra_field_length that pushes payload past `file_len` fails.
    let mut overflow_headers: Vec<(u64, [u8; 30])> = offsets
        .iter()
        .map(|&o| {
            let s = o as usize;
            (o, source[s..s + 30].try_into().unwrap())
        })
        .collect();
    if let Some((_, hdr)) = overflow_headers.last_mut() {
        hdr[28..30].copy_from_slice(&u16::MAX.to_le_bytes());
    }
    assert!(
        meta.plan(&doc, &overflow_headers).is_err(),
        "payload pushed past file_len must be rejected"
    );

    // Tail longer than file_len must fail in from_directory.
    assert!(LazyArchiveMeta::from_directory(10, tail, 0, cd).is_err());
}

fn named_synthetic_archive(
    meta_name: &str,
    img_byte: u8,
    extra_name: &str,
    extra_body: &[u8],
) -> Vec<u8> {
    let base = Document::open(fixture("structure.fig")).expect("open structure.fig");
    let canvas = base.encode_canvas().expect("encode canvas");
    let thumb = base.thumbnail.clone().expect("structure.fig has thumbnail");
    let img_data = vec![img_byte; 96 * 1024];
    let img_hash = figstudio_fig::sha1::sha1_hex(&img_data);

    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("canvas.fig", stored).unwrap();
    z.write_all(&canvas).unwrap();
    z.start_file("meta.json", stored).unwrap();
    z.write_all(format!(r#"{{"file_name":"{meta_name}"}}"#).as_bytes())
        .unwrap();
    z.start_file("thumbnail.png", stored).unwrap();
    z.write_all(&thumb).unwrap();
    z.start_file(extra_name, stored).unwrap();
    z.write_all(extra_body).unwrap();
    z.add_directory("images/", stored).unwrap();
    z.start_file(format!("images/{img_hash}"), stored).unwrap();
    z.write_all(&img_data).unwrap();
    z.finish().unwrap().into_inner()
}

#[test]
fn native_lazy_save_rejects_replaced_or_same_len_changed_source_and_protects_target() {
    let bytes_a = named_synthetic_archive("DocA", 0x11, "custom/a.txt", b"only-in-a");
    let bytes_b = named_synthetic_archive("DocB", 0x22, "custom/b.txt", b"only-in-b-longer");
    // Same length and identical entry layout as A, but different meta.json / custom bytes (different CRCs).
    let bytes_a_mutated = named_synthetic_archive("DocX", 0x11, "custom/a.txt", b"changed-a");
    assert_eq!(bytes_a.len(), bytes_a_mutated.len());

    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("source.fig");
    std::fs::write(&src, &bytes_a).unwrap();

    let mut doc = Document::open_lazy(&src).expect("open_lazy A");
    let target = doc.children(doc.pages[0])[0];
    let mut txn = Txn::new("rename A");
    txn.set_field(target, "name", Val::Str("EditedFromA".into()));
    doc.apply_txn(&txn).unwrap();

    let new_dst = dir.path().join("new_copy.fig");
    let existing_dst = dir.path().join("existing_copy.fig");
    std::fs::write(&existing_dst, b"untouched-destination").unwrap();

    // 1. Replace source path with B before saving.
    std::fs::write(&src, &bytes_b).unwrap();
    assert!(
        doc.save_copy_checked(Some(&src), &new_dst, false, true)
            .is_err(),
        "save_copy_checked must reject replaced source archive"
    );
    assert!(!new_dst.exists(), "failed lazy save must not create target");

    assert!(
        doc.save_copy_checked(Some(&src), &existing_dst, true, true)
            .is_err(),
        "overwrite save_copy_checked must reject replaced source archive"
    );
    assert_eq!(
        std::fs::read(&existing_dst).unwrap(),
        b"untouched-destination",
        "existing destination must stay untouched on failure"
    );

    assert!(
        doc.save_with_lazy_source(Some(&src), &existing_dst)
            .is_err(),
        "save_with_lazy_source must reject replaced source archive"
    );
    assert_eq!(
        std::fs::read(&existing_dst).unwrap(),
        b"untouched-destination"
    );

    // 2. Replace source in-place with same-length archive whose central-directory CRCs differ.
    std::fs::write(&src, &bytes_a_mutated).unwrap();
    assert!(
        doc.save_copy_checked(Some(&src), &new_dst, false, true)
            .is_err(),
        "same-length modified archive must be rejected"
    );
    assert!(!new_dst.exists());

    // Ensure no `.saving` temp files were left behind on failures.
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["existing_copy.fig", "source.fig"]);
}

#[test]
fn native_lazy_save_rejects_in_place_modification_during_streaming() {
    use std::io::{Seek, SeekFrom};

    struct MutateMidStream {
        buf: Vec<u8>,
        replacement: Vec<u8>,
        pos: usize,
        mutated: bool,
        payload_start: usize,
        payload_end: usize,
    }

    impl Read for MutateMidStream {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let avail = self.buf.len().saturating_sub(self.pos);
            let n = out.len().min(avail);
            out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
            let read_at = self.pos;
            self.pos += n;
            // Mutate only once `raw_copy_file` is actively streaming the image payload
            // (after pre-stream metadata checks and `ZipArchive::new` already passed).
            if !self.mutated && read_at >= self.payload_start && read_at < self.payload_end {
                self.mutated = true;
                self.buf = self.replacement.clone();
            }
            Ok(n)
        }
    }

    impl Seek for MutateMidStream {
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            let next = match from {
                SeekFrom::Start(x) => x as i128,
                SeekFrom::End(d) => self.buf.len() as i128 + i128::from(d),
                SeekFrom::Current(d) => self.pos as i128 + i128::from(d),
            };
            if next < 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "negative seek",
                ));
            }
            self.pos = usize::try_from(next).unwrap_or(usize::MAX);
            Ok(self.pos as u64)
        }
    }

    let bytes_a = named_synthetic_archive("DocA", 0x11, "custom/a.txt", b"only-in-a");
    let bytes_b = named_synthetic_archive("DocB", 0x22, "custom/b.txt", b"only-in-b");

    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("source.fig");
    std::fs::write(&src, &bytes_a).unwrap();

    let mut doc = Document::open_lazy(&src).expect("open_lazy A");
    let img_entry = doc.lazy_images.values().next().unwrap().clone();
    let payload_start = usize::try_from(img_entry.header_offset + 30).unwrap();
    let payload_end = payload_start + 8192;
    assert!(
        (payload_end as u64) < bytes_a.len() as u64 - figstudio_fig::archive::TAIL_LEN,
        "trigger range must sit strictly before archive tail"
    );
    let target = doc.children(doc.pages[0])[0];
    let mut txn = Txn::new("rename A");
    txn.set_field(target, "name", Val::Str("MidStreamEdited".into()));
    doc.apply_txn(&txn).unwrap();

    let reader = MutateMidStream {
        buf: bytes_a,
        replacement: bytes_b,
        pos: 0,
        mutated: false,
        payload_start,
        payload_end,
    };
    let res = doc.write_lazy_archive(reader, Cursor::new(Vec::new()));
    assert!(
        res.is_err(),
        "in-place archive mutation during streaming must fail rather than returning mixed archive"
    );
}

#[test]
fn browser_lazy_plan_rejects_mismatched_archive_metadata() {
    use figstudio_fig::archive;
    use figstudio_fig::lazy_save::LazyArchiveMeta;

    let bytes_a = named_synthetic_archive("DocA", 0x11, "custom/a.txt", b"only-in-a");
    let bytes_b = named_synthetic_archive("DocB", 0x22, "custom/b.txt", b"only-in-b");
    let doc_a = open_browser_lazy(&bytes_a);

    let file_len_b = bytes_b.len() as u64;
    let tail_len_b = archive::TAIL_LEN.min(file_len_b) as usize;
    let tail_b = bytes_b[bytes_b.len() - tail_len_b..].to_vec();
    let (cd_off_b, cd_size_b) = archive::central_directory(&tail_b, file_len_b).unwrap();
    let cd_b = bytes_b[cd_off_b as usize..(cd_off_b + cd_size_b) as usize].to_vec();
    let meta_b = LazyArchiveMeta::from_directory(file_len_b, tail_b, cd_off_b, cd_b).unwrap();

    assert!(meta_b.needed_local_headers(&doc_a).is_err());
    assert!(meta_b.plan(&doc_a, &[]).is_err());
}
