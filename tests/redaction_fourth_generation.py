"""Reviewer 5f generator (fourth, independent of gen1/gen2/gen3).

Targets: secret-name composition styles (camel/Pascal/kebab/dotted/concatenated),
decoded keyed detection (percent, entities, JSON escapes), XML element/attribute/
key-value forms, cookie forms, unclosed structured openers, documented exceptions
followed by a credential, and the credential-free URL readability rule
(credentials disguised as paths/handles, schemeless pairs, scheme spellings).
Sentinels use only the letters qxzjvkw (+ J/9X), so 4-char fragments never occur
in planted prose.  Kinds ending in '-ctl' have no secret; their 'keep' must
survive verbatim.
Adopted from reviewer 5f gen/gen4.py; only the CLI now streams JSONL.
usage: redaction_fourth_generation.py COUNT SEED
"""
import json
import random
import sys
from urllib.parse import quote

SA = 'qxzjvkw'


def sentinel(i, rnd, lower=False):
    body = ''
    n = i
    for _ in range(7):
        body += SA[n % 7]
        n //= 7
    if lower:
        return 'qz' + body + rnd.choice(SA)
    return 'Jq' + body + rnd.choice(SA) + '9X'


PREFIXES = ['', '', 'admin', 'new', 'db', 'pg', 'session', 'github', 'jwt', 'aws', 'client', 'wallet', 'rpc', 'my', 'bitcoin',
            'electrum', 'lnd', 'mysql', 'root', 'user', 'app', 'master', 'x']
CORES = [['password'], ['passwd'], ['pwd'], ['pass'], ['secret'], ['token'], ['api', 'key'], ['priv', 'key'], ['private', 'key'],
         ['passphrase'], ['credentials'], ['mnemonic'], ['seed'], ['macaroon'], ['auth', 'token'], ['access', 'token'],
         ['secret', 'key'], ['client', 'secret'], ['rpcauth'], ['seed', 'words'], ['recovery', 'phrase'], ['xprv'], ['wif']]
SUFFIXES = ['', '', '', '2', 'v2', 'value', 'hex', 'b64', 'pem', 'raw', 'enc', 'str', 'new', 'old']
STYLES = ['snake', 'camel', 'pascal', 'kebab', 'dot', 'upper', 'lowercat', 'uppercat', 'train', 'mixedcat']


def compose(parts, style):
    parts = [p for p in parts if p]
    if style == 'snake': return '_'.join(parts)
    if style == 'camel': return parts[0] + ''.join(p.capitalize() for p in parts[1:])
    if style == 'pascal': return ''.join(p.capitalize() for p in parts)
    if style == 'kebab': return '-'.join(parts)
    if style == 'dot': return '.'.join(parts)
    if style == 'upper': return '_'.join(parts).upper()
    if style == 'lowercat': return ''.join(parts)
    if style == 'uppercat': return ''.join(parts).upper()
    if style == 'train': return '-'.join(p.capitalize() for p in parts)
    return ''.join(p if i % 2 == 0 else p.upper() for i, p in enumerate(parts))


def keyname(rnd):
    pre, core, suf, style = rnd.choice(PREFIXES), rnd.choice(CORES), rnd.choice(SUFFIXES), rnd.choice(STYLES)
    concat = style in ('lowercat', 'uppercat') and (pre or suf or len(core) > 1)
    return compose([pre] + core + [suf], style), style, bool(pre), concat


CONTAINERS = ['k=v', 'k: v', 'k = v', '"k":"v"', '"k": "v"', "'k': 'v'", 'k="v"', "k='v'", 'export k=v', 'set k=v', '--k v', '--k=v',
              '<k>v</k>', '<ns:k>v</ns:k>', '<x k="v"/>', '  k: v', '- k: v', '# k: v', 'k := v', 'k => v', 'k -> v', '{k: v}', '[k=v]',
              '{"data":{"k":"v"}}', '\\"k\\":\\"v\\"', 'k\t=\tv', 'k==v', 'k: "v"', '?k=v', '&k=v']


def gen_key(rnd, s):
    k, style, pre, concat = keyname(rnd)
    c = rnd.choice(CONTAINERS)
    if c.startswith('--'):
        k = k.replace('_', '-').replace('.', '-')
    return f'key-{style}' + ('-cat' if concat else ''), c.replace('k', '\0').replace('v', s).replace('\0', k) if 'k' in c else c


