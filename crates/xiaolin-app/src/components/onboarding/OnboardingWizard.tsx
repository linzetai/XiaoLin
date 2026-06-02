/**
 * Onboarding Wizard — first-run setup flow for XiaoLin.
 *
 * ## Step Flow
 *
 * ```
 *  welcome  ──→  model  ──→  features  ──→  done
 *                  │
 *                  ├─ SubStep 1: Provider grid  (or "自定义")
 *                  ├─ SubStep 2: Model list     (preset path only)
 *                  └─ SubStep 3: API key form   (both paths)
 * ```
 *
 * Trigger condition (AppLayout):
 *   - `onboarding.completed` is falsy  AND  models list is empty
 *
 * On completion:
 *   - Persists `onboarding.completed = true`
 *   - Emits `xiaolin:models-updated` event
 */

import { useState, useCallback, useReducer } from "react";
import * as transport from "../../lib/transport";
import { WelcomeStep } from "./WelcomeStep";
import { FeaturesStep } from "./FeaturesStep";
import { DoneStep } from "./DoneStep";
import { ModelStep, INITIAL_MODEL_STATE, modelReducer } from "./model";

type WizardStep = "welcome" | "model" | "features" | "done";

export interface OnboardingWizardProps {
  onComplete: () => void;
}

export function OnboardingWizard({ onComplete }: OnboardingWizardProps) {
  const [step, setStep] = useState<WizardStep>("welcome");
  const [fadeClass, setFadeClass] = useState("ob-fade-in");
  const [ms, dispatch] = useReducer(modelReducer, INITIAL_MODEL_STATE);

  const goTo = useCallback((next: WizardStep) => {
    setFadeClass("ob-fade-out");
    setTimeout(() => {
      setStep(next);
      setFadeClass("ob-fade-in");
    }, 250);
  }, []);

  const handleImportClick = useCallback(async () => {
    try {
      if (!transport.isTauri) {
        alert("迁移功能仅在桌面应用中可用");
        return;
      }
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        filters: [{ name: "小林迁移文件", extensions: ["json", "fcdata"] }],
        multiple: false,
      });
      if (!selected) return;
      const { readFile } = await import("@tauri-apps/plugin-fs");
      const fileContents = await readFile(selected as string);
      await transport.importData(new Uint8Array(fileContents), {
        merge: false,
        overwriteConfig: true,
        overwriteAgents: true,
        overwriteSessions: true,
        overwriteSkills: true,
      });
      goTo("model");
    } catch (error) {
      console.error("导入失败:", error);
      alert("导入失败: " + (error as Error).message);
    }
  }, [goTo]);

  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center"
      style={{ background: "var(--bg-primary)" }}
    >
      <div className={`w-full max-w-[560px] px-6 ${fadeClass}`}>
        {step === "welcome" && (
          <WelcomeStep onNext={() => goTo("model")} onImport={handleImportClick} />
        )}
        {step === "model" && (
          <ModelStep
            state={ms}
            dispatch={dispatch}
            onNext={() => goTo("features")}
            onPrev={() => goTo("welcome")}
          />
        )}
        {step === "features" && (
          <FeaturesStep onNext={() => goTo("done")} onPrev={() => goTo("model")} />
        )}
        {step === "done" && <DoneStep onComplete={onComplete} />}
      </div>

      <div className="fixed bottom-8 left-1/2 flex -translate-x-1/2 items-center gap-2">
        {(["welcome", "model", "features", "done"] as WizardStep[]).map((s) => (
          <div
            key={s}
            className={`ob-dot ${step === s ? "ob-dot-active" : ""}`}
            style={{
              background: step === s ? "var(--fill-primary)" : "var(--fill-quaternary)",
            }}
          />
        ))}
      </div>
    </div>
  );
}
