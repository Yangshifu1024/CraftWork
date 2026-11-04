// 应用内 toast 队列（右下角堆叠，最多 3 条）。自动消失计时由视图层 radix Toast 管理，
// 超时 / 右滑 / 手动关闭统一走 dismiss 清数据。
import { create } from "zustand";

export interface ToastItem {
  id: number;
  tone: "info" | "ok" | "warn" | "danger";
  title: string;
  message?: string;
  createdAt: number;
}

interface ToastState {
  items: ToastItem[];
  push(tone: ToastItem["tone"], title: string, message?: string): void;
  dismiss(id: number): void;
}

let nextId = 1;

export const useToasts = create<ToastState>((set) => ({
  items: [],
  push(tone, title, message) {
    const item: ToastItem = { id: nextId++, tone, title, message, createdAt: Date.now() };
    set((state) => ({ items: [...state.items.slice(-2), item] }));
  },
  dismiss(id) {
    set((state) => ({ items: state.items.filter((t) => t.id !== id) }));
  },
}));
