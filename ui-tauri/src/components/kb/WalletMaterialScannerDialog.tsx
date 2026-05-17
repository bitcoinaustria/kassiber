import * as React from "react";
import { Camera, Loader2, ScanLine, VideoOff } from "lucide-react";
import QrScanner from "qr-scanner";
import qrScannerWorkerUrl from "qr-scanner/qr-scanner-worker.min.js?url";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import {
  emptyBbqrCollectorState,
  processWalletMaterialQrScan,
  type BbqrCollectorState,
  type BbqrProgress,
  type QrScanMode,
} from "@/lib/bbqrWalletMaterial";

QrScanner.WORKER_PATH = qrScannerWorkerUrl;

interface WalletMaterialScannerDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onMaterialScanned: (material: string) => void;
  title?: string;
}

function scanModeLabel(mode: QrScanMode) {
  if (mode === "bbqr") return "BBQR";
  if (mode === "single") return "Single QR";
  return "Auto";
}

function calculateScanRegion(video: HTMLVideoElement): QrScanner.ScanRegion {
  const shortestSide = Math.min(video.videoWidth, video.videoHeight);
  const size = Math.round(shortestSide * 0.72);
  return {
    x: Math.round((video.videoWidth - size) / 2),
    y: Math.round((video.videoHeight - size) / 2),
    width: size,
    height: size,
    downScaledWidth: 400,
    downScaledHeight: 400,
  };
}

function isAbortLikeScannerError(error: Error | string) {
  if (typeof error === "string") {
    return error.toLowerCase().includes("operation was aborted");
  }
  return (
    error.name === "AbortError" ||
    error.message.toLowerCase().includes("operation was aborted")
  );
}

