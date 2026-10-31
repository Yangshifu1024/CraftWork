import { describe, expect, it, vi } from "vitest";
import type { DiffLine } from "../../lib/types";

vi.mock("../../lib/api", () => ({
  api: {
    workspace: {
      computeDiff: vi.fn(),
      readFile: vi.fn(),
      writeFile: vi.fn(),
    },
    threads: { list: vi.fn().mockResolvedValue([]) },
  },
}));
vi.mock("../../lib/bus", () => ({
  onBatch: () => () => undefined,
  dispatch: vi.fn(),
  isDesktop: false,
  envelope: vi.fn(),
}));

const { sideRows } = await import("./DiffPanel");

function line(kind: DiffLine["kind"], text: string, old?: number, newLine?: number): DiffLine {
  return { kind, text, old, new: newLine };
}

describe("并排差异行", () => {
  it("把删除与新增配对，并保留未配对的行", () => {
    const rows = sideRows([
      line("ctx", "a", 1, 1),
      line("del", "b", 2),
      line("add", "c", undefined, 2),
      line("del", "d", 3),
      line("del", "e", 4),
      line("add", "f", undefined, 3),
    ]);

    expect(rows).toHaveLength(4);
    expect(rows[0].left?.text).toBe("a");
    expect(rows[0].right?.text).toBe("a");
    expect(rows[1].left?.text).toBe("b");
    expect(rows[1].right?.text).toBe("c");
    expect(rows[2].left?.text).toBe("d");
    expect(rows[2].right).toBeUndefined();
    expect(rows[3].left?.text).toBe("e");
    expect(rows[3].right?.text).toBe("f");
  });
});
