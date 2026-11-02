/**
 * 工作区 store 的 file tab 数据槽单测（对应 docs/panel-tabs/issues/04）。
 *
 * 覆盖：openFile 写入正确槽并开 tab、多文件草稿互不覆盖、关闭清槽、
 * 编辑模式与保存流程按槽隔离。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fileTabId } from "../lib/sidePane";

const readFileMock = vi.fn();
const writeFileMock = vi.fn();

vi.mock("../lib/api", () => ({
  api: {
    workspace: {
      list: vi.fn().mockResolvedValue([]),
      active: vi.fn().mockResolvedValue(null),
      listDir: vi.fn().mockResolvedValue([]),
      readFile: (...args: unknown[]) => readFileMock(...args),
      locateFile: vi.fn(),
      gitStatuses: vi.fn().mockResolvedValue({}),
      writeFile: (...args: unknown[]) => writeFileMock(...args),
      computeDiff: vi.fn(),
    },
    threads: { list: vi.fn().mockResolvedValue([]) },
    settings: { get: vi.fn() },
    providers: { list: vi.fn().mockResolvedValue([]), warnings: vi.fn().mockResolvedValue([]) },
    app: { info: vi.fn() },
    documents: { preview: vi.fn() },
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
const { useWorkspaceStore } = await import("./workspace");
const { useUiStore } = await import("./ui");

/** 构造后端文件内容的返回值。 */
function fileContent(path: string, text: string, hash = "h") {
  return {
    workspaceId: "ws1",
    folderId: "",
    path,
    content: text,
    language: "go",
    lines: 1,
    binary: false,
    truncated: false,
    size: text.length,
    hash,
  };
}

beforeEach(() => {
  readFileMock.mockReset();
  writeFileMock.mockReset();
  useAppStore.setState({
    mode: "code",
    activeWorkspace: { id: "ws1", name: "ws1", path: "/tmp/ws1", trust: "trusted", purpose: "project" } as never,
  });
  usePanelStore.setState({ open: true, maximized: false, tabs: [], activeTabId: null, recentClosed: [] });
  useWorkspaceStore.setState({ files: {}, diff: null, doc: null, docLoading: false });
  // askConfirm 默认拒绝，避免保存流程里弹出的确认框改变用例走向。
  vi.spyOn(useUiStore.getState(), "askConfirm").mockResolvedValue(false);
});

describe("file tab 数据槽", () => {
  it("openFile 把内容写进按 tab id 推导的槽，并打开面板 tab", async () => {
    readFileMock.mockResolvedValue(fileContent("a.go", "package main"));
    const opened = await useWorkspaceStore.getState().openFile("", "a.go");
    expect(opened).toBe(true);
    const tabId = fileTabId("ws1", "", "a.go");
    const slot = useWorkspaceStore.getState().files[tabId];
    expect(slot?.content?.content).toBe("package main");
    expect(slot?.draft).toBe("package main");
    expect(slot?.editorMode).toBe("read");
    // 面板里出现了同名 tab 并激活。
    const panel = usePanelStore.getState();
    expect(panel.tabs.some((tab) => tab.id === tabId)).toBe(true);
    expect(panel.activeTabId).toBe(tabId);
  });

  it("两个文件的草稿互不覆盖", async () => {
    readFileMock.mockImplementation((_ws: string, _folder: string, path: string) =>
      Promise.resolve(fileContent(path, `content of ${path}`)),
    );
    await useWorkspaceStore.getState().openFile("", "a.go");
    await useWorkspaceStore.getState().openFile("", "b.go");
    const tabA = fileTabId("ws1", "", "a.go");
    const tabB = fileTabId("ws1", "", "b.go");

    useWorkspaceStore.getState().setEditorMode(tabA, "edit");
    useWorkspaceStore.getState().setDraft(tabA, "draft of a");

    expect(useWorkspaceStore.getState().files[tabA]?.draft).toBe("draft of a");
    expect(useWorkspaceStore.getState().files[tabB]?.draft).toBe("content of b.go");
  });

  it("closeFileSlot 清空该 tab 的槽，其他 tab 不受影响", async () => {
    readFileMock.mockImplementation((_ws: string, _folder: string, path: string) =>
      Promise.resolve(fileContent(path, `content of ${path}`)),
    );
    await useWorkspaceStore.getState().openFile("", "a.go");
    await useWorkspaceStore.getState().openFile("", "b.go");
    const tabA = fileTabId("ws1", "", "a.go");

    useWorkspaceStore.getState().closeFileSlot(tabA);
    const files = useWorkspaceStore.getState().files;
    expect(files[tabA]).toBeUndefined();
    expect(Object.keys(files)).toHaveLength(1);
  });

  it("saveSlot 按槽保存并回读刷新，不影响其他 tab", async () => {
    readFileMock.mockImplementation((_ws: string, _folder: string, path: string) =>
      Promise.resolve(fileContent(path, `content of ${path}`, `hash-${path}`)),
    );
    await useWorkspaceStore.getState().openFile("", "a.go");
    await useWorkspaceStore.getState().openFile("", "b.go");
    const tabA = fileTabId("ws1", "", "a.go");
    useWorkspaceStore.getState().setEditorMode(tabA, "edit");
    useWorkspaceStore.getState().setDraft(tabA, "new content");
    writeFileMock.mockResolvedValue(undefined);
    // 保存后的回读取自 readFileMock（同一路径返回原内容——这里只断言写盘参数）。

    await useWorkspaceStore.getState().saveSlot(tabA);
    expect(writeFileMock).toHaveBeenCalledWith("ws1", "", "a.go", "new content", "hash-a.go");
    // 保存成功后回到只读。
    expect(useWorkspaceStore.getState().files[tabA]?.editorMode).toBe("read");
  });

  it("openFileAt 在短路径解析成功后把定位行写进对应槽", async () => {
    // 第一跳直接成功：内容就绪后定位行写入同一个槽。
    readFileMock.mockResolvedValue(fileContent("deep/a.go", "package main"));
    const { api } = await import("../lib/api");
    // 第一跳直接成功，locateFile 不会被走到；mock 成 reject 防意外调用。
    vi.mocked(api.workspace.locateFile).mockRejectedValue(new Error("not used"));
    await useWorkspaceStore.getState().openFileAt("deep/a.go", 42);
    const slot = useWorkspaceStore.getState().files[fileTabId("ws1", "", "deep/a.go")];
    expect(slot?.targetLine).toBe(42);
  });
});
