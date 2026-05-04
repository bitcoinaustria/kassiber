/**
 * BookSwitcherPopover — compact books switcher anchored to the
 * books pill in AppHeader.
 *
 * Shares the kassiber-themed Dialog shell used by action confirmations but with
 * a denser visual layout: each books set gets a compact header with a jurisdiction
 * chip and a 2-column grid of book cards. Clicking a book updates the active
 * daemon context and closes the popover.
 */
import { useEffect, useState } from "react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { cn } from "@/lib/utils";
import type { ProfilesSnapshot, Profile, Workspace } from "@/mocks/profiles";

interface BookSwitcherPopoverProps {
  open: boolean;
  onClose: () => void;
}

export function BookSwitcherPopover({
  open,
  onClose,
}: BookSwitcherPopoverProps) {
  const { data, isLoading } = useDaemon<ProfilesSnapshot>(
    "ui.profiles.snapshot",
  );

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent
        className={cn(
          "max-h-[80vh] w-full max-w-[520px] gap-0 overflow-y-auto",
          "rounded-none border-ink bg-paper p-0 shadow-hard-ink",
          "data-[state=open]:zoom-in-100 data-[state=closed]:zoom-out-100",
        )}
      >
        <DialogHeader className="flex-row items-center justify-between gap-2 border-b border-line px-4 py-3">
          <DialogTitle className="font-sans text-sm font-semibold tracking-[-0.005em] text-ink">
            Switch books
          </DialogTitle>
          <DialogDescription className="sr-only">
            Pick a books set and one book to make active.
          </DialogDescription>
        </DialogHeader>

        {isLoading || !data?.data ? (
          <div className="flex items-center justify-center px-4 py-10 font-mono text-xs text-ink-3">
            loading…
          </div>
        ) : (
          <SwitcherBody snapshot={data.data} onClose={onClose} />
        )}
      </DialogContent>
    </Dialog>
  );
}

interface SwitcherBodyProps {
  snapshot: ProfilesSnapshot;
  onClose: () => void;
}

function SwitcherBody({ snapshot, onClose }: SwitcherBodyProps) {
  const switchProfile = useDaemonMutation<{ activeProfileId: string }>(
    "ui.profiles.switch",
  );
  const [activeId, setActiveId] = useState(snapshot.activeProfileId);

  // Keep the local active id in sync if the underlying snapshot changes
  // while the popover is mounted (e.g. another tab updated identity).
  useEffect(() => {
    setActiveId(snapshot.activeProfileId);
  }, [snapshot.activeProfileId]);

  const handlePick = (profile: Profile) => {
    if (profile.id === activeId) {
      onClose();
      return;
    }
    switchProfile.mutate(
      { profile_id: profile.id },
      {
        onSuccess: () => {
          setActiveId(profile.id);
          onClose();
        },
      },
    );
  };

  return (
    <div className="flex flex-col gap-4 p-4">
      {snapshot.workspaces.map((ws) => (
        <WorkspaceBlock
          key={ws.id}
          workspace={ws}
          activeId={activeId}
          onPick={handlePick}
        />
      ))}
    </div>
  );
}

interface WorkspaceBlockProps {
  workspace: Workspace;
  activeId: string;
  onPick: (profile: Profile) => void;
}

function WorkspaceBlock({ workspace, activeId, onPick }: WorkspaceBlockProps) {
  return (
    <div>
      <div className="mb-2 flex items-baseline gap-2 border-b border-ink pb-1">
        <span className="font-sans text-[14px] tracking-[-0.005em] text-ink">
          {workspace.name}
        </span>
        <span className="border border-line px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-[0.1em] text-ink-2">
          {workspace.jurisdiction}
        </span>
        <span className="font-mono text-[9px] uppercase tracking-[0.08em] text-ink-3">
          {workspace.kind} · {workspace.currency}
        </span>
      </div>

      <div className="grid grid-cols-2 gap-2">
        {workspace.profiles.map((p) => (
          <ProfileCard
            key={p.id}
            profile={p}
            isActive={p.id === activeId}
            onPick={() => onPick(p)}
          />
        ))}
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
        "relative flex cursor-pointer flex-col gap-1.5 border px-3 py-2.5 text-left font-sans",
        isActive ? "border-ink bg-paper-2" : "border-line bg-paper",
      )}
    >
      {isActive && (
        <span className="absolute right-2 top-2 flex items-center gap-1 font-mono text-[8px] uppercase tracking-[0.14em] text-accent">
          <span className="size-1 rounded-full bg-accent" />
          Active
        </span>
      )}

      <div>
        <div className="font-sans text-[13px] tracking-[-0.005em] text-ink">
          {profile.name}
        </div>
        <div className="mt-0.5 font-mono text-[9px] uppercase tracking-[0.08em] text-ink-3">
          Opened {profile.lastOpened}
        </div>
      </div>

      <div className="flex gap-3 font-mono text-[10px] text-ink-2">
        <span>{profile.accounts} buckets</span>
        <span>{profile.wallets} wallets</span>
        <span className="flex-1" />
        <span
          className={cn(
            "text-[9px] uppercase tracking-[0.08em]",
            isActive ? "text-accent" : "text-ink-3",
          )}
        >
          {isActive ? "Current" : "Switch →"}
        </span>
      </div>
    </button>
  );
}
