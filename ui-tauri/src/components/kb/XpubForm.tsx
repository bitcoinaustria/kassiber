/**
 * XPub add-connection form — translated from
 * claude-design/screens/connections.jsx (XpubForm).
 *
 * Opened from ConnectionTypePicker when the user picks `xpub`. The
 * actual add-wallet daemon kind isn't wired yet, so the Save button
 * just calls `onSaved(payload)` and lets the parent close the modal.
 */

import { useMemo, useState } from "react";
import { ChevronDown, Check, X, ArrowLeft } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { LabeledInput } from "@/components/kb/LabeledInput";
import { cn } from "@/lib/utils";

const ADDRESS_TYPES: Array<[
  AddrType,
  string,
  string,
]> = [
  ["p2pkh", "Pay to Public Key Hash", "1A1zP1…"],
  ["p2sh", "Pay to Script Hash", "3J98t1…"],
  ["p2wpkh", "Pay to Witness Pub Hash", "bc1qar…"],
  ["p2tr", "Pay to Taproot", "bc1p5c…"],
];

type AddrType = "p2pkh" | "p2sh" | "p2wpkh" | "p2tr";

const EXAMPLE_XPUB =
  "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4ogpiMZbpiaQL2j…";

export interface XpubPayload {
  kind: "xpub";
  label: string;
  xpub: string;
  addrTypes: Record<AddrType, boolean>;
  gap: number;
}

interface XpubFormProps {
  open: boolean;
  onClose: () => void;
  onBack?: () => void;
  onSaved?: (payload: XpubPayload) => void;
}

export function XpubForm({ open, onClose, onBack, onSaved }: XpubFormProps) {
  const [label, setLabel] = useState("Cold Storage");
  const [xpub, setXpub] = useState("");
  const [addrTypes, setAddrTypes] = useState<Record<AddrType, boolean>>({
    p2pkh: false,
    p2sh: false,
    p2wpkh: true,
    p2tr: false,
  });
  const [gap, setGap] = useState(10);

  const detected = useMemo(() => {
    if (!xpub) return "—";
    if (xpub.startsWith("zpub")) return "BIP84 · native segwit";
    if (xpub.startsWith("ypub")) return "BIP49 · nested";
    return "BIP44";
  }, [xpub]);
  const fingerprint = xpub ? "5f3a · 8c0e" : "—";

  const submit = () => {
    if (!xpub.trim()) return;
    onSaved?.({
      kind: "xpub",
      label: label.trim() || "Cold Storage",
      xpub: xpub.trim(),
      addrTypes,
      gap,
    });
    onClose();
  };

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        showCloseButton={false}
        className="max-w-[620px] gap-0 rounded-none border border-ink bg-paper p-0 shadow-hard-ink"
      >
        <DialogHeader className="flex-row items-center gap-2 border-b border-line px-5 py-4">
          {onBack && (
            <button
              onClick={onBack}
              aria-label="Back"
              className="cursor-pointer border border-line bg-transparent p-1.5"
            >
              <ArrowLeft className="size-3 text-ink-2" />
            </button>
          )}
          <DialogTitle className="font-sans text-lg font-semibold text-ink">
            XPub
          </DialogTitle>
          <span className="flex-1" />
          <button
            onClick={onClose}
            aria-label="Close"
            className="cursor-pointer border border-line bg-transparent p-1.5"
          >
            <X className="size-3 text-ink-2" />
          </button>
        </DialogHeader>
        <DialogDescription className="px-5 pt-4 font-sans text-[13px] text-ink-2">
          Enter your extended public key. Kassiber will derive addresses and
          sync on-chain history.
        </DialogDescription>

        <div className="flex flex-col gap-4 px-5 py-4">
          <LabeledInput
            label="Connection label"
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder="e.g. Cold Storage"
          />

          <div>
            <LabeledInput
              label="xpub / ypub / zpub"
              value={xpub}
              onChange={(e) => setXpub(e.target.value)}
              placeholder={EXAMPLE_XPUB}
              mono
            />
            <div className="mt-1.5 flex gap-4 font-mono text-[10px] text-ink-3">
              <span>
                Detected: <span className="text-ink-2">{detected}</span>
              </span>
              <span>
                Fingerprint: <span className="text-ink-2">{fingerprint}</span>
              </span>
            </div>
          </div>

          <div>
            <div className="mb-2 font-sans text-[10px] font-semibold uppercase tracking-[0.12em] text-ink-2">
              Address types to derive
            </div>
            <div className="grid grid-cols-2 gap-1.5">
              {ADDRESS_TYPES.map(([k, name, prefix]) => (
                <label
                  key={k}
                  className={cn(
                    "flex cursor-pointer items-center gap-2.5 border border-line px-2.5 py-2",
                    addrTypes[k] && "bg-paper-2",
                  )}
                >
                  <input
                    type="checkbox"
                    checked={addrTypes[k]}
                    onChange={() =>
                      setAddrTypes((a) => ({ ...a, [k]: !a[k] }))
                    }
                    className="accent-accent"
                  />
                  <span className="flex-1 font-sans text-xs text-ink">
                    {name}
                  </span>
                  <span className="font-mono text-[10px] text-ink-3">
                    {prefix}
                  </span>
                </label>
              ))}
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <LabeledInput
              label="Gap limit"
              value={String(gap)}
              onChange={(e) =>
                setGap(Number.parseInt(e.target.value, 10) || 0)
              }
              type="number"
              mono
            />
            <div>
              <div className="mb-1.5 font-sans text-[10px] font-semibold uppercase tracking-[0.12em] text-ink-2">
                Sync backend
              </div>
              <div className="flex items-center justify-between border border-line bg-paper-2 px-2.5 py-2 font-sans text-xs text-ink">
                Mempool.space (default)
                <ChevronDown className="size-2.5 text-ink-3" />
              </div>
            </div>
          </div>
        </div>

        <div className="flex justify-end gap-2.5 border-t border-line px-5 py-3">
          <Button
            variant="ghost"
            onClick={onClose}
            size="sm"
            className="rounded-none"
          >
            Cancel
          </Button>
          <Button
            onClick={submit}
            disabled={!xpub.trim()}
            size="sm"
            className="rounded-none"
          >
            <Check className="size-3" /> Save and sync
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
