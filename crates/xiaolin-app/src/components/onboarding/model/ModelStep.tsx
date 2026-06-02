import { useCallback } from "react";
import { ChevronLeft } from "lucide-react";
import { ICON } from "../../../lib/ui-tokens";
import { useModelTest, saveModelConfig } from "../../../lib/model-utils";
import type { ModelState, ModelAction } from "./model-state";
import { SubStepBreadcrumb } from "./SubStepBreadcrumb";
import { ProviderSelectStep } from "./ProviderSelectStep";
import { ModelSelectStep } from "./ModelSelectStep";
import { ApiKeyConfigStep } from "./ApiKeyConfigStep";
import { ModelSavedConfirmation } from "./ModelSavedConfirmation";

export function ModelStep({
  state,
  dispatch,
  onNext,
  onPrev,
}: {
  state: ModelState;
  dispatch: React.Dispatch<ModelAction>;
  onNext: () => void;
  onPrev: () => void;
}) {
  const { testStatus, testMsg, runTest, resetTest } = useModelTest();

  const handleSave = useCallback(async () => {
    dispatch({ type: "SET_SAVING", saving: true });
    try {
      await saveModelConfig({
        key: state.key,
        provider: state.provider,
        model: state.model,
        baseUrl: state.baseUrl,
        apiKey: state.apiKey,
        contextWindow: state.contextWindow,
      });
      dispatch({ type: "SET_SAVED" });
    } catch {
      resetTest();
      dispatch({ type: "SET_SAVING", saving: false });
    }
  }, [state, dispatch, resetTest]);

  if (state.saved) {
    return <ModelSavedConfirmation model={state.model} onNext={onNext} />;
  }

  return (
    <div className="relative">
      <div className="absolute -top-12 left-0 flex">
        <button
          onClick={state.subStep > 1 ? () => dispatch({ type: "GO_PREV_SUB" }) : onPrev}
          className="flex cursor-pointer items-center gap-1 text-[13px] font-medium transition-colors hover:opacity-80"
          style={{ color: "var(--fill-tertiary)" }}
        >
          <ChevronLeft {...ICON.md} />
          {state.subStep > 1 ? "上一步" : "返回"}
        </button>
      </div>

      <div className="mb-5 text-center">
        <h2 className="text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
          添加你的第一个模型
        </h2>
        <p className="mt-1.5 text-[13px]" style={{ color: "var(--fill-secondary)" }}>
          选择 AI 提供商，配置 API 密钥即可开始
        </p>
      </div>

      <SubStepBreadcrumb current={state.subStep} isCustom={state.isCustom} />

      {state.subStep === 1 && (
        <ProviderSelectStep dispatch={dispatch} />
      )}
      {state.subStep === 2 && state.selectedProvider && (
        <ModelSelectStep provider={state.selectedProvider} dispatch={dispatch} />
      )}
      {state.subStep === 3 && (
        <ApiKeyConfigStep
          state={state}
          dispatch={dispatch}
          testStatus={testStatus}
          testMsg={testMsg}
          onTest={() => runTest(state.baseUrl, state.apiKey, state.model)}
          onSave={handleSave}
        />
      )}
    </div>
  );
}
