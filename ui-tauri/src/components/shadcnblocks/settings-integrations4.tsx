"use client";

import { useMemo, useState, type ReactNode } from "react";

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
  image: string;
  title: string;
  description: string;
  isConnected?: boolean;
  isSelected?: boolean;
  category: string;
  categoryLabel?: string;
  className?: string;
  actionLabel?: string;
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
  return (
    <div
      className={cn(
        "flex items-start gap-4 rounded-lg border p-4 transition-colors",
        isSelected && "border-primary bg-primary/5",
      )}
    >
      <img
        src={integration.image}
        alt={integration.title}
        className={cn("size-10 shrink-0 rounded-md", integration.className)}
      />
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <p className="min-w-0 truncate font-medium">{integration.title}</p>
          {integration.isConnected && (
            <Badge variant="secondary" className="bg-green-100 text-green-800">
              Connected
            </Badge>
          )}
        </div>
        <p className="text-sm text-muted-foreground">
          {integration.description}
        </p>
      </div>
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
  integrations: initialIntegrations = [
    {
      image:
        "https://deifkwefumgah.cloudfront.net/shadcnblocks/block/logos/slack-icon.svg",
      title: "Slack",
      description: "Send notifications and updates to your team channels.",
      isConnected: true,
      category: "communication",
    },
    {
      image:
        "https://deifkwefumgah.cloudfront.net/shadcnblocks/block/logos/discord-icon.svg",
      title: "Discord",
      description: "Connect your Discord server for real-time alerts.",
      category: "communication",
    },
    {
      image:
        "https://deifkwefumgah.cloudfront.net/shadcnblocks/block/logos/microsoft-teams-icon.svg",
      title: "Microsoft Teams",
      description: "Integrate with Teams for seamless collaboration.",
      category: "communication",
    },
    {
      image:
        "https://deifkwefumgah.cloudfront.net/shadcnblocks/block/logos/google-icon.svg",
      title: "Google Drive",
      description: "Access and sync files from your Google Drive.",
      isConnected: true,
      category: "storage",
    },
    {
      image:
        "https://deifkwefumgah.cloudfront.net/shadcnblocks/block/logos/dropbox-icon.svg",
      title: "Dropbox",
      description: "Store and share files securely in the cloud.",
      category: "storage",
    },
    {
      image:
        "https://deifkwefumgah.cloudfront.net/shadcnblocks/block/logos/microsoft-onedrive-icon.svg",
      title: "OneDrive",
      description: "Sync your Microsoft OneDrive files and folders.",
      category: "storage",
    },
    {
      image:
        "https://deifkwefumgah.cloudfront.net/shadcnblocks/block/logos/github-icon.svg",
      title: "GitHub",
      description: "Connect repositories and automate workflows.",
      isConnected: true,
      category: "development",
      className: "dark:invert",
    },
    {
      image:
        "https://deifkwefumgah.cloudfront.net/shadcnblocks/block/logos/gitlab-icon.svg",
      title: "GitLab",
      description: "Integrate with GitLab for CI/CD pipelines.",
      category: "development",
    },
    {
      image:
        "https://deifkwefumgah.cloudfront.net/shadcnblocks/block/logos/vercel-icon.svg",
      title: "Vercel",
      description: "Deploy and preview your web applications.",
      category: "development",
      className: "dark:invert",
    },
    {
      image:
        "https://deifkwefumgah.cloudfront.net/shadcnblocks/block/logos/stripe-icon.svg",
      title: "Stripe",
      description: "Process payments and manage subscriptions.",
      isConnected: true,
      category: "payments",
    },
    {
      image:
        "https://deifkwefumgah.cloudfront.net/shadcnblocks/block/logos/paypal-icon.svg",
      title: "PayPal",
      description: "Accept PayPal payments from customers.",
      category: "payments",
    },
  ],
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
