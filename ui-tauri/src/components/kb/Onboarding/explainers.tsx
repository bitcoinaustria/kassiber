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
                    ? "border-[var(--color-accent)] bg-paper-2"
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
                      <span className="ml-2 font-mono text-[9px] font-medium uppercase tracking-[0.12em] text-[var(--color-accent)]">
                        Selected
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

export const SyncExplainer = ({ form }: { form: OnboardingForm }) => (
  <ExplainerShell
    icon={Network}
    eyebrow="Why this matters"
    title="How syncing works"
    lead="A sync backend discovers your watch-only addresses and refreshes balances and history from the network. Kassiber stays watch-only — private keys never enter the app."
    activeKey={form.backendSetupMode}
    options={[
      {
        key: "default",
        icon: Globe2,
        title: "Built-in public backends",
        body: "Bundled Esplora, Electrum, and Liquid endpoints. Fast start, but the operator can still see your address queries — use Tor/VPN or run your own if privacy matters.",
      },
      {
        key: "custom",
        icon: ServerCog,
        title: "Custom sync backend",
        body: "Point at your own node or a trusted server. Best privacy and sovereignty; you manage the endpoint.",
      },
      {
        key: "skip",
        icon: AlertTriangle,
        title: "Skip for now",
        body: "Manual imports only. No address discovery or refresh until you add a backend from Settings.",
      },
    ]}
    footer="Whichever you pick, queries reveal which addresses you watch to that server — running your own keeps it private. Credentials are added later, after the encrypted database is open."
  />
);

export const AiExplainer = ({ form }: { form: OnboardingForm }) => (
  <ExplainerShell
    icon={Sparkles}
    eyebrow="Why this matters"
    title="How the assistant works"
    lead="The assistant helps categorize transactions, draft notes, and answer questions about your books. It is optional and can be changed anytime in Settings."
    activeKey={form.aiSetupMode}
    options={[
      {
        key: "local",
        icon: BrainCircuit,
        title: "Local endpoint",
        body: "Runs against a model on this machine (Ollama by default). Prompts and accounting context stay on-device.",
      },
      {
        key: "remote",
        icon: ShieldAlert,
        title: "Remote provider",
        body: "An OpenAI-compatible endpoint or Claude/Codex CLI. Prompts and accounting context may leave this device.",
      },
      {
        key: "disabled",
        icon: Power,
        title: "Disabled",
        body: "Hides the Assistant and floating chat. You can turn AI back on later in Settings.",
      },
    ]}
    footer="No API keys or secrets are collected during setup. Accounting context — labels, notes, hostnames, report details — can be sensitive; keep that in mind for remote providers."
  />
);
