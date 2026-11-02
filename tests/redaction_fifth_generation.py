"""Reviewer 5g generator (fifth, independent of the author's four and of reviewer gen3/gen4).

Each case composes 1-4 records from realistic log families (URLs, schemeless
credentials, JSON, YAML, XML, CLI flags, env/conf lines, tracebacks, provider
errors) with Unicode, odd quoting and escapes.  Planted secrets carry a sentinel
built only from the letters QJXZVKW (any case) plus one '9', so no 4-character
fragment can occur in prose, hex, paths or numbers.  Numeric secrets (PIN/OTP/
numeric keys) are checked as exact digit runs.

Each case: {kind, input, secrets:[...], numeric:[...], keep:[...], soft:[...], oos:bool}
  keep: non-secret support data that must survive verbatim
  soft: non-secret data whose loss is a documented fail-closed cost
  oos:  the secret shape is outside the documented contract (informational)
Adopted from reviewer 5g gen/gen5.py; only the CLI now streams JSONL.
usage: redaction_fifth_generation.py COUNT SEED
"""
import json
import random
import sys
from urllib.parse import quote

A = 'QJXZVKWqjxzvkw'


def sentinel(rnd, special=False):
    s = ''.join(rnd.choice(A) for _ in range(5)) + '9' + ''.join(rnd.choice(A) for _ in range(5))
    if special:
        pos = rnd.randrange(3, len(s) - 2)
        s = s[:pos] + rnd.choice(list("#@:/%+=&!$'*,;~^`|\\?") + ['"', '<', ']']) + s[pos:]
    return s


def numeric(rnd):
    return str(rnd.choice([86, 87, 88, 89])) + ''.join(rnd.choice('0123456789') for _ in range(rnd.choice([2, 4, 5, 6, 7])))


def txid(rnd):
    return ''.join(rnd.choice('0123456789abcdef') for _ in range(64))


KEEP_GEN = [
    lambda r: txid(r),
    lambda r: f'{r.randrange(0, 21)}.{r.randrange(0, 10**8):08d} BTC',
    lambda r: f'{r.randrange(1, 10**7)} sats',
    lambda r: r.choice(['/home/alice/.bitcoin/debug.log', '/var/lib/kassiber/books/main.sqlite', 'C:\\Users\\bob\\AppData\\Roaming\\Electrum\\config',
                        '~/.lnd/data/chain/bitcoin/mainnet', '/tmp/kassiber-export-2026.csv', './wallets/cold.json']),
    lambda r: r.choice(['ECONNREFUSED', 'ETIMEDOUT', 'HTTP 401 Unauthorized', 'HTTP 429', 'error code -26', 'code=-32603', 'RPC_IN_WARMUP',
                        'E_SYNC_STALLED', 'errno 111', 'status=503', 'SQLITE_BUSY', 'non-mandatory-script-verify-flag']),
    lambda r: f'height {r.randrange(700000, 900000)}',
    lambda r: r.choice(['electrum.blockstream.info:50002', 'mempool.space', 'node.example.onion:8332', '[2001:db8::1]:9735', 'btcpay.example.org']),
    lambda r: f'block {r.randrange(700000, 900000)} hash 00000000000000000{"".join(r.choice("0123456789abcdef") for _ in range(47))}',
]

CORE_KEYS = ['password', 'passphrase', 'secret', 'token', 'api_key', 'apiKey', 'API_KEY', 'access_token', 'refresh_token', 'client_secret',
             'rpcpassword', 'rpcauth', 'macaroon', 'private_key', 'secret_key', 'auth_token', 'authToken', 'bearer', 'credentials',
             'seed', 'mnemonic', 'recovery_phrase', 'wallet_passphrase', 'pwd', 'passwd', 'db_password', 'x-api-key', 'X-Auth-Token']
EMBED_PRE = ['', 'btcpay', 'BTCPAY', 'lnd', 'electrum', 'nostr', 'aws', 'stripe', 'my', 'app', 'kassiber', 'kraken', 'bitstamp', 'admin', 'Wallet']
NUM_KEYS = ['pin', 'otp', 'wallet_pin', 'device_pin', 'user_pin', 'login_otp', 'PIN', 'OTP']  # documented final-part aliases
NUM_OOS_KEYS = ['pin_code', 'otp_code', 'totp', 'totp_code', 'mfa_code', 'twofa', '2fa_code', 'verification_code', 'pincode']
COUNT_SECRET_KEYS = ['token_num', 'api_key_num', 'token_ttl', 'session_token_max', 'pin_len']


