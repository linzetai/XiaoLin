import { forwardRef, type InputHTMLAttributes, type ButtonHTMLAttributes, type SelectHTMLAttributes } from "react";

export const inputCls = "w-full rounded-[var(--radius-xs)] px-3 py-2 text-[13px] outline-none transition-all duration-150 focus:ring-2 focus:ring-[var(--tint)]";
export const inputStyle: React.CSSProperties = {
  background: "var(--bg-base)",
  color: "var(--fill-primary)",
  border: "0.5px solid var(--separator-opaque)",
};

export const labelCls = "mb-1.5 block text-[11px] font-medium";
export const labelStyle: React.CSSProperties = { color: "var(--fill-tertiary)" };

export interface FormInputProps extends InputHTMLAttributes<HTMLInputElement> {
  hasError?: boolean;
}

export const FormInput = forwardRef<HTMLInputElement, FormInputProps>(
  function FormInput({ className = "", hasError, style, ...props }, ref) {
    return (
      <input
        ref={ref}
        className={`${inputCls} ${className}`}
        style={{
          ...inputStyle,
          borderColor: hasError ? "var(--red, #FC8181)" : undefined,
          ...style,
        }}
        {...props}
      />
    );
  },
);

export interface FormSelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  hasError?: boolean;
}

export const FormSelect = forwardRef<HTMLSelectElement, FormSelectProps>(
  function FormSelect({ className = "", children, style, ...props }, ref) {
    return (
      <select
        ref={ref}
        className={`${inputCls} appearance-none cursor-pointer ${className}`}
        style={{ ...inputStyle, paddingRight: 32, ...style }}
        {...props}
      >
        {children}
      </select>
    );
  },
);

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

const BUTTON_STYLES: Record<ButtonVariant, { base: React.CSSProperties; hover: string }> = {
  primary: {
    base: { background: "var(--tint)", color: "#fff" },
    hover: "hover:opacity-90",
  },
  secondary: {
    base: { background: "var(--bg-secondary)", color: "var(--fill-secondary)", border: "0.5px solid var(--separator-opaque)" },
    hover: "hover:bg-[var(--bg-hover)]",
  },
  ghost: {
    base: { background: "transparent", color: "var(--fill-secondary)" },
    hover: "hover:bg-[var(--bg-hover)]",
  },
  danger: {
    base: { background: "transparent", color: "var(--red)" },
    hover: "hover:opacity-80",
  },
};

export interface FormButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
}

export const FormButton = forwardRef<HTMLButtonElement, FormButtonProps>(
  function FormButton({ variant = "primary", className = "", style, children, disabled, ...props }, ref) {
    const vs = BUTTON_STYLES[variant];
    return (
      <button
        ref={ref}
        className={`rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium transition-colors duration-150 disabled:cursor-not-allowed disabled:opacity-50 ${vs.hover} ${className}`}
        style={{
          ...vs.base,
          cursor: disabled ? "not-allowed" : "pointer",
          ...style,
        }}
        disabled={disabled}
        {...props}
      >
        {children}
      </button>
    );
  },
);

export function FormLabel({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  return (
    <label className={`${labelCls} ${className}`} style={labelStyle}>
      {children}
    </label>
  );
}
