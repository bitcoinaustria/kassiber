# Austrian-German UI glossary

Canonical terminology for transcreating the Kassiber desktop UI into **Austrian
German**, researched against BMF / FinanzOnline / oesterreich.gv.at sources and
adversarially verified. This is the single source of truth for translators and
for the i18n resource bundles — match these terms verbatim instead of
re-inventing. Companion to [i18n.md](i18n.md).

## Style guide

### Register & tone
- **Always informal `du`**, never `Sie`. When copy addresses the user, use the
  **du-imperative** in prose: „Wähle…", „Speichere…", „Reiche … beim Finanzamt
  ein" — not „Wählen Sie…".
- **Standalone buttons use the infinitive**: „Speichern", „Abbrechen",
  „Exportieren". The du-imperative is for sentences that address the user, not
  for button labels.
- Neutral, factual tone. **No standing „alles in Ordnung"/reassurance copy** —
  render status only in the actionable / not-OK state. Empty states stay neutral
  and impersonal („Noch nichts vorhanden").

### Austrian German (vs German German)
- Month **„Jänner"** not „Januar"; „Februar" is safe. „heuer" for „this year"
  where natural. Drive month names via `localeForLanguage` (de-AT).
- Tax terms follow **official BMF / FinanzOnline wording**: Beilage (not
  Anlage/Anhang), KESt, Kennzahl (not Zeile), Wegzugsbesteuerung, fiktive
  Veräußerung, Verlustausgleich, Bemessungsgrundlage, gleitender
  Durchschnittspreis (not -kosten), Veranlagungsjahr.

### Numbers, currency, dates
- **Decimal comma**: „27,5 %", „0,5 BTC" — never „27.5%". Thousands separator is
  the dot („1.000,00 €").
- **Space before `%`** and the currency sign — „27,5 %" (the existing
  `{{percent}} %` key). Use a normal space (not a narrow no-break space) to keep
  the formatting pipeline simple.
- Date format **TT.MM.JJJJ**.

### Capitalisation / casing
- **All German nouns are capitalised** (Name, Datum, Betrag, Gebühr,
  Transaktion, Einstellungen).
- Noun labels capitalised normally; sentence-style status / empty-state text
  uses sentence case („Wird geladen…", „Etwas ist schiefgelaufen").
- Keep the **ellipsis „…"** in progress states, and use the **passive idiom**
  („Wird geladen…", „Wird synchronisiert…"), not „Lädt…".

### Bitcoin / crypto loanwords — KEEP ENGLISH
Keep as-is with a German article: **die Wallet, die Blockchain, der Node, der
Mempool, der Hash, der/das UTXO, der xpub/ypub/zpub, der Descriptor, der Seed,
der Peg-in, der Peg-out, der Swap, der Submarine Swap, das Lightning (Network),
Liquid** (proper name), **der Explorer, der Coin, der Satoshi / die Sats, BTC**.
`on-chain` / `off-chain` stay lowercase, hyphenated, adjectival. Hybrids:
**Blockhöhe**, „Wallet-Adresse", „Lightning-Zahlung", „Transaktionsgebühr".
**Never** translate to Geldbörse / Knoten / Saat / Deskriptor.

Ordinary words **DO translate**: transaction→Transaktion, fee→Gebühr,
address→Adresse, balance→Guthaben (verfügbar) / Saldo (buchhalterisch),
amount→Betrag, sent→gesendet, received→erhalten, settings→Einstellungen,
confirmation→Bestätigung.

### Generic-tech terms — also KEEP ENGLISH
Beyond Bitcoin jargon, these space-common tech terms stay English (owner
decision), used consistently:
- **AI** — keep the English abbreviation, **not** „KI": `AI-Provider`,
  `AI-Funktionen`, `AI-Assistent`, `AI-Modelle`, standalone `AI`. (The feature
  noun „Assistent" stays German.)
- **Provider** — `der Provider` (AI / market-rate provider). Use everywhere
  (`AI-Provider`, „Provider wählen", `Provider-Marktpreis`); **not** „Anbieter".
  Compound data-field labels (`Provider-Order-id`, `Provider-Payment-id`…) keep
  Provider too. `Drittanbieter` (= third-party) is a different word — leave it.
- **Reconcile screen** — the German nav label must say *what* it reconciles, so
  it is **„Adress-Abgleich"** (not a bare „Abgleich"): the side-nav label alone
  should tell the user the screen matches addresses/txids to their wallets.
  (Contextual „Abgleichlücken" elsewhere is fine.)
- **Sync** — `der Sync` (noun/label/button), progress „Sync läuft…", status
  „Synced" / „Nicht synced", inline adjective „gesyncte …"; compounds
  `Sync-Backend`, `Lightning-Node-Sync`. **not** „Synchronisieren/-ung". The
  verb may still read „… per Sync …" where natural.
- **Setup** — `das Setup` (the noun: `Verbindungs-Setup`, „das Wallet-Setup",
  „nach dem Setup"); **not** „Einrichtung". The verb „einrichten" stays German.

Other kept-English space terms already in use: **Backend, Backup, Cache,
Label** (BIP329), **Token, Indexer, Explorer, Watch-only, Settlement, Private
Key, Dashboard, Passphrase, on-chain/off-chain**.

### Critical one-EN→one-DE disambiguations
- **Ledger = Hauptbuch** (the app's accounting ledger) — but the Bitcoin chain
  itself is **die Blockchain**, never „Hauptbuch".
- **Swap (Bitcoin/Liquid jargon) = der Swap** (English) vs **the tax
  crypto-to-crypto exchange = der Tausch** (German). Do not conflate.
- **Cost basis / acquisition cost / „basis" = Anschaffungskosten** (all collapse
  to one AT term).
- **Sync = synchronisieren** vs **Refresh = Aktualisieren** — keep distinct.
- **Delete = Löschen** (destructive) vs **Remove = Entfernen** (detach).
- **Continue = Next = Weiter** (paired with **Back = Zurück**).
- **Pending = Ausstehend** (generic) vs **unconfirmed on-chain = Unbestätigt**.
- State vs action: „Aktiviert/Deaktiviert" (state) vs „Aktivieren/Deaktivieren"
  (action); „Verborgen" (state) vs „Ausblenden" (action).

### Acronyms & form codes (literal, never translated)
FIFO, KESt, ESt, Kz./KZ, **Formular E 1**, **Beilage E 1kv**, § 27 / § 27a /
§ 27b EStG. The tax-method label **ATM** (Austrian Tax Method) and model id
`qwen3.6:35b` are intentional — do not re-flag.

## Resolved decisions (the synthesis raised these as open; defaults below)
1. **Capital gains** — UI columns use **„Realisierter Gewinn / Verlust"**; the
   statutory „Einkünfte aus realisierten Wertsteigerungen" is reserved for
   report/tax-summary headings (deferred surface).
2. **Taxable event** — UI uses **„steuerpflichtiges Ereignis"**; precise tax
   contexts may use „steuerpflichtige Realisierung".
3. **Alt/Neu** — UI uses **Altbestand / Neubestand** (clear pairing, matches the
   code's `Alt`/`Neu`), even though BMF prose often says „Neuvermögen".
4. **balance** — **Guthaben** for wallet/available funds; **Saldo** for a
   ledger/account net figure. Choose per screen.
5. **Dashboard ≠ Übersicht** — keep „Dashboard" (English) for the dashboard
   concept distinct from „Übersicht" for overview sections.
6. **Lot = Tranche** where lot-level views surface.
7. **Micro-typography** — normal space before „%"/currency (matches existing
   keys); no narrow no-break space.

> These are defaults, centralized here and in the locale JSON — changing a term
> later is a one-place edit. Flag any you disagree with.

## Tax terms (§ 27 EStG regime, post Ökosoziale Steuerreform 2022)

| English | Austrian German | Notes |
| --- | --- | --- |
| Cryptocurrency | Kryptowährung | die Kryptowährung; § 27b Abs 4 EStG. Tax umbrella term is German even though Bitcoin jargon stays English. |
| Capital gains | Einkünfte aus realisierten Wertsteigerungen | Statutory BMF term (Substanzgewinne). UI short form: „Realisierter Gewinn/Verlust". |
| Realized gain / loss | Realisierter Gewinn / Verlust | Per-transaction column. |
| Income from capital assets | Einkünfte aus Kapitalvermögen | The § 27 EStG income type crypto falls under since 1.3.2022. |
| Current income (staking/lending/mining) | laufende Einkünfte aus Kryptowährungen | The „fruits". Staking/Lending/Mining stay English in surrounding text. |
| Special tax rate 27.5% | besonderer Steuersatz von 27,5 % | § 27a Abs 1 EStG. Decimal comma + space before %. |
| Tax-neutral crypto-to-crypto exchange | steuerneutraler Tausch Krypto-zu-Krypto | BMF: crypto→crypto is no disposal; cost basis carries. Crypto→fiat/goods IS taxable. |
| Taxable event | steuerpflichtiges Ereignis | UI form; „steuerpflichtige Realisierung" in precise contexts. |
| Disposal / Sale | Veräußerung | EStG term; verb „veräußern". A crypto-crypto swap is *not* a Veräußerung. |
| Holding period | Behaltedauer | No 1-year Spekulationsfrist for Neubestand anymore; only delimits Alt/Neu. |
| Old holdings (pre 1.3.2021, tax-free) | Altbestand | Acquired ≤ 28.2.2021; tax-free on disposal. |
| New holdings | Neubestand | Acquired after 28.2.2021; § 27 regime, 27,5 %. |
| Income tax | Einkommensteuer | Abk. ESt. |
| Income tax return | Einkommensteuererklärung | Underlying form „Formular E 1" via FinanzOnline. |
| Capital gains tax / KESt | Kapitalertragsteuer (KESt) | Keep abbreviation KESt. |
| Tax authority | Finanzamt | „Finanzamt Österreich". |
| Tax form line item | Kennzahl | Abk. Kz./KZ. Not „Zeile". Already in code. |
| Capital-income annex E1kv | Beilage E 1kv | „Beilage", not Anlage/Anhang. Form code „E 1kv" literal. |
| Loss offsetting | Verlustausgleich | Intra-year offset within the 27,5 % pool. Not „Verlustverrechnung". |
| Tax base / assessment basis | Bemessungsgrundlage | Avoid „Steuerbasis". |
| Exit taxation | Wegzugsbesteuerung | § 27 Abs 6 EStG. Already in code. |
| Deemed disposal | fiktive Veräußerung | a.k.a. Veräußerungsfiktion. |
| Tax year | Veranlagungsjahr | „Steuerjahr" as the shorter UI label. Month „Jänner". |
| Tax residency | steuerliche Ansässigkeit | Giving it up = Wegzug. |
| Cost basis / acquisition cost / basis | Anschaffungskosten | One AT term for all three. Fees increase it (Anschaffungsnebenkosten). |
| Moving average cost | gleitender Durchschnittspreis | § 4 KryptowährungsVO, per coin & wallet. Not „-kosten". |
| FIFO | FIFO | „das FIFO-Verfahren". |
| Proceeds | Veräußerungserlös | short „Erlös". |
| Acquisition | Anschaffung | verb „anschaffen/erwerben". |
| Pricing / Valuation | Bewertung | Plain price = „Kurs"/„Preis". |
| Lot | Tranche | a.k.a. Posten. Rarely surfaces under moving-average default. |

## Finance / accounting terms

| English | Austrian German | Notes |
| --- | --- | --- |
| Inflow | Zugang | inventory inflow; time-of-receipt = „Zufluss". |
| Outflow | Abgang | inventory outflow; „Abfluss" for cash. |
| Ledger | Hauptbuch | the app ledger — NOT the Blockchain. |
| Journal | Journal | das Journal (Grundbuch). |
| Journal entry | Journaleintrag | a.k.a. Buchung. |
| Quarantine | Quarantäne | data held back from a report for missing pricing. Verb „in Quarantäne stellen". |
| address | Adresse | „Wallet-Adresse", „Empfangsadresse". |
| transaction | Transaktion | txid stays as-is. |
| fee | Gebühr | „Transaktionsgebühr"/„Netzwerkgebühr". |
| balance | Guthaben / Saldo | Guthaben = available funds; Saldo = ledger net. |
| sent / received | gesendet / erhalten | direction labels. |

## Bitcoin / crypto — KEEP ENGLISH (with German article)

| Term | Article / note |
| --- | --- |
| Wallet | die Wallet, Pl. die Wallets. Never Geldbörse. |
| Blockchain | die Blockchain. |
| Lightning (Network) | das Lightning (Network). „Lightning-Zahlung". |
| Node | der Node. Not „Knoten". |
| Mempool | der Mempool. |
| Hash | der Hash, Pl. Hashes. |
| UTXO | der/das UTXO, Pl. UTXOs. |
| xpub / ypub / zpub | der xpub, lowercase. |
| Descriptor | der Descriptor. Not „Deskriptor". |
| Seed | der Seed; „die Seed-Phrase". |
| Peg-in / Peg-out | der Peg-in / der Peg-out. |
| Swap / Submarine Swap | der Swap (jargon) — distinct from tax „Tausch". |
| Liquid | proper name; „auf Liquid", „das Liquid Network". |
| on-chain / off-chain | lowercase, hyphenated, adjectival. |
| Explorer | der Explorer. |
| Block height | Blockhöhe (Block stays English). |
| Confirmation(s) | Bestätigung(en) — translated; underlying Block stays English. |
| Coin | der Coin (app is Bitcoin-only — prefer BTC/Sats where concrete). |
| Satoshi / Sats | der Satoshi / die Sats. |
| BTC | unit/ticker, „0,5 BTC". |

## General UI vocabulary (du register)

Buttons are infinitive; the du-imperative (in parentheses) is for prose.

| English | Austrian German | Notes |
| --- | --- | --- |
| Save / Cancel / Close | Speichern / Abbrechen / Schließen | |
| Confirm / Delete / Remove | Bestätigen / Löschen / Entfernen | Löschen destructive vs Entfernen detach. |
| Add / Edit / Create | Hinzufügen / Bearbeiten / Erstellen | |
| Open / Import / Export | Öffnen / Importieren / Exportieren | |
| Search / Filter / Sort | Suchen / Filtern / Sortieren | placeholder „Suchen…". |
| Refresh / Sync | Aktualisieren / Synchronisieren | keep distinct. „Wird synchronisiert…". |
| Retry / Reset | Erneut versuchen / Zurücksetzen | |
| Continue / Back / Next / Done | Weiter / Zurück / Weiter / Fertig | |
| Copy | Kopieren | |
| Settings / Overview / Reports | Einstellungen / Übersicht / Berichte | |
| Dashboard | Dashboard | kept English; distinct from Übersicht. |
| Loading… / Saving… | Wird geladen… / Wird gespeichert… | passive idiom. |
| Error / Warning / Success | Fehler / Warnung / Erfolg | success as action confirmation, not standing badge. |
| Details | Details | „Details anzeigen". |
| Show / Hide | Anzeigen / Ausblenden | state „Verborgen". |
| Enabled / Disabled | Aktiviert / Deaktiviert | actions: Aktivieren/Deaktivieren. |
| Connected / Disconnected | Verbunden / Getrennt | |
| Pending / Confirmed / Failed | Ausstehend / Bestätigt / Fehlgeschlagen | on-chain pending → „Unbestätigt". |
| Yes / No | Ja / Nein | |
| Optional / Required | Optional / Erforderlich | „Pflichtfeld" for a required field. |
| name / date / amount | Name / Datum / Betrag | date AT format TT.MM.JJJJ, „Jänner". |
| status / type / actions | Status / Typ / Aktionen | |
| Nothing to show yet | Noch nichts vorhanden | neutral empty state. |
| Something went wrong | Etwas ist schiefgelaufen | generic error. |
