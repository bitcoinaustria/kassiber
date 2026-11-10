//! Deterministic Elements regtest chain states for the LWK parity oracle.
//! Writes `crates/wallets/tests/vectors/lwk_chain.json`; the Python oracle
//! serves those states over a loopback Electrum fake to Python LWK 0.18.0 and
//! to the native observer. No network, database or product state is touched.
//!
//! Regenerate: `cargo run -p kassiber-wallets --example lwk_vectors
//! --features test-harness --offline --locked > crates/wallets/tests/vectors/lwk_chain.json`
use lwk_wollet::elements::bitcoin::bip32::{Xpriv, Xpub};
use lwk_wollet::{
    elements::{
        confidential::{Asset, Nonce, Value as ConfidentialValue},
        encode::serialize,
        hashes::{sha256, Hash},
        hex::ToHex,
        secp256k1_zkp::{
            rand::{rngs::StdRng, SeedableRng},
            PublicKey, Secp256k1, SecretKey,
        },
        Address, AddressParams, AssetId, BlockExtData, BlockHash, BlockHeader, LockTime, OutPoint,
        Script, Sequence, Transaction, TxIn, TxInWitness, TxMerkleNode, TxOut, TxOutSecrets,
        TxOutWitness, Txid,
    },
    WolletDescriptor,
};
use serde_json::{json, Map, Value};
use std::{collections::BTreeMap, str::FromStr};

/// `tests.test_lwk_observer.descriptor()`: abandon-about xpub, fixed slip77.
const DESCRIPTOR: &str = "ct(slip77(f46e151e2e0d81601e9e25334f7a9f4ffaf9790192b387af036a70c88304e933),elwpkh(xpub6Bm9M1SxZdzL3TxdNV8897FgtTLBgehR1wVNnMyJ5VLRK5n3tFqXxrCVnVQj4zooN4eFSkf6Sma84reWc5ZCXMxPbLXQs3BcaBdTd4YQa3B/<0;1>/*))";
const POLICY_ASSET: &str = "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225";
const ISSUED_ASSET: &str = "a771b62bb1bb2b1de5a1fe0cbc6e9bc1d3b7a3b8c53e5e31c9a6e6c5f0e9e6c1";
const PARAMS: &AddressParams = &AddressParams::ELEMENTS;

fn tagged(tag: &str) -> [u8; 32] {
    sha256::Hash::hash(tag.as_bytes()).to_byte_array()
}
fn outpoint(tag: &str, vout: u32) -> OutPoint {
    OutPoint::new(Txid::from_byte_array(tagged(tag)), vout)
}
fn key(tag: &str) -> SecretKey {
    SecretKey::from_slice(&tagged(tag)).unwrap()
}

struct Builder {
    rng: StdRng,
    secp: Secp256k1<lwk_wollet::elements::secp256k1_zkp::All>,
    lbtc: AssetId,
    issued: AssetId,
    descriptor: WolletDescriptor,
    wrong: WolletDescriptor,
    view: Xpriv,
}

