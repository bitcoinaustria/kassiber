/**
 * AppFooter — version + watch-only badge, donate link, BTC/EUR rate,
 * mainnet pill, GitHub link.
 *
 * Translated from claude-design/components/chrome.jsx. The price ticker
 * cycles through fixture values until the rates daemon kind is wired
 * in. The donate target is a placeholder; GitHub link points to the
 * project page.
 */

import { useEffect, useState } from "react";
import { RefreshCw, Github, Heart } from "lucide-react";
import { cn } from "@/lib/utils";

const PRICE_SAMPLES = [
  71_420.18, 71_452.03, 71_398.77, 71_510.44, 71_488.91, 71_463.20,
];

const REFRESH_MS = 60_000;

export function AppFooter() {
  const [priceIdx, setPriceIdx] = useState(0);
  const [updated, setUpdated] = useState(() => Date.now());
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    const id = setInterval(() => {
      setPriceIdx((i) => (i + 1) % PRICE_SAMPLES.length);
      setUpdated(Date.now());
    }, REFRESH_MS);
    return () => clearInterval(id);
  }, []);

  const refresh = () => {
    setRefreshing(true);
    setTimeout(() => {
      setPriceIdx((i) => (i + 1) % PRICE_SAMPLES.length);
      setUpdated(Date.now());
      setRefreshing(false);
    }, 650);
  };

  const price = PRICE_SAMPLES[priceIdx];
  const since = Math.floor((Date.now() - updated) / 1000);
  const sinceLabel =
    since < 5 ? "just now" : since < 60 ? `${since}s ago` : `${Math.floor(since / 60)}m ago`;

  return (
    <footer className="relative flex h-7 flex-shrink-0 items-center justify-between gap-2 border-t border-line bg-paper px-3 font-mono text-[10px] tracking-[0.05em] text-ink-3 sm:px-4.5">
      <div className="flex min-w-0 items-center gap-2 sm:gap-4.5">
        <span className="shrink-0">KASSIBER v0.21.0</span>
        <span className="hidden items-center gap-1.5 md:flex">
          <span className="size-1.5 shrink-0 rounded-full bg-[#3fa66a]" />
          WATCH-ONLY · LOCAL ENCRYPTED VAULT
        </span>
      </div>

      <a
        href="#donate"
        className="absolute left-1/2 top-0 hidden h-full -translate-x-1/2 items-center gap-1.5 whitespace-nowrap border-x border-line bg-paper-2 px-3.5 font-mono text-[10px] uppercase tracking-[0.1em] text-accent no-underline lg:inline-flex"
      >
        <Heart className="size-2.5 fill-accent/20 stroke-accent" />
        DONATE SATS
      </a>

      <div className="flex min-w-0 items-center gap-2 sm:gap-3.5">
        <span
          title={`Updated ${sinceLabel} · source: CoinGecko`}
          className="inline-flex min-w-0 items-center gap-1.5"
        >
          <span className="hidden text-ink-3 sm:inline">BTC/EUR</span>
          <span className="truncate font-semibold text-ink">
            €
            {price.toLocaleString("de-AT", {
              minimumFractionDigits: 2,
              maximumFractionDigits: 2,
            })}
          </span>
          <span className="hidden text-ink-3 md:inline">· COINGECKO</span>
          <button
            onClick={refresh}
            title="Refresh rate"
            aria-label="Refresh BTC/EUR rate"
            className="ml-px inline-flex size-4 shrink-0 cursor-pointer items-center justify-center border border-line bg-transparent p-0"
          >
            <RefreshCw
              className={cn("size-2.5 text-ink-2", refreshing && "animate-spin")}
            />
          </button>
        </span>
        <span className="hidden sm:inline">MAINNET</span>
        <a
          href="https://github.com/bitcoinaustria/kassiber"
          target="_blank"
          rel="noreferrer"
          className="hidden items-center gap-1.5 text-ink-3 no-underline sm:inline-flex"
        >
          <Github className="size-2.5" />
          GITHUB
        </a>
      </div>
    </footer>
  );
}
