"""Synthetic signed loopback exchanges for Kraken, Coinbase and Binance.

Each fake checks the request signature the way its provider does, serves
scripted history pages and can inject throttling, outages, credential refusals
and malformed bodies. The request audit records only what both
implementations must agree on: path, page cursor and stable parameters, never
nonces, timestamps or signatures.
"""
import base64
from contextlib import contextmanager
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
from urllib.parse import parse_qsl, urlsplit

KEY = "synthetic-exchange-key"
SECRET = "synthetic-exchange-secret"
VOLATILE = {"nonce", "timestamp", "signature", "endTime"}


class Exchange:
    """Shared script controls. `faults` maps a request ordinal (0-based) to a
    `(status, headers, body_bytes)` reply served instead of the real page."""

    kind = ""

    def __init__(self):
        self.requests = []
        self.faults = {}
        self.always = None

    def reset(self):
        self.requests.clear()

    def secret_for_backend(self):
        return SECRET

    def audit(self, method, path, params):
        stable = sorted((k, v) for k, v in params if k not in VOLATILE)
        self.requests.append({"method": method, "path": path, "params": stable})

    def handle(self, method, target, headers, body):
        parts = urlsplit(target)
        params = parse_qsl(parts.query, keep_blank_values=True)
        if method == "POST":
            params += parse_qsl(body.decode(), keep_blank_values=True)
        if not self.authentic(method, target, headers, body):
            return 401, {}, b'{"error":["EAPI:Invalid key"],"msg":"Invalid API-key, IP, or permissions for action."}'
        ordinal = len(self.requests)
        self.audit(method, parts.path, params)
        if self.always is not None:
            return self.always
        if ordinal in self.faults:
            return self.faults[ordinal]
        status, payload = self.reply(method, parts.path, dict(params))
        body = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
        return status, {}, body


class Kraken(Exchange):
    kind = "kraken"

    def __init__(self):
        super().__init__()
        self.page = 1
        self.trades = {
            "Z": {"pair": "XXBTZEUR", "cost": "400.00", "fee": "1.20", "price": "40000.00"},
            "A": {"pair": "XBTEUR", "cost": "42.00", "fee": "0.26", "price": "42000.00"},
            "Q": {"pair": "wbtc/zeur", "cost": "10.00", "fee": "20.00"},
        }
        self.ledgers = {
            "Z": {"refid": "Z", "time": 1700000000.125, "type": "trade", "asset": "XXBT", "amount": "0.01", "fee": "0"},
            "A": {"refid": "A", "time": "1700000100", "type": "trade", "asset": "XBT", "amount": "-0.001", "fee": "0.000001"},
            "Q": {"refid": "Q", "time": 1700000150.9999995, "type": "Trade ", "asset": "btc", "amount": "-0.0003", "fee": "0"},
            "W": {"time": "1700000200", "type": "withdrawal", "asset": "BTC", "amount": "-0.002", "fee": "0.00001"},
            "D": {"time": "1700000300", "type": "deposit", "asset": "BTC", "amount": "3E-3", "fee": "0", "ledger_id": "shadow"},
            "E": {"type": "staking", "asset": "ETH", "amount": "10"},
            "F": {"type": "deposit", "asset": "ZEUR", "amount": "100"},
        }
        self.counts = {}

    def secret_for_backend(self):
        return base64.b64encode(SECRET.encode()).decode()

    def authentic(self, method, target, headers, body):
        if method != "POST":
            return False
        nonce = dict(parse_qsl(body.decode())).get("nonce", "")
        digest = hashlib.sha256(nonce.encode() + body).digest()
        expected = base64.b64encode(hmac.new(SECRET.encode(), urlsplit(target).path.encode() + digest, hashlib.sha512).digest()).decode()
        return headers.get("API-Key") == KEY and hmac.compare_digest(headers.get("API-Sign", ""), expected)

    def reply(self, method, path, params):
        offset = int(params.get("ofs", "0"))
        field = "trades" if path == "/0/private/TradesHistory" else "ledger"
        rows = self.trades if field == "trades" else self.ledgers
        page = dict(list(rows.items())[offset:offset + self.page])
        count = self.counts.get(field, len(rows))
        return 200, {"error": [], "result": {"count": count, field: page}}


