/**
 * 工作区 store：文件树的加载与展开、文件预览与编辑草稿、外部修改冲突处理、
 * 差异预览与应用、Office / PDF 文档预览。
 *
 * 所有数据都按「当前激活工作区」请求，工作区来自 app store；模式切换时由 App.tsx 调用 reset() 清空。
 */
import { create } from "zustand";
import { api } from "../lib/api";
import type { DiffResult, DocumentPreview, FileContent, FileNode } from "../lib/types";
import { useAppStore } from "./app";
import { usePanelStore } from "./panel";
import { useUiStore } from "./ui";

/**
 * 工作区 store 的状态与操作。
 *
 * 关键状态说明：
 * - `tree`：按目录路径缓存的子节点列表，`"."` 是根目录；展开目录时才按需拉取；
 * - `expanded`：目录是否展开；
 * - `filter`：文件树过滤关键字；
 * - `git`：文件路径到 Git 状态字母的映射，用于在树上显示标记；
 * - `selected` / `content`：当前打开的文件路径与内容，`content` 为 null 表示没有打开文件；
 * - `editorMode`：只读或编辑；
 * - `draft`：编辑中的文本，保存成功前不写回 `content`；
 * - `targetLine`：从搜索或差异跳转时要定位到的行号；
 * - `diff`：当前差异预览的数据与视图方式（统一 / 并排）；
 * - `doc` / `docLoading`：文档预览内容与加载中标记。
 */
interface WorkspaceState {
  /** 目录路径到子节点列表的缓存，`"."` 表示根目录。 */
  tree: Record<string, FileNode[]>;
  /** 目录路径到是否展开的映射。 */
  expanded: Record<string, boolean>;
  /** 文件树过滤关键字。 */
  filter: string;
  /** 文件路径到 Git 状态字母的映射。 */
  git: Record<string, string>;
  /** 当前打开的文件路径；null 表示没有打开文件。 */
  selected: string | null;
  /** 当前打开文件的内容与元信息。 */
  content: FileContent | null;
  /** 编辑器模式：只读或编辑。 */
  editorMode: "read" | "edit";
  /** 编辑草稿；保存成功后才写回 `content`。 */
  draft: string;
  /** 需要在编辑器中定位的行号。 */
  targetLine: number | null;
  /** 差异预览：标题、原文、新文、计算结果与视图方式。 */
  diff: { title: string; oldText: string; newText: string; result: DiffResult | null; view: "unified" | "side" } | null;
  /** 文档（Word / Excel / PDF / 文本）预览内容。 */
  doc: DocumentPreview | null;
  /** 文档是否正在加载。 */
  docLoading: boolean;
  /** 加载根目录文件树与 Git 状态。 */
  loadRoot(): Promise<void>;
  /** 按需加载某个目录的子节点。 */
  loadDir(path: string): Promise<void>;
  /** 展开或收起目录；首次展开时才请求子节点。 */
  toggleDir(path: string): Promise<void>;
  /** 设置文件树过滤关键字。 */
  setFilter(value: string): void;
  /** 打开文件：Office / PDF 交给文档预览，其它文件读取文本内容。 */
  openFile(path: string, options?: { quiet?: boolean }): Promise<boolean>;
  /** 打开文件并定位到指定行。 */
  openFileAt(path: string, line: number | null): Promise<void>;
  /** 关闭当前文件并清空草稿。 */
  closeFile(): void;
  /** 切换只读 / 编辑；二进制或内容被截断的文件不允许编辑。 */
  setEditorMode(mode: "read" | "edit"): void;
  /** 更新编辑草稿。 */
  setDraft(value: string): void;
  /** 保存草稿；遇到「文件已被外部修改」时询问用户是否重新加载。 */
  save(): Promise<void>;
  /** 打开差异预览并切换到差异标签页。 */
  openDiff(title: string, oldText: string, newText: string): Promise<void>;
  /** 二次确认后把差异中的新内容整份写入文件。 */
  applyDiff(): Promise<void>;
  /** 切换差异视图（统一 / 并排）。 */
  setDiffView(view: "unified" | "side"): void;
  /** 加载文档预览。 */
  openDocument(path: string): Promise<void>;
  /** 清空所有工作区数据，模式切换时调用。 */
  reset(): void;
}

/** 取出当前激活工作区的 ID；没有激活工作区时返回空字符串。 */
function workspaceID(): string {
  return useAppStore.getState().activeWorkspace?.id ?? "";
}

/**
 * 工作区数据与文件编辑流程的状态容器。
 *
 * 初始状态：没有缓存目录、没有打开文件、只读模式、草稿为空。
 */
