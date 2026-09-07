# Public PSBT reference fixtures

`psbt_bip_vectors.json` contains the BIP174 and BIP370 invalid/valid encoding
vectors, BIP174 failing-signer evidence vectors, and BIP370 locktime vectors.
`payjoin_bip78_vectors.json` contains the original/proposal/filled-proposal
vectors from BIP78. These are published synthetic protocol fixtures, not wallet
data. No private keys or recovery material from the BIPs are included.

Sources, retrieved 2026-09-06:

- [BIP174, Ava Chow](https://github.com/bitcoin/bips/blob/master/bip-0174.mediawiki)
- [BIP370, Ava Chow](https://github.com/bitcoin/bips/blob/master/bip-0370.mediawiki)
- [BIP78, Nicolas Dorier](https://github.com/bitcoin/bips/blob/master/bip-0078.mediawiki)

Each BIP is licensed under BSD-2-Clause. The fixture data retain that license;
the surrounding independently implemented Kassiber code retains the project
license. The BIP authors' license grant includes the following conditions:

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE
LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE
GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT
LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT
OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH
DAMAGE.
