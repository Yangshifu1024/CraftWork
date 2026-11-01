import { describe, expect, it } from "vitest";
import { matchPathLineRef } from "./markdownRef";

describe("matchPathLineRef", () => {
  it("匹配单层文件名引用", () => {
    expect(matchPathLineRef("runner.go:31")).toEqual({ path: "runner.go", line: 31 });
  });

  it("匹配多层目录引用", () => {
    expect(matchPathLineRef("internal/agent/runner.go:208")).toEqual({
      path: "internal/agent/runner.go",
      line: 208,
    });
  });

  it("匹配带点目录与多点文件名", () => {
    expect(matchPathLineRef("internal/app.v2/MessageView.test.tsx:12")).toEqual({
      path: "internal/app.v2/MessageView.test.tsx",
      line: 12,
    });
  });

  it("接受 1 到 5 位行号", () => {
    expect(matchPathLineRef("a.go:1")).toEqual({ path: "a.go", line: 1 });
    expect(matchPathLineRef("a.go:99999")).toEqual({ path: "a.go", line: 99999 });
  });

  it("行号为 0 时原样返回，由调用方按「不定位到行」处理", () => {
    expect(matchPathLineRef("a.go:0")).toEqual({ path: "a.go", line: 0 });
  });

  it("拒绝混合文本，只做整体判定", () => {
    expect(matchPathLineRef("见 runner.go:31 的定义")).toBeNull();
    expect(matchPathLineRef("foo.go:1 bar.go:2")).toBeNull();
  });

  it("拒绝无行号的纯路径", () => {
    expect(matchPathLineRef("internal/agent/runner.go")).toBeNull();
  });

  it("拒绝带列号的引用", () => {
    expect(matchPathLineRef("runner.go:31:5")).toBeNull();
  });

  it("拒绝无扩展名文件", () => {
    expect(matchPathLineRef("Makefile:3")).toBeNull();
  });

  it("拒绝超过 5 位的行号", () => {
    expect(matchPathLineRef("a.go:123456")).toBeNull();
  });

  it("拒绝超过 8 位的扩展名", () => {
    expect(matchPathLineRef("backup.abcdefghij:1")).toBeNull();
  });
});
