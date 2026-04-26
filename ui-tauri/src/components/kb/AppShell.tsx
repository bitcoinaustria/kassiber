import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import {
  BarChart3,
  Bell,
  BookOpen,
  Bug,
  ChevronRight,
  ChevronsUpDown,
  ClipboardList,
  Download,
  Eye,
  EyeOff,
  Heart,
  LayoutDashboard,
  LogOut,
  Search,
  Server,
  Settings,
  CircleDollarSign,
  ShieldAlert,
  User,
  Users,
  Wallet,
} from "lucide-react";
import * as React from "react";

import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  SidebarProvider,
  SidebarRail,
  SidebarTrigger,
} from "@/components/ui/sidebar";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useUiStore } from "@/store/ui";
import { useDaemon } from "@/daemon/client";
import type { OverviewSnapshot } from "@/mocks/seed";
import { SettingsModal } from "./SettingsModal";
import { ScreenAssistantMockup } from "./ScreenAssistantMockup";
import { PreAlphaBanner } from "./PreAlphaBanner";

type NavItem = {
  label: string;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  href: string;
  children?: NavItem[];
};

type NavGroup = {
  title: string;
  items: NavItem[];
};

type RouteMeta = {
  title: string;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  searchLabel: string;
  searchPlaceholder: string;
};

const APP_VERSION = "0.21.0";

const NAV_GROUPS: NavGroup[] = [
  {
    title: "Main",
    items: [
      { label: "Overview", icon: LayoutDashboard, href: "/overview" },
      { label: "Transactions", icon: ClipboardList, href: "/transactions" },
      { label: "Reports", icon: BarChart3, href: "/reports" },
    ],
  },
  {
    title: "Ledger",
    items: [
      {
        label: "Connections",
        icon: Wallet,
        href: "/connections",
        children: [
          { label: "Wallets", icon: Wallet, href: "/connections" },
          { label: "Profiles", icon: Users, href: "/profiles" },
          { label: "Imports", icon: Download, href: "/transactions" },
        ],
      },
      { label: "Journals", icon: BookOpen, href: "/journals" },
    ],
  },
  {
    title: "Review",
    items: [
      { label: "Tax Events", icon: CircleDollarSign, href: "/tax-events" },
      { label: "Quarantine", icon: ShieldAlert, href: "/quarantine" },
    ],
  },
];

const ROUTE_META: Array<[string, RouteMeta]> = [
  [
    "/connections/",
    {
      title: "Connection Detail",
      icon: Wallet,
      searchLabel: "Search connections",
      searchPlaceholder: "Search wallets, profiles...",
    },
  ],
  [
    "/connections",
    {
      title: "Connections",
      icon: Wallet,
      searchLabel: "Search connections",
      searchPlaceholder: "Search wallets, profiles...",
    },
  ],
  [
    "/profiles",
    {
      title: "Profiles",
      icon: Users,
      searchLabel: "Search profiles",
      searchPlaceholder: "Search profiles, countries...",
    },
  ],
  [
    "/journals",
    {
      title: "Journals",
      icon: BookOpen,
      searchLabel: "Search journals",
      searchPlaceholder: "Search entry type, wallet, asset...",
    },
  ],
  [
    "/reports",
    {
      title: "Reports",
      icon: BarChart3,
      searchLabel: "Search reports",
      searchPlaceholder: "Search reports, exports...",
    },
  ],
  [
    "/tax-events",
    {
      title: "Tax Events",
      icon: CircleDollarSign,
      searchLabel: "Search tax events",
      searchPlaceholder: "Search event, basis, account...",
    },
  ],
  [
    "/quarantine",
    {
      title: "Quarantine",
      icon: ShieldAlert,
      searchLabel: "Search quarantine",
      searchPlaceholder: "Search issue, account, source...",
    },
  ],
  [
    "/transactions",
    {
      title: "Transactions",
      icon: ClipboardList,
      searchLabel: "Search transactions",
      searchPlaceholder: "Search counterparty, tag, account...",
    },
  ],
  [
    "/overview",
    {
      title: "Overview",
      icon: LayoutDashboard,
      searchLabel: "Search overview",
      searchPlaceholder: "Search transactions, reports...",
    },
  ],
];

