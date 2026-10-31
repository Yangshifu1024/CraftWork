import { useEffect, useRef } from "react";
import { AppShell } from "./app/AppShell";
import { useAppStore } from "./stores/app";
import { useThreadStore } from "./stores/threads";
import { useTerminalStore } from "./stores/terminal";
import { useWorkspaceStore } from "./stores/workspace";

export default function App() {
  const bootedRef = useRef(false);
  const bootedModeRef = useRef<string | null>(null);
  const mode = useAppStore((state) => state.mode);

  useEffect(() => {
    if (bootedRef.current) {
      return;
    }
    bootedRef.current = true;
    void (async () => {
      await useAppStore.getState().boot();
      useThreadStore.getState().connect();
      useTerminalStore.getState().connect();
      await useThreadStore.getState().boot();
      await useWorkspaceStore.getState().loadRoot();
      await useAppStore.getState().refreshProviders();
      await useTerminalStore.getState().boot();
      bootedModeRef.current = useAppStore.getState().mode;
    })();
  }, []);

  useEffect(() => {
    if (!bootedModeRef.current) {
      return;
    }
    if (bootedModeRef.current === mode) {
      return;
    }
    bootedModeRef.current = mode;
    useWorkspaceStore.getState().reset();
    void useThreadStore.getState().boot();
    void useWorkspaceStore.getState().loadRoot();
  }, [mode]);

  return <AppShell />;
}
