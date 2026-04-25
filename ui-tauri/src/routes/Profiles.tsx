/**
 * Profiles / workspaces switcher screen.
 *
 * Visual translation of claude-design/screens/profiles.jsx. Lists each
 * workspace with its profiles laid out in a 2-column grid, surfaces tax
 * policy, accounts, and wallet counts per profile, and provides
 * "+ Profile" / "+ Workspace" affordances.
 *
 * Outstanding before this screen is feature-complete:
 *  - Wire the active profile id back to the daemon (currently local
 *    state seeded from the mock snapshot)
 *  - Real workspace creation flow + per-profile tax policy editor —
 *    deferred until corresponding daemon kinds land
 *  - ProfileSwitcherPopover (the compact popover variant from the source
 *    JSX) lands when AppHeader gets a workspace crumb to anchor it
 */

import { useState } from "react";
import { Plus } from "lucide-react";
import { useDaemon } from "@/daemon/client";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { ProfilesSnapshot, Profile, Workspace } from "@/mocks/profiles";

export function Profiles() {
  const { data, isLoading } = useDaemon<ProfilesSnapshot>(
    "ui.profiles.snapshot",
  );

  if (isLoading || !data?.data) {
    return (
      <div className="flex flex-1 items-center justify-center font-mono text-xs text-ink-3">
        loading…
      </div>
    );
  }

  return <ProfilesView snapshot={data.data} />;
}

function ProfilesView({ snapshot }: { snapshot: ProfilesSnapshot }) {
  const [activeId, setActiveId] = useState(snapshot.activeProfileId);
  const workspaces = snapshot.workspaces;
  const profileCount = workspaces.reduce((a, w) => a + w.profiles.length, 0);

  return (
    <div className="flex-1 overflow-auto bg-paper p-[22px]">
      <div className="mb-[22px] flex items-end justify-between">
        <div>
          <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-ink-3">
            Identity · {workspaces.length} workspaces · {profileCount} profiles
          </div>
          <h2 className="m-0 mt-1 font-sans text-[32px] font-semibold tracking-[-0.01em] text-ink">
            Switch profile
          </h2>
          <div className="mt-1.5 max-w-[640px] font-sans text-[12.5px] leading-[1.5] text-ink-2">
            Each profile keeps its own books, tax policy, accounts and wallets.
            Nothing is shared across profiles — switching reloads the ledger in
            read-only mode.
          </div>
        </div>
        <div className="flex gap-2">
          <Button
            variant="ghost"
            size="sm"
            className="rounded-none"
            onClick={() => {}}
          >
            ← Back
          </Button>
          <Button
            variant="secondary"
            size="sm"
            className="rounded-none"
            onClick={() => {}}
          >
            <Plus className="size-3" />
            Profile
          </Button>
          <Button size="sm" className="rounded-none" onClick={() => {}}>
            <Plus className="size-3" />
            Workspace
          </Button>
        </div>
      </div>

      <div className="flex flex-col gap-[22px]">
        {workspaces.map((ws) => (
          <ProfileWorkspaceBlock
            key={ws.id}
            workspace={ws}
            activeId={activeId}
            onPick={(p) => setActiveId(p.id)}
            onNewProfile={() => {}}
          />
        ))}

        <button
          onClick={() => {}}
          className="flex cursor-pointer items-center gap-3.5 border border-dashed border-line-2 bg-transparent px-5 py-[18px] font-sans"
        >
          <div className="font-sans text-[26px] leading-none text-ink-2">+</div>
          <div className="text-left">
            <div className="font-sans text-base text-ink">New workspace</div>
            <div className="font-sans text-xs text-ink-3">
              Separate books · separate tax policy · separate backups
            </div>
          </div>
        </button>
      </div>
    </div>
  );
}

interface ProfileWorkspaceBlockProps {
  workspace: Workspace;
  activeId: string;
  onPick: (profile: Profile) => void;
  onNewProfile: () => void;
}

