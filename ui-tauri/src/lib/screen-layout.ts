/*
 * The page frame every screen renders inside the shell's panel, from the
 * layout scale in globals.css: one gutter on every side and one gap between
 * the blocks of a page. Screens take no padding of their own, which keeps the
 * first row and the page edges in the same place on every screen.
 */
export const screenShellClassName =
  "w-full space-y-(--kb-page-gap) bg-background p-(--kb-page-gutter)";

export const screenPanelClassName =
  "w-full bg-background p-(--kb-page-gutter)";

/*
 * The first row of a page: its state, filters, or informative title on the
 * left and its actions on the right. The title bar already names the page (it
 * is the page's `h1`), so a page does not repeat its name here and has no
 * eyebrow line above it; `pageTitleClassName` is only for what the title bar
 * does not say, such as a settings section or a wallet's name.
 */
export const pageHeaderClassName =
  "flex flex-col gap-2.5 sm:flex-row sm:items-center sm:justify-between sm:gap-3";

export const pageHeaderActionsClassName =
  "flex flex-wrap items-center gap-2 sm:gap-3";

export const pageHeaderActionClassName = "h-8 gap-2";

export const pageHeaderIconButtonClassName = "size-8";

export const pageTitleClassName = "text-lg leading-tight font-semibold";

export const pageDescriptionClassName = "max-w-3xl text-sm text-muted-foreground";
