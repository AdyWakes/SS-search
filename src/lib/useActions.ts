import { useCallback } from "react";
import { useApi } from "./ApiContext";
import { useToast } from "../components/Toast";

export function useActions() {
  const api = useApi();
  const toast = useToast();
  const run = useCallback(
    async (f: () => Promise<unknown>, ok?: string) => {
      try {
        await f();
        if (ok) toast(ok);
      } catch (e) {
        toast(String(e), true);
      }
    },
    [toast],
  );
  return {
    open: (id: number) => run(() => api.open(id)),
    reveal: (id: number) => run(() => api.reveal(id)),
    copyImage: (id: number) => run(() => api.copyImage(id), "Image copied"),
    copyText: (id: number) => run(() => api.copyText(id), "Text copied"),
  };
}

/** Shared keyboard shortcuts for a selected result. Returns true if handled. */
export function handleResultKeys(e: KeyboardEvent | React.KeyboardEvent, id: number | null, a: ReturnType<typeof useActions>): boolean {
  if (id == null) return false;
  const mod = e.ctrlKey || e.metaKey;
  if (e.key === "Enter" && mod) {
    a.reveal(id);
    return true;
  }
  if (e.key === "Enter") {
    a.open(id);
    return true;
  }
  if (mod && e.shiftKey && (e.key === "c" || e.key === "C")) {
    a.copyText(id);
    return true;
  }
  if (mod && (e.key === "c" || e.key === "C") && !hasTextSelection()) {
    a.copyImage(id);
    return true;
  }
  return false;
}

function hasTextSelection() {
  const s = window.getSelection();
  return !!s && s.toString().length > 0;
}
