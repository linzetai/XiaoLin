import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Lightning, FileMagnifyingGlass, PlugsConnected } from "@phosphor-icons/react";

function SuggestionCard({
  icon,
  text,
}: {
  icon: ReactNode;
  text: string;
}) {
  return (
    <button
      type="button"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "10px 14px",
        borderRadius: 10,
        border: "1px solid var(--border-shell-subtle)",
        background: "var(--bg-elevated)",
        color: "var(--fill-secondary)",
        fontSize: 13,
        fontWeight: 500,
        cursor: "pointer",
        textAlign: "left",
        transition: "background 0.15s",
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.background = "var(--bg-hover)";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = "var(--bg-elevated)";
      }}
    >
      <span
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          color: "var(--fill-quaternary)",
          flexShrink: 0,
        }}
      >
        {icon}
      </span>
      <span>{text}</span>
    </button>
  );
}

export function WelcomeView() {
  const { t } = useTranslation("common");
  return (
    <div
      className="welcome-view"
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        flex: 1,
        minHeight: 0,
        padding: "32px 24px",
      }}
    >
      <h1
        style={{
          fontSize: 28,
          fontWeight: 700,
          color: "var(--fill-primary)",
          margin: 0,
          letterSpacing: "-0.02em",
        }}
      >
        What should we build?
      </h1>

      <div style={{ marginTop: 24, width: "100%", maxWidth: 560 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            minHeight: 48,
            padding: "12px 16px",
            borderRadius: 12,
            border: "1px solid var(--bg-input-border)",
            background: "var(--bg-elevated)",
            color: "var(--fill-quaternary)",
            fontSize: 14,
          }}
        >
          <span>Ask anything...</span>
        </div>
      </div>

      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: 10,
          marginTop: 16,
          width: "100%",
          maxWidth: 560,
          justifyContent: "center",
        }}
      >
        <SuggestionCard
          icon={<Lightning size={16} />}
          text={t("welcomeBuild")}
        />
        <SuggestionCard
          icon={<FileMagnifyingGlass size={16} />}
          text={t("welcomeReview")}
        />
        <SuggestionCard
          icon={<PlugsConnected size={16} />}
          text={t("welcomePlugins")}
        />
      </div>
    </div>
  );
}
