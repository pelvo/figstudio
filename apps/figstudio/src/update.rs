//! Update checks (Settings → About). Asks GitHub for the recent releases and compares the
//! newest with this build's version; the notes of every newer release show in a popup. Nothing is sent unless the user turns on the check at start or
//! clicks the button; the request carries only figstudio's version in its User-Agent.
//!
//! On macOS (Apple silicon), the app running from a `figstudio.app` bundle that Homebrew did
//! not install also updates itself: it downloads the release's app zip and `SHA256SUMS`,
//! checks the zip's SHA-256, unpacks it with `ditto` (as the release workflow packs it),
//! checks the bundle's signature and version, then swaps it in place of the running bundle.
//! The old bundle stays in a hidden `.figstudio-update-*` folder beside it until the next
//! launch (or install) removes it, so the running copy keeps its files. A restart starts the
//! new one. Homebrew installs (the running bundle is the one a `Caskroom/figstudio/<version>`
//! link points to) are told to use `brew upgrade`, as a swapped bundle would leave the cask's
//! record behind; a copy macOS runs from App Translocation (started from a quarantined
//! download) is told to move to Applications first; everything else gets the release page.
//!
//! Trust: `SHA256SUMS` comes from the same release as the zip, so it only catches a corrupt
//! or partial download, and `codesign --verify` on an ad-hoc signature only shows the bundle
//! is consistent with itself (the download carries no quarantine flag, so Gatekeeper does
//! not assess it). What makes it figstudio's is the build attestation (`update_attest.rs`):
//! the zip's SHA-256 must be attested, through Sigstore, by figstudio's release workflow on
//! `main`, so uploading a release asset alone cannot ship code to installs.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use semver::Version;
use sha2::{Digest, Sha256};

/// The recent releases, newest first (drafts and pre-releases are skipped here).
const RELEASES: &str = "https://api.github.com/repos/pelvo/figstudio/releases?per_page=30";
/// The checksum list the release workflow attaches last.
const SUMS: &str = "SHA256SUMS";
/// What Homebrew users run instead.
pub const BREW_UPGRADE: &str = "brew upgrade --cask figstudio";

/// A published release newer than this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The release's page on GitHub.
    pub page: String,
    /// The macOS app zip and the checksum list, when the release has both.
    pub app: Option<Download>,
    /// The release notes of every release newer than this build, newest first.
    pub notes: Vec<Block>,
}

/// One line of the release notes as the popup draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// A release's heading ("0.2.0 (2026-10-24)").
    Version(String),
    /// A group in it ("Features", "Bug Fixes").
    Section(String),
    /// A change: its words, bold where the notes are (`**app:**`).
    Item(Vec<Span>),
    /// Any other paragraph.
    Text(Vec<Span>),
}

/// A run of words, bold or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub strong: bool,
}

/// The notes' Markdown as blocks: headings, list items and paragraphs, with links reduced
/// to their words (pulldown-cmark parses; nothing here is HTML).
pub fn blocks(markdown: &str) -> Vec<Block> {
    use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

    #[derive(Clone, Copy)]
    enum Kind {
        Heading(HeadingLevel),
        Item,
        Text,
    }
    fn flush(out: &mut Vec<Block>, open: &mut Option<(Kind, Vec<Span>)>) {
        let Some((kind, spans)) = open.take() else {
            return;
        };
        let mut merged: Vec<Span> = Vec::new();
        for span in spans {
            match merged.last_mut() {
                Some(last) if last.strong == span.strong => last.text.push_str(&span.text),
                _ => merged.push(span),
            }
        }
        if let Some(first) = merged.first_mut() {
            first.text = first.text.trim_start().to_owned();
        }
        if let Some(last) = merged.last_mut() {
            last.text = last.text.trim_end().to_owned();
        }
        merged.retain(|s| !s.text.is_empty());
        if merged.is_empty() {
            return;
        }
        let plain = || merged.iter().map(|s| s.text.as_str()).collect::<String>();
        out.push(match kind {
            Kind::Heading(HeadingLevel::H1 | HeadingLevel::H2) => Block::Version(plain()),
            Kind::Heading(_) => Block::Section(plain()),
            Kind::Item => Block::Item(merged),
            Kind::Text => Block::Text(merged),
        });
    }

    let mut out = Vec::new();
    let mut open: Option<(Kind, Vec<Span>)> = None;
    let mut strong = 0_usize;
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut out, &mut open);
                open = Some((Kind::Heading(level), Vec::new()));
            }
            // A nested list's item ends its parent's words.
            Event::Start(Tag::Item) => {
                flush(&mut out, &mut open);
                open = Some((Kind::Item, Vec::new()));
            }
            Event::Start(Tag::Paragraph) if open.is_none() => {
                open = Some((Kind::Text, Vec::new()));
            }
            Event::End(TagEnd::Heading(_) | TagEnd::Item) => flush(&mut out, &mut open),
            Event::End(TagEnd::Paragraph) if matches!(open, Some((Kind::Text, _))) => {
                flush(&mut out, &mut open);
            }
            Event::Start(Tag::Strong) => strong += 1,
            Event::End(TagEnd::Strong) => strong = strong.saturating_sub(1),
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, spans)) = &mut open {
                    spans.push(Span {
                        text: text.into_string(),
                        strong: strong > 0,
                    });
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, spans)) = &mut open {
                    spans.push(Span {
                        text: " ".to_owned(),
                        strong: strong > 0,
                    });
                }
            }
            _ => {}
        }
    }
    flush(&mut out, &mut open);
    out
}

/// The app zip and where its checksum is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sums_url: String,
}

#[derive(serde::Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(serde::Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

/// This build's version.
pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0))
}

