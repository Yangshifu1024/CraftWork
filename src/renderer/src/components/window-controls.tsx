// 自绘标题栏的窗口控制按钮（决策 D22）。
//
// Windows 上窗口以 `decorations: false` 启动（`src-tauri/tauri.windows.conf.json`），
// 最小化 / 最大化还原 / 关闭由本组件提供。与固定 overlay 方案（tauri-plugin-decoration，
// 已试并撤销：按钮无法随面板迁移且盖住应用元素）不同，本组件是**内联迁移式**——
// 作为普通 flex 子元素嵌入「拥有右上角的表面」的顶行右端，随面板状态真实迁移：
//   面板关 → 会话头右端（conversation.tsx WorkspaceHeader 动作区）
//   面板开 → 面板 tab 条右端（panel.tsx 按钮组末尾）
//   设置浮层 → 顶部条右端；图片预览 → 右上角 absolute（全页浮层控件必须可达）
// macOS 不渲染（原生交通灯在左上）；Linux 不在本期范围（D7）。
//
// 三条约定：
// - 宿主行须是拖拽区（data-tauri-drag-region），按钮是可点击元素被 drag.js 自动排除，
//   双击宿主行空白处最大化窗口——Windows caption 行的全部原生行为。
// - 最大化态无事件推送，用 DOM `resize` 兜底刷新图标：本窗口的最大化/还原必然改变
//   视口尺寸，拖拽吸附到边缘时同样触发。
// - 关闭语义与原生关闭键一致：走 CloseRequested → 「关闭到托盘」设置（D20）。
//   已知取舍：自绘按钮无 Win11 Snap Layouts 分屏菜单（撤销插件方案的代价，用户裁决）。
import { useEffect, useState } from "react";
import { Minus, Square, X } from "lucide-react";
import { api } from "../lib/api";

/** 三个按钮各自的壳：28×28 紧凑图标钮（rounded-lg），无悬停底色——
 *  hover 底色由各按钮自带（关闭键红色与常规灰键不同类，同属性类放一起会被
 *  Tailwind 输出顺序覆盖——见 pitfalls「同属性工具类互相覆盖」）。 */
const BUTTON_CLS =
  "flex size-7 items-center justify-center rounded-lg text-foreground-subtle outline-none transition-colors duration-100 focus-visible:bg-surface-hover";

/** 还原图标：两个错落的小方框（Windows 系统惯例的画法）。
 *  lucide 的 `Copy` 观感接近但语义不符（复制），故用一个内联 svg 明确表达「还原」。 */
function RestoreIcon(): React.JSX.Element {
  return (
    <svg viewBox="0 0 16 16" className="size-4" fill="none" stroke="currentColor" strokeWidth={1.4} aria-hidden>
      <rect x="5" y="5" width="8.5" height="8.5" rx="1" />
      <path d="M11 5V3.6A1.1 1.1 0 0 0 9.9 2.5H3.6A1.1 1.1 0 0 0 2.5 3.6v6.3A1.1 1.1 0 0 0 3.6 11H5" />
    </svg>
  );
}

/** Windows 窗口控件（最小化 / 最大化还原 / 关闭），内联嵌入宿主行右端。
 *  仅 Windows 渲染；macOS 走原生交通灯，其他平台由系统标题栏承担。 */
export function WindowControls(): React.JSX.Element {
  const [maximised, setMaximised] = useState(false);

  useEffect(() => {
    let alive = true;
    const sync = (): void => {
      void api
        .windowIsMaximised()
        .then((value) => {
          if (alive) setMaximised(value);
        })
        // 查询失败保持上一次状态：图标不刷新只影响观感，不值得打扰用户
        .catch(() => undefined);
    };
    sync();
    window.addEventListener("resize", sync);
    return () => {
      alive = false;
      window.removeEventListener("resize", sync);
    };
  }, []);

  return (
    <div
      // 内联 flex 子元素：占位即自身宽度（3×28px + 2px 间距），宿主行无需任何让位变量。
      // 按钮固定 28×28 不随宿主行拉伸——全高 44×48 的原生式矩形观感过大
      // （实机对比后用户裁决改紧凑款）。
      className="flex items-center gap-0.5"
      // 右键不弹应用菜单（原生标题栏的既有语义），也避免与项目自绘右键菜单混用
      onContextMenu={(e) => e.preventDefault()}
    >
      <button
        className={`${BUTTON_CLS} hover:bg-surface-hover hover:text-foreground`}
        title="最小化"
        aria-label="最小化"
        onClick={() => void api.minimiseWindow()}
      >
        <Minus className="size-4" />
      </button>
      <button
        className={`${BUTTON_CLS} hover:bg-surface-hover hover:text-foreground`}
        title={maximised ? "还原" : "最大化"}
        aria-label={maximised ? "还原" : "最大化"}
        onClick={() => void api.toggleMaximiseWindow()}
      >
        {maximised ? <RestoreIcon /> : <Square className="size-4" />}
      </button>
      <button
        // 关闭键悬停红（主题 destructive token：亮 #e03131 / 暗 #ff5c5c），Windows 惯例
        className={`${BUTTON_CLS} hover:bg-destructive hover:text-foreground-inverse`}
        title="关闭"
        aria-label="关闭"
        onClick={() => void api.closeWindow()}
      >
        <X className="size-4" />
      </button>
    </div>
  );
}
