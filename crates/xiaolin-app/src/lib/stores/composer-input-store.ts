import { create } from "zustand";
import i18n from "../../i18n";

export interface ComposerInsertRequest {
  id: string;
  mode: "replace" | "append";
  text: string;
  focus?: boolean;
}

interface ComposerInputState {
  pendingInsert: ComposerInsertRequest | null;
  requestInsert: (req: Omit<ComposerInsertRequest, "id">) => void;
  clearPending: () => void;
}

let insertCounter = 0;

export const useComposerInputStore = create<ComposerInputState>((set) => ({
  pendingInsert: null,

  requestInsert: (req) => {
    insertCounter += 1;
    set({
      pendingInsert: {
        ...req,
        id: `insert-${insertCounter}-${Date.now()}`,
      },
    });
  },

  clearPending: () => set({ pendingInsert: null }),
}));

function toBlockquote(text: string): string {
  return text
    .split("\n")
    .map((line) => `> ${line}`)
    .join("\n");
}

/** Fill chat composer from browser selection toolbar actions. */
export function fillChatFromBrowserSelection(opts: {
  action: "ask" | "quote";
  text: string;
  url: string;
}): void {
  const { action, text, url } = opts;
  if (action === "ask") {
    const block = `${toBlockquote(text)}\n\n${i18n.t("chat:browserQuoteSource", { url })}\n\n`;
    useComposerInputStore.getState().requestInsert({
      mode: "replace",
      text: block,
      focus: true,
    });
    return;
  }
  useComposerInputStore.getState().requestInsert({
    mode: "append",
    text: toBlockquote(text),
    focus: true,
  });
}
