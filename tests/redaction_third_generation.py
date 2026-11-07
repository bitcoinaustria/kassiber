"""Reviewer 5e generator: independent of tests/redaction_*generation.py and 5d adv/gen.py.

Emphasis: several schemes per whitespace-free token, the full RFC 3986 userinfo
alphabet, non-ASCII adjacency, mixed line terminators, percent / JSON / slash
escaped variants, and long inputs.  Each case carries an uppercase sentinel that
only appears inside a credential position.
Adopted from reviewer 5e gen/gen3.py; only the CLI now streams JSONL.
usage: redaction_third_generation.py COUNT SEED
"""
import json
import random
import sys
from urllib.parse import quote

ALPHA = 'BCDFGHJKMNPRTVWXYZ'


def sentinel(i, rnd):
    body = ''
    n = i
    for _ in range(6):
        body += ALPHA[n % len(ALPHA)]
        n //= len(ALPHA)
    return 'ZK' + body + rnd.choice(ALPHA) + 'KZ'


UNRESERVED = 'abcdefghijklmnopqrstuvwxyz0123456789-._~'
SUBDELIMS = "!$&'()*+,;="
PCT = ['%40', '%3A', '%2F', '%25', '%20', '%0A', '%3a', '%2f', '%E2%82%AC', '%5C', '%22', '%23', '%3F']
ILLEGAL = list('@/?#[]\\"<>{}|^`') + ['\r', '\n', '\r\n', ' ', '\u0085', ' ', 'é', '接', '😀', '​', ' ']
SCHEMES = ['http', 'https', 'HTTP', 'Https', 'ssl', 'tcp', 'wss', 'socks5h', 'electrum', 'postgresql', 'mysql', 'redis',
           'mongodb+srv', 'git+ssh', 'svn+ssh', 'coap+tcp', 'a.b-c', 'x1', 'z', 's3', 'amqps', 'ftp', 'ldap', 'smb']
HOSTS = ['h', 'node.example', '127.0.0.1', '[::1]', '[fe80::1%25eth0]', 'bücher.example', 'xn--bcher-kva.example',
         'abc.onion', 'localhost', '', 'H.EXAMPLE']
PORTS = ['', ':8332', ':50002', ':', ':0', ':65535']
TAILS = ['', '/', '/api/v1', '/?x=1', '?a=b&c=d', '#frag', '/a@b', '/p;q=1', '/%40x', '/?u=a@b.c', '/x/y.git@v1']
LIST_SEP = [',', ';', '|', '/', '', '?next=', '#', '&u=', '\\/', '%2C', '","', "','", '),(', ']', '[', '>', '<', '=', ':', '!', '+',
            '.', '-', '_', '%7C', '\\\\', '\\"']
WS_SEP = [' ', '\t', '\n', '\r\n', '\r', ' ', ' ', '\u0085', ' ', '　', '\v', '\f']
ADJ = ['', '', '', 'é', '接', 'ſ', 'K', 'ß', 'İ', '​', '­', '😀', '́', '‏', 'ё', 'ا',
       '.', '-', '+', '_', '1', '(', '"', "'", '=', ':', '<', '[', '{', '`', '\\', '/', '@', '%', '&', '*']
LINES = ['\n', '\r\n', '\r', ' ', ' ', '\u0085']
KEYS = ['password', 'Password', 'PASSWORD', 'rpcpassword', 'rpcPassword', 'RPC_PASSWORD', 'bitcoin.rpcpassword', 'db-password',
        'api_key', 'apiKey', 'X-Api-Key', 'token', 'access_token', 'refreshToken', 'authToken', 'secret', 'client_secret',
        'clientSecret', 'macaroon', 'macaroon_hex', 'rune', 'passphrase', 'walletPassphrase', 'privkey', 'private_key',
        'secretKey', 'rpcauth', 'Authorization', 'Proxy-Authorization', 'Cookie', 'Set-Cookie', 'mnemonic', 'seed_words', 'xprv']
