//! 用量口径的单一事实源：生成窗口选取、输出速率与上下文占用的公式只存在这一份
//! （此前 runner 与 view 各实现一份，且分母含首包等待、Anthropic 占用漏算缓存）。
//!
//! 口径约定（对齐数据诚实性）：
//! - 生成窗口 = 收流结束 − 首 token（TTFT/建流时间不算生成时间）；「隐藏思考」特判
//!   （有 reasoning token 却未见流式思考输出，如 OpenAI 系）回退全程窗口；
//! - 窗口无效（取消、过短、超长）为 unrated：速率记 0，绝不当作真实的 0 tok/s；
//! - 上下文占用 = 完整提示词（含缓存读+写）+ 本次输出；prompt_tokens 未填充的旧行
//!   回退 input + output（Anthropic 旧行为，缓存命中时偏低，可接受）。

/// 生成窗口有效性下限：短于该值的窗口视为噪声（首块即收尾、计时毛刺），不参与速率。
pub const MIN_GEN_MS: i64 = 200;
/// 生成窗口有效性上限：超过视为异常（挂起/时钟漂移），不参与速率。
pub const MAX_GEN_MS: i64 = 3_600_000;

/// gen_window_ms 的 unrated 哨兵：落库时窗口无效写 -1（区别于 0 = 旧行回退 duration）。
pub const GEN_WINDOW_UNRATED: i64 = -1;

/// 生成窗口（速率分母的事实源）。
///
/// - 基础口径：`duration_ms - ttft_ms`（首 token 之后的纯生成时间）；
/// - 隐藏思考特判：`reasoning_tokens > 0` 且 `reasoning_streamed == false`（思考未流式
///   输出，首 chunk 前已完成推理，如 OpenAI 系）时窗口须含思考时间，回退全程 duration；
/// - 无效窗口（缺 TTFT、非正、越界）返回 None，调用方按 unrated 处理。
pub fn generation_window_ms(duration_ms: i64, ttft_ms: i64, reasoning_tokens: i64, reasoning_streamed: bool) -> Option<i64> {
    if duration_ms <= 0 {
        return None;
    }
    // 隐藏思考：output 里计入了推理 token，但推理时间全在首 token 之前——
    // 剔除首包等待会把这些 token 的生成时间一并剔掉，速率严重虚高，故回退全程。
    let window = if reasoning_tokens > 0 && !reasoning_streamed {
        duration_ms
    } else if ttft_ms > 0 {
        duration_ms - ttft_ms
    } else {
        // 无 TTFT（旧路径）：退回全程窗口，优于 unrated（旧行为可延续）
        duration_ms
    };
    if (MIN_GEN_MS..=MAX_GEN_MS).contains(&window) {
        Some(window)
    } else {
        None
    }
}

/// 输出速率（tok/s）：可评分窗口返回 Some；窗口无效返回 None（unrated，绝不当作 0）。
pub fn tokens_per_second(output_tokens: i64, gen_window_ms: i64) -> Option<f64> {
    if gen_window_ms <= 0 || output_tokens < 0 {
        return None;
    }
    Some(output_tokens as f64 / (gen_window_ms as f64 / 1000.0))
}

/// 上下文占用：完整提示词（含缓存读+写）+ 本次输出。
///
/// `prompt_tokens` 为 0（适配器未填充的旧行）回退 `input + output`——
/// Anthropic 旧行为不含缓存、占用偏低，但历史行无法重建归一化值，保持可读。
pub fn context_tokens(prompt_tokens: i64, input_tokens: i64, output_tokens: i64) -> i64 {
    if prompt_tokens > 0 {
        prompt_tokens + output_tokens
    } else {
        input_tokens + output_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_window_excludes_time_to_first_token() {
        // 全程 5000ms、首包 1200ms → 纯生成窗口 3800ms
        assert_eq!(generation_window_ms(5000, 1200, 0, true), Some(3800));
    }

    #[test]
    fn hidden_reasoning_falls_back_to_full_duration() {
        // OpenAI 系隐藏思考：有 reasoning token 但未流式输出，窗口回退全程
        assert_eq!(generation_window_ms(5000, 4200, 300, false), Some(5000));
        // Anthropic 系思考流式可见：正常剔除首包
        assert_eq!(generation_window_ms(5000, 4200, 300, true), Some(800));
    }

    #[test]
    fn invalid_windows_are_unrated() {
        // 缺 duration / 非正窗口
        assert_eq!(generation_window_ms(0, 0, 0, true), None);
        assert_eq!(generation_window_ms(-5, 0, 0, true), None);
        // 首 token 晚于结束（计时毛刺）→ 窗口为负
        assert_eq!(generation_window_ms(1000, 1500, 0, true), None);
        // 窗口低于噪声下限
        assert_eq!(generation_window_ms(250, 200, 0, true), None);
        // 超长窗口（挂起）
        assert_eq!(generation_window_ms(MAX_GEN_MS + 1, 0, 0, true), None);
        // 恰好在边界内有效
        assert_eq!(generation_window_ms(MIN_GEN_MS, 0, 0, true), Some(MIN_GEN_MS));
    }

    #[test]
    fn missing_ttft_falls_back_to_full_duration() {
        // 旧路径无 TTFT：回退全程窗口（旧行为），优于 unrated
        assert_eq!(generation_window_ms(3000, 0, 0, true), Some(3000));
    }

    #[test]
    fn tokens_per_second_uses_window() {
        assert_eq!(tokens_per_second(380, 3800), Some(100.0));
        // 窗口无效 / 无输出：unrated
        assert_eq!(tokens_per_second(100, 0), None);
        assert_eq!(tokens_per_second(100, GEN_WINDOW_UNRATED), None);
        assert_eq!(tokens_per_second(0, 3800), Some(0.0));
    }

    #[test]
    fn context_tokens_prefers_normalized_prompt() {
        // 新行：完整提示词（含缓存）+ 输出
        assert_eq!(context_tokens(11_500, 100, 200), 11_700);
        // 旧行（prompt 未填充）：回退 input + output
        assert_eq!(context_tokens(0, 100, 200), 300);
    }
}