export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  tree: {},
  expanded: {},
  filter: "",
  git: {},
  selected: null,
  content: null,
  editorMode: "read",
  draft: "",
  targetLine: null,
  diff: null,
  doc: null,
  docLoading: false,

  // 根目录与 Git 状态并行拉取；Git 状态失败不影响文件树，退化为空映射。
  async loadRoot() {
    const id = workspaceID();
    if (!id) {
      return;
    }
    const [nodes, git] = await Promise.all([
      api.workspace.listDir(id, "."),
      api.workspace.gitStatuses(id).catch(() => ({})),
    ]);
    set((state) => ({ tree: { ...state.tree, ".": nodes }, git }));
  },

  // 懒加载单个目录：只覆盖该路径下的缓存，其余目录保持不动。
  async loadDir(path) {
    const id = workspaceID();
    if (!id) {
      return;
    }
    const nodes = await api.workspace.listDir(id, path);
    set((state) => ({ tree: { ...state.tree, [path]: nodes } }));
  },

  // 先翻转展开状态；只有当目录刚被展开且还没有缓存时才去请求子节点，
  // 再次展开不会重复请求。
  async toggleDir(path) {
    const expanded = { ...get().expanded, [path]: !get().expanded[path] };
    set({ expanded });
    if (expanded[path] && !get().tree[path]) {
      await get().loadDir(path);
    }
  },

  setFilter(value) {
    set({ filter: value });
  },

  // 打开文件：Office 与 PDF 不在编辑器里显示文本，而是走文档预览；
  // 文档预览目前只在 Work 模式提供，Code 模式下给出提示而不是静默失败。
  // 返回是否成功打开，供带行号定位的入口决定是否走短路径解析兜底。
  async openFile(path, options) {
    const id = workspaceID();
    if (!id) {
      return false;
    }
    const lower = path.toLowerCase();
    if (lower.endsWith(".docx") || lower.endsWith(".xlsx") || lower.endsWith(".pdf")) {
      const mode = useAppStore.getState().mode;
      if (mode === "work") {
        await get().openDocument(path);
        usePanelStore.getState().setTab("work", "document");
        return true;
      }
      useUiStore.getState().toast({
        title: "Office / PDF 预览在 Work 模式",
        description: "切换到 Work 模式后可以在右侧面板查看文档。",
        tone: "info",
      });
      return false;
    }
    try {
      const content = await api.workspace.readFile(id, path);
      // 打开即进入只读模式，草稿与文件内容保持一致，便于随时切到编辑。
      // targetLine 在这里复位：正常打开文件不需要定位；带行号的入口
      // （openFileAt）会在内容就绪后重新设置。
      set({
        selected: path,
        content,
        draft: content.content,
        editorMode: "read",
        targetLine: null,
      });
      // 从正文链接等入口打开时，确保右侧面板切到文件页签并展开；
      // setTab 自带 open: true，面板关闭时会被同时唤起。
      usePanelStore.getState().setTab(useAppStore.getState().mode, "files");
      return true;
    } catch (error) {
      // 静默模式用于短路径兜底的第一跳：失败后还有解析重试，不打扰用户。
      if (!options?.quiet) {
        useUiStore.getState().toast({ title: "读取文件失败", description: String(error), tone: "danger" });
      }
      return false;
    }
  },

  // 打开文件并定位到指定行。
  //
  // 顺序很重要：先等 openFile 把内容读进来（它会复位 targetLine），
  // 再设置目标行。这样编辑器的定位 effect 触发时内容与行号都已就绪；
  // 反过来先设行号会被 openFile 的复位清掉，滚动从不发生。
  //
  // 模型输出的引用常是短路径（policy.go 指代 internal/policy/policy.go），
  // 直接读取失败时调用后端的 locateFile 按文件名或路径后缀解析唯一匹配，
  // 解析结果唯一才重试打开；仍失败则保持第一次的错误提示。
  async openFileAt(path, line) {
    let opened = await get().openFile(path, { quiet: true });
    if (!opened) {
      const id = workspaceID();
      if (id) {
        try {
          const resolved = await api.workspace.locateFile(id, path);
          if (resolved && resolved !== path) {
            opened = await get().openFile(resolved);
          }
        } catch {
          // 解析失败：沿用第一次尝试的错误路径，由下方统一提示。
        }
      }
    }
    if (!opened) {
      useUiStore.getState().toast({
        title: "无法打开引用的文件",
        description: `在工作区内找不到 ${path}，请确认路径是否正确`,
        tone: "warn",
      });
      return;
    }
    set({ targetLine: line });
  },

  // 关闭文件时把草稿、模式、定位行一起复位，避免残留到下一个文件。
  closeFile() {
    set({ selected: null, content: null, draft: "", editorMode: "read", targetLine: null });
  },

  // 二进制文件或内容被截断的文件（只读到一部分）进入编辑会导致写回时丢内容，因此直接拒绝。
  setEditorMode(mode) {
    const content = get().content;
    if (mode === "edit" && (content?.binary || content?.truncated)) {
      return;
    }
    set({ editorMode: mode, draft: content?.content ?? get().draft });
  },

  setDraft(value) {
    set({ draft: value });
  },

  // 保存流程：
  // 1. 带上打开文件时拿到的 hash 写入，后端据此判断文件是否被外部改动；
  // 2. 成功后重新读取文件，用后端返回的最新内容与 hash 刷新，并回到只读模式；
  // 3. 若后端报「外部修改」，说明磁盘上的内容已经变了，弹确认框由用户选择是否重新加载；
  //    重新加载会丢弃当前草稿，所以默认按钮保持危险样式；
  // 4. 其它错误走 toast 提示，草稿保留以便用户重试。
  async save() {
    const id = workspaceID();
    const content = get().content;
    if (!id || !content) {
      return;
    }
    try {
      await api.workspace.writeFile(id, content.path, get().draft, content.hash);
      const refreshed = await api.workspace.readFile(id, content.path);
      set({ content: refreshed, draft: refreshed.content, editorMode: "read" });
      useUiStore.getState().toast({ title: "已保存", description: content.path, tone: "ok" });
    } catch (error) {
      const message = String(error);
      if (message.includes("外部修改")) {
        const reload = await useUiStore.getState().askConfirm({
          title: "文件已被外部修改",
          description: "重新加载会丢弃当前未保存的修改。",
          detail: content.path,
          confirmText: "重新加载",
          danger: true,
        });
        if (reload) {
          await get().openFile(content.path);
        }
        return;
      }
      useUiStore.getState().toast({ title: "保存失败", description: message, tone: "danger" });
    }
  },

  // 打开差异：先让后端计算差异，结果写入 store 后把右侧面板切到差异标签页。
  // 视图方式沿用上一次的选择，默认统一视图。
  async openDiff(title, oldText, newText) {
    const result = await api.workspace.computeDiff(oldText, newText);
    set({ diff: { title, oldText, newText, result, view: get().diff?.view ?? "unified" } });
    usePanelStore.getState().setTab(useAppStore.getState().mode, "diff");
  },

  // 应用差异属于覆盖写入，必须先确认；确认后整份写入新内容（不带 hash，表示允许覆盖），
  // 写完重新打开文件，让编辑器显示磁盘上的最新状态。
  async applyDiff() {
    const diff = get().diff;
    const id = workspaceID();
    if (!diff || !id) {
      return;
    }
    const confirmed = await useUiStore.getState().askConfirm({
      title: "应用全部修改？",
      description: "将把右侧的新内容写入文件。写入前会备份原文件。",
      detail: diff.title,
      confirmText: "应用修改",
      danger: true,
    });
    if (!confirmed) {
      return;
    }
    try {
      await api.workspace.writeFile(id, diff.title, diff.newText, "");
      useUiStore.getState().toast({ title: "已应用修改", description: diff.title, tone: "ok" });
      await get().openFile(diff.title);
    } catch (error) {
      useUiStore.getState().toast({ title: "应用失败", description: String(error), tone: "danger" });
    }
  },

  // 只切换视图方式，保留已计算好的差异结果。
  setDiffView(view) {
    const diff = get().diff;
    if (diff) {
      set({ diff: { ...diff, view } });
    }
  },

  // 文档预览期间置 docLoading，让面板显示加载态；失败时清标记并提示。
  async openDocument(path) {
    const id = workspaceID();
    if (!id) {
      return;
    }
    set({ docLoading: true });
    try {
      const doc = await api.documents.preview(id, path);
      set({ doc, docLoading: false });
    } catch (error) {
      set({ docLoading: false });
      useUiStore.getState().toast({ title: "读取文档失败", description: String(error), tone: "danger" });
    }
  },

  // 模式切换时整体清空：文件树、展开状态、Git 标记、打开的文件、草稿、差异与文档预览。
  reset() {
    set({ tree: {}, expanded: {}, git: {}, selected: null, content: null, draft: "", diff: null, doc: null, targetLine: null });
  },
}));
