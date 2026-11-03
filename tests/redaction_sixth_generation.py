"""Reviewer 5h generator (sixth), independent of gen1-gen5.

Targets the 5h rules: word segmentation and its tie-break, case runs, the
`-flag` value end, the URL line cut and its `@`-token exception, provider-body
literals, new aliases, chat-service tokens.

Secrets are built from the letters FHMNRTW (any case) plus one '7', so no
4-character fragment occurs in support data.  Numeric secrets start with 73
and are checked as exact digit runs.

case: {kind, cls, input, secrets:[[s, tag]], numeric:[[n, tag]], keep:[k]}
  cls: 'in'   -- inside the documented contract (any leak is a finding)
       'oos'  -- outside it per logging.md (informational)
       'base' -- name segmentation outside the 5h contract that 2addbf459 masked
Adopted from reviewer 5h gen/gen6.py; only the CLI now streams JSONL. Round 5i
brought the 'oos' and 'base' shapes inside the contract.
usage: redaction_sixth_generation.py COUNT SEED
"""
import json
import random
import sys
from urllib.parse import quote

ALPHA = 'FHMNRTWfhmnrtw'


def sentinel(r, special=False, words=False):
    s = ''.join(r.choice(ALPHA) for _ in range(r.randint(4, 7))) + '7' + ''.join(r.choice(ALPHA) for _ in range(r.randint(4, 7)))
    if special:
        p = r.randrange(2, len(s) - 2)
        s = s[:p] + r.choice(list("#@:/%+=&!$*,;~^|?.-_") + ['"', "'"]) + s[p:]
    if words:
        s = s + ' ' + ''.join(r.choice(ALPHA) for _ in range(5)) + '7' + ''.join(r.choice(ALPHA) for _ in range(3))
    return s


def pin(r):
    return '73' + ''.join(r.choice('0123456789') for _ in range(r.choice([2, 4, 6])))


def txid(r):
    return ''.join(r.choice('0123456789abcdef') for _ in range(64))


def support(r):
    return r.choice([
        lambda: txid(r),
        lambda: f'{r.randint(0, 20)}.{r.randint(0, 99999999):08d} BTC',
        lambda: f'{r.randint(1, 999999)} sats',
        lambda: r.choice(['/var/lib/kassiber/books/main.sqlite', '/home/alice/.bitcoin/debug.log', '~/.lnd/tls.cert', 'C:\\Users\\alice\\kassiber']),
        lambda: r.choice(['RPC_IN_WARMUP', 'E_SYNC_TIMEOUT', 'HTTP 503', 'code=-32603', 'ECONNREFUSED']),
        lambda: f'height {r.randint(700000, 900000)}',
        lambda: r.choice(['mempool.space', 'electrum.blockstream.info:50002', 'btcpay.example.org']),
    ])()


# --- names -----------------------------------------------------------------
SEP_SECRET = ['password', 'passphrase', 'passwd', 'pwd', 'secret', 'token', 'mnemonic', 'macaroon', 'seed',
              'cookie', 'credential', 'credentials', 'rpcauth', 'rune', 'xprv', 'passcode', 'jwt', 'session', 'sid',
              'signature', 'bearer']
LAST_ONLY = ['pin', 'otp', 'totp', 'pw']
COMPOUNDS = [('api', 'key'), ('private', 'key'), ('secret', 'key'), ('seed', 'words'), ('seed', 'phrase'),
             ('recovery', 'phrase'), ('session', 'id'), ('pin', 'code'), ('otp', 'code'), ('mfa', 'code'),
             ('verification', 'code'), ('spend', 'key'), ('scan', 'key'), ('master', 'key'), ('encryption', 'key'),
             ('signing', 'key'), ('api', 'secret'), ('api', 'token'), ('auth', 'code'), ('priv', 'key')]
QUAL = ['rpc', 'db', 'wallet', 'admin', 'api', 'btcpay', 'lnd', 'kraken', 'root', 'main', 'old', 'new', 'prod',
        'client', 'access', 'refresh', 'hex', 'raw', 'electrum', 'mempool', 'strike', 'my', 'local', 'remote', 'web']
