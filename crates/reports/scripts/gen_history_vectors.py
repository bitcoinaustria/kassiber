"""Synthetic process-contract cases for stored historical holdings."""

def add_cases(case):
    def entry(id_, at, kind, quantity, fiat=0, basis=0, wallet="cold", asset="BTC", created="fixed", account="'acct'"):
        return ("INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,account_id,occurred_at,entry_type,asset,quantity,fiat_value,cost_basis,created_at) VALUES"
                f"('{id_}','ws','p','in','{wallet}',{account},'{at}','{kind}','{asset}',{quantity},{fiat},{basis},'{created}');")

    data = "DELETE FROM journal_entries;"
    data += entry("buy", "2024-02-28T23:00:00Z", "acquisition", 100000000007, 30000.123456789)
    data += entry("send", "2024-02-29T00:00:00Z", "transfer_out", -60000001001)
    data += entry("receive", "2024-02-29T00:00:00Z", "transfer_in", 60000000000, wallet="hot")
    data += entry("network-fee", "2024-02-29T00:00:00Z", "transfer_fee", -1001, basis=.0003003)
    data += entry("earn-lot", "2024-02-29T00:00:00Z", "acquisition", 10000000000, 4000, wallet="hot")
    data += entry("earn-recognition", "2024-02-29T00:00:00Z", "income", 10000000000, 4000, wallet="hot")
    data += entry("sell", "2024-03-01T00:00:00Z", "disposal", -10000000000, basis=2500)
    data += entry("fee", "2024-03-01T00:00:00Z", "fee", -3, basis=.0000009)
    data += entry("lbtc", "2024-03-01T00:00:00Z", "acquisition", 1001, .000123, asset="LBTC", account="NULL")
    data += entry("other-book", "2024-03-01T00:00:00Z", "acquisition", 100000000, 10000, wallet="other")
    data += "UPDATE journal_entries SET profile_id='other' WHERE id='other-book';"
    data += "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,fiat_rate,occurred_at,created_at) VALUES('other-price','ws','other','other','other-price','BTC','inbound',100000,0,777777,'2024-03-01T00:00:00Z','fixed');"
    args = {"interval":"day", "start":"2024-02-28", "end":"2024-03-02"}
    def add(name, sql=data, **options):
        selected = {**args, **options}
        group = "history"
        if any(word in name for word in ("cache", "quote", "transaction")):
            group = "history-quotes"
        elif name.startswith(("wallet", "account", "combined", "nonpositive")):
            group = "history-scope"
        elif name.startswith(("day", "negative", "zero", "created", "id-order", "decimal", "quantity")):
            group = "history-quantity"
        case("history-"+name, group, "reports.balance-history", sql, selected)
        if "workspace" not in options:
            case("desktop-history-"+name, "desktop-history", "ui.reports.balance_history", sql, {"limit":120, **selected})
    for interval in ("hour", "day", "week", "month"):
        add(interval, interval=interval)
    for wallet in ("cold", " Hot ", "absent", "other", ""):
        add("wallet-"+wallet, wallet=wallet)
    for account in ("acct", " TrEaSuRy ", "Treasury", "absent", ""):
        add("account-"+account, account=account)
    for asset in ("BTC", "LBTC", "UNKNOWN", "", None):
        add("asset-"+str(asset), asset=asset)
    add("asset-filter-before-invalid", sql=data+"UPDATE journal_entries SET fiat_value='invalid' WHERE asset='LBTC';", asset="BTC")
    add("combined", wallet="Hot", account="treasury", asset="BTC")
    add("pre-period-holdings", start="2024-03-01", end="2024-03-01")
    add("first-event-start", start=None)
    add("inferred-start-after-end", start=None, end="2020-01-01")
    add("hour-fraction", interval="hour", start="2024-02-28T23:30:05.123456Z", end="2024-02-29T00:30:00Z")
    add("hour-intraday-floor", interval="hour", start="2024-02-29T12:34:56.123456Z", end="2024-02-29T14:15:00Z")
    add("zero-quantity-basis", sql=data+entry("basis-zero", "2024-02-29T00:00:00Z", "acquisition", 0, 1))
    add("quantity-invalid", sql=data+"UPDATE journal_entries SET quantity='invalid' WHERE id='buy';")
    add("quantity-legacy-real", sql=data+"UPDATE journal_entries SET quantity=1001.9 WHERE id='buy';")
    add("whole-final-bucket", end="2024-02-28T23:00:00Z")
    add("offset", start="2024-02-29T01:00:00+02:00", end="2024-02-29T23:00:00-02:00")
    add("week-year", start="2023-12-31", end="2024-01-02", interval="week", asset="BTC")
    add("december", start="2023-12-31", end="2024-01-02", interval="month", asset="BTC")
    add("bad-start", start="no date")
    add("bad-start-type", start=False)
    add("unsupported-interval-before-scope", interval="bad", workspace="absent")
    for asset in ([], {}, ["BTC"], {"asset":"BTC"}, 0, False, 123):
        add("asset-type-"+repr(asset), asset=asset)
    add("bad-end", end=" ")
    add("reversed", start="2024-03-03")
    add("no-bounds-empty", sql="DELETE FROM journal_entries;", start=None, end=None)
    add("empty-skips-unused-price-parse", sql="DELETE FROM journal_entries; UPDATE transactions SET fiat_rate='invalid',occurred_at='invalid';", start=None, end=None)
    add("empty-one-bound", sql="DELETE FROM journal_entries;", start=None)
    add("empty-bounds", sql="DELETE FROM journal_entries;")
    add("empty-explicit-asset", sql="DELETE FROM journal_entries;", asset="BTC")
    add("negative", sql=data+entry("negative", "2024-02-28T23:00:00Z", "disposal", -200000000000, basis=30000))
    add("zero", sql=data+entry("zero", "2024-03-01T00:00:00Z", "fee", -99999999003))
    add("zero-explicit-asset", sql=data+entry("zero", "2024-03-01T00:00:00Z", "fee", -99999999003), asset="BTC")
    add("no-quotes", sql=data+"UPDATE transactions SET fiat_rate=NULL,fiat_value=NULL;")
    add("no-cache", sql=data+"DROP TABLE rates_cache;")
    add("invalid-cache", sql=data+"INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2024-02-29T00:00:00Z',1,'bad','manual','fixed');")
    add("missing-basis-is-still-stale", sql=data+"UPDATE profiles SET last_processed_at=NULL;")
    add("nonpositive-pool", sql=data+entry("big-sell", "2024-02-29T00:00:00Z", "disposal", -200000000000, basis=40000), wallet="hot")
    # Journal order affects precision-32 cancellation, independent of float output.
    rounding = "DELETE FROM journal_entries;"
    for id_, fiat, created in (("b", 1e30, "a"), ("z", .06, "b"), ("a", -1e30, "c")):
        rounding += entry(id_, "2024-02-29T00:00:00Z", "acquisition", 1, fiat, created=created)
    add("created-order", sql=rounding)
    add("id-order", sql=rounding+"UPDATE journal_entries SET created_at='same';")
    # HALF_EVEN tie at the report's precision, observable after basis cancellation.
    add("decimal-tie", sql=rounding.replace("0.06", "0.05"))
    # Three tie breakers at the same timestamp, plus a quote after the end cap.
    cache = "DELETE FROM rates_cache;"
    for source, fetched, rate in (("zzz", "a", 1), ("aaa", "a", 2), ("bbb", "z", 3), ("manual", "a", 4)):
        cache += f"INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2024-02-29T00:00:00Z',{rate},'{rate}.12345678901234567890123456789','{source}','{fetched}');"
    cache += "INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at) VALUES('BTC-EUR','2024-03-02T00:00:01Z',999,'manual','z');"
    cache += "INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at) VALUES('BTC-EUR','2024-02-28T00:00:00Z',0.5,'prior','fixed');"
    for label, extra in (("manual",""),("fetched","DELETE FROM rates_cache WHERE source='manual';"),("source","DELETE FROM rates_cache WHERE source='manual' OR fetched_at='z';")):
        add("cache-"+label, sql=data+cache+extra)
    add("cache-null-exact", sql=data+cache+"UPDATE rates_cache SET rate_exact=NULL;")
    add("cache-over-transaction", sql=data+cache+"UPDATE transactions SET occurred_at='2024-03-01T00:00:00Z',fiat_rate=999999;")
    add("quote-boundary", sql=data+cache, end="2024-02-29T00:00:00Z")
    for field in ("occurred_at", "created_at", "id"):
        sql=data+"UPDATE transactions SET occurred_at='2024-02-29T00:00:00Z',created_at='same',fiat_rate=100,fiat_value=NULL;"
        sql+="UPDATE transactions SET fiat_rate=200 WHERE id='out';"
        if field=="occurred_at": sql+="UPDATE transactions SET occurred_at='2024-02-29T12:00:00Z' WHERE id='out';"
        if field=="created_at": sql+="UPDATE transactions SET created_at='z' WHERE id='out';"
        add("transaction-order-"+field, sql=sql)
    add("quote-value-only", sql=data+"UPDATE transactions SET occurred_at='2024-02-29T00:00:00Z',fiat_rate=NULL,fiat_value=10;")
    add("quote-zero-amount", sql=data+"UPDATE transactions SET occurred_at='2024-02-29T00:00:00Z',fiat_rate=NULL,fiat_value=10,amount=0;")
    add("quote-invalid-amount", sql=data+"UPDATE transactions SET occurred_at='2024-02-29T00:00:00Z',fiat_rate=NULL,fiat_value=10,amount='invalid';")
    add("quote-excluded", sql=data+"UPDATE transactions SET excluded=1; UPDATE profiles SET last_processed_tx_count=0;")
    add("wallet-ambiguous", sql=data+"INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('duplicate','ws','p','COLD','custom','{}','fixed');", wallet="Cold")
    duplicate="INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('acct2','ws','p','other','Treasury','asset','fixed');"
    add("account-ambiguous", sql=data+duplicate+"UPDATE accounts SET label='Shared';", account="Shared")
    add("account-label", sql=data+"UPDATE accounts SET label='Reserve Account';", account="reserve account")
    add("account-other-profile", sql=data+"INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('foreign','ws','other','foreign','Foreign','asset','fixed');", account="foreign")
    add("account-code-first", sql=data+duplicate, account="treasury")
    add("account-id-first", sql=data+duplicate+"UPDATE accounts SET code='acct' WHERE id='acct2';", account="acct")
    # Desktop truncation keeps the last rows, after the full sorted projection.
    case("desktop-history-truncated", "desktop-history", "ui.reports.balance_history", data, {**args,"limit":2})
    case("desktop-history-exact-limit", "desktop-history", "ui.reports.balance_history", data, {**args,"limit":6})
    # Every pre-existing readiness case also guards historical delivery.
    return args
