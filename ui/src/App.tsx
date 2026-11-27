import React, { lazy, Suspense, useEffect, useState, type ReactNode } from "react";
import { DesktopWorkbench } from "@/components/desktop/DesktopWorkbench";
import { markUiSmoke } from "@/lib/uiSmoke";
import {
  installGlobalDiagnosticsCapture,
  recordDiagnosticError,
} from "@/shell/api/diagnostics";

class DesktopErrorBoundary extends React.Component<
  { children: ReactNode },
  { hasError: boolean; message: string }
> {
  constructor(props: { children: ReactNode }) {
    super(props);
    this.state = { hasError: false, message: "" };
  }

  static getDerivedStateFromError(error: Error) {
    return { hasError: true, message: error?.message || "界面渲染异常" };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    void recordDiagnosticError({
      category: "react_boundary",
      errorCode: error?.name || "REACT_RENDER_ERROR",
      source: "DesktopErrorBoundary",
      message: error?.message || "React boundary caught error",
      stack: info?.componentStack || error?.stack || "",
    });
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="flex h-screen w-screen items-center justify-center bg-sp-canvas p-6 text-sp-text">
          <div className="w-full max-w-md rounded-card border border-sp-line bg-sp-card p-6 space-y-3 shadow-modal">
            <h2 className="text-base font-semibold">界面出现意外异常</h2>
            <p className="text-xs text-sp-text-secondary break-words">{this.state.message}</p>
            <button
              type="button"
              onClick={() => {
                this.setState({ hasError: false, message: "" });
                if (typeof window !== "undefined") window.location.reload();
              }}
              className="eo-settings-btn-primary"
            >
              重新加载工作台
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

const DesktopHarness = import.meta.env.DEV
  ? lazy(() =>
      import("@/components/desktop/DesktopHarness.tsx").then((module) => ({
        default: module.DesktopHarness,
      })),
    )
  : null;

export default function App() {
  const [dark, setDark] = useState(() => {
    if (typeof window === "undefined") return false;
    const h = window.location.hash;
    let isDark = false;
    if (h.includes("theme=light")) {
      isDark = false;
    } else if (h.includes("theme=dark") || (h.includes("dark") && !h.includes("light"))) {
      isDark = true;
    } else {
      isDark = window.localStorage.getItem("eostage-theme") === "dark";
    }
    if (typeof document !== "undefined") {
      document.documentElement.classList.toggle("dark", isDark);
    }
    return isDark;
  });

  useEffect(() => {
    installGlobalDiagnosticsCapture();
    markUiSmoke("react_mounted", true);
    const frame = window.requestAnimationFrame(() => {
      markUiSmoke(
        "workbench_rendered",
        Boolean(document.querySelector('[data-ui-smoke-workbench="true"]')),
      );
    });
    return () => window.cancelAnimationFrame(frame);
  }, []);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    if (typeof window !== "undefined") {
      window.localStorage.setItem("eostage-theme", dark ? "dark" : "light");
    }
  }, [dark]);

  if (
    DesktopHarness &&
    typeof window !== "undefined" &&
    window.location.hash.startsWith("#harness")
  ) {
    return (
      <Suspense fallback={null}>
        <DesktopHarness dark={dark} onToggleDark={() => setDark((v) => !v)} />
      </Suspense>
    );
  }

  return (
    <DesktopErrorBoundary>
      <DesktopWorkbench
        dark={dark}
        onToggleDark={() => setDark((v) => !v)}
      />
    </DesktopErrorBoundary>
  );
}
