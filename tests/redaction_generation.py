"""Seeded wire-format generator shared with redactionGeneration.ts through its spec."""
from __future__ import annotations

import json
from pathlib import Path

SPEC = json.loads((Path(__file__).parent / 'fixtures/secret_redaction_generator.json').read_text())


def generated_cases():
    state = SPEC['seed']

    def pick(name):
        nonlocal state
        state = (1664525 * state + 1013904223) & 0xffffffff
        return SPEC[name][(state >> 8) % len(SPEC[name])]

    def quoted(value):
        return json.dumps(value, ensure_ascii=False, separators=(',', ':'))

    for index in range(SPEC['count']):
        sentinel = 'QZX' + ''.join('BCDFGJKLMNQRVXYZ'[int(ch, 16)] for ch in f'{index:08x}') + 'ZVQJK'
        left, right = sentinel[:8], sentinel[8:]
        scheme, host, port = pick('schemes'), pick('hosts'), pick('ports')
        user, separator = pick('users'), pick('userinfo_separators')
        delimiter, key = pick('delimiters'), pick('keys')
        help_text, keep = pick('helps'), pick('nonsecrets')
        flag_prefix = pick('flag_prefixes')
        malformed = pick('malformed_separators')
        url = f'{scheme}://{host}{port}'
        secret = left + '@' + right + ['\\"9', "'007", '\\\\', '1234\n'][index // 20 % 4]
        q = quoted(secret)
        shape = index % 20
        if shape == 0:
            text = f'{scheme}://{user}:{left}{separator}{right}@{host}{port}/{delimiter}'
        elif shape == 1:
            text = '{"error":' + quoted('cannot reach (' + url + delimiter) + ',"' + key + '":' + q + '}'
        elif shape == 2:
            text = key + '=' + q
        elif shape == 3:
            text = '{"' + key + '":' + q + ',"url":' + quoted(url) + '}'
        elif shape == 4:
            text = key + ': ' + q
        elif shape == 5:
            text = '--' + key + '=' + q
        elif shape == 6:
            text = 'cmd ' + flag_prefix + key + ' ' + q
        elif shape == 7:
            text = 'Authorization: ' + ['Basic', 'Bearer', 'Token', 'Digest', 'Negotiate'][index // 20 % 5] + ' ' + sentinel + '\n'
        elif shape in (8, 9, 16):
            text = '{"backend":{"url":' + quoted(url) + ',"' + key + '":' + q + '}}'
            for _ in range(index // 20 % 4):
                text = quoted(text)
            if shape == 9:
                text = "'" + text.replace('\\', '\\\\').replace("'", "\\'") + "'"
        elif shape == 10:
            text = f'{scheme}://u:{left}{malformed}{right}@{host}{port}/'
        elif shape == 11:
            text = 'url=' + url + '|' + key + '=' + q + '|status=ready'
        elif shape == 12:
            text = help_text
            sentinel = ''
        elif shape == 13:
            text = 'cmd ' + flag_prefix + key + ' ' + sentinel + ('.' if index // 20 % 2 else '') + ' --verbose'
        elif shape == 14:
            text = key + '=' + left + '@' + right + delimiter
        elif shape == 15:
            text = 'url=' + url + '&rpcpassword=' + left + '@' + right
        elif shape == 17:
            text = key + '=' + quoted(secret + '\\')
        elif shape == 18:
            text = f'{scheme}://u:{left}@ss@{right}@{host}{port}/'
        else:
            text = '{' + repr(key) + ': ' + "'" + secret.replace('\\', '\\\\').replace("'", "\\'") + "'}"
        preserved = [keep, help_text]
        yield {'index': index, 'input': text + '\n' + keep + '\n' + help_text,
               'sentinel': sentinel, 'preserved': preserved}
