// 上下文窗口徽章格式化：纯函数直测（Intl compact，en-US）。
import { describe, expect, it } from "vitest";
import { formatContextK, formatContextTitle } from "../src/renderer/src/lib/format-context";

describe("formatContextK（徽章紧凑文案，输入 token 数）", () => {
  it("1000 → 1K", () => {
    expect(formatContextK(1000)).toBe("1K");
  });

  it("128K → 128K（spec §6 示例）", () => {
    expect(formatContextK(128_000)).toBe("128K");
  });

  it("1048.6K 约整 → 1M（spec §6 示例）", () => {
    expect(formatContextK(1_048_576)).toBe("1M");
    expect(formatContextK(1_048_600)).toBe("1M");
  });

  it("不足 1000 原样展示；中段值保留一位小数", () => {
    expect(formatContextK(999)).toBe("999");
    expect(formatContextK(1500)).toBe("1.5K");
  });
});

describe("formatContextTitle（徽章 title，完整 token 数）", () => {
  it("输出千分位完整数字 + tokens 后缀", () => {
    expect(formatContextTitle(1_048_576)).toBe("1,048,576 tokens");
    expect(formatContextTitle(128_000)).toBe("128,000 tokens");
  });
});
