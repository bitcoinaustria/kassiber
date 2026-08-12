import * as React from "react";

import { openExternalUrl } from "@/daemon/transport";

type ExternalBrowserLinkProps = Omit<
  React.AnchorHTMLAttributes<HTMLAnchorElement>,
  "href" | "onClick"
> & {
  href: string;
  openUrl?: typeof openExternalUrl;
};

export function ExternalBrowserLink({
  href,
  openUrl = openExternalUrl,
  ...props
}: ExternalBrowserLinkProps) {
  const onClick = async (event: React.MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    try {
      await openUrl(href);
    } catch (error) {
      console.error("Failed to open external URL", error);
    }
  };

  return <a {...props} href={href} onClick={onClick} />;
}
