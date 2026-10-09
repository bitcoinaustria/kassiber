import {
  ArrowRight,
  BookPlus,
  Database,
  Eye,
  FolderOpen,
  LockKeyhole,
  Server,
  type LucideIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { KassiberMark } from "@/components/kb/KassiberMark";
import { Wordmark } from "@/components/kb/Wordmark";
import { Button } from "@/components/ui/button";

import { ChainArtwork } from "./chain/ChainArtwork";

interface StartChoicePanelProps {
  importAvailable: boolean;
  importing: boolean;
  openingRegtest?: boolean;
  onSetup: () => void;
  onImport: () => void;
  onQuickStart: () => void;
  onOpenRegtest?: () => void;
  regtestAvailable?: boolean;
}

const START_HIGHLIGHTS: Array<{
  icon: LucideIcon;
  titleKey: "localDatabase" | "encrypted" | "watchOnly";
}> = [
  { icon: Database, titleKey: "localDatabase" },
  { icon: LockKeyhole, titleKey: "encrypted" },
  { icon: Eye, titleKey: "watchOnly" },
];

/**
 * The first screen: what Kassiber is, one primary way in, and the chain
 * artwork. Creating books is the solid action; opening existing ones and the
 * express path step down from it, so the eye lands on one decision.
 */
export function StartChoicePanel({
  importAvailable,
  importing,
  openingRegtest = false,
  onSetup,
  onImport,
  onQuickStart,
  onOpenRegtest,
  regtestAvailable = false,
}: StartChoicePanelProps) {
  const { t } = useTranslation("onboarding");
  return (
    <div className="mx-auto grid min-h-full w-full max-w-6xl content-center items-center gap-8 px-6 py-10 sm:px-10 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)] lg:gap-12">
      <div className="flex flex-col gap-9 animate-in fade-in-0 slide-in-from-bottom-2 duration-500">
        <div className="flex items-center gap-2.5">
          <KassiberMark className="size-7" />
          <Wordmark size={17} />
        </div>

        <header className="space-y-4">
          <p className="font-mono text-2xs font-medium uppercase tracking-[0.14em] text-ink-3">
            {t("brand.tagline")}
          </p>
          <h1 className="text-5xl leading-[1.05] font-semibold tracking-tight text-balance text-ink">
            {t("start.headline")}
          </h1>
          <p className="max-w-md text-base leading-7 text-pretty text-ink-2">
            {t("start.lead")}
          </p>
        </header>

        <div className="space-y-4">
          {regtestAvailable ? (
            <button
              type="button"
              disabled={openingRegtest}
              onClick={onOpenRegtest}
              className="group flex w-full max-w-md items-center gap-3 rounded-(--kb-radius-card) border border-emerald-500/35 bg-emerald-500/10 p-3 text-left text-ink transition hover:bg-emerald-500/15 disabled:cursor-wait disabled:opacity-80 dark:border-emerald-400/30 dark:bg-emerald-400/10"
            >
              <span className="flex size-8 shrink-0 items-center justify-center rounded-(--kb-radius-inset) bg-emerald-500 text-white">
                <Server className="size-4" aria-hidden="true" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-semibold">
                  {openingRegtest
                    ? t("start.regtest.titleOpening")
                    : t("start.regtest.title")}
                </span>
                <span className="mt-0.5 block text-xs leading-5 text-ink-2">
                  {t("start.regtest.body")}
                </span>
              </span>
              <ArrowRight
                className="size-4 shrink-0 transition-transform group-hover:translate-x-0.5"
                aria-hidden="true"
              />
            </button>
          ) : null}

          <div className="flex flex-wrap items-center gap-3">
            <Button size="lg" onClick={onSetup} className="group">
              <BookPlus aria-hidden="true" />
              {t("start.createNew.title")}
              <ArrowRight
                className="transition-transform group-hover:translate-x-0.5"
                aria-hidden="true"
              />
            </Button>
            <Button
              size="lg"
              variant="outline"
              disabled={!importAvailable || importing}
              onClick={onImport}
              title={
                importAvailable
                  ? t("start.openExisting.tooltipAvailable")
                  : t("start.openExisting.tooltipUnavailable")
              }
            >
              <FolderOpen aria-hidden="true" />
              {importing
                ? t("start.openExisting.titleOpening")
                : t("start.openExisting.title")}
            </Button>
          </div>
          {!importAvailable && (
            <p className="font-mono text-2xs font-medium uppercase tracking-[0.14em] text-ink-3">
              {t("start.openExisting.desktopOnly")}
            </p>
          )}
          <button
            type="button"
            onClick={onQuickStart}
            className="group -mx-1 inline-flex items-center gap-1.5 rounded-md px-1 py-0.5 text-sm font-medium text-ink-2 underline-offset-4 outline-none transition-colors hover:text-ink hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50"
          >
            {t("start.quickStart")}
            <ArrowRight
              className="size-3.5 transition-transform group-hover:translate-x-0.5"
              aria-hidden="true"
            />
          </button>
        </div>

        <ul className="grid list-none gap-4 border-t border-border p-0 pt-6 sm:grid-cols-3">
          {START_HIGHLIGHTS.map(({ icon: Icon, titleKey }) => (
            <li key={titleKey} className="space-y-1">
              <p className="flex items-center gap-2 text-sm font-medium text-ink">
                <Icon className="size-3.5 shrink-0 text-ink-3" aria-hidden="true" />
                {t(`start.highlights.${titleKey}.title`)}
              </p>
              <p className="text-xs leading-5 text-ink-3">
                {t(`start.highlights.${titleKey}.body`)}
              </p>
            </li>
          ))}
        </ul>
      </div>

      {/* Three mined blocks and the next one filling from the mempool, on a
          loop: it fills, is found, and the chain moves back a slot. */}
      <ChainArtwork
        live
        mined={3}
        fill={0.5}
        className="order-first h-56 animate-in fade-in-0 duration-700 lg:order-none lg:h-[30rem]"
      />
    </div>
  );
}