SEPS = ['=', ': ', ':', ' = ', '\t=\t', ' => ', ' := ', '=>', ':=', ' : ', '= ', ' =']
VALPUNCT = list("&|,;{}()[]'\"<>!$*+=:/?#@~^`") + ['-', '--', ' ', '\\', '%26', '%7C']
NONSECRETS = ['9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08', '123e4567-e89b-12d3-a456-426614174000',
              'node.example.org', 'b1946ac92492d2347c6235b4d2611184', 'height 800000 synced', 'Use --password to set a value.']


def userinfo_chars(rnd, s, illegal_p):
    out = []
    for _ in range(rnd.randint(0, 4)):
        r = rnd.random()
        if r < illegal_p:
            out.append(rnd.choice(ILLEGAL))
        elif r < 0.45:
            out.append(rnd.choice(SUBDELIMS))
        elif r < 0.6:
            out.append(':')
        elif r < 0.75:
            out.append(rnd.choice(PCT))
        else:
            out.append(rnd.choice(UNRESERVED))
    cut = rnd.randint(0, len(out))
    return ''.join(out[:cut]) + s + ''.join(out[cut:])


def url_item(rnd, s=None, illegal_p=0.0):
    scheme = rnd.choice(SCHEMES)
    host = rnd.choice(HOSTS) + rnd.choice(PORTS) + rnd.choice(TAILS)
    if s is None:
        if rnd.random() < 0.3:
            user = ''.join(rnd.choice(UNRESERVED) for _ in range(rnd.randint(1, 6)))
            return f'{scheme}://{user}@{host}'
        return f'{scheme}://{host}'
    where = rnd.random()
    user = ''.join(rnd.choice(UNRESERVED + SUBDELIMS) for _ in range(rnd.randint(0, 6)))
    if where < 0.7:
        ui = user + ':' + userinfo_chars(rnd, s, illegal_p)
    elif where < 0.85:
        ui = userinfo_chars(rnd, s, illegal_p)  # sentinel in user part / token-only userinfo
    else:
        ui = userinfo_chars(rnd, s, illegal_p) + ':' + userinfo_chars(rnd, '', illegal_p)
    return f'{scheme}://{ui}@{host}'


def encode(rnd, text):
    """Return (variant_name, encoded_text)."""
    r = rnd.randrange(12)
    if r < 4:
        return 'plain', text
    if r == 4:
        return 'pct1', quote(text, safe='')
    if r == 5:
        return 'pct2', quote(quote(text, safe=''), safe='')
    if r == 6:
        return 'pct3', quote(quote(quote(text, safe=''), safe=''), safe='')
    if r == 7:
        return 'pctlow', quote(text, safe='').lower().replace('%', '%')  # lowercase hex AND lowercased letters
    if r == 8:
        return 'slash', text.replace('/', '\\/')
    if r == 9:
        k = rnd.randint(1, 3)
        t = text
        for _ in range(k):
            t = json.dumps(t, ensure_ascii=rnd.random() < 0.5)[1:-1]
        return f'json{k}', t
    if r == 10:
        return 'slash2', text.replace('/', '\\\\/')
    return 'pctmix', text.replace(':', '%3A', 1).replace('@', '%40')


def gen_url(rnd, s):
    k = rnd.randint(1, 5)
    hot = set(rnd.sample(range(k), rnd.randint(1, min(2, k))))
    illegal_p = 0.0 if rnd.random() < 0.6 else 0.15
    items = [url_item(rnd, s if j in hot else None, illegal_p) for j in range(k)]
    tok = items[0]
    for it in items[1:]:
        tok += (rnd.choice(LIST_SEP) if rnd.random() < 0.8 else rnd.choice(WS_SEP)) + it
    variant, tok = encode(rnd, tok)
    if variant == 'pctlow':
        variant = 'pctlow'  # sentinel lowercased too: check case-insensitively
    pre = rnd.choice(ADJ) + rnd.choice(['', '', 'url=', 'servers=', '{"url":"', "'", '(', '<', 'GET ', 'proxy: ', '- ', '"u":"', 'ELECTRUM_URL='])
    post = rnd.choice(['', '', '"', "'", ')', '>', '"}', ',', ';', '.', ':', ' ok', '&y=2', '|x', '\\n']) + rnd.choice(ADJ)
    legal = illegal_p == 0.0
    return f'url-{"legal" if legal else "illegal"}-{variant}', pre + tok + post