class Coinbase(Exchange):
    kind = "coinbase"

    def __init__(self):
        super().__init__()
        self.page = 2
        self.accounts = [
            {"id": "btc-wallet", "currency": {"code": "BTC"}},
            {"id": "eur-wallet", "currency": {"code": "EUR"}},
            {"id": "", "currency": "BTC"},
            "not-an-account",
            {"id": "l btc/€", "currency": "lbtc"},
        ]
        self.transactions = {
            "btc-wallet": [
                {"id": "buy-1", "type": "buy", "created_at": "2024-01-02T03:04:05Z",
                 "amount": {"amount": "0.01", "currency": "BTC"},
                 "native_amount": {"amount": "-400.00", "currency": "EUR"},
                 "buy": {"commission": "1.50"}},
                {"id": "sell-1", "type": "Sell", "created_at": "2024-01-03T03:04:05Z",
                 "amount": {"amount": "-0.002", "currency": "BTC"},
                 "native_amount": {"amount": "-80.00", "currency": "eur"}},
                {"id": "send-1", "type": "send", "created_at": "2024-01-04T03:04:05Z",
                 "amount": {"amount": "-0.001", "currency": "BTC"},
                 "network": {"hash": "  " + "ab" * 32 + " "}},
                {"id": "", "type": "interest", "createdAt": "2024-01-05T00:00:00Z",
                 "amount": {"amount": "0.00000100"}},
                {"id": "zero", "type": "mystery", "amount": {"amount": "0"}},
                {"id": "eth", "type": "buy", "amount": {"amount": "1", "currency": "ETH"}},
                {"id": "stake", "type": "staking_reward", "created_at": "", "createdAt": "2024-01-06T00:00:00Z",
                 "amount": {"amount": 2e-06, "currency": "BTC"}},
            ],
            "l btc/€": [
                {"id": "lbtc-1", "type": "exchange_deposit", "created_at": "2024-02-01T00:00:00Z",
                 "amount": {"amount": "0.5", "currency": "L-BTC"}},
            ],
        }

    def authentic(self, method, target, headers, body):
        timestamp = headers.get("CB-ACCESS-TIMESTAMP", "")
        expected = hmac.new(SECRET.encode(), f"{timestamp}{method}{target}".encode(), hashlib.sha256).hexdigest()
        return (method == "GET" and headers.get("CB-ACCESS-KEY") == KEY
                and headers.get("CB-VERSION") == "2017-11-27"
                and hmac.compare_digest(headers.get("CB-ACCESS-SIGN", ""), expected))

    def collection(self, path):
        if path == "/v2/accounts":
            return self.accounts
        from urllib.parse import unquote
        account = unquote(path.removeprefix("/v2/accounts/").removesuffix("/transactions"))
        return self.transactions.get(account, [])

    def reply(self, method, path, params):
        rows = self.collection(path)
        start = int(params.get("starting_after", "0"))
        page = rows[start:start + self.page]
        following = start + self.page
        next_uri = f"{self.raw_path(path)}?starting_after={following}" if following < len(rows) else None
        return 200, {"data": page, "pagination": {"next_uri": next_uri}}

    @staticmethod
    def raw_path(path):
        from urllib.parse import quote
        return quote(path)


