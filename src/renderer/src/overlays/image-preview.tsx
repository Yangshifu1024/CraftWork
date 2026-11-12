// 图片放大预览浮层：全屏遮罩 + 居中原图 + 文件名。Esc / 点遮罩关闭。
// 动效遵循 prefers-reduced-motion（淡入动画类在 styles.css 带 no-preference 守卫）。
// Windows 右上角保留窗口控件（治理规则：全页浮层之上必须仍可操作窗口）。
import { useEffect } from "react";
import { X } from "lucide-react";
import { useOverlays } from "../store/overlays";
import { useApp } from "../store/app";
import { WindowControls } from "../components/window-controls";

/** 图片放大预览浮层：全屏遮罩 + 居中原图 + 文件名；由消息流附件缩略图点击打开。 */
export function ImagePreview(): React.JSX.Element | null {
  const preview = useOverlays((s) => s.imagePreview);
  const close = useOverlays((s) => s.closeImagePreview);
  const showWindowControls = useApp((s) => s.platform) === "windows";

  useEffect(() => {
    if (!preview) return;
    // 捕获阶段拦截 Esc 并阻断传播：这一次按键只关预览，不再落入全局 Esc 退出链把底层浮层一并退掉
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape" && !e.isComposing) {
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [preview, close]);

  if (!preview) return null;
  return (
    <div
      className="fixed inset-0 z-[60] flex flex-col items-center justify-center gap-3 bg-black/80 p-8"
      role="dialog"
      aria-modal="true"
      aria-label={`图片预览：${preview.name}`}
      onClick={close}
    >
      {/* 全页浮层之上窗口控件仍可达（治理规则）；黑色遮罩上控件用反色文字 */}
      {showWindowControls && (
        <div className="absolute right-1 top-1 [&_button]:text-white/70 [&_button:hover]:bg-white/10 [&_button:hover]:text-white">
          <WindowControls />
        </div>
      )}
      <img
        src={preview.src}
        alt={preview.name}
        className="stream-fade-in max-h-[80vh] max-w-full select-text rounded-lg object-contain shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      />
      <div className="flex items-center gap-2 text-ui-sm text-white/80">
        <span className="max-w-[60vw] truncate">{preview.name}</span>
        <button
          type="button"
          className="rounded-md p-1 hover:bg-white/10"
          aria-label="关闭预览"
          onClick={(e) => {
            e.stopPropagation();
            close();
          }}
        >
          <X className="size-4" />
        </button>
      </div>
    </div>
  );
}
