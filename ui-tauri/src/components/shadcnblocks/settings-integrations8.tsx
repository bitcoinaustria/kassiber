"use client";

import { useMemo, useState } from "react";
import { ChevronRight, Search } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";

export interface SettingsIntegration8Item {
  id: string;
  title: string;
  description: string;
  category: string;
  categoryLabel: string;
  image?: string;
  initials?: string;
  isConnected?: boolean;
  disabled?: boolean;
  actionLabel?: string;
  className?: string;
}

interface SettingsIntegrations8Props {
  className?: string;
  heading?: string;
  subHeading?: string;
  integrations: SettingsIntegration8Item[];
  selectedId?: string | null;
  onSelect?: (integration: SettingsIntegration8Item) => void;
}

export function SettingsIntegrations8({
  className,
  heading = "Integrations",
  subHeading = "Browse wallet sources, services, exchanges, and local import formats.",
  integrations,
  selectedId,
  onSelect,
}: SettingsIntegrations8Props) {
  const [query, setQuery] = useState("");
  const [activeCategory, setActiveCategory] = useState("all");

  const categories = useMemo(() => {
    const grouped = new Map<
      string,
      { id: string; label: string; count: number }
    >();
    for (const integration of integrations) {
      const current = grouped.get(integration.category);
      grouped.set(integration.category, {
        id: integration.category,
        label: integration.categoryLabel,
        count: (current?.count ?? 0) + 1,
      });
    }
    return [
      { id: "all", label: "All integrations", count: integrations.length },
      ...Array.from(grouped.values()),
    ];
  }, [integrations]);

  const filteredIntegrations = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    return integrations.filter((integration) => {
      const matchesCategory =
        activeCategory === "all" || integration.category === activeCategory;
      const haystack = [
        integration.title,
        integration.description,
        integration.categoryLabel,
      ]
        .join(" ")
        .toLowerCase();
      return (
        matchesCategory && (!normalizedQuery || haystack.includes(normalizedQuery))
      );
    });
  }, [activeCategory, integrations, query]);

  return (
    <section className={cn("rounded-lg border bg-card text-card-foreground shadow-sm", className)}>
      <div className="space-y-2 border-b px-6 py-5">
        <h2 className="text-2xl font-semibold tracking-tight">{heading}</h2>
        <p className="text-sm text-muted-foreground">{subHeading}</p>
      </div>

      <div className="grid min-h-[560px] grid-cols-1 md:grid-cols-[260px_minmax(0,1fr)]">
        <aside className="border-b p-4 md:border-r md:border-b-0">
          <div className="relative">
            <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search integrations..."
              className="pl-9"
            />
          </div>

          <div className="mt-4 space-y-1">
            {categories.map((category) => {
              const active = activeCategory === category.id;
              return (
                <Button
                  key={category.id}
                  type="button"
                  variant={active ? "secondary" : "ghost"}
                  className="h-9 w-full justify-between px-3"
                  onClick={() => setActiveCategory(category.id)}
                >
                  <span className="truncate">{category.label}</span>
                  <span className="text-xs text-muted-foreground">
                    {category.count}
                  </span>
                </Button>
              );
            })}
          </div>
        </aside>

        <ScrollArea className="h-[560px]">
          <div className="space-y-3 p-4">
            {filteredIntegrations.length === 0 ? (
              <div className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">
                No integrations match this search.
              </div>
            ) : (
              filteredIntegrations.map((integration) => (
                <IntegrationRow
                  key={integration.id}
                  integration={integration}
                  selected={selectedId === integration.id}
                  onSelect={onSelect}
                />
              ))
            )}
          </div>
        </ScrollArea>
      </div>
    </section>
  );
}

function IntegrationRow({
  integration,
  selected,
  onSelect,
}: {
  integration: SettingsIntegration8Item;
  selected: boolean;
  onSelect?: (integration: SettingsIntegration8Item) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onSelect?.(integration)}
      aria-disabled={integration.disabled}
      className={cn(
        "group flex w-full items-start gap-4 rounded-lg border p-4 text-left transition-colors",
        selected && "border-primary bg-primary/5",
        !selected && "hover:bg-muted/40",
        integration.disabled && "opacity-70",
      )}
    >
      <span className="flex size-12 shrink-0 items-center justify-center rounded-lg border bg-background">
        {integration.image ? (
          <img
            src={integration.image}
            alt=""
            className={cn("size-7", integration.className)}
            aria-hidden="true"
          />
        ) : (
          <span className="text-xs font-semibold tracking-wide text-muted-foreground">
            {integration.initials}
          </span>
        )}
      </span>

      <span className="min-w-0 flex-1 space-y-1">
        <span className="flex flex-wrap items-center gap-2">
          <span className="font-medium">{integration.title}</span>
          {integration.isConnected && (
            <Badge variant="secondary" className="bg-green-100 text-green-800">
              <span className="size-1.5 rounded-full bg-green-600" />
              Connected
            </Badge>
          )}
          {integration.disabled && <Badge variant="outline">Soon</Badge>}
        </span>
        <span className="block text-sm text-muted-foreground">
          {integration.description}
        </span>
      </span>

      <span className="flex items-center gap-2 text-sm text-muted-foreground">
        {integration.actionLabel}
        <ChevronRight
          className={cn(
            "size-4 opacity-0 transition-opacity group-hover:opacity-100",
            selected && "opacity-100",
          )}
          aria-hidden="true"
        />
      </span>
    </button>
  );
}
