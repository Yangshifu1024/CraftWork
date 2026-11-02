/**
 * 面板多标签页 store 的单元测试。
 *
 * 覆盖 spec 第 2、3 节在 store 层的行为：打开入口、关闭收敛、工作区分桶、
 * 切线程恢复、线程删除清理、最近关闭重开与终端的全局公共性。
 *
 * store 的上下文（工作区 + 线程）通过直接 setState 驱动；订阅接线在
 * panelWiring.ts（应用入口加载），测试里手动调 syncScope。
 *
 * 注意：panel store 的记忆桶与终端全局标志是模块级单例，用例之间用
 * 递增的独立工作区 id 隔离（每个用例拿到全新的桶），并在用例前关闭
 * 终端 singleton 清掉全局标志。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fileTabId, TERMINAL_TAB_ID } from "../lib/sidePane";
import type { WorkspaceView } from "../lib/types";

vi.mock("../lib/api", () => ({
  api: {
    workspace: {
      list: vi.fn().mockResolvedValue([]),
      active: vi.fn().mockResolvedValue(null),
      listDir: vi.fn().mockResolvedValue([]),
      readFile: vi.fn(),
      locateFile: vi.fn(),
      gitStatuses: vi.fn().mockResolvedValue({}),
    },
    threads: { list: vi.fn().mockResolvedValue([]) },
    settings: { get: vi.fn() },
    providers: { list: vi.fn().mockResolvedValue([]), warnings: vi.fn().mockResolvedValue([]) },
    app: { info: vi.fn() },
  },
}));
vi.mock("../lib/bus", () => ({
  onBatch: () => () => undefined,
  dispatch: vi.fn(),
  isDesktop: false,
  envelope: vi.fn(),
}));

const { useAppStore } = await import("./app");
const { usePanelStore } = await import("./panel");
const { useThreadStore } = await import("./threads");

/** 构造一个最小的 WorkspaceView；测试只关心 id。 */
function workspace(id: string): WorkspaceView {
  return { id, name: id, path: `/tmp/${id}`, trust: "trusted", purpose: "project" } as WorkspaceView;
}

/** 递增的工作区序号：每个用例（乃至用例内的多工作区场景）拿全新的桶。 */
let wsSeq = 0;
function freshWs(): string {
  wsSeq += 1;
  return `ws${wsSeq}`;
}

/** 把全局上下文设为「工作区 ws 的线程 thread」。 */
function setContext(ws: string, thread: string | null): void {
  useAppStore.setState({ mode: "code", activeWorkspace: ws ? workspace(ws) : null });
  useThreadStore.setState({ activeID: { code: thread, work: null } });
}

/** 打开一个 file tab 的便捷方法。 */
function openFile(path: string): void {
  usePanelStore.getState().openFileTab({ folderId: "", path, title: path });
}

beforeEach(() => {
  // zustand store 是模块级单例：恢复投影初始状态，并清掉终端全局标志
  // （openTerminalTab 会同步写进所有已存在的桶，必须先关闭再跑用例）。
  useAppStore.setState({ mode: "code", activeWorkspace: null });
  useThreadStore.setState({ activeID: { code: null, work: null } });
  usePanelStore.setState({
    open: true,
    maximized: false,
    narrow: false,
    tabs: [],
    activeTabId: null,
    recentClosed: [],
  });
  usePanelStore.getState().closeTab(TERMINAL_TAB_ID);
});

describe("打开入口", () => {
  it("openFileTab 追加 tab 并激活，重复打开幂等", () => {
    const ws = freshWs();
    setContext(ws, "t1");
    const panel = usePanelStore.getState();
    panel.openFileTab({ folderId: "", path: "a.go", title: "a.go" });
    panel.openFileTab({ folderId: "", path: "b.go", title: "b.go" });
    panel.openFileTab({ folderId: "", path: "a.go", title: "a.go" });

    const state = usePanelStore.getState();
    expect(state.tabs).toHaveLength(2);
    expect(state.activeTabId).toBe(fileTabId(ws, "", "a.go"));
    expect(state.open).toBe(true);
    // 归属冻结为打开时的上下文。
    expect(state.tabs.every((tab) => tab.workspaceId === ws && tab.threadId === "t1")).toBe(true);
  });

  it("openDiffTab 是 singleton：已存在时只刷新标题并聚焦", () => {
    setContext(freshWs(), "t1");
    usePanelStore.getState().openDiffTab("第一处差异");
    usePanelStore.getState().openDiffTab("第二处差异");
    const state = usePanelStore.getState();
    expect(state.tabs.filter((tab) => tab.type === "diff")).toHaveLength(1);
    expect(state.tabs.find((tab) => tab.type === "diff")?.title).toBe("第二处差异");
    expect(state.activeTabId).toBe("diff");
  });

  it("openTerminalTab 二次调用只聚焦", () => {
    setContext(freshWs(), "t1");
    usePanelStore.getState().openTerminalTab();
    usePanelStore.getState().openTerminalTab();
    const state = usePanelStore.getState();
    expect(state.tabs.filter((tab) => tab.id === TERMINAL_TAB_ID)).toHaveLength(1);
    expect(state.activeTabId).toBe(TERMINAL_TAB_ID);
  });
});