def gen_key_safe(rnd, s):
    # Replace k/v placeholders without touching letters of the key itself.
    k, style, pre, concat = keyname(rnd)
    c = rnd.choice(CONTAINERS)
    if c.startswith('--'):
        k = k.replace('_', '-').replace('.', '-')
    out = c.replace('ns:', '\1').replace('k', '\2').replace('v', '\3').replace('x ', '\4')
    out = out.replace('\1', 'ns:').replace('\4', 'x ').replace('\2', k).replace('\3', s)
    out = out.replace('export ', 'export ').replace('set ', 'set ')
    return f'key-{style}' + ('-cat' if concat else ''), out


KEYS_PLAIN = ['password', 'rpcpassword', 'api_key', 'token', 'secret', 'passphrase', 'access_token', 'privkey', 'client_secret']


def pct(text, rnd, layers):
    for _ in range(layers):
        text = quote(text, safe='' if rnd.random() < 0.5 else 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789')
    return text


def ent(ch, rnd):
    return rnd.choice([f'&#{ord(ch)};', f'&#x{ord(ch):x};', f'&#X{ord(ch):X};', f'&#00{ord(ch)};'])


def gen_encoded(rnd, s):
    k = rnd.choice(KEYS_PLAIN)
    form = rnd.randrange(9)
    if form == 0:
        return 'enc-pct-all', 'https://h.example/cb?next=' + pct(f'/x?{k}={s}&a=1', rnd, rnd.randint(1, 3))
    if form == 1:
        return 'enc-pct-sep', f'{k}{pct("=", rnd, rnd.randint(1, 3))}{s}'
    if form == 2:
        i = rnd.randrange(len(k))
        e = f'%{ord(k[i]):02X}'
        return 'enc-pct-keychar', k[:i] + (e if rnd.random() < 0.7 else e.replace('%', '%25')) + k[i + 1:] + '=' + s
    if form == 3:
        return 'enc-ent-sep', k + ent(rnd.choice('=:'), rnd) + s
    if form == 4:
        q = ent('"', rnd)
        return 'enc-ent-json', f'{{{q}{k}{q}:{q}{s}{q}}}'
    if form == 5:
        q = rnd.choice(['\\u0022', '\\\\u0022', '\\x22'])
        return 'enc-uesc-json' + ('-x' if 'x' in q else ''), f'{q}{k}{q}:{q}{s}{q}'
    if form == 6:
        d = rnd.randint(1, 4)
        q = '\\' * (2 ** d - 1) + '"'
        return f'enc-json-depth{d}', f'{{{q}{k}{q}:{q}{s}{q}}}'
    if form == 7:
        return 'enc-named-ent', rnd.choice([f'&quot;{k}&quot;:&quot;{s}&quot;', f'&lt;{k}&gt;{s}&lt;/{k}&gt;', f'{k}&equals;{s}'])
    return 'enc-form-body', f'POST body: user=alice&{k}={pct(s + "&x", rnd, 1)}'


def gen_xml(rnd, s):
    k = rnd.choice(['password', 'Password', 'wsse:Password', 'ns1:rpcpassword', 'token', 'apiKey', 'PrivateKey', 'secret', 'passphrase', 'auth', 'pass'])
    q = rnd.choice('"\'')
    sp = rnd.choice(['', ' ', '\n', '\t', ' \r\n '])
    forms = [
        (f'<{k}{sp}>{s}</{k}{sp}>', 'xml-elem'),
        (f'<{k} Type={q}PasswordText{q}{sp}>{s}</{k}>', 'xml-elem-attr'),
        (f'<{k}><![CDATA[{s}]]></{k}>', 'xml-cdata'),
        (f'<{k}>\n  {s}\n</{k}>', 'xml-multiline'),
        (f'<add key={q}{k}{q} value={q}{s}{q} />', 'xml-addkey'),
        (f'<add value={q}{s}{q} key={q}{k}{q}/>', 'xml-addkey-rev'),
        (f'<property name={q}{k}{q}>{s}</property>', 'xml-named-body'),
        (f'<param name={q}{k}{q}><value>{s}</value></param>', 'xml-named-nested'),
        (f'<entry{sp}key={q}{k}{q}{sp}value={q}{s}{q}/>', 'xml-entry-ws'),
        (f'<conn {k}={q}{s}{q} host={q}h{q}/>', 'xml-attr'),
        (f'<{k}>{s}', 'xml-unclosed'),
        (f'<{k}>{s}</{k}x>', 'xml-wrong-close'),
        (f'<{k}>{s}</{k.upper()}>', 'xml-case-close'),
        (f'<input type={q}password{q} value={q}{s}{q}>', 'xml-input-type'),
        (f'<input name={k} value={s}>', 'xml-unquoted-attr'),
        (f'<key>{k}</key><string>{s}</string>', 'xml-plist'),
    ]
    text, kind = rnd.choice(forms)
    return kind, text + ' done'


def gen_cookie(rnd, s):
    n = rnd.choice(['sid', '__Host-sid', 'session', 'JSESSIONID', 'auth'])
    pre = rnd.choice(['', 'a=1; ', 'a=1;', 'a="x;y"; ', 'theme=dark; lang=de; '])
    forms = [
        ('Cookie: ' + pre + f'{n}={s}', 'cookie-header'),
        ('Set-Cookie: ' + f'{n}={s}; Path=/; HttpOnly', 'cookie-set'),
        ('cookie=' + pre + f'{n}={s}', 'cookie-assign'),
        ('cookies: ' + pre + f'{n}={s}', 'cookie-assign-plural'),
        ('"cookie":"' + pre + f'{n}={s}"', 'cookie-json'),
        ('"cookies":[{"name":"' + n + f'","value":"{s}"' + '}]', 'cookie-json-list'),
        ('curl -b "' + pre + f'{n}={s}" https://h.example', 'cookie-curl-b'),
        (f'curl --cookie {n}={s} https://h.example', 'cookie-curl-long'),
        ('curl -H "Cookie: ' + pre + f'{n}={s}" https://h.example', 'cookie-curl-H'),
        ('Cookie: ' + pre.rstrip() + f'\r\n\t{n}={s}', 'cookie-folded'),
        ('document.cookie = "' + f'{n}={s}; path=/"', 'cookie-js'),
        ('cookie = {"' + n + f'": "{s}"' + '}', 'cookie-object'),
        ('requests.get(url, cookies={"' + n + f'": "{s}"' + '})', 'cookie-kwarg'),
        (f'Cookie:{pre}{n}="{s}"', 'cookie-quoted'),
    ]
    text, kind = rnd.choice(forms)
    return kind, text


def gen_unclosed(rnd, s):
    k = rnd.choice(KEYS_PLAIN + ['password'])
    sep = rnd.choice(['=', ': ', ' = '])
    op = rnd.choice(['{', '[', '(', '$(', '${', '"', "'", "'''", '"""', '`', '<', '{"a": [', '[{', '("'])
    first = rnd.choice(['', 'a, ', 'x: 1, ', '"a": "b", ', 'echo '])
    cont = rnd.random() < 0.5
    body = op + first + (('\n  ' + s) if cont else s)
    return 'open-' + ('cont-' if cont else '') + op.replace('"', 'Q').replace("'", 'q').replace('`', 'b'), f'{k}{sep}{body}'


STATUS = ['none', 'expired', '(not set)', 'None', 'EXPIRED', 'none,', '(not set),']


def gen_exception(rnd, s, sl):
    form = rnd.randrange(9)
    glue = rnd.choice([' ', '', '\t', ', ', ' - '])
    if form == 0:
        return 'exc-status', f'{rnd.choice(["password", "token", "secret", "api_key"])}: {rnd.choice(STATUS)}{glue}{s}'
    if form == 1:
        return 'exc-flagprose', f'{rnd.choice(["--password", "--token", "--rpcpassword", "--passphrase"])} {rnd.choice(["the", "to", "is", "value", "none", "with"])}{glue}{s}'
    if form == 2:
        return 'exc-seedwords', f'--seed-words {rnd.choice(["12", "24"])}{glue}{s}'
    if form == 3:
        return 'exc-testspass', f'tests pass: {rnd.randint(1, 99)}{glue}{s}'
    if form == 4:
        return 'exc-userprose', f'{rnd.choice(["user", "auth"])}: {rnd.choice(["alice", "bob", "root", "ok", "required", "enabled"])}{glue}{s}'
    if form == 5:
        return 'exc-userprose-lower', f'{rnd.choice(["user", "auth"])}: {rnd.choice(["alice", "root", "ok", "required"])} {sl}'
    if form == 6:
        return 'exc-flag-lowerpunct', f'{rnd.choice(["try", "run with", "set"])} --password {sl}{rnd.choice(".!?,")}'
    if form == 7:
        return 'exc-passphrase-words', f'{rnd.choice(["passphrase", "password", "walletpassphrase", "seed_passphrase"])}{rnd.choice([": ", "="])}{sl[::-1]} {sl[1:]}{sl[0]} {sl}'
    key = rnd.choice(KEYS_PLAIN)
    prefix = rnd.choice(["", "x", "ab", "^", "`", "%7C", "|"])
    quote_char = rnd.choice(['"', "'"])
    return 'exc-strayquote', f'{key}={prefix}{quote_char} {s}'



ZW = ['\u200b', '\u200c', '\u200d', '\u2060', '\ufeff', '\u00ad', '\u0301', '\u034f', '\u180e']
NONASCII = ['ś', 'ш', 'é', 'ı', 'ſ', '接', 'ü', 'р', 'һ']
PUNCT = ['_', ')', '"', "'", '~', '!', '*', '$', ']', '}', '^', '`']
QUOTERS = ['"', "'", ',', ';', '|', '`', 'x', '/', '\\', '%22', '&quot;', '{"u":"']


def gen_url(rnd, s, sl):
    user = rnd.choice(['alice', 'bob', 'x-access-token', 'john.doe', 'rpc', 'u'])
    host = rnd.choice(['h.example', 'node.example.org', '127.0.0.1', 'proxy.example.com:3128', 'db', '[::1]:8332'])
    form = rnd.randrange(10)
    if form == 0:
        x = rnd.choice(ZW + NONASCII + PUNCT)
        where = rnd.randrange(3)
        sch = rnd.choice(['http', 'https', 'socks5h', 'electrum', 'ssl'])
        u = [f'{sch}{x}://', f'{sch}:{x}//', f'{sch}:/{x}/'][where]
        return f'url-schemechar{where}', f'{u}{user}:{s}@{host}/'
    if form == 1:
        q = rnd.choice(QUOTERS)
        return 'url-relative-after', f'{q}//{user}:{s}@{host}/a.js'
    if form == 2:
        k = rnd.choice(['proxy', 'https_proxy', 'ALL_PROXY', 'DATABASE_URL', 'dsn', 'upstream', 'server', 'url', 'endpoint', '--proxy'])
        return 'url-schemeless-assign', f'{k}{rnd.choice(["=", ": ", " = "])}{user}:{s}@{host}'
    if form == 3:
        return 'url-schemeless-punct', f'{user}:{s[:4]}{rnd.choice("/?#=")}{s[4:]}@{host}'
    if form == 4:
        # dotted user + numeric-prefixed password containing a slash or space
        return 'url-dotted-port-path', f'https://{rnd.choice(["john.doe", "a.b", "alice.smith"])}:{rnd.randint(1, 99999)}{rnd.choice(["/", " "])}{sl}@{host}'
    if form == 5:
        return 'url-path-handle', f'https://{host}/{rnd.choice(["@", "~", "u/", ""])}{user}:{s}@other.example/'
    if form == 6:
        enc = rnd.choice([lambda t: t.replace(':', '%3A').replace('@', '%40'), lambda t: t.replace('@', '&#64;'),
                          lambda t: t.replace(':', '&colon;'), lambda t: t.replace('@', '\\u0040'), lambda t: t.replace('@', '＠'),
                          lambda t: t.replace('@', '﹫'), lambda t: t.replace(':', '：')])
        return 'url-encoded-userinfo', 'https://' + enc(f'{user}:{s}@') + host + '/'
    if form == 7:
        sep = rnd.choice([' ', '\t', '%20', '%09', '+', '\\u0020', '&#32;'])
        return 'url-space-in-pw', f'http://{user}:{s[:5]}{sep}{s[5:]}@{host}/'
    if form == 8:
        prefix = rnd.choice(["(", "[", "<", '"', "'", "url="])
        return 'url-schemeless-quoted', f'{prefix}{user}:{s}@{host}'
    return 'url-cli', rnd.choice([f'curl -u{user}:{s} https://{host}', f'sshpass -p {s} ssh {user}@{host}', f'mysql -u {user} -p{s}',
                                  f'wget --user={user} --password={s} https://{host}', f'git clone https://{s}@github.com/o/r'])


CTL_URLS = ['https://medium.com/@bob/post', 'https://www.npmjs.com/package/@scope/name', 'npm i pkg@1.2.3', 'https://cdn.jsdelivr.net/npm/lodash@4.17.21/lodash.min.js',
            'https://deno.land/std@0.200.0/http/server.ts', 'https://www.google.com/maps/@48.2082,16.3738,12z', 'git@github.com:org/repo.git',
            'https://mastodon.social/@alice/1234567', 'https://example.com/?q=a@b', 'http://127.0.0.1:8332/wallet/main', 'tcp://electrum.example.org:50001',
            'ssl://fulcrum.example.org:50002/', 'http://[::1]:8332/', 'https://example.com/docs#section', 'socks5h://127.0.0.1:9050',
            'https://github.com/org/repo/compare/v1.0.0...v1.1.0', 'https://twitter.com/@kassiber_app', '@scope/name@2.0.0']
CTL_PROSE = ['Use --password to set a value.', 'The password field is required.', 'token expired', 'password: (not set)', 'auth: ok', 'user: alice logged in',
             'tests pass: 12', '--seed-words 24', 'set the rpcpassword option in bitcoin.conf', 'the secret key is missing', 'invalid password',
             'height=800000 synced', 'fee 1.5 sat/vB', 'cookie consent banner shown', 'session started for @alice', 'reply to @bob',
             'alice@example.com', 'contact support@example.org', 'version 1.2.3 build abc123', 'see RFC 3986 section 3.2.1']
KEEPS = ['4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b', '123e4567-e89b-12d3-a456-426614174000', 'electrum.blockstream.info',
         '@kassiber_app', 'b1946ac92492d2347c6235b4d2611184', 'sync complete', 'bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq', 'medium.com/@bob/post']
PRE = ['', '', 'INFO ', '2026-11-01T12:00:00Z WARN sync: ', 'error: ', '> ', '\t', '{"msg":"', 'note é: ', '接 ', 'x ']
TERM = ['\n', '\r\n', '\n\n', '\u2028', '\r']


def cases(count, seed):
    rnd = random.Random(seed)
    for i in range(count):
        s = sentinel(i, rnd)
        sl = sentinel(i, rnd, lower=True)
        r = rnd.random()
        secret = s
        if r < 0.24:
            kind, text = gen_key_safe(rnd, s)
        elif r < 0.36:
            kind, text = gen_encoded(rnd, s)
        elif r < 0.48:
            kind, text = gen_xml(rnd, s)
        elif r < 0.58:
            kind, text = gen_cookie(rnd, s)
        elif r < 0.68:
            kind, text = gen_unclosed(rnd, s)
        elif r < 0.78:
            kind, text = gen_exception(rnd, s, sl)
            if kind in ('exc-userprose-lower', 'exc-flag-lowerpunct', 'exc-passphrase-words'):
                secret = sl
        elif r < 0.92:
            kind, text = gen_url(rnd, s, sl)
            if kind == 'url-dotted-port-path':
                secret = sl
        elif r < 0.97:
            u = rnd.choice(CTL_URLS)
            keep = rnd.choice(KEEPS)
            yield {'i': i, 'kind': 'url-ctl', 'input': rnd.choice(PRE) + u + rnd.choice(TERM) + 'status ' + keep, 'sentinel': '', 'keep': [u, keep]}
            continue
        else:
            p = rnd.choice(CTL_PROSE)
            keep = rnd.choice(KEEPS)
            yield {'i': i, 'kind': 'prose-ctl', 'input': rnd.choice(PRE) + p + rnd.choice(TERM) + 'status ' + keep, 'sentinel': '', 'keep': [p, keep]}
            continue
        keep = rnd.choice(KEEPS)
        yield {'i': i, 'kind': kind, 'input': rnd.choice(PRE) + text + rnd.choice(TERM) + 'status ' + keep, 'sentinel': secret, 'keep': [keep]}


if __name__ == '__main__':
    for case in cases(int(sys.argv[1]), int(sys.argv[2])):
        print(json.dumps(case, ensure_ascii=True))
