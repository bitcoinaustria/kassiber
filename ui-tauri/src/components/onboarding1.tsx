"use client";

import React, { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  AlertTriangle,
  Check,
  ChevronLeft,
  Database,
  Globe2,
  KeyRound,
  LockKeyhole,
  ServerCog,
  ShieldCheck,
  WalletCards,
} from "lucide-react";
import { motion } from "motion/react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Wordmark } from "@/components/kb/Wordmark";
import { cn } from "@/lib/utils";
import { useUiStore, type Identity } from "@/store/ui";

type TaxCountry = "at" | "generic";
type FiatCurrency = "EUR" | "USD" | "CHF" | "GBP";
type GainsAlgorithm = "FIFO" | "LIFO" | "HIFO" | "LOFO";
type DatabaseMode = "sqlcipher" | "plaintext";
type BackendSetupMode = "default" | "custom" | "skip";
type BackendKind =
  | "esplora"
  | "electrum"
  | "bitcoinrpc"
  | "btcpay"
  | "liquid-esplora"
  | "custom";

const DEFAULT_BACKEND_NAME = "mempool";
const DEFAULT_BACKEND_URL = "https://mempool.bitcoin-austria.at/api";

interface OnboardingForm {
  name: string;
  workspace: string;
  profile: string;
  taxCountry: TaxCountry;
  fiatCurrency: FiatCurrency;
  taxLongTermDays: string;
  gainsAlgorithm: GainsAlgorithm;
  databaseMode: DatabaseMode;
  recoveryAcknowledged: boolean;
  plaintextAcknowledged: boolean;
  migrateCredentials: boolean;
  backendSetupMode: BackendSetupMode;
  backendKind: BackendKind;
  backendName: string;
  backendUrl: string;
  skipBackendsAcknowledged: boolean;
}

interface StepComponentProps {
  form: OnboardingForm;
  update: <K extends keyof OnboardingForm>(
    key: K,
    value: OnboardingForm[K],
  ) => void;
  onSubmit: () => void;
  goBack?: () => void;
  currentStep: number;
  totalSteps: number;
}

interface OnboardingStep {
  component: React.ComponentType<StepComponentProps>;
  isComplete: (form: OnboardingForm) => boolean;
}

interface OnboardingStepHeaderProps {
  title: string;
  eyebrow: string;
  stepIndex: number;
  totalSteps: number;
  goBack?: () => void;
}

const DEFAULT_FORM: OnboardingForm = {
  name: "",
  workspace: "Personal",
  profile: "main",
  taxCountry: "at",
  fiatCurrency: "EUR",
  taxLongTermDays: "365",
  gainsAlgorithm: "FIFO",
  databaseMode: "sqlcipher",
  recoveryAcknowledged: false,
  plaintextAcknowledged: false,
  migrateCredentials: true,
  backendSetupMode: "default",
  backendKind: "esplora",
  backendName: DEFAULT_BACKEND_NAME,
  backendUrl: DEFAULT_BACKEND_URL,
  skipBackendsAcknowledged: false,
};

const FIAT_CURRENCIES: FiatCurrency[] = ["EUR", "USD", "CHF", "GBP"];
const GAINS_ALGORITHMS: GainsAlgorithm[] = ["FIFO", "LIFO", "HIFO", "LOFO"];
const BACKEND_KINDS: BackendKind[] = [
  "esplora",
  "electrum",
  "bitcoinrpc",
  "btcpay",
  "liquid-esplora",
  "custom",
];

const BACKEND_KIND_LABELS: Record<BackendKind, string> = {
  esplora: "Esplora",
  electrum: "Electrum",
  bitcoinrpc: "Bitcoin Core RPC",
  btcpay: "BTCPay",
  "liquid-esplora": "Liquid Esplora",
  custom: "Custom",
};

const PUBLIC_BACKEND_DEFAULTS = [
  [DEFAULT_BACKEND_NAME, "Esplora", DEFAULT_BACKEND_URL],
  ["fulcrum", "Electrum", "ssl://index.bitcoin-austria.at:50002"],
  ["liquid", "Electrum", "ssl://les.bullbitcoin.com:995"],
];

