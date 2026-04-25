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
    <footer className="relative flex h-7 flex-shrink-0 items-center justify-between border-t border-line bg-paper px-4.5 font-mono text-[10px] tracking-[0.05em] text-ink-3">
      <div className="flex items-center gap-4.5">
        <span>KASSIBER v0.0.0</span>
        <span className="flex items-center gap-1.5">
          <span className="size-1.5 rounded-full bg-[#3fa66a]" />
          WATCH-ONLY · LOCAL ENCRYPTED VAULT
        </span>
      </div>

      <a
        href="#donate"
        className="absolute left-1/2 top-0 inline-flex h-full -translate-x-1/2 items-center gap-1.5 border-x border-line bg-paper-2 px-3.5 font-mono text-[10px] uppercase tracking-[0.1em] text-accent no-underline"
      >
        <Heart className="size-2.5 fill-accent/20 stroke-accent" />
        DONATE SATS
      </a>

      <div className="flex items-center gap-3.5">
        <span
          title={`Updated ${sinceLabel} · source: CoinGecko`}
          className="inline-flex items-center gap-1.5"
        >
          <span className="text-ink-3">BTC/EUR</span>
          <span className="font-semibold text-ink">
            €
            {price.toLocaleString("de-AT", {
              minimumFractionDigits: 2,
              maximumFractionDigits: 2,
            })}
          </span>
          <span className="text-ink-3">· COINGECKO</span>
          <button
            onClick={refresh}
            title="Refresh rate"
            aria-label="Refresh BTC/EUR rate"
            className="ml-px inline-flex size-4 cursor-pointer items-center justify-center border border-line bg-transparent p-0"
          >
            <RefreshCw
              className={cn("size-2.5 text-ink-2", refreshing && "animate-spin")}
            />
          </button>
        </span>
        <span>MAINNET</span>
        <a
          href="https://github.com/bitcoinaustria/kassiber"
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1.5 text-ink-3 no-underline"
        >
          <Github className="size-2.5" />
          GITHUB
        </a>
      </div>
    </footer>
  );
}
