// 行级 diff 算法测试。
import { describe, expect, it } from "vitest";
import { diff } from "../src/main/core/editor";

describe("diff", () => {
  it("无差异返回空", () => {
    const result = diff("a\nb\nc", "a\nb\nc");
    expect(result.additions).toBe(0);
    expect(result.deletions).toBe(0);
    expect(result.hunks).toHaveLength(0);
  });

  it("单行修改产生一个 hunk（含上下文）", () => {
    const result = diff("a\nb\nc\nd\ne", "a\nb\nX\nd\ne");
    expect(result.additions).toBe(1);
    expect(result.deletions).toBe(1);
    expect(result.hunks).toHaveLength(1);
    const hunk = result.hunks[0]!;
    expect(hunk.lines.some((l) => l.kind === "del" && l.text === "c")).toBe(true);
    expect(hunk.lines.some((l) => l.kind === "add" && l.text === "X")).toBe(true);
  });

  it("末尾追加", () => {
    const result = diff("a", "a\nb\nc");
    expect(result.additions).toBe(2);
    expect(result.deletions).toBe(0);
  });

  it("CRLF 归一化", () => {
    const result = diff("a\r\nb", "a\nb");
    expect(result.additions).toBe(0);
    expect(result.deletions).toBe(0);
  });

  it("超大输入标记截断", () => {
    const big = Array.from({ length: 30000 }, (_, i) => `line ${i}`).join("\n");
    const result = diff(big, `${big}\nx`);
    expect(result.truncated).toBe(true);
  });

  it("小规模随机一致性：增删数与行内容对得上", () => {
    const oldText = "1\n2\n3\n4\n5";
    const newText = "1\n3\n4\n5.5\n6";
    const result = diff(oldText, newText);
    expect(result.deletions).toBe(2); // 2、5 移除
    expect(result.additions).toBe(2); // 5.5、6 新增
  });
});
