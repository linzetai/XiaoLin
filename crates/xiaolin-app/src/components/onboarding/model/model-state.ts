/**
 * Model step state machine for the onboarding wizard.
 *
 * ## Flow
 *
 * ```
 * ┌──────────────┐  SELECT_PROVIDER   ┌──────────────┐  SELECT_MODEL   ┌──────────────┐
 * │  SubStep 1   │ ─────────────────→ │  SubStep 2   │ ──────────────→ │  SubStep 3   │
 * │ Provider Grid│                    │ Model List   │                 │ Config Form  │
 * └──────────────┘                    └──────────────┘                 └──────────────┘
 *        │                                   ↑                               │
 *        │  SELECT_CUSTOM                    │  GO_PREV_SUB                  │
 *        │  (isCustom=true)                  │  (!isCustom)                  │
 *        │                                   └───────────────────────────────┘
 *        │                                                                   │
 *        └───────────────────────── SubStep 3 ──────── GO_PREV_SUB ──────────┘
 *                                   (isCustom=true → back to SubStep 1)
 * ```
 *
 * - Preset path:  SubStep 1 → 2 → 3
 * - Custom path:  SubStep 1 → 3  (skips model list)
 * - Back from 3:  → 2 (preset) or → 1 (custom, resets isCustom)
 */

import type { ProviderPreset } from "../../../lib/model-registry";

export interface ModelState {
  key: string;
  provider: string;
  model: string;
  baseUrl: string;
  apiKey: string;
  contextWindow: number;
  selectedProvider: ProviderPreset | null;
  subStep: 1 | 2 | 3;
  isCustom: boolean;
  saving: boolean;
  saved: boolean;
}

export type ModelAction =
  | { type: "SET_FIELD"; field: keyof ModelState; value: unknown }
  | { type: "SELECT_PROVIDER"; provider: ProviderPreset }
  | { type: "SELECT_CUSTOM" }
  | { type: "SELECT_MODEL"; modelId: string; contextWindow: number }
  | { type: "GO_PREV_SUB" }
  | { type: "SET_SAVING"; saving: boolean }
  | { type: "SET_SAVED" };

export const INITIAL_MODEL_STATE: ModelState = {
  key: "",
  provider: "openai_compatible",
  model: "",
  baseUrl: "",
  apiKey: "",
  contextWindow: 0,
  selectedProvider: null,
  subStep: 1,
  isCustom: false,
  saving: false,
  saved: false,
};

export function modelReducer(state: ModelState, action: ModelAction): ModelState {
  switch (action.type) {
    case "SET_FIELD":
      return { ...state, [action.field]: action.value };

    case "SELECT_PROVIDER":
      return {
        ...state,
        selectedProvider: action.provider,
        provider: action.provider.provider,
        baseUrl: action.provider.baseUrl,
        key: action.provider.id,
        model: "",
        contextWindow: 0,
        isCustom: false,
        subStep: 2,
      };

    case "SELECT_CUSTOM":
      return {
        ...state,
        selectedProvider: null,
        provider: "openai_compatible",
        baseUrl: "",
        key: "",
        model: "",
        contextWindow: 0,
        isCustom: true,
        subStep: 3,
      };

    case "SELECT_MODEL":
      return {
        ...state,
        model: action.modelId,
        contextWindow: action.contextWindow,
        subStep: 3,
      };

    case "GO_PREV_SUB": {
      let prevStep: 1 | 2 | 3;
      if (state.subStep === 3 && state.isCustom) {
        prevStep = 1;
      } else {
        prevStep = (state.subStep > 1 ? state.subStep - 1 : 1) as 1 | 2 | 3;
      }
      return {
        ...state,
        subStep: prevStep,
        isCustom: prevStep === 1 ? false : state.isCustom,
      };
    }

    case "SET_SAVING":
      return { ...state, saving: action.saving };

    case "SET_SAVED":
      return { ...state, saving: false, saved: true };
  }
}
