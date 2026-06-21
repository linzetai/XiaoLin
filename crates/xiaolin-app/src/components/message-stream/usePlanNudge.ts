import { useState, useEffect, useRef, useCallback } from "react";
import type { InlineMention } from "./MentionInput";

export type NudgeRuleId = "keyword" | "complexity" | "education";

export interface NudgeState {
  visible: boolean;
  messageKey: string;
  ruleId: NudgeRuleId;
}

const HIDDEN: NudgeState = { visible: false, messageKey: "", ruleId: "keyword" };

const PLAN_KEYWORDS_ZH = ["规划", "设计方案", "架构设计", "重构方案", "实施计划"];
const PLAN_KEYWORDS_EN_RE = /\b(?:plan|design|architecture|refactor)\b/i;

const ACTION_VERBS_RE = /(?:添加|修改|删除|创建|更新|移除|重构|create|update|delete|add|remove|modify|refactor)/gi;
const LIST_ITEM_RE = /(?:^|\n)\s*(?:\d+[.)]\s|[-*]\s)/g;

const MIN_INTERVAL_MS = 5 * 60 * 1000;

function containsPlanKeyword(text: string): boolean {
  for (const kw of PLAN_KEYWORDS_ZH) {
    if (text.includes(kw)) return true;
  }
  return PLAN_KEYWORDS_EN_RE.test(text);
}

function detectComplexity(text: string, mentions: InlineMention[]): boolean {
  let score = 0;
  if (text.length > 200) score++;
  const listItems = text.match(LIST_ITEM_RE);
  if (listItems && listItems.length >= 3) score++;
  if (mentions.filter((m) => m.type === "file").length >= 3) score++;
  const verbMatches = text.match(ACTION_VERBS_RE);
  if (verbMatches && verbMatches.length >= 3) score++;
  return score >= 2;
}

function isDismissed(sessionId: string, ruleId: NudgeRuleId): boolean {
  try {
    const raw = localStorage.getItem(`xiaolin:plan-discovery-dismissed-${sessionId}`);
    if (!raw) return false;
    const arr = JSON.parse(raw) as string[];
    return arr.includes(ruleId);
  } catch {
    return false;
  }
}

function markDismissedInStorage(sessionId: string, ruleId: NudgeRuleId) {
  try {
    const key = `xiaolin:plan-discovery-dismissed-${sessionId}`;
    const raw = localStorage.getItem(key);
    const arr: string[] = raw ? JSON.parse(raw) : [];
    if (!arr.includes(ruleId)) arr.push(ruleId);
    localStorage.setItem(key, JSON.stringify(arr));
  } catch { /* ignore */ }
}

function isRateLimited(): boolean {
  try {
    const last = localStorage.getItem("xiaolin:plan-nudge-last-shown");
    if (!last) return false;
    return Date.now() - Number(last) < MIN_INTERVAL_MS;
  } catch {
    return false;
  }
}

function recordShown() {
  try {
    localStorage.setItem("xiaolin:plan-nudge-last-shown", String(Date.now()));
  } catch { /* ignore */ }
}

function getEducationCount(): number {
  try {
    return Number(localStorage.getItem("xiaolin:plan-nudge-education-count") || "0");
  } catch {
    return 0;
  }
}

function incrementEducationCount() {
  try {
    const c = getEducationCount() + 1;
    localStorage.setItem("xiaolin:plan-nudge-education-count", String(c));
  } catch { /* ignore */ }
}

function hasUsedPlanMode(): boolean {
  try {
    return !!localStorage.getItem("xiaolin:plan-mode-ever-used");
  } catch {
    return false;
  }
}

type EvalResult = { ruleId: NudgeRuleId; messageKey: string } | null;

function evaluate(
  text: string,
  mentions: InlineMention[],
  sessionId: string,
  messageCount: number,
  complexityCount: number,
): EvalResult {
  if (!text.trim() || text.startsWith("/")) return null;
  if (isRateLimited()) return null;

  if (!isDismissed(sessionId, "keyword") && containsPlanKeyword(text)) {
    return { ruleId: "keyword", messageKey: "plan_nudge_keyword" };
  }

  if (!isDismissed(sessionId, "complexity") && complexityCount < 2 && detectComplexity(text, mentions)) {
    return { ruleId: "complexity", messageKey: "plan_nudge_complexity" };
  }

  if (
    !isDismissed(sessionId, "education") &&
    !hasUsedPlanMode() &&
    messageCount >= 5 &&
    text.length > 100 &&
    getEducationCount() < 3
  ) {
    return { ruleId: "education", messageKey: "plan_nudge_education" };
  }

  return null;
}

export function usePlanNudge(
  text: string,
  mentions: InlineMention[],
  executionMode: string,
  sessionId: string,
  messageCount: number,
): NudgeState & { dismiss: () => void } {
  const [nudge, setNudge] = useState<NudgeState>(HIDDEN);
  const complexityCountRef = useRef(0);
  const nudgeRef = useRef(nudge);
  nudgeRef.current = nudge;

  const dismiss = useCallback(() => {
    const cur = nudgeRef.current;
    if (cur.visible && sessionId) {
      markDismissedInStorage(sessionId, cur.ruleId);
      setNudge(HIDDEN);
    }
  }, [sessionId]);

  useEffect(() => {
    setNudge(HIDDEN);
    complexityCountRef.current = 0;
  }, [sessionId]);

  useEffect(() => {
    if (executionMode === "plan" || !sessionId) {
      setNudge(HIDDEN);
      return;
    }

    const timer = setTimeout(() => {
      const result = evaluate(text, mentions, sessionId, messageCount, complexityCountRef.current);
      if (result) {
        if (result.ruleId === "complexity") complexityCountRef.current++;
        if (result.ruleId === "education") incrementEducationCount();
        recordShown();
        setNudge({ visible: true, messageKey: result.messageKey, ruleId: result.ruleId });
      } else {
        setNudge(HIDDEN);
      }
    }, 400);

    return () => clearTimeout(timer);
  }, [text, mentions, executionMode, sessionId, messageCount]);

  return { ...nudge, dismiss };
}
