"use client";

import {
  useEffect,
  useMemo,
  useState,
  type ComponentType,
  type ReactNode,
  type SVGProps,
} from "react";

import { ScrollableTabsList } from "@/components/shadcnblocks/scrollable-tabslist";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { cn } from "@/lib/utils";

export interface IntegrationItem {
  id?: string;
  image?: string;
  icon?: ComponentType<SVGProps<SVGSVGElement>>;
  title: string;
  description: string;
  isConnected?: boolean;
  isSelected?: boolean;
  category: string;
  categoryLabel?: string;
  className?: string;
  imageFrameClassName?: string;
  actionLabel?: string;
  action?: ReactNode;
  statusLabel?: string;
}

interface IntegrationCardProps {
  integration: IntegrationItem;
  isSelected?: boolean;
  onToggle?: (integration: IntegrationItem) => void;
}

const IntegrationCard = ({
  integration,
  isSelected = false,
  onToggle,
}: IntegrationCardProps) => {
  const Icon = integration.icon;
  return (
    <div
      className={cn(
        "flex items-start gap-4 rounded-lg border p-4 transition-colors",
        isSelected && "border-primary bg-primary/5",
      )}
    >
      {integration.image ? (
        <span
          className={cn(
            "flex size-10 shrink-0 items-center justify-center",
            integration.imageFrameClassName,
          )}
        >
          <img
            src={integration.image}
            alt={integration.title}
            className={cn(
              "max-h-full max-w-full object-contain",
              integration.className,
            )}
          />
        </span>
      ) : Icon ? (
        <span
          className={cn(
            "flex size-10 shrink-0 items-center justify-center rounded-md border bg-background text-muted-foreground",
            integration.imageFrameClassName,
          )}
          aria-hidden="true"
        >
          <Icon className={cn("size-5", integration.className)} />
        </span>
      ) : null}
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <p className="min-w-0 truncate font-medium">{integration.title}</p>
          {integration.isConnected && (
            <Badge variant="secondary" className="bg-green-100 text-green-800">
              {integration.statusLabel ?? "Connected"}
            </Badge>
          )}
        </div>
        <p className="text-sm text-muted-foreground">
          {integration.description}
        </p>
      </div>
      {integration.action ? (
        <div className="shrink-0">{integration.action}</div>
      ) : (
        <Button
          variant={
            isSelected
              ? "secondary"
              : integration.isConnected
                ? "outline"
                : "default"
          }
          size="sm"
          className="shrink-0"
          onClick={() => onToggle?.(integration)}
        >
          {isSelected
            ? "Selected"
            : integration.actionLabel ??
            (integration.isConnected ? "Disconnect" : "Connect")}
        </Button>
      )}
    </div>
  );
};

interface SettingsIntegrations4Props {
  className?: string;
  heading?: string;
  subHeading?: string;
  integrations?: IntegrationItem[];
  selectedId?: string;
  onSelect?: (integration: IntegrationItem) => void;
  onToggleIntegration?: (integration: IntegrationItem) => void;
  renderDetail?: (integration: IntegrationItem) => ReactNode;
}

const SettingsIntegrations4 = ({
  className,
  heading = "Integrations",
  subHeading = "Connect your favorite tools and services to streamline your workflow.",
  integrations: initialIntegrations = [],
  selectedId,
  onSelect,
  onToggleIntegration,
  renderDetail,
}: SettingsIntegrations4Props) => {
  const integrations = initialIntegrations;
  const categories = useMemo(
    () => [...new Set(integrations.map((i) => i.category))],
    [integrations],
  );
  const [activeCategoryId, setActiveCategoryId] = useState<string | null>(null);
  const activeCategory =
    activeCategoryId && categories.includes(activeCategoryId)
      ? activeCategoryId
      : categories[0];

  const handleToggle = (integration: IntegrationItem) => {
    onSelect?.(integration);
    onToggleIntegration?.(integration);
  };

  const getCategoryLabel = (category: string) => {
    return (
      integrations.find((item) => item.category === category)?.categoryLabel ??
      category.charAt(0).toUpperCase() + category.slice(1)
    );
  };
  const selectedIntegration =
    selectedId == null
      ? null
      : integrations.find(
          (integration) =>
            integration.id === selectedId || integration.title === selectedId,
        ) ?? null;
  useEffect(() => {
    if (!selectedIntegration?.category) return;
    setActiveCategoryId(selectedIntegration.category);
  }, [selectedIntegration?.category]);
  const visibleSelectedIntegration =
    selectedIntegration?.category === activeCategory ? selectedIntegration : null;
  const detail =
    visibleSelectedIntegration && renderDetail
      ? renderDetail(visibleSelectedIntegration)
      : null;

  return (
    <Card className={cn("min-w-0 overflow-hidden", className)}>
      <CardHeader className="border-b">
        <CardTitle>{heading}</CardTitle>
        <CardDescription>{subHeading}</CardDescription>
      </CardHeader>
      <CardContent className="p-0">
        <div className="border-b px-4 py-3">
          <ScrollableTabsList>
            <div className="inline-flex h-9 w-fit items-center justify-center rounded-lg bg-muted p-[3px] text-muted-foreground">
              {categories.map((category) => {
                const active = activeCategory === category;
                return (
                  <button
                    key={category}
                    type="button"
                    className={cn(
                      "inline-flex h-[calc(100%-1px)] flex-1 items-center justify-center gap-1.5 rounded-md border border-transparent px-2 py-1 text-sm font-medium whitespace-nowrap transition-[color,box-shadow]",
                      active &&
                        "bg-background text-foreground shadow-sm dark:border-input dark:bg-input/30",
                    )}
                    onClick={() => setActiveCategoryId(category)}
                  >
                    {getCategoryLabel(category)}
                  </button>
                );
              })}
            </div>
          </ScrollableTabsList>
        </div>

        <div className="space-y-3 p-4">
          {integrations
            .filter((i) => i.category === activeCategory)
            .map((integration) => {
              const isSelected =
                integration.isSelected ||
                (selectedId != null &&
                  (integration.id === selectedId ||
                    integration.title === selectedId));
              return (
                <IntegrationCard
                  key={integration.id ?? integration.title}
                  integration={integration}
                  isSelected={isSelected}
                  onToggle={handleToggle}
                />
              );
            })}
        </div>
        {detail ? <div className="border-t bg-muted/20 p-4">{detail}</div> : null}
      </CardContent>
    </Card>
  );
};

export { SettingsIntegrations4 };