function ProfileWorkspaceBlock({
  workspace,
  activeId,
  onPick,
  onNewProfile,
}: ProfileWorkspaceBlockProps) {
  const profileCount = workspace.profiles.length;
  return (
    <div>
      <div className="mb-2.5 flex items-baseline gap-3 border-b border-ink pb-1.5">
        <div className="flex items-center gap-2">
          <svg
            width="12"
            height="12"
            viewBox="0 0 12 12"
            aria-hidden="true"
          >
            <rect
              x="1"
              y="1"
              width="10"
              height="10"
              stroke="var(--color-ink)"
              fill="none"
              strokeWidth="1"
            />
          </svg>
          <span className="font-sans text-[19px] tracking-[-0.005em] text-ink">
            {workspace.name}
          </span>
        </div>
        <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-ink-3">
          {workspace.kind} · {workspace.currency} · {workspace.jurisdiction} ·
          since {workspace.created}
        </span>
        <span className="flex-1" />
        <span className="font-mono text-[10px] text-ink-3">
          {profileCount} profile{profileCount === 1 ? "" : "s"}
        </span>
      </div>

      <div className="grid grid-cols-2 gap-2.5">
        {workspace.profiles.map((p) => (
          <ProfileCard
            key={p.id}
            profile={p}
            isActive={p.id === activeId}
            onPick={() => onPick(p)}
          />
        ))}

        <button
          onClick={onNewProfile}
          className="flex min-h-[150px] cursor-pointer flex-col items-center justify-center gap-1.5 border border-dashed border-line-2 bg-transparent px-4 py-3.5 font-sans text-ink-3"
        >
          <div className="font-sans text-[22px] text-ink-2">+</div>
          <div className="font-sans text-xs text-ink-2">
            New profile in {workspace.name}
          </div>
          <div className="font-mono text-[9px] uppercase tracking-[0.12em]">
            Inherit tax defaults
          </div>
        </button>
      </div>
    </div>
  );
}

interface ProfileCardProps {
  profile: Profile;
  isActive: boolean;
  onPick: () => void;
}

function ProfileCard({ profile, isActive, onPick }: ProfileCardProps) {
  return (
    <button
      onClick={onPick}
      className={cn(
        "relative flex cursor-pointer flex-col gap-2.5 border px-4 py-3.5 text-left font-sans",
        isActive ? "border-ink bg-paper-2" : "border-line bg-paper",
      )}
    >
      {isActive && (
        <span className="absolute right-3 top-2.5 flex items-center gap-1.5 font-mono text-[9px] uppercase tracking-[0.14em] text-accent">
          <span className="size-1.5 rounded-full bg-accent" />
          Active
        </span>
      )}

      <div>
        <div className="font-sans text-[17px] tracking-[-0.005em] text-ink">
          {profile.name}
        </div>
        <div className="mt-0.5 font-mono text-[10px] uppercase tracking-[0.08em] text-ink-3">
          {profile.role} · opened {profile.lastOpened}
        </div>
      </div>

      <div className="border-l-2 border-accent pl-2.5 font-sans text-xs leading-[1.4] text-ink-2">
        <div className="mb-0.5 font-mono text-[9px] uppercase tracking-[0.12em] text-ink-3">
          Tax policy
        </div>
        {profile.taxPolicy}
      </div>

      <div className="flex gap-[18px] font-mono text-[11px] text-ink">
        <span>
          <span className="mb-0.5 block text-[9px] uppercase tracking-[0.12em] text-ink-3">
            Accounts
          </span>
          {profile.accounts}
        </span>
        <span>
          <span className="mb-0.5 block text-[9px] uppercase tracking-[0.12em] text-ink-3">
            Wallets
          </span>
          {profile.wallets}
        </span>
        <span className="flex-1" />
        <span
          className={cn(
            "self-end text-[10px] uppercase tracking-[0.08em]",
            isActive ? "text-accent" : "text-ink-3",
          )}
        >
          {isActive ? "Current →" : "Open →"}
        </span>
      </div>
    </button>
  );
}
