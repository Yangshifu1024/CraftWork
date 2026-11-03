// 应用内 toast 队列（右下角堆叠，4.2s 自动消失，悬停暂停由组件层处理）。
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
    setTimeout(() => {
      set((state) => ({ items: state.items.filter((t) => t.id !== item.id) }));
    }, 4200);
  },
  dismiss(id) {
    set((state) => ({ items: state.items.filter((t) => t.id !== id) }));
  },
}));