const OnboardingStepHeader = ({
  title,
  eyebrow,
  stepIndex,
  totalSteps,
  goBack,
}: OnboardingStepHeaderProps) => {
  return (
    <div className="relative">
      {goBack && stepIndex > 0 && (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          onClick={goBack}
          className="absolute right-full top-1/2 -translate-x-1/2 -translate-y-1/2 text-ink-2"
          aria-label="Go back"
        >
          <ChevronLeft className="size-4" />
        </Button>
      )}
      <div>
        <p className="font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-ink-3">
          {eyebrow} - {stepIndex + 1}/{totalSteps}
        </p>
        <h3 className="mt-2 text-2xl font-semibold tracking-normal text-ink md:whitespace-nowrap">
          {title}
        </h3>
      </div>
    </div>
  );
};

const OnboardingStepFrame = ({ children }: { children: React.ReactNode }) => {
  return (
    <div className="flex w-full flex-col-reverse gap-8 rounded-lg border border-line bg-paper md:min-h-[78dvh] md:flex-row lg:rounded-lg">
      {children}
    </div>
  );
};

const OnboardingStepLeftWrapper = ({
  title,
  eyebrow,
  currentStep,
  totalSteps,
  goBack,
  children,
}: {
  title: string;
  eyebrow: string;
  currentStep: number;
  totalSteps: number;
  children: React.ReactNode;
  goBack?: () => void;
}) => {
  return (
    <div className="flex flex-1/2 justify-center px-5 py-6 sm:px-10 sm:py-10 md:py-16 lg:justify-start lg:pl-20">
      <div className="flex h-full w-full max-w-md shrink-0 flex-col gap-6">
        <OnboardingStepHeader
          title={title}
          eyebrow={eyebrow}
          stepIndex={currentStep}
          totalSteps={totalSteps}
          goBack={goBack}
        />
        {children}
      </div>
    </div>
  );
};