/// The release zip's name for `version` (the release workflow's `figstudio-$TAG-macos-arm64.zip`).
fn app_zip(version: &Version) -> String {
    format!("figstudio-v{version}-macos-arm64.zip")
}

/// The newest release in GitHub's `releases` JSON if it is newer than `current`, with the
/// notes of every published release newer than `current`. Drafts, pre-releases and tags
/// that are not versions are skipped.
pub fn newer_release(json: &str, current: &Version) -> anyhow::Result<Option<Release>> {
    let list: Vec<GhRelease> =
        serde_json::from_str(json).context("unexpected reply from GitHub")?;
    let mut newer: Vec<(Version, GhRelease)> = list
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let tag = r.tag_name.trim();
            let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
            (version > *current).then_some((version, r))
        })
        .collect();
    newer.sort_by(|a, b| b.0.cmp(&a.0));
    let notes = newer
        .iter()
        .filter_map(|(_, r)| r.body.as_deref())
        .flat_map(|body| blocks(&tidy(body)))
        .collect();
    let Some((version, gh)) = newer.into_iter().next() else {
        return Ok(None);
    };
    let zip = app_zip(&version);
    let asset = |name: &str| gh.assets.iter().find(|a| a.name == name);
    let app = match (asset(&zip), asset(SUMS)) {
        (Some(z), Some(s)) => Some(Download {
            name: z.name.clone(),
            url: z.browser_download_url.clone(),
            size: z.size,
            sums_url: s.browser_download_url.clone(),
        }),
        _ => None,
    };
    Ok(Some(Release {
        version,
        page: gh.html_url,
        app,
        notes,
    }))
}

/// Release-please's notes without the pull request and commit links that end each line
/// (` ([#55](…)) ([7fc6fb6](…))`): the popup is for what changed.
fn tidy(notes: &str) -> String {
    /// A trailing ` ([label](url))` whose label is `#123` or a commit hash.
    fn reference(line: &str) -> Option<&str> {
        let rest = line.strip_suffix("))")?;
        let open = rest.rfind(" ([")?;
        let (label, url) = rest[open + 3..].split_once("](")?;
        let number = label
            .strip_prefix('#')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        let hash = (7..=40).contains(&label.len()) && label.bytes().all(|b| b.is_ascii_hexdigit());
        ((number || hash) && !url.contains(char::is_whitespace)).then(|| &line[..open])
    }
    notes
        .trim()
        .lines()
        .map(|line| {
            let mut line = line.trim_end();
            while let Some(shorter) = reference(line) {
                line = shorter;
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The SHA-256 (lower-case hex) that `SHA256SUMS` lists for `name` (`sha256sum` format).
pub fn listed_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start();
        let file = file.strip_prefix('*').unwrap_or(file);
        (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

/// The `.app` bundle an executable at `exe` runs from (`X.app/Contents/MacOS/exe`).
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then(|| app.to_path_buf())
}

/// Puts the bundle at `new` where `target` is, moving the old one to `old`. On macOS and
/// Linux the two trade places in one step (`RENAME_SWAP`, `RENAME_EXCHANGE`), so there is
/// no moment without an app at `target`; elsewhere, or on a volume without it,
/// [`swap_by_renames`] does it in two moves.
pub fn swap(new: &Path, target: &Path, old: &Path) -> std::io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use rustix::fs::{CWD, RenameFlags, renameat_with};
        use rustix::io::Errno;
        match renameat_with(CWD, new, CWD, target, RenameFlags::EXCHANGE) {
            Ok(()) => {
                // `target` is the new bundle now and `new` holds the replaced one.
                if let Err(e) = std::fs::rename(new, old) {
                    tracing::warn!("the replaced app stays at {}: {e}", new.display());
                }
                return Ok(());
            }
            // The volume or the kernel has no exchange.
            Err(e)
                if [Errno::INVAL, Errno::NOTSUP, Errno::OPNOTSUPP, Errno::NOSYS].contains(&e) => {}
            Err(e) => return Err(e.into()),
        }
    }
    swap_by_renames(new, target, old)
}

/// [`swap`] in two moves: `target` to `old`, then `new` to `target`. If the second move
/// fails, the old bundle goes back; if that fails too, the error says where it is.
fn swap_by_renames(new: &Path, target: &Path, old: &Path) -> std::io::Result<()> {
    std::fs::rename(target, old)?;
    if let Err(e) = std::fs::rename(new, target) {
        if let Err(back) = std::fs::rename(old, target) {
            return Err(std::io::Error::new(
                e.kind(),
                format!(
                    "{e}; the previous app could not be put back ({back}) and is at {}",
                    old.display()
                ),
            ));
        }
        return Err(e);
    }
    Ok(())
}

/// How this copy can be updated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// Download and swap the bundle at this path.
    Bundle(PathBuf),
    /// Installed with Homebrew: `brew upgrade --cask figstudio`.
    Homebrew,
    /// Run from App Translocation (a quarantined copy started where it was downloaded):
    /// macOS runs it from a read-only folder, so it must be moved to Applications first.
    Translocated,
    /// Anything else: the release page.
    Manual,
}

/// How the running copy can be updated (worked out once: it does not change while running).
pub fn route() -> Route {
    static ROUTE: std::sync::OnceLock<Route> = std::sync::OnceLock::new();
    ROUTE.get_or_init(find_route).clone()
}

fn find_route() -> Route {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Route::Manual;
    }
    let bundle = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()
        .and_then(|exe| bundle_of(&exe));
    route_for(bundle, &caskrooms())
}

/// Homebrew's cask folders: `HOMEBREW_PREFIX` (when set) and the two default prefixes.
fn caskrooms() -> Vec<PathBuf> {
    std::env::var_os("HOMEBREW_PREFIX")
        .map(PathBuf::from)
        .into_iter()
        .chain(["/opt/homebrew", "/usr/local"].map(PathBuf::from))
        .map(|prefix| prefix.join("Caskroom"))
        .collect()
}