def embed(rnd, key):
    pre = rnd.choice(EMBED_PRE)
    if not pre:
        return key
    style = rnd.randrange(5)
    if style == 0: return pre + '_' + key
    if style == 1: return (pre + '_' + key).upper()
    if style == 2: return pre.lower() + ''.join(p[:1].upper() + p[1:] for p in key.replace('-', '_').split('_'))
    if style == 3: return pre.lower() + '.' + key
    return pre.lower() + '-' + key.replace('_', '-')


QUOTES = [('"', '"'), ("'", "'"), ('\\"', '\\"'), ('\\\\\\"', '\\\\\\"'), ('&quot;', '&quot;'), ('\u201c', '\u201d'), ('`', '`'), ('%22', '%22')]
NL = ['\n', '\r\n', '\r', '\u2028', '\u0085']
SCHEMES = ['https', 'http', 'electrum', 'ssl', 'tcp', 'socks5h', 'postgresql', 'mysql', 'redis', 'amqps', 'git+ssh', 'wss', 'mongodb+srv',
           'ftp', 'lndconnect', 'HTTPS', 'Http']
HOSTS = ['node.example.org', 'electrum.blockstream.info:50002', '127.0.0.1:8332', '[::1]:10009', 'abcdefghijklmnop.onion:50001',
         'bücher.example', 'xn--bcher-kva.example:443', 'localhost', 'btcpay.example.org']
USERS = ['alice', 'rpcuser', 'admin', 'bob.smith', 'user%40example.com', 'u-1', 'électrum', '__cookie__', 'x']


def rec_url(rnd, s):
    scheme = rnd.choice(SCHEMES)
    host = rnd.choice(HOSTS)
    form = rnd.randrange(6)
    user = rnd.choice(USERS)
    if form == 0: auth = f'{user}:{s}'
    elif form == 1: auth = s  # token as username
    elif form == 2: auth = f'{user}:{quote(s, safe="")}'
    elif form == 3: auth = f':{s}'
    elif form == 4: auth = f'{user}:{s}:{rnd.choice(["x", "9"])}'
    else: auth = f'{user}:{s.replace("9", "%39")}'
    path = rnd.choice(['', '/', '/api/v1/invoices', '/wallet/default?verbose=1', '#frag', '/db?sslmode=require'])
    url = f'{scheme}://{auth}@{host}{path}'
    wrap = rnd.randrange(8)
    if wrap == 0: url = f'<{url}>'
    elif wrap == 1: url = f'[docs]({url})'
    elif wrap == 2: url = f'"{url}"'
    elif wrap == 3: url = f'({url})'
    elif wrap == 4: url = url.replace('/', '\\/')
    elif wrap == 5: url = quote(url, safe='')
    pre = rnd.choice(['connecting to ', 'Error: cannot reach ', 'server=', 'ELECTRUM_SERVER=', 'url: ', '', 'proxy -> ', 'GET '])
    return pre + url, True


def rec_schemeless(rnd, s):
    host = rnd.choice(HOSTS)
    user = rnd.choice(['alice', 'rpcuser', 'proxyuser', 'u'])
    key = rnd.choice(['https_proxy', 'ALL_PROXY', 'DATABASE_URL', 'dsn', 'proxy', 'server', 'upstream', 'electrum_server', ''])
    val = f'{user}:{s}@{host}'
    if key:
        return f'{key}={val}', True
    return f'proxy {val} refused', True


def quoted(rnd, s):
    o, c = rnd.choice(QUOTES)
    return o + s + c


def rec_json(rnd, s):
    key = embed(rnd, rnd.choice(CORE_KEYS))
    form = rnd.randrange(7)
    if form == 0: return json.dumps({'id': rnd.randrange(100), key: s, 'network': 'mainnet'}), True
    if form == 1: return json.dumps({'error': {'code': -32603, 'data': {key: s}}}, indent=rnd.choice([None, 2])), True
    if form == 2: return repr({key: s, 'height': 812000}), True
    if form == 3: return json.dumps(json.dumps({key: s})), True  # double-encoded
    if form == 4: return '{' + f'"{key}"\u00a0:\u00a0"{s}"' + '}', True
    if form == 5: return json.dumps({key: s}, ensure_ascii=True).replace(key[0], '\\u%04x' % ord(key[0]), 1), True
    return '{' + f'{key}: "{s}", keep: "yes"' + '}', True


