/**
 * The book switcher, anchored to the book crumb in the title bar.
 *
 * A picker, not a page: the Codex project picker's shape. A search field on
 * top, the books grouped under their book set, one row per book with its
 * wallets and tax policy, a check on the open one, and "Manage books" below.
 * Focus stays in the field; arrows move, Enter switches, Esc closes. The
 * books screen keeps the full cards for comparing and editing books.
 */
import { Link } from "@tanstack/react-router";
import { Check, Eye, Loader2, Search, Settings2 } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { cn } from "@/lib/utils";
import type { Profile, ProfilesSnapshot, Workspace } from "@/mocks/profiles";

interface BookSwitcherPopoverProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The crumb that opens the switcher; it becomes the popover's anchor. */
  children: React.ReactNode;
}

export function BookSwitcherPopover({
  open,
  onOpenChange,
  children,
}: BookSwitcherPopoverProps) {
  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent
        align="start"
        sideOffset={6}
        className="flex w-80 flex-col overflow-hidden p-0"
      >
        <SwitcherPanel onClose={() => onOpenChange(false)} />
      </PopoverContent>
    </Popover>
  );
}

interface BookEntry {
  workspace: Workspace;
  profile: Profile;
}

function matches(entry: BookEntry, query: string) {
  if (!query) return true;
  const haystack = [
    entry.profile.name,
    entry.profile.taxPolicy,
    entry.workspace.name,
  ]
    .join(" ")
    .toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .every((word) => haystack.includes(word));
}

function initials(name: string) {
  return (
    name
      .split(/\s+/)
      .map((part) => part[0])
      .join("")
      .slice(0, 2)
      .toUpperCase() || "?"
  );
}

