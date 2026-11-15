/**
 * What every row on the Transfers screen offers besides its own answers: a
 * way to each side's transaction on the Transactions page, and, for two
 * sides that count on their own, the place to say what each one really was.
 *
 * Saying what a transaction was is decided on its own page, where the core
 * previews the change; nothing here books a payment or an income.
 */
import type * as React from "react";
import { Link } from "@tanstack/react-router";
import { ArrowUpRight, MoreHorizontal, Tag } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

/** Menu items that open the send and the receipt on Transactions. */
export function TransactionsMenuItems({
  outId,
  inId,
  sayWhat = false,
}: {
  outId: string;
  inId: string;
  /** Also offer to say what each side was: for two sides that are not linked. */
  sayWhat?: boolean;
}) {
  const { t } = useTranslation("review");
  return (
    <>
      <DropdownMenuItem asChild>
        <Link to="/transactions" search={{ tx: outId }}>
          <ArrowUpRight className="size-4" aria-hidden="true" />
          {t("transfersPage.rowMenu.showSent")}
        </Link>
      </DropdownMenuItem>
      <DropdownMenuItem asChild>
        <Link to="/transactions" search={{ tx: inId }}>
          <ArrowUpRight className="size-4" aria-hidden="true" />
          {t("transfersPage.rowMenu.showReceived")}
        </Link>
      </DropdownMenuItem>
      {sayWhat ? (
        <>
          <DropdownMenuSeparator />
          <DropdownMenuItem asChild>
            <Link to="/transactions" search={{ tx: outId, tab: "tax" }}>
              <Tag className="size-4" aria-hidden="true" />
              {t("transfersPage.rowMenu.saySent")}
            </Link>
          </DropdownMenuItem>
          <DropdownMenuItem asChild>
            <Link to="/transactions" search={{ tx: inId, tab: "tax" }}>
              <Tag className="size-4" aria-hidden="true" />
              {t("transfersPage.rowMenu.sayReceived")}
            </Link>
          </DropdownMenuItem>
        </>
      ) : null}
    </>
  );
}

/** A row's "more" menu: its own items first, then the ways to its transactions. */
export function RowMenu({
  route,
  outId,
  inId,
  sayWhat = false,
  children,
}: {
  /** "A to B", for the button's name. */
  route: string;
  outId: string;
  inId: string;
  sayWhat?: boolean;
  children?: React.ReactNode;
}) {
  const { t } = useTranslation("review");
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button type="button" variant="ghost" size="icon" className="size-7 shrink-0" aria-label={t("transfersPage.rowMenu.label", { route })}>
          <MoreHorizontal className="size-4" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {children}
        {children ? <DropdownMenuSeparator /> : null}
        <TransactionsMenuItems outId={outId} inId={inId} sayWhat={sayWhat} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
