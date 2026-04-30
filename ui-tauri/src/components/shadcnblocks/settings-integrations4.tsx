"use client";

import { useEffect, useState } from "react";

import { ScrollableTabsList } from "@/components/shadcnblocks/scrollable-tabslist";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";

export interface IntegrationItem {
  image: string;
  title: string;
  description: string;
  isConnected?: boolean;
  category: string;
  className?: string;
  actionLabel?: string;
}

interface IntegrationCardProps {
  integration: IntegrationItem;
  onToggle?: (integration: IntegrationItem) => void;
}

const IntegrationCard = ({ integration, onToggle }: IntegrationCardProps) => {
  return (
    <div className="flex items-start gap-4 rounded-lg border p-4">
      <img
        src={integration.image}
        alt={integration.title}
        className={cn("size-10 shrink-0", integration.className)}
      />
      <div className="flex-1 space-y-1">
        <div className="flex items-center gap-2">
          <p className="font-medium">{integration.title}</p>
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
        variant={integration.isConnected ? "outline" : "default"}
        size="sm"
        onClick={() => onToggle?.(integration)}
      >
        {integration.actionLabel ??
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
  onToggleIntegration?: (integration: IntegrationItem) => void;
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
  onToggleIntegration,
}: SettingsIntegrations4Props) => {
  const [integrations, setIntegrations] = useState(initialIntegrations);

  useEffect(() => {
    setIntegrations(initialIntegrations);
  }, [initialIntegrations]);

  const categories = [...new Set(integrations.map((i) => i.category))];

  const handleToggle = (integration: IntegrationItem) => {
    onToggleIntegration?.(integration);
    if (integration.actionLabel) return;
    setIntegrations((prev) =>
      prev.map((i) =>
        i.title === integration.title
          ? { ...i, isConnected: !i.isConnected }
          : i,
      ),
    );
  };

  const getCategoryLabel = (category: string) => {
    return category.charAt(0).toUpperCase() + category.slice(1);
  };

  return (
    <section className={cn("py-32", className)}>
      <div className="container max-w-4xl">
        <div className="space-y-2 border-b pb-6">
          <h2 className="text-2xl font-semibold tracking-tight">{heading}</h2>
          <p className="text-sm text-muted-foreground">{subHeading}</p>
        </div>

        <Tabs defaultValue={categories[0]} className="mt-6">
          <ScrollableTabsList className="rounded-none">
            <TabsList className="overflow-y-hidden">
              {categories.map((category) => (
                <TabsTrigger key={category} value={category}>
                  {getCategoryLabel(category)}
                </TabsTrigger>
              ))}
            </TabsList>
          </ScrollableTabsList>

          {categories.map((category) => (
            <TabsContent key={category} value={category} className="mt-6">
              <div className="space-y-4">
                {integrations
                  .filter((i) => i.category === category)
                  .map((integration) => (
                    <IntegrationCard
                      key={integration.title}
                      integration={integration}
                      onToggle={handleToggle}
                    />
                  ))}
              </div>
            </TabsContent>
          ))}
        </Tabs>
      </div>
    </section>
  );
};

export { SettingsIntegrations4 };
