/**
 * XPub add-connection form — translated from
 * claude-design/screens/connections.jsx (XpubForm).
 *
 * Opened from ConnectionTypePicker when the user picks `xpub`. The
 * actual add-wallet daemon kind isn't wired yet, so the Save button
 * just calls `onSaved(payload)` and lets the parent close the modal.
 */

import { useMemo, useState } from "react";
import { ArrowLeft, Check, X } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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
        className="max-w-[680px] gap-0 overflow-hidden rounded-lg border bg-background p-0 shadow-xl"
      >
        <DialogHeader className="flex-row items-center gap-3 border-b px-6 py-5">
          {onBack && (
            <Button
              type="button"
              variant="outline"
              size="icon"
              onClick={onBack}
              aria-label="Back"
            >
              <ArrowLeft className="size-4" />
            </Button>
          )}
          <div className="min-w-0 flex-1">
            <DialogTitle className="text-2xl font-semibold tracking-tight">
              XPub connection
            </DialogTitle>
            <DialogDescription className="mt-1 text-sm text-muted-foreground">
              Enter an extended public key. Kassiber derives addresses and
              syncs on-chain history without private keys.
            </DialogDescription>
          </div>
          <Button
            type="button"
            variant="outline"
            size="icon"
            onClick={onClose}
            aria-label="Close"
          >
            <X className="size-4" />
          </Button>
        </DialogHeader>

        <div className="max-h-[68vh] space-y-5 overflow-y-auto px-6 py-5">
          <div className="grid gap-2">
            <Label htmlFor="xpub-label">Connection label</Label>
            <Input
              id="xpub-label"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="e.g. Cold Storage"
            />
          </div>

          <div className="grid gap-2">
            <Label htmlFor="xpub-key">XPUB / YPUB / ZPUB</Label>
            <Input
              id="xpub-key"
              value={xpub}
              onChange={(e) => setXpub(e.target.value)}
              placeholder={EXAMPLE_XPUB}
              className="font-mono text-xs"
            />
            <div className="flex flex-wrap gap-2 pt-1">
              <Badge variant="secondary">Detected: {detected}</Badge>
              <Badge variant="outline">Fingerprint: {fingerprint}</Badge>
            </div>
          </div>

          <section className="grid gap-3">
            <Label>
              Address types to derive
            </Label>
            <div className="grid gap-2 sm:grid-cols-2">
              {ADDRESS_TYPES.map(([k, name, prefix]) => (
                <Card
                  key={k}
                  className={cn(
                    "cursor-pointer transition-colors hover:bg-muted/40",
                    addrTypes[k] && "border-primary bg-primary/5",
                  )}
                  onClick={() =>
                    setAddrTypes((a) => ({ ...a, [k]: !a[k] }))
                  }
                >
                  <CardContent className="flex items-center gap-3 p-3">
                    <Checkbox
                      checked={addrTypes[k]}
                      onCheckedChange={(checked) =>
                        setAddrTypes((a) => ({ ...a, [k]: checked === true }))
                      }
                      onClick={(event) => event.stopPropagation()}
                      aria-label={name}
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block text-sm font-medium">{name}</span>
                      <span className="block font-mono text-xs text-muted-foreground">
                        {prefix}
                      </span>
                    </span>
                  </CardContent>
                </Card>
              ))}
            </div>
          </section>

          <div className="grid grid-cols-2 gap-4">
            <div className="grid gap-2">
              <Label htmlFor="xpub-gap">Gap limit</Label>
              <Input
                id="xpub-gap"
                value={String(gap)}
                onChange={(e) =>
                  setGap(Number.parseInt(e.target.value, 10) || 0)
                }
                type="number"
                className="font-mono"
              />
            </div>
            <div className="grid gap-2">
              <Label>Sync backend</Label>
              <Select value="mempool-space">
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="mempool-space">
                    Mempool.space (default)
                  </SelectItem>
                  <SelectItem value="electrum" disabled>
                    Electrum (soon)
                  </SelectItem>
                  <SelectItem value="bitcoin-core" disabled>
                    Bitcoin Core RPC (soon)
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
        </div>

        <div className="flex justify-end gap-2.5 border-t bg-muted/30 px-6 py-4">
          <Button
            variant="ghost"
            onClick={onClose}
            size="sm"
          >
            Cancel
          </Button>
          <Button
            onClick={submit}
            disabled={!xpub.trim()}
            size="sm"
            className="gap-2"
          >
            <Check className="size-4" /> Save and sync
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