/// How the bundle at `bundle` (canonical) can be updated, with Homebrew's cask folders at
/// `caskrooms`.
fn route_for(bundle: Option<PathBuf>, caskrooms: &[PathBuf]) -> Route {
    let Some(bundle) = bundle else {
        return Route::Manual;
    };
    if bundle
        .components()
        .any(|c| c.as_os_str() == "AppTranslocation")
    {
        Route::Translocated
    } else if cask_owns(&bundle, caskrooms) {
        Route::Homebrew
    } else {
        Route::Bundle(bundle)
    }
}

/// Whether a figstudio cask installed `bundle`: each `Caskroom/figstudio/<version>/
/// figstudio.app` is a link to where the cask put the app (any `--appdir`). Another copy
/// on a machine with the cask (a local build, one in Downloads) is not the cask's.
fn cask_owns(bundle: &Path, caskrooms: &[PathBuf]) -> bool {
    caskrooms
        .iter()
        .filter_map(|room| std::fs::read_dir(room.join("figstudio")).ok())
        .flatten()
        .flatten()
        .filter_map(|version| std::fs::canonicalize(version.path().join("figstudio.app")).ok())
        .any(|installed| installed == bundle)
}

/// Removes the hidden `.figstudio-update-*` folders in `folder` that an install left (the
/// replaced bundle kept until now, or a download that was cut off). Returns how many went.
fn clean_staging(folder: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with(STAGING)
                && e.file_type().is_ok_and(|t| t.is_dir())
        })
        .filter(|e| std::fs::remove_dir_all(e.path()).is_ok())
        .count()
}

/// At launch, off the UI thread: works out the [`route`] (so Settings and About do not scan
/// the disk on the UI thread) and removes what the last install left beside this bundle
/// (the bundle it replaced). Nothing is removed for other routes.
pub fn clean_up_at_launch() {
    let _ = std::thread::Builder::new()
        .name("figstudio-update-cleanup".into())
        .spawn(|| {
            let Route::Bundle(bundle) = route() else {
                return;
            };
            let Some(folder) = bundle.parent() else {
                return;
            };
            let removed = clean_staging(folder);
            if removed > 0 {
                tracing::info!(removed, "removed what the last update left");
            }
        });
}

/// The start of the hidden staging folders' names, beside the app.
const STAGING: &str = ".figstudio-update-";

fn client() -> anyhow::Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("figstudio/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .context("could not start the HTTP client")
}

/// Asks GitHub for the recent releases.
fn check() -> anyhow::Result<Option<Release>> {
    let reply = client()?
        .get(RELEASES)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(30))
        .send()
        .context("could not reach GitHub")?
        .error_for_status()
        .context("GitHub refused the update check")?;
    let reply = read_text(reply, MAX_RELEASES).context("could not read GitHub's reply")?;
    newer_release(&reply, &current())
}

/// Downloads, checks and swaps in the release's app bundle in place of `bundle`. `got`
/// counts the zip's bytes as they arrive.
fn install(release: &Release, bundle: &Path, got: &AtomicU64) -> anyhow::Result<()> {
    let Some(app) = &release.app else {
        bail!("this release has no macOS app to install");
    };
    let folder = bundle.parent().context("the app has no folder")?;
    // A download that was cut off, or a replaced bundle the last launch did not remove.
    clean_staging(folder);
    // In the app's own folder so the final move is a rename on one volume; this also fails
    // early when the folder is not writable.
    let staging = tempfile::Builder::new()
        .prefix(STAGING)
        .tempdir_in(folder)
        .with_context(|| format!("cannot write to {}", folder.display()))?;
    let http = client()?;
    let sums = http
        .get(&app.sums_url)
        .timeout(Duration::from_secs(60))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .context("could not download SHA256SUMS")?;
    let sums = read_text(sums, MAX_SUMS).context("could not download SHA256SUMS")?;
    let want = listed_sum(&sums, &app.name).context("SHA256SUMS does not list the app")?;

    let zip = staging.path().join(&app.name);
    let mut reply = http
        .get(&app.url)
        .timeout(Duration::from_secs(30 * 60))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .context("could not download the app")?;
    let mut file = std::fs::File::create(&zip).context("could not save the download")?;
    let limit = if app.size > 0 { app.size } else { MAX_DOWNLOAD };
    let sum = save_capped(&mut reply, &mut file, limit, got)?;
    drop(file);
    let download = Fetched {
        zip: &zip,
        sum: &sum,
        want: &want,
        version: &release.version,
    };
    place(&download, staging, bundle, |sum| {
        crate::update_attest::check(&http, sum)
    })
}

/// A downloaded app zip: its SHA-256, the one `SHA256SUMS` lists, and the version the
/// release names.
struct Fetched<'a> {
    zip: &'a Path,
    sum: &'a str,
    want: &'a str,
    version: &'a Version,
}

