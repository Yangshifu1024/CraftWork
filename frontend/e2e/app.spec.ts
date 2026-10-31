import { expect, test, type Page } from "@playwright/test";

async function boot(page: Page, screenshotMode = false) {
  await page.clock.install({ time: new Date("2026-11-01T10:00:00+08:00") });
  await page.goto("/");
  await expect(page.locator(".chat-pane")).toBeVisible();
  if (screenshotMode) {
    await page.addStyleTag({
      content: `*, *::before, *::after { animation: none !important; transition: none !important; caret-color: transparent !important; }`,
    });
  }
}

async function sendMessage(page: Page, text: string) {
  const input = page.locator("#composer-input");
  await input.fill(text);
  await page.getByRole("button", { name: "发送" }).click();
}

test("发送消息、工具调用与审批闭环", async ({ page }) => {
  await boot(page);
  await sendMessage(page, "帮我修复登录刷新超时");

  const allowButton = page.getByRole("button", { name: "允许一次" });
  await expect(allowButton).toBeVisible();
  await allowButton.click();

  await expect(page.getByText("已完成修改：把刷新窗口从 10 分钟调整为 12 分钟。").first()).toBeVisible();
  await expect(page.getByText("已允许", { exact: false }).first()).toBeVisible();
});

test("设置面板与主题切换", async ({ page }) => {
  await boot(page);
  await page.locator('.side-link[title^="设置"]').click();
  await expect(page.getByRole("dialog", { name: "设置" })).toBeVisible();
  await page.locator(".settings-nav-item", { hasText: "外观" }).click();

  await page.locator(".theme-card.light").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByRole("button", { name: "关闭设置" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("界面截图基线 @screenshot", async ({ page }) => {
  await boot(page, true);
  await expect(page.getByText("修复登录刷新超时").first()).toBeVisible();
  await expect(page).toHaveScreenshot("app.png");
});

test("设置面板截图基线 @screenshot", async ({ page }) => {
  await boot(page, true);
  await page.locator('.side-link[title^="设置"]').click();
  await expect(page.getByRole("dialog", { name: "设置" })).toBeVisible();
  await expect(page).toHaveScreenshot("settings.png");
});
