import {
  AlertTriangle,
  BrainCircuit,
  Globe2,
  Network,
  Power,
  ServerCog,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  type LucideIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

import type { OnboardingForm } from "./types";

interface ExplainerOption {
  key: string;
  icon: LucideIcon;
  title: string;
  body: string;
}

/**
 * Educational right-hand panel for the Sync and AI steps. Lists each option
 * with its real trade-off and highlights the one currently selected, so the
 * panel teaches the decision instead of decorating the page.
 */
const ExplainerShell = ({
  icon: Icon,
  eyebrow,
  title,
  lead,
  options,
  activeKey,
  footer,
}: {
  icon: LucideIcon;
  eyebrow: string;
  title: string;
  lead: string;
  options: ExplainerOption[];
  activeKey: string;
  footer: string;
}) => {
  const { t } = useTranslation("onboarding");
  return (
    <div className="flex h-full items-start">
      <div className="sticky top-8 w-full max-w-lg">
        <div className="flex size-10 items-center justify-center rounded-md bg-ink text-paper">
          <Icon className="size-5" />
        </div>
        <p className="mt-4 font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-ink-3">
          {eyebrow}
        </p>
        <h4 className="mt-2 text-lg font-semibold text-ink">{title}</h4>
        <p className="mt-2 text-sm leading-6 text-ink-2">{lead}</p>

        <ul className="mt-5 space-y-2.5">
          {options.map(({ key, icon: OptionIcon, title: optTitle, body }) => {
            const active = key === activeKey;
            return (
              <li
                key={key}
                className={cn(
                  "flex gap-3 rounded-lg border p-3.5 transition-colors",
                  active
                    ? "border-[var(--kb-accent)] bg-paper-2"
                    : "border-line bg-paper opacity-70",
                )}
              >
                <span
                  className={cn(
                    "mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md",
                    active ? "bg-ink text-paper" : "bg-paper-2 text-ink-2",
                  )}
                >
                  <OptionIcon className="size-4" />
                </span>
                <div className="min-w-0">
                  <p className="m-0 text-sm font-semibold text-ink">
                    {optTitle}
                    {active && (
                      <span className="ml-2 font-mono text-[9px] font-medium uppercase tracking-[0.12em] text-[var(--kb-accent)]">
                        {t("explainer.selected")}
                      </span>
                    )}
                  </p>
                  <p className="m-0 mt-1 text-xs leading-5 text-ink-2">{body}</p>
                </div>
              </li>
            );
          })}
        </ul>

        <div className="mt-5 flex items-start gap-3 border-t border-line pt-4 text-xs leading-5 text-ink-2">
          <ShieldCheck className="mt-0.5 size-4 shrink-0 text-ink" />
          <p className="m-0">{footer}</p>
        </div>
      </div>
    </div>
  );
};

export const SyncExplainer = ({ form }: { form: OnboardingForm }) => {
  const { t } = useTranslation("onboarding");
  return (
    <ExplainerShell
      icon={Network}
      eyebrow={t("explainer.whyThisMatters")}
      title={t("explainer.sync.title")}
      lead={t("explainer.sync.lead")}
      activeKey={form.backendSetupMode}
      options={[
        {
          key: "default",
          icon: Globe2,
          title: t("explainer.sync.default.title"),
          body: t("explainer.sync.default.body"),
        },
        {
          key: "custom",
          icon: ServerCog,
          title: t("explainer.sync.custom.title"),
          body: t("explainer.sync.custom.body"),
        },
        {
          key: "skip",
          icon: AlertTriangle,
          title: t("explainer.sync.skip.title"),
          body: t("explainer.sync.skip.body"),
        },
      ]}
      footer={t("explainer.sync.footer")}
    />
  );
};

export const AiExplainer = ({ form }: { form: OnboardingForm }) => {
  const { t } = useTranslation("onboarding");
  return (
    <ExplainerShell
      icon={Sparkles}
      eyebrow={t("explainer.whyThisMatters")}
      title={t("explainer.ai.title")}
      lead={t("explainer.ai.lead")}
      activeKey={form.aiSetupMode}
      options={[
        {
          key: "local",
          icon: BrainCircuit,
          title: t("explainer.ai.local.title"),
          body: t("explainer.ai.local.body"),
        },
        {
          key: "remote",
          icon: ShieldAlert,
          title: t("explainer.ai.remote.title"),
          body: t("explainer.ai.remote.body"),
        },
        {
          key: "disabled",
          icon: Power,
          title: t("explainer.ai.disabled.title"),
          body: t("explainer.ai.disabled.body"),
        },
      ]}
      footer={t("explainer.ai.footer")}
    />
  );
};
