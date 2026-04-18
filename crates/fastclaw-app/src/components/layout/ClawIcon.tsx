interface ClawIconProps {
  size?: number;
  className?: string;
}

export function ClawIcon({ size = 24, className }: ClawIconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 1024 1024"
      fill="none"
      className={className}
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect width="1024" height="1024" rx="220" ry="220" fill="currentColor" opacity="0.12" />
      <g stroke="currentColor" strokeLinecap="round" opacity="0.85" strokeWidth="52">
        <path d="M 310 230 C 330 420, 290 600, 280 790" />
        <path d="M 500 190 C 520 420, 500 600, 490 810" strokeWidth="56" />
        <path d="M 690 230 C 710 420, 690 600, 700 790" />
      </g>
    </svg>
  );
}
