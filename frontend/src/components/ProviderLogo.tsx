/**
 * 提供商 logo：优先展示 models.dev 目录给出的图标。
 *
 * 图标由后端按目录 ID 代理抓取（返回 data URL，不放开 CSP 的远程图片），
 * 并且滚动进入视口后才按需加载；离线或抓取失败一律回退为两位字符的
 * 文字头像，与原有外观保持一致。
 */
import { useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { cn } from "../lib/utils";

export function ProviderLogo({
  providerID,
  name,
  className,
}: {
  /** 目录提供商标识；空表示没有 logo 来源（手工添加的提供商），直接文字头像。 */
  providerID?: string;
  /** 提供商名称，回退头像取前两个字符。 */
  name: string;
  /** 头像样式类，与原先文字头像使用的类保持一致。 */
  className?: string;
}) {
  const [visible, setVisible] = useState(false);
  const [logo, setLogo] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const holder = useRef<HTMLDivElement | null>(null);

  // 目录列表一次渲染上百项：进入视口（含少量预加载余量）才发起抓取。
  useEffect(() => {
    if (visible || !providerID) return;
    const element = holder.current;
    if (!element) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "120px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [visible, providerID]);

  useEffect(() => {
    if (!visible || !providerID || failed || logo) return;
    let cancelled = false;
    api.providers
      .catalogLogo(providerID)
      .then((dataURL) => {
        if (cancelled) return;
        if (dataURL) {
          setLogo(dataURL);
        } else {
          setFailed(true);
        }
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [visible, providerID, failed, logo]);

  if (logo) {
    return (
      <img
        className={cn(className)}
        src={logo}
        alt=""
        draggable={false}
        onError={() => {
          setLogo(null);
          setFailed(true);
        }}
      />
    );
  }
  return (
    <div ref={holder} className={cn(className)} aria-hidden>
      {name.slice(0, 2)}
    </div>
  );
}
