/**
 * AppHeader — top navigation, workspace label, hide-sensitive toggle,
 * and a small overflow menu for currency/language/settings/lock.
 *
 * Translated from claude-design/components/chrome.jsx. The lang and
 * currency toggles wire into the existing Zustand UI store; settings
 * and lock are stubs until those flows land.
 */

import { useEffect, useRef, useState } from "react";
import { Link, useRouterState } from "@tanstack/react-router";
import { Eye, EyeOff, MoreHorizontal, Settings, Lock } from "lucide-react";

import { Wordmark } from "./Wordmark";
import { SettingsModal } from "./SettingsModal";
import { useUiStore } from "@/store/ui";
import { cn } from "@/lib/utils";

const NAV_ITEMS = [
  { to: "/overview", label: "Overview" },
  { to: "/transactions", label: "Transactions" },
  { to: "/reports", label: "Reports" },
] as const;

export function AppHeader() {
  const identity = useUiStore((s) => s.identity);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);

  const routerState = useRouterState();
  const path = routerState.location.pathname;

  return (
    <header className="flex h-13 flex-shrink-0 items-center gap-7 border-b border-ink bg-paper px-4.5">
      <Wordmark size={20} />
      <div className="h-5 w-px bg-line" />

      <nav className="flex flex-1 gap-1">
        {NAV_ITEMS.map((item) => {
          const active = path === item.to || path.startsWith(`${item.to}/`);
          return (
            <Link
              key={item.to}
              to={item.to}
              className={cn(
                "relative px-3 py-1.5 font-sans text-xs font-medium tracking-[0.02em] no-underline",
                active ? "text-ink" : "text-ink-3 hover:text-ink-2",
              )}
            >
              {item.label}
              {active && (
                <span className="absolute -bottom-[17px] left-0 right-0 h-0.5 bg-accent" />
              )}
            </Link>
          );
        })}
      </nav>

      <div className="flex items-center gap-2">
        {identity && (
          <button
            title={`Workspace · ${identity.workspace} · signed in as ${identity.name}`}
            className="flex h-6.5 cursor-pointer items-center gap-1.5 border border-line bg-transparent px-2.5"
          >
            <span className="size-1 bg-accent" />
            <span className="font-mono text-[11px] font-medium text-ink">
              {identity.workspace}
            </span>
            <svg
              width="7"
              height="7"
              viewBox="0 0 10 10"
              className="ml-px"
            >
              <path
                d="M2 4 L5 7 L8 4"
                stroke="var(--color-ink-3)"
                strokeWidth="1.2"
                fill="none"
              />
            </svg>
          </button>
        )}

        <button
          onClick={() => setHideSensitive(!hideSensitive)}
          title={hideSensitive ? "Show sensitive data" : "Hide sensitive data"}
          className={cn(
            "flex size-6.5 cursor-pointer items-center justify-center border",
            hideSensitive ? "border-ink bg-ink" : "border-line bg-transparent",
          )}
        >
          {hideSensitive ? (
            <EyeOff className="size-3.5 text-paper" />
          ) : (
            <Eye className="size-3.5 text-ink-2" />
          )}
        </button>

        <OverflowMenu />
      </div>
    </header>
  );
}

function OverflowMenu() {
  const [open, setOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const lang = useUiStore((s) => s.lang);
  const setLang = useUiStore((s) => s.setLang);
  const currency = useUiStore((s) => s.currency);
  const setCurrency = useUiStore((s) => s.setCurrency);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <SettingsModal
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
      />
      <button
        onClick={() => setOpen((v) => !v)}
        title="More"
        aria-label="More options"
        className={cn(
          "flex size-6.5 cursor-pointer items-center justify-center border",
          open ? "border-ink bg-ink" : "border-line bg-transparent",
        )}
      >
        <MoreHorizontal
          className={cn("size-3.5", open ? "text-paper" : "text-ink-2")}
        />
      </button>

      {open && (
        <div className="absolute right-0 top-8 z-50 flex w-[230px] flex-col gap-2.5 border border-ink bg-paper p-2.5 shadow-hard-ink">
          <MenuRow label="Display">
            <PillToggle
              options={[
                { value: "btc", label: "₿" },
                { value: "eur", label: "€" },
              ]}
              value={currency}
              onChange={(v) => setCurrency(v as "btc" | "eur")}
            />
          </MenuRow>
          <MenuRow label="Language">
            <PillToggle
              options={[
                { value: "en", label: "EN" },
                { value: "de", label: "DE" },
              ]}
              value={lang}
              onChange={(v) => setLang(v as "en" | "de")}
            />
          </MenuRow>

          <div className="-mx-2.5 h-px bg-line" />

          <MenuButton
            onClick={() => {
              setOpen(false);
              setSettingsOpen(true);
            }}
            icon={<Settings className="size-3.5 text-ink-2" />}
          >
            Settings
          </MenuButton>
          <MenuButton
            onClick={() => setOpen(false)}
            icon={<Lock className="size-3.5 text-ink-2" />}
            shortcut="⌘L"
          >
            Lock Kassiber
          </MenuButton>
        </div>
      )}
    </div>
  );
}

function MenuRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-2.5">
      <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-ink-3">
        {label}
      </span>
      {children}
    </div>
  );
}

interface PillToggleProps {
  options: Array<{ value: string; label: string }>;
  value: string;
  onChange: (value: string) => void;
}

function PillToggle({ options, value, onChange }: PillToggleProps) {
  return (
    <div className="flex border border-line">
      {options.map((opt) => {
        const active = value === opt.value;
        return (
          <button
            key={opt.value}
            onClick={() => onChange(opt.value)}
            className={cn(
              "h-5.5 w-6.5 cursor-pointer border-none p-0 font-mono font-semibold tracking-[0.08em]",
              opt.label.length === 1 ? "text-xs" : "text-[10px]",
              active ? "bg-ink text-paper" : "bg-transparent text-ink-2",
            )}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}

interface MenuButtonProps {
  onClick: () => void;
  icon: React.ReactNode;
  shortcut?: string;
  children: React.ReactNode;
}

function MenuButton({ onClick, icon, shortcut, children }: MenuButtonProps) {
  return (
    <button
      onClick={onClick}
      className="flex w-full cursor-pointer items-center gap-2.5 border-none bg-transparent px-1 py-1.5 text-left font-sans text-xs text-ink"
    >
      {icon}
      {children}
      {shortcut && (
        <span className="ml-auto font-mono text-[9px] tracking-[0.08em] text-ink-3">
          {shortcut}
        </span>
      )}
    </button>
  );
}
