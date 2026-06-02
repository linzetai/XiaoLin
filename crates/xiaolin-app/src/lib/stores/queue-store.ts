import { create } from "zustand";
import type { QueuedMessage } from "./types";

export interface QueueState {
  queues: Record<string, QueuedMessage[]>;

  enqueueMessage: (chatId: string, message: Omit<QueuedMessage, "id">) => void;
  dequeueMessage: (chatId: string) => QueuedMessage | undefined;
  updateQueuedMessage: (chatId: string, messageId: string, updates: Partial<QueuedMessage>) => void;
  removeQueuedMessage: (chatId: string, messageId: string) => void;
  clearQueue: (chatId: string) => void;
  reorderQueue: (chatId: string, fromIndex: number, toIndex: number) => void;
}

export const useQueueStore = create<QueueState>((set) => ({
  queues: {},

  enqueueMessage: (chatId, message) => {
    set((state) => {
      const queue = state.queues[chatId] ?? [];
      if (queue.length >= 10) return state;
      const newMsg: QueuedMessage = {
        ...message,
        id: `queue-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      };
      return { queues: { ...state.queues, [chatId]: [...queue, newMsg] } };
    });
  },

  dequeueMessage: (chatId) => {
    let result: QueuedMessage | undefined;
    set((state) => {
      const queue = state.queues[chatId] ?? [];
      if (queue.length === 0) return state;
      result = queue[0];
      return { queues: { ...state.queues, [chatId]: queue.slice(1) } };
    });
    return result;
  },

  updateQueuedMessage: (chatId, messageId, updates) => {
    set((state) => {
      const queue = state.queues[chatId] ?? [];
      return {
        queues: {
          ...state.queues,
          [chatId]: queue.map((m) => (m.id === messageId ? { ...m, ...updates } : m)),
        },
      };
    });
  },

  removeQueuedMessage: (chatId, messageId) => {
    set((state) => {
      const queue = state.queues[chatId] ?? [];
      return { queues: { ...state.queues, [chatId]: queue.filter((m) => m.id !== messageId) } };
    });
  },

  clearQueue: (chatId) => {
    set((state) => ({
      queues: { ...state.queues, [chatId]: [] },
    }));
  },

  reorderQueue: (chatId, fromIndex, toIndex) => {
    set((state) => {
      const queue = state.queues[chatId] ?? [];
      if (fromIndex < 0 || fromIndex >= queue.length || toIndex < 0 || toIndex >= queue.length) return state;
      const newQueue = [...queue];
      const [moved] = newQueue.splice(fromIndex, 1);
      newQueue.splice(toIndex, 0, moved);
      return { queues: { ...state.queues, [chatId]: newQueue } };
    });
  },
}));
