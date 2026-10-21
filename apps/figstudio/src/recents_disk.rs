//! Where the desktop keeps the Recents list (`recents.rs`): `<config>/figstudio/recents.json`
//! (full paths: it is how a file is opened again) and the pictures in
//! `<config>/figstudio/thumbs/<hash of the path>.png`, both written owner-only and never
//! leaving this computer. One thread draws the pictures, another looks for files that were
//! moved or deleted while the page shows.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

use crate::recents::{Entry, Id, MAX_ENTRIES, News, ThumbJob, thumb_png};
use crate::spaces;

/// Largest list read (bytes); a bigger one is ignored, not read.
const MAX_BYTES: u64 = 1 << 20;
/// Largest picture read back (bytes).
const MAX_THUMB_BYTES: u64 = 4 << 20;
/// Seconds between looks for moved or deleted files while the page is up (at most 60
/// `stat` calls).
const RECHECK: f64 = 1.0;

/// The looking thread's two ends: looks asked for (by generation, with the paths) and
/// its answers (the generation, and the paths not found).
struct Looker {
    ask: mpsc::Sender<(u64, Vec<PathBuf>)>,
    answers: mpsc::Receiver<(u64, HashSet<PathBuf>)>,
}

/// The list's file, the pictures and the two threads.
pub(crate) struct Io {
    /// Where the list is saved; `None` keeps it in memory (tests, an unreadable list).
    file: Option<PathBuf>,
    /// Where the pictures are; `None` draws none.
    thumbs: Option<PathBuf>,
    painter: Option<mpsc::Sender<(u64, PathBuf, ThumbJob)>>,
    /// Pictures written since the last frame: their textures are loaded again.
    fresh: Arc<Mutex<Vec<PathBuf>>>,
    /// Picture files still wanted, each with its newest job: a job finishing after its file
    /// was removed from the list, or after a newer job for it, writes nothing (checked and
    /// written under this lock), so Remove sticks and the last open's page wins.
    pub(crate) wanted: Arc<Mutex<HashMap<PathBuf, u64>>>,
    /// The number the next picture job gets.
    next_job: u64,
    /// By picture file; `None` for none (yet).
    textures: HashMap<PathBuf, Option<egui::TextureHandle>>,
    /// When the page's last look for moved or deleted files returned (`egui` time).
    checked: f64,
    /// The one thread that looks for moved or deleted files (a slow disk or network mount
    /// holds it up, not the window), spawned with the first look.
    looker: Option<Looker>,
    /// The generation of the look the thread is on. No other is asked for until it
    /// answers, so a `stat` that hangs holds up that one thread and nothing piles up.
    pub(crate) asked: Option<u64>,
    /// Goes up whenever the page learns about the disk outside a look (a click, a load, a
    /// failed open) and when the page appears: an answer asked for under an older one is
    /// dropped, as it may be from before.
    generation: u64,
    /// Look as soon as the thread is free (set with each new generation).
    stale: bool,
    /// Whether a listed file is there: `Path::is_file`, as a click checks (tests put in a
    /// slow one).
    pub(crate) probe: Arc<dyn Fn(&Path) -> bool + Send + Sync>,
}

impl Io {
    /// Nothing saved, no pictures.
    pub fn memory() -> Self {
        Self {
            file: None,
            thumbs: None,
            painter: None,
            fresh: Arc::default(),
            wanted: Arc::default(),
            next_job: 0,
            textures: HashMap::new(),
            checked: 0.0,
            looker: None,
            asked: None,
            generation: 0,
            stale: false,
            probe: Arc::new(Path::is_file),
        }
    }

    /// The list of this computer, in the config folder.
    pub fn load() -> (Self, Vec<Entry>) {
        match dirs::config_dir() {
            Some(dir) => {
                let dir = dir.join("figstudio");
                Self::at(dir.join("recents.json"), Some(dir.join("thumbs")))
            }
            None => (Self::memory(), Vec::new()),
        }
    }