def rec_yaml(rnd, s):
    key = embed(rnd, rnd.choice(CORE_KEYS))
    nl = rnd.choice(NL)
    form = rnd.randrange(6)
    ind = rnd.choice(['  ', '    ', '\t'])
    if form == 0: return f'config:{nl}{ind}{key}: {s}', True
    if form == 1: return f'{key}: |{nl}{ind}{s}{nl}{ind}{s[::-1]}', True
    if form == 2: return f'{key}: >-{nl}{ind}{s}', True
    if form == 3: return f'- {key}: {quoted(rnd, s)}', True
    if form == 4: return f'env:{nl}- name: {key.upper().replace("-", "_").replace(".", "_")}{nl}  value: {s}', True
    return f'{key}:{nl}{ind}{s}', True


def rec_xml(rnd, s):
    key = rnd.choice(['password', 'Password', 'apiKey', 'token', 'rpcpassword', 'secret', 'passphrase'])
    form = rnd.randrange(6)
    if form == 0: return f'<{key}>{s}</{key}>', True
    if form == 1: return f'<ns:{key} type="x">{s}</ns:{key}>', True
    if form == 2: return f'<add key="{key}" value="{s}"/>', True
    if form == 3: return f'<{key}><![CDATA[{s}]]></{key}>', True
    if form == 4: return f'<entry name="{key}">{s}</entry>', True
    return f'&lt;{key}&gt;{s}&lt;/{key}&gt;', True


def rec_cli(rnd, s):
    form = rnd.randrange(10)
    if form == 0: return f'bitcoin-cli -rpcuser=alice -rpcpassword={s} getblockchaininfo', True
    if form == 1: return f'bitcoind -rpcauth=alice:{s[:6]}${s} -prune=550', True
    if form == 2: return f'curl -u alice:{s} https://btcpay.example.org/api/v1/stores', True
    if form == 3: return f"curl -H 'Authorization: Bearer {s}' https://api.example.com/v1", True
    if form == 4: return f'docker run -e BTCPAY_API_KEY={s} -e NETWORK=mainnet btcpay', True
    if form == 5: return f'lncli --macaroon-hex {s} getinfo', True
    if form == 6: return f'kassiber backends set --name core --password {quoted(rnd, s)} --verbose', True
    if form == 7: return f'electrum --rpcpassword {s}', True
    if form == 8: return f'python -m tool --api-key={s}', True
    return f'$ env AUTH_TOKEN={s} node app.js', True


def rec_env(rnd, s):
    key = embed(rnd, rnd.choice(CORE_KEYS)).replace('-', '_').replace('.', '_')
    form = rnd.randrange(7)
    if form == 0: return f'export {key.upper()}={s}', True
    if form == 1: return f'{key.upper()}="{s}"', True
    if form == 2: return f'set {key.upper()}={s}', True
    if form == 3: return f'$env:{key.upper()} = "{s}"', True
    if form == 4: return f'{key}={s} # rotated 2026-10', True
    if form == 5: return f'{key.upper()}\uff1d{s}', True  # fullwidth equals
    return f'{key} = {s}', True


def rec_trace(rnd, s):
    key = rnd.choice(['password', 'api_key', 'token', 'passphrase'])
    nl = rnd.choice(NL[:3])
    return (f'Traceback (most recent call last):{nl}  File "kassiber/backends.py", line 88, in connect{nl}'
            f'    client = Client(host, {key}={quoted(rnd, s)}){nl}ConnectionError: {rnd.choice(["ECONNREFUSED", "timed out"])}'), True


def rec_provider(rnd, s):
    form = rnd.randrange(5)
    if form == 0:
        return json.dumps({'error': {'message': f'Incorrect API key provided: {s}. You can find your API key at https://example.com.', 'type': 'invalid_request_error'}}), False
    if form == 1:
        return f'401 Unauthorized: invalid x-api-key {s}', False
    if form == 2:
        return f'BTCPay: Greenfield error {{"code":"unauthenticated","message":"token {s} is not valid"}}', False
    if form == 3:
        return f'lnd: rpc error: code = Unknown desc = verification failed: signature mismatch after caveat verification (macaroon {s})', False
    return f'electrum: {{"jsonrpc":"2.0","error":{{"code":-32600,"message":"bad auth {s}"}}}}', False


