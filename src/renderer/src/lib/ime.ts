// 渲染层共享 IME 守卫：Enter/Escape 提交路径统一在处理前过滤组合输入（spec §10）。
// keyCode === 229 是部分浏览器/框架在 IME 组合期间派发按键事件的兼容信号，与 isComposing 一并拦截。

/** 该按键事件是否处于 IME 组合中（组合中的 Enter 候选确认 / Esc 取消候选不应触发提交或关闭）。 */
export function isImeComposing(event: { nativeEvent: { isComposing?: boolean }; keyCode: number }): boolean {
  return event.nativeEvent.isComposing === true || event.keyCode === 229;
}
