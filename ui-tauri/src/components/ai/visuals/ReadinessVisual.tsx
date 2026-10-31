import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle2, CircleAlert, OctagonX, Sparkles } from "lucide-react";

import { cn } from "@/lib/utils";
import type { CheckStatus, ReadinessCheck, ReadinessVisual as ReadinessVisualModel } from "./model";
import type { VisualFormat } from "./format";
import { stagger } from "./motion";

const RADIUS = 26;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

const STATUS_ICON: Record<CheckStatus, typeof CheckCircle2> = {
  ok: CheckCircle2,
  warning: CircleAlert,
  blocked: OctagonX,
};

const STATUS_CLASS: Record<CheckStatus, string> = {
  ok: "text-emerald-600 dark:text-emerald-400",
  warning: "text-amber-600 dark:text-amber-400",
  blocked: "text-red-600 dark:text-red-400",
};

interface Props {
  visual: ReadinessVisualModel;
  fmt: VisualFormat;
  onAsk?: (prompt: string) => void;
}

export function ReadinessVisual({ visual, fmt, onAsk }: Props) {
  const { t } = useTranslation("assistant");
  const { summary } = visual;
  const fraction = summary.total ? summary.passed / summary.total : 0;
  // The ring fills after mount; its CSS transition is stopped under reduced motion.
  const [shown, setShown] = useState(() => (typeof window === "undefined" ? fraction : 0));
  useEffect(() => {
    const frame = window.requestAnimationFrame(() => setShown(fraction));
    return () => window.cancelAnimationFrame(frame);
  }, [fraction]);
  const open = visual.checks.filter((check) => check.status !== "ok").length;
  const ringClass = summary.ready && !open ? STATUS_CLASS.ok : open && visual.checks.some((c) => c.status === "blocked") ? STATUS_CLASS.blocked : STATUS_CLASS.warning;
  return (
    <div className="space-y-3">
      <div className="kb-visual-rise flex items-center gap-4">
        <svg viewBox="0 0 64 64" className="size-16 shrink-0 -rotate-90" role="img" aria-label={t("visual.readiness.progress", { passed: summary.passed, total: summary.total })}>
          <circle cx="32" cy="32" r={RADIUS} fill="none" strokeWidth="6" className="stroke-muted" />
          <circle
            cx="32"
            cy="32"
            r={RADIUS}
            fill="none"
            strokeWidth="6"
            strokeLinecap="round"
            stroke="currentColor"
            strokeDasharray={CIRCUMFERENCE}
            strokeDashoffset={CIRCUMFERENCE * (1 - shown)}
            className={cn("kb-visual-ring", ringClass)}
          />
        </svg>
        <div className="min-w-0">
          <div className="text-base font-semibold">
            {summary.ready && !open ? t("visual.readiness.ready") : t("visual.readiness.open", { count: open })}
          </div>
          <div className="text-xs text-muted-foreground">
            {t("visual.readiness.progress", { passed: summary.passed, total: summary.total })}
          </div>
        </div>
      </div>
      <ul className="space-y-1.5">
        {visual.checks.map((check, index) => (
          <CheckRow key={`${check.check}-${index}`} check={check} index={index} fmt={fmt} onAsk={onAsk} />
        ))}
      </ul>
    </div>
  );
}

function CheckRow({
  check,
  index,
  fmt,
  onAsk,
}: {
  check: ReadinessCheck;
  index: number;
  fmt: VisualFormat;
  onAsk?: (prompt: string) => void;
}) {
  const { t } = useTranslation("assistant");
  const Icon = STATUS_ICON[check.status];
  const count = check.count ?? 0;
  const action = check.action;
  const detail =
    check.check === "blocker"
      ? check.detail
      : check.check === "journals"
        ? !check.detail
          ? check.status === "ok" ? null : t("visual.readiness.detail.journals.never")
          : check.status === "ok"
            ? t("visual.readiness.detail.journals.ok", { date: fmt.date(check.detail, "short") })
            : t("visual.readiness.detail.journals.warning", { date: fmt.date(check.detail, "short") })
        : check.check === "prices"
          ? check.status === "ok"
            ? t("visual.readiness.detail.pricesOk")
            : t("visual.readiness.detail.prices", { count, coverable: Number(check.detail ?? 0) })
          : t(`visual.readiness.detail.${check.check}.${check.status}`, { count, defaultValue: "" });
  return (
    <li
      className={cn(
        "kb-visual-rise flex items-start gap-2.5 rounded-md border px-3 py-2",
        check.status === "ok" ? "border-border/50" : check.status === "warning" ? "border-amber-500/30 bg-amber-500/5" : "border-red-500/30 bg-red-500/5",
      )}
      style={{ animationDelay: `${stagger(index, 60) + 120}ms` }}
    >
      <Icon className={cn("mt-0.5 size-4 shrink-0", STATUS_CLASS[check.status])} aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <div className="text-xs font-medium">
          {check.check === "blocker" && check.title ? check.title : t(`visual.readiness.check.${check.check}`)}
        </div>
        {detail ? <div className="text-2xs text-muted-foreground">{detail}</div> : null}
      </div>
      {action && onAsk ? (
        <button
          type="button"
          onClick={() => onAsk(t(`visual.readiness.prompt.${action}`))}
          className="inline-flex shrink-0 items-center gap-1 rounded-full border border-border/60 bg-background/60 px-2 py-0.5 text-2xs font-medium transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          <Sparkles className="size-3" aria-hidden="true" />
          {t(`visual.readiness.action.${action}`)}
        </button>
      ) : null}
    </li>
  );
}
