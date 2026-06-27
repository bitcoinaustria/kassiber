import { ManySatsCalculator } from "@/components/manysats/ManySatsCalculator";

export function ManySats() {
  return (
    <main className="min-h-full bg-background">
      <div className="mx-auto flex w-full max-w-xl flex-col items-center gap-6 px-4 py-12 sm:px-6">
        <p className="text-center font-sans text-base italic text-muted-foreground sm:text-lg">
          Your simple Fiat to Satoshi converter
        </p>
        <ManySatsCalculator />
      </div>
    </main>
  );
}
