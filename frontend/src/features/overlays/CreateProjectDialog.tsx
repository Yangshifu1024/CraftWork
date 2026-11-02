/**
 * 「创建项目」对话框。
 *
 * 交互与 ZCode 对齐：项目名称（未编辑时从主要目录名推导）、设备下拉
 * （本地部署固定「此电脑」）、源文件夹列表（第一个自动成为「主要」，
 * 可移除、可追加），提交前弹「信任此文件夹吗？」确认——每次创建都
 * 确认，信任结果不持久化。确认通过后以可信档创建项目，并复用侧栏的
 * 编排（重同步视图 + 新建会话），与「添加项目即可开始干活」一步到位。
 *
 * 弹层结构：overlay-layer + backdrop + role="dialog"，信任确认作为对话框
 * 内部的第二视图渲染（与父表单同层，避免嵌套浮层的焦点与关闭联动问题）。
 */
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "../../components/Icon";
import { api } from "../../lib/api";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";
import { resyncWorkspaceViews } from "../../stores/workspaceFlow";

/** 对话框的两个视图：表单与信任确认。 */
type View = "form" | "trust";

/** 取路径的最后一段作为展示名。 */
function pathLeaf(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const index = trimmed.lastIndexOf("/");
  return index >= 0 ? trimmed.slice(index + 1) || trimmed : trimmed;
}

/**
 * 「创建项目」对话框，不接收 props；开关来自 useUiStore。
 *
 * 关键状态：
 * - `view`：当前视图，信任确认期间冻结表单输入；
 * - `name` / `nameEdited`：名称与「用户是否改过」——未改过时提交不带名称，
 *   由后端从主要目录名推导（与 ZCode 的行为一致）；
 * - `folders`：已选源文件夹（绝对路径），第一个是主要目录；
 * - `busy`：创建请求进行中，按钮禁用防重复提交。
 */
