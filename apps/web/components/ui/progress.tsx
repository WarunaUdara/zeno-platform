import type { HTMLAttributes } from "react";
import { cn } from "@/lib/utils";

export function Progress({
  value,
  className,
  ...props
}: HTMLAttributes<HTMLDivElement> & { value: number }) {
  const percentage = Math.min(100, Math.max(0, value));
  return (
    <div
      aria-valuemax={100}
      aria-valuemin={0}
      aria-valuenow={percentage}
      data-slot="progress"
      role="progressbar"
      className={cn("relative h-1.5 w-full overflow-hidden rounded-full bg-secondary", className)}
      {...props}
    >
      <div className="h-full bg-primary transition-[width]" style={{ width: `${percentage}%` }} />
    </div>
  );
}
