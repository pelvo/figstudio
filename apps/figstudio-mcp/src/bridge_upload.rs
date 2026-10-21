use crate::live::{Snapshot, TabInfo};
use figstudio_fig::Document;
use figstudio_mcp_core::bridge::{TabState, parse_guid};
use std::{path::PathBuf, sync::Arc};
use tokio::io::AsyncWriteExt;

struct Upload {
    tab: u64,
    path: PathBuf,
    file: tokio::fs::File,
    expected: u64,
    received: u64,
}

pub(crate) struct Session {
    pub snapshot: Snapshot,
    dir: tempfile::TempDir,
    upload: Option<Upload>,
    serial: u64,
}
impl Session {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            snapshot: Snapshot::default(),
            dir: tempfile::tempdir().map_err(|_| "cannot create upload directory")?,
            upload: None,
            serial: 0,
        })
    }
    pub async fn open(
        &mut self,
        tab: u64,
        name: String,
        size: u64,
        cap: u64,
    ) -> Result<(), String> {
        self.discard().await;
        if size > cap {
            return Err("upload exceeds byte limit".into());
        }
        if name.is_empty()
            || name.len() > 255
            || name.contains(['/', '\\', ':'])
            || name.chars().any(char::is_control)
            || name == "."
            || name == ".."
        {
            return Err("invalid filename".into());
        }
        self.serial += 1;
        let dir = self.dir.path().join(self.serial.to_string());
        tokio::fs::create_dir(&dir)
            .await
            .map_err(|_| "cannot create upload directory")?;
        let path = dir.join(if name.ends_with(".fig") {
            name
        } else {
            format!("{name}.fig")
        });
        let file = tokio::fs::File::create(&path)
            .await
            .map_err(|_| "cannot create upload")?;
        self.upload = Some(Upload {
            tab,
            path,
            file,
            expected: size,
            received: 0,
        });
        Ok(())
    }
    pub fn uploading(&self) -> Option<u64> {
        self.upload.as_ref().map(|upload| upload.tab)
    }
    pub async fn bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        let upload = self.upload.as_mut().ok_or("binary frame without upload")?;
        if bytes.len() as u64 > upload.expected.saturating_sub(upload.received) {
            self.discard().await;
            return Err("upload exceeds declared size".into());
        }
        upload
            .file
            .write_all(bytes)
            .await
            .map_err(|_| "cannot write upload")?;
        upload.received += bytes.len() as u64;
        Ok(())
    }
    pub async fn end(&mut self, tab: u64) -> Result<(), String> {
        let Some(mut upload) = self.upload.take() else {
            return Err("no upload in progress".into());
        };
        if upload.tab != tab || upload.received != upload.expected {
            drop(upload.file);
            let _ = tokio::fs::remove_file(upload.path).await;
            return Err("incomplete upload".into());
        }
        upload
            .file
            .flush()
            .await
            .map_err(|_| "cannot finish upload")?;
        drop(upload.file);
        let path = upload.path.clone();
        let result = tokio::task::spawn_blocking(move || Document::open_auto(path)).await;
        let doc = match result {
            Ok(Ok(doc)) => Arc::new(doc),
            _ => {
                let _ = tokio::fs::remove_file(upload.path).await;
                return Err("invalid fig file".into());
            }
        };
        let page = doc
            .pages
            .first()
            .copied()
            .unwrap_or(figstudio_fig::Guid(0, 0));
        self.snapshot.tabs.retain(|old| old.id != tab);
        self.snapshot.tabs.push(TabInfo {
            id: tab,
            path: upload.path,
            doc,
            page,
            selection: None,
            viewport: None,
        });
        self.snapshot.active = Some(tab);
        Ok(())
    }
    pub fn state(&mut self, tabs: Vec<TabState>, active: Option<u64>) -> Result<(), String> {
        let mut next = self.snapshot.clone();
        for state in tabs {
            let tab = next
                .tabs
                .iter_mut()
                .find(|tab| tab.id == state.tab)
                .ok_or("unknown tab")?;
            if let Some(page) = state.page {
                let page = parse_guid(&page).map_err(|_| "invalid page id")?;
                if !tab.doc.pages.contains(&page) && !tab.doc.internal_pages.contains(&page) {
                    return Err("unknown page".into());
                }
                tab.page = page;
            }
            tab.selection = state
                .selection
                .map(|id| parse_guid(&id))
                .transpose()
                .map_err(|_| "invalid selection id")?;
            if tab.selection.is_some_and(|id| tab.doc.node(id).is_none()) {
                return Err("unknown selection".into());
            }
            if state.viewport.is_some_and(|rect| {
                rect.iter().any(|v| !v.is_finite()) || rect[2] < 0. || rect[3] < 0.
            }) {
                return Err("invalid viewport".into());
            }
            tab.viewport = state.viewport;
        }
        if active.is_some_and(|id| !next.tabs.iter().any(|tab| tab.id == id)) {
            return Err("unknown active tab".into());
        }
        next.active = active;
        self.snapshot = next;
        Ok(())
    }
    pub async fn close(&mut self, tab: u64) {
        if self.upload.as_ref().is_some_and(|upload| upload.tab == tab) {
            self.discard().await;
        }
        self.snapshot.tabs.retain(|old| old.id != tab);
        if self.snapshot.active == Some(tab) {
            self.snapshot.active = None;
        }
    }
    async fn discard(&mut self) {
        if let Some(upload) = self.upload.take() {
            drop(upload.file);
            let _ = tokio::fs::remove_file(upload.path).await;
        }
    }
}