class Binance(Exchange):
    kind = "binance"

    def __init__(self):
        super().__init__()
        self.histories = {
            "/sapi/v1/fiat/payments": {"code": "000000", "data": [
                {"orderNo": "F1", "status": "Completed", "cryptoCurrency": "BTC", "fiatCurrency": "EUR",
                 "obtainAmount": "0.01", "sourceAmount": "-400.5", "price": "40050", "createTime": 1700000000123},
                {"orderNo": "F2", "status": "Failed", "cryptoCurrency": "BTC", "fiatCurrency": "EUR",
                 "obtainAmount": "1", "sourceAmount": "1"},
                {"orderNo": "", "orderId": 0, "status": " completed ", "cryptoCurrency": "xbt",
                 "fiatCurrency": "eur", "obtainAmount": "0.002", "sourceAmount": "80", "createTime": "1700000001999"},
                {"orderNo": "F4", "status": "Completed", "cryptoCurrency": "ETH", "fiatCurrency": "EUR"},
            ]},
            "/sapi/v1/capital/deposit/hisrec": [
                {"coin": "BTC", "status": 1, "amount": "0.005", "insertTime": 1700000100000, "txId": "d" * 64},
                {"coin": "BTC", "status": 0, "amount": "0.006", "insertTime": 1700000200000},
                {"coin": "BTC", "status": 2, "amount": "9", "insertTime": 1700000300000, "txId": "rejected"},
                {"asset": "LBTC", "amount": 0.25, "insertTime": 1700000400000, "id": "lq-1"},
            ],
            "/sapi/v1/capital/withdraw/history": [
                {"coin": "BTC", "status": 6, "amount": "0.004", "transactionFee": "0.0001",
                 "applyTime": 1700000500000, "id": "w-1", "txId": ""},
                {"coin": "BTC", "status": "Completed", "amount": "0.001", "fee": "0.00002",
                 "successTime": 1700000600000, "withdrawOrderId": "  "},
            ],
            "/sapi/v1/asset/assetDividend": {"total": 3, "rows": [
                {"id": 7, "asset": "BTC", "amount": "0.0000123", "divTime": 1700000700000, "enInfo": "BTC Mining Pool"},
                {"tranId": 8, "coinName": "BTC", "profitAmount": "0.0000004", "time": 1700000800000, "type": "Savings"},
                {"id": 9, "asset": "BTC", "amount": "0", "divTime": 1700000900000},
            ]},
        }

    def authentic(self, method, target, headers, body):
        query = urlsplit(target).query
        unsigned, _, signature = query.rpartition("&signature=")
        expected = hmac.new(SECRET.encode(), unsigned.encode(), hashlib.sha256).hexdigest()
        return (method == "GET" and headers.get("X-MBX-APIKEY") == KEY
                and hmac.compare_digest(signature, expected))

    # A read-only key, as `GET /sapi/v1/account/apiRestrictions` describes it
    # (the desktop's key check reads it; the sync does not).
    restrictions = {"ipRestrict": False, "createTime": 1700000000000, "enableReading": True,
                    "enableWithdrawals": False, "enableInternalTransfer": False, "enableMargin": False,
                    "enableFutures": False, "permitsUniversalTransfer": False, "enableVanillaOptions": False,
                    "enableSpotAndMarginTrading": False, "enablePortfolioMarginTrading": False}

    def reply(self, method, path, params):
        if path == "/sapi/v1/account/apiRestrictions":
            return 200, self.restrictions
        return 200, self.histories.get(path, [])


BITPANDA_BTC = "b86c034b-efe3-11eb-b56f-0691764446a7"
BITPANDA_ETH = "b86c0a4c-efe3-11eb-b56f-0691764446a7"
BITPANDA_EUR = "b88b8466-efe3-11eb-b56f-0691764446a7"


def _bitpanda_tx(tx_id, value, flow, credited_at, *, asset=None, currency=None, wallet="w-btc",
                 balance=None, fee=None, trade=None):
    amount = {"value": value, "asset_id": asset, "currency_id": currency}
    return {"transaction_id": tx_id, "asset_id": asset, "currency_id": currency, "wallet_id": wallet,
            "wallet_owner": "user", "asset_amount": amount, "fee_amount": fee, "transaction_type": "",
            "flow": flow, "order_id": None, "credited_at": credited_at,
            "asset_balance_after": None if balance is None else {"value": balance, "asset_id": asset, "currency_id": None},
            "compensates": None, "compensates_info": None, "index_asset_id": None, "trade": trade}


