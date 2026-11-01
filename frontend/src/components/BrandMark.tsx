/**
 * 品牌标识组件：显示应用 logo，用于侧边栏顶部与启动画面。
 *
 * 图片是静态资源，放在 `public/logo.png`，桌面端打包后通过同样的根路径访问；
 * 源图为 1024×1024，界面使用 256px 版本。
 *
 * 负责：按指定尺寸渲染 logo 图片。
 * props：`size` 为边长（像素），默认 22，侧边栏与标题栏共用。
 * 关键设置：`alt=""` 表示这是装饰性图片，名称由旁边的文字承担，避免读屏重复；
 * `draggable={false}` 防止用户误拖走图片。
 */
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
