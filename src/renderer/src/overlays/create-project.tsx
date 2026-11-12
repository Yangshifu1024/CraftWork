// 「创建项目」对话框：项目名称（未编辑时提交空串，
// 由后端从主要目录名推导、重名自动加序号）、设备（本地部署固定「此电脑」）、
// 源文件夹多选（第一个自动成为「主要」，可移除、可追加），提交前弹「信任此文件夹
// 吗？」确认——每次创建都确认，信任结果不持久化。确认通过后以可信档创建项目，
// 并复用侧栏编排（刷新列表 + 进入该项目草稿态）。
//
// 弹层结构：遮罩 / Esc / 焦点圈定由 radix Dialog 承担；信任确认是内部第二视图
// （不嵌套浮层）：Esc 回表单（onEscapeKeyDown 分视图），焦点落「取消」——信任
// 必须是明确选择，不提供 Enter 直通。样式走语义 token（bg-popover / border-border /
// bg-input / bg-brand / bg-surface-hover），深浅两套主题同源。
import { useEffect, useRef, useState } from "react";
import { Dialog } from "radix-ui";
import { ChevronDown, Folder, FolderPlus, Monitor, X } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../store/app";
import { useOverlays } from "../store/overlays";
import { useToasts } from "../store/toast";
import { isImeComposing } from "../lib/ime";
import { pathLeaf } from "../lib/format";

/** 对话框的两个视图：表单与信任确认。 */
type View = "form" | "trust";

const inputCls = "h-8 w-full rounded-lg border border-border bg-input px-2.5 text-ui-sm outline-none focus:border-foreground-subtle";
const ghostBtnCls = "h-8 rounded-lg border border-border px-3 text-ui-sm text-foreground-subtle hover:bg-surface-hover";
const primaryBtnCls = "h-8 rounded-lg bg-brand px-4 text-ui-sm text-foreground-inverse disabled:cursor-not-allowed disabled:opacity-40";

