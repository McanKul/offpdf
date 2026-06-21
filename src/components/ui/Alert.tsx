import type { ReactNode } from "react";
import { Icon, type IconName } from "./Icon";

export type AlertVariant = "info" | "success" | "warning" | "danger";

const ICONS: Record<AlertVariant, IconName> = {
  info: "info",
  success: "checkCircle",
  warning: "alertTriangle",
  danger: "alertTriangle",
};

export function Alert({
  variant = "info",
  title,
  children,
  icon,
}: {
  variant?: AlertVariant;
  title?: ReactNode;
  children?: ReactNode;
  icon?: IconName;
}) {
  return (
    <div className={`alert alert--${variant}`} role="note">
      <Icon name={icon ?? ICONS[variant]} size={18} className="alert__icon" />
      <div>
        {title && <div className="alert__title">{title}</div>}
        {children && <div className="alert__body">{children}</div>}
      </div>
    </div>
  );
}