enum Pay {
    Receive(u32),
    Change(u32),
    /// Our receive script, blinded to an unrelated SLIP-77 key.
    WrongKey(u32),
    /// Our receive script with explicit asset and value.
    Explicit(u32),
    Foreign(&'static str),
    /// A foreign script with explicit asset and value.
    ForeignExplicit(&'static str),
    /// Our script, blinded to the view-key wallet's key (receive or change).
    View(lwk_wollet::Chain, u32),
    /// Receive script `index`, blinded to a fixed `ct(<WIF>, ...)` wallet's
    /// key: the view key plus the script tweak, nothing derived.
    FixedView(u32),
}

/// The view-key wallet's blinding key for one script, as embit derives it
/// (`wallet_descriptors.liquid_blinding_secret`): the view key at
/// `<chain>/<index>` plus `tagged_hash("elements/blindingkey", script)`.
/// Written independently of the product code, which the parity test checks.
fn view_blinding(view: &Xpriv, chain: lwk_wollet::Chain, index: u32, script: &Script) -> PublicKey {
    use lwk_wollet::elements::bitcoin::{bip32::ChildNumber, secp256k1};
    let secp = secp256k1::Secp256k1::new();
    let branch = u32::from(chain == lwk_wollet::Chain::Internal);
    let child = view
        .derive_priv(
            &secp,
            &[ChildNumber::from(branch), ChildNumber::from(index)],
        )
        .unwrap();
    tweaked(&child.private_key, script)
}

/// `secret + tagged_hash("elements/blindingkey", script)`, as a public key.
fn tweaked(
    secret: &lwk_wollet::elements::bitcoin::secp256k1::SecretKey,
    script: &Script,
) -> PublicKey {
    use lwk_wollet::elements::bitcoin::secp256k1;
    let secp = secp256k1::Secp256k1::new();
    let tag = sha256::Hash::hash(b"elements/blindingkey");
    let mut engine = sha256::Hash::engine();
    lwk_wollet::elements::hashes::HashEngine::input(&mut engine, tag.as_ref());
    lwk_wollet::elements::hashes::HashEngine::input(&mut engine, tag.as_ref());
    lwk_wollet::elements::hashes::HashEngine::input(&mut engine, script.as_bytes());
    let tweak = secp256k1::Scalar::from_be_bytes(sha256::Hash::from_engine(engine).to_byte_array())
        .unwrap();
    let secret = secret.add_tweak(&tweak).unwrap();
    PublicKey::from_slice(&secret.public_key(&secp).serialize()).unwrap()
}

/// The fixed view-key wallet's WIF secret.
fn fixed_view_key() -> lwk_wollet::elements::bitcoin::secp256k1::SecretKey {
    lwk_wollet::elements::bitcoin::secp256k1::SecretKey::from_slice(&tagged("fixed-view-key"))
        .unwrap()
}

impl Builder {
    fn address(&self, pay: &Pay) -> Address {
        match pay {
            Pay::Receive(i) => self.descriptor.address(*i, PARAMS).unwrap(),
            Pay::Change(i) => self.descriptor.change(*i, PARAMS).unwrap(),
            Pay::WrongKey(i) => {
                let mut ours = self.descriptor.address(*i, PARAMS).unwrap();
                ours.blinding_pubkey = self.wrong.address(*i, PARAMS).unwrap().blinding_pubkey;
                ours
            }
            Pay::Explicit(i) => self
                .descriptor
                .address(*i, PARAMS)
                .unwrap()
                .to_unconfidential(),
            Pay::ForeignExplicit(tag) => {
                let spend = PublicKey::from_secret_key(&self.secp, &key(&format!("spend:{tag}")));
                Address::p2wpkh(
                    &lwk_wollet::elements::bitcoin::PublicKey::new(spend),
                    None,
                    PARAMS,
                )
            }
            Pay::FixedView(i) => {
                let script = self
                    .descriptor
                    .script_pubkey(lwk_wollet::Chain::External, *i)
                    .unwrap();
                let blind = tweaked(&fixed_view_key(), &script);
                Address::from_script(&script, Some(blind), PARAMS).unwrap()
            }
            Pay::View(chain, i) => {
                let script = self.descriptor.script_pubkey(*chain, *i).unwrap();
                let blind = view_blinding(&self.view, *chain, *i, &script);
                Address::from_script(&script, Some(blind), PARAMS).unwrap()
            }
            Pay::Foreign(tag) => {
                let spend = PublicKey::from_secret_key(&self.secp, &key(&format!("spend:{tag}")));
                let blind = PublicKey::from_secret_key(&self.secp, &key(&format!("blind:{tag}")));
                Address::p2wpkh(
                    &lwk_wollet::elements::bitcoin::PublicKey::new(spend),
                    Some(blind),
                    PARAMS,
                )
            }
        }
    }
    fn output(&mut self, pay: Pay, value: u64, asset: AssetId, domain: &[TxOutSecrets]) -> TxOut {
        let address = self.address(&pay);
        if address.blinding_pubkey.is_none() {
            return TxOut {
                asset: Asset::Explicit(asset),
                value: ConfidentialValue::Explicit(value),
                nonce: Nonce::Null,
                script_pubkey: address.script_pubkey(),
                witness: TxOutWitness::default(),
            };
        }
        TxOut::new_not_last_confidential(&mut self.rng, &self.secp, value, address, asset, domain)
            .unwrap()
            .0
    }
    fn domain(&self) -> Vec<TxOutSecrets> {
        [self.lbtc, self.issued]
            .into_iter()
            .map(|asset| {
                TxOutSecrets::new(
                    asset,
                    lwk_wollet::elements::confidential::AssetBlindingFactor::zero(),
                    1,
                    lwk_wollet::elements::confidential::ValueBlindingFactor::zero(),
                )
            })
            .collect()
    }
}

fn input(previous_output: OutPoint, witness: Vec<Vec<u8>>) -> TxIn {
    TxIn {
        previous_output,
        is_pegin: false,
        script_sig: Script::new(),
        sequence: Sequence::MAX,
        asset_issuance: Default::default(),
        witness: TxInWitness {
            script_witness: witness,
            ..Default::default()
        },
    }
}

fn tx(inputs: Vec<TxIn>, outputs: Vec<TxOut>) -> Transaction {
    Transaction {
        version: 2,
        lock_time: LockTime::ZERO,
        input: inputs,
        output: outputs,
    }
}

/// Boltz v1 submarine redeem script (`htlc_parser.parse_htlc_redeem_script`).
fn htlc_script(preimage: &[u8; 32]) -> Vec<u8> {
    use lwk_wollet::elements::hashes::hash160;
    let mut script = vec![0xa9, 20];
    script.extend(hash160::Hash::hash(preimage).to_byte_array());
    script.extend([0x87, 0x63, 33]);
    script.extend([0x02; 33]);
    script.extend([0x67, 3, 0x10, 0x27, 0x00, 0xb1, 0x75, 33]);
    script.extend([0x03; 33]);
    script.extend([0x68, 0xac]);
    script
}

fn header(height: u32, epoch: u32, prev: BlockHash) -> BlockHeader {
    BlockHeader {
        version: 0x2000_0000,
        prev_blockhash: prev,
        merkle_root: TxMerkleNode::from_byte_array(tagged(&format!("merkle:{height}:{epoch}"))),
        time: 1_700_000_000 + height * 60 + epoch * 7,
        height,
        ext: BlockExtData::Proof {
            challenge: Script::from(vec![0x51]),
            solution: Script::new(),
        },
    }
}

/// One Electrum-visible chain: confirmed heights (0 = mempool) per txid.
fn phase(
    builder: &Builder,
    tip: u32,
    reorg_from: Option<u32>,
    txs: &[(&Transaction, u32)],
) -> Value {
    let mut headers = Map::new();
    let mut prev = BlockHash::all_zeros();
    for height in 0..=tip {
        let epoch = u32::from(reorg_from.is_some_and(|from| height >= from));
        let header = header(height, epoch, prev);
        prev = header.block_hash();
        headers.insert(height.to_string(), json!(serialize(&header).to_hex()));
    }
    // Electrum history per script: every transaction funding or spending it.
    let mut owned = BTreeMap::<Script, ()>::new();
    for chain in [lwk_wollet::Chain::External, lwk_wollet::Chain::Internal] {
        for index in 0..8 {
            owned.insert(builder.descriptor.script_pubkey(chain, index).unwrap(), ());
        }
    }
    let by_txid: BTreeMap<Txid, (&Transaction, u32)> =
        txs.iter().map(|(t, h)| (t.txid(), (*t, *h))).collect();
    let mut history = BTreeMap::<String, Vec<Value>>::new();
    for (txid, (t, height)) in &by_txid {
        let mut scripts: Vec<Script> = t.output.iter().map(|o| o.script_pubkey.clone()).collect();
        for i in &t.input {
            if let Some((parent, _)) = by_txid.get(&i.previous_output.txid) {
                scripts.push(
                    parent.output[i.previous_output.vout as usize]
                        .script_pubkey
                        .clone(),
                );
            }
        }
        scripts.sort();
        scripts.dedup();
        for script in scripts.into_iter().filter(|s| owned.contains_key(s)) {
            let mut hash = sha256::Hash::hash(script.as_bytes()).to_byte_array();
            hash.reverse();
            history
                .entry(hash.to_hex())
                .or_default()
                .push(json!({"tx_hash": txid.to_string(), "height": height}));
        }
    }
    for entries in history.values_mut() {
        // Electrum orders confirmed history by height, then mempool last.
        entries.sort_by_key(|e| {
            let h = e["height"].as_u64().unwrap();
            (
                if h == 0 { u64::MAX } else { h },
                e["tx_hash"].as_str().unwrap().to_owned(),
            )
        });
    }
    json!({
        "tip": tip,
        "headers": headers,
        "transactions": by_txid.iter().map(|(id, (t, _))| (id.to_string(), json!(serialize(*t).to_hex()))).collect::<Map<_, _>>(),
        "history": history,
    })
}

fn main() {
    let mut b = Builder {
        rng: StdRng::seed_from_u64(0x4c574b),
        secp: Secp256k1::new(),
        lbtc: AssetId::from_str(POLICY_ASSET).unwrap(),
        issued: AssetId::from_str(ISSUED_ASSET).unwrap(),
        descriptor: WolletDescriptor::from_str(DESCRIPTOR).unwrap(),
        wrong: WolletDescriptor::from_str(&DESCRIPTOR.replace(
            "f46e151e2e0d81601e9e25334f7a9f4ffaf9790192b387af036a70c88304e933",
            "0505050505050505050505050505050505050505050505050505050505050505",
        ))
        .unwrap(),
        view: Xpriv::new_master(
            lwk_wollet::elements::bitcoin::NetworkKind::Test,
            &tagged("view-key"),
        )
        .unwrap(),
    };
    let domain = b.domain();
    let (lbtc, issued) = (b.lbtc, b.issued);
    let fund = {
        let outputs = vec![
            b.output(Pay::Receive(0), 100_000, lbtc, &domain),
            b.output(Pay::Explicit(1), 50_000, lbtc, &domain),
            b.output(Pay::Receive(0), 7_000, issued, &domain),
            b.output(Pay::Foreign("payer-change"), 40_000, lbtc, &domain),
            TxOut::new_fee(300, lbtc),
        ];
        tx(
            vec![
                input(outpoint("external:a", 0), vec![]),
                input(outpoint("external:a2", 1), vec![]),
            ],
            outputs,
        )
    };
    let spend = {
        let outputs = vec![
            b.output(Pay::Foreign("merchant"), 60_000, lbtc, &domain),
            b.output(Pay::Change(0), 39_500, lbtc, &domain),
            TxOut::new_fee(500, lbtc),
        ];
        tx(
            vec![input(
                OutPoint::new(fund.txid(), 0),
                vec![vec![0x30; 71], vec![0x02; 33]],
            )],
            outputs,
        )
    };
    let mixed = {
        let outputs = vec![
            b.output(Pay::Foreign("coinjoin"), 70_000, lbtc, &domain),
            b.output(Pay::Change(1), 29_000, lbtc, &domain),
            TxOut::new_fee(1_000, lbtc),
        ];
        tx(
            vec![
                input(OutPoint::new(fund.txid(), 1), vec![]),
                input(outpoint("external:b", 0), vec![]),
            ],
            outputs,
        )
    };
    let wrong_key = {
        let outputs = vec![
            b.output(Pay::WrongKey(2), 5_000, lbtc, &domain),
            TxOut::new_fee(100, lbtc),
        ];
        tx(vec![input(outpoint("external:c", 0), vec![])], outputs)
    };
    let preimage = tagged("preimage:claim");
    let claim = {
        let outputs = vec![
            b.output(Pay::Receive(3), 20_000, lbtc, &domain),
            TxOut::new_fee(200, lbtc),
        ];
        tx(
            vec![input(
                outpoint("external:htlc", 0),
                vec![
                    vec![0x30; 71],
                    preimage.to_vec(),
                    vec![1],
                    htlc_script(&preimage),
                ],
            )],
            outputs,
        )
    };
    let refund = {
        let outputs = vec![
            b.output(Pay::Change(2), 15_000, lbtc, &domain),
            TxOut::new_fee(150, lbtc),
        ];
        tx(
            vec![input(
                outpoint("external:htlc-refund", 1),
                vec![
                    vec![0x30; 71],
                    vec![],
                    htlc_script(&tagged("preimage:refund")),
                ],
            )],
            outputs,
        )
    };
    let asset_send = {
        let outputs = vec![
            b.output(Pay::Foreign("asset-buyer"), 7_000, issued, &domain),
            b.output(Pay::Change(4), 19_800, lbtc, &domain),
            TxOut::new_fee(200, lbtc),
        ];
        tx(
            vec![
                input(OutPoint::new(fund.txid(), 2), vec![]),
                input(OutPoint::new(claim.txid(), 0), vec![]),
            ],
            outputs,
        )
    };
    let mempool = {
        let outputs = vec![
            b.output(Pay::Foreign("cafe"), 10_000, lbtc, &domain),
            b.output(Pay::Change(3), 29_000, lbtc, &domain),
            TxOut::new_fee(500, lbtc),
        ];
        tx(vec![input(OutPoint::new(spend.txid(), 1), vec![])], outputs)
    };
    let initial = [
        (&fund, 2),
        (&spend, 3),
        (&mixed, 4),
        (&wrong_key, 4),
        (&claim, 5),
        (&refund, 5),
        (&asset_send, 6),
        (&mempool, 0),
    ];
    // Block 4 is replaced: the mixed spend drops out, the mempool spend
    // confirms in the new block 7, and every header from 4 changes identity.
    let reorg = [
        (&fund, 2),
        (&spend, 3),
        (&wrong_key, 4),
        (&claim, 5),
        (&refund, 5),
        (&asset_send, 6),
        (&mempool, 7),
    ];
    // The same history without the issued asset. Python's prospective
    // negative-balance overlay keeps one record per txid, so full-book parity
    // runs on this chain; the multi-asset chain checks the matching refusal.
    let lbtc_fund = {
        let outputs = vec![
            b.output(Pay::Receive(0), 100_000, lbtc, &domain),
            b.output(Pay::Explicit(1), 50_000, lbtc, &domain),
            b.output(Pay::Foreign("payer-change"), 40_000, lbtc, &domain),
            TxOut::new_fee(300, lbtc),
        ];
        tx(vec![input(outpoint("external:l", 0), vec![])], outputs)
    };
    let lbtc_spend = {
        let outputs = vec![
            b.output(Pay::Foreign("merchant"), 60_000, lbtc, &domain),
            b.output(Pay::Change(0), 39_500, lbtc, &domain),
            TxOut::new_fee(500, lbtc),
        ];
        tx(
            vec![input(OutPoint::new(lbtc_fund.txid(), 0), vec![])],
            outputs,
        )
    };
    let lbtc_mixed = {
        let outputs = vec![
            b.output(Pay::Foreign("coinjoin"), 70_000, lbtc, &domain),
            b.output(Pay::Change(1), 29_000, lbtc, &domain),
            TxOut::new_fee(1_000, lbtc),
        ];
        tx(
            vec![
                input(OutPoint::new(lbtc_fund.txid(), 1), vec![]),
                input(outpoint("external:lb", 0), vec![]),
            ],
            outputs,
        )
    };
    let lbtc_mempool = {
        let outputs = vec![
            b.output(Pay::Foreign("cafe"), 10_000, lbtc, &domain),
            b.output(Pay::Change(3), 29_000, lbtc, &domain),
            TxOut::new_fee(500, lbtc),
        ];
        tx(
            vec![input(OutPoint::new(lbtc_spend.txid(), 1), vec![])],
            outputs,
        )
    };
    let lbtc_initial = [
        (&lbtc_fund, 2),
        (&lbtc_spend, 3),
        (&lbtc_mixed, 4),
        (&wrong_key, 4),
        (&claim, 5),
        (&refund, 5),
        (&lbtc_mempool, 0),
    ];
    let lbtc_reorg = [
        (&lbtc_fund, 2),
        (&lbtc_spend, 3),
        (&wrong_key, 4),
        (&claim, 5),
        (&refund, 5),
        (&lbtc_mempool, 7),
    ];
    // Python's Liquid compatibility observers decode every input's previous
    // transaction, so this chain carries its foreign parents too. It is
    // built after the LWK chains, leaving their blinding untouched.
    let c_parent_a = {
        let outputs = vec![
            b.output(Pay::ForeignExplicit("payer-a"), 150_000, lbtc, &domain),
            b.output(Pay::Foreign("payer-b"), 20_000, lbtc, &domain),
            TxOut::new_fee(100, lbtc),
        ];
        tx(vec![input(outpoint("external:ca", 0), vec![])], outputs)
    };
    let c_fund = {
        let outputs = vec![
            b.output(Pay::Receive(0), 100_000, lbtc, &domain),
            b.output(Pay::Explicit(1), 50_000, lbtc, &domain),
            b.output(Pay::Receive(0), 7_000, issued, &domain),
            b.output(Pay::ForeignExplicit("payer-change"), 19_700, lbtc, &domain),
            TxOut::new_fee(300, lbtc),
        ];
        tx(
            vec![
                input(OutPoint::new(c_parent_a.txid(), 0), vec![]),
                input(OutPoint::new(c_parent_a.txid(), 1), vec![]),
            ],
            outputs,
        )
    };
    let c_spend = {
        let outputs = vec![
            b.output(Pay::Foreign("merchant"), 60_000, lbtc, &domain),
            b.output(Pay::Change(0), 39_500, lbtc, &domain),
            TxOut::new_fee(500, lbtc),
        ];
        tx(
            vec![input(
                OutPoint::new(c_fund.txid(), 0),
                vec![vec![0x30; 71], vec![0x02; 33]],
            )],
            outputs,
        )
    };
    let c_parent_b = {
        let outputs = vec![b.output(Pay::ForeignExplicit("coinjoiner"), 50_000, lbtc, &domain)];
        tx(vec![input(outpoint("external:cb", 0), vec![])], outputs)
    };
    let c_mixed = {
        let outputs = vec![
            b.output(Pay::Foreign("coinjoin"), 70_000, lbtc, &domain),
            b.output(Pay::Change(1), 29_000, lbtc, &domain),
            TxOut::new_fee(1_000, lbtc),
        ];
        tx(
            vec![
                input(OutPoint::new(c_fund.txid(), 1), vec![]),
                input(OutPoint::new(c_parent_b.txid(), 0), vec![]),
            ],
            outputs,
        )
    };
    let c_parent_htlc = {
        let outputs = vec![
            b.output(Pay::ForeignExplicit("htlc-claim"), 20_200, lbtc, &domain),
            b.output(Pay::ForeignExplicit("htlc-refund"), 15_150, lbtc, &domain),
        ];
        tx(vec![input(outpoint("external:chtlc", 0), vec![])], outputs)
    };
    let c_claim = {
        let outputs = vec![
            b.output(Pay::Receive(3), 20_000, lbtc, &domain),
            TxOut::new_fee(200, lbtc),
        ];
        tx(
            vec![input(
                OutPoint::new(c_parent_htlc.txid(), 0),
                vec![
                    vec![0x30; 71],
                    preimage.to_vec(),
                    vec![1],
                    htlc_script(&preimage),
                ],
            )],
            outputs,
        )
    };
    let c_refund = {
        let outputs = vec![
            b.output(Pay::Change(2), 15_000, lbtc, &domain),
            TxOut::new_fee(150, lbtc),
        ];
        tx(
            vec![input(
                OutPoint::new(c_parent_htlc.txid(), 1),
                vec![
                    vec![0x30; 71],
                    vec![],
                    htlc_script(&tagged("preimage:refund")),
                ],
            )],
            outputs,
        )
    };
    let c_asset_send = {
        let outputs = vec![
            b.output(Pay::Foreign("asset-buyer"), 7_000, issued, &domain),
            b.output(Pay::Change(4), 19_800, lbtc, &domain),
            TxOut::new_fee(200, lbtc),
        ];
        tx(
            vec![
                input(OutPoint::new(c_fund.txid(), 2), vec![]),
                input(OutPoint::new(c_claim.txid(), 0), vec![]),
            ],
            outputs,
        )
    };
    let c_mempool = {
        let outputs = vec![
            b.output(Pay::Foreign("cafe"), 10_000, lbtc, &domain),
            b.output(Pay::Change(3), 29_000, lbtc, &domain),
            TxOut::new_fee(500, lbtc),
        ];
        tx(
            vec![input(OutPoint::new(c_spend.txid(), 1), vec![])],
            outputs,
        )
    };
    let c_parent_c = {
        let outputs = vec![b.output(Pay::ForeignExplicit("payer-c"), 5_100, lbtc, &domain)];
        tx(vec![input(outpoint("external:cc", 0), vec![])], outputs)
    };
    let c_wrong_key = {
        let outputs = vec![
            b.output(Pay::WrongKey(2), 5_000, lbtc, &domain),
            TxOut::new_fee(100, lbtc),
        ];
        tx(
            vec![input(OutPoint::new(c_parent_c.txid(), 0), vec![])],
            outputs,
        )
    };
    let compat_initial = [
        (&c_parent_a, 1),
        (&c_parent_b, 1),
        (&c_parent_htlc, 1),
        (&c_fund, 2),
        (&c_spend, 3),
        (&c_mixed, 4),
        (&c_claim, 5),
        (&c_refund, 5),
        (&c_asset_send, 6),
        (&c_mempool, 0),
    ];
    let compat_reorg = [
        (&c_parent_a, 1),
        (&c_parent_b, 1),
        (&c_parent_htlc, 1),
        (&c_fund, 2),
        (&c_spend, 3),
        (&c_claim, 5),
        (&c_refund, 5),
        (&c_asset_send, 6),
        (&c_mempool, 7),
    ];
    // A later block that only touches receive script 5: unchanged scripts
    // keep their histories, and their outputs' transactions are fetched anew.
    let c_parent_d = {
        let outputs = vec![b.output(Pay::ForeignExplicit("payer-d"), 8_100, lbtc, &domain)];
        tx(vec![input(outpoint("external:cd", 0), vec![])], outputs)
    };
    let c_extra = {
        let outputs = vec![
            b.output(Pay::Receive(5), 8_000, lbtc, &domain),
            TxOut::new_fee(100, lbtc),
        ];
        tx(
            vec![input(OutPoint::new(c_parent_d.txid(), 0), vec![])],
            outputs,
        )
    };
    let mut compat_more = compat_initial.to_vec();
    compat_more.extend([(&c_parent_d, 1), (&c_extra, 7)]);
    let mut compat_wrong_key = compat_initial.to_vec();
    compat_wrong_key.extend([(&c_parent_c, 1), (&c_wrong_key, 4)]);
    // A wallet over the same spend key with a private view key
    // (`ct(<tprv>/<0;1>/*, ...)`), which LWK rejects: Python's compatibility
    // observers unblind with embit's view-key derivation. Built last, so
    // every chain above keeps its bytes.
    use lwk_wollet::Chain::{External, Internal};
    let v_parent = {
        let outputs = vec![
            b.output(Pay::ForeignExplicit("view-payer"), 130_000, lbtc, &domain),
            b.output(Pay::Foreign("view-payer-b"), 12_000, lbtc, &domain),
            TxOut::new_fee(100, lbtc),
        ];
        tx(vec![input(outpoint("external:va", 0), vec![])], outputs)
    };
    let v_fund = {
        let outputs = vec![
            b.output(Pay::View(External, 0), 90_000, lbtc, &domain),
            b.output(Pay::Explicit(1), 30_000, lbtc, &domain),
            b.output(Pay::View(External, 2), 6_000, issued, &domain),
            b.output(
                Pay::ForeignExplicit("view-payer-change"),
                15_700,
                lbtc,
                &domain,
            ),
            TxOut::new_fee(300, lbtc),
        ];
        tx(
            vec![
                input(OutPoint::new(v_parent.txid(), 0), vec![]),
                input(OutPoint::new(v_parent.txid(), 1), vec![]),
            ],
            outputs,
        )
    };
    let v_spend = {
        let outputs = vec![
            b.output(Pay::Foreign("view-merchant"), 50_000, lbtc, &domain),
            b.output(Pay::View(Internal, 0), 39_600, lbtc, &domain),
            TxOut::new_fee(400, lbtc),
        ];
        tx(
            vec![input(
                OutPoint::new(v_fund.txid(), 0),
                vec![vec![0x30; 71], vec![0x02; 33]],
            )],
            outputs,
        )
    };
    let v_mempool = {
        let outputs = vec![
            b.output(Pay::ForeignExplicit("view-cafe"), 9_000, lbtc, &domain),
            b.output(Pay::View(Internal, 1), 30_300, lbtc, &domain),
            TxOut::new_fee(300, lbtc),
        ];
        tx(
            vec![input(OutPoint::new(v_spend.txid(), 1), vec![])],
            outputs,
        )
    };
    let v_parent_b = {
        let outputs = vec![b.output(Pay::ForeignExplicit("view-payer-d"), 7_100, lbtc, &domain)];
        tx(vec![input(outpoint("external:vb", 0), vec![])], outputs)
    };
    let v_extra = {
        let outputs = vec![
            b.output(Pay::View(External, 4), 7_000, lbtc, &domain),
            TxOut::new_fee(100, lbtc),
        ];
        tx(
            vec![input(OutPoint::new(v_parent_b.txid(), 0), vec![])],
            outputs,
        )
    };
    let view_initial = [(&v_parent, 1), (&v_fund, 2), (&v_spend, 3), (&v_mempool, 0)];
    let mut view_more = view_initial.to_vec();
    view_more.extend([(&v_parent_b, 1), (&v_extra, 4)]);
    // Block 3 is replaced: the spend confirms one block later and the
    // mempool spend of its change drops out.
    let view_reorg = [(&v_parent, 1), (&v_fund, 2), (&v_spend, 4)];
    // Wallets with one fixed branch, receive script 7 of the same key:
    // `ct(slip77(...), elwpkh(xpub/0/7))` and `ct(<WIF>, elwpkh(xpub/0/7))`.
    // The first phase pays it explicitly only; the others add a
    // confidential output each wallet's own key blinds.
    let f_parent = {
        let outputs = vec![
            b.output(Pay::ForeignExplicit("fixed-payer"), 60_000, lbtc, &domain),
            b.output(Pay::ForeignExplicit("fixed-payer-b"), 40_000, lbtc, &domain),
            TxOut::new_fee(100, lbtc),
        ];
        tx(vec![input(outpoint("external:fa", 0), vec![])], outputs)
    };
    let f_explicit = {
        let outputs = vec![
            b.output(Pay::Explicit(7), 20_000, lbtc, &domain),
            b.output(Pay::ForeignExplicit("fixed-change"), 39_800, lbtc, &domain),
            TxOut::new_fee(200, lbtc),
        ];
        tx(
            vec![input(OutPoint::new(f_parent.txid(), 0), vec![])],
            outputs,
        )
    };
    let f_slip77 = {
        let outputs = vec![
            b.output(Pay::Receive(7), 25_000, lbtc, &domain),
            b.output(
                Pay::ForeignExplicit("fixed-change-b"),
                14_800,
                lbtc,
                &domain,
            ),
            TxOut::new_fee(200, lbtc),
        ];
        tx(
            vec![input(OutPoint::new(f_parent.txid(), 1), vec![])],
            outputs,
        )
    };
    let f_view = {
        let outputs = vec![
            b.output(Pay::FixedView(7), 18_000, lbtc, &domain),
            b.output(
                Pay::ForeignExplicit("fixed-change-c"),
                21_800,
                lbtc,
                &domain,
            ),
            TxOut::new_fee(200, lbtc),
        ];
        tx(
            vec![input(OutPoint::new(f_parent.txid(), 1), vec![])],
            outputs,
        )
    };
    let fixed_explicit = [(&f_parent, 1), (&f_explicit, 2)];
    let fixed_slip77 = [(&f_parent, 1), (&f_explicit, 2), (&f_slip77, 3)];
    let fixed_view = [(&f_parent, 1), (&f_explicit, 2), (&f_view, 3)];
    let fixed_wif = lwk_wollet::elements::bitcoin::PrivateKey::new(
        fixed_view_key(),
        lwk_wollet::elements::bitcoin::NetworkKind::Test,
    );
    let mut fixed_wif_uncompressed = fixed_wif;
    fixed_wif_uncompressed.compressed = false;
    let view_key = b.view.to_string();
    let view_xpub = Xpub::from_priv(
        &lwk_wollet::elements::bitcoin::secp256k1::Secp256k1::new(),
        &b.view,
    );
    let spend_key = DESCRIPTOR
        .split("elwpkh(")
        .nth(1)
        .unwrap()
        .trim_end_matches("))");
    let names = json!({
        "fund": fund.txid().to_string(), "spend": spend.txid().to_string(),
        "mixed": mixed.txid().to_string(), "wrong_key": wrong_key.txid().to_string(),
        "claim": claim.txid().to_string(), "refund": refund.txid().to_string(),
        "asset_send": asset_send.txid().to_string(), "mempool": mempool.txid().to_string(),
        "lbtc_fund": lbtc_fund.txid().to_string(), "lbtc_spend": lbtc_spend.txid().to_string(),
        "lbtc_mixed": lbtc_mixed.txid().to_string(), "lbtc_mempool": lbtc_mempool.txid().to_string(),
        "c_fund": c_fund.txid().to_string(), "c_spend": c_spend.txid().to_string(),
        "c_mixed": c_mixed.txid().to_string(), "c_claim": c_claim.txid().to_string(),
        "c_refund": c_refund.txid().to_string(), "c_asset_send": c_asset_send.txid().to_string(),
        "c_mempool": c_mempool.txid().to_string(), "c_wrong_key": c_wrong_key.txid().to_string(),
        "c_extra": c_extra.txid().to_string(),
        "v_fund": v_fund.txid().to_string(), "v_spend": v_spend.txid().to_string(),
        "v_mempool": v_mempool.txid().to_string(), "v_extra": v_extra.txid().to_string(),
    });
    let vectors = json!({
        "descriptor": DESCRIPTOR,
        "network": "elementsregtest",
        "policy_asset": POLICY_ASSET,
        "issued_asset": ISSUED_ASSET,
        "view_descriptor": format!("ct({view_key}/<0;1>/*,elwpkh({spend_key}))"),
        "public_view_descriptor": format!("ct({view_xpub}/<0;1>/*,elwpkh({spend_key}))"),
        "legacy_set_view_descriptor": format!("ct({view_key}/{{0,1}}/*,elwpkh({}))", spend_key.replace("/<0;1>/*", "/{0,1}/*")),
        "fixed_slip77_descriptor": DESCRIPTOR.replace("/<0;1>/*", "/0/7"),
        "fixed_view_descriptor": format!("ct({fixed_wif},elwpkh({}))", spend_key.replace("/<0;1>/*", "/0/7")),
        "fixed_view_uncompressed_descriptor": format!("ct({fixed_wif_uncompressed},elwpkh({}))", spend_key.replace("/<0;1>/*", "/0/7")),
        "transactions": names,
        "phases": {
            "empty": phase(&b, 3, None, &[]),
            "initial": phase(&b, 6, None, &initial),
            "reorg": phase(&b, 7, Some(4), &reorg),
            "lagging": phase(&b, 3, None, &initial[..2]),
            "lbtc_initial": phase(&b, 6, None, &lbtc_initial),
            "lbtc_reorg": phase(&b, 7, Some(4), &lbtc_reorg),
            "lbtc_lagging": phase(&b, 3, None, &lbtc_initial[..2]),
            "compat_initial": phase(&b, 6, None, &compat_initial),
            "compat_reorg": phase(&b, 7, Some(4), &compat_reorg),
            "compat_wrong_key": phase(&b, 6, None, &compat_wrong_key),
            "compat_more": phase(&b, 7, None, &compat_more),
            "view_initial": phase(&b, 3, None, &view_initial),
            "view_more": phase(&b, 4, None, &view_more),
            "view_reorg": phase(&b, 4, Some(3), &view_reorg),
            "fixed_explicit": phase(&b, 2, None, &fixed_explicit),
            "fixed_slip77": phase(&b, 3, None, &fixed_slip77),
            "fixed_view": phase(&b, 3, None, &fixed_view),
        },
    });
    println!("{}", serde_json::to_string_pretty(&vectors).unwrap());
}