UNKNOWN_PRE = ['btc', 'sql', 'ftp', 'vpn', 'node', 'hd', 'ln', 'bitcoind', 'nwc', 'boltz', 'alby', 'liquid',
               'sparrow', 'specter', 'trezor', 'zeus', 'phoenix', 'bisq', 'joinmarket', 'tor']
UNKNOWN_SUF = ['backup', 'confirm', 'repeat', 'string', 'json', 'full', 'copy', 'tmp', 'cache', 'list', 'export']
STRONG = {'password', 'passphrase', 'passwd', 'secret', 'token', 'mnemonic', 'macaroon', 'cookie', 'credential',
          'credentials', 'rpcauth', 'passcode', 'bearer', 'pwd', 'xprv'}


def style(words, r, joined):
    """Spell a word list; joined spellings concatenate, others separate."""
    if joined:
        mode = r.choice(['lower', 'upper', 'caserun', 'capfirst'])
        if mode == 'lower':
            return ''.join(words)
        if mode == 'upper':
            return ''.join(words).upper()
        if mode == 'capfirst':
            return ''.join(words).capitalize()
        # case run: one word upper inside a lowercase run (rpcPASSWORDold)
        i = r.randrange(len(words))
        return ''.join(w.upper() if j == i else w for j, w in enumerate(words))
    mode = r.choice(['snake', 'kebab', 'dot', 'camel', 'pascal', 'screaming', 'train'])
    if mode == 'snake':
        return '_'.join(words)
    if mode == 'kebab':
        return '-'.join(words)
    if mode == 'dot':
        return '.'.join(words)
    if mode == 'camel':
        return words[0] + ''.join(w.capitalize() for w in words[1:])
    if mode == 'pascal':
        return ''.join(w.capitalize() for w in words)
    if mode == 'train':
        return '-'.join(w.capitalize() for w in words)
    return '_'.join(words).upper()


def secret_name(r):
    """(name, cls, numeric_ok)."""
    shape = r.random()
    if shape < 0.12:
        a, b = r.choice(COMPOUNDS)
        pre = [r.choice(QUAL)] if r.random() < 0.4 else []
        words = pre + [a, b]
        return style(words, r, r.random() < 0.4), 'in', a in ('pin', 'otp', 'mfa', 'verification', 'auth')
    if shape < 0.22:
        w = r.choice(LAST_ONLY)
        pre = [r.choice(QUAL)] if r.random() < 0.6 else []
        return style(pre + [w], r, bool(pre) and r.random() < 0.4), 'in', True
    w = r.choice(SEP_SECRET)
    if shape < 0.62:
        # separated: whole secret word part among qualifiers / unknown words
        pre = [r.choice(QUAL + UNKNOWN_PRE) for _ in range(r.choice([0, 1, 1, 2]))]
        post = [r.choice(QUAL + UNKNOWN_SUF) for _ in range(r.choice([0, 0, 1]))]
        if w in ('jwt', 'session', 'sid', 'signature') and post:
            post = []
        return style(pre + [w] + post, r, False), 'in', False
    if w in ('jwt', 'session', 'sid', 'signature', 'bearer'):
        w = 'password'
    # concatenated
    kind = r.random()
    if kind < 0.45:
        pre = [r.choice(QUAL) for _ in range(r.choice([1, 1, 2]))]
        post = [r.choice(['old', 'new', 'hex', 'raw', 'value', 'enc', 'prod'])] if r.random() < 0.3 else []
        return style(pre + [w] + post, r, True), 'in', False
    if kind < 0.7:
        pre = [r.choice(UNKNOWN_PRE)]
        cls = 'in' if w in STRONG else 'base'
        return style(pre + [w], r, True), cls, False
    pre = [r.choice(QUAL)] if r.random() < 0.5 else []
    return style(pre + [w, r.choice(UNKNOWN_SUF)], r, True), 'base', False