export function AppShell() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const [settingsFocus, setSettingsFocus] = React.useState<
    "backends" | null
  >(null);
  const [assistantCollapsed, setAssistantCollapsed] = React.useState(false);
  const mainRef = React.useRef<HTMLElement>(null);
  const routeMeta =
    ROUTE_META.find(([prefix]) => pathname.startsWith(prefix))?.[1] ?? {
      title: "Kassiber",
      icon: LayoutDashboard,
      searchLabel: "Search Kassiber",
      searchPlaceholder: "Search transactions, reports...",
    };

  React.useEffect(() => {
    const main = mainRef.current;
    if (!main) {
      return;
    }

    const syncAssistantState = () => {
      setAssistantCollapsed(main.scrollTop > 32);
    };

    syncAssistantState();
    main.addEventListener("scroll", syncAssistantState, { passive: true });

    return () => {
      main.removeEventListener("scroll", syncAssistantState);
    };
  }, [pathname]);

  React.useEffect(() => {
    const openSettings = (event: Event) => {
      const detail = (event as CustomEvent<{ section?: "backends" }>).detail;
      setSettingsFocus(detail?.section ?? null);
      setSettingsOpen(true);
    };

    window.addEventListener("kassiber:open-settings", openSettings);

    return () => {
      window.removeEventListener("kassiber:open-settings", openSettings);
    };
  }, []);

  return (
    <TooltipProvider>
      <div className="flex h-svh flex-col overflow-hidden bg-sidebar">
        <PreAlphaBanner className="shrink-0" />
        <SidebarProvider className="min-h-0 flex-1 bg-sidebar">
          <a
            href="#app-main"
            className="sr-only focus:not-sr-only focus:absolute focus:top-4 focus:left-4 focus:z-50 focus:rounded-md focus:bg-background focus:px-3 focus:py-2 focus:text-sm focus:text-foreground focus:ring-2 focus:ring-ring"
          >
            Skip to main content
          </a>
          <AppSidebar
            pathname={pathname}
            onSettingsClick={() => {
              setSettingsFocus(null);
              setSettingsOpen(true);
            }}
          />
          <div className="min-h-0 w-full overflow-hidden lg:p-2">
            <div className="relative flex h-full w-full flex-col items-center justify-start overflow-hidden bg-background lg:rounded-xl lg:border">
              <AppDashboardHeader meta={routeMeta} />
              <main
                id="app-main"
                ref={mainRef}
                tabIndex={-1}
                className={`min-h-0 w-full flex-1 overflow-auto bg-background transition-[padding-bottom] duration-200 ${
                  assistantCollapsed ? "pb-[150px]" : "pb-[240px]"
                }`}
              >
                <Outlet />
              </main>
              <ScreenAssistantMockup
                collapsed={assistantCollapsed}
                className="absolute inset-x-0 bottom-0 z-20"
              />
            </div>
          </div>
        </SidebarProvider>
      </div>
      <SettingsModal
        open={settingsOpen}
        focusSection={settingsFocus}
        onClose={() => setSettingsOpen(false)}
      />
    </TooltipProvider>
  );
}

