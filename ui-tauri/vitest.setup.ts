// Initialize i18next once for the whole test run so components that call
// `useTranslation` resolve real (English) strings under `renderToStaticMarkup`
// instead of echoing raw keys. The UI store is never touched here, so tests
// stay on the default language unless they change it explicitly.
import "@/i18n";

// No UI test has any business reaching the network. Rendering a component
// should never make a request, and a component that starts one on mount is
// exactly the defect this suite keeps finding -- so make it throw rather than
// hang or quietly succeed against a real host.
globalThis.fetch = (input: RequestInfo | URL) => {
  throw new Error(
    `Blocked network request in a test: ${String(
      typeof input === "string" || input instanceof URL ? input : input.url,
    )}. Gate the query with \`enabled\` or mock the transport.`,
  );
};
