---
branch: fix/web-deploy-secrets
pr:
title: Release web deploy gets the cloudflare environment's secrets
---

## P5 — Delivery
- [x] A release's web deploy (release-please `web` job calling `web-deploy.yml`) passes its secrets, so the `cloudflare` environment's `CLOUDFLARE_API_TOKEN` reaches the deploy step
- [x] A Node test in the `tests-npm` slice fails if a job that calls `web-deploy.yml` stops passing its secrets

## Progress

### 2026-11-11T080000Z web-deploy-secrets
- What changed and how it was checked: v0.3.0's release run deployed nothing; its `web / deploy figstudio.pelvo.dev` job stopped at "CLOUDFLARE_API_TOKEN or CLOUDFLARE_ACCOUNT_ID is not set". Both secrets exist in the `cloudflare` environment, and the manual `web-deploy` run for v0.2.0 with the same environment deployed. The difference is the call: a reusable workflow sees only the secrets its caller passes, and the release's `web` job passed none. It now has `secrets: inherit`. The repository has no repository-level secrets, so the deploy job still sees only its own environment's two. New `npm/figstudio-mcp/test/release-workflow.test.mjs` fails on `main` ("release-please.yml job web passes its secrets to web-deploy") and passes with the change; the `tests-npm` slice is 12/12. actionlint passes on both workflows.
- Not verified: a release-triggered deploy on GitHub, until the next release. v0.3.0 itself was deployed by hand from the release run's `web-dist` artifact (same `figstudio_bg.wasm` hash).
