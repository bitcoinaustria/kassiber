import { Link } from "@tanstack/react-router";
import { Compass } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { screenShellClassName } from "@/lib/screen-layout";

/**
 * Unknown route, rendered inside the app shell.
 *
 * It hangs off the nav layout (and is the router's default not-found), so a
 * mistyped or stale link still leaves the nav, the search palette and the
 * breadcrumb in place — a bare root-level 404 left the window black with no way
 * back except quitting.
 */
export function NotFound() {
  const { t } = useTranslation("chrome");
  return (
    <div className={screenShellClassName}>
      <div className="flex flex-col items-start gap-2 py-10">
        <Compass className="size-5 text-muted-foreground" aria-hidden="true" />
        <h1 className="text-base font-semibold">{t("notFound.title")}</h1>
        <p className="max-w-prose text-sm text-muted-foreground">
          {t("notFound.message", { path: window.location.pathname })}
        </p>
        <Button asChild size="sm" className="mt-2">
          <Link to="/overview">{t("notFound.backToOverview")}</Link>
        </Button>
      </div>
    </div>
  );
}