ORDINARY = [
    ('prune', '550'), ('pruned', 'true'), ('pruneheight', '700000'), ('dnsseed', '0'), ('seednode', 'seed.bitcoin.sipa.be'),
    ('bypass_cache', 'true'), ('passive', 'false'), ('passed', '12'), ('tokenizer', 'cl100k_base'),
    ('sighash', 'ALL'), ('sighash_type', '1'), ('author', 'alice'), ('spinner', 'dots'), ('compass', 'north'),
    ('pinned', 'true'), ('password_hint', 'dog'), ('rpccookiefile', '/srv/btc/.cookie'),
    ('macaroonpath', '/home/u/.lnd/admin.macaroon'), ('max_tokens', '4096'), ('token_count', '12'),
    ('session_ttl', '3600'), ('descriptors', 'true'), ('private_keys_enabled', 'false'), ('seeders', '3'),
    ('passthrough', 'on'), ('tokens_used', '512'), ('keypool', '1000'), ('keypoolsize', '1000'), ('wallet_name', 'cold'),
    ('user_count', '3'), ('authority', 'ca.example.org'), ('signer', 'hwi'), ('pinentry', 'curses'),
]


def assign(name, value, r, quotey=None):
    f = r.choice(['eq', 'colon', 'json', 'yaml', 'spaced', 'env', 'repr', 'xml'] if quotey is None else [quotey])
    if f == 'eq':
        return f'{name}={value}'
    if f == 'colon':
        return f'{name}: {value}'
    if f == 'json':
        return json.dumps({name: value})
    if f == 'repr':
        return repr({name: value})
    if f == 'yaml':
        return f'{name}: "{value}"'
    if f == 'spaced':
        return f'{name} = {value}'
    if f == 'env':
        return f'export {name}={value}'
    return f'<{name}>{value}</{name}>'


# --- record families --------------------------------------------------------
def rec_name(r, c):
    name, cls, numeric_ok = secret_name(r)
    if numeric_ok and r.random() < 0.6:
        n = pin(r)
        c['numeric'].append([n, f'name:{cls}'])
        value = n
        rec = assign(name, value, r, r.choice(['eq', 'colon', 'yaml', 'env']))
        if r.random() < 0.3:
            rec = '{' + json.dumps(name) + ': ' + n + '}'
    else:
        s = sentinel(r, special=r.random() < 0.25)
        c['secrets'].append([s, f'name:{cls}'])
        rec = assign(name, s, r)
    c['names'].append([name, cls])
    return rec


def rec_ordinary(r, c):
    name, value = r.choice(ORDINARY)
    if r.random() < 0.5:
        name = style(name.replace('-', '_').split('_'), r, False) if '_' in name else name
    rec = assign(name, value, r, r.choice(['eq', 'colon', 'json', 'env']))
    c['keep'].append([rec, 'ordinary'])
    c['ordinary'].append(name)
    return rec


def rec_flags(r, c):
    cmd = r.choice(['bitcoin-cli', 'bitcoind', 'lncli', 'kassiber backends set', 'electrs', 'lightning-cli', 'curl', 'docker run'])
    keepflag = r.choice(['-prune=550', '--network=mainnet', '-named', '--rpcserver=localhost:10009', '-txindex=1', '--verbose', '-v'])
    s = sentinel(r)
    flag = r.choice(['-rpcpassword', '--password', '--macaroon', '--token', '--api-key', '--rune', '--passphrase', '-rpcauth', '--seed-phrase'])
    form = r.random()
    sep = r.choice([' ', '  ', '\t'])
    if form < 0.4:
        rec = f'{cmd} {flag}={s}{sep}{keepflag}'
        c['secrets'].append([s, 'flag:eq'])
        c['keep'].append([keepflag, 'flag'])
    elif form < 0.65:
        rec = f'{cmd} {flag} {s}{sep}{keepflag}'
        c['secrets'].append([s, 'flag:space'])
        c['keep'].append([keepflag, 'flag'])
    elif form < 0.75:
        # value that itself starts with '-' (doc: consumed even when it begins with -)
        rec = f'{cmd} {flag} -{s}{sep}{keepflag}'
        c['secrets'].append([s, 'flag:dash-value'])
    elif form < 0.85:
        # multi-word passphrase whose second word begins with '-': can it be split off?
        s2 = sentinel(r)
        rec = f'{cmd} {flag} {s} -{s2} {keepflag}'
        c['secrets'].append([s, 'flag:split1'])
        c['secrets'].append([s2, 'flag:split2'])
    elif form < 0.93:
        # key assignment inside a recognized command line (env style)
        rec = f'{cmd} rpcpassword={s}{sep}{keepflag}'
        c['secrets'].append([s, 'flag:cmd-key'])
        c['keep'].append([keepflag, 'flag'])
    else:
        # shell continuation then flag
        rec = f'{cmd} {flag} {s} \\\n  {keepflag}'
        c['secrets'].append([s, 'flag:cont'])
    return rec