def rec_path_token(rnd, s):
    form = rnd.randrange(3)
    if form == 0: return f'POST https://api.telegram.org/bot8612345:{s}/sendMessage failed', False
    if form == 1: return f'webhook https://hooks.slack.com/services/T0001/B0002/{s} 404', False
    return f'GET https://api.example.com/v1/data?access_token={s}&page=2', True


def rec_numeric(rnd, n):
    if rnd.random() < 0.5:
        key = rnd.choice(NUM_KEYS)
        sep = rnd.choice(['=', ': ', '="', ' = '])
        return f'{key}{sep}{n}{chr(34) if sep.endswith(chr(34)) else ""}', True
    if rnd.random() < 0.5:
        key = rnd.choice(NUM_OOS_KEYS)
        return f'{key}={n}', False
    key = rnd.choice(COUNT_SECRET_KEYS)
    return f'{key}={n}', False


def rec_passphrase(rnd, s):
    words = [s] + [sentinel(rnd) for _ in range(rnd.randrange(1, 3))]
    key = embed(rnd, rnd.choice(['passphrase', 'password', 'wallet_passphrase', 'seed_passphrase']))
    return f'{key}={" ".join(words)}', True, words


FAMILIES = [rec_url, rec_schemeless, rec_json, rec_yaml, rec_xml, rec_cli, rec_env, rec_trace, rec_provider, rec_path_token]
NOISE = ['\u200b', '\u200e', '\u00a0', '\ufeff', '\u202f']


def make(i, rnd):
    case = {'kind': None, 'secrets': [], 'numeric': [], 'keep': [], 'soft': [], 'oos': False}
    parts = []
    nrec = rnd.randrange(1, 5)
    kinds = []
    for _ in range(nrec):
        before_s, before_n = len(case['secrets']), len(case['numeric'])
        r = rnd.random()
        if r < 0.55:
            fam = rnd.choice(FAMILIES)
            s = sentinel(rnd, special=rnd.random() < 0.25 and fam not in (rec_xml,))
            text, contract = fam(rnd, s)
            illegal = fam in (rec_url, rec_schemeless) and any(ch in s for ch in '#/?"<]\\`|^')
            case['secrets'].append([s, fam.__name__[4:] + ('' if contract else '-oos') + ('-illegalchar' if illegal else '')])
            kinds.append(fam.__name__[4:])
        elif r < 0.62:
            n = numeric(rnd)
            text, contract = rec_numeric(rnd, n)
            case['numeric'].append([n, 'numeric:' + text.split(n)[0].rstrip('=:" ')])
            kinds.append('numeric')
        elif r < 0.67:
            s = sentinel(rnd)
            text, contract, words = rec_passphrase(rnd, s)
            case['secrets'].extend([w, 'passphrase'] for w in words)
            kinds.append('passphrase')
        else:
            k = rnd.choice(KEEP_GEN)(rnd)
            text = rnd.choice(['sync ok ', 'saw ', 'wrote ', 'tx ', 'fee ', 'error: ', '']) + k
            case['keep'].append(k)
            contract = True
            kinds.append('keep')
        if not contract:
            case['oos'] = True
        if rnd.random() < 0.08:
            pos = rnd.randrange(len(text) + 1)
            text = text[:pos] + rnd.choice(NOISE) + text[pos:]
            for item in case['secrets'][before_s:] + case['numeric'][before_n:]:
                item[1] += '~noise'
        parts.append(text)
    out = parts[0]
    for p in parts[1:]:
        out += rnd.choice(['\n', '\n', '\r\n', '; ', ' | ', ', ', '\t', ' ', '\u2028']) + p
    # keeps on the same line after an unquoted secret are a documented cost
    case['kind'] = '+'.join(kinds)
    case['input'] = out
    return case


def cases(count, seed):
    rnd = random.Random(seed)
    for i in range(count):
        case = make(i, rnd)
        case['i'] = i
        yield case


if __name__ == '__main__':
    for case in cases(int(sys.argv[1]), int(sys.argv[2])):
        print(json.dumps(case, ensure_ascii=True))
