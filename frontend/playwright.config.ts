import { defineConfig } from "@playwright/test";

const channel = process.env.PLAYWRIGHT_CHANNEL;

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  expect: {
    timeout: 15_000,
    toHaveScreenshot: { animations: "disabled", maxDiffPixelRatio: 0.01 },
  },
  fullyParallel: false,
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:9245",
    // channel 的目标类型就是 string，环境变量缺省时为 undefined，无需断言。
    channel,
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "npm run dev -- --port 9245",
    url: "http://127.0.0.1:9245",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
  projects: [
    { name: "desktop-1440", use: { viewport: { width: 1440, height: 900 } } },
    { name: "narrow-1100", use: { viewport: { width: 1100, height: 800 } } },
  ],
});