def gen_key(rnd, s):
    key = rnd.choice(KEYS)
    sep = rnd.choice(SEPS)
    v = ''
    for _ in range(rnd.randint(0, 3)):
        v += rnd.choice(VALPUNCT)
    cut = rnd.randint(0, len(v))
    val = v[:cut] + s + v[cut:] if rnd.random() < 0.6 else s
    shape = rnd.randrange(14)
    if shape == 0: text = f'{key}{sep}{val}'
    elif shape == 1: text = f'{rnd.choice(["--", "-"])}{key} {val}'
    elif shape == 2: text = f'{rnd.choice(["--", "-"])}{key}={val}'
    elif shape == 3: text = json.dumps({key: val}, ensure_ascii=rnd.random() < 0.5, separators=(',', ':'))
    elif shape == 4:
        t = json.dumps({'url': 'http://h:8332', key: val, 'n': 1}, ensure_ascii=False)
        for _ in range(rnd.randint(1, 3)): t = json.dumps(t, ensure_ascii=False)
        text = t
    elif shape == 5: text = f'{rnd.choice(SCHEMES)}://h{rnd.choice(PORTS)}/?a=1&{key}={val}&b=2'
    elif shape == 6: text = f"{{'{key}': '{val}'}}"
    elif shape == 7: text = f'kassiber backends set --{key} {json.dumps(val)}'
    elif shape == 8: text = f'{key}{sep}"{val}"'
    elif shape == 9: text = f'<{key}>{val}</{key}>'
    elif shape == 10: text = f'{key}{sep}{val}{rnd.choice(LINES)}next=1'
    elif shape == 11: text = f'{rnd.choice(["x", "a b", "- ", "  "])}{rnd.choice(LINES)}{key}{sep}{val}'
    elif shape == 12: text = f'{key}{sep}' + quote(val, safe='')
    else: text = f'{rnd.choice(ADJ)}{key}{sep}{val}{rnd.choice(ADJ)}'
    return f'key{shape}', text


def gen_long(rnd, s):
    shape = rnd.randrange(8)
    n = rnd.randint(5_000, 40_000)
    if shape == 0: text = ('http://a.example,' * (n // 17)) + f'http://alice:{s}@h/'
    elif shape == 1: text = f'http://alice:{s}' + ('%25' * (n // 3)) + '@h'
    elif shape == 2: text = ('\\' * n) + f'/\\/alice:{s}@h'
    elif shape == 3: text = ('password=' * (n // 9)) + s
    elif shape == 4: text = ('a@' * (n // 2)) + f' http://u:{s}@h'
    elif shape == 5: text = f'Authorization: Basic {s}' + ('\r\n ' + 'x' * 50) * (n // 53)
    elif shape == 6: text = ('x' * n) + f'http://u:{s}@h' + ('y' * n)
    else: text = (rnd.choice(LINES).join(['line %d ok' % j for j in range(n // 10)])) + f'\ntoken={s}'
    return f'long{shape}', text


def cases(count, seed):
    rnd = random.Random(seed)
    for i in range(count):
        s = sentinel(i, rnd)
        r = rnd.random()
        if r < 0.55:
            kind, text = gen_url(rnd, s)
        elif r < 0.995:
            kind, text = gen_key(rnd, s)
        else:
            kind, text = gen_long(rnd, s)
        keep = NONSECRETS[i % len(NONSECRETS)]
        term = rnd.choice(LINES)
        yield {'i': i, 'kind': kind, 'input': text + term + 'status ' + keep, 'sentinel': s, 'keep': keep}


if __name__ == '__main__':
    for case in cases(int(sys.argv[1]), int(sys.argv[2])):
        print(json.dumps(case, ensure_ascii=True))