export function WalletMaterialScannerDialog({
  open,
  onOpenChange,
  onMaterialScanned,
  title = "Scan wallet export",
}: WalletMaterialScannerDialogProps) {
  const videoRef = React.useRef<HTMLVideoElement | null>(null);
  const scannerRef = React.useRef<QrScanner | null>(null);
  const lastScannedRef = React.useRef<string | null>(null);
  const scanModeRef = React.useRef<QrScanMode>("auto");
  const scannedMaterialRef = React.useRef<string | null>(null);
  const selectedDeviceIdRef = React.useRef("");
  const [devices, setDevices] = React.useState<QrScanner.Camera[]>([]);
  const [selectedDeviceId, setSelectedDeviceId] = React.useState("");
  const [scanMode, setScanMode] = React.useState<QrScanMode>("auto");
  const [, setCollectorState] = React.useState<BbqrCollectorState>(() =>
    emptyBbqrCollectorState(),
  );
  const [progress, setProgress] = React.useState<BbqrProgress | null>(null);
  const [status, setStatus] = React.useState("Camera is idle.");
  const [error, setError] = React.useState<string | null>(null);
  const [isStarting, setIsStarting] = React.useState(false);
  const [scannedMaterial, setScannedMaterial] = React.useState<string | null>(
    null,
  );

  const stopScanner = React.useCallback(() => {
    scannerRef.current?.stop();
    scannerRef.current?.destroy();
    scannerRef.current = null;
  }, []);

  React.useEffect(() => {
    scanModeRef.current = scanMode;
  }, [scanMode]);

  React.useEffect(() => {
    scannedMaterialRef.current = scannedMaterial;
  }, [scannedMaterial]);

  React.useEffect(() => {
    selectedDeviceIdRef.current = selectedDeviceId;
  }, [selectedDeviceId]);

  React.useEffect(() => {
    if (!open) {
      stopScanner();
      return;
    }
    setCollectorState(emptyBbqrCollectorState());
    setProgress(null);
    setError(null);
    setScannedMaterial(null);
    lastScannedRef.current = null;
    setStatus("Starting camera.");
  }, [open, stopScanner]);

  React.useEffect(() => {
    if (!open) return;
    setCollectorState(emptyBbqrCollectorState());
    setProgress(null);
    lastScannedRef.current = null;
    setStatus(
      scanMode === "bbqr"
        ? "Looking for BBQR frames."
        : scanMode === "single"
          ? "Looking for a single QR."
          : "Looking for a wallet QR.",
    );
  }, [open, scanMode]);

  React.useEffect(() => {
    if (!open || scannedMaterialRef.current) return;

    let cancelled = false;

    const start = async () => {
      setIsStarting(true);
      setError(null);
      try {
        await new Promise<void>((resolve) => {
          requestAnimationFrame(() => resolve());
        });
        if (cancelled) return;
        const video = videoRef.current;
        if (!video) {
          setStatus("Camera preview unavailable.");
          return;
        }
        stopScanner();
        const scanner = new QrScanner(
          video,
          (result) => {
            if (cancelled || scannedMaterialRef.current) return;
            const text = result.data.trim();
            if (!text || text === lastScannedRef.current) return;
            lastScannedRef.current = text;
            setCollectorState((current) => {
              const processed = processWalletMaterialQrScan(
                text,
                scanModeRef.current,
                current,
              );
              if (processed.status === "single") {
                scannerRef.current?.stop();
                setScannedMaterial(processed.material);
                setProgress(null);
                setStatus("Wallet QR scanned.");
                setError(null);
                return emptyBbqrCollectorState();
              }
              if (processed.status === "bbqr_progress") {
                setProgress(processed.progress);
                setStatus(
                  `BBQR ${processed.progress.received} / ${processed.progress.total} frames scanned.`,
                );
                setError(null);
                return processed.state;
              }
              if (processed.status === "bbqr_complete") {
                scannerRef.current?.stop();
                setScannedMaterial(processed.material);
                setProgress(processed.progress);
                setStatus(
                  `BBQR complete: ${processed.progress.received} / ${processed.progress.total} frames scanned.`,
                );
                setError(null);
                return processed.state;
              }
              if (processed.status === "ignored") {
                setStatus(processed.message);
                return current;
              }
              setError(processed.message);
              setStatus("Scan failed.");
              return current;
            });
          },
          {
            calculateScanRegion,
            maxScansPerSecond: 8,
            onDecodeError: (scanError) => {
              if (
                scanError === QrScanner.NO_QR_CODE_FOUND ||
                scannedMaterialRef.current ||
                isAbortLikeScannerError(scanError)
              ) {
                return;
              }
              setError(
                scanError instanceof Error
                  ? scanError.message
                  : String(scanError),
              );
            },
            preferredCamera: selectedDeviceIdRef.current || "environment",
            returnDetailedScanResult: true,
          },
        );
        scannerRef.current = scanner;
        await scanner.start();
        if (cancelled) {
          scanner.destroy();
          return;
        }
        const nextDevices = await QrScanner.listCameras(true);
        if (!cancelled) {
          setDevices(nextDevices);
          setSelectedDeviceId((current) => current || nextDevices[0]?.id || "");
          setStatus((current) =>
            current === "Starting camera." ? "Looking for a wallet QR." : current,
          );
        }
      } catch (startError) {
        if (
          !cancelled &&
          !(
            startError instanceof Error && isAbortLikeScannerError(startError)
          )
        ) {
          setError(
            startError instanceof Error
              ? startError.message
              : "Could not start the camera.",
          );
          setStatus("Camera unavailable.");
        }
      } finally {
        if (!cancelled) setIsStarting(false);
      }
    };

    void start();

    return () => {
      cancelled = true;
      stopScanner();
    };
  }, [open, stopScanner]);

  const useScannedMaterial = () => {
    if (!scannedMaterial) return;
    onMaterialScanned(scannedMaterial);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[640px]">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            Scan a descriptor, xpub-family export, Liquid descriptor, or BBQR
            sequence from a wallet display.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="flex flex-wrap items-center gap-2">
            {(["auto", "single", "bbqr"] as const).map((mode) => (
              <Button
                key={mode}
                type="button"
                size="sm"
                variant={scanMode === mode ? "secondary" : "outline"}
                onClick={() => {
                  setScanMode(mode);
                  setScannedMaterial(null);
                }}
              >
                {scanModeLabel(mode)}
              </Button>
            ))}
            <Badge variant="outline" className="ml-auto">
              {status}
            </Badge>
          </div>

          <div className="relative aspect-square overflow-hidden rounded-lg border bg-black">
            <video
              ref={videoRef}
              className={cn(
                "h-full w-full object-cover",
                scannedMaterial && "opacity-60",
              )}
              muted
              playsInline
            />
            <div className="pointer-events-none absolute inset-0 bg-black/20" />
            <div className="pointer-events-none absolute inset-[14%] rounded-lg border border-white/70 shadow-[0_0_0_999px_rgba(0,0,0,0.35)]">
              <span className="absolute top-0 left-0 size-10 rounded-tl-lg border-t-4 border-l-4 border-primary" />
              <span className="absolute top-0 right-0 size-10 rounded-tr-lg border-t-4 border-r-4 border-primary" />
              <span className="absolute bottom-0 left-0 size-10 rounded-bl-lg border-b-4 border-l-4 border-primary" />
              <span className="absolute right-0 bottom-0 size-10 rounded-br-lg border-r-4 border-b-4 border-primary" />
              <ScanLine className="absolute top-1/2 left-1/2 size-10 -translate-x-1/2 -translate-y-1/2 text-white/80" />
            </div>
            <div className="absolute top-3 right-3 left-3 flex items-center gap-2">
              <div className="flex items-center gap-2 rounded-md bg-black/60 px-2 py-1 text-xs text-white">
                {isStarting ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : error ? (
                  <VideoOff className="size-3.5" />
                ) : (
                  <Camera className="size-3.5" />
                )}
                <span>{scanModeLabel(scanMode)}</span>
              </div>
              {devices.length > 1 ? (
                <select
                  className="ml-auto h-8 min-w-0 rounded-md border border-white/20 bg-black/70 px-2 text-xs text-white"
                  value={selectedDeviceId}
                  onChange={(event) => {
                    const nextDeviceId = event.target.value;
                    selectedDeviceIdRef.current = nextDeviceId;
                    setSelectedDeviceId(nextDeviceId);
                    setScannedMaterial(null);
                    scannerRef.current?.setCamera(nextDeviceId).catch((cameraError) => {
                      if (
                        cameraError instanceof Error &&
                        isAbortLikeScannerError(cameraError)
                      ) {
                        return;
                      }
                      setError(
                        cameraError instanceof Error
                          ? cameraError.message
                          : "Could not switch camera.",
                      );
                    });
                  }}
                >
                  {devices.map((device, index) => (
                    <option key={device.id} value={device.id}>
                      {device.label || `Camera ${index + 1}`}
                    </option>
                  ))}
                </select>
              ) : null}
            </div>
          </div>

          {progress ? (
            <div className="space-y-2 rounded-md border bg-background p-3 text-xs">
              <div className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">
                  BBQR type {progress.fileType}
                </span>
                <span className="font-medium tabular-nums">
                  {progress.received} / {progress.total}
                </span>
              </div>
              <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full bg-primary transition-all"
                  style={{
                    width: `${Math.min(
                      100,
                      (progress.received / progress.total) * 100,
                    )}%`,
                  }}
                />
              </div>
            </div>
          ) : null}

          {scannedMaterial ? (
            <div className="rounded-md border border-emerald-500/30 bg-emerald-500/10 p-3 text-sm text-emerald-700 dark:text-emerald-300">
              Wallet material scanned. Review the pasted field after applying it.
            </div>
          ) : null}
          {error ? (
            <div className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
              {error}
            </div>
          ) : null}
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="button"
            onClick={useScannedMaterial}
            disabled={!scannedMaterial}
          >
            Use scanned text
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
