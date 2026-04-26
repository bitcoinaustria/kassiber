import { BookOpen, RefreshCw, ShieldAlert } from "lucide-react";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useDaemon } from "@/daemon/client";

interface JournalEntryType {
  type: string;
  count: number;
  gainLossEur: number;
}

interface RecentJournalEntry {
  date: string;
  type: string;
  wallet: string;
  asset: string;
  quantity: number;
  fiatValueEur: number;
  gainLossEur: number;
}

interface JournalsSnapshot {
  status: {
    workspace: string | null;
    profile: string | null;
    transactionCount: number;
    journalEntryCount: number;
    needsJournals: boolean;
    quarantines: number;
    lastProcessedAt: string | null;
  };
  entryTypes: JournalEntryType[];
  recent: RecentJournalEntry[];
}

const eur = new Intl.NumberFormat("de-AT", {
  style: "currency",
  currency: "EUR",
});

export function Journals() {
  const { data, isLoading } = useDaemon<JournalsSnapshot>(
    "ui.journals.snapshot",
  );

  if (isLoading) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading journals...
      </div>
    );
  }

  if (data?.error || !data?.data) {
    return (
      <div className="w-full bg-background p-3 sm:p-4 md:p-6">
        <Card>
          <CardHeader>
            <CardTitle>Journals unavailable</CardTitle>
            <CardDescription>
              {data?.error?.message ?? "The daemon did not return journal data."}
            </CardDescription>
          </CardHeader>
        </Card>
      </div>
    );
  }

  const snapshot = data.data;
  const status = snapshot.status;

  return (
    <div className="w-full space-y-4 bg-background p-3 sm:space-y-6 sm:p-4 md:p-6">
      {status.needsJournals && (
        <Card className="border-amber-200 bg-amber-50 text-amber-950 dark:border-amber-900/60 dark:bg-amber-950/30 dark:text-amber-100">
          <CardHeader>
            <CardTitle>Journals need processing</CardTitle>
            <CardDescription className="text-amber-800 dark:text-amber-200">
              Recent transaction changes are not reflected in trusted report
              totals yet.
            </CardDescription>
          </CardHeader>
        </Card>
      )}

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <JournalMetric
          label="Transactions"
          value={status.transactionCount.toLocaleString("en-US")}
          sub="active ledger rows"
        />
        <JournalMetric
          label="Journal entries"
          value={status.journalEntryCount.toLocaleString("en-US")}
          sub="processed accounting rows"
        />
        <JournalMetric
          label="Quarantine"
          value={status.quarantines.toLocaleString("en-US")}
          sub={status.quarantines ? "needs review" : "clear"}
        />
        <JournalMetric
          label="Last processed"
          value={status.lastProcessedAt ? status.lastProcessedAt.slice(0, 10) : "never"}
          sub={status.profile ?? "local profile"}
        />
      </div>

      <div className="grid grid-cols-1 gap-4 xl:grid-cols-[360px_minmax(0,1fr)]">
        <Card>
          <CardHeader className="border-b">
            <CardTitle className="flex items-center gap-2">
              <BookOpen className="size-4" aria-hidden="true" />
              Entry types
            </CardTitle>
            <CardDescription>
              Current journal composition by accounting event type.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4">
            {snapshot.entryTypes.length ? (
              snapshot.entryTypes.map((entry) => (
                <div
                  key={entry.type}
                  className="flex items-center justify-between gap-3 rounded-md border px-3 py-2"
                >
                  <div>
                    <p className="text-sm font-medium">{entry.type}</p>
                    <p className="text-xs text-muted-foreground">
                      {entry.count.toLocaleString("en-US")} rows
                    </p>
                  </div>
                  <p className="text-sm tabular-nums">
                    {eur.format(entry.gainLossEur)}
                  </p>
                </div>
              ))
            ) : (
              <p className="text-sm text-muted-foreground">
                No journal entries have been processed yet.
              </p>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="border-b">
            <CardTitle>Recent journal entries</CardTitle>
            <CardDescription>
              Latest accounting rows produced by journal processing.
            </CardDescription>
          </CardHeader>
          <CardContent className="overflow-x-auto p-0">
            <Table>
              <TableHeader>
                <TableRow className="bg-muted/50 hover:bg-muted/50">
                  <TableHead>Date</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead>Wallet</TableHead>
                  <TableHead className="text-right">Quantity</TableHead>
                  <TableHead className="text-right">Fiat</TableHead>
                  <TableHead className="text-right">Gain/Loss</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {snapshot.recent.length ? (
                  snapshot.recent.map((entry, index) => (
                    <TableRow key={`${entry.date}-${entry.type}-${index}`}>
                      <TableCell className="font-mono text-xs text-muted-foreground">
                        {entry.date}
                      </TableCell>
                      <TableCell>{entry.type}</TableCell>
                      <TableCell>{entry.wallet}</TableCell>
                      <TableCell className="text-right tabular-nums">
                        {entry.quantity.toFixed(8)} {entry.asset}
                      </TableCell>
                      <TableCell className="text-right tabular-nums">
                        {eur.format(entry.fiatValueEur)}
                      </TableCell>
                      <TableCell className="text-right tabular-nums">
                        {eur.format(entry.gainLossEur)}
                      </TableCell>
                    </TableRow>
                  ))
                ) : (
                  <TableRow>
                    <TableCell
                      colSpan={6}
                      className="h-24 text-center text-sm text-muted-foreground"
                    >
                      No journal entries available.
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            {status.needsJournals ? (
              <RefreshCw className="size-4" aria-hidden="true" />
            ) : (
              <ShieldAlert className="size-4" aria-hidden="true" />
            )}
            Processing state
          </CardTitle>
          <CardDescription>
            Journals are the interpreted accounting layer. Reports read from
            this state.
          </CardDescription>
        </CardHeader>
      </Card>
    </div>
  );
}

function JournalMetric({
  label,
  value,
  sub,
}: {
  label: string;
  value: string;
  sub: string;
}) {
  return (
    <Card className="gap-3 py-5">
      <CardContent className="space-y-2">
        <p className="text-xs font-medium text-muted-foreground">{label}</p>
        <p className="text-2xl font-semibold tracking-tight">{value}</p>
        <p className="text-xs text-muted-foreground">{sub}</p>
      </CardContent>
    </Card>
  );
}
