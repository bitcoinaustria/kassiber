import { Bitcoin } from "lucide-react";

import { ManySatsCalculator } from "@/components/manysats/ManySatsCalculator";

export function ManySats() {
  return (
    <main className="min-h-full bg-background">
      <div className="mx-auto flex w-full max-w-2xl flex-col items-center gap-7 px-4 py-10 sm:px-6">
        <header className="flex flex-col items-center gap-2 text-center">
          <div className="flex items-center gap-2.5">
            <Bitcoin className="size-9 text-[#f7931a]" aria-hidden="true" />
            <h1 className="font-sans text-3xl font-bold tracking-tight text-foreground">
              ManySats
            </h1>
          </div>
          <p className="font-sans text-lg italic text-[var(--color-accent)]">
            Your simple Fiat to Satoshi converter
          </p>
        </header>
        <ManySatsCalculator />
      </div>
    </main>
  );
}