/** 创建项目对话框：表单 → 信任确认两视图切换（同一浮层内部分流，不嵌套浮层）；每次打开重置，Esc / 点遮罩行为随视图分流。 */
export function CreateProjectDialog(): React.JSX.Element | null {
  const open = useOverlays((s) => s.createProject);
  const close = useOverlays((s) => s.closeCreateProject);
  const refreshWorkspaces = useApp((s) => s.refreshWorkspaces);
  const newDraft = useApp((s) => s.newDraft);
  const pushToast = useToasts((s) => s.push);

  const [view, setView] = useState<View>("form");
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const [folders, setFolders] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  const nameInputRef = useRef<HTMLInputElement>(null);
  const trustCancelRef = useRef<HTMLButtonElement>(null);

  // 每次打开都回到初始状态，避免上一次的输入残留。
  useEffect(() => {
    if (open) {
      setView("form");
      setName("");
      setNameEdited(false);
      setFolders([]);
      setBusy(false);
    }
  }, [open]);

  // 打开时聚焦名称输入框；信任确认视图把焦点放在「取消」——信任必须是明确选择。
  useEffect(() => {
    if (!open) return;
    if (view === "form") nameInputRef.current?.focus();
    else trustCancelRef.current?.focus();
  }, [open, view]);

  if (!open) return null;

  /** 弹系统目录选择器并追加一个源文件夹；重复路径忽略。 */
  async function addFolder(): Promise<void> {
    try {
      const path = await api.pickFolder();
      if (!path) return; // 用户取消
      setFolders((current) => (current.includes(path) ? current : [...current, path]));
    } catch (err) {
      pushToast("danger", "选择文件夹失败", String(err));
    }
  }

  function removeFolder(path: string): void {
    setFolders((current) => current.filter((item) => item !== path));
  }

  /** 表单提交：进入信任确认视图。无源文件夹时按钮已禁用，这里兜底。 */
  function startTrust(): void {
    if (folders.length === 0 || busy) return;
    setView("trust");
  }

  /** 信任确认通过：以可信档创建项目，成功后刷新列表并进入该项目草稿态。 */
  async function create(): Promise<void> {
    if (busy) return;
    setBusy(true);
    try {
      // 未编辑过名称时交空串，由后端从主要目录名推导（重名自动加序号）。
      const ws = await api.workspaceAddProject(nameEdited ? name.trim() : "", folders, "trusted");
      close();
      await refreshWorkspaces();
      newDraft(ws.id);
      pushToast("ok", "已创建项目", nameEdited && name.trim() ? name.trim() : pathLeaf(folders[0] ?? ""));
    } catch (err) {
      pushToast("danger", "创建项目失败", String(err));
      // 创建失败回到表单，保留已填内容便于修正后重试。
      setView("form");
      setBusy(false);
    }
  }

  return (
    <Dialog.Root
      open
      onOpenChange={(o) => {
        if (!o && !busy) close();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[55] bg-black/60" />
        <Dialog.Content
          aria-label="创建项目"
          aria-describedby={undefined}
          className="fixed left-1/2 top-1/2 z-[55] flex max-h-[min(85vh,640px)] w-[440px] max-w-[calc(100vw-40px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 overflow-y-auto rounded-2xl border border-popover-border bg-popover p-5 shadow-2xl outline-none"
          onEscapeKeyDown={(e) => {
            // radix 事件是原生 KeyboardEvent：组合中的 Esc（取消候选）不关窗
            if (e.isComposing || e.keyCode === 229) {
              e.preventDefault();
              return;
            }
            // 信任确认视图 Esc 回表单；创建进行中不可关
            if (view === "trust" || busy) e.preventDefault();
            if (view === "trust") setView("form");
          }}
          onInteractOutside={(e) => {
            // 信任确认视图点遮罩等同取消：回表单；创建进行中不可关
            if (busy) e.preventDefault();
            else if (view === "trust") {
              e.preventDefault();
              setView("form");
            }
          }}
        >
          <div className="flex items-center justify-between">
            <Dialog.Title className="text-ui-lg font-medium text-foreground">
              {view === "form" ? "创建项目" : folders.length > 1 ? "信任这些文件夹吗？" : "信任此文件夹吗？"}
            </Dialog.Title>
            <Dialog.Close
              className="rounded-md p-1 text-foreground-subtlest hover:bg-surface-hover hover:text-foreground"
              aria-label="关闭"
              disabled={busy}
            >
              <X className="size-4" />
            </Dialog.Close>
          </div>

          {view === "form" ? (
            <>
              <div className="flex flex-col gap-1.5">
                <label htmlFor="create-project-name" className="text-ui-xs text-foreground-subtle">
                  项目名称
                </label>
                <input
                  id="create-project-name"
                  ref={nameInputRef}
                  className={inputCls}
                  value={name}
                  placeholder="新项目"
                  onChange={(e) => {
                    setName(e.target.value);
                    setNameEdited(true);
                  }}
                  onKeyDown={(e) => {
                    // Enter 视为点击「创建项目」：进入信任确认。
                    if (e.key === "Enter" && !isImeComposing(e)) {
                      e.preventDefault();
                      startTrust();
                    }
                  }}
                />
              </div>

              {/* 设备是纯展示的静态信息（本地部署只有此电脑），没有可关联的表单控件 */}
              <div className="flex flex-col gap-1.5">
                <span className="text-ui-xs text-foreground-subtle">设备</span>
                <span className="flex h-8 items-center gap-2 rounded-lg border border-border bg-surface px-2.5 text-ui-sm text-foreground-subtle">
                  <Monitor className="size-3.5" />
                  此电脑
                  <ChevronDown className="ml-auto size-3.5 text-foreground-subtlest" />
                </span>
              </div>

              <div className="flex flex-col gap-1.5">
                <span className="text-ui-xs text-foreground-subtle">源文件夹</span>
                {folders.length === 0 ? (
                  /* 空态：虚线大按钮引导选择第一个文件夹 */
                  <button
                    type="button"
                    className="flex w-full flex-col items-center gap-1.5 rounded-xl border border-dashed border-border py-6 text-foreground-subtle hover:bg-surface-hover"
                    onClick={() => void addFolder()}
                  >
                    <FolderPlus className="size-5" />
                    <span className="text-ui-sm text-foreground">选择文件夹</span>
                    <span className="text-ui-xs text-foreground-subtlest">CraftWork 可读取和编辑这些文件夹</span>
                  </button>
                ) : (
                  /* 已选列表：第一项带「主要」徽标；底部「添加文件夹」行继续追加 */
                  <div className="flex flex-col gap-0.5">
                    {folders.map((folder, index) => (
                      <div key={folder} className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-ui-sm hover:bg-surface-hover">
                        <Folder className="size-3.5 shrink-0 text-foreground-subtle" />
                        <span className="min-w-0 truncate" title={folder}>
                          {pathLeaf(folder)}
                        </span>
                        {index === 0 && (
                          <span className="shrink-0 rounded bg-selected px-1.5 py-0.5 text-ui-xs text-foreground">主要</span>
                        )}
                        <span className="flex-1" />
                        <button
                          type="button"
                          className="rounded p-1 text-foreground-subtlest hover:bg-menu-hover hover:text-foreground"
                          aria-label={`移除文件夹 ${pathLeaf(folder)}`}
                          onClick={() => removeFolder(folder)}
                        >
                          <X className="size-3.5" />
                        </button>
                      </div>
                    ))}
                    <button
                      type="button"
                      className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-ui-sm text-foreground-subtle hover:bg-surface-hover"
                      onClick={() => void addFolder()}
                    >
                      <FolderPlus className="size-3.5" />
                      添加文件夹
                    </button>
                  </div>
                )}
              </div>

              <div className="mt-1 flex items-center justify-end gap-2">
                <button type="button" className={ghostBtnCls} onClick={close} disabled={busy}>
                  取消
                </button>
                <button type="button" className={primaryBtnCls} disabled={folders.length === 0 || busy} onClick={startTrust}>
                  创建项目
                </button>
              </div>
            </>
          ) : (
            <>
              <p className="flex items-center gap-2 text-ui-sm text-foreground-subtle">
                <Monitor className="size-3.5" />
                此电脑
              </p>
              <div className="flex max-h-36 flex-col gap-1 overflow-y-auto rounded-lg border border-border bg-surface px-2.5 py-2">
                {folders.map((folder) => (
                  <div key={folder} className="break-all text-ui-xs text-foreground-subtle">
                    {folder}
                  </div>
                ))}
              </div>
              <p className="text-ui-xs leading-relaxed text-foreground-subtlest">
                CraftWork 可在当前权限设置允许的范围内读取、编辑这些文件夹中的文件，并运行命令。请仅在你信任列出的所有文件夹时继续。
              </p>
              <div className="mt-1 flex items-center justify-end gap-2">
                <button ref={trustCancelRef} type="button" className={ghostBtnCls} onClick={() => setView("form")} disabled={busy}>
                  取消
                </button>
                <button type="button" className={primaryBtnCls} disabled={busy} onClick={() => void create()}>
                  {busy ? "创建中…" : "信任文件夹并创建"}
                </button>
              </div>
            </>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