/// Installs a downloaded zip (in `staging`, beside `bundle`) over `bundle`. Nothing of it is
/// unpacked or run before its SHA-256 matches `SHA256SUMS` and `attest` (the build
/// attestation check, [`crate::update_attest::check`]) accepts that digest. Then the app is
/// unpacked, must be a real folder with a valid signature and the release's version, and
/// takes the old bundle's place.
fn place(
    download: &Fetched<'_>,
    staging: tempfile::TempDir,
    bundle: &Path,
    attest: impl FnOnce(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if download.sum != download.want {
        bail!("the download does not match its SHA-256; nothing was changed");
    }
    // Built and signed by figstudio's release workflow, not only uploaded to the release.
    attest(download.sum).context(
        "could not confirm figstudio's release workflow built this download; nothing was \
         changed (download it from the release page instead)",
    )?;

    let unpacked = staging.path().join("unpacked");
    run(Command::new("ditto")
        .args(["-x", "-k"])
        .arg(download.zip)
        .arg(&unpacked))
    .context("could not unpack the app")?;
    let new = unpacked.join("figstudio.app");
    // The bundle itself, not a link to one somewhere else.
    let is_folder = std::fs::symlink_metadata(&new).is_ok_and(|meta| meta.file_type().is_dir());
    if !is_folder {
        bail!("the download has no figstudio.app folder; nothing was changed");
    }
    run(Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&new))
    .context("the downloaded app's signature does not verify")?;
    let shown = run(Command::new("plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(new.join("Contents/Info.plist")))
    .context("could not read the downloaded app's version")?;
    if shown.trim() != download.version.to_string() {
        bail!(
            "the downloaded app is version {}, not {}",
            shown.trim(),
            download.version
        );
    }
    if let Err(e) = swap(&new, bundle, &staging.path().join("old.app")) {
        // The old bundle could not be put back: keep it in the staging folder.
        if !bundle.exists() {
            let _ = staging.keep();
        }
        return Err(e).with_context(|| {
            format!(
                "could not replace {} (download it from the release page instead)",
                bundle.display()
            )
        });
    }
    // The replaced bundle is the running one: it stays until the next launch removes it
    // (`clean_up_at_launch`), so this copy keeps its files and there is a way back.
    let _ = staging.keep();
    Ok(())
}

/// The most the release list may be (30 releases with their notes).
const MAX_RELEASES: u64 = 8 * 1024 * 1024;
/// The most `SHA256SUMS` may be (a line per release file).
const MAX_SUMS: u64 = 64 * 1024;

/// Reads `reader` to the end as UTF-8 text, failing once it is longer than `limit` bytes.
pub fn read_text(reader: impl Read, limit: u64) -> anyhow::Result<String> {
    let mut raw = Vec::new();
    reader.take(limit.saturating_add(1)).read_to_end(&mut raw)?;
    if raw.len() as u64 > limit {
        bail!("the reply is larger than {limit} bytes");
    }
    String::from_utf8(raw).context("the reply is not text")
}

/// The most a download may be when the release does not list the asset's size.
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

/// Copies `reader` into `writer`, counting bytes in `got`, and returns the SHA-256 (lower-case
/// hex). Stops with an error once more than `limit` bytes arrive, so a reply larger than the
/// release lists cannot fill the app's disk.
fn save_capped(
    reader: &mut impl Read,
    writer: &mut impl std::io::Write,
    limit: u64,
    got: &AtomicU64,
) -> anyhow::Result<String> {
    let mut hash = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let n = reader.read(&mut buf).context("the download broke off")?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > limit {
            bail!(
                "the download is larger than the release lists ({limit} bytes); nothing was changed"
            );
        }
        hash.update(&buf[..n]);
        writer
            .write_all(&buf[..n])
            .context("could not save the download")?;
        got.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Runs a system tool; its standard output, or its error output as the error.
fn run(cmd: &mut Command) -> anyhow::Result<String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd
        .output()
        .with_context(|| format!("could not run {program}"))?;
    if !out.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Starts the bundle at `bundle` once this process has exited (a second copy started now
/// would hand its files to this one and quit). The caller closes the window.
pub fn relaunch(bundle: &Path) -> anyhow::Result<()> {
    Command::new("/bin/sh")
        .args([
            "-c",
            r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec /usr/bin/open "$2""#,
            "sh",
        ])
        .arg(std::process::id().to_string())
        .arg(bundle)
        .spawn()
        .context("could not schedule the restart")?;
    Ok(())
}

/// Where the update stands, for Settings → About.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    /// Downloading and swapping in this release.
    Installing(Release),
    /// The new bundle is in place; a restart runs it.
    Installed(Release, PathBuf),
    Failed(String),
}

/// The update check and install, run on a thread of their own.
#[derive(Debug, Default)]
pub struct Updater {
    pub state: State,
    job: Option<mpsc::Receiver<State>>,
    /// Bytes of the app zip downloaded so far.
    got: Arc<AtomicU64>,
    /// The check at start has run (once per launch).
    started: bool,
    /// The release notes popup is showing.
    pub notes_open: bool,
    /// The running check is the one at start (a skipped version opens no popup then).
    auto: bool,
    /// The version the person asked not to be reminded of.
    skipped: Option<Version>,
}

impl Updater {
    /// `at_start` false: the check at start never runs (tests stay off the network).
    pub fn new(at_start: bool) -> Self {
        Self {
            started: !at_start,
            ..Self::default()
        }
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// The download's progress, 0 to 1, while installing.
    pub fn progress(&self) -> Option<f32> {
        match &self.state {
            State::Installing(r) => {
                r.app.as_ref().filter(|a| a.size > 0).map(|a| {
                    (self.got.load(Ordering::Relaxed) as f64 / a.size as f64).min(1.0) as f32
                })
            }
            _ => None,
        }
    }

    /// The check at start: decided once, on the first frame. When `on` (Settings → About's
    /// switch) it checks; turning the switch on later waits for the next launch, as its
    /// label says. A release at `skipped` ("Don't remind me of this version again") opens
    /// no popup from this check.
    pub fn at_start(&mut self, on: bool, skipped: &str, ctx: &egui::Context) {
        if self.started {
            return;
        }
        self.started = true;
        // Only a check that started here is the one at start (a Check now still running
        // keeps its own rules).
        if on && self.check(ctx) {
            self.auto = true;
            self.skipped = Version::parse(skipped).ok();
        }
    }

    /// Checks now (Check now): a newer release always opens its popup. Returns whether a
    /// check started (not while another check or an install runs).
    pub fn check(&mut self, ctx: &egui::Context) -> bool {
        if self.busy() {
            return false;
        }
        self.auto = false;
        self.state = State::Checking;
        self.spawn(ctx, || match check() {
            Ok(Some(r)) => State::Available(r),
            Ok(None) => State::UpToDate,
            Err(e) => State::Failed(format!("{e:#}")),
        });
        self.busy()
    }

    pub fn install(&mut self, bundle: PathBuf, ctx: &egui::Context) {
        let State::Available(release) = &self.state else {
            return;
        };
        if self.busy() {
            return;
        }
        let release = release.clone();
        self.state = State::Installing(release.clone());
        self.got.store(0, Ordering::Relaxed);
        let got = Arc::clone(&self.got);
        self.spawn(ctx, move || match install(&release, &bundle, &got) {
            Ok(()) => State::Installed(release, bundle),
            Err(e) => State::Failed(format!("{e:#}")),
        });
    }

    fn spawn(&mut self, ctx: &egui::Context, work: impl FnOnce() -> State + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("figstudio-update".into())
            .spawn(move || {
                let _ = tx.send(work());
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.job = Some(rx),
            Err(e) => self.state = State::Failed(format!("could not start the update: {e}")),
        }
    }

    /// Takes a finished check or install. Returns the new state when one arrived.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<&State> {
        let job = self.job.as_ref()?;
        match job.try_recv() {
            Ok(state) => {
                self.job = None;
                // A newer release shows its notes, unless the check at start found the one
                // the person skipped.
                let skipped = self.auto
                    && matches!(&state, State::Available(r) if Some(&r.version) == self.skipped.as_ref());
                self.notes_open |= matches!(state, State::Available(_)) && !skipped;
                self.state = state;
                Some(&self.state)
            }
            Err(mpsc::TryRecvError::Empty) => {
                // The progress bar moves while the download runs.
                if matches!(self.state, State::Installing(_)) {
                    ctx.request_repaint_after(Duration::from_millis(200));
                }
                None
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.job = None;
                self.state = State::Failed("the update stopped unexpectedly".into());
                Some(&self.state)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> serde_json::Value {
        let assets: Vec<_> = assets
            .iter()
            .map(|n| {
                serde_json::json!({
                    "name": n,
                    "browser_download_url": format!("https://example.invalid/{n}"),
                    "size": 1234,
                })
            })
            .collect();
        serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://example.invalid/{tag}"),
            "body": format!("## {tag}\n\n* a change"),
            "assets": assets,
        })
    }

    fn span(text: &str, strong: bool) -> Span {
        Span {
            text: text.to_owned(),
            strong,
        }
    }

    fn item(text: &str) -> Block {
        Block::Item(vec![span(text, false)])
    }

    #[test]
    fn the_notes_become_versions_sections_and_changes() {
        let notes = "## [0.2.0](https://example.invalid/compare) (2026-10-24)\n\n\n\
                     ### Features\n\n\
                     * **app,web:** draw `code` and [a link](https://example.invalid)\n\
                     * two\n  lines\n\
                       * nested\n\n\
                     A closing paragraph.\n\n\
                     <b>raw html</b>\n";
        assert_eq!(
            blocks(notes),
            [
                Block::Version("0.2.0 (2026-10-24)".into()),
                Block::Section("Features".into()),
                Block::Item(vec![
                    span("app,web:", true),
                    span(" draw code and a link", false)
                ]),
                item("two lines"),
                item("nested"),
                Block::Text(vec![span("A closing paragraph.", false)]),
                // Tags are dropped, their words kept.
                Block::Text(vec![span("raw html", false)]),
            ]
        );
        assert!(blocks("").is_empty());
    }

    fn list(releases: &[serde_json::Value]) -> String {
        serde_json::Value::Array(releases.to_vec()).to_string()
    }

    fn version_of(json: &str, now: &Version) -> Option<Version> {
        newer_release(json, now).ok().flatten().map(|r| r.version)
    }

    #[test]
    fn only_a_newer_release_is_offered() {
        let now = Version::new(0, 2, 0);
        let older = list(&[release("v0.1.9", &[])]);
        let same = list(&[release("v0.2.0", &[]), release("v0.1.0", &[])]);
        let newer = list(&[release("v0.2.1", &[]), release("v0.2.0", &[])]);
        assert_eq!(version_of(&older, &now), None);
        assert_eq!(version_of(&same, &now), None);
        assert_eq!(version_of(&newer, &now), Some(Version::new(0, 2, 1)));
        assert_eq!(version_of("[]", &now), None);
        // 0.10 is newer than 0.9 (a string comparison would say otherwise), whatever the
        // list's order.
        let ten = list(&[release("v0.9.5", &[]), release("v0.10.0", &[])]);
        assert_eq!(
            version_of(&ten, &Version::new(0, 9, 0)),
            Some(Version::new(0, 10, 0))
        );
    }

    #[test]
    fn drafts_pre_releases_and_odd_tags_are_skipped() {
        let now = Version::new(0, 2, 0);
        let mut draft = release("v0.4.0", &[]);
        draft["draft"] = true.into();
        let mut pre = release("v0.3.0", &[]);
        pre["prerelease"] = true.into();
        let json = list(&[draft, pre, release("nightly", &[]), release("v0.2.1", &[])]);
        let found = newer_release(&json, &now).ok().flatten();
        assert_eq!(
            found.as_ref().map(|r| r.version.clone()),
            Some(Version::new(0, 2, 1))
        );
        assert_eq!(
            found.map(|r| r.notes),
            Some(vec![Block::Version("v0.2.1".into()), item("a change")])
        );
    }

    #[test]
    fn the_notes_cover_every_newer_release_newest_first() {
        let now = Version::new(0, 2, 0);
        let mut empty = release("v0.2.2", &[]);
        empty["body"] = serde_json::Value::Null;
        let json = list(&[
            release("v0.2.1", &[]),
            release("v0.3.0", &[]),
            empty,
            release("v0.2.0", &[]),
        ]);
        let notes = newer_release(&json, &now).ok().flatten().map(|r| r.notes);
        assert_eq!(
            notes,
            Some(vec![
                Block::Version("v0.3.0".into()),
                item("a change"),
                Block::Version("v0.2.1".into()),
                item("a change"),
            ])
        );
    }

    #[test]
    fn the_notes_drop_the_pull_request_and_commit_links() {
        let body = "## [0.2.0](https://example.invalid/compare) (2026-10-24)\n\n\
                    ### Features\n\n\
                    * **app:** a change ([#55](https://example.invalid/55)) \
                    ([7fc6fb6](https://example.invalid/c/7fc6fb6))\n\
                    * keeps [a link](https://example.invalid) and (#12) in its text\n";
        assert_eq!(
            tidy(body),
            "## [0.2.0](https://example.invalid/compare) (2026-10-24)\n\n### Features\n\n\
             * **app:** a change\n\
             * keeps [a link](https://example.invalid) and (#12) in its text"
        );
    }

    #[test]
    fn a_bad_reply_is_an_error_not_a_panic() {
        let now = Version::new(0, 2, 0);
        assert!(newer_release("not json", &now).is_err());
        assert!(
            newer_release("{}", &now).is_err(),
            "a list, not one release"
        );
        assert!(newer_release("[{}]", &now).is_err());
    }

    #[test]
    fn the_app_needs_its_zip_and_the_checksums() {
        let now = Version::new(0, 2, 0);
        let zip = "figstudio-v0.3.0-macos-arm64.zip";
        let both = list(&[release("v0.3.0", &[zip, "SHA256SUMS", "other.tar.gz"])]);
        let app = newer_release(&both, &now)
            .ok()
            .flatten()
            .and_then(|r| r.app);
        assert_eq!(app.as_ref().map(|a| a.name.as_str()), Some(zip));
        assert_eq!(
            app.map(|a| a.sums_url),
            Some("https://example.invalid/SHA256SUMS".to_owned())
        );
        for assets in [&[zip][..], &["SHA256SUMS"][..]] {
            let one = list(&[release("v0.3.0", assets)]);
            let r = newer_release(&one, &now).ok().flatten();
            assert_eq!(r.map(|r| r.app), Some(None), "{assets:?}");
        }
    }

    #[test]
    fn the_checksum_is_read_for_the_named_file_only() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!("{a}  other.zip\n{b} *app.zip\nshort  bad.zip\n");
        assert_eq!(listed_sum(&sums, "app.zip"), Some("b".repeat(64)));
        assert_eq!(listed_sum(&sums, "other.zip"), Some(a));
        assert_eq!(listed_sum(&sums, "bad.zip"), None);
        assert_eq!(listed_sum(&sums, "missing.zip"), None);
    }

    #[test]
    fn the_bundle_is_found_from_the_executable() {
        let exe = Path::new("/Applications/figstudio.app/Contents/MacOS/figstudio");
        assert_eq!(
            bundle_of(exe),
            Some(PathBuf::from("/Applications/figstudio.app"))
        );
        assert_eq!(bundle_of(Path::new("/repo/target/release/figstudio")), None);
        assert_eq!(
            bundle_of(Path::new("/x/figstudio/Contents/MacOS/figstudio")),
            None
        );
    }

    #[test]
    fn a_swap_replaces_the_bundle_and_a_failed_one_keeps_it() -> std::io::Result<()> {
        // `swap` (one step where the system has it) and the two-move fallback.
        type Swap = fn(&Path, &Path, &Path) -> std::io::Result<()>;
        let swaps: [Swap; 2] = [swap, swap_by_renames];
        for swap in swaps {
            let dir = tempfile::tempdir()?;
            let target = dir.path().join("figstudio.app");
            let new = dir.path().join("new.app");
            let old = dir.path().join("old.app");
            std::fs::create_dir(&target)?;
            std::fs::write(target.join("v"), "1")?;
            std::fs::create_dir(&new)?;
            std::fs::write(new.join("v"), "2")?;
            swap(&new, &target, &old)?;
            assert_eq!(std::fs::read_to_string(target.join("v"))?, "2");
            assert_eq!(std::fs::read_to_string(old.join("v"))?, "1");
            assert!(!new.exists());

            // The new bundle is missing: the swap fails and the running one stays.
            let gone = dir.path().join("gone.app");
            let old2 = dir.path().join("old2.app");
            assert!(swap(&gone, &target, &old2).is_err());
            assert_eq!(std::fs::read_to_string(target.join("v"))?, "2");
            assert!(!old2.exists());
        }
        Ok(())
    }

    /// Puts a garbage "zip" through [`place`] with the given sums and attestation answer;
    /// returns the result and the digest the attestation check was asked about.
    fn place_garbage(
        bundle: &Path,
        sum: &str,
        attest: anyhow::Result<()>,
    ) -> anyhow::Result<(anyhow::Result<()>, Option<String>)> {
        let folder = bundle.parent().context("no folder")?;
        let staging = tempfile::Builder::new()
            .prefix(STAGING)
            .tempdir_in(folder)?;
        let zip = staging.path().join("figstudio.zip");
        std::fs::write(&zip, "not a zip")?;
        let version = Version::new(9, 0, 0);
        let fetched = Fetched {
            zip: &zip,
            sum,
            want: "aa",
            version: &version,
        };
        let mut asked = None;
        let placed = place(&fetched, staging, bundle, |sum| {
            asked = Some(sum.to_owned());
            attest
        });
        Ok((placed, asked))
    }

    #[test]
    fn a_download_is_unpacked_only_after_its_sum_and_attestation_check() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let bundle = dir.path().join("figstudio.app");
        std::fs::create_dir(&bundle)?;
        std::fs::write(bundle.join("v"), "1")?;

        // The sum does not match: not even the attestation is looked up.
        let (placed, asked) = place_garbage(&bundle, "bb", Ok(()))?;
        assert!(format!("{:#}", placed.err().context("placed")?).contains("SHA-256"));
        assert_eq!(asked, None);

        // The sum matches but nothing attests it: refused before unpacking (a garbage zip
        // reaching the unpack step would fail there instead, without asking).
        let (placed, asked) = place_garbage(&bundle, "aa", Err(anyhow::anyhow!("none")))?;
        let why = format!("{:#}", placed.err().context("placed")?);
        assert!(
            why.contains("release workflow built this download"),
            "{why}"
        );
        assert_eq!(asked.as_deref(), Some("aa"));

        // Attested: only now is it unpacked (and this one is not a zip).
        let (placed, asked) = place_garbage(&bundle, "aa", Ok(()))?;
        assert!(format!("{:#}", placed.err().context("placed")?).contains("unpack"));
        assert_eq!(asked.as_deref(), Some("aa"));

        // The running bundle was never touched, and no staging folder is left.
        assert_eq!(std::fs::read_to_string(bundle.join("v"))?, "1");
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
        Ok(())
    }

    /// An ad-hoc signed `figstudio.app` at `at` showing `version`.
    #[cfg(target_os = "macos")]
    fn signed_app(at: &Path, version: &str) -> anyhow::Result<()> {
        let contents = at.join("Contents");
        std::fs::create_dir_all(contents.join("MacOS"))?;
        std::fs::write(
            contents.join("Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleExecutable</key><string>figstudio</string>\
                 <key>CFBundleIdentifier</key><string>dev.figstudio.test</string>\
                 <key>CFBundleShortVersionString</key><string>{version}</string>\
                 </dict></plist>\n"
            ),
        )?;
        std::fs::copy("/usr/bin/true", contents.join("MacOS/figstudio"))?;
        run(Command::new("codesign")
            .args(["--sign", "-", "--force"])
            .arg(at))?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_attested_zip_replaces_the_bundle_and_a_linked_app_does_not() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let bundle = dir.path().join("figstudio.app");
        signed_app(&bundle, "1.0.0")?;
        let version = Version::new(2, 0, 0);
        let shown = |at: &Path| {
            run(Command::new("plutil")
                .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
                .arg(at.join("Contents/Info.plist")))
        };
        let install = |make_zip: &dyn Fn(&Path) -> anyhow::Result<()>| -> anyhow::Result<()> {
            let staging = tempfile::Builder::new()
                .prefix(STAGING)
                .tempdir_in(dir.path())?;
            let zip = staging.path().join("figstudio.zip");
            make_zip(&zip)?;
            let fetched = Fetched {
                zip: &zip,
                sum: "aa",
                want: "aa",
                version: &version,
            };
            place(&fetched, staging, &bundle, |_| Ok(()))
        };

        // The zip's figstudio.app is a link to a valid app elsewhere: refused.
        let elsewhere = tempfile::tempdir()?;
        signed_app(&elsewhere.path().join("figstudio.app"), "2.0.0")?;
        let linked = install(&|zip| {
            let src = tempfile::tempdir()?;
            std::os::unix::fs::symlink(
                elsewhere.path().join("figstudio.app"),
                src.path().join("figstudio.app"),
            )?;
            run(Command::new("zip")
                .current_dir(src.path())
                .args(["-q", "-y", "-r"])
                .arg(zip)
                .arg("figstudio.app"))?;
            Ok(())
        });
        let why = format!("{:#}", linked.err().context("installed a link")?);
        assert!(why.contains("no figstudio.app folder"), "{why}");
        assert!(!std::fs::symlink_metadata(&bundle)?.is_symlink());
        assert_eq!(shown(&bundle)?.trim(), "1.0.0");

        // A real, signed 2.0.0 bundle: it takes the old one's place.
        install(&|zip| {
            let src = tempfile::tempdir()?;
            let app = src.path().join("figstudio.app");
            signed_app(&app, "2.0.0")?;
            run(Command::new("ditto")
                .args(["-c", "-k", "--keepParent"])
                .arg(&app)
                .arg(zip))?;
            Ok(())
        })?;
        assert_eq!(shown(&bundle)?.trim(), "2.0.0");
        Ok(())
    }

    #[test]
    fn text_replies_are_size_capped() -> anyhow::Result<()> {
        assert_eq!(read_text(&b"abcd"[..], 4)?, "abcd");
        assert!(read_text(&b"abcde"[..], 4).is_err());
        Ok(())
    }

    #[test]
    fn a_newer_release_found_opens_its_notes_and_nothing_else_does() {
        let ctx = egui::Context::default();
        let finish = |state: State| {
            let mut updater = Updater::new(false);
            updater.spawn(&ctx, move || state);
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while updater.poll(&ctx).is_none() {
                assert!(std::time::Instant::now() < deadline, "the job never ended");
                std::thread::yield_now();
            }
            assert!(!updater.busy());
            updater.notes_open
        };
        let json = list(&[release("v9.0.0", &[])]);
        let found = newer_release(&json, &Version::new(0, 2, 0)).ok().flatten();
        assert!(finish(State::Available(found.expect("newer"))));
        assert!(!finish(State::UpToDate));
        assert!(!finish(State::Failed("offline".into())));
    }

    #[test]
    fn a_skipped_version_opens_no_popup_at_start_but_does_on_check_now() {
        let ctx = egui::Context::default();
        let json = list(&[release("v9.0.0", &[])]);
        let found = newer_release(&json, &Version::new(0, 2, 0))
            .ok()
            .flatten()
            .expect("newer");
        let opens = |auto: bool, skipped: Option<Version>| {
            let mut updater = Updater::new(false);
            updater.auto = auto;
            updater.skipped = skipped;
            let state = State::Available(found.clone());
            updater.spawn(&ctx, move || state);
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while updater.poll(&ctx).is_none() {
                assert!(std::time::Instant::now() < deadline, "the job never ended");
                std::thread::yield_now();
            }
            updater.notes_open
        };
        let nine = Some(Version::new(9, 0, 0));
        assert!(!opens(true, nine.clone()), "skipped, at start");
        assert!(opens(false, nine), "Check now still shows it");
        assert!(
            opens(true, Some(Version::new(8, 0, 0))),
            "a newer one than skipped"
        );
        assert!(opens(true, None));
    }

    #[test]
    fn the_check_at_start_is_decided_on_the_first_frame() {
        let ctx = egui::Context::default();
        // Off at start: turning the switch on later does not check in this session.
        let mut updater = Updater::new(true);
        updater.at_start(false, "", &ctx);
        updater.at_start(true, "", &ctx);
        assert!(!updater.busy(), "no check until the next launch");
        assert_eq!(updater.state, State::Idle);
    }

    #[test]
    fn a_check_now_still_running_is_not_taken_for_the_check_at_start() {
        let ctx = egui::Context::default();
        let json = list(&[release("v9.0.0", &[])]);
        let found = newer_release(&json, &Version::new(0, 2, 0))
            .ok()
            .flatten()
            .expect("newer");
        let mut updater = Updater::new(true);
        // A Check now in flight (its reply held back until the check at start has run).
        let (go, wait) = mpsc::channel::<()>();
        updater.state = State::Checking;
        updater.spawn(&ctx, move || {
            let _ = wait.recv();
            State::Available(found)
        });
        updater.at_start(true, "9.0.0", &ctx);
        assert!(!updater.auto && updater.skipped.is_none());
        let _ = go.send(());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while updater.poll(&ctx).is_none() {
            assert!(std::time::Instant::now() < deadline, "the job never ended");
            std::thread::yield_now();
        }
        assert!(
            updater.notes_open,
            "Check now shows even the skipped version"
        );
    }

    // Homebrew's version folders hold symbolic links (macOS; any Unix for the test).
    #[cfg(unix)]
    #[test]
    fn the_route_follows_the_running_bundle() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = std::fs::canonicalize(dir.path())?;
        let app = |folder: &str| -> std::io::Result<PathBuf> {
            let path = root.join(folder).join("figstudio.app");
            std::fs::create_dir_all(&path)?;
            Ok(path)
        };
        let applications = app("Applications")?;
        let elsewhere = app("Custom")?;
        let downloads = app("Downloads")?;
        let rooms = [root.join("Caskroom")];
        assert_eq!(
            route_for(Some(applications.clone()), &rooms),
            Route::Bundle(applications.clone()),
            "no cask installed"
        );
        let link = root.join("Caskroom/figstudio/0.1.0");
        std::fs::create_dir_all(&link)?;
        std::os::unix::fs::symlink(&applications, link.join("figstudio.app"))?;
        assert_eq!(
            route_for(Some(applications.clone()), &rooms),
            Route::Homebrew
        );
        // Another copy on a machine with the cask is not the cask's.
        assert_eq!(
            route_for(Some(downloads.clone()), &rooms),
            Route::Bundle(downloads)
        );
        // A cask installed with another --appdir links there.
        let link = root.join("Caskroom/figstudio/0.2.0");
        std::fs::create_dir_all(&link)?;
        std::os::unix::fs::symlink(&elsewhere, link.join("figstudio.app"))?;
        assert_eq!(route_for(Some(elsewhere), &rooms), Route::Homebrew);
        let moved = PathBuf::from("/private/var/folders/x/T/AppTranslocation/0000/d/figstudio.app");
        assert_eq!(route_for(Some(moved), &rooms), Route::Translocated);
        assert_eq!(route_for(None, &rooms), Route::Manual);
        Ok(())
    }

    #[test]
    fn only_the_update_staging_folders_are_cleaned() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let left = dir.path().join(".figstudio-update-abc123");
        std::fs::create_dir_all(left.join("old.app/Contents"))?;
        std::fs::create_dir_all(dir.path().join("figstudio.app"))?;
        std::fs::create_dir_all(dir.path().join("Other.app"))?;
        std::fs::write(
            dir.path().join(".figstudio-update-note"),
            "a file, not ours",
        )?;
        assert_eq!(clean_staging(dir.path()), 1);
        assert!(!left.exists());
        assert!(dir.path().join("figstudio.app").exists());
        assert!(dir.path().join("Other.app").exists());
        assert!(dir.path().join(".figstudio-update-note").exists());
        assert_eq!(clean_staging(&dir.path().join("missing")), 0);
        Ok(())
    }

    #[test]
    fn a_download_stops_past_the_listed_size() -> anyhow::Result<()> {
        let got = AtomicU64::new(0);
        let mut out = Vec::new();
        let sum = save_capped(&mut &b"abc"[..], &mut out, 3, &got)?;
        assert_eq!(out, b"abc");
        assert_eq!(got.load(Ordering::Relaxed), 3);
        assert_eq!(
            sum,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut out = Vec::new();
        let big = vec![7_u8; 200 * 1024];
        let err = save_capped(&mut &big[..], &mut out, 100 * 1024, &AtomicU64::new(0));
        assert!(err.is_err(), "larger than listed");
        assert!(
            out.len() <= 100 * 1024,
            "stopped at the limit: {}",
            out.len()
        );
        Ok(())
    }

    #[test]
    fn this_build_has_a_version() {
        assert_eq!(current().to_string(), env!("CARGO_PKG_VERSION"));
    }
}
