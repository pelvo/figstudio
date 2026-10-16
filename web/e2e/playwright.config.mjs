import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  testMatch: "relay.spec.mjs",
  workers: 1,
  use: { browserName: "chromium", viewport: { width: 1280, height: 800 } },
});