    /// The list saved at `file`, pictures in `thumbs`. A list that cannot be read or does
    /// not parse is left as it is, and this session's list stays in memory.
    pub fn at(file: PathBuf, thumbs: Option<PathBuf>) -> (Self, Vec<Entry>) {
        use std::io::Read as _;
        let read = std::fs::File::open(&file).and_then(|f| {
            let mut bytes = Vec::new();
            f.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(std::io::Error::other("too big"));
            }
            Ok(bytes)
        });
        let entries = match read {
            Ok(bytes) => parse(&bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Vec::new()),
            Err(_) => None,
        };
        let mut io = Self::memory();
        match entries {
            Some(entries) => {
                io.file = Some(file);
                io.thumbs = thumbs;
                (io, entries)
            }
            None => {
                tracing::warn!("recent files not read, kept in memory this time");
                (io, Vec::new())
            }
        }
    }

    pub fn save(&self, entries: &[Entry]) {
        let Some(file) = &self.file else {
            return;
        };
        match serde_json::to_vec_pretty(entries) {
            Ok(bytes) => {
                if let Err(e) = spaces::write_private(file, &bytes) {
                    tracing::warn!("recent files not saved: {e}");
                }
            }
            Err(e) => tracing::warn!("recent files not saved: {e}"),
        }
    }

    /// Where the picture of `id` is kept.
    pub fn thumb_file(&self, id: &Id) -> Option<PathBuf> {
        use sha2::Digest as _;
        let hash = sha2::Sha256::digest(id.path.to_string_lossy().as_bytes());
        let name: String = hash[..12].iter().map(|b| format!("{b:02x}")).collect();
        self.thumbs
            .as_ref()
            .map(|dir| dir.join(format!("{name}.png")))
    }

    /// `id` left the list: its picture goes too.
    pub fn forget(&mut self, id: &Id) {
        if let Some(file) = self.thumb_file(id) {
            self.textures.remove(&file);
            // Under the lock: a job writing it now finishes first, or finds it unwanted.
            let mut wanted = self.wanted.lock().unwrap_or_else(|e| e.into_inner());
            wanted.remove(&file);
            let _ = std::fs::remove_file(file);
        }
    }

    /// Draws the picture of `id` in the background (one file at a time, on one thread).
    pub fn paint(&mut self, ctx: &egui::Context, id: &Id, job: ThumbJob) {
        let Some(out) = self.thumb_file(id) else {
            return;
        };
        if self.painter.is_none() {
            let (tx, rx) = mpsc::channel::<(u64, PathBuf, ThumbJob)>();
            let fresh = self.fresh.clone();
            let wanted = self.wanted.clone();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("thumbnails".into())
                .spawn(move || {
                    for (number, out, job) in rx {
                        // As in `Workers::export`: a picture that panics is skipped, not the
                        // thread; it only reads an immutable document.
                        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            write_thumb(&job, &out, number, &wanted)
                        }));
                        match done {
                            Ok(Ok(false)) => {}
                            Ok(Ok(true)) => {
                                if let Ok(mut fresh) = fresh.lock() {
                                    fresh.push(out);
                                }
                                ctx.request_repaint();
                            }
                            Ok(Err(e)) => tracing::warn!("no picture for a recent file: {e:#}"),
                            Err(_) => tracing::warn!("no picture for a recent file: it panicked"),
                        }
                    }
                });
            if spawned.is_err() {
                return;
            }
            self.painter = Some(tx);
        }
        let number = self.want(&out);
        if let Some(tx) = &self.painter
            && tx.send((number, out, job)).is_err()
        {
            self.painter = None;
        }
    }

    /// Marks picture file `out` as wanted from a new job, and returns that job's number.
    pub(crate) fn want(&mut self, out: &Path) -> u64 {
        self.next_job += 1;
        self.wanted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(out.to_path_buf(), self.next_job);
        self.next_job
    }

    /// The texture of `id`'s picture, loading at most `budget` more this frame.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        id: &Id,
        budget: &mut usize,
    ) -> Option<egui::TextureHandle> {
        let file = self.thumb_file(id)?;
        if let Some(t) = self.textures.get(&file) {
            return t.clone();
        }
        if *budget == 0 {
            ctx.request_repaint();
            return None;
        }
        *budget -= 1;
        let texture = (|| {
            let len = std::fs::metadata(&file).ok()?.len();
            if len > MAX_THUMB_BYTES {
                return None;
            }
            let bytes = std::fs::read(&file).ok()?;
            crate::recents::png_texture(ctx, &format!("recent-{}", file.display()), &bytes)
        })();
        self.textures.insert(file, texture.clone());
        texture
    }

    /// Nothing arrives in the background but the looks (`begin_frame`).
    pub fn poll(&mut self) -> News {
        News::default()
    }

    /// The threads wake the window themselves.
    pub fn busy(&self) -> bool {
        false
    }

    /// What the page knows changed outside a look: the look running now (begun before it)
    /// is not believed, and the next one starts once the thread is free.
    pub fn changed(&mut self) {
        self.generation += 1;
        self.stale = true;
    }

    /// Called each frame the page shows: on appearing, and [`RECHECK`] seconds after the
    /// last look returned while it stays up (a frame comes with any pointer move), it looks
    /// for moved or deleted files on another thread, so a card, its label and its menu
    /// follow the disk and a slow path never holds up the window; a freshly drawn picture
    /// is loaded again.
    pub fn begin_frame(
        &mut self,
        ctx: &egui::Context,
        time: f64,
        appeared: bool,
        entries: &[Entry],
    ) -> News {
        if appeared {
            self.changed();
        }
        let mut answers = Vec::new();
        let mut gone = false;
        if let Some(looker) = &self.looker {
            loop {
                match looker.answers.try_recv() {
                    Ok(answer) => answers.push(answer),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        gone = true;
                        break;
                    }
                }
            }
        }
        let mut news = News::default();
        for (generation, missing) in answers {
            self.asked = None;
            self.checked = time;
            if generation == self.generation {
                news.missing = Some(missing.into_iter().map(Id::file).collect());
            }
        }
        if gone {
            // The thread panicked: a new one with the next look.
            self.looker = None;
            self.asked = None;
            self.checked = time;
        }
        if self.asked.is_none() && (self.stale || time - self.checked >= RECHECK) {
            self.stale = false;
            self.look(ctx, time, entries);
        }
        let fresh: Vec<PathBuf> = self
            .fresh
            .lock()
            .map(|mut f| std::mem::take(&mut *f))
            .unwrap_or_default();
        for out in fresh {
            self.textures.remove(&out);
        }
        news
    }

    /// Asks the looking thread (spawned the first time) to look for moved or deleted files.
    fn look(&mut self, ctx: &egui::Context, time: f64, entries: &[Entry]) {
        if self.looker.is_none() {
            let (ask, asks) = mpsc::channel::<(u64, Vec<PathBuf>)>();
            let (tell, answers) = mpsc::channel();
            let probe = self.probe.clone();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("recents-look".into())
                .spawn(move || {
                    for (generation, paths) in asks {
                        let missing: HashSet<PathBuf> =
                            paths.into_iter().filter(|p| !probe(p)).collect();
                        if tell.send((generation, missing)).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                });
            if spawned.is_err() {
                // No thread: try again after the usual wait, not every frame.
                self.checked = time;
                return;
            }
            self.looker = Some(Looker { ask, answers });
        }
        let paths: Vec<PathBuf> = entries.iter().map(|e| e.path.clone()).collect();
        let sent = self
            .looker
            .as_ref()
            .is_some_and(|l| l.ask.send((self.generation, paths)).is_ok());
        if sent {
            self.asked = Some(self.generation);
        } else {
            self.looker = None;
            self.checked = time;
        }
    }
}