function SwitcherPanel({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation("chrome");
  const listId = React.useId();
  const { data, error, isLoading } = useDaemon<ProfilesSnapshot>(
    "ui.profiles.snapshot",
  );
  const switchProfile = useDaemonMutation<{ activeProfileId: string }>(
    "ui.profiles.switch",
  );
  const [query, setQuery] = React.useState("");
  const [pendingId, setPendingId] = React.useState<string | null>(null);
  const snapshot = data?.data;
  const currentId = snapshot?.activeProfileId;

  const entries = React.useMemo<BookEntry[]>(
    () =>
      (snapshot?.workspaces ?? []).flatMap((workspace) =>
        workspace.profiles.map((profile) => ({ workspace, profile })),
      ),
    [snapshot],
  );
  const visible = React.useMemo(
    () => entries.filter((entry) => matches(entry, query.trim())),
    [entries, query],
  );
  // The open book is highlighted on arrival, so Enter alone closes the
  // switcher and a single arrow press reaches its neighbours.
  const [activeIndex, setActiveIndex] = React.useState(0);
  React.useEffect(() => {
    const current = visible.findIndex(
      (entry) => entry.profile.id === currentId,
    );
    setActiveIndex(query.trim() || current < 0 ? 0 : current);
  }, [visible, currentId, query]);

  const pick = (entry: BookEntry | undefined) => {
    if (!entry || switchProfile.isPending) return;
    if (entry.profile.id === currentId) {
      onClose();
      return;
    }
    setPendingId(entry.profile.id);
    switchProfile.mutate(
      { profile_id: entry.profile.id },
      {
        onSuccess: () => onClose(),
        onSettled: () => setPendingId(null),
      },
    );
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (visible.length === 0) return;
    const last = visible.length - 1;
    const moves: Record<string, number> = {
      ArrowDown: activeIndex >= last ? 0 : activeIndex + 1,
      ArrowUp: activeIndex <= 0 ? last : activeIndex - 1,
      Home: 0,
      End: last,
    };
    if (event.key in moves) {
      event.preventDefault();
      setActiveIndex(moves[event.key]);
    } else if (event.key === "Enter") {
      event.preventDefault();
      pick(visible[activeIndex]);
    }
  };

  React.useEffect(() => {
    document
      .getElementById(`${listId}-${activeIndex}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, listId]);

  // Rows keep their index into `visible`, so the groups below can render in
  // book-set order while the arrows walk one flat list.
  const groups = visible.reduce<
    Array<{ workspace: Workspace; rows: Array<BookEntry & { index: number }> }>
  >((acc, entry, index) => {
    const group = acc.find((item) => item.workspace.id === entry.workspace.id);
    if (group) group.rows.push({ ...entry, index });
    else acc.push({ workspace: entry.workspace, rows: [{ ...entry, index }] });
    return acc;
  }, []);

  const activeEntry = visible[activeIndex];

  return (
    <>
      <div className="flex items-center gap-2 border-b px-3">
        <Search
          className="size-4 shrink-0 text-muted-foreground"
          aria-hidden="true"
        />
        <input
          autoFocus
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={onKeyDown}
          placeholder={t("bookSwitcher.searchPlaceholder")}
          aria-label={t("bookSwitcher.title")}
          role="combobox"
          aria-expanded="true"
          aria-controls={listId}
          aria-activedescendant={
            activeEntry ? `${listId}-${activeIndex}` : undefined
          }
          className="h-10 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        />
      </div>

      <div
        id={listId}
        role="listbox"
        aria-label={t("bookSwitcher.title")}
        className="max-h-80 overflow-y-auto p-1"
      >
        {isLoading ? (
          <p className="flex items-center gap-2 px-2 py-3 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            {t("bookSwitcher.loading")}
          </p>
        ) : error ? (
          <p className="px-2 py-3 text-sm text-destructive">
            {error instanceof Error ? error.message : t("bookSwitcher.loadError")}
          </p>
        ) : entries.length === 0 ? (
          <p className="px-2 py-3 text-sm text-muted-foreground">
            {t("bookSwitcher.empty")}
          </p>
        ) : visible.length === 0 ? (
          <p className="px-2 py-3 text-sm text-muted-foreground">
            {t("bookSwitcher.noMatches")}
          </p>
        ) : (
          groups.map(({ workspace, rows }) => (
            <div key={workspace.id} role="group" aria-label={workspace.name}>
              <div className="flex h-7 items-center gap-2 pr-1 pl-2 text-xs font-medium text-muted-foreground">
                <span className="min-w-0 truncate">{workspace.name}</span>
                <span className="shrink-0 text-muted-foreground/70">
                  {workspace.currency}
                </span>
                <Link
                  to="/books/$workspaceId/birds-eye"
                  params={{ workspaceId: workspace.id }}
                  data-testid={`switcher-birds-eye-${workspace.id}`}
                  aria-label={t("bookSwitcher.bookSetOverview")}
                  title={t("bookSwitcher.bookSetOverview")}
                  onClick={onClose}
                  className="ml-auto inline-flex size-6 items-center justify-center rounded-md hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                >
                  <Eye className="size-3.5" aria-hidden="true" />
                </Link>
              </div>
              {rows.map(({ profile, index, ...entry }) => {
                const current = profile.id === currentId;
                return (
                  <div
                    key={profile.id}
                    id={`${listId}-${index}`}
                    role="option"
                    aria-selected={index === activeIndex}
                    aria-current={current ? "true" : undefined}
                    data-active={index === activeIndex}
                    onMouseMove={() => setActiveIndex(index)}
                    onClick={() => pick({ ...entry, profile })}
                    className={cn(
                      "flex cursor-default items-center gap-2.5 rounded-sm px-2 py-1.5 text-sm select-none data-[active=true]:bg-accent data-[active=true]:text-accent-foreground",
                      switchProfile.isPending && "opacity-60",
                    )}
                  >
                    <span
                      className={cn(
                        "flex size-7 shrink-0 items-center justify-center rounded-md text-xs font-medium",
                        current
                          ? "bg-primary text-primary-foreground"
                          : "bg-muted text-muted-foreground",
                      )}
                      aria-hidden="true"
                    >
                      {initials(profile.name)}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-medium">
                        {profile.name}
                      </span>
                      <span className="block truncate text-xs text-muted-foreground">
                        {t("bookSwitcher.wallets", { count: profile.wallets })}
                        {" · "}
                        {profile.taxPolicy}
                      </span>
                    </span>
                    {pendingId === profile.id ? (
                      <Loader2
                        className="size-4 shrink-0 animate-spin text-muted-foreground"
                        aria-hidden="true"
                      />
                    ) : current ? (
                      <Check className="size-4 shrink-0" aria-hidden="true" />
                    ) : null}
                  </div>
                );
              })}
            </div>
          ))
        )}
      </div>

      {switchProfile.error ? (
        <p className="border-t px-3 py-2 text-xs text-destructive">
          {switchProfile.error instanceof Error
            ? switchProfile.error.message
            : t("bookSwitcher.switchError")}
        </p>
      ) : null}

      <div className="border-t p-1">
        <Link
          to="/books"
          onClick={onClose}
          className="flex items-center gap-2 rounded-sm px-2 py-1.5 text-sm hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent focus-visible:outline-none"
        >
          <Settings2 className="size-4 text-muted-foreground" aria-hidden="true" />
          {t("bookSwitcher.manageBooks")}
        </Link>
      </div>
    </>
  );
}
