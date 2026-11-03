// 策略引擎与命令分类测试：处置规则表必须与产品规格一致。
import { describe, expect, it } from "vitest";
import { Engine, parseLevel, reasonFor, levelWidestRisk } from "../src/main/policy/engine";
import { classifyCommand } from "../src/main/policy/command";
import { isSensitiveAttachment } from "../src/main/policy/attachment";

describe("策略档位处置", () => {
  it("分级档：R0 放行，R1-R3 询问，R4 拒绝", () => {
    const engine = new Engine("graded");
    expect(engine.action(false, "R0")).toBe("allow");
    expect(engine.action(false, "R1")).toBe("ask");
    expect(engine.action(false, "R2")).toBe("ask");
    expect(engine.action(false, "R3")).toBe("ask");
    expect(engine.action(false, "R4")).toBe("deny");
  });

  it("分级档 + 受限工作区：R3 直接拒绝（默认断网）", () => {
    const engine = new Engine("graded");
    expect(engine.action(true, "R3")).toBe("deny");
    expect(engine.action(true, "R2")).toBe("ask");
  });

  it("严格档：R1-R3 逐次询问；永久授权一律不可用", () => {
    const engine = new Engine("strict");
    expect(engine.action(false, "R1")).toBe("ask");
    expect(engine.action(false, "R4")).toBe("deny");
    expect(engine.allowsPermanent(false, "R1")).toBe(false);
    expect(engine.allowsPermanent(false, "R3")).toBe(false);
  });

  it("只读档：R0 放行，其余拒绝", () => {
    const engine = new Engine("readonly");
    expect(engine.action(false, "R0")).toBe("allow");
    expect(engine.action(false, "R1")).toBe("deny");
    expect(engine.action(false, "R2")).toBe("deny");
  });

  it("自由档：放行一切（含 R4 与受限 R3）", () => {
    const engine = new Engine("free");
    expect(engine.action(false, "R4")).toBe("allow");
    expect(engine.action(true, "R3")).toBe("allow");
  });

  it("永久授权：仅分级 + 非受限 + R1/R2", () => {
    const engine = new Engine("graded");
    expect(engine.allowsPermanent(false, "R1")).toBe(true);
    expect(engine.allowsPermanent(false, "R2")).toBe(true);
    expect(engine.allowsPermanent(true, "R1")).toBe(false);
    expect(engine.allowsPermanent(false, "R3")).toBe(false);
  });

  it("未知档位回退分级", () => {
    expect(parseLevel("anything")).toBe("graded");
    expect(parseLevel("strict")).toBe("strict");
  });

  it("风险原因与档位最宽风险", () => {
    expect(reasonFor("R0")).toBe("只读操作");
    expect(levelWidestRisk("free")).toBe("R4");
    expect(levelWidestRisk("readonly")).toBe("R0");
    expect(levelWidestRisk("graded")).toBe("R3");
  });
});

describe("命令分类", () => {
  it("破坏性命令判 R4", () => {
    expect(classifyCommand("sudo rm -rf /").risk).toBe("R4");
    expect(classifyCommand("git push --force").risk).toBe("R4");
    expect(classifyCommand("git reset --hard HEAD~1").risk).toBe("R4");
  });

  it("网络命令判 R3", () => {
    expect(classifyCommand("curl https://example.com").risk).toBe("R3");
    expect(classifyCommand("npm install left-pad").risk).toBe("R3");
    expect(classifyCommand("git clone https://github.com/x/y").risk).toBe("R3");
  });

  it("普通命令判 R2", () => {
    expect(classifyCommand("ls -la").risk).toBe("R2");
    expect(classifyCommand("go test ./...").risk).toBe("R2");
  });

  it("空命令 R2", () => {
    expect(classifyCommand("")).toEqual({ risk: "R2", reason: "执行命令" });
  });
});

describe("敏感附件文件名", () => {
  it("密钥与环境文件被拦截", () => {
    expect(isSensitiveAttachment(".env")).toBe(true);
    expect(isSensitiveAttachment(".env.local")).toBe(true);
    expect(isSensitiveAttachment("id_rsa")).toBe(true);
    expect(isSensitiveAttachment("server.pem")).toBe(true);
    expect(isSensitiveAttachment("key.KEY")).toBe(true);
  });

  it("普通文件放行", () => {
    expect(isSensitiveAttachment("main.go")).toBe(false);
    expect(isSensitiveAttachment("notes.txt")).toBe(false);
    expect(isSensitiveAttachment("environment")).toBe(false);
  });
});
