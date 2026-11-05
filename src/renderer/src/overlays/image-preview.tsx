// 图片放大预览浮层：全屏遮罩 + 居中原图 + 文件名。Esc / 点遮罩关闭。
// 动效遵循 prefers-reduced-motion（淡入动画类在 styles.css 带 no-preference 守卫）。
import { useEffect } from "react";
import { X } from "lucide-react";
import { useOverlays } from "../store/overlays";

export function ImagePreview(): React.JSX.Element | null {
  const preview = useOverlays((s) => s.imagePreview);
  const close = useOverlays((s) => s.closeImagePreview);

  useEffect(() => {
    if (!preview) return;
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
