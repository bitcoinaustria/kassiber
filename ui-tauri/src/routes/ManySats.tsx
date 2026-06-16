import { Bitcoin } from "lucide-react";

import { ManySatsCalculator } from "@/components/manysats/ManySatsCalculator";

export function ManySats() {
  return (
    <main className="min-h-full bg-background">
      <div className="mx-auto flex w-full max-w-xl flex-col items-center gap-8 px-4 py-12 sm:px-6">
        <header className="flex flex-col items-center gap-3 text-center">
          <Bitcoin className="size-10 text-[#f7931a]" aria-hidden="true" />
          <h1 className="bg-gradient-to-r from-[var(--color-accent)] to-[var(--color-accent-2)] bg-clip-text font-sans text-4xl font-extrabold tracking-tight text-transparent sm:text-5xl">
            ManySats
          </h1>
          <p className="font-sans text-base italic text-muted-foreground sm:text-lg">
            Your simple Fiat to Satoshi converter
          </p>
        </header>
        <ManySatsCalculator />
      </div>
    </main>
  );
}