/// The entries in a saved list, skipping damaged ones; `None` when it is not a JSON list.
fn parse(bytes: &[u8]) -> Option<Vec<Entry>> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Some(Vec::new());
    }
    let items: Vec<serde_json::Value> = serde_json::from_slice(bytes).ok()?;
    let mut entries: Vec<Entry> = items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .filter(|e: &Entry| {
            e.path.is_absolute()
                && e.handle.is_none()
                && e.key.as_ref().is_none_or(|k| k.len() <= 64)
        })
        .collect();
    entries.truncate(MAX_ENTRIES);
    Some(entries)
}

/// Renders the job's page to fit the card and saves it at `out`, if job `number` is still
/// the one `wanted` for that file (false: the file was removed, or opened again since;
/// nothing written).
pub(crate) fn write_thumb(
    job: &ThumbJob,
    out: &Path,
    number: u64,
    wanted: &Mutex<HashMap<PathBuf, u64>>,
) -> anyhow::Result<bool> {
    use anyhow::Context as _;
    let png = thumb_png(job)?;
    let wanted = wanted.lock().unwrap_or_else(|e| e.into_inner());
    if wanted.get(out) != Some(&number) {
        return Ok(false);
    }
    spaces::write_private(out, &png).context("save the picture")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recents::Recents;
    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("figstudio-recents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn settle(r: &mut Recents, ctx: &egui::Context, time: f64) {
        let waiting = r.io.asked;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while waiting.is_some() && r.io.asked == waiting {
            assert!(
                std::time::Instant::now() < deadline,
                "the look never returned"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
            r.begin_frame(ctx, time);
        }
    }

    fn held_probe(
        looks: &Arc<std::sync::atomic::AtomicUsize>,
        gate: &Arc<Mutex<()>>,
    ) -> Arc<dyn Fn(&Path) -> bool + Send + Sync> {
        let (l, g) = (looks.clone(), gate.clone());
        Arc::new(move |p: &Path| {
            let there = p.is_file();
            l.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            drop(g.lock());
            there
        })
    }

    #[test]
    fn a_slow_disk_neither_holds_up_the_page_nor_piles_up_looks() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ctx = egui::Context::default();
        let mut r = Recents::memory();
        let slow = Path::new("/slow/a.fig");
        r.note(slow, None, 1);
        let looks = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        r.io.probe = held_probe(&looks, &gate);
        let started = std::time::Instant::now();
        r.begin_frame(&ctx, 0.0);
        // Frames go on while the disk hangs, long past the wait, and clicks and loads that
        // drop the running look ask for no second one while it is stuck.
        for t in [0.5, 2.0, 5.0, 9.0] {
            r.begin_frame(&ctx, t);
            r.set_missing(&Id::file(slow), true);
            r.note(Path::new("/slow/b.fig"), None, 2);
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "the page waited"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(
            looks.load(Ordering::SeqCst),
            1,
            "one look at a time, one thread"
        );
        drop(held);
        settle(&mut r, &ctx, 9.5);
        // That answer was from before the clicks: dropped, and a new look asked right away.
        assert!(r.io.asked.is_some(), "the look after the clicks");
        settle(&mut r, &ctx, 9.5);
        assert!(r.is_missing(&Id::file(slow)));
        // The wait counts from when the look returned, not when it began.
        r.begin_frame(&ctx, 10.0);
        assert!(r.io.asked.is_none(), "no look right after one returned");
        r.begin_frame(&ctx, 10.6);
        assert!(r.io.asked.is_some(), "the next look, a second later");
        settle(&mut r, &ctx, 10.7);
    }

    #[test]
    fn a_look_begun_before_a_file_was_opened_does_not_undo_it() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ctx = egui::Context::default();
        let dir = temp_dir("older");
        let path = dir.join("back.fig");
        let mut r = Recents::memory();
        r.note(&path, None, 1);
        assert!(r.is_missing(&Id::file(&path)));
        let looks = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        r.io.probe = held_probe(&looks, &gate);
        r.begin_frame(&ctx, 0.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while looks.load(Ordering::SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline, "the look never began");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // The look saw it gone; then it is put back and opened from the dialog.
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/structure.fig"
            ),
            &path,
        )
        .unwrap();
        r.note(&path, None, 2);
        assert!(!r.is_missing(&Id::file(&path)));
        drop(held);
        settle(&mut r, &ctx, 0.1);
        assert!(
            !r.is_missing(&Id::file(&path)),
            "the older look's answer is dropped, not stored"
        );
        settle(&mut r, &ctx, 0.2);
        assert!(!r.is_missing(&Id::file(&path)), "the new look agrees");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_where_the_file_was_counts_as_missing() {
        let ctx = egui::Context::default();
        let dir = temp_dir("folder");
        let path = dir.join("now-a-folder.fig");
        std::fs::create_dir_all(&path).unwrap();
        let mut r = Recents::memory();
        r.note(&path, None, 1);
        assert!(r.is_missing(&Id::file(&path)), "noted as missing");
        r.begin_frame(&ctx, 0.0);
        settle(&mut r, &ctx, 0.1);
        assert!(
            r.is_missing(&Id::file(&path)),
            "a look agrees with a click: not a file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_list_is_capped_and_the_oldest_pictures_go_with_it() {
        let dir = temp_dir("cap");
        let mut r = Recents::at(dir.join("recents.json"), Some(dir.join("thumbs")));
        std::fs::create_dir_all(dir.join("thumbs")).unwrap();
        let first = PathBuf::from("/files/0.fig");
        let thumb = r.io.thumb_file(&Id::file(&first)).unwrap();
        std::fs::write(&thumb, b"png").unwrap();
        for i in 0..=MAX_ENTRIES {
            r.note(&PathBuf::from(format!("/files/{i}.fig")), None, i as u64);
        }
        assert_eq!(r.entries().len(), MAX_ENTRIES);
        assert!(r.entries().iter().all(|e| e.path != first));
        assert!(!thumb.exists(), "the dropped file's picture is deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_list_survives_a_restart_and_a_damaged_one_is_left_alone() {
        let dir = temp_dir("disk");
        let file = dir.join("recents.json");
        let mut r = Recents::at(file.clone(), None);
        r.note(Path::new("/x/a.fig"), Some("key"), 5);
        r.note(Path::new("/x/b.fig"), None, 6);
        let again = Recents::at(file.clone(), None);
        assert_eq!(again.entries(), r.entries());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "only its owner reads the list");
        }
        // One bad entry is skipped, not the list; relative paths are not kept.
        std::fs::write(
            &file,
            r#"[{"path":"/x/a.fig","opened":1},{"path":7},{"path":"rel.fig","opened":2}]"#,
        )
        .unwrap();
        assert_eq!(Recents::at(file.clone(), None).entries().len(), 1);
        // Not a list at all: kept in memory, the file is not written over.
        std::fs::write(&file, "not json").unwrap();
        let mut damaged = Recents::at(file.clone(), None);
        damaged.note(Path::new("/x/c.fig"), None, 9);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "not json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_a_file_deletes_its_picture() {
        let dir = temp_dir("remove");
        std::fs::create_dir_all(dir.join("thumbs")).unwrap();
        let mut r = Recents::at(dir.join("recents.json"), Some(dir.join("thumbs")));
        let path = Path::new("/y/a.fig");
        r.note(path, None, 1);
        let thumb = r.io.thumb_file(&Id::file(path)).unwrap();
        std::fs::write(&thumb, b"png").unwrap();
        r.remove(&Id::file(path));
        assert!(r.entries().is_empty());
        assert!(!thumb.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn structure_job() -> ThumbJob {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/structure.fig"
        );
        let doc = Arc::new(figstudio_fig::Document::open(path).unwrap());
        ThumbJob {
            page: doc.pages[0],
            doc,
            source: None,
        }
    }

    #[test]
    fn a_picture_finished_after_its_file_was_removed_is_not_written() {
        let dir = temp_dir("late");
        std::fs::create_dir_all(dir.join("thumbs")).unwrap();
        let mut r = Recents::at(dir.join("recents.json"), Some(dir.join("thumbs")));
        let path = Path::new("/z/a.fig");
        r.note(path, None, 1);
        let out = r.io.thumb_file(&Id::file(path)).unwrap();
        // Queued (wanted), then removed before the job got to it.
        let first = r.io.want(&out);
        r.remove(&Id::file(path));
        assert!(!write_thumb(&structure_job(), &out, first, &r.io.wanted).unwrap());
        assert!(!out.exists(), "Remove from recents sticks on disk");
        // Still wanted: written.
        r.note(path, None, 2);
        let second = r.io.want(&out);
        assert!(write_thumb(&structure_job(), &out, second, &r.io.wanted).unwrap());
        assert!(out.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn of_two_jobs_for_one_file_only_the_newest_writes() {
        let dir = temp_dir("twice");
        std::fs::create_dir_all(dir.join("thumbs")).unwrap();
        let mut r = Recents::at(dir.join("recents.json"), Some(dir.join("thumbs")));
        let path = Path::new("/z/a.fig");
        r.note(path, None, 1);
        let out = r.io.thumb_file(&Id::file(path)).unwrap();
        let older = r.io.want(&out);
        let newer = r.io.want(&out);
        assert!(write_thumb(&structure_job(), &out, newer, &r.io.wanted).unwrap());
        assert!(
            !write_thumb(&structure_job(), &out, older, &r.io.wanted).unwrap(),
            "the older job, finishing last, does not replace the newer picture"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
