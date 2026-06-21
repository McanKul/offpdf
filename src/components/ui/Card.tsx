import type { HTMLAttributes, ReactNode } from "react";

export interface CardProps extends HTMLAttributes<HTMLDivElement> {
  /** Adds default internal padding. */
  padded?: boolean;
  /** Hover/cursor affordance for clickable cards. */
  interactive?: boolean;
  children: ReactNode;
}

export function Card({
  padded = false,
  interactive = false,
  className,
  children,
  ...rest
}: CardProps) {
  const classes = [
    "card",
    padded ? "card--pad" : "",
    interactive ? "card--interactive" : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <div className={classes} {...rest}>
      {children}
    </div>
  );
}

export function CardHeader({ title, description }: { title: ReactNode; description?: ReactNode }) {
  return (
    <div className="card__header">
      <div className="card__title">{title}</div>
      {description && <div className="card__desc">{description}</div>}
    </div>
  );
}

export function CardBody({ children }: { children: ReactNode }) {
  return <div className="card__body">{children}</div>;
}

export function CardFooter({ children }: { children: ReactNode }) {
  return <div className="card__footer">{children}</div>;
}
