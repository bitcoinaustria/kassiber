"""Reviewer 5d independent generator, adapted only for streaming and preservation.

Original adv/gen.py SHA-256: 60b21b22683b5662564fecb62caa9b2bfb2ed9093b0a5d8f00be222853cca0a4.
Random draws and original inputs are preserved before appending a non-secret.
"""
import json
import random
import sys
def sentinel(i):
    return 'QJ' + ''.join('BCDFGHKMNPRTVWXZ'[int(c, 16)] for c in f'{i:07x}') + 'JQ'
SCHEMES = ['http', 'https', 'ssl', 'tcp', 'ws', 'socks5h', 'electrum', 'HTTP', 'Https']
HOSTS = ['h', '127.0.0.1', 'node.example', '[::1]', '[fe80::1%25eth0]', '[fe80::1%eth0]', 'xn--bcher-kva.example', 'h.onion']
PORTS = ['', ':8332', ':50002', ':0', ':']
TAILS = ['', '/', '/api', '/?x=1', '?x=1', '#f', '/a@b', '/path?next=http://x/']
USERS = ['alice', '', 'a%40b', 'a.b', 'a-b_c', '%E2%82%AC', 'ünï', '1234', 'a~b']
PWPUNCT = list("!$&'()*+,;=-._~%:") + ['%40', '%3A', '%2F', '%25', '%0D%0A', 'é', '€', ' x', '😀', '[', ']', '{', '}', '"', '<', '>', '|', '\\', '^', '`', '#', '?', '/']
PREFIX = ['', 'url=', 'url: ', 'connect ', '"', "'", '(', '<', '[', '{"url":"', "{'url': '", 'x,', 'x;', 'x|', 'x&', '接続', 'é', 'next=', 'https://proxy.example/', 'https://h/login?next=', 'http://a.example,', 'http://a.example;', 'http://a.example|', 'http://a.example ', 'http://a:1,', '\\"', 'env HTTP_PROXY=', 'proxy=', 'Error: GET ']
SUFFIX = ['', ' ', '"', "'", ')', '>', ']', '"}', "'}", ',', ';', '.', ':', '\n', '\r\n', ' ok', '&y=2', '|status=ok', ',"password":"x"']
KEYS = ['password', 'rpcpassword', 'api_key', 'X-Api-Key', 'token', 'secret', 'macaroon', 'seed', 'mnemonic', 'passphrase', 'clientSecret', 'Authorization', 'Proxy-Authorization', 'Cookie', 'rpcauth', 'access_token', 'bitcoin.rpcpassword', 'BITCOIN_RPCPASSWORD', 'db-passphrase', 'private_key']
def jq(s):
    return json.dumps(s, ensure_ascii=False)
def pw(s, rnd):
    a = ''.join(rnd.choice(PWPUNCT) for _ in range(rnd.randint(0, 2)))
    b = ''.join(rnd.choice(PWPUNCT) for _ in range(rnd.randint(0, 2)))
    return a + s + b
# Keep this generator independent of redaction_generation.py and its spec.
NONSECRETS = [
    '9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08',
    '123e4567-e89b-12d3-a456-426614174000',
    'b1946ac92492d2347c6235b4d2611184',
    'https://clean.example:8332/api',
    'ordinary prose stays readable',
    'Use --password to set a value.',
]


def generated_cases(count=100_000, seed=7):
    rnd = random.Random(seed)
    for i in range(count):
        s = sentinel(i)
        t = rnd.randrange(10)
        if t <= 4:
            user = rnd.choice(USERS)
            p = pw(s, rnd)
            ui = (user + ':' + p) if rnd.random() < 0.8 else p
            url = f'{rnd.choice(SCHEMES)}://{ui}@{rnd.choice(HOSTS)}{rnd.choice(PORTS)}{rnd.choice(TAILS)}'
            text = rnd.choice(PREFIX) + url + rnd.choice(SUFFIX)
            kind = 'url'
            # tag whether the password is "plain" (only RFC-legal userinfo chars incl. pct-encoding) so we can split leak classes
            legal = all(c.isalnum() or c in "!$&'()*+,;=-._~%:" for c in p) and p.isascii()
            kind += '-legal' if legal else '-illegal'
        elif t <= 7:
            key = rnd.choice(KEYS)
            val = pw(s, rnd) if rnd.random() < 0.3 else s
            shape = rnd.randrange(12)
            if shape == 0: text = f'{key}={val}'
            elif shape == 1: text = f'{key}: {val}'
            elif shape == 2: text = '{' + jq(key) + ':' + jq(val) + '}'
            elif shape == 3:
                text = '{' + jq('url') + ':' + jq(rnd.choice(SCHEMES) + '://' + rnd.choice(HOSTS) + rnd.choice(PORTS)) + ',' + jq(key) + ':' + jq(val) + '}'
                for _ in range(rnd.randint(0, 5)): text = jq(text)
            elif shape == 4: text = f'--{key} {val}'
            elif shape == 5: text = f'cmd --{key}={jq(val)}'
            elif shape == 6: text = f'{key}={val}\r\nnext=1'
            elif shape == 7: text = f'{rnd.choice(SCHEMES)}://{rnd.choice(HOSTS)}{rnd.choice(PORTS)}/?{key}={val}&x=1'
            elif shape == 8: text = f'{key} = "{val}"'
            elif shape == 9: text = f"{{'{key}': '{val}'}}"
            elif shape == 10: text = f'{rnd.choice(SCHEMES)}://{rnd.choice(HOSTS)}{rnd.choice(PORTS)}{rnd.choice(",;|&")}{key}={val}'
            else: text = f'{key}\t=\t{val}'
            kind = f'key{shape}'
            if shape == 3: kind += f'-d{text.count(chr(92)+chr(34)) and 1}'
        else:
            # long tokens / split encodings / lookalikes
            shape = rnd.randrange(6)
            if shape == 0: text = 'http://alice:' + 'A' * rnd.randint(1000, 5000) + s + '@h/'
            elif shape == 1: text = 'token=' + 'B' * rnd.randint(1000, 5000) + s
            elif shape == 2: text = 'http://alice:' + s[:6] + '%' + s[6:] + '@h/'
            elif shape == 3: text = 'password=' + s[:6] + '\\u0040' + s[6:]
            elif shape == 4: text = 'http://alice:' + s + '@h' + '/' + 'x' * rnd.randint(0, 3000)
            else: text = f'{rnd.choice(["Authorization: Basic ", "Authorization: Bearer ", "Authorization: Digest username=a, response="])}{s}'
            kind = f'misc{shape}'
        keep = NONSECRETS[i % len(NONSECRETS)]
        yield {'index': i, 'kind': kind, 'input': text + '\n' + keep, 'sentinel': s, 'preserved': [keep]}


if __name__ == '__main__':
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 100_000
    seed = int(sys.argv[2]) if len(sys.argv) > 2 else 7
    for case in generated_cases(count, seed):
        print(json.dumps(case, ensure_ascii=False))
