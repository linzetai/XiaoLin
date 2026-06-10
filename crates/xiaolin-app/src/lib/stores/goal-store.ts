import { create } from "zustand";
import * as transport from "../transport";
import type { GoalData } from "./types";

export interface GoalStoreState {
  /** Goal state keyed by session/chat id. */
  goals: Record<string, GoalData>;
  /** Per-chat flag: user selected "Goal" mode in the mode selector. */
  goalMode: Record<string, boolean>;
  setActiveGoal: (chatId: string, goal: GoalData | null) => void;
  clearGoal: (chatId: string, goalId?: string) => void;
  setGoalMode: (chatId: string, active: boolean) => void;
}

export const useGoalStore = create<GoalStoreState>((set, get) => ({
  goals: {},
  goalMode: {},

  setActiveGoal: (chatId, goal) => {
    set((state) => {
      if (!goal) {
        const { [chatId]: _, ...rest } = state.goals;
        return { goals: rest };
      }
      return { goals: { ...state.goals, [chatId]: goal } };
    });
  },

  clearGoal: (chatId, goalId) => {
    const current = get().goals[chatId];
    if (goalId && current && current.id !== goalId) return;
    set((state) => {
      const { [chatId]: _, ...rest } = state.goals;
      return { goals: rest };
    });
  },

  setGoalMode: (chatId, active) => {
    set((state) => ({
      goalMode: { ...state.goalMode, [chatId]: active },
    }));
  },
}));

let _unsubUpdated: (() => void) | undefined;
let _unsubCleared: (() => void) | undefined;

function parseGoalData(raw: unknown): GoalData | null {
  if (!raw || typeof raw !== "object") return null;
  const goal = raw as Record<string, unknown>;
  if (typeof goal.id !== "string" || typeof goal.description !== "string") return null;
  return {
    id: goal.id,
    description: goal.description,
    status: typeof goal.status === "string" ? goal.status : "active",
    token_budget: typeof goal.token_budget === "number" ? goal.token_budget : undefined,
    tokens_used: Number(goal.tokens_used ?? 0),
    time_used_seconds: Number(goal.time_used_seconds ?? 0),
    pause_reason: typeof goal.pause_reason === "string" ? goal.pause_reason : undefined,
    continuation_rounds: Number(goal.continuation_rounds ?? 0),
    created_at: Number(goal.created_at ?? 0),
    updated_at: Number(goal.updated_at ?? 0),
  };
}

function handleGoalUpdated(msg: unknown, chatId: string) {
  const data = (msg as { data?: Record<string, unknown> })?.data;
  const goal = parseGoalData(data?.goal);
  if (goal) {
    useGoalStore.getState().setActiveGoal(chatId, goal);
  }
}

function handleGoalCleared(msg: unknown, chatId: string) {
  const data = (msg as { data?: { goal_id?: string } })?.data;
  useGoalStore.getState().clearGoal(chatId, data?.goal_id);
}

/** Subscribe to goal broadcast events for the active chat session. */
export function initGoalListener(getActiveChatId: () => string): void {
  _unsubUpdated?.();
  _unsubCleared?.();
  _unsubUpdated = transport.onWsEvent("goal_updated", (msg) => {
    const chatId = getActiveChatId();
    if (chatId) handleGoalUpdated(msg, chatId);
  });
  _unsubCleared = transport.onWsEvent("goal_cleared", (msg) => {
    const chatId = getActiveChatId();
    if (chatId) handleGoalCleared(msg, chatId);
  });
}

export function teardownGoalListener(): void {
  _unsubUpdated?.();
  _unsubCleared?.();
  _unsubUpdated = undefined;
  _unsubCleared = undefined;
}

export function handleGoalUpdatedForChat(msg: unknown, chatId: string): void {
  handleGoalUpdated(msg, chatId);
}

export function handleGoalClearedForChat(msg: unknown, chatId: string): void {
  handleGoalCleared(msg, chatId);
}
