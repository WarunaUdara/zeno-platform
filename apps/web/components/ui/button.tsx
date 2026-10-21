import Link from "next/link";
import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

type ButtonVariant = "primary" | "secondary" | "outline" | "ghost";
type ButtonSize = "default" | "sm" | "lg" | "icon";

const variants: Record<ButtonVariant, string> = {
  primary: "bg-primary text-primary-foreground hover:bg-primary/90",
  secondary: "bg-secondary text-secondary-foreground hover:bg-secondary/80",
  outline: "border border-input bg-transparent text-foreground hover:bg-accent hover:text-accent-foreground",
  ghost: "bg-transparent text-muted-foreground hover:bg-accent hover:text-foreground",
};

const sizes: Record<ButtonSize, string> = {
  default: "h-9 px-4 py-2",
  sm: "h-8 rounded-md px-3 text-xs",
  lg: "h-10 rounded-md px-6",
  icon: "size-9 p-0",
};

type SharedProps = {
  variant?: ButtonVariant;
  size?: ButtonSize;
  className?: string;
};

type ButtonProps = SharedProps &
  Omit<ComponentProps<"button">, "className"> & {
    href?: never;
  };

type ButtonLinkProps = SharedProps &
  Omit<ComponentProps<typeof Link>, "className" | "href"> & {
    href: string;
  };

const base = "inline-flex shrink-0 items-center justify-center gap-2 whitespace-nowrap rounded-md text-[0.9375rem] font-medium transition-colors outline-none focus-visible:ring-[3px] focus-visible:ring-ring/40 disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0";

export function Button(props: ButtonProps | ButtonLinkProps) {
  const { variant = "primary", size = "default", className } = props;
  const classes = cn(base, variants[variant], sizes[size], className);

  if ("href" in props && props.href) {
    const { href, ...linkProps } = props;
    return <Link {...linkProps} data-slot="button" className={classes} href={href} />;
  }

  const buttonProps: ComponentProps<"button"> = props as ButtonProps;
  return <button {...buttonProps} data-slot="button" className={classes} />;
}
