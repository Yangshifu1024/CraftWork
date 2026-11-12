// 提供商套餐额度 chip（决策 D34）：composer 模型选择器左侧显示当前提供商的套餐余量。
// 数据来自 providerQuota 通道（服务端 30s TTL 缓存），本组件 60s 轮询 + 点击手动刷新；
// null（白名单外 / 无密钥 / 无套餐）与查询失败都不渲染——chip 缺席本身是状态，不打扰输入。
// 窗口键映射：five_hour→5h，weekly→周，monthly→月；显示「剩余百分比」（used 取补），
// 按剩余量着色（>30% 常规 subtle、10–30% warning、<10% destructive，只用语义 token）。
// 无 shimmer、无 spinner（UI 约定 §7.6）：刷新等待不显示任何运行态标记。
// 明细展示走 HoverCard（hover 弹出，120/80ms 延迟与 turn-navigator 同参）；点击仍是手动刷新，
// 不开浮层——刷新与查看是两个动作。
import { useCallback, useEffect, useRef, useState } from "react";
import { HoverCard } from "radix-ui";
import { api } from "../lib/api";
import type { ProviderQuotaView } from "../../../shared/types";

/** 轮询间隔：60s（服务端另有 30s TTL 缓存，同一数据最多上游两次请求/分钟）。 */
const POLL_MS = 60_000;

/** 窗口键 → 展示标签（顺序即渲染顺序）。 */
const WINDOW_LABELS: Array<readonly [ProviderQuotaView["windows"][number]["key"], string]> = [
  ["five_hour", "5 小时"],
  ["weekly", "每周"],
  ["monthly", "每月"],
];

/** 剩余百分比着色阈值（剩余 ≤10% 危险、≤30% 警告、其余常规）。 */
function remainingTone(remaining: number): string {
  if (remaining <= 10) return "text-destructive";
  if (remaining <= 30) return "text-warning";
  return "text-foreground-subtle";
}

export function QuotaChip({ providerId, providerName }: { providerId: string; providerName: string }): React.JSX.Element | null {
  const [quota, setQuota] = useState<ProviderQuotaView | null>(null);
  const [tick, setTick] = useState(0);
  // 防重入：StrictMode 双挂载下第一个实例的请求可能在途时第二个实例已发起。
  // 用「代际」而非布尔：cleanup 不复位（布尔卡 true 会吞掉 StrictMode 第二次挂载的
  // 全部加载——正是「启动时不出 chip」的根因），代际号让旧实例的在途结果被新实例判废。
  const generationRef = useRef(0);

  const refresh = useCallback(() => setTick((t) => t + 1), []);

  // providerId 切换即清空（旧提供商的余量不跨家显示）；手动刷新（tick）保留旧数据避免闪烁
  useEffect(() => {
    setQuota(null);
  }, [providerId]);

  useEffect(() => {
    if (!providerId) return;
    generationRef.current += 1;
    const generation = generationRef.current;
    let cancelled = false;
    const load = async (): Promise<void> => {
      if (cancelled) return;
      try {
        const view = await api.providerQuota(providerId);
        // 旧代际的结果不覆盖新代际（StrictMode 双挂载 / 快速切换模型时请求乱序返回）
        if (!cancelled && generation === generationRef.current) setQuota(view);
      } catch (err) {
        // 瞬时失败保持本轮隐藏（保留上一次成功数据会在断网后显示过期余量，误导输入判断）
        console.warn("providerQuota", err);
        if (!cancelled && generation === generationRef.current) setQuota(null);
      }
    };
    void load();
    const timer = window.setInterval(() => void load(), POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [providerId, tick]);

  // 无数据 / 不支持：不渲染（无高度占位，composer 布局不变）
  if (!quota || quota.windows.length === 0) return null;

  // 明细行：每窗口一行（标签 / 已用 / 剩余 / 重置时间），muted 语义与 ContextRow 一致
  const fmtTime = (iso: string): string => {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
  };

  return (
    <HoverCard.Root openDelay={120} closeDelay={80}>
      <HoverCard.Trigger asChild>
        <button
          type="button"
          onClick={refresh}
          aria-label={`${providerName} 套餐额度`}
          data-quota-chip
          className="flex h-7 shrink-0 items-center gap-1.5 rounded-lg px-2 text-ui-xs hover:bg-surface-hover"
        >
          {quota.windows.map((w) => {
            const remaining = Math.round((100 - w.usedPercent) * 10) / 10;
            const label = WINDOW_LABELS.find(([k]) => k === w.key)?.[1] ?? w.key;
            return (
              <span key={w.key} className={`flex items-center gap-1 ${remainingTone(remaining)}`} data-quota-window={w.key}>
                <span className="text-foreground-subtlest">{label}</span>
                <span className="tabular-nums">{remaining}%</span>
              </span>
            );
          })}
        </button>
      </HoverCard.Trigger>
      <HoverCard.Portal>
        <HoverCard.Content
          side="top"
          align="end"
          sideOffset={8}
          aria-label="套餐额度详情"
          className="z-50 w-80 rounded-xl border border-popover-border bg-popover p-3 shadow-md"
        >
          <p className="pb-2 text-ui-sm font-medium text-foreground">{providerName} 套餐额度</p>
          <dl className="flex flex-col gap-1.5 text-ui-xs">
            {quota.windows.map((w) => {
              const remaining = Math.round((100 - w.usedPercent) * 10) / 10;
              const label = WINDOW_LABELS.find(([k]) => k === w.key)?.[1] ?? w.key;
              return (
                <div key={w.key} className="flex items-baseline justify-between gap-3">
                  <dt className="shrink-0 text-foreground-subtle">{label}</dt>
                  <dd className="text-right tabular-nums">
                    已用 {w.usedPercent}% / 剩余{" "}
                    <span className={remainingTone(remaining)}>{remaining}%</span>
                    {w.resetsAt && (
                      <span className="text-foreground-subtlest">，{fmtTime(w.resetsAt)} 重置</span>
                    )}
                  </dd>
                </div>
              );
            })}
          </dl>
          <p className="pt-2 text-ui-xs text-foreground-subtlest">
            取数于 {fmtTime(quota.fetchedAt)}；点击 chip 手动刷新，自动每 60s 轮询。
          </p>
        </HoverCard.Content>
      </HoverCard.Portal>
    </HoverCard.Root>
  );
}