def rec_url(r, c):
    s = sentinel(r, special=r.random() < 0.2)
    user = r.choice(['alice', 'rpc', 'u', 'admin.ops', 'x-token'])
    host = r.choice(['node.example.org:8332', 'db.example.org', '10.0.0.5:50002', 'abc.onion'])
    scheme = r.choice(['https', 'http', 'postgresql', 'electrum', 'redis', 'amqps', 'socks5h'])
    enc = quote(s, safe='') if r.random() < 0.7 else s
    if '#' in s and enc == s:
        enc = quote(s, safe='')
    form = r.random()
    email = r.choice(['bob@example.org', 'ops@kassiber.example', '@alice'])
    if form < 0.35:
        rec = f'{scheme}://{user}:{enc}@{host}/path?x=1'
        c['secrets'].append([s, 'url:plain'])
    elif form < 0.55:
        # URL with path, newline, an email or handle on the next line
        rec = f'see https://docs.example.org/errors/E{r.randint(1, 99)}\n{r.choice(["", "  "])}contact {email}'
        c['keep'].append([email, 'url:email'])
        c['keep'].append(['docs.example.org/errors', 'url:public'])
    elif form < 0.7:
        # first token of the next line carries an email directly (the @-token exception)
        rec = f'see https://docs.example.org/errors/E{r.randint(1, 99)}\n{email} replied'
        c['keep'].append([email, 'url:email-token'])
    elif form < 0.85:
        # public URL, then credential URL on the next line
        rec = f'GET https://mempool.space/api/tx/{txid(r)}\n{scheme}://{user}:{enc}@{host}'
        c['secrets'].append([s, 'url:nextline'])
    else:
        # wrapped userinfo: break inside the password (authority not finished)
        cut = r.randrange(2, len(enc) - 1)
        rec = f'{scheme}://{user}:{enc[:cut]}\n{enc[cut:]}@{host}/x'
        c['secrets'].append([s, 'url:wrapped'])
    return rec


def rec_provider(r, c):
    body = {'error': {'message': r.choice(['Rate limit reached', 'context_length_exceeded', 'overloaded']),
                      'code': r.choice([429, 400, 529])},
            'usage': {'prompt_tokens': r.randint(1, 9000), 'completion_tokens': 0,
                      'prompt_tokens_details': {'cached_tokens': 0}},
            'seed': r.randint(0, 99999), 'max_tokens': r.choice([None, 1024]), 'stream': r.choice([True, False])}
    keeps = [f'"prompt_tokens": {body["usage"]["prompt_tokens"]}', '"cached_tokens": 0', f'"seed": {body["seed"]}']
    form = r.random()
    if form < 0.3:
        s = sentinel(r)
        body[r.choice(['api_key', 'session_token', 'access_token', 'x-api-key'])] = s
        c['secrets'].append([s, 'provider:str'])
    elif form < 0.55:
        n = pin(r)
        body[r.choice(['pin', 'otp', 'otp_code', 'pin_code', 'totp'])] = int(n)
        c['numeric'].append([n, 'provider:num'])
    elif form < 0.7:
        n = pin(r) + pin(r)
        body[r.choice(['api_key', 'token', 'secret', 'password'])] = int(n)
        c['numeric'].append([n, 'provider:num-secret'])
    text = json.dumps(body, indent=r.choice([None, 2]))
    if r.random() < 0.3:
        text = repr(body)
        keeps = [k.replace('"', "'") for k in keeps]
    c['keep'].extend([k, 'provider'] for k in keeps)
    return ('provider error body: ' if r.random() < 0.5 else '') + text