describe("关闭收敛", () => {
  it("关闭末个可见 tab 后面板收起并退出最大化", () => {
    const ws = freshWs();
    setContext(ws, "t1");
    usePanelStore.setState({ maximized: true });
    openFile("a.go");
    usePanelStore.getState().closeTab(fileTabId(ws, "", "a.go"));
    const state = usePanelStore.getState();
    expect(state.tabs).toHaveLength(0);
    expect(state.open).toBe(false);
    expect(state.maximized).toBe(false);
  });

  it("关闭中间 tab 时激活邻居，面板保持展开", () => {
    const ws = freshWs();
    setContext(ws, "t1");
    openFile("a.go");
    openFile("b.go");
    openFile("c.go");
    usePanelStore.getState().closeTab(fileTabId(ws, "", "b.go"));
    const state = usePanelStore.getState();
    expect(state.activeTabId).toBe(fileTabId(ws, "", "c.go"));
    expect(state.open).toBe(true);
  });

  it("关闭的 tab 进入最近关闭栈，重开时归属重置为当前线程", () => {
    const ws = freshWs();
    setContext(ws, "t1");
    openFile("a.go");
    usePanelStore.getState().closeTab(fileTabId(ws, "", "a.go"));
    expect(usePanelStore.getState().recentClosed).toHaveLength(1);

    // 切到线程 t2：栈里的 tab 属于 t1，对 t2 不可见，重开被拒绝。
    setContext(ws, "t2");
    usePanelStore.getState().reopenRecentClosed(fileTabId(ws, "", "a.go"));
    expect(usePanelStore.getState().tabs).toHaveLength(0);

    // 切回 t1 后重开成功，归属变成 t1。
    setContext(ws, "t1");
    usePanelStore.getState().reopenRecentClosed(fileTabId(ws, "", "a.go"));
    const state = usePanelStore.getState();
    expect(state.tabs).toHaveLength(1);
    expect(state.tabs[0].threadId).toBe("t1");
    expect(state.recentClosed).toHaveLength(0);
  });
});

describe("工作区分桶", () => {
  it("切换工作区后 tab 互不串台，切回恢复", () => {
    const w1 = freshWs();
    setContext(w1, "t1");
    openFile("a.go");

    const w2 = freshWs();
    setContext(w2, "t1");
    usePanelStore.getState().syncScope();
    expect(usePanelStore.getState().tabs).toHaveLength(0);
    // 新工作区开自己的文件。
    openFile("b.ts");
    expect(usePanelStore.getState().tabs.map((tab) => tab.title)).toEqual(["b.ts"]);

    // 切回 w1：a.go 还在，且激活项恢复。
    setContext(w1, "t1");
    usePanelStore.getState().syncScope();
    const state = usePanelStore.getState();
    expect(state.tabs.map((tab) => tab.title)).toEqual(["a.go"]);
    expect(state.activeTabId).toBe(fileTabId(w1, "", "a.go"));
  });

  it("终端是全局公共 tab：切换工作区后仍然可见", () => {
    const w1 = freshWs();
    setContext(w1, "t1");
    usePanelStore.getState().openTerminalTab();

    const w2 = freshWs();
    setContext(w2, "t1");
    usePanelStore.getState().syncScope();
    const state = usePanelStore.getState();
    expect(state.tabs.some((tab) => tab.id === TERMINAL_TAB_ID)).toBe(true);
    expect(state.activeTabId).toBe(TERMINAL_TAB_ID);
  });

  it("关闭终端 tab 后全局消失（切工作区也不复活）", () => {
    const w1 = freshWs();
    setContext(w1, "t1");
    usePanelStore.getState().openTerminalTab();
    usePanelStore.getState().closeTab(TERMINAL_TAB_ID);
    expect(terminalTabsIn(usePanelStore.getState())).toBe(0);

    const w2 = freshWs();
    setContext(w2, "t1");
    usePanelStore.getState().syncScope();
    expect(terminalTabsIn(usePanelStore.getState())).toBe(0);
  });

  it("切换线程时恢复显式偏好；无偏好保持当前开合（不因无 tab 强制收起）", () => {
    const ws = freshWs();
    setContext(ws, "t1");
    openFile("a.go");
    // 用户在线程 t1 显式收起面板。
    usePanelStore.getState().setOpen(false);

    // 切到没有 tab 的线程 t2：无偏好 → 保持收起。
    setContext(ws, "t2");
    usePanelStore.getState().syncScope();
    expect(usePanelStore.getState().open).toBe(false);

    // 展开后切到另一个无 tab 线程：无偏好 → 保持展开（文件树兜底可见），
    // 不被「无可见 tab」强制收起——初始启动与切工作区同理。
    usePanelStore.getState().setOpen(true);
    setContext(ws, "t2");
    usePanelStore.getState().syncScope();
    const ws2 = freshWs();
    setContext(ws2, "t1");
    usePanelStore.getState().syncScope();
    expect(usePanelStore.getState().open).toBe(true);

    // 切回 t1：恢复 t1 的偏好（收起）。
    setContext(ws, "t1");
    usePanelStore.getState().syncScope();
    expect(usePanelStore.getState().open).toBe(false);
  });
});

