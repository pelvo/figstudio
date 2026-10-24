# Changelog

## [0.2.0](https://github.com/pelvo/figstudio/compare/v0.1.0...v0.2.0) (2026-10-24)


### Features

* **ci,docs:** linux release binaries and homebrew install docs ([#55](https://github.com/pelvo/figstudio/issues/55)) ([7fc6fb6](https://github.com/pelvo/figstudio/commit/7fc6fb670b6b410645de07b368831dac29f1902a))
* **compare:** screen-by-screen report and lossless WebP output ([#52](https://github.com/pelvo/figstudio/issues/52)) ([4bffd00](https://github.com/pelvo/figstudio/commit/4bffd00a17d7da61de3e50a361d812597fdb6d1a))
* **diff:** list edits of layers a recreated screen pairs by name ([#57](https://github.com/pelvo/figstudio/issues/57)) ([7a992f2](https://github.com/pelvo/figstudio/commit/7a992f227ce7084e772e66be7ed9a414c337c8fc))
* **mcp:** run with npx -y figstudio-mcp, open the public build by ([7a6f9b0](https://github.com/pelvo/figstudio/commit/7a6f9b08b320127d7c9e4617671872d8ffb5debb))
* **mcp:** run with npx -y figstudio-mcp, open the public build by default ([#43](https://github.com/pelvo/figstudio/issues/43)) ([7a6f9b0](https://github.com/pelvo/figstudio/commit/7a6f9b08b320127d7c9e4617671872d8ffb5debb))


### Bug Fixes

* **render,app,web:** draw boolean results, Figma-style canvas labels, browser text layout ([#50](https://github.com/pelvo/figstudio/issues/50)) ([1b46309](https://github.com/pelvo/figstudio/commit/1b46309ebdb092f0fa84bdaf102d9cd9a896c08c))
* **web:** draw cjk file names on the recents page ([#51](https://github.com/pelvo/figstudio/issues/51)) ([00a5e73](https://github.com/pelvo/figstudio/commit/00a5e73da555d2acf64a9e954d94ee89c5762dbb))

## 0.1.0 (2026-10-22)


### Features

* **app,mcp:** open figma.com links from local copies, add app icon ([#27](https://github.com/pelvo/figstudio/issues/27)) ([54d80d8](https://github.com/pelvo/figstudio/commit/54d80d8321a7b273aca121e4a97be5e6a84b825e))
* **app,render,web:** smooth tiled canvas and vello_gpu rendering ([#7](https://github.com/pelvo/figstudio/issues/7)) ([d22e4a8](https://github.com/pelvo/figstudio/commit/d22e4a8d47b25de1bc9184ba3b95d9e57b8b55cc))
* **app,render:** select layers and copy text like Figma ([#42](https://github.com/pelvo/figstudio/issues/42)) ([28f448e](https://github.com/pelvo/figstudio/commit/28f448e2f1597b1c0e1a7241f8787c59d4960e5a))
* **app,web:** hide cloud relay on a static-only deploy ([#39](https://github.com/pelvo/figstudio/issues/39)) ([536b331](https://github.com/pelvo/figstudio/commit/536b33186328cde2485b059999510a57fd015886))
* **app,web:** keep recents in the browser ([#47](https://github.com/pelvo/figstudio/issues/47)) ([115a4a5](https://github.com/pelvo/figstudio/commit/115a4a56f84447f35f67d4113041b4a8dfdb45ef))
* **app:** native menu bar on macos and windows with muda ([#17](https://github.com/pelvo/figstudio/issues/17)) ([42fdf7e](https://github.com/pelvo/figstudio/commit/42fdf7e102f4da728afc9a10e252f9c102e7d36a))
* **app:** onboarding and a recents page ([#44](https://github.com/pelvo/figstudio/issues/44)) ([c1295fd](https://github.com/pelvo/figstudio/commit/c1295fd0cffa6c79a6092536f928df52fce8269d))
* **app:** open internal pages from the sidebar ([#48](https://github.com/pelvo/figstudio/issues/48)) ([ab63661](https://github.com/pelvo/figstudio/commit/ab6366153effc947d74c06648dc982354f668f89))
* **app:** persist workspace and mcp access settings ([#36](https://github.com/pelvo/figstudio/issues/36)) ([feba1e4](https://github.com/pelvo/figstudio/commit/feba1e457defe65286ed57494fd9f6d5fd76c94b))
* **app:** redesign the UI in the Arc browser style ([#28](https://github.com/pelvo/figstudio/issues/28)) ([fa365f5](https://github.com/pelvo/figstudio/commit/fa365f5fa290c18a4e264afd2ecc33caa9148ed4))
* **diff:** add whole-file version compare CLI and MCP tools ([#19](https://github.com/pelvo/figstudio/issues/19)) ([786d11c](https://github.com/pelvo/figstudio/commit/786d11c46865357a5ccab046c8c112464c848e26))
* **mcp,app,web:** add zero-upload local browser proxy (--browser-proxy) ([#23](https://github.com/pelvo/figstudio/issues/23)) ([555b043](https://github.com/pelvo/figstudio/commit/555b0438b5a192018f5087fd1fe1bd4092a866d5))
* **mcp,app,web:** drive the browser app from MCP via a local bridge ([#16](https://github.com/pelvo/figstudio/issues/16)) ([352b3bc](https://github.com/pelvo/figstudio/commit/352b3bc04c0b4b160f37e9180bcfc10d0fe7af71))
* **mcp,app:** MCP inside the app, copy links and regions for agents ([#10](https://github.com/pelvo/figstudio/issues/10)) ([8754bd5](https://github.com/pelvo/figstudio/commit/8754bd50ed7082237b76344291ee1ca3c9c5fee7))
* **mcp,web:** execute browser tools through a hibernating relay ([#18](https://github.com/pelvo/figstudio/issues/18)) ([f3443bf](https://github.com/pelvo/figstudio/commit/f3443bfcf05a48c0e47fadf0ecfee1e6be79edb1))
* **mcp,web:** multi-device session routing for the cloud relay ([#21](https://github.com/pelvo/figstudio/issues/21)) ([0d99fb1](https://github.com/pelvo/figstudio/commit/0d99fb131f2dccf60f89a1c04646216c3c156eb3))
* **mcp:** add offline figstudio-mcp server for local .fig files ([#5](https://github.com/pelvo/figstudio/issues/5)) ([1cf2c89](https://github.com/pelvo/figstudio/commit/1cf2c892a19e32d93e58b8d665f4028f5514456e))
* **web:** deploy to Cloudflare with a selectable CJK font ([#14](https://github.com/pelvo/figstudio/issues/14)) ([dc816d4](https://github.com/pelvo/figstudio/commit/dc816d45431e93c6afd7fca81c8e827fe40c24f8))


### Bug Fixes

* **app:** keep the properties panel resizable and tint it per space ([#38](https://github.com/pelvo/figstudio/issues/38)) ([8ce51be](https://github.com/pelvo/figstudio/commit/8ce51bee9866d46fadc00b1fbbe1d39af3541c66))
* **fig:** follow parameterConsumptionMap property bindings ([#24](https://github.com/pelvo/figstudio/issues/24)) ([0f79e45](https://github.com/pelvo/figstudio/commit/0f79e45fed755e9220b887ef0ae3c49b8cfa50af))
* **mcp,web:** keep relay_keys verbatim for Copy link parity and cap label input at 64 chars ([#22](https://github.com/pelvo/figstudio/issues/22)) ([2de8c0b](https://github.com/pelvo/figstudio/commit/2de8c0bb83cda9ad6a02ab2d1a3fd836822a9304))
* **render,app:** close stroke, shadow, fill and nested-instance gaps ([#4](https://github.com/pelvo/figstudio/issues/4)) ([286a047](https://github.com/pelvo/figstudio/commit/286a0470a043dae4fb2d6a20a2e53f767c719c6c))
* **render,app:** draw symbol and emoji text via outline-capable fallback faces ([#41](https://github.com/pelvo/figstudio/issues/41)) ([e71a3f3](https://github.com/pelvo/figstudio/commit/e71a3f38c6c30c73041edcce3411d8117db87eeb))
* **render,web:** draw stored emoji glyphs and show symbols in browser names ([#46](https://github.com/pelvo/figstudio/issues/46)) ([0525713](https://github.com/pelvo/figstudio/commit/052571338f36df0576dc736fa214c0f4dc5c219b))


### Performance Improvements

* **ci:** run check slices in parallel and cache each target dir ([#35](https://github.com/pelvo/figstudio/issues/35)) ([2bb3ee0](https://github.com/pelvo/figstudio/commit/2bb3ee02e291e81008068d83b754fa5f49c5388d))
