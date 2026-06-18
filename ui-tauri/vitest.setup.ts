// Initialize i18next once for the whole test run so components that call
// `useTranslation` resolve real (English) strings under `renderToStaticMarkup`
// instead of echoing raw keys. The UI store is never touched here, so tests
// stay on the default language unless they change it explicitly.
import "@/i18n";
