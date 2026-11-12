// 「编辑项目源文件夹」对话框：增删附加目录、或更换主要目录（列表首项即主要目录）。
//
// 与「创建项目」的分工：项目名称有独立的右键「重命名」入口，因此这里只有源文件夹一节，
// 不重复设备与信任确认（信任档由侧栏设置项单独管理）。文件夹行样式与创建对话框同源，
// 保证两个入口的观感与操作习惯一致。
//
// 主要目录的更换靠「设为主要」把该项移到列表首位——不做拖拽排序：排序唯一有语义的
// 位置就是首项，拖拽带来的偏移量是纯噪声，而移首位是单一确定动作，也便于解释。
//
// 行为规格见 `workspace_replace_folders` 的文档注释。
import { useEffect, useRef, useState } from "react";
import { Dialog } from "radix-ui";
import { Folder, FolderPlus, Star, X } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../store/app";
import { useOverlays } from "../store/overlays";
import { useToasts } from "../store/toast";
import { pathLeaf } from "../lib/format";

const ghostBtnCls = "h-8 rounded-lg border border-border px-3 text-ui-sm text-foreground-subtle hover:bg-surface-hover";
const primaryBtnCls = "h-8 rounded-lg bg-brand px-4 text-ui-sm text-foreground-inverse disabled:cursor-not-allowed disabled:opacity-40";

