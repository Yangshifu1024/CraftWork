/**
 * 图片放大预览浮层。
 *
 * 点击会话或输入区的图片缩略图后打开：遮罩铺满全屏，图片居中按原尺寸展示
 * （超出视口时缩小适配），底部显示文件名。点遮罩、点关闭按钮或按 Esc 关闭。
 *
 * Esc 用捕获阶段监听并阻止冒泡，避免触发全局 Esc 逻辑
 * （关闭浮层组 / 停止正在运行的会话），与 PromptDialog 的防护方式一致。
 */
import { useEffect } from "react";
import { createPortal } from "react-dom";
import { Icon } from "../../components/Icon";
import { useUiStore } from "../../stores/ui";

export function ImagePreviewOverlay() {
  const preview = useUiStore((state) => state.imagePreview);
  const close = useUiStore((state) => state.closeImagePreview);

  // 有预览时接管 Esc；无预览时不注册监听。
  useEffect(() => {
    if (!preview) {
      return;
    }
    function handleKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        close();
      }
    }
    document.addEventListener("keydown", handleKey, true);
    return () => document.removeEventListener("keydown", handleKey, true);
  }, [preview, close]);

  // 没有待预览的图片时完全不渲染。
  if (!preview) {
    return null;
  }

  return createPortal(
    <div className="overlay-layer">
      {/* 点击遮罩等同于关闭 */}
      {/* biome-ignore lint/a11y/noStaticElementInteractions: 遮罩是纯装饰的便利点击目标，键盘用户走 Esc 与关闭按钮 */}
      <div className="overlay-backdrop" role="presentation" onClick={close} />
      <div className="image-preview" role="dialog" aria-modal="true" aria-label={preview.name}>
        <img className="image-preview-img" src={preview.url} alt={preview.name} />
        <div className="image-preview-foot">
          <span className="image-preview-name">{preview.name}</span>
          <button type="button" className="icon-btn" aria-label="关闭预览" onClick={close}>
            <Icon name="x" />
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
