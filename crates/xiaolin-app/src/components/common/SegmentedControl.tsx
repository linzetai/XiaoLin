interface SegmentedControlItem<T extends string> {
  value: T;
  label: string;
  count?: number;
}

interface SegmentedControlProps<T extends string> {
  value: T;
  onChange: (val: T) => void;
  items: SegmentedControlItem<T>[];
}

export function SegmentedControl<T extends string>({
  value,
  onChange,
  items,
}: SegmentedControlProps<T>) {
  return (
    <div
      className="flex items-center gap-1 rounded-lg p-0.5"
      style={{ background: "var(--bg-tertiary)" }}
    >
      {items.map((item) => (
        <button
          key={item.value}
          onClick={() => onChange(item.value)}
          className="flex-1 rounded-md px-3 py-1.5 text-[12px] font-medium transition-all duration-150"
          style={{
            background: value === item.value ? "var(--bg-elevated)" : "transparent",
            color: value === item.value ? "var(--fill-primary)" : "var(--fill-tertiary)",
            boxShadow: value === item.value ? "var(--shadow-sm)" : "none",
            cursor: "pointer",
            border: "none",
          }}
        >
          {item.label}
          {item.count != null && ` (${item.count})`}
        </button>
      ))}
    </div>
  );
}