/** 编辑项目源文件夹对话框：打开时用当前文件夹列表初始化，保存时整体替换。 */
export function EditProjectDialog(): React.JSX.Element | null {
  const workspaceId = useOverlays((s) => s.editProject);
  const close = useOverlays((s) => s.closeEditProject);
  const workspaces = useApp((s) => s.workspaces);
  const refreshWorkspaces = useApp((s) => s.refreshWorkspaces);
  const pushToast = useToasts((s) => s.push);

  const [folders, setFolders] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const cancelRef = useRef<HTMLButtonElement>(null);

  const workspace = workspaceId ? workspaces.find((w) => w.id === workspaceId) : undefined;
  // 提前固定成非空字符串：保存等闭包里 TypeScript 无法从上面的 return 收窄 workspaceId
  const targetId = workspaceId ?? "";

  // 打开时按当前工作区的真实文件夹列表初始化（已规范化路径，直接回传即可）。
  useEffect(() => {
    if (!workspaceId) return;
    setFolders(workspace?.folders.map((f) => f.path) ?? []);
    setBusy(false);
    setError("");
    // workspace 只在打开瞬间取一次初值；之后列表由本地状态驱动，避免保存后事件刷新把编辑覆盖掉
  }, [workspaceId]);

  // 打开时焦点落「取消」：编辑是非破坏性操作，焦点不抢在保存按钮上
  useEffect(() => {
    if (workspaceId) cancelRef.current?.focus();
  }, [workspaceId]);

  if (!workspaceId || !workspace) return null;

  /** 弹系统目录选择器并追加一个源文件夹；重复路径忽略。 */
  async function addFolder(): Promise<void> {
    try {
      const path = await api.pickFolder();
      if (!path) return; // 用户取消
      setFolders((current) => (current.includes(path) ? current : [...current, path]));
      setError("");
    } catch (err) {
      pushToast("danger", "选择文件夹失败", String(err));
    }
  }

  /** 移除一项；至少保留一项（列表不能为空，首项即主要目录）。 */
  function removeFolder(path: string): void {
    setFolders((current) => (current.length <= 1 ? current : current.filter((item) => item !== path)));
    setError("");
  }

  /** 把某项设为主要目录（移到首位；其余保持相对顺序）。 */
  function makePrimary(path: string): void {
    setFolders((current) => [path, ...current.filter((item) => item !== path)]);
    setError("");
  }

  async function save(): Promise<void> {
    if (busy || folders.length === 0) return;
    setBusy(true);
    setError("");
    try {
      await api.workspaceReplaceFolders(targetId, folders);
      close();
      await refreshWorkspaces();
      pushToast("ok", "已更新源文件夹", workspace?.name ?? "");
    } catch (err) {
      // 后端返回可读中文错误（跨工作区占用、worktree 守卫等），就地展示在对话框里
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      setBusy(false);
    }
  }

  const mainChanged = folders.length > 0 && folders[0] !== workspace.folders[0]?.path;

  return (
    <Dialog.Root
      open
      onOpenChange={(open) => {
        // 保存中不可关闭：请求已在飞，中途关闭会让用户误以为没生效
        if (!open && !busy) close();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[55] bg-black/60" />
        <Dialog.Content
          className="fixed left-1/2 top-1/2 z-[55] flex w-[460px] max-w-[calc(100vw-40px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-3 rounded-2xl border border-popover-border bg-popover p-5 shadow-2xl outline-none"
          onEscapeKeyDown={(e) => {
            if (busy) e.preventDefault();
          }}
          onInteractOutside={(e) => {
            if (busy) e.preventDefault();
          }}
        >
          <div className="flex items-center justify-between">
            <Dialog.Title className="text-ui-lg font-medium text-foreground">编辑源文件夹</Dialog.Title>
            <Dialog.Close
              className="rounded-md p-1 text-foreground-subtlest hover:bg-surface-hover hover:text-foreground"
              aria-label="关闭"
              disabled={busy}
            >
              <X className="size-4" />
            </Dialog.Close>
          </div>

          <Dialog.Description className="text-ui-xs text-foreground-subtle">
            项目「{workspace.name}」的可读写范围。首项是主要目录，决定会话工作目录、Git 操作与技能扫描位置。
          </Dialog.Description>

          <div className="flex flex-col gap-1.5">
            <span className="text-ui-xs text-foreground-subtle">源文件夹</span>
            <div className="flex flex-col gap-0.5">
              {folders.map((folder, index) => (
                <div key={folder} className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-ui-sm hover:bg-surface-hover">
                  <Folder className="size-3.5 shrink-0 text-foreground-subtle" />
                  <span className="min-w-0 truncate" title={folder}>
                    {pathLeaf(folder)}
                  </span>
                  {index === 0 ? (
                    <span className="shrink-0 rounded bg-selected px-1.5 py-0.5 text-ui-xs text-foreground">主要</span>
                  ) : (
                    <button
                      type="button"
                      className="shrink-0 rounded px-1.5 py-0.5 text-ui-xs text-foreground-subtlest hover:bg-menu-hover hover:text-foreground"
                      onClick={() => makePrimary(folder)}
                      title="把它设为主要目录"
                    >
                      <span className="flex items-center gap-1">
                        <Star className="size-3" /> 设为主要
                      </span>
                    </button>
                  )}
                  <span className="flex-1" />
                  <button
                    type="button"
                    className="rounded p-1 text-foreground-subtlest hover:bg-menu-hover hover:text-foreground disabled:cursor-not-allowed disabled:opacity-30"
                    aria-label={`移除文件夹 ${pathLeaf(folder)}`}
                    title={folders.length <= 1 ? "至少要保留一个源文件夹" : "移除此文件夹"}
                    disabled={folders.length <= 1 || busy}
                    onClick={() => removeFolder(folder)}
                  >
                    <X className="size-3.5" />
                  </button>
                </div>
              ))}
              <button
                type="button"
                className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-ui-sm text-foreground-subtle hover:bg-surface-hover disabled:opacity-40"
                onClick={() => void addFolder()}
                disabled={busy}
              >
                <FolderPlus className="size-3.5" /> 添加文件夹
              </button>
            </div>
          </div>

          {mainChanged && (
            <p className="rounded-lg border border-warning/40 bg-warning-surface px-2.5 py-2 text-ui-xs text-warning-surface-foreground">
              主要目录将改为「{pathLeaf(folders[0] ?? "")}」。会话工作目录、Git 操作、技能扫描随之切换；
              已绑定 git 工作树的会话会阻止这次更改（工作树的来源仓库是原主要目录）。
            </p>
          )}

          {error && (
            <p className="rounded-lg border border-destructive/40 bg-destructive-surface px-2.5 py-2 text-ui-xs text-destructive-surface-foreground">
              {error}
            </p>
          )}

          <div className="flex justify-end gap-2 pt-1">
            <button ref={cancelRef} type="button" className={ghostBtnCls} onClick={close} disabled={busy}>
              取消
            </button>
            <button
              type="button"
              className={primaryBtnCls}
              onClick={() => void save()}
              disabled={busy || folders.length === 0}
            >
              {busy ? "保存中…" : "保存"}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
