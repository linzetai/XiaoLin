import { Component, type ErrorInfo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Warning } from "@phosphor-icons/react";

class AppErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[AppErrorBoundary]", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <AppErrorFallback
          error={this.state.error}
          onRetry={() => this.setState({ error: null })}
        />
      );
    }
    return this.props.children;
  }
}

function AppErrorFallback({ error, onRetry }: { error: Error; onRetry: () => void }) {
  const { t } = useTranslation("common");
  return (
    <div
      className="flex h-screen w-screen flex-col items-center justify-center gap-4 px-6"
      style={{ background: "var(--bg-base)", color: "var(--fill-primary)" }}
    >
      <Warning size={32} style={{ color: "var(--red)" }} />
      <p className="max-w-md text-center text-[14px]" style={{ color: "var(--fill-secondary)" }}>
        {t("errorPrefix", { message: error.message })}
      </p>
      <button
        onClick={onRetry}
        className="cursor-pointer rounded-[var(--radius-sm)] px-4 py-2 text-[13px] font-medium"
        style={{
          background: "var(--tint)",
          color: "var(--bg-base)",
        }}
      >
        {t("retry")}
      </button>
    </div>
  );
}

export { AppErrorBoundary };