class Bitpanda(Exchange):
    """Bitpanda's public API (Rust only): `x-api-key` GETs, cursor pages of
    the documented shapes (docs/reference/bitpanda.md)."""

    kind = "bitpanda"

    def __init__(self):
        super().__init__()
        self.page = 2
        self.limits = (401, {"error": {"code": "insufficient_scope"}})
        btc = dict(asset=BITPANDA_BTC)
        eur = dict(currency=BITPANDA_EUR, wallet="w-eur")
        trade = lambda trade_id: {"trade_id": trade_id, "fee": {"value": "1.49", "asset_id": None, "currency_id": BITPANDA_EUR},
                                  "fee_percentage": "0.0149", "rate": "40000", "rate_with_fee": "40149", "to_eur_rate": "1"}
        self.operations = [
            {"operation_id": "op-buy", "operation_type": "BUY", "transactions": [
                _bitpanda_tx("t-buy", "0.01", "INCOMING", "2024-01-02T10:00:00+01:00", balance="0.01", trade=trade("trade-1"), **btc),
                _bitpanda_tx("t-buy-eur", "401.49", "OUTGOING", "2024-01-02T10:00:00+01:00", **eur)]},
            {"operation_id": "op-deposit", "operation_type": "DEPOSIT", "transactions": [
                _bitpanda_tx("t-deposit", "0.005", "INCOMING", "2024-01-03T00:00:00Z", balance="0.015", **btc)]},
            {"operation_id": "op-withdrawal", "operation_type": "WITHDRAWAL", "transactions": [
                _bitpanda_tx("t-withdrawal", "0.004", "OUTGOING", "2024-01-05T00:00:00Z", balance="0.0109",
                             fee={"value": "0.0001", "asset_id": BITPANDA_BTC, "currency_id": None}, **btc)]},
            {"operation_id": "op-sell", "operation_type": "SELL", "transactions": [
                _bitpanda_tx("t-sell", "0.002", "OUTGOING", "2024-01-06T00:00:00Z", balance="0.0089", trade=trade("trade-2"), **btc),
                _bitpanda_tx("t-sell-eur", "84.00", "INCOMING", "2024-01-06T00:00:00Z", **eur)]},
            {"operation_id": "op-reward", "operation_type": "REWARD", "transactions": [
                _bitpanda_tx("t-reward", "0.00001", "INCOMING", "2024-01-07T00:00:00Z", balance="0.00891", **btc)]},
            {"operation_id": "op-fiat", "operation_type": "DEPOSIT", "transactions": [
                _bitpanda_tx("t-fiat", "500", "INCOMING", "2024-01-01T00:00:00Z", **eur)]},
            {"operation_id": "op-eth", "operation_type": "BUY", "transactions": [
                _bitpanda_tx("t-eth", "1", "INCOMING", "2024-01-02T00:00:00Z", asset=BITPANDA_ETH, wallet="w-eth", trade=trade("trade-3")),
                _bitpanda_tx("t-eth-eur", "2000", "OUTGOING", "2024-01-02T00:00:00Z", **eur)]},
        ]

    def authentic(self, method, target, headers, body):
        return method == "GET" and headers.get("x-api-key") == KEY and not body

    def reply(self, method, path, params):
        if path == "/v1/trading-limits":
            return self.limits
        if path == "/v1/assets":
            return 200, {"data": [{"id": BITPANDA_BTC, "symbol": "BTC", "type": "CRYPTO", "group": "CRYPTOCOIN"}],
                         "self_cursor": "", "next_cursor": "", "has_next_page": False}
        if path == "/v1/currencies":
            return 200, {"data": [{"id": BITPANDA_EUR, "symbol": "EUR", "name": "Euro"}]}
        start = int(params.get("cursor", "c0")[1:])
        end = min(start + self.page, len(self.operations))
        more = end < len(self.operations)
        return 200, {"data": self.operations[start:end], "self_cursor": f"c{start}",
                     "next_cursor": f"c{end}" if more else None, "has_next_page": more}


@contextmanager
def serve(exchange, **overrides):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def respond(self, method):
            body = self.rfile.read(int(self.headers.get("Content-Length", "0") or "0"))
            status, headers, payload = exchange.handle(method, self.path, self.headers, body)
            self.send_response(status)
            for name, value in headers.items():
                self.send_header(name, value)
            self.send_header("Content-Length", str(len(payload)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(payload)

        def do_GET(self):
            self.respond("GET")

        def do_POST(self):
            self.respond("POST")

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    backend = {"name": "fixture", "kind": exchange.kind, "url": f"http://127.0.0.1:{server.server_port}",
               "token": KEY, "auth_header": exchange.secret_for_backend(), "timeout": 3}
    backend.update(overrides)
    try:
        yield backend
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        assert not thread.is_alive()
