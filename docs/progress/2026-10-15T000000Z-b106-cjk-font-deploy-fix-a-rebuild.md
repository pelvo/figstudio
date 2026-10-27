- 2026-10-15 — CJK font deploy fix: a rebuild on a machine with the Noto TTC but no
  fontTools made `scripts/web-font.sh` warn and exit 0, `web-build.sh` had already
  cleared `dist/fonts`, and the deploy served `/fonts/cjk-jp.otf` as 404 (verified with
  an authenticated production fetch; not an SPA HTML fallback), so CJK names showed as
  boxes. The script now fails on missing inputs by default and only
  `FIGSTUDIO_ALLOW_MISSING_CJK=1` opts out for a local dev build; new test
  `apps/figstudio-cli/tests/web-font.rs` (missing input seen exiting 0 before the fix)
  covers failure, the opt-out and the success path (five OTTO faces, ideographs in all,
  kana in JP, Hangul only in KR, OFL license). fontTools provisioned once via
  `uv tool install fonttools`. Verified in a real (headless Chromium) browser against a
  served dist: `/fonts/cjk-jp.otf` answers 200 `font/otf` with the exact dist bytes, the
  browser decodes it (`FontFace.load`) and rasterizes あ through that face. Not verified:
  the deploy itself (owner decides; the production 404 stands until a redeploy from a
  font-ready machine), and the app's own lazy fetch when a CJK-named file is open
  (unchanged code).