export function CreateProjectDialog() {
  const open = useUiStore((state) => state.createProjectOpen);
  const close = useUiStore((state) => state.closeCreateProject);
  const addProject = useAppStore((state) => state.addProject);
  const createThread = useThreadStore((state) => state.create);
  const toast = useUiStore((state) => state.toast);

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

  // 打开时聚焦名称输入框；信任确认视图把焦点放在「取消」——信任必须是
  // 明确选择，不提供 Enter 直通的确认路径。
  useEffect(() => {
    if (!open) {
      return;
    }
    if (view === "form") {
      nameInputRef.current?.focus();
    } else {
      trustCancelRef.current?.focus();
    }
  }, [open, view]);

  // Esc 的视图相关退出：信任确认回到表单，表单直接关闭。
  useEffect(() => {
    if (!open) {
      return;
    }
    function handleKey(event: KeyboardEvent) {
      if (event.key !== "Escape") {
        return;
      }
      event.stopPropagation();
      if (view === "trust") {
        setView("form");
      } else {
        close();
      }
    }
    document.addEventListener("keydown", handleKey, true);
    return () => document.removeEventListener("keydown", handleKey, true);
  }, [open, view, close]);

  if (!open) {
    return null;
  }

  /** 弹系统目录选择器并追加一个源文件夹；重复路径忽略。 */
  async function addFolder() {
    try {
      const path = await api.workspace.pickFolder();
      if (!path) {
        return;
      }
      setFolders((current) => (current.includes(path) ? current : [...current, path]));
    } catch (error) {
      toast({ title: "选择文件夹失败", description: String(error), tone: "danger" });
    }
  }

  /** 移除一个源文件夹；移除主要目录后由下一个顺延（列表顺序即主要顺序）。 */
  function removeFolder(path: string) {
    setFolders((current) => current.filter((item) => item !== path));
  }

  /** 表单提交：进入信任确认视图。无源文件夹时按钮已禁用，这里兜底。 */
  function startTrust() {
    if (folders.length === 0 || busy) {
      return;
    }
    setView("trust");
  }

  /** 信任确认通过：以可信档创建项目，成功后重同步视图并新建会话。 */
  async function create() {
    if (busy) {
      return;
    }
    setBusy(true);
    try {
      // 未编辑过名称时交空串，由后端从主要目录名推导（重名自动加序号）。
      await addProject(nameEdited ? name.trim() : "", folders, "trusted");
      close();
      await resyncWorkspaceViews();
      await createThread();
      toast({
        title: "已创建项目",
        description: nameEdited && name.trim() ? name.trim() : pathLeaf(folders[0]),
        tone: "ok",
      });
    } catch (error) {
      toast({ title: "创建项目失败", description: String(error), tone: "danger" });
      // 创建失败回到表单，保留已填内容便于修正后重试。
      setView("form");
    } finally {
      setBusy(false);
    }
  }

  return createPortal(
    <div className="overlay-layer">
      {/* 遮罩点击等同取消：信任态回表单，表单态关闭 */}
      {/* biome-ignore lint/a11y/noStaticElementInteractions: 遮罩是纯装饰的便利点击目标，键盘用户走 Esc 与按钮 */}
      <div
        className="overlay-backdrop"
        role="presentation"
        onClick={() => {
          if (view === "trust") {
            setView("form");
          } else {
            close();
          }
        }}
      />
      <div className="create-project" role="dialog" aria-modal="true" aria-label="创建项目">
        {view === "form" ? (
          <>
            <header className="cp-head">
              <h2>创建项目</h2>
              <button type="button" className="icon-btn sm" aria-label="关闭" onClick={close}>
                <Icon name="x" />
              </button>
            </header>

            <label className="field cp-field">
              <span>项目名称</span>
              <input
                ref={nameInputRef}
                className="text-input wide"
                value={name}
                placeholder="新项目"
                onChange={(event) => {
                  setName(event.target.value);
                  setNameEdited(true);
                }}
                onKeyDown={(event) => {
                  // Enter 视为点击「创建项目」：进入信任确认。
                  if (event.key === "Enter") {
                    event.preventDefault();
                    startTrust();
                  }
                }}
              />
            </label>

            {/* 设备是纯展示的静态信息（本地部署只有此电脑），没有可关联的表单控件 */}
            <div className="field cp-field">
              <span>设备</span>
              <span className="cp-device">
                <Icon name="monitor" className="ico" />
                <span className="cp-device-label">此电脑</span>
                <Icon name="chevron-down" className="ico cp-device-caret" />
              </span>
            </div>

            <div className="field cp-field">
              <span>源文件夹</span>
              {folders.length === 0 ? (
                /* 空态：虚线大按钮引导选择第一个文件夹 */
                <button type="button" className="cp-drop" onClick={() => void addFolder()}>
                  <span className="cp-drop-icon">
                    <Icon name="folder-plus" />
                  </span>
                  <span className="cp-drop-title">选择文件夹</span>
                  <span className="cp-drop-hint">CraftWork 可读取和编辑这些文件夹</span>
                </button>
              ) : (
                /* 已选列表：第一项带「主要」徽标；底部「添加文件夹」行继续追加 */
                <div className="cp-folders">
                  {folders.map((folder, index) => (
                    <div key={folder} className="cp-folder-row">
                      <Icon name="folder" className="ico" />
                      <span className="cp-folder-name">{pathLeaf(folder)}</span>
                      <span className="spacer" />
                      {index === 0 ? <span className="cp-folder-badge">主要</span> : null}
                      <button
                        type="button"
                        className="icon-btn sm"
                        aria-label={`移除文件夹 ${pathLeaf(folder)}`}
                        onClick={() => removeFolder(folder)}
                      >
                        <Icon name="x" />
                      </button>
                    </div>
                  ))}
                  <button type="button" className="cp-folder-row cp-folder-add" onClick={() => void addFolder()}>
                    <Icon name="folder-plus" className="ico" />
                    <span>添加文件夹</span>
                  </button>
                </div>
              )}
            </div>

            <footer className="cp-actions">
              <span className="spacer" />
              <button type="button" className="btn" onClick={close}>
                取消
              </button>
              <button type="button" className="btn primary" disabled={folders.length === 0} onClick={startTrust}>
                创建项目
              </button>
            </footer>
          </>
        ) : (
          <>
            <header className="cp-head">
              <h2>{folders.length > 1 ? "信任这些文件夹吗？" : "信任此文件夹吗？"}</h2>
              <button type="button" className="icon-btn sm" aria-label="关闭" onClick={() => setView("form")}>
                <Icon name="x" />
              </button>
            </header>
            <p className="cp-trust-device">此电脑</p>
            <div className="cp-trust-paths">
              {folders.map((folder) => (
                <div key={folder} className="cp-trust-path">
                  {folder}
                </div>
              ))}
            </div>
            <p className="cp-trust-note">
              CraftWork
              可在当前权限设置允许的范围内读取、编辑这些文件夹中的文件，并运行命令。请仅在你信任列出的所有文件夹时继续。
            </p>
            <footer className="cp-actions">
              <span className="spacer" />
              <button ref={trustCancelRef} type="button" className="btn" onClick={() => setView("form")}>
                取消
              </button>
              <button type="button" className="btn primary" disabled={busy} onClick={() => void create()}>
                {busy ? "创建中…" : "信任文件夹并创建"}
              </button>
            </footer>
          </>
        )}
      </div>
    </div>,
    document.body,
  );
}