def rec_chat(r, c):
    s = sentinel(r)
    trail = support(r)
    form = r.random()
    cls = 'in'
    if form < 0.2:
        rec = f'POST https://api.telegram.org/bot{r.randint(100000, 9999999)}:{s}/sendMessage failed {trail}'
    elif form < 0.35:
        rec = json.dumps({'webhook': f'https://hooks.slack.com/services/T0{r.randint(1, 9)}AB/B0{r.randint(1, 9)}CD/{s}'}) + ' ' + trail
    elif form < 0.5:
        rec = f'[hook](https://discord.com/api/webhooks/{r.randint(10**15, 10**16)}/{s}) {trail}'
    elif form < 0.65:
        rec = f'slack auth {r.choice(["xoxb", "xoxp", "xoxa", "xoxs"])}-{r.randint(10, 99)}-{r.randint(10, 99)}-{s} {trail}'
    elif form < 0.75:
        rec = f'https://discord.com/api/v10/webhooks/{r.randint(10**15, 10**16)}/{s} {trail}'
        cls = 'oos'
    elif form < 0.85:
        rec = f'https://hooks.slack.com/workflows/T0AB/A0CD/{r.randint(1, 99999)}/{s} {trail}'
        cls = 'oos'
    else:
        rec = f'slack {r.choice(["xoxe-1", "xapp-1-A0", "xoxe.xoxp-1"])}-{s} {trail}'
        cls = 'oos' if 'xoxe.xoxp' not in rec else 'in'
    c['secrets'].append([s, f'chat:{cls}'])
    c['keep'].append([trail, 'chat-trail'])
    return rec


def rec_alias(r, c):
    s = sentinel(r)
    form = r.choice([
        lambda: f'GET https://maps.example.com/api?q=x&key={s} {support(r)}',
        lambda: f'Cookie: connect.sid={s}; theme=dark',
        lambda: f'{{"jwt": "{s}", "ok": false}}',
        lambda: f'X-Amz-Signature={s}',
        lambda: f'session: {s}',
        lambda: f'Passcode: {s}',
        lambda: f'credential={s}',
        lambda: f'sig={s}&se=2030',
        lambda: f'"Secret Key": "{s}"',
        lambda: f'API key: {s}',
        lambda: f'Recovery phrase: {s}',
    ])
    c['secrets'].append([s, 'alias'])
    return form()


def rec_support(r, c):
    k = support(r)
    c['keep'].append([k, 'support'])
    return r.choice(['sync ok ', 'wrote ', 'error: ', 'tx ', '']) + k


FAMILIES = [(rec_name, 30), (rec_ordinary, 14), (rec_flags, 12), (rec_url, 12), (rec_provider, 8), (rec_chat, 8),
            (rec_alias, 6), (rec_support, 10)]
JOINS = ['\n', '\r\n', ' | ', '; ', '\u2028', '\n  ', '\t']


def case(r):
    c = {'secrets': [], 'numeric': [], 'keep': [], 'names': [], 'ordinary': []}
    fams = r.choices([f for f, _ in FAMILIES], weights=[w for _, w in FAMILIES], k=r.choice([1, 1, 2, 3, 4]))
    recs = [f(r, c) for f in fams]
    text = recs[0]
    for rec in recs[1:]:
        text += r.choice(JOINS) + rec
    c['kind'] = '+'.join(f.__name__[4:] for f in fams)
    c['input'] = text
    return c


def cases(count, seed):
    r = random.Random(seed)
    for i in range(count):
        c = case(r)
        c['i'] = i
        yield c


if __name__ == '__main__':
    for c in cases(int(sys.argv[1]), int(sys.argv[2])):
        print(json.dumps(c, ensure_ascii=True))
