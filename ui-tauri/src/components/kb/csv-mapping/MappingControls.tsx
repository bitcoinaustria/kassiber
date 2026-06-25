/**
 * Left pane of the CSV mapping workbench: the editable mapping controls,
 * grouped into Card sections (file & parsing, date, amount, fee, identity &
 * fields, pricing, filters). Pure presentation over a `DraftSpec` — all state
 * lives in the parent route.
 */
import { Trans, useTranslation } from "react-i18next";
import { Plus, X } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";

import {
  AMOUNT_MODES,
  FIELD_TARGETS,
  FILTER_OPS,
  UNITS,
  type AmountMode,
  type DraftSpec,
  type FieldDraft,
  type FilterOp,
  type Unit,
} from "./spec";

const NONE = "__none__";

type SetDraft = (updater: (draft: DraftSpec) => DraftSpec) => void;

interface ControlsProps {
  draft: DraftSpec;
  setDraft: SetDraft;
  headers: string[];
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="space-y-1.5">
      <Label className="text-xs font-medium text-muted-foreground">{label}</Label>
      {children}
      {hint ? <p className="text-[11px] leading-snug text-muted-foreground">{hint}</p> : null}
    </div>
  );
}

function ColumnSelect({
  value,
  onChange,
  headers,
  placeholder,
  allowNone = true,
  noneLabel,
}: {
  value: string;
  onChange: (value: string) => void;
  headers: string[];
  placeholder: string;
  allowNone?: boolean;
  noneLabel?: string;
}) {
  return (
    <Select
      value={value === "" ? (allowNone ? NONE : "") : value}
      onValueChange={(next) => onChange(next === NONE ? "" : next)}
    >
      <SelectTrigger className="w-full">
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        {allowNone ? <SelectItem value={NONE}>{noneLabel ?? placeholder}</SelectItem> : null}
        {headers.map((header) => (
          <SelectItem key={header} value={header}>
            {header}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

export function MappingControls({ draft, setDraft, headers }: ControlsProps) {
  const { t } = useTranslation("csvMapping");

  const patchAmount = (patch: Partial<DraftSpec["amount"]>) =>
    setDraft((d) => ({ ...d, amount: { ...d.amount, ...patch } }));
  const patchDirection = (patch: Partial<DraftSpec["amount"]["direction"]>) =>
    setDraft((d) => ({ ...d, amount: { ...d.amount, direction: { ...d.amount.direction, ...patch } } }));

  const unitOptions = (
    <>
      {UNITS.map((unit) => (
        <SelectItem key={unit} value={unit}>
          {t(`unit.${unit}` as never)}
        </SelectItem>
      ))}
    </>
  );

  return (
    <div className="space-y-4">
      {/* File & parsing */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">{t("parsing.heading")}</CardTitle>
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-3">
          <Field label={t("parsing.delimiter")}>
            <Select
              value={draft.delimiter === "" ? NONE : draft.delimiter}
              onValueChange={(v) => setDraft((d) => ({ ...d, delimiter: v === NONE ? "" : v }))}
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE}>{t("parsing.delimiterAuto")}</SelectItem>
                <SelectItem value=",">,</SelectItem>
                <SelectItem value=";">;</SelectItem>
                <SelectItem value={"\t"}>\t</SelectItem>
                <SelectItem value="|">|</SelectItem>
              </SelectContent>
            </Select>
          </Field>
          <Field label={t("parsing.encoding")}>
            <Select value={draft.encoding} onValueChange={(v) => setDraft((d) => ({ ...d, encoding: v }))}>
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="utf-8-sig">UTF-8</SelectItem>
                <SelectItem value="latin-1">Latin-1</SelectItem>
                <SelectItem value="utf-16">UTF-16</SelectItem>
              </SelectContent>
            </Select>
          </Field>
          <Field label={t("parsing.skipRows")} hint={t("parsing.skipRowsHint")}>
            <Input
              type="number"
              min={0}
              value={draft.skipRows}
              onChange={(e) =>
                setDraft((d) => ({ ...d, skipRows: Math.max(0, Number(e.target.value) || 0) }))
              }
            />
          </Field>
        </CardContent>
      </Card>

      {/* Date & time */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">{t("timestamp.heading")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <Field label={t("timestamp.column")}>
            <ColumnSelect
              value={draft.timestampColumn}
              onChange={(v) => setDraft((d) => ({ ...d, timestampColumn: v }))}
              headers={headers}
              placeholder={t("field.notMapped")}
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label={t("timestamp.format")} hint={t("timestamp.formatHint")}>
              <Input
                placeholder={t("timestamp.formatAuto")}
                value={draft.timestampFormat}
                onChange={(e) => setDraft((d) => ({ ...d, timestampFormat: e.target.value }))}
              />
            </Field>
            <Field label={t("timestamp.timezone")}>
              <Input
                value={draft.timezone}
                onChange={(e) => setDraft((d) => ({ ...d, timezone: e.target.value }))}
              />
            </Field>
          </div>
        </CardContent>
      </Card>

      {/* Amount */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">{t("amount.heading")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <Tabs value={draft.amount.mode} onValueChange={(v) => patchAmount({ mode: v as AmountMode })}>
            <TabsList className="grid w-full grid-cols-3">
              {AMOUNT_MODES.map((mode) => (
                <TabsTrigger key={mode} value={mode} className="text-xs">
                  {t(`amount.mode${mode.charAt(0).toUpperCase()}${mode.slice(1)}` as never)}
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>

          {draft.amount.mode === "signed" ? (
            <Field label={t("amount.column")}>
              <ColumnSelect
                value={draft.amount.column}
                onChange={(v) => patchAmount({ column: v })}
                headers={headers}
                placeholder={t("field.notMapped")}
              />
            </Field>
          ) : null}

          {draft.amount.mode === "split" ? (
            <div className="grid grid-cols-2 gap-3">
              <Field label={t("amount.inboundColumn")}>
                <ColumnSelect
                  value={draft.amount.inboundColumn}
                  onChange={(v) => patchAmount({ inboundColumn: v })}
                  headers={headers}
                  placeholder={t("field.notMapped")}
                />
              </Field>
              <Field label={t("amount.outboundColumn")}>
                <ColumnSelect
                  value={draft.amount.outboundColumn}
                  onChange={(v) => patchAmount({ outboundColumn: v })}
                  headers={headers}
                  placeholder={t("field.notMapped")}
                />
              </Field>
            </div>
          ) : null}

          {draft.amount.mode === "absolute" ? (
            <div className="space-y-3">
              <Field label={t("amount.column")}>
                <ColumnSelect
                  value={draft.amount.column}
                  onChange={(v) => patchAmount({ column: v })}
                  headers={headers}
                  placeholder={t("field.notMapped")}
                />
              </Field>
              <Tabs
                value={draft.amount.direction.mode}
                onValueChange={(v) => patchDirection({ mode: v as "const" | "column" })}
              >
                <TabsList className="grid w-full grid-cols-2">
                  <TabsTrigger value="column" className="text-xs">
                    {t("amount.directionColumn")}
                  </TabsTrigger>
                  <TabsTrigger value="const" className="text-xs">
                    {t("amount.directionConst")}
                  </TabsTrigger>
                </TabsList>
              </Tabs>
              {draft.amount.direction.mode === "const" ? (
                <Field label={t("amount.direction")}>
                  <Select
                    value={draft.amount.direction.const}
                    onValueChange={(v) => patchDirection({ const: v as "inbound" | "outbound" })}
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="inbound">{t("direction.inbound")}</SelectItem>
                      <SelectItem value="outbound">{t("direction.outbound")}</SelectItem>
                    </SelectContent>
                  </Select>
                </Field>
              ) : (
                <div className="space-y-3">
                  <Field label={t("amount.directionColumnLabel")}>
                    <ColumnSelect
                      value={draft.amount.direction.column}
                      onChange={(v) => patchDirection({ column: v })}
                      headers={headers}
                      placeholder={t("field.notMapped")}
                    />
                  </Field>
                  <div className="grid grid-cols-2 gap-3">
                    <Field label={t("amount.inboundValues")} hint={t("amount.valuesHint")}>
                      <Input
                        value={draft.amount.direction.inboundValues}
                        onChange={(e) => patchDirection({ inboundValues: e.target.value })}
                      />
                    </Field>
                    <Field label={t("amount.outboundValues")} hint={t("amount.valuesHint")}>
                      <Input
                        value={draft.amount.direction.outboundValues}
                        onChange={(e) => patchDirection({ outboundValues: e.target.value })}
                      />
                    </Field>
                  </div>
                  <Field label={t("amount.default")}>
                    <Select
                      value={draft.amount.direction.default === "" ? NONE : draft.amount.direction.default}
                      onValueChange={(v) =>
                        patchDirection({ default: v === NONE ? "" : (v as "inbound" | "outbound") })
                      }
                    >
                      <SelectTrigger className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value={NONE}>{t("amount.defaultNone")}</SelectItem>
                        <SelectItem value="inbound">{t("direction.inbound")}</SelectItem>
                        <SelectItem value="outbound">{t("direction.outbound")}</SelectItem>
                      </SelectContent>
                    </Select>
                  </Field>
                </div>
              )}
            </div>
          ) : null}

          <div className="grid grid-cols-2 gap-3">
            <Field label={t("amount.unit")}>
              <Select value={draft.amount.unit} onValueChange={(v) => patchAmount({ unit: v as Unit })}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>{unitOptions}</SelectContent>
              </Select>
            </Field>
            <Field label={t("amount.decimalSeparator")}>
              <Select
                value={draft.amount.decimalSeparator}
                onValueChange={(v) => patchAmount({ decimalSeparator: v as "." | "," })}
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value=".">{t("decimal.dot")}</SelectItem>
                  <SelectItem value=",">{t("decimal.comma")}</SelectItem>
                </SelectContent>
              </Select>
            </Field>
          </div>
        </CardContent>
      </Card>

      {/* Fee */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">{t("fee.heading")}</CardTitle>
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-3">
          <Field label={t("fee.column")}>
            <ColumnSelect
              value={draft.fee.column}
              onChange={(v) => setDraft((d) => ({ ...d, fee: { ...d.fee, column: v } }))}
              headers={headers}
              placeholder={t("fee.none")}
              noneLabel={t("fee.none")}
            />
          </Field>
          <Field label={t("fee.unit")}>
            <Select
              value={draft.fee.unit}
              onValueChange={(v) => setDraft((d) => ({ ...d, fee: { ...d.fee, unit: v as Unit } }))}
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>{unitOptions}</SelectContent>
            </Select>
          </Field>
        </CardContent>
      </Card>

      {/* Identity & fields */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">{t("identity.heading")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <Field label={t("identity.txid")} hint={t("identity.txidHint")}>
            <ColumnSelect
              value={draft.txidColumn}
              onChange={(v) => setDraft((d) => ({ ...d, txidColumn: v }))}
              headers={headers}
              placeholder={t("field.notMapped")}
            />
          </Field>
          {FIELD_TARGETS.map((target) => (
            <FieldSourceRow
              key={target}
              label={t(`field.${target}` as never)}
              value={draft.fields[target]}
              headers={headers}
              onChange={(next) =>
                setDraft((d) => ({ ...d, fields: { ...d.fields, [target]: next } }))
              }
            />
          ))}
        </CardContent>
      </Card>

      {/* Pricing */}
      <Card>
        <Collapsible open={draft.pricing.enabled}>
          <CardHeader className="flex-row items-center justify-between space-y-0 pb-3">
            <CardTitle className="text-sm">{t("pricing.heading")}</CardTitle>
            <CollapsibleTrigger asChild>
              <span className="flex items-center gap-2">
                <Label className="text-xs text-muted-foreground">{t("pricing.enable")}</Label>
                <Switch
                  checked={draft.pricing.enabled}
                  onCheckedChange={(checked) =>
                    setDraft((d) => ({ ...d, pricing: { ...d.pricing, enabled: checked } }))
                  }
                />
              </span>
            </CollapsibleTrigger>
          </CardHeader>
          <CollapsibleContent>
            <CardContent className="space-y-3">
              <FieldSourceRow
                label={t("pricing.currency")}
                value={draft.pricing.currency}
                headers={headers}
                onChange={(next) =>
                  setDraft((d) => ({ ...d, pricing: { ...d.pricing, currency: next } }))
                }
              />
              <FieldSourceRow
                label={t("pricing.rate")}
                value={draft.pricing.rate}
                headers={headers}
                onChange={(next) =>
                  setDraft((d) => ({ ...d, pricing: { ...d.pricing, rate: next } }))
                }
              />
              <FieldSourceRow
                label={t("pricing.value")}
                value={draft.pricing.value}
                headers={headers}
                onChange={(next) =>
                  setDraft((d) => ({ ...d, pricing: { ...d.pricing, value: next } }))
                }
              />
            </CardContent>
          </CollapsibleContent>
        </Collapsible>
      </Card>

      {/* Filters */}
      <Card>
        <CardHeader className="flex-row items-center justify-between space-y-0 pb-3">
          <CardTitle className="text-sm">{t("filters.heading")}</CardTitle>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() =>
              setDraft((d) => ({
                ...d,
                filters: [...d.filters, { column: "", op: "equals", value: "" }],
              }))
            }
          >
            <Plus className="size-3.5" aria-hidden="true" />
            {t("filters.add")}
          </Button>
        </CardHeader>
        <CardContent className="space-y-2">
          <p className="text-[11px] leading-snug text-muted-foreground">{t("filters.hint")}</p>
          {draft.filters.length === 0 ? (
            <p className="text-xs text-muted-foreground">{t("filters.empty")}</p>
          ) : null}
          {draft.filters.map((filter, index) => (
            <div key={index} className="flex items-center gap-2">
              <div className="flex-1">
                <ColumnSelect
                  value={filter.column}
                  onChange={(v) =>
                    setDraft((d) => ({
                      ...d,
                      filters: d.filters.map((f, i) => (i === index ? { ...f, column: v } : f)),
                    }))
                  }
                  headers={headers}
                  placeholder={t("field.notMapped")}
                />
              </div>
              <Select
                value={filter.op}
                onValueChange={(v) =>
                  setDraft((d) => ({
                    ...d,
                    filters: d.filters.map((f, i) => (i === index ? { ...f, op: v as FilterOp } : f)),
                  }))
                }
              >
                <SelectTrigger className="w-[130px] shrink-0">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {FILTER_OPS.map((op) => (
                    <SelectItem key={op} value={op}>
                      {t(`filters.op${op.charAt(0).toUpperCase()}${op.slice(1).replace(/_([a-z])/g, (_, c) => c.toUpperCase())}` as never)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {filter.op !== "not_empty" ? (
                <Input
                  className="w-[150px] shrink-0"
                  placeholder={
                    filter.op === "in" ? t("filters.valueInPlaceholder") : t("filters.valuePlaceholder")
                  }
                  value={filter.value}
                  onChange={(e) =>
                    setDraft((d) => ({
                      ...d,
                      filters: d.filters.map((f, i) => (i === index ? { ...f, value: e.target.value } : f)),
                    }))
                  }
                />
              ) : null}
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={t("filters.remove")}
                onClick={() =>
                  setDraft((d) => ({ ...d, filters: d.filters.filter((_, i) => i !== index) }))
                }
              >
                <X className="size-4" aria-hidden="true" />
              </Button>
            </div>
          ))}
        </CardContent>
      </Card>

      {headers.length === 0 ? (
        <Badge variant="outline" className="text-muted-foreground">
          <Trans i18nKey="file.none" ns="csvMapping" />
        </Badge>
      ) : null}
    </div>
  );
}

function FieldSourceRow({
  label,
  value,
  headers,
  onChange,
}: {
  label: string;
  value: FieldDraft;
  headers: string[];
  onChange: (next: FieldDraft) => void;
}) {
  const { t } = useTranslation("csvMapping");
  const mode = value.mode;
  return (
    <div className="space-y-1.5">
      <Label className="text-xs font-medium text-muted-foreground">{label}</Label>
      <div className="flex items-center gap-2">
        <Select
          value={mode}
          onValueChange={(v) => onChange({ ...value, mode: v as FieldDraft["mode"] })}
        >
          <SelectTrigger className="w-[140px] shrink-0">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="none">{t("field.notMapped")}</SelectItem>
            <SelectItem value="column">{t("field.fromColumn")}</SelectItem>
            <SelectItem value="const">{t("field.fixedValue")}</SelectItem>
          </SelectContent>
        </Select>
        {mode === "column" ? (
          <div className="flex-1">
            <ColumnSelect
              value={value.column}
              onChange={(v) => onChange({ ...value, column: v })}
              headers={headers}
              placeholder={t("field.notMapped")}
              allowNone={false}
            />
          </div>
        ) : null}
        {mode === "const" ? (
          <Input
            className="flex-1"
            placeholder={t("field.constPlaceholder")}
            value={value.const}
            onChange={(e) => onChange({ ...value, const: e.target.value })}
          />
        ) : null}
      </div>
    </div>
  );
}
