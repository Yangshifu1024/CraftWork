// 提供商 logo：命中内嵌资产（models.dev 清洗后 data URL）用 <img>；未命中回退字母头像。
// 全部本地资源，渲染层无网络请求（spec §2.2）。
import { PROVIDER_LOGOS } from "../assets/provider-logos";

/** 提供商 logo：命中内嵌资产用 <img>，否则回退首字母头像（取 name 首字符大写）。 */
export function ProviderLogo({
  providerId,
  name,
  className,
}: {
  providerId: string;
  name: string;
  className?: string;
}): React.JSX.Element {
  const dataUrl = PROVIDER_LOGOS[providerId];
  const box = className ?? "size-4";
  if (dataUrl) {
    return <img src={dataUrl} alt="" draggable={false} className={`${box} shrink-0 rounded-[3px] object-contain`} />;
  }
  // 字母头像回退：首字符 + bg-tag 圆角方块
  const letter = (name.trim()[0] ?? "?").toUpperCase();
  return (
    <span
      aria-hidden
      className={`flex ${box} shrink-0 select-none items-center justify-center rounded-[3px] bg-tag text-ui-xs font-medium text-foreground-subtle`}
    >
      {letter}
    </span>
  );
}
