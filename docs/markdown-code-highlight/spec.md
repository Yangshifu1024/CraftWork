# spec：对话代码块语法高亮

## 行为约定

1. **围栏代码块按语言着色**：带语言标注的围栏代码块（```rust、```ts、```json 等）按该语言的语法着色；着色由 shiki 提供（`@streamdown/code` 插件），在本地完成，不联网、不加载 wasm。
2. **未标注与未知语言降级为纯文本**：没有语言标注、或标注了 shiki 未收录的语言（如 ```foo）时，按纯文本渲染，不报错、不出现错误提示块。
3. **深浅双主题跟随应用主题**：代码配色随 `.theme-craft-light` / `.theme-craft-dark` 切换即时生效，**不跟随系统配色**。系统深色 + 应用浅色、系统浅色 + 应用深色两种组合下，代码配色都必须与应用主题一致。
4. **代码块底色用语义 token**：代码块底板取 `--craft-surface`、边框取 `--craft-border`，不使用 shiki 主题自带的背景色（`--sdm-bg` / `--shiki-dark-bg`）；shiki 只贡献 token 前景色。
5. **行内代码有底色**：`` `code` `` 渲染为带 `--craft-surface` 底色的等宽片段（依赖 `muted` / `muted-foreground` 语义 token 映射）。
6. **既有排版约定不变**：代码块不限高、不折叠、无展开按钮，按真实内容高度展开；宽代码行在内容列内横向滚动；外层卡片外框仍由样式层移除（见 `docs/conversation-width/spec.md` 第 5、7 条）。
7. **流式期间可用**：代码块在流式输出过程中逐块着色，未闭合的代码块不做长期缓存（内容仍会继续增长）；不因高亮引入明显卡顿或闪烁。
8. **CSP 不变**：高亮只产生行内 `style` 属性与内联样式变量，由既有 `style-src 'unsafe-inline'` 放行，不新增 CSP 例外，不加载远程资源。
9. **折叠区解说裁掉首尾空行**：中间解说正文按纯文本 + `whitespace-pre-wrap` 渲染，模型常在末尾多吐换行（`\n\n\n\n`），尾随换行会变成真实空行（实测每个 `\n` 约 21px）。渲染前用 `trimBlankLines` 裁掉首尾空行；段内空行（作者有意的分段）保留。最终正文走 Markdown 渲染，解析器本就吞掉尾部空行，无需处理。

## 实现落点

| 位置 | 作用 |
| --- | --- |
| `src/renderer/src/components/markdown.tsx` | 给 `<Streamdown>` 传 `plugins={{ code }}`（`@streamdown/code` 的预配置插件） |
| `src/renderer/src/styles.css` `@custom-variant dark` | 把 `dark:` 变体从系统偏好改绑到 `.theme-craft-dark` 类，shiki 深色 token 才能跟随应用主题 |
| `src/renderer/src/styles.css` `@theme inline` | 补 `--color-muted` / `--color-muted-foreground` 映射（复用 `--craft-surface` / `--craft-foreground-subtle`） |
| `src/renderer/src/styles.css` `.markdown-body [data-streamdown="code-block-body"]` | 代码块底板与边框改走语义 token，内层 `pre` 背景置透明 |
| `src/renderer/src/lib/format.ts` `trimBlankLines` | 裁掉纯文本渲染场景的首尾空行（折叠区中间解说、思考文本、用户消息气泡） |

## 依赖

- `@streamdown/code@2.0.0`（运行时依赖 shiki ^4.4.3，使用纯 JS 正则引擎 `shiki/engine/javascript`，不加载 oniguruma wasm）。
- 语言与主题按需动态导入：构建产物为数百个懒加载语言 chunk，运行时只加载实际用到的语言。
- streamdown 2.7.1 核心包不含高亮器，`plugins.code` 缺省时代码块退化为单色纯文本——这是本 spec 存在的原因。
