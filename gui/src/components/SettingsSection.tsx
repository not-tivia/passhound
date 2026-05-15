import type { ReactNode } from "react";

interface SettingsSectionProps {
  title: string;
  children: ReactNode;
}

export default function SettingsSection({ title, children }: SettingsSectionProps) {
  return (
    <div className="settings-section">
      <div className="settings-section__title">{title}</div>
      <div className="settings-section__body">{children}</div>
    </div>
  );
}