function AppSidebar({
  pathname,
  onSettingsClick,
}: {
  pathname: string;
  onSettingsClick: () => void;
}) {
  return (
    <Sidebar
      variant="inset"
      collapsible="icon"
      className="top-9 h-[calc(100svh-2.25rem)]"
    >
      <SidebarHeader>
        <div className="flex items-center gap-2 group-data-[collapsible=icon]:flex-col group-data-[collapsible=icon]:items-center">
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton
                size="lg"
                tooltip="Kassiber"
                asChild
                className="group-data-[collapsible=icon]:size-9! group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:p-0!"
              >
                <Link
                  to="/overview"
                  className="min-w-0 group-data-[collapsible=icon]:justify-center"
                >
                  <div className="flex aspect-square size-8 shrink-0 items-center justify-center rounded-sm bg-primary group-data-[collapsible=icon]:size-9">
                    <Wallet
                      className="size-5 text-primary-foreground"
                      aria-hidden="true"
                    />
                  </div>
                  <div className="flex min-w-0 flex-col gap-0.5 leading-none group-data-[collapsible=icon]:hidden">
                    <span className="truncate font-medium">Kassiber</span>
                    <span className="truncate text-xs text-muted-foreground">
                      Private Bitcoin Books
                    </span>
                  </div>
                </Link>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
          <SidebarTrigger className="ml-auto group-data-[collapsible=icon]:ml-0" />
        </div>
      </SidebarHeader>
      <SidebarContent>
        {NAV_GROUPS.map((group) => (
          <SidebarGroup key={group.title}>
            <SidebarGroupLabel>{group.title}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {group.items.map((item) => (
                  <NavMenuItem key={item.label} item={item} pathname={pathname} />
                ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
      </SidebarContent>
      <SidebarFooter>
        <SidebarActions onSettingsClick={onSettingsClick} />
        <NavUser />
        <AppVersion />
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  );
}

function SidebarActions({ onSettingsClick }: { onSettingsClick: () => void }) {
  const dataMode = useUiStore((state) => state.dataMode);
  const setDataMode = useUiStore((state) => state.setDataMode);
  const isRealData = dataMode === "real";

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <div className="flex min-h-8 w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-0">
          <Server className="size-4 shrink-0" aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate group-data-[collapsible=icon]:hidden">
            {isRealData ? "Real data" : "Mock data"}
          </span>
          <Switch
            checked={isRealData}
            aria-label="Toggle real data mode"
            onCheckedChange={(checked) =>
              setDataMode(checked ? "real" : "mock")
            }
            className="group-data-[collapsible=icon]:hidden"
          />
        </div>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <SidebarMenuButton asChild tooltip="Donate sats">
          <a href="#donate">
            <Heart className="size-4" aria-hidden="true" />
            <span>Donate sats</span>
          </a>
        </SidebarMenuButton>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <SidebarMenuButton asChild tooltip="Bug report">
          <a
            href="https://github.com/bitcoinaustria/kassiber/issues"
            target="_blank"
            rel="noreferrer"
          >
            <Bug className="size-4" aria-hidden="true" />
            <span>Bug report</span>
          </a>
        </SidebarMenuButton>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <SidebarMenuButton onClick={onSettingsClick} tooltip="Settings">
          <Settings className="size-4" aria-hidden="true" />
          <span>Settings</span>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

function NavMenuItem({
  item,
  pathname,
}: {
  item: NavItem;
  pathname: string;
}) {
  const Icon = item.icon;
  const active = pathname === item.href || pathname.startsWith(`${item.href}/`);

  if (!item.children?.length) {
    return (
      <SidebarMenuItem>
        <SidebarMenuButton asChild isActive={active} tooltip={item.label}>
          <a href={item.href}>
            <Icon className="size-4" aria-hidden="true" />
            <span>{item.label}</span>
          </a>
        </SidebarMenuButton>
      </SidebarMenuItem>
    );
  }

  return (
    <Collapsible asChild defaultOpen={active} className="group/collapsible">
      <SidebarMenuItem>
        <CollapsibleTrigger asChild>
          <SidebarMenuButton isActive={active} tooltip={item.label}>
            <Icon className="size-4" aria-hidden="true" />
            <span>{item.label}</span>
            <ChevronRight className="ml-auto size-4 transition-transform duration-200 group-data-[state=open]/collapsible:rotate-90" />
          </SidebarMenuButton>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <SidebarMenuSub>
            {item.children.map((child) => {
              const childActive =
                pathname === child.href || pathname.startsWith(`${child.href}/`);
              return (
                <SidebarMenuSubItem key={child.label}>
                  <SidebarMenuSubButton asChild isActive={childActive}>
                    <a href={child.href}>{child.label}</a>
                  </SidebarMenuSubButton>
                </SidebarMenuSubItem>
              );
            })}
          </SidebarMenuSub>
        </CollapsibleContent>
      </SidebarMenuItem>
    </Collapsible>
  );
}

function NavUser() {
  const identity = useUiStore((s) => s.identity);
  const setIdentity = useUiStore((s) => s.setIdentity);
  const name = identity?.workspace ?? "Demo Workspace";
  const detail = identity?.name ?? "local profile";

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <SidebarMenuButton
              size="lg"
              className="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground group-data-[collapsible=icon]:size-9! group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:p-0!"
            >
              <Avatar className="size-8 shrink-0 rounded-lg group-data-[collapsible=icon]:size-9">
                <AvatarFallback className="rounded-lg text-sm font-medium group-data-[collapsible=icon]:text-sm">
                  {name
                    .split(" ")
                    .map((part) => part[0])
                    .join("")
                    .slice(0, 2)}
                </AvatarFallback>
              </Avatar>
              <div className="grid flex-1 text-left text-sm leading-tight group-data-[collapsible=icon]:hidden">
                <span className="truncate font-medium">{name}</span>
                <span className="truncate text-xs text-muted-foreground">
                  {detail}
                </span>
              </div>
              <ChevronsUpDown
                className="ml-auto size-4 group-data-[collapsible=icon]:hidden"
                aria-hidden="true"
              />
            </SidebarMenuButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            className="w-(--radix-dropdown-menu-trigger-width) min-w-56 rounded-lg"
            side="bottom"
            align="end"
            sideOffset={4}
          >
            <DropdownMenuLabel className="p-0 font-normal">
              <div className="flex items-center gap-2 px-1 py-1.5 text-left text-sm">
                <Avatar className="size-8 rounded-lg">
                  <AvatarFallback className="rounded-lg">
                    {name
                      .split(" ")
                      .map((part) => part[0])
                      .join("")
                      .slice(0, 2)}
                  </AvatarFallback>
                </Avatar>
                <div className="grid flex-1 text-left text-sm leading-tight">
                  <span className="truncate font-medium">{name}</span>
                  <span className="truncate text-xs text-muted-foreground">
                    {detail}
                  </span>
                </div>
              </div>
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuItem asChild>
              <Link to="/profiles">
                <User className="mr-2 size-4" aria-hidden="true" />
                Profiles
              </Link>
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => setIdentity(null)}>
              <LogOut className="mr-2 size-4" aria-hidden="true" />
              Lock Kassiber
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

function AppVersion() {
  return (
    <a
      href="https://github.com/bitcoinaustria/kassiber"
      target="_blank"
      rel="noreferrer"
      className="px-2 pb-1 text-center text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline group-data-[collapsible=icon]:hidden"
    >
      Kassiber v{APP_VERSION}
    </a>
  );
}

function AppDashboardHeader({ meta }: { meta: RouteMeta }) {
  const Icon = meta.icon;
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const dataMode = useUiStore((s) => s.dataMode);
  const appNotifications = useUiStore((s) => s.notifications);
  const clearNotifications = useUiStore((s) => s.clearNotifications);
  const { data } = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const snapshot = data?.data;
  const systemNotificationItems = [
    ...(snapshot?.status?.needsJournals
      ? [
          {
            id: "journals-stale",
            title: "Journals need processing",
            body: "Reports are not trusted until journals are processed.",
            tone: "warning" as const,
          },
        ]
      : []),
    ...((snapshot?.status?.quarantines ?? 0) > 0
      ? [
          {
            id: "quarantines",
            title: "Transactions quarantined",
            body: `${snapshot?.status?.quarantines ?? 0} transactions need review.`,
            tone: "warning" as const,
          },
        ]
      : []),
    {
      id: "data-mode",
      title: dataMode === "mock" ? "Mock data active" : "Real data active",
      body:
        dataMode === "mock"
          ? "The UI is showing fixture data."
          : "The UI is reading from the local daemon.",
      tone: "info" as const,
    },
  ];
  const notificationItems = [...appNotifications, ...systemNotificationItems];
  const notificationCount = notificationItems.filter(
    (item) =>
      item.tone !== "info" ||
      item.title.toLowerCase().includes("sync"),
  ).length;

  return (
    <header className="flex w-full items-center gap-3 border-b bg-background px-4 py-4 sm:px-6">
      <Icon className="size-5" aria-hidden="true" />
      <h1 className="text-base font-medium">{meta.title}</h1>

      <div className="ml-auto flex items-center gap-2">
        <div className="relative hidden w-80 lg:block lg:w-96 xl:w-[28rem]">
          <Search
            className="absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
          <Input
            type="search"
            name="header-search"
            inputMode="search"
            autoComplete="off"
            aria-label={meta.searchLabel}
            placeholder={meta.searchPlaceholder}
            className="h-10 w-full pr-14 pl-9 text-sm"
          />
          <kbd className="pointer-events-none absolute top-1/2 right-2 hidden -translate-y-1/2 rounded-md border bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground md:inline-flex">
            {"\u2318"}
            {"\u00a0"}K
          </kbd>
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="outline"
              size="icon"
              className="relative size-9"
              aria-label="Notifications"
            >
              <Bell className="size-4" aria-hidden="true" />
              {notificationCount > 0 && (
                <span className="absolute -top-1 -right-1 flex size-4 items-center justify-center rounded-full bg-destructive text-[9px] font-medium text-destructive-foreground">
                  {notificationCount}
                </span>
              )}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-80">
            <div className="flex items-center justify-between gap-2 px-2 py-1.5">
              <DropdownMenuLabel className="p-0">
                Notifications
              </DropdownMenuLabel>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-7 px-2 text-xs"
                disabled={appNotifications.length === 0}
                onClick={(event) => {
                  event.preventDefault();
                  clearNotifications();
                }}
              >
                Clear all
              </Button>
            </div>
            <DropdownMenuSeparator />
            {notificationItems.map((item) => (
              <DropdownMenuItem
                key={item.id}
                className="flex flex-col items-start gap-0.5 whitespace-normal"
              >
                <span className="font-medium">{item.title}</span>
                <span className="text-xs text-muted-foreground">
                  {item.body}
                </span>
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
        <Button
          variant="outline"
          size="icon"
          className={
            hideSensitive
              ? "size-9 bg-primary text-primary-foreground hover:bg-primary/90 hover:text-primary-foreground"
              : "size-9"
          }
          aria-label={
            hideSensitive ? "Show sensitive data" : "Hide sensitive data"
          }
          aria-pressed={hideSensitive}
          title={hideSensitive ? "Show sensitive data" : "Hide sensitive data"}
          onClick={() => setHideSensitive(!hideSensitive)}
        >
          {hideSensitive ? (
            <EyeOff className="size-4" aria-hidden="true" />
          ) : (
            <Eye className="size-4" aria-hidden="true" />
          )}
        </Button>
      </div>
    </header>
  );
}
