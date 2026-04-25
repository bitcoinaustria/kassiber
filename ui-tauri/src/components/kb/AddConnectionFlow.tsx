/**
 * Add-connection flow — orchestrates the picker → per-kind form
 * sequence as a single open/close boolean.
 *
 * Picking `xpub` from the picker opens the XPub form. Other kinds
 * surface a "coming soon" panel inside the picker dialog (still TBD)
 * since their per-kind forms aren't translated yet — for now picking
 * a non-xpub kind closes the flow.
 */

import { useState } from "react";
import {
  ConnectionTypePicker,
  type ConnectionKindKey,
} from "./ConnectionTypePicker";
import { XpubForm, type XpubPayload } from "./XpubForm";

interface AddConnectionFlowProps {
  open: boolean;
  onClose: () => void;
  onSaved?: (payload: XpubPayload) => void;
}

export function AddConnectionFlow({
  open,
  onClose,
  onSaved,
}: AddConnectionFlowProps) {
  const [stage, setStage] = useState<"picker" | ConnectionKindKey>("picker");

  const reset = () => {
    setStage("picker");
    onClose();
  };

  return (
    <>
      <ConnectionTypePicker
        open={open && stage === "picker"}
        onClose={reset}
        onPick={(kind) => {
          if (kind === "xpub") {
            setStage("xpub");
          } else {
            // Other kinds aren't translated yet — close until they land.
            reset();
          }
        }}
      />
      <XpubForm
        open={open && stage === "xpub"}
        onClose={reset}
        onBack={() => setStage("picker")}
        onSaved={(payload) => {
          onSaved?.(payload);
          reset();
        }}
      />
    </>
  );
}
