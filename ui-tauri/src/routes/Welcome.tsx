/**
 * Welcome flow — three-step onboarding.
 *
 * Visual translation of claude-design/screens/welcome.jsx. The step state
 * machine and form logic are preserved verbatim; inline styles become
 * Tailwind classes against the theme tokens in styles/globals.css.
 *
 * The encryption step exposes UX for at-rest encryption that the Python
 * core does not yet implement. Wire-up to a real daemon kind happens
 * once the corresponding command lands; for now the choice is captured
 * locally on the identity record.
 */

import { useState, type ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";

import { Button } from "@/components/ui/button";
import { LabeledInput } from "@/components/kb/LabeledInput";
import { Wordmark } from "@/components/kb/Wordmark";
import { useUiStore, type Identity } from "@/store/ui";
import { cn } from "@/lib/utils";

const COUNTRIES = ["AT", "DE", "CH", "IT", "NL", "Other"] as const;

type Step = "intro" | "setup" | "encrypt";

const SETUP_STEPS = [
  { id: "setup", label: "Identity", n: "01" },
  { id: "encrypt", label: "Encryption", n: "02" },
] as const;

export function Welcome() {
  const navigate = useNavigate();
  const setIdentity = useUiStore((s) => s.setIdentity);
  const [step, setStep] = useState<Step>("intro");
  const [partial, setPartial] = useState<{
    name: string;
    workspace: string;
    country: string;
  } | null>(null);

  const finish = (
    name: string,
    workspace: string,
    country: string,
    encrypted: boolean,
  ) => {
    const identity: Identity = { name, workspace, country, encrypted };
    setIdentity(identity);
    void navigate({ to: "/overview" });
  };

  if (step === "intro") {
    return <WelcomeIntro onNext={() => setStep("setup")} />;
  }

  if (step === "setup") {
    return (
      <WelcomeSetup
        initial={partial}
        onBack={() => setStep("intro")}
        onNext={(id) => {
          setPartial(id);
          setStep("encrypt");
        }}
      />
    );
  }

  return (
    <WelcomeEncrypt
      onBack={() => setStep("setup")}
      onFinish={(encrypted) =>
        partial &&
        finish(partial.name, partial.workspace, partial.country, encrypted)
      }
    />
  );
}

function WelcomeShell({
  step,
  children,
}: {
  step: Step;
  children: ReactNode;
}) {
  return (
    <div className="relative flex min-h-screen flex-1 flex-col overflow-hidden bg-paper">
      <div className="flex items-center justify-center gap-2.5 border-b border-ink bg-paper-2 px-7 py-2.5 font-mono text-[10px] uppercase tracking-[0.18em] text-ink-2">
        <span
          className="inline-block size-1.5 rounded-full bg-accent"
          style={{ boxShadow: "0 0 0 3px rgba(166,47,47,0.12)" }}
        />
        <span>Watch-only</span>
        <span className="text-ink-3">·</span>
        <span>This app never touches your private keys.</span>
      </div>

      {step !== "intro" && (
        <div className="flex items-center gap-[18px] border-b border-line px-7 py-2.5 font-mono text-[10px] uppercase tracking-[0.14em]">
          {SETUP_STEPS.map((s, i, arr) => {
            const active = s.id === step;
            const done = arr.findIndex((x) => x.id === step) > i;
            return (
              <span
                key={s.id}
                className={cn(
                  "flex items-center gap-2",
                  active && "text-ink",
                  done && !active && "text-ink-2",
                  !active && !done && "text-ink-3",
                )}
              >
                <span
                  className={cn(
                    "flex size-[18px] items-center justify-center border text-[9px] font-bold",
                    active && "border-accent text-accent",
                    done && !active && "border-ink-2",
                    !active && !done && "border-ink-3",
                  )}
                >
                  {done ? "✓" : s.n}
                </span>
                <span>{s.label}</span>
                {i < arr.length - 1 && (
                  <span className="ml-[18px] text-ink-3">·</span>
                )}
              </span>
            );
          })}
          <span className="ml-auto text-ink-3">Setup</span>
        </div>
      )}

      <div className="flex min-h-0 flex-1">{children}</div>
    </div>
  );
}

function WelcomeIntro({ onNext }: { onNext: () => void }) {
  const facts: Array<[string, string]> = [
    ["Local", "Plain files on your disk. Export anytime."],
    ["Watch-only", "xpubs & read-keys. Never private keys."],
    ["Jurisdictions", "Presets for AT · DE · CH · more coming."],
    ["Encrypted", "Optional at-rest passphrase."],
  ];
  return (
    <WelcomeShell step="intro">
      <div className="flex flex-1 items-center justify-center px-12 py-10">
        <div className="flex w-full max-w-[520px] flex-col">
          <Wordmark size={22} />
          <h1 className="mb-0 mt-7 font-sans text-[40px] font-semibold leading-[1.1] tracking-[-0.02em] text-ink">
            Your books. Your keys.
          </h1>
          <p className="mb-0 mt-3.5 max-w-[440px] font-sans text-sm leading-[1.55] text-ink-2">
            A Bitcoin-only ledger that runs locally. No cloud, no custodian,
            no account.
          </p>

          <dl className="m-0 mt-7 border-t border-line p-0">
            {facts.map(([k, v]) => (
              <div
                key={k}
                className="grid grid-cols-[140px_1fr] items-baseline border-b border-line py-2.5"
              >
                <dt className="font-mono text-[10px] uppercase tracking-[0.14em] text-ink-3">
                  {k}
                </dt>
                <dd className="m-0 font-sans text-[13px] text-ink-2">{v}</dd>
              </div>
            ))}
          </dl>

          <div className="mt-7 flex items-center gap-4">
            <Button size="lg" onClick={onNext} className="rounded-none">
              Continue
              <ArrowRight className="size-3.5" />
            </Button>
            <span className="font-mono text-[10px] uppercase tracking-[0.1em] text-ink-3">
              Two-minute setup
            </span>
          </div>
        </div>
      </div>
    </WelcomeShell>
  );
}

function WelcomeSetup({
  initial,
  onBack,
  onNext,
}: {
  initial: { name: string; workspace: string; country: string } | null;
  onBack: () => void;
  onNext: (id: { name: string; workspace: string; country: string }) => void;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  const [workspace, setWorkspace] = useState(initial?.workspace ?? "");
  const [country, setCountry] = useState(initial?.country ?? "AT");

  const submit = () => {
    if (!name.trim()) return;
    onNext({
      name: name.trim(),
      workspace: workspace.trim() || "My Books",
      country,
    });
  };

  const dirSlug = (workspace || "my-books").toLowerCase().replace(/\s+/g, "-");

  return (
    <WelcomeShell step="setup">
      <div className="grid flex-1 grid-cols-2">
        <div className="relative flex flex-col justify-between overflow-hidden border-r border-line bg-paper-2 px-14 py-12">
          <div>
            <div className="mb-3.5 font-mono text-[10px] uppercase tracking-[0.2em] text-accent">
              Step 01 of 02
            </div>
            <h2 className="m-0 font-sans text-[64px] font-semibold leading-[0.95] tracking-[-0.025em] text-ink">
              Tell us
              <br />
              who&apos;s writing.
            </h2>
            <p className="mt-5 max-w-[380px] font-sans text-sm leading-[1.55] text-ink-2">
              Your name and workspace live only on this device. The workspace
              becomes a folder of plain files — you can rename, move, or
              delete it at any time.
            </p>
          </div>
          <div className="mt-8 font-mono text-[10px] leading-[1.7] tracking-[0.12em] text-ink-3">
            <div>{"// Stored at"}</div>
            <div className="text-ink-2">~/.kassiber/{dirSlug}/</div>
          </div>
        </div>

        <div className="flex flex-col gap-6 px-14 py-12">
          <div className="flex flex-col gap-[18px]">
            <LabeledInput
              label="Your name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Alice"
            />
            <LabeledInput
              label="Workspace name"
              value={workspace}
              onChange={(e) => setWorkspace(e.target.value)}
              placeholder="My Books"
            />

            <div className="flex flex-col gap-2">
              <span className="font-sans text-[10px] font-semibold uppercase tracking-[0.12em] text-ink-2">
                Tax residency
              </span>
              <div className="flex flex-wrap gap-1.5">
                {COUNTRIES.map((c) => {
                  const active = country === c;
                  return (
                    <button
                      key={c}
                      type="button"
                      onClick={() => setCountry(c)}
                      className={cn(
                        "cursor-pointer border px-3.5 py-1.5 font-mono text-[11px] font-semibold uppercase tracking-[0.08em]",
                        active
                          ? "border-ink bg-ink text-paper"
                          : "border-line bg-transparent text-ink",
                      )}
                    >
                      {c}
                    </button>
                  );
                })}
              </div>
              <span className="font-sans text-[11px] italic text-ink-3">
                Jurisdiction presets load sensible defaults. You can customize
                everything later.
              </span>
            </div>
          </div>

          <div className="mt-auto flex items-center justify-between border-t border-ink pt-5">
            <button
              onClick={onBack}
              className="cursor-pointer border-none bg-transparent px-0 py-1 font-mono text-[11px] uppercase tracking-[0.12em] text-ink-2"
            >
              ← Back
            </button>
            <Button
              size="lg"
              onClick={submit}
              disabled={!name.trim()}
              className="rounded-none"
            >
              Continue
              <ArrowRight className="size-3.5" />
            </Button>
          </div>
        </div>
      </div>
    </WelcomeShell>
  );
}

interface PassphraseStrength {
  level: 0 | 1 | 2 | 3 | 4;
  label: string;
}

function scorePassphrase(pw: string): PassphraseStrength {
  if (!pw) return { level: 0, label: "— none —" };
  if (pw.length < 12) return { level: 0, label: "Too short" };
  let score = 1;
  if (pw.length >= 16) score++;
  if (pw.length >= 20) score++;
  if (/[a-z]/.test(pw) && /[A-Z]/.test(pw)) score++;
  if (/\d/.test(pw) && /[^A-Za-z0-9]/.test(pw)) score++;
  const labels = ["Too short", "Weak", "OK", "Strong", "Excellent"] as const;
  const level = Math.min(score, 4) as 0 | 1 | 2 | 3 | 4;
  return { level, label: labels[level] };
}

function WelcomeEncrypt({
  onBack,
  onFinish,
}: {
  onBack: () => void;
  onFinish: (encrypted: boolean) => void;
}) {
  const [mode, setMode] = useState<"encrypt" | "plain">("encrypt");
  const [passphrase, setPassphrase] = useState("");
  const [confirm, setConfirm] = useState("");
  const [showPw, setShowPw] = useState(false);

  const pwStrength = scorePassphrase(passphrase);
  const pwMatch = passphrase.length === 0 || passphrase === confirm;
  const canFinish =
    mode === "plain" || (passphrase.length >= 12 && pwMatch);

  const finish = () => {
    if (!canFinish) return;
    onFinish(mode === "encrypt");
  };

  return (
    <WelcomeShell step="encrypt">
      <div className="grid flex-1 grid-cols-2">
        <div className="relative flex flex-col justify-between overflow-hidden border-r border-line bg-paper-2 px-14 py-12">
          <div>
            <div className="mb-3.5 font-mono text-[10px] uppercase tracking-[0.2em] text-accent">
              Step 02 of 02
            </div>
            <h2 className="m-0 font-sans text-[64px] font-semibold leading-[0.95] tracking-[-0.025em] text-ink">
              Lock the
              <br />
              <span className="italic">door.</span>
            </h2>
            <p className="mt-5 max-w-[380px] font-sans text-sm leading-[1.55] text-ink-2">
              Kassiber can encrypt your database file at rest with a
              passphrase only you know. Anyone with your disk would see opaque
              ciphertext — not balances, not addresses, not tags.
            </p>
            <div className="mt-5 border border-line bg-paper p-3.5 font-sans text-xs leading-[1.55] text-ink-2">
              <div className="flex items-start gap-2">
                <span className="font-bold text-accent">⚠</span>
                <div>
                  <b className="text-ink">We can&apos;t recover it.</b>{" "}
                  Kassiber never sees your passphrase. If you lose it, the
                  encrypted workspace is unreadable — including by us. Write
                  it down.
                </div>
              </div>
            </div>
          </div>
          <div className="mt-6 font-mono text-[10px] leading-[1.8] tracking-[0.1em] text-ink-3">
            <div>{"// Cipher"}</div>
            <div className="text-ink-2">AES-256-GCM</div>
            <div className="mt-1.5">{"// Key derivation"}</div>
            <div className="text-ink-2">Argon2id · 256 MB · 3 passes</div>
          </div>
        </div>

        <div className="flex flex-col gap-5 px-14 py-12">
          <div className="grid grid-cols-2 gap-2.5">
            <ChoiceCard
              active={mode === "encrypt"}
              onClick={() => setMode("encrypt")}
              n="A"
              title="Encrypt"
              tagline="Recommended"
              desc="Passphrase required to open the workspace. Data at rest is unreadable without it."
            />
            <ChoiceCard
              active={mode === "plain"}
              onClick={() => setMode("plain")}
              n="B"
              title="Plain"
              tagline="Insecure · not recommended"
              warning
              desc="Debug / evaluation only. Your database is written in the clear — anyone with disk access can read every balance, address and tag."
            />
          </div>

          {mode === "encrypt" && (
            <div className="flex flex-col gap-3.5 border border-ink bg-paper-2 px-5 py-4.5">
              <div className="grid grid-cols-2 gap-2.5">
                <LabeledInput
                  label="Passphrase"
                  value={passphrase}
                  onChange={(e) => setPassphrase(e.target.value)}
                  placeholder="at least 12 characters"
                  type={showPw ? "text" : "password"}
                  mono
                />
                <LabeledInput
                  label="Confirm passphrase"
                  value={confirm}
                  onChange={(e) => setConfirm(e.target.value)}
                  placeholder="repeat"
                  type={showPw ? "text" : "password"}
                  mono
                />
              </div>

              <div className="flex items-center gap-2.5">
                <div className="flex h-1 flex-1 gap-px bg-line">
                  {[0, 1, 2, 3].map((i) => {
                    const active = i < pwStrength.level;
                    const color =
                      pwStrength.level >= 3
                        ? "bg-[#3fa66a]"
                        : pwStrength.level === 2
                          ? "bg-ink"
                          : "bg-accent";
                    return (
                      <div
                        key={i}
                        className={cn(
                          "flex-1",
                          active ? color : "bg-transparent",
                        )}
                      />
                    );
                  })}
                </div>
                <span
                  className={cn(
                    "min-w-[72px] text-right font-mono text-[10px] uppercase tracking-[0.14em]",
                    passphrase ? "text-ink-2" : "text-ink-3",
                  )}
                >
                  {passphrase ? pwStrength.label : "— none —"}
                </span>
                <button
                  type="button"
                  onClick={() => setShowPw((s) => !s)}
                  className="cursor-pointer border border-line bg-transparent px-2.5 py-1 font-mono text-[10px] uppercase tracking-[0.1em] text-ink-2"
                >
                  {showPw ? "Hide" : "Show"}
                </button>
              </div>

              {!pwMatch && (
                <div className="font-mono text-[10px] uppercase tracking-[0.08em] text-accent">
                  Passphrases don&apos;t match.
                </div>
              )}
              {passphrase && passphrase.length < 12 && (
                <div className="font-mono text-[10px] uppercase tracking-[0.08em] text-ink-3">
                  At least 12 characters required · {passphrase.length}/12
                </div>
              )}
            </div>
          )}

          {mode === "plain" && (
            <div className="flex items-start gap-2.5 border border-accent border-l-4 bg-[rgba(166,47,47,0.05)] px-4.5 py-3.5 font-sans text-xs leading-[1.55] text-ink-2">
              <span className="shrink-0 font-mono text-[11px] font-bold uppercase tracking-[0.1em] text-accent">
                ⚠ Insecure
              </span>
              <div>
                <b className="text-ink">Do not use this for real books.</b>{" "}
                Plain mode is intended for debugging and early evaluation only
                — your database is readable by anything with disk access.
                Switch to encrypted before tracking real funds via{" "}
                <b>Settings → App lock</b>.
              </div>
            </div>
          )}

          <div className="mt-auto flex items-center justify-between border-t border-ink pt-5">
            <button
              onClick={onBack}
              className="cursor-pointer border-none bg-transparent px-0 py-1 font-mono text-[11px] uppercase tracking-[0.12em] text-ink-2"
            >
              ← Back
            </button>
            <Button
              size="lg"
              onClick={finish}
              disabled={!canFinish}
              className="rounded-none"
            >
              Open ledger
              <ArrowRight className="size-3.5" />
            </Button>
          </div>
        </div>
      </div>
    </WelcomeShell>
  );
}

interface ChoiceCardProps {
  active: boolean;
  onClick: () => void;
  n: string;
  title: string;
  tagline: string;
  desc: string;
  warning?: boolean;
}

function ChoiceCard({
  active,
  onClick,
  n,
  title,
  tagline,
  desc,
  warning,
}: ChoiceCardProps) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "relative flex cursor-pointer flex-col gap-2 px-4 pb-4.5 pt-4 text-left outline-none transition-shadow border",
        active
          ? warning
            ? "border-accent bg-paper-2 shadow-[4px_4px_0_var(--color-accent)]"
            : "border-ink bg-paper-2 shadow-[4px_4px_0_var(--color-ink)]"
          : "border-line bg-transparent",
      )}
    >
      {warning && (
        <div
          aria-hidden="true"
          className="absolute right-0 top-0 bg-accent px-2 py-0.5 font-mono text-[9px] font-bold uppercase tracking-[0.16em] text-paper"
        >
          ⚠ Insecure
        </div>
      )}
      <div className="flex items-baseline gap-2.5">
        <span
          className={cn(
            "font-mono text-[11px] font-bold tracking-[0.12em]",
            active ? "text-accent" : "text-ink-3",
          )}
        >
          {n}
        </span>
        <span className="font-sans text-xl font-semibold tracking-[-0.01em] text-ink">
          {title}
        </span>
        {active && !warning && (
          <span className="ml-auto font-mono text-[9px] font-bold uppercase tracking-[0.14em] text-accent">
            ● selected
          </span>
        )}
      </div>
      <div
        className={cn(
          "font-mono text-[9px] uppercase tracking-[0.14em]",
          warning ? "font-bold text-accent" : "text-ink-3",
        )}
      >
        {tagline}
      </div>
      <div className="font-sans text-[12.5px] leading-[1.5] text-ink-2">
        {desc}
      </div>
    </button>
  );
}