const OnboardingStepRightWrapper = ({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) => {
  return (
    <div
      className={cn(
        "hidden flex-1/2 overflow-hidden border-b border-line bg-paper-2 md:block md:border-b-0 md:border-l",
        className,
      )}
    >
      {children}
    </div>
  );
};

const DashboardIllustration = ({
  form,
  variant = "zoomed-out",
  transformOrigin = "-20% -10%",
}: {
  form: OnboardingForm;
  variant?: "zoomed-in" | "zoomed-out";
  transformOrigin?: string;
}) => {
  const workspace = form.workspace.trim() || "Personal";
  const profile = form.profile.trim() || "main";
  return (
    <motion.div
      style={{ transformOrigin }}
      animate={{ scale: variant === "zoomed-in" ? 1.3 : 1 }}
      transition={{ type: "spring", stiffness: 300, damping: 40 }}
      className="flex h-full min-h-[520px] w-[980px] overflow-hidden rounded-lg border border-line bg-paper shadow-sm"
    >
      <div className="h-full w-[280px] shrink-0 overflow-hidden bg-paper-2">
        <div className="flex items-center justify-between gap-2 border-b border-line p-4">
          <div className="flex min-w-0 items-center gap-2">
            <div className="flex size-9 shrink-0 items-center justify-center rounded-md bg-ink text-paper">
              <WalletCards className="size-4" />
            </div>
            <div className="min-w-0">
              <p className="truncate font-semibold text-ink">{workspace}</p>
              <p className="font-mono text-[10px] uppercase tracking-[0.12em] text-ink-3">
                {profile}
              </p>
            </div>
          </div>
          <ChevronLeft className="size-4 text-ink-3" />
        </div>
        <ul className="space-y-2 p-4">
          {["Overview", "Connections", "Transactions", "Reports", "Profiles"].map(
            (item, index) => (
              <li
                key={item}
                className={cn(
                  "rounded-md border px-3 py-2 text-xs",
                  index === 0
                    ? "border-ink bg-paper text-ink"
                    : "border-line bg-paper/70 text-ink-2",
                )}
              >
                {item}
              </li>
            ),
          )}
        </ul>
      </div>
      <div className="flex min-w-0 flex-1 flex-col justify-between p-4">
        <div className="space-y-4">
          <div className="flex items-center justify-between gap-4">
            <div className="space-y-1">
              <div className="h-4 w-40 rounded-md bg-ink" />
              <div className="h-3 w-64 rounded-md bg-line-2" />
            </div>
            <Button variant="outline" size="sm">
              Add wallet
            </Button>
          </div>

          <div className="grid grid-cols-4 gap-3">
            {[
              ["Policy", form.taxCountry === "at" ? "Austria" : "Generic"],
              ["Currency", form.fiatCurrency],
              [
                "Backend",
                form.backendSetupMode === "skip"
                  ? "Skipped"
                  : form.backendSetupMode === "custom"
                    ? BACKEND_KIND_LABELS[form.backendKind]
                    : "Built-ins",
              ],
              ["Database", form.databaseMode === "sqlcipher" ? "SQLCipher" : "Plain"],
            ].map(([label, value]) => (
              <div key={label} className="rounded-lg border border-line p-3">
                <div className="font-mono text-[10px] uppercase tracking-[0.12em] text-ink-3">
                  {label}
                </div>
                <div className="mt-2 text-lg font-semibold text-ink">
                  {value}
                </div>
              </div>
            ))}
          </div>

          <div className="grid grid-cols-3 gap-3">
            {[
              [
                "Endpoint",
                form.backendSetupMode === "custom"
                  ? form.backendName || "custom"
                  : form.backendSetupMode === "skip"
                    ? "none"
                    : "mempool",
              ],
              ["Sync", form.backendSetupMode === "skip" ? "manual import" : "enabled"],
              ["Secrets", "encrypted path"],
            ].map(([label, value]) => (
              <div key={label} className="rounded-lg border border-line p-3">
                <div className="font-mono text-[10px] uppercase tracking-[0.12em] text-ink-3">
                  {label}
                </div>
                <div className="mt-2 text-lg font-semibold text-ink">
                  {value}
                </div>
              </div>
            ))}
          </div>

          <div className="overflow-hidden rounded-lg border border-line">
            <Table>
              <TableHeader>
                <TableRow className="bg-paper-2">
                  {["Source", "Asset", "Status", "Scope"].map((head) => (
                    <TableHead key={head} className="h-9 border-r last:border-r-0">
                      {head}
                    </TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {[
                  ["Treasury", "BTC", "watch-only", "local"],
                  ["BTCPay", "BTC", "credentials encrypted", "profile"],
                  ["Liquid", "LBTC", "manual pair", "audit"],
                  ["Reports", form.fiatCurrency, form.gainsAlgorithm, "tax"],
                ].map((row) => (
                  <TableRow key={row.join("-")} className="even:bg-paper-2/60">
                    {row.map((cell) => (
                      <TableCell key={cell} className="h-10 border-r last:border-r-0">
                        {cell}
                      </TableCell>
                    ))}
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {["Sync", "Journal", "Report"].map((item) => (
            <div
              key={item}
              className="rounded-md border border-line bg-paper-2 px-3 py-2 text-xs text-ink-2"
            >
              {item}
            </div>
          ))}
        </div>
      </div>
    </motion.div>
  );
};

const TextField = ({
  label,
  name,
  value,
  placeholder,
  onChange,
}: {
  label: string;
  name: string;
  value: string;
  placeholder: string;
  onChange: (value: string) => void;
}) => {
  return (
    <div className="space-y-2">
      <Label htmlFor={name}>{label}</Label>
      <Input
        id={name}
        name={name}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
        className="w-full rounded-md border-line"
      />
    </div>
  );
};

const SelectField = <T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: T[];
  onChange: (value: T) => void;
}) => {
  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      <Select value={value} onValueChange={(next) => onChange(next as T)}>
        <SelectTrigger className="w-full rounded-md border-line">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem key={option} value={option}>
              {option}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
};

const ChoiceCard = ({
  active,
  title,
  description,
  onClick,
  tone = "default",
}: {
  active: boolean;
  title: string;
  description: string;
  onClick: () => void;
  tone?: "default" | "warning";
}) => {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "flex min-h-[112px] cursor-pointer items-start gap-3 rounded-lg border p-4 text-left text-sm transition",
        active
          ? tone === "warning"
            ? "border-accent bg-[rgba(227,0,15,0.04)]"
            : "border-ink bg-paper"
          : "border-line hover:bg-paper-2",
      )}
    >
      <span
        className={cn(
          "mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full border",
          active ? "border-ink bg-ink text-paper" : "border-line",
        )}
      >
        {active && <Check className="size-3.5" />}
      </span>
      <span>
        <span className="block font-semibold text-ink">{title}</span>
        <span className="mt-1 block text-xs leading-5 text-ink-2">
          {description}
        </span>
      </span>
    </button>
  );
};

const CheckRow = ({
  id,
  checked,
  onCheckedChange,
  label,
  description,
}: {
  id: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  label: string;
  description: string;
}) => {
  return (
    <div className="flex items-start gap-3 rounded-lg border border-line p-3">
      <Checkbox
        id={id}
        checked={checked}
        onCheckedChange={(value) => onCheckedChange(value === true)}
        className="mt-0.5"
      />
      <div className="grid gap-1">
        <Label htmlFor={id} className="font-semibold text-ink">
          {label}
        </Label>
        <p className="m-0 text-xs leading-5 text-ink-2">{description}</p>
      </div>
    </div>
  );
};

const StepOneComponent = ({
  form,
  update,
  onSubmit,
  currentStep,
  totalSteps,
}: StepComponentProps) => {
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Set up your local workspace"
        eyebrow="Identity"
        currentStep={currentStep}
        totalSteps={totalSteps}
      >
        <form onSubmit={(event) => event.preventDefault()} className="space-y-6 py-4">
          <div className="space-y-4 border-b border-line pb-6">
            <TextField
              label="Your name"
              name="name"
              value={form.name}
              placeholder="Alice"
              onChange={(value) => update("name", value)}
            />
            <TextField
              label="Workspace name"
              name="workspace"
              value={form.workspace}
              placeholder="Personal"
              onChange={(value) => update("workspace", value)}
            />
            <TextField
              label="Profile"
              name="profile"
              value={form.profile}
              placeholder="main"
              onChange={(value) => update("profile", value)}
            />
          </div>

          <div className="flex items-start gap-3 rounded-lg border border-line bg-paper-2 p-3 text-xs leading-5 text-ink-2">
            <WalletCards className="mt-0.5 size-4 shrink-0 text-ink" />
            <p className="m-0">
              Creating a profile seeds the first wallet/reporting bucket named
              <span className="font-mono text-ink"> treasury</span>. This is a
              bucket, not a double-entry chart of accounts.
            </p>
          </div>

          <Button type="submit" onClick={onSubmit} className="mt-4 w-full">
            Continue
          </Button>
        </form>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <DashboardIllustration form={form} />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};

const StepTwoComponent = ({
  form,
  update,
  currentStep,
  totalSteps,
  onSubmit,
  goBack,
}: StepComponentProps) => {
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Choose tax defaults"
        eyebrow="Accounting"
        currentStep={currentStep}
        totalSteps={totalSteps}
        goBack={goBack}
      >
        <div className="flex h-full flex-col justify-between gap-6 py-4">
          <div className="space-y-6">
            <div className="space-y-3">
              <ChoiceCard
                active={form.taxCountry === "at"}
                title="Austria"
                description="EUR defaults, section 27b buckets, and moving average semantics through rp2's AT plugin."
                onClick={() => {
                  update("taxCountry", "at");
                  update("fiatCurrency", "EUR");
                }}
              />
              <ChoiceCard
                active={form.taxCountry === "generic"}
                title="Generic"
                description="Country-neutral FIFO/LIFO/HIFO/LOFO profile for non-Austrian workflows."
                onClick={() => update("taxCountry", "generic")}
              />
            </div>

            <div className="grid gap-4 sm:grid-cols-2">
              <SelectField
                label="Fiat currency"
                value={form.fiatCurrency}
                options={FIAT_CURRENCIES}
                onChange={(value) => update("fiatCurrency", value)}
              />
              <SelectField
                label="Lot selection"
                value={form.gainsAlgorithm}
                options={GAINS_ALGORITHMS}
                onChange={(value) => update("gainsAlgorithm", value)}
              />
            </div>
            <TextField
              label="Long-term holding days"
              name="taxLongTermDays"
              value={form.taxLongTermDays}
              placeholder="365"
              onChange={(value) => update("taxLongTermDays", value)}
            />
          </div>

          <Button onClick={onSubmit} className="w-full">
            Continue
          </Button>
        </div>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <DashboardIllustration
          form={form}
          variant="zoomed-in"
          transformOrigin="10% 10%"
        />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};

const StepConnectionsComponent = ({
  form,
  update,
  currentStep,
  totalSteps,
  onSubmit,
  goBack,
}: StepComponentProps) => {
  const skipSelected = form.backendSetupMode === "skip";
  const customSelected = form.backendSetupMode === "custom";
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Choose sync connections"
        eyebrow="Connections"
        currentStep={currentStep}
        totalSteps={totalSteps}
        goBack={goBack}
      >
        <div className="flex h-full flex-col justify-between gap-6 py-4">
          <div className="space-y-5">
            <div className="space-y-3">
              <ChoiceCard
                active={form.backendSetupMode === "default"}
                title="Use built-in backends"
                description="Start with Kassiber's bundled Bitcoin, Electrum, and Liquid endpoints. You can replace them later."
                onClick={() => {
                  update("backendSetupMode", "default");
                  update("backendKind", "esplora");
                  update("backendName", DEFAULT_BACKEND_NAME);
                  update("backendUrl", DEFAULT_BACKEND_URL);
                }}
              />
              <ChoiceCard
                active={customSelected}
                title="Use a custom backend"
                description="Point onboarding at your own Esplora, Electrum, Bitcoin Core RPC, BTCPay, Liquid Esplora, or custom endpoint."
                onClick={() => {
                  update("backendSetupMode", "custom");
                  if (
                    form.backendName === DEFAULT_BACKEND_NAME &&
                    form.backendUrl === DEFAULT_BACKEND_URL
                  ) {
                    update("backendName", "");
                    update("backendUrl", "");
                  }
                }}
              />
              <ChoiceCard
                active={skipSelected}
                title="Skip connections for now"
                description="Continue with manual imports only. Wallet sync can be configured from Settings later."
                tone="warning"
                onClick={() => update("backendSetupMode", "skip")}
              />
            </div>

            {customSelected && (
              <div className="space-y-4 rounded-lg border border-line bg-paper-2 p-4">
                <SelectField
                  label="Backend kind"
                  value={form.backendKind}
                  options={BACKEND_KINDS}
                  onChange={(value) => update("backendKind", value)}
                />
                <TextField
                  label="Display name"
                  name="backendName"
                  value={form.backendName}
                  placeholder="home-node"
                  onChange={(value) => update("backendName", value)}
                />
                <TextField
                  label="Endpoint URL"
                  name="backendUrl"
                  value={form.backendUrl}
                  placeholder="https://... or ssl://..."
                  onChange={(value) => update("backendUrl", value)}
                />
                <div className="flex items-start gap-3 rounded-lg border border-line bg-paper p-3 text-xs leading-5 text-ink-2">
                  <KeyRound className="mt-0.5 size-4 shrink-0 text-ink" />
                  <p className="m-0">
                    Do not paste API tokens, RPC passwords, cookies, or bearer
                    headers here. Credentials should be added only after the
                    encrypted database is open.
                  </p>
                </div>
              </div>
            )}

            {skipSelected && (
              <div className="space-y-3 rounded-lg border border-accent bg-[rgba(227,0,15,0.04)] p-4">
                <div className="flex items-start gap-3">
                  <AlertTriangle className="mt-0.5 size-5 shrink-0 text-accent" />
                  <div>
                    <p className="m-0 font-semibold text-ink">
                      Wallet sync will not be ready.
                    </p>
                    <p className="m-0 mt-1 text-xs leading-5 text-ink-2">
                      You can still import files, but address discovery,
                      BTCPay live history, and node-backed sync remain disabled
                      until a backend is configured.
                    </p>
                  </div>
                </div>
                <CheckRow
                  id="skip-backends-ack"
                  checked={form.skipBackendsAcknowledged}
                  onCheckedChange={(checked) =>
                    update("skipBackendsAcknowledged", checked)
                  }
                  label="I understand sync needs a backend later."
                  description="Settings can add built-in or custom backends after onboarding."
                />
              </div>
            )}
          </div>

          <Button onClick={onSubmit} className="w-full">
            Continue
          </Button>
        </div>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <ConnectionsPanel form={form} />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};

const StepThreeComponent = ({
  form,
  update,
  currentStep,
  totalSteps,
  onSubmit,
  goBack,
}: StepComponentProps) => {
  const encrypted = form.databaseMode === "sqlcipher";
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Protect the database"
        eyebrow="Database"
        currentStep={currentStep}
        totalSteps={totalSteps}
        goBack={goBack}
      >
        <div className="flex h-full flex-col justify-between gap-6 py-4">
          <div className="space-y-5">
            <div className="space-y-3">
              <ChoiceCard
                active={encrypted}
                title="SQLCipher database"
                description="Recommended for real books. The local SQLite file is encrypted at rest with a passphrase."
                onClick={() => update("databaseMode", "sqlcipher")}
              />
              <ChoiceCard
                active={form.databaseMode === "plaintext"}
                title="Plaintext preview"
                description="For mock data or throwaway evaluation only. The database remains readable on disk."
                tone="warning"
                onClick={() => update("databaseMode", "plaintext")}
              />
            </div>

            {encrypted ? (
              <div className="space-y-3">
                <div className="rounded-lg border border-line bg-paper-2 p-3">
                  <div className="flex items-center gap-2 text-sm font-semibold text-ink">
                    <KeyRound className="size-4" />
                    Passphrase capture
                  </div>
                  <p className="m-0 mt-2 text-xs leading-5 text-ink-2">
                    The passphrase belongs in the native sidecar fd handoff,
                    not persisted webview state. This step records the
                    SQLCipher setup intent only.
                  </p>
                </div>
                <CheckRow
                  id="recovery-ack"
                  checked={form.recoveryAcknowledged}
                  onCheckedChange={(checked) =>
                    update("recoveryAcknowledged", checked)
                  }
                  label="I understand there is no passphrase recovery path."
                  description="If the passphrase is lost, the SQLCipher database cannot be opened."
                />
                <CheckRow
                  id="migrate-credentials"
                  checked={form.migrateCredentials}
                  onCheckedChange={(checked) =>
                    update("migrateCredentials", checked)
                  }
                  label="Move existing backend credentials into the encrypted DB."
                  description="Tokens, RPC passwords, auth headers, and usernames should not remain in backends.env."
                />
              </div>
            ) : (
              <CheckRow
                id="plaintext-ack"
                checked={form.plaintextAcknowledged}
                onCheckedChange={(checked) =>
                  update("plaintextAcknowledged", checked)
                }
                label="I understand plaintext mode is not for real wallet data."
                description="Balances, addresses, tags, and backend metadata are readable by anything with disk access."
              />
            )}
          </div>

          <Button onClick={onSubmit} className="w-full">
            Open ledger
          </Button>
        </div>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <DatabasePanel form={form} />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};

const ConnectionsPanel = ({ form }: { form: OnboardingForm }) => {
  const modeLabel =
    form.backendSetupMode === "default"
      ? "Built-in backends"
      : form.backendSetupMode === "custom"
        ? "Custom backend"
        : "Skipped";
  const activeRows =
    form.backendSetupMode === "default"
      ? PUBLIC_BACKEND_DEFAULTS
      : form.backendSetupMode === "custom"
        ? [
            [
              form.backendName.trim() || "custom",
              BACKEND_KIND_LABELS[form.backendKind],
              form.backendUrl.trim() || "endpoint pending",
            ],
          ]
        : [["None", "Manual import", "configure later"]];

  return (
    <div className="flex h-full items-center">
      <div className="w-full max-w-lg rounded-lg border border-line bg-paper p-5 shadow-sm">
        <div className="flex items-center gap-3">
          <div
            className={cn(
              "flex size-10 items-center justify-center rounded-md text-paper",
              form.backendSetupMode === "skip" ? "bg-accent" : "bg-ink",
            )}
          >
            {form.backendSetupMode === "skip" ? (
              <AlertTriangle className="size-5" />
            ) : form.backendSetupMode === "custom" ? (
              <ServerCog className="size-5" />
            ) : (
              <Globe2 className="size-5" />
            )}
          </div>
          <div>
            <p className="font-semibold text-ink">{modeLabel}</p>
            <p className="text-xs text-ink-2">
              {form.backendSetupMode === "skip"
                ? "No live sync until settings are configured."
                : "Endpoint choices only; credentials stay out of onboarding."}
            </p>
          </div>
        </div>

        <div className="mt-5 overflow-hidden rounded-lg border border-line">
          <Table>
            <TableHeader>
              <TableRow className="bg-paper-2">
                {["Name", "Kind", "Endpoint"].map((head) => (
                  <TableHead key={head} className="h-9 border-r last:border-r-0">
                    {head}
                  </TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {activeRows.map(([name, kind, url]) => (
                <TableRow key={name} className="even:bg-paper-2/60">
                  <TableCell className="h-10 border-r font-medium">
                    {name}
                  </TableCell>
                  <TableCell className="h-10 border-r">{kind}</TableCell>
                  <TableCell className="h-10 max-w-[240px] truncate">
                    {url}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      </div>
    </div>
  );
};

const DatabasePanel = ({ form }: { form: OnboardingForm }) => {
  const rows = [
    ["State root", "~/.kassiber/{data,config,exports,attachments}"],
    ["Database", form.databaseMode === "sqlcipher" ? "SQLCipher 4" : "Plain SQLite"],
    ["KDF", "SQLCipher stock kdf_iter 256000"],
    ["Backup", "tar | age bundle"],
    ["Outside perimeter", "attachments, exports, dotenv addressing rows"],
  ];
  return (
    <div className="flex h-full items-center">
      <div className="w-full max-w-lg rounded-lg border border-line bg-paper p-5 shadow-sm">
        <div className="flex items-center gap-3">
          <div className="flex size-10 items-center justify-center rounded-md bg-ink text-paper">
            {form.databaseMode === "sqlcipher" ? (
              <LockKeyhole className="size-5" />
            ) : (
              <Database className="size-5" />
            )}
          </div>
          <div>
            <p className="font-semibold text-ink">
              {form.databaseMode === "sqlcipher"
                ? "Encrypted at rest"
                : "Plaintext preview"}
            </p>
            <p className="text-xs text-ink-2">
              Watch-only wallet data stays local.
            </p>
          </div>
        </div>
        <dl className="mt-5 space-y-3">
          {rows.map(([label, value]) => (
            <div
              key={label}
              className="flex items-start justify-between gap-4 border-b border-line pb-2 last:border-b-0"
            >
              <dt className="font-mono text-[10px] uppercase tracking-[0.12em] text-ink-3">
                {label}
              </dt>
              <dd className="m-0 max-w-[280px] text-right text-xs font-medium leading-5 text-ink">
                {value}
              </dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  );
};

interface Onboarding1Props {
  className?: string;
  steps?: OnboardingStep[];
}

const steps: OnboardingStep[] = [
  {
    component: StepOneComponent,
    isComplete: (form) =>
      Boolean(form.name.trim() && form.workspace.trim() && form.profile.trim()),
  },
  {
    component: StepTwoComponent,
    isComplete: (form) => {
      const days = Number.parseInt(form.taxLongTermDays, 10);
      return Number.isFinite(days) && days > 0;
    },
  },
  {
    component: StepConnectionsComponent,
    isComplete: (form) => {
      if (form.backendSetupMode === "skip") {
        return form.skipBackendsAcknowledged;
      }
      if (form.backendSetupMode === "custom") {
        return Boolean(form.backendName.trim() && form.backendUrl.trim());
      }
      return true;
    },
  },
  {
    component: StepThreeComponent,
    isComplete: (form) =>
      form.databaseMode === "plaintext"
        ? form.plaintextAcknowledged
        : form.recoveryAcknowledged,
  },
];

const Onboarding1 = ({ className, steps: customSteps }: Onboarding1Props) => {
  const navigate = useNavigate();
  const setIdentity = useUiStore((state) => state.setIdentity);
  const [currentStep, setCurrentStep] = useState(0);
  const [form, setForm] = useState<OnboardingForm>(DEFAULT_FORM);
  const activeSteps = customSteps ?? steps;
  const step = activeSteps[currentStep];

  const update = <K extends keyof OnboardingForm>(
    key: K,
    value: OnboardingForm[K],
  ) => {
    setForm((current) => ({ ...current, [key]: value }));
  };

  const finish = () => {
    const identity: Identity = {
      name: form.name.trim(),
      workspace: form.workspace.trim() || "Personal",
      country: form.taxCountry === "at" ? "AT" : "Generic",
      encrypted: form.databaseMode === "sqlcipher",
      profile: form.profile.trim() || "main",
      taxCountry: form.taxCountry,
      fiatCurrency: form.fiatCurrency,
      taxLongTermDays: Number.parseInt(form.taxLongTermDays, 10) || 365,
      gainsAlgorithm: form.gainsAlgorithm,
      databaseMode: form.databaseMode,
      migrateCredentials: form.migrateCredentials,
      backendSetupMode: form.backendSetupMode,
      backendKind: form.backendSetupMode === "custom" ? form.backendKind : undefined,
      backendName:
        form.backendSetupMode === "custom"
          ? form.backendName.trim() || "custom"
          : undefined,
      backendUrl:
        form.backendSetupMode === "custom"
          ? form.backendUrl.trim()
          : undefined,
    };
    setIdentity(identity);
    void navigate({ to: "/overview" });
  };

  const handleSubmit = () => {
    if (!step.isComplete(form)) return;
    if (currentStep !== activeSteps.length - 1) {
      setCurrentStep(currentStep + 1);
      return;
    }
    finish();
  };

  const handleGoBack = () => {
    if (currentStep > 0) setCurrentStep(currentStep - 1);
  };

  return (
    <section className="min-h-screen bg-paper px-4 py-6 text-ink sm:px-8 lg:px-10">
      <div className={cn("mx-auto flex max-w-7xl flex-col items-center gap-8", className)}>
        <div className="flex w-full items-center justify-between gap-4">
          <Wordmark size={22} />
          <div className="hidden items-center gap-2 text-xs text-ink-2 sm:flex">
            <ShieldCheck className="size-4" />
            Local-first - watch-only - SQLCipher-aware
          </div>
        </div>

        <step.component
          form={form}
          update={update}
          onSubmit={handleSubmit}
          currentStep={currentStep}
          totalSteps={activeSteps.length}
          goBack={handleGoBack}
        />

        <div className="flex flex-wrap items-center justify-center gap-4 text-xs text-ink-3">
          <p>Private keys never enter Kassiber.</p>
          <span>State stays under ~/.kassiber unless overridden.</span>
          <span>Run backups before tracking real funds.</span>
        </div>
      </div>
    </section>
  );
};

export { Onboarding1 };
