---
name: zustand5-react19-selector-pitfall
description: Avoid and fix Zustand 5 with React 19 selector stability bugs, especially infinite render loops from selectors returning new references or unstable useShallow usage. Use when writing Zustand selectors, debugging Maximum update depth exceeded, or reviewing React store subscriptions in XiaoLin.
---

# Zustand 5 + React 19: Selector Stability Requirements

## When to Use
Use when writing Zustand store selectors in this project. Applies any time you want to derive/transform state inside a selector, or use `useShallow` from `zustand/react/shallow`.

## Critical Constraint

In **Zustand 5.0.12 + React 19**, `useShallow` and any selector that returns a **new object reference** on each call will cause an infinite render loop:

```
Error: The result of getSnapshot should be cached to avoid an infinite loop
Error: Maximum update depth exceeded
```

### Root Cause

Zustand 5's `useStore` implementation:
```js
function useStore(api, selector = identity) {
  const slice = React.useSyncExternalStore(
    api.subscribe,
    React.useCallback(() => selector(api.getState()), [api, selector]),
    //                                                       ^^^^^^^^
    // selector is a useCallback dependency
  );
}
```

`useShallow` returns a **new closure every render** (because it uses `useRef` internally). This causes `useCallback` to create a new function on each render, triggering `useSyncExternalStore` to re-subscribe, which re-evaluates the snapshot, which triggers another render.

### What DOES NOT Work

```tsx
// BROKEN: useShallow creates new closure → infinite loop
const data = useAgentStore(useShallow((s) => ({
  name: s.name,
  count: s.count,
})));

// BROKEN: Selector creates new object every call
const summaries = useAgentStore((s) => {
  const result = {};
  for (const [id, v] of Object.entries(s.items)) {
    result[id] = { label: v.label };
  }
  return result;  // new object reference → infinite loop
});
```

### What DOES Work

```tsx
// OK: Returns stable reference from store (same object instance)
const agentChats = useAgentStore((s) => s.agentChats);

// OK: Returns stable reference (sub-object from store)
const activeData = useAgentStore((s) => s.agentChats[s.activeAgentId]);

// OK: Returns primitive value
const count = useAgentStore((s) => s.items.length);
```

### Optimization Strategies

Since derived selectors are unsafe, use these patterns instead:

1. **Subscribe to a stable sub-reference** then derive in component body:
   ```tsx
   const ac = useAgentStore((s) => s.agentChats[agentId]);
   const summary = useMemo(() => ({ lastMsg: ac?.lastMsg }), [ac?.lastMsg]);
   ```

2. **React.memo child components** to prevent cascading re-renders:
   ```tsx
   const AgentRow = memo(function AgentRow({ lastMsg, lastTime }: Props) { ... });
   ```

3. **Per-item subscriptions** in list items (each item subscribes to its own data):
   ```tsx
   function AgentItem({ id }: { id: string }) {
     const data = useAgentStore((s) => s.agentChats[id]);
     // Only re-renders when THIS agent's data changes
   }
   ```

## Project-Specific Details

- Store location: `src/lib/stores/index.ts`
- Safe selectors: `src/lib/stores/selectors.ts` (all return store references)
- Zustand version: 5.0.12
- React version: 19.x
