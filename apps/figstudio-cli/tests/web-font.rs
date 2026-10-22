//! scripts/web-font.sh must not let a build ship without the CJK fonts: after a
//! rebuild on a machine without fontTools it warned and exited 0, so the deploy
//! had no fonts directory, /fonts/cjk-*.otf answered 404 and CJK layer names
//! showed as boxes. Missing inputs now fail the build; FIGSTUDIO_ALLOW_MISSING_CJK=1
//! is the explicit opt-out for a local dev build that does not need CJK names.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/web-font.sh");

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("figstudio-web-font-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_font_script(out: &Path, allow_missing: bool) -> Output {
    let mut cmd = Command::new(SCRIPT);
    cmd.env_remove("FIGSTUDIO_CJK_FONT")
        .env_remove("FIGSTUDIO_ALLOW_MISSING_CJK")
        .env("FIGSTUDIO_CJK_FONT", "/nonexistent/NotoSansCJK-Regular.ttc")
        .arg(out);
    if allow_missing {
        cmd.env("FIGSTUDIO_ALLOW_MISSING_CJK", "1");
    }
    cmd.output().unwrap()
}

#[test]
fn missing_font_input_fails_the_build() {
    let out = TempDir::new("missing");
    let res = run_font_script(&out.0, false);
    assert!(
        !res.status.success(),
        "web-font.sh must fail on missing inputs, not warn and exit 0; stderr: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("boxes"),
        "warn about the consequence: {stderr}"
    );
    assert!(
        stderr.contains("FIGSTUDIO_ALLOW_MISSING_CJK"),
        "point at the dev opt-out: {stderr}"
    );
    assert!(
        !out.0.join("cjk-jp.otf").exists(),
        "nothing may be written on failure"
    );
}

#[test]
fn missing_font_opt_out_is_explicit_and_warns() {
    let out = TempDir::new("optout");
    let res = run_font_script(&out.0, true);
    assert!(
        res.status.success(),
        "the explicit opt-out must still build: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(stderr.contains("boxes"), "the opt-out must warn: {stderr}");
    assert!(
        stderr.contains("FIGSTUDIO_ALLOW_MISSING_CJK"),
        "the warning must name the active opt-out: {stderr}"
    );
    assert!(
        !out.0.join("cjk-jp.otf").exists(),
        "the opt-out writes no fonts"
    );
}

/// The success path needs a Noto CJK collection and pyftsubset on this machine
/// (web-build.sh uses the same detection), so it skips with a note where they are
/// absent; the two failure tests above always run.
#[test]
fn writes_all_five_faces_and_the_license() {
    let candidates = [
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    ];
    let home = std::env::var("HOME").unwrap_or_default();
    let subset = ["pyftsubset", "ttx"].iter().all(|tool| {
        Command::new(tool)
            .arg("--help")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            || Path::new(&home).join(".local/bin").join(tool).exists()
    });
    let font = std::env::var_os("FIGSTUDIO_CJK_FONT")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| candidates.iter().map(PathBuf::from).find(|p| p.is_file()));
    let (Some(font), true) = (font, subset) else {
        eprintln!("skipped: no Noto Sans CJK collection or fontTools (pyftsubset) here");
        return;
    };

    let out = TempDir::new("success");
    // This test is about the CJK group. The script builds the symbol group after it,
    // which needs four more Noto fonts that the skip check above does not look for, so a
    // machine with CJK and fontTools but without them failed here. The symbol group's
    // opt-out lets that machine pass; below, it may only have skipped a missing font.
    let res = Command::new(SCRIPT)
        .env_remove("FIGSTUDIO_ALLOW_MISSING_CJK")
        .env("FIGSTUDIO_ALLOW_MISSING_SYMBOLS", "1")
        .env("FIGSTUDIO_CJK_FONT", &font)
        .arg(&out.0)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        res.status.success(),
        "with inputs present the script must succeed: {stderr}"
    );
    if let Some(skipped) = stderr.lines().find(|l| l.contains("symbol fallback omitted")) {
        assert!(
            skipped.starts_with("web-font: missing "),
            "with its fonts installed the symbol group must build: {skipped}"
        );
    } else {
        for face in ["symbols", "symbols2", "math", "emoji"] {
            assert!(out.0.join(format!("{face}.ttf")).is_file(), "{face}.ttf");
        }
    }

    let license = fs::read_to_string(out.0.join("cjk-LICENSE.txt")).unwrap();
    assert!(
        license.contains("Open Font License"),
        "the license file must name the OFL"
    );
    for variant in ["jp", "sc", "tc", "hk", "kr"] {
        let font = out.0.join(format!("cjk-{variant}.otf"));
        let data = fs::read(&font).unwrap_or_else(|e| panic!("cjk-{variant}.otf: {e}"));
        assert_eq!(
            &data[..4],
            b"OTTO",
            "cjk-{variant}.otf is an OpenType/CFF font"
        );
        let cmap = cmap_dump(&font);
        assert!(
            cmap.contains("0x4e00"),
            "cjk-{variant}.otf covers the ideograph block"
        );
        if variant == "kr" {
            assert!(cmap.contains("0xac00"), "kr covers Hangul");
        } else {
            assert!(
                !cmap.contains("0xac00"),
                "only kr has Hangul; cjk-{variant}.otf must not"
            );
            if variant == "jp" {
                assert!(cmap.contains("0x3042"), "jp covers kana");
            }
        }
    }
}

/// `ttx -t cmap` dumps the character map; pyftsubset already proved each face's
/// full name (see the script), so coverage is what is left to assert.
fn cmap_dump(font: &Path) -> String {
    let ttx = home_bin("ttx");
    let res = Command::new(&ttx)
        .args(["-q", "-o", "-", "-t", "cmap"])
        .arg(font)
        .output()
        .unwrap();
    assert!(
        res.status.success(),
        "ttx cmap dump failed: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    String::from_utf8_lossy(&res.stdout).into_owned()
}

fn home_bin(tool: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let local = Path::new(&home).join(".local/bin").join(tool);
    if local.exists() {
        local
    } else {
        PathBuf::from(tool)
    }
}
