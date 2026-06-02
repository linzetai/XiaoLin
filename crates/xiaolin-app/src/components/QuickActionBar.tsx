import { useState, useEffect, useRef, useCallback } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function QuickActionBar() {
  const [input, setInput] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const appWindow = getCurrentWindow();

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    const unlisten = appWindow.onFocusChanged(({ payload: focused }) => {
      if (!focused) {
        appWindow.hide();
        setInput("");
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [appWindow]);

  const handleSubmit = useCallback(async () => {
    const text = input.trim();
    if (!text || isSubmitting) return;
    setIsSubmitting(true);
    try {
      // TODO: connect to gateway WebSocket to send the message
      console.log("Quick action submit:", text);
    } finally {
      setInput("");
      setIsSubmitting(false);
      await appWindow.hide();
    }
  }, [input, isSubmitting, appWindow]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Escape") {
        appWindow.hide();
        setInput("");
      } else if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSubmit();
      }
    },
    [appWindow, handleSubmit],
  );

  return (
    <div className="quick-action-root">
      <div className="quick-action-bar">
        <div className="quick-action-icon">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
        </div>
        <input
          ref={inputRef}
          type="text"
          className="quick-action-input"
          placeholder="问小林任何事…"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          disabled={isSubmitting}
          autoFocus
        />
        {input.trim() && (
          <button
            className="quick-action-send"
            onClick={handleSubmit}
            disabled={isSubmitting}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <line x1="22" y1="2" x2="11" y2="13" />
              <polygon points="22 2 15 22 11 13 2 9 22 2" />
            </svg>
          </button>
        )}
      </div>
    </div>
  );
}
