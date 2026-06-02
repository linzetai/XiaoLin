export interface FuzzyResult {
  score: number;
  indices: number[];
}

/**
 * fzf-style sequential fuzzy matching.
 * Returns null if no match, otherwise { score, indices }.
 * Higher score = better match. Consecutive matches and
 * word-boundary matches are rewarded.
 */
export function fuzzyMatch(pattern: string, text: string): FuzzyResult | null {
  if (!pattern) return { score: 0, indices: [] };

  const pLower = pattern.toLowerCase();
  const tLower = text.toLowerCase();
  const pLen = pLower.length;
  const tLen = tLower.length;

  if (pLen > tLen) return null;

  const indices: number[] = [];
  let pi = 0;

  for (let ti = 0; ti < tLen && pi < pLen; ti++) {
    if (tLower[ti] === pLower[pi]) {
      indices.push(ti);
      pi++;
    }
  }

  if (pi < pLen) return null;

  let score = 0;
  for (let i = 0; i < indices.length; i++) {
    const idx = indices[i];
    if (i > 0 && indices[i] === indices[i - 1] + 1) {
      score += 8;
    }
    if (idx === 0 || text[idx - 1] === "/" || text[idx - 1] === "\\" ||
        text[idx - 1] === "." || text[idx - 1] === "-" || text[idx - 1] === "_" ||
        text[idx - 1] === " ") {
      score += 6;
    }
    if (text[idx] === pattern[i]) {
      score += 2;
    }
    score += 1;
  }

  if (indices[0] === 0) score += 4;
  score -= indices[indices.length - 1] - indices[0] - indices.length + 1;

  return { score, indices };
}

/**
 * Filter + sort items by fuzzy match score.
 * Returns items with their matched indices.
 */
export function fuzzyFilter<T>(
  items: T[],
  pattern: string,
  getText: (item: T) => string,
  getAlt?: (item: T) => string | undefined,
): Array<{ item: T; result: FuzzyResult }> {
  if (!pattern) {
    return items.map((item) => ({ item, result: { score: 0, indices: [] } }));
  }

  const results: Array<{ item: T; result: FuzzyResult }> = [];

  for (const item of items) {
    const primary = fuzzyMatch(pattern, getText(item));
    const alt = getAlt ? fuzzyMatch(pattern, getAlt(item) ?? "") : null;

    const best = primary && alt
      ? (primary.score >= alt.score ? primary : alt)
      : primary ?? alt;

    if (best) {
      results.push({ item, result: best });
    }
  }

  results.sort((a, b) => b.result.score - a.result.score);
  return results;
}
