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

test("首轮消息自动命名新线程", async ({ page }) => {
  await boot(page);
  await page.getByRole("button", { name: "新建线程" }).click();
  await sendMessage(page, "帮我修复登录刷新超时");

  await expect(
    page.locator(".thread-title", { hasText: "帮我修复登录刷新超时" }),
  ).toBeVisible();
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

test("添加自定义提供商与模型", async ({ page }) => {
  await boot(page);
  await page.locator('.side-link[title^="设置"]').click();
  await expect(page.getByRole("dialog", { name: "设置" })).toBeVisible();
  await page.locator(".settings-nav-item", { hasText: "模型与提供商" }).click();

  await page.getByRole("button", { name: "手动添加" }).click();
  await page.getByPlaceholder("例如 公司网关").fill("我的网关");
  await page.getByPlaceholder("https://api.example.com/v1").fill("https://gateway.example.com/v1");
  await page.getByLabel("模型 1 ID").fill("gateway-model");
  await page.getByLabel("模型 1 名称").fill("Gateway Model");
  await page.getByRole("button", { name: "保存提供商" }).click();

  await expect(page.locator(".provider-name", { hasText: "我的网关" })).toBeVisible();
  await expect(page.locator(".provider-card", { hasText: "我的网关" })).toContainText(
    "gateway.example.com",
  );
});

test("从模型目录添加提供商", async ({ page }) => {
  await boot(page);
  await page.locator('.side-link[title^="设置"]').click();
  await expect(page.getByRole("dialog", { name: "设置" })).toBeVisible();
  await page.locator(".settings-nav-item", { hasText: "模型与提供商" }).click();

  await page.getByRole("button", { name: "从模型目录添加" }).click();
  await expect(page.getByText("从模型目录添加", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: /Groq/ }).click();
  await page.getByRole("checkbox", { name: "选择 Qwen3.8 27B" }).check();
  await page.getByRole("button", { name: "导入提供商" }).click();

  await expect(page.locator(".provider-name", { hasText: "Groq" })).toBeVisible();
});

test("在提供商编辑器中设置密钥并保存", async ({ page }) => {
  await boot(page);
  await page.locator('.side-link[title^="设置"]').click();
  await expect(page.getByRole("dialog", { name: "设置" })).toBeVisible();
  await page.locator(".settings-nav-item", { hasText: "模型与提供商" }).click();
  await page.getByRole("button", { name: "手动添加" }).click();

  const keyInput = page.getByLabel("API Key", { exact: true });
  await expect(keyInput).toBeVisible();
  await page.getByRole("checkbox", { name: "需要 API Key" }).uncheck();
  await expect(keyInput).toHaveCount(0);
  await page.getByRole("checkbox", { name: "需要 API Key" }).check();

  await page.getByPlaceholder("例如 公司网关").fill("密钥网关");
  await page.getByPlaceholder("https://api.example.com/v1").fill("https://gateway.example.com/v1");
  await page.getByLabel("模型 1 ID").fill("gateway-model");
  await keyInput.fill("sk-e2e-test-123456");
  await page.getByRole("button", { name: "保存提供商" }).click();

  const card = page.locator(".provider-card", { hasText: "密钥网关" });
  await expect(card.getByText("可用")).toBeVisible();
  await expect(page.getByText("还需配置 API Key")).toHaveCount(0);

  // 重新编辑时密钥不回显，只显示掩码提示。
  await card.getByRole("button", { name: /编辑/ }).click();
  await expect(page.getByLabel("API Key", { exact: true })).toHaveValue("");
  await expect(page.getByLabel("API Key", { exact: true })).toHaveAttribute("placeholder", /已配置/);
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

test("模型目录截图基线 @screenshot", async ({ page }) => {
  await boot(page, true);
  await page.locator('.side-link[title^="设置"]').click();
  await expect(page.getByRole("dialog", { name: "设置" })).toBeVisible();
  await page.locator(".settings-nav-item", { hasText: "模型与提供商" }).click();
  await page.getByRole("button", { name: "从模型目录添加" }).click();
  await page.getByRole("button", { name: /Groq/ }).click();
  await expect(page.getByRole("checkbox", { name: "选择 Qwen3.8 27B" })).toBeVisible();
  await expect(page).toHaveScreenshot("catalog.png");
});
