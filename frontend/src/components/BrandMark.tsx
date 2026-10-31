/** 应用品牌标识。源图为 1024×1024，界面使用 256px 版本。 */
export function BrandMark({ size = 22 }: { size?: number }) {
  return (
    <img
      className="brand-logo"
      src="/logo.png"
      width={size}
      height={size}
      alt=""
      draggable={false}
    />
  );
}