describe("线程清理", () => {
  it("pruneThread 清掉该线程名下的 tab，终端保留，不入最近关闭栈", () => {
    const ws = freshWs();
    // 在线程 t2 名下开 other.go（真实的归属冻结路径），切回 t1 再开 a.go。
    setContext(ws, "t2");
    openFile("other.go");
    setContext(ws, "t1");
    openFile("a.go");
    usePanelStore.getState().openTerminalTab();

    usePanelStore.getState().pruneThread("t2");
    let state = usePanelStore.getState();
    expect(state.tabs.map((tab) => tab.id)).toEqual([fileTabId(ws, "", "a.go"), TERMINAL_TAB_ID]);

    usePanelStore.getState().pruneThread("t1");
    state = usePanelStore.getState();
    // 终端是公共类型，不随线程清理。
    expect(state.tabs.map((tab) => tab.id)).toEqual([TERMINAL_TAB_ID]);
    // 被清理的 tab 不进最近关闭栈。
    expect(state.recentClosed).toHaveLength(0);
  });
});

describe("偏好持久化", () => {
  it("宽度与开合写入 localStorage；重新加载模块后恢复", async () => {
    try {
      usePanelStore.getState().setWidth(500);
      usePanelStore.getState().setOpen(false);
      expect(localStorage.getItem("cw-panel-width")).toBe("500");
      expect(localStorage.getItem("cw-panel-open")).toBe("false");

      // 重新加载模块：初始值从 localStorage 恢复。
      vi.resetModules();
      const { usePanelStore: fresh } = await import("./panel");
      expect(fresh.getState().width).toBe(500);
      expect(fresh.getState().open).toBe(false);
    } finally {
      localStorage.removeItem("cw-panel-width");
      localStorage.removeItem("cw-panel-open");
    }
  });

  it("损坏的持久化值回退默认（宽度夹取回合法区间，开合缺失视为展开）", async () => {
    try {
      localStorage.setItem("cw-panel-width", "99999");
      localStorage.setItem("cw-panel-open", "not-a-bool");
      vi.resetModules();
      const { usePanelStore: fresh } = await import("./panel");
      expect(fresh.getState().width).toBe(760);
      // 只有显式写入的 "false" 视为收起。
      expect(fresh.getState().open).toBe(true);
    } finally {
      localStorage.removeItem("cw-panel-width");
      localStorage.removeItem("cw-panel-open");
    }
  });

  it("窄窗口的临时收起不覆盖持久化的开合偏好", () => {
    try {
      usePanelStore.getState().setOpen(true);
      usePanelStore.getState().setNarrow(true);
      expect(usePanelStore.getState().open).toBe(false);
      usePanelStore.getState().setNarrow(false);
      // 离开窄窗口后开合仍为展开，且 localStorage 记录未被临时态污染。
      expect(usePanelStore.getState().open).toBe(true);
      expect(localStorage.getItem("cw-panel-open")).toBe("true");
    } finally {
      localStorage.removeItem("cw-panel-open");
    }
  });
});

/** 统计当前投影里终端 tab 的数量。 */
function terminalTabsIn(state: { tabs: ReturnType<typeof usePanelStore.getState>["tabs"] }): number {
  return state.tabs.filter((tab) => tab.id === TERMINAL_TAB_ID).length;
}
