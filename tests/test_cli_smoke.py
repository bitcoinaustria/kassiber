"""End-to-end CLI smoke test.

This exercises the AGENTS.md "safe local workflow" against a temp data root
and asserts that every command returns the expected JSON envelope shape
(kind, schema_version, data).

It is intentionally stdlib-only (no pytest dep) and invokes the CLI as a
subprocess so it pins the external contract — not implementation details.
The suite is designed to survive a pure-refactor split of kassiber/app.py
into modules.
"""

import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


_PHOENIX_CSV = """date,id,type,amount_msat,amount_fiat,fee_credit_msat,mining_fee_sat,mining_fee_fiat,service_fee_msat,service_fee_fiat,payment_hash,tx_id,destination,description
2024-05-01T10:15:00Z,11111111-aaaa-bbbb-cccc-000000000001,swap_in,5000000000,2000 USD,0,250,0.10 USD,0,0 USD,,abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789,bc1qexamplefakedestination0000000000000000,Onchain deposit
2024-05-02T12:00:00Z,22222222-aaaa-bbbb-cccc-000000000002,lightning_received,3000000,1.20 USD,0,0,0 USD,0,0 USD,1111111111111111111111111111111111111111111111111111111111111111,,03abcdefnodepubkeyfakefakefakefakefakefakefakefakefakefakefakefake,Tip from friend
2024-05-03T14:30:00Z,33333333-aaaa-bbbb-cccc-000000000003,lightning_sent,-5000000,-2.00 USD,0,0,0 USD,50000,0.02 USD,2222222222222222222222222222222222222222222222222222222222222222,,03deadbeefcafebabefakefakefakefakefakefakefakefakefakefakefakefake,Coffee shop
2024-05-04T09:00:00Z,44444444-aaaa-bbbb-cccc-000000000004,channel_close,-500000000,-200 USD,0,1500,0.60 USD,0,0 USD,,fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210,bc1qexamplefakechannelclose0000000000000000,Channel close to self
"""

_RIVER_CSV = """Date,Reference Code,Transaction Type,Sent Amount,Sent Currency,Received Amount,Received Currency,Fee Amount,Fee Currency,Total Amount,Total Currency,Method,Source,Destination,Cost Basis Amount,Cost Basis Currency,Bitcoin Price Amount,Bitcoin Price Currency,Transaction ID,Recurring,Tag
2026-01-01T12:00:00Z,RIV-BUY-1,Buy,1000.00,USD,0.01000000,BTC,5.00,USD,-1005.00,USD,ACH,Linked bank,Bitcoin balance,,,100000.00,USD,,False,Buy
2026-01-02T12:00:00Z,RIV-WD-1,Automatic Withdrawal,0.00200000,BTC,,,0.00001000,BTC,-0.00201000,BTC,on-chain,Bitcoin balance,bc1qriverwithdrawal,,,60000.00,USD,aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,False,Withdrawal
2026-01-03T12:00:00Z,RIV-INT-1,Interest,,BTC,0.00010000,BTC,0,BTC,0.00010000,BTC,internal,River interest,Bitcoin balance,,,62000.00,USD,,False,Interest
"""

_BULLBITCOIN_EXISTING_CSV = """date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,kind,description,counterparty
2026-04-15T09:50:00Z,bull-sell-tx,outbound,BTC,0.06811291,0.00001413,4277.69,62802.96,withdrawal,Synced from wallet,Synced from fulcrum
"""

_BULLBITCOIN_OTHER_WALLET_EXISTING_CSV = """date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,kind,description,counterparty
2026-04-16T09:50:00Z,other-wallet-tx,outbound,BTC,0.01000000,0.00000100,610.00,61000.00,withdrawal,Synced from other wallet,Synced from fulcrum
"""

_BULLBITCOIN_ORDERS_CSV = """ORDER_NUMBER,ORDER_TYPE,ORDER_SUBTYPE,MESSAGE,ORDER_ID,PAYIN_AMOUNT,PAYIN_CURRENCY,PAYOUT_AMOUNT,PAYOUT_CURRENCY,EXCHANGE_RATE_AMOUNT,EXCHANGE_RATE_CURRENCY,PAYIN_METHOD,PAYOUT_METHOD,ORDER_STATUS,PAYIN_STATUS,PAYOUT_STATUS,CREATED_AT (UTC),COMPLETED_AT (UTC),SENT_AT (UTC),INDEX_RATE_AMOUNT,INDEX_RATE_CURRENCY,TRANSACTION_ID,ADDRESS
1001,Fiat Payment,Market Order,,order-1,0.06811291,BTC,4202.19,USD,61694.45,USD,Bitcoin On-Chain,SEPA Transfer (USD),Completed,Completed,Completed,2026-04-15 09:40:00.000Z,2026-04-15 09:50:23.370Z,2026-04-15 09:51:00.000Z,62868.74,USD,bull-sell-tx,bc1qbullsell
1002,Fiat Payment,Market Order,,order-2,0.01000000,BTC,600.00,USD,60000.00,USD,Bitcoin On-Chain,SEPA Transfer (USD),Completed,Completed,Completed,2026-04-16 09:40:00.000Z,2026-04-16 09:50:00.000Z,2026-04-16 09:51:00.000Z,60000.00,USD,other-wallet-tx,bc1qotherwallet
1003,Fiat Payment,Market Order,,order-3,0.02000000,BTC,1200.00,USD,60000.00,USD,Bitcoin On-Chain,SEPA Transfer (USD),Canceled,Awaiting payment,Not started,2026-04-17 09:40:00.000Z,,,,60000.00,USD,,bc1qcanceled
"""

_BULLBITCOIN_LN_EXISTING_CSV = """date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,kind,description,counterparty
2026-04-18T09:50:00Z,bull-ln-buy-tx,inbound,BTC,0.01000000,0,600.00,60000.00,deposit,Synced from LN wallet,Synced from lightning
"""

_BULLBITCOIN_LN_ORDERS_CSV = """ORDER_NUMBER,ORDER_TYPE,ORDER_SUBTYPE,MESSAGE,ORDER_ID,PAYIN_AMOUNT,PAYIN_CURRENCY,PAYOUT_AMOUNT,PAYOUT_CURRENCY,EXCHANGE_RATE_AMOUNT,EXCHANGE_RATE_CURRENCY,PAYIN_METHOD,PAYOUT_METHOD,ORDER_STATUS,PAYIN_STATUS,PAYOUT_STATUS,CREATED_AT (UTC),COMPLETED_AT (UTC),SENT_AT (UTC),INDEX_RATE_AMOUNT,INDEX_RATE_CURRENCY,TRANSACTION_ID,ADDRESS
1004,Fiat Payment,Market Order,,order-ln-1,600.00,USD,0.01000000,BTC,60000.00,USD,SEPA Transfer (USD),Bitcoin Lightning,Completed,Completed,Completed,2026-04-18 09:40:00.000Z,2026-04-18 09:50:00.000Z,2026-04-18 09:51:00.000Z,60000.00,USD,bull-ln-buy-tx,lnbc1example
"""

_BULLBITCOIN_WALLET_CSV = """date,type,direction,amount_sats,amount_btc,fee_sats,status,txid,network,address,swap_id,preimage,total_swap_fees_sats,send_network,receive_network,send_txid,receive_txid
2026-01-15T10:30:00Z,onchain,incoming,500000,0.00500000,0,confirmed,bull-wallet-btc-in,bitcoin,bc1qsalary,,,,,,,
2026-02-01T08:00:00Z,liquid,outgoing,200000,0.00200000,350,confirmed,bull-wallet-lbtc-out,liquid,lq1merchant,,,,,,,
2026-03-10T14:22:00Z,lightning_send,outgoing,50000,0.00050000,150,completed,cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc,lightning,,swap-ln,1111111111111111111111111111111111111111111111111111111111111111,125,,,,
2026-04-05T09:15:00Z,chain_swap,outgoing,1000000,0.01000000,500,completed,bull-chain-send,bitcoin,VJLCbLBTCksDqx1,swap-chain,,750,bitcoin,liquid,bull-chain-send,bull-chain-recv
2026-04-05T09:20:00Z,chain_swap,incoming,990000,0.00990000,200,completed,bull-chain-recv,liquid,VJLCbLBTCksDqx1,swap-chain,,750,bitcoin,liquid,bull-chain-send,bull-chain-recv
2026-04-06T09:20:00Z,onchain,self,10000,0.00010000,120,confirmed,bull-self,bitcoin,bc1qself,,,,,,,
2026-04-07T09:20:00Z,onchain,outgoing,10000,0.00010000,120,failed,bull-failed,bitcoin,bc1qfailed,,,,,,,
"""

_BULLBITCOIN_WALLET_REFUND_CSV = """date,type,direction,amount_sats,amount_btc,fee_sats,status,txid,network,address,swap_id,preimage,total_swap_fees_sats,send_network,receive_network,send_txid,receive_txid
2026-04-08T09:15:00Z,chain_swap,outgoing,1000000,0.01000000,500,refunded,bull-refund-lockup,bitcoin,bc1qrefund,swap-refund,,2500,bitcoin,bitcoin,bull-refund-lockup,
"""

_COINFINITY_EXISTING_CSV = """date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,kind,description,counterparty
2026-05-11T13:19:36Z,coinfinity-buy-tx,inbound,BTC,0.00147403,0,100.00,68872.40,deposit,Synced from self custody wallet,Synced from node
"""

_COINFINITY_ORDERS_CSV = """"Order ID",Type,Date,"Amount EUR","Amount Crypto",Crypto,"Rate EUR","Mining Fee Crypto","Mining Fee EUR","Service Fee EUR","Total Fee EUR",Address,Transaction,"LN Invoice","Transaction type"
BCBC-229A-AB,sell,"2026-05-11 13:19:36",100.00,0.00147403,BTC,68872.400000000000000000000000,,,1.52,1.52,bc1qcoinfinityreceive,coinfinity-buy-tx,,Onchain
BFBC-EAC9-38,buy,"2026-02-20 13:50:45",3000.00,0.05153544,BTC,57337.700000000000000000000000,0.00000134,0.08,45.00,45.08,bc1qcoinfinitysell,coinfinity-sell-tx,,Onchain
"""

_COINFINITY_LN_EXISTING_CSV = """date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,kind,description,counterparty
2026-05-12T08:00:00Z,lnbc1coinfinityinvoice,inbound,BTC,0.01000000,0,500.00,50000.00,deposit,Synced from LN wallet,Synced from lightning
2026-05-12T08:10:00Z,lnbc1otherinvoice,inbound,BTC,0.01000000,0,500.00,50000.00,deposit,Synced from LN wallet,Synced from lightning
"""

_COINFINITY_LN_ORDERS_CSV = """"Order ID",Type,Date,"Amount EUR","Amount Crypto",Crypto,"Rate EUR","Mining Fee Crypto","Mining Fee EUR","Service Fee EUR","Total Fee EUR",Address,Transaction,"LN Invoice","Transaction type"
BCBC-LN-01,sell,"2026-05-12 08:00:00",500.00,0.01000000,BTC,50000.00,,,5.00,5.00,,,"lnbc1coinfinityinvoice",Lightning
"""

_TWENTYONEBITCOIN_EXISTING_CSV = """date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,kind,description,counterparty
2022-06-01T03:00:42Z,21bitcoin:2,inbound,BTC,0.00049106,0,36.93,75204.25,buy,Synced from wallet,Synced from 21bitcoin
2022-10-07T16:31:20Z,l1-withdrawal-tx,outbound,BTC,0.00040000,0.00001000,,,withdrawal,Synced withdrawal,Synced from 21bitcoin
"""

_TWENTYONEBITCOIN_RECEIVE_CSV = """date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,kind,description,counterparty
2022-10-07T16:31:20Z,l1-withdrawal-tx,inbound,BTC,0.00040000,0,,,receive,Synced receive,Self custody
"""

_TWENTYONEBITCOIN_TRANSACTIONS_CSV = """id,exchange_name,depot_name,transaction_date,buy_asset,buy_amount,sell_asset,sell_amount,fee_asset,fee_amount,transaction_type,note,linked_transaction
1,21bitcoin,main,01.01.222 03:00:39,EUR,22.67,,,,,deposit,Promotion Payout EUR Deposit,
2,21bitcoin,main,01.06.222 03:00:42,BTC,0.00049106,EUR,36.93,EUR,0.56,trade,Promotion Payout BTC Purchase,
16,21bitcoin,main,07.10.2022 16:31:20,,,BTC,0.00040000,BTC,0.00001,withdrawal,Automatic Limit L1 BTC Withdrawal,l1-withdrawal-tx
"""

_STRIKE_CSV = """Reference,Date & Time (UTC),Transaction Type,Amount EUR,Fee EUR,Amount BTC,Fee BTC,BTC Price,Cost Basis (EUR),Destination,Description,Transaction Hash,Note
strike-fiat-1,Mar 07 2026 21:59:51,Deposit,100.00,,-,-,,,,,,Bank deposit
strike-buy-1,Mar 08 2026 09:15:00,Buy,-100.00,1.00,0.00100000,-,100000.00,100.00,,BTC purchase,,Cash balance
strike-sell-1,Mar 08 2026 09:45:00,Sell,49.00,1.00,-0.00050000,-,100000.00,50.00,,BTC sale,,Cash balance
strike-ln-1,Mar 08 2026 10:37:13,Receive,,,0.00272794,,,,lnbc1sampleinvoice,,aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,Transfer Coinos
strike-price-only,Mar 08 2026 11:00:00,,,,0.00050000,-,80000.00,,,Price-only inbound,,
strike-chain-1,Mar 09 2026 10:37:13,Send,,,-0.00100000,0.00001000,60000.00,55.00,bc1qstrikewithdrawal,On-chain withdrawal,bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,Self custody
"""

_LEDGERLIVE_CSV = """Operation Date,Currency Ticker,Operation Type,Operation Amount,Operation Fees,Operation Hash,Account Name,Account xpub,Countervalue Ticker,Countervalue at Operation Date
2026-06-01T08:00:00.000Z,BTC,IN,0.01000000,,ledger-in,Bitcoin,xpub-secret,USD,600.00
2026-06-02T08:00:00.000Z,BTC,OUT,-0.00200000,0.00001000,ledger-out,Bitcoin,xpub-secret,USD,120.00
"""

_BINANCE_SUPPLEMENTAL_CSV = """timestamp UTC,base asset symbol,quote asset amount + symbol,trading fee (in quote asset),base asset amount + symbol,source of funds
2026-06-03 10:00:00,BTC,100.00 USD,1.00 USD,0.002 BTC,Spot Wallet
"""

_POCKETBITCOIN_EXISTING_CSV = """date,txid,direction,asset,amount,fee,description
2022-07-19T23:15:28Z,pocket-wallet-tx,inbound,BTC,0.00228101,0,Synced from wallet
"""

_POCKETBITCOIN_CSV = """type,date,reference,price.currency,price.amount,cost.currency,cost.amount,fee.currency,fee.amount,value.currency,value.amount
withdrawal,2022-07-19T23:15:28.000Z,,,,,,BTC,0.00000046,BTC,0.00228101
exchange,2022-07-19T12:35:37.130Z,REF000001,EUR,21586.90000000,EUR,49.25000000,EUR,0.75000000,BTC,0.00228147
deposit,2022-07-19T12:35:37.130Z,REF000002,,,,,EUR,0.00000000,EUR,50.00000000
"""

_CACHE_PRICING_CSV = """date,txid,direction,asset,amount,fee,description
2024-05-10T09:00:00Z,cache-price-1,inbound,BTC,0.01000000,0,Cached price sample
"""

_CONFIRMED_PRICING_CSV = """date,confirmed_at,txid,direction,asset,amount,fee,description
2024-05-09T09:00:00Z,2024-05-10T12:00:00Z,confirmed-price-1,inbound,BTC,0.01000000,0,Confirmed price sample
"""

_L_BTC_CONFIRMED_PRICING_CSV = """date,confirmed_at,txid,direction,asset,amount,fee,description
2024-05-09T09:00:00Z,2024-05-10T12:00:00Z,lbtc-confirmed-price-1,inbound,L-BTC,0.01000000,0,Liquid confirmed price sample
"""

# Cross-wallet self-transfer scenario: cold wallet receives 1 BTC, then sends
# 0.5 BTC + 0.001 BTC network fee to the hot wallet. The same on-chain txid
# appears in both wallet exports, which is the trigger for IntraTransaction
# detection. With detection on: only the 0.001 BTC network fee is realized as
# a disposal; the 0.5 BTC transfer carries its cost basis to the hot wallet.
_COLD_TRANSFER_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-01T10:00:00Z,cold-funding-1,inbound,BTC,1.00000000,0,60000,Cold acquisition
2026-02-01T12:00:00Z,onchain-self-transfer-1,outbound,BTC,0.50000000,0.001,65000,Move to hot wallet
"""

_HOT_TRANSFER_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-02-01T12:00:00Z,onchain-self-transfer-1,inbound,BTC,0.50000000,0,65000,Receive from cold wallet
"""

_COLD_TRANSFER_VALUE_ONLY_CSV = """date,txid,direction,asset,amount,fee,fiat_value,description
2026-01-01T10:00:00Z,cold-funding-value-1,inbound,BTC,1.00000000,0,60000,Cold acquisition
2026-02-01T12:00:00Z,onchain-self-transfer-value-1,outbound,BTC,0.50000000,0.001,32500,Move to hot wallet
"""

_HOT_TRANSFER_VALUE_ONLY_CSV = """date,txid,direction,asset,amount,fee,fiat_value,description
2026-02-01T12:00:00Z,onchain-self-transfer-value-1,inbound,BTC,0.50000000,0,32500,Receive from cold wallet
"""

_FEE_ONLY_CONSOLIDATION_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-01T10:00:00Z,fee-only-funding-1,inbound,BTC,1.00000000,0,60000,Funding
2026-02-01T12:00:00Z,fee-only-consolidation-1,outbound,BTC,0,0.001,65000,Wallet consolidation fee
"""

# Split-peg scenario: one outbound (0.04702253) fans out to an owned wallet
# (0.02750000, the returned change) AND a Liquid peg (~0.0195, a non-owned
# federation address that produces no owned inbound). Auto-pairing sees only the
# 1-out/1-in BTC shape and would otherwise absorb the ~0.0195 peg as a transfer
# "fee" and tax it as a disposal. The implausible-fee guard must quarantine it.
_SPLIT_PEG_COLD_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-01T10:00:00Z,splitpeg-funding-1,inbound,BTC,0.10000000,0,60000,Cold acquisition
2026-05-31T15:06:39Z,splitpeg-1,outbound,BTC,0.04702253,0,63255,Spend split between hot wallet and Liquid peg
"""
_SPLIT_PEG_HOT_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-05-31T15:06:39Z,splitpeg-1,inbound,BTC,0.02750000,0,63255,Returned change portion
"""

# Per-account over-sell: "Onchain" sells BTC on 2026-01-15, but its funding
# transfer from "Source" only arrives 2026-02-01. The coins exist globally (in
# Source), so the old global gate passed the sell and rp2's per-account
# BalanceSet then crashed the whole report; the per-account gate must instead
# quarantine just the sell and compute the rest.
_OVERSELL_SOURCE_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-01T10:00:00Z,oversell-fund,inbound,BTC,1.00000000,0,60000,Source funding
2026-02-01T12:00:00Z,oversell-move,outbound,BTC,0.50000000,0,65000,Move to Onchain
"""
_OVERSELL_ONCHAIN_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-15T09:00:00Z,oversell-sell,outbound,BTC,0.30000000,0,62000,Sell before funded
2026-02-01T12:00:00Z,oversell-move,inbound,BTC,0.50000000,0,65000,Receive from Source
"""

# Same-timestamp buy + sell in one wallet: the buy must fund the sell regardless
# of import/uuid order (IN sorts before OUT at an equal timestamp).
_SAMETS_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-04-01T12:00:00Z,samets-buy,inbound,BTC,0.50000000,0,70000,Same-second buy
2026-04-01T12:00:00Z,samets-sell,outbound,BTC,0.50000000,0,71000,Same-second sell
"""

# A gift disposal (kind=gift) must be quarantined, not taxed as a market SELL.
_GIFT_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description,kind
2026-01-01T10:00:00Z,gift-fund,inbound,BTC,0.20000000,0,60000,Funding,buy
2026-03-01T10:00:00Z,gift-out,outbound,BTC,0.05000000,0,72000,Gift to a friend,gift
"""

# An income-looking inbound kind that isn't a recognized earn type (kind=reward)
# must be quarantined for classification, not silently booked as a plain buy.
_REWARD_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description,kind
2026-01-01T10:00:00Z,reward-fund,inbound,BTC,0.10000000,0,60000,Funding,buy
2026-02-01T10:00:00Z,reward-in,inbound,BTC,0.01000000,0,65000,Referral reward,reward
"""

# Split swap: one 0.05 BTC spend returns 0.03 to an owned wallet (self-transfer)
# and pegs 0.02 to Liquid. Pairing the BTC out with the L-BTC in and declaring
# --out-amount 0.02 must split it into a clean self-transfer MOVE + a
# carrying-value peg, not a single transfer with a 0.02 "fee".
_SPLIT_SWAP_SPEND_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2025-06-01T10:00:00Z,splitswap-fund,inbound,BTC,0.10000000,0,30000,Funding
2025-09-01T12:00:00Z,splitswap-out,outbound,BTC,0.05000000,0,60000,Spend: change back + peg to Liquid
"""
_SPLIT_SWAP_KEEP_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2025-09-01T12:00:00Z,splitswap-out,inbound,BTC,0.03000000,0,60000,Change returned on-chain
"""
_SPLIT_SWAP_LBTC_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2025-09-01T12:30:00Z,splitswap-peg,inbound,LBTC,0.01980000,0,60000,Pegged-in L-BTC
"""

# Basis provenance: an early acquisition is dropped for coarse pricing. A later
# sell is funded per-account (0.7 held) but FIFO-consumes past the 0.2 priced
# before the drop, so it would re-base onto the wrong lot and must be
# quarantined (basis_provenance_incomplete) rather than silently mis-based.
_BASIS_PROVENANCE_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description,pricing_quality
2025-12-01T10:00:00Z,h2-acq0,inbound,BTC,0.20000000,0,55000,Priced acquisition,
2026-01-01T10:00:00Z,h2-acq1,inbound,BTC,0.50000000,0,60000,Coarse acquisition,coarse_fallback
2026-02-01T10:00:00Z,h2-acq2,inbound,BTC,0.50000000,0,70000,Priced acquisition,
2026-03-01T10:00:00Z,h2-sell,outbound,BTC,0.50000000,0,72000,Sell,
"""

# Unclassified income before a priced acquisition+sale: the dropped income lot
# is missing from the FIFO, so the later sale must be flagged
# basis_provenance_incomplete (not silently re-based onto the wrong lot).
_INCOME_PROVENANCE_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description,kind
2026-01-01T10:00:00Z,inc-reward,inbound,BTC,0.01000000,0,60000,Referral reward,reward
2026-02-01T10:00:00Z,inc-fund,inbound,BTC,0.50000000,0,65000,Funding,buy
2026-03-01T10:00:00Z,inc-sell,outbound,BTC,0.30000000,0,70000,Sell,sell
"""

# A quarantined gift is still a real outflow: it must debit availability so a
# later sale of the (now-gone) coins is gated insufficient, not booked.
_GIFT_DEBIT_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description,kind
2026-01-01T10:00:00Z,gd-fund,inbound,BTC,0.10000000,0,60000,Funding,buy
2026-02-01T10:00:00Z,gd-gift,outbound,BTC,0.08000000,0,72000,Gift to a friend,gift
2026-03-01T10:00:00Z,gd-sell,outbound,BTC,0.05000000,0,73000,Sell the rest,sell
"""

# Manual same-asset pair scenario: two BTC legs whose external_ids deliberately
# don't match, so auto-detection skips them. The user knows they're paired
# (e.g., a swap via a custom counterparty) and creates a manual pair.
_MANUAL_FROM_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-03-01T10:00:00Z,manual-fund-1,inbound,BTC,0.20000000,0,70000,Acquisition
2026-03-15T10:00:00Z,manual-out-leg,outbound,BTC,0.10000000,0.0005,72000,Manual swap out
"""

_MANUAL_TO_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-03-15T10:05:00Z,manual-in-leg,inbound,BTC,0.10000000,0,72000,Manual swap in
"""

_FAILED_SWAP_REFUND_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-03-01T09:00:00Z,refund-fund,inbound,BTC,0.20000000,0,70000,Funding
2026-03-02T09:00:00Z,failed-swap-send,outbound,BTC,0.10000000,0.00010000,72000,Failed swap send
2026-03-02T11:00:00Z,failed-swap-refund,inbound,BTC,0.09980000,0,72000,Refund from failed swap
"""

# A failed swap whose on-chain refund carries the funding (lockup) txid link
# that chain sync stamps on transactions.swap_refund_funding_txid. The lockup
# and refund share one wallet and sit days apart, so only the deterministic
# link (not the time+amount heuristic) can pair them. The funding txid must be
# a real 64-hex value to survive normalize_import_record's validation.
_LOCKUP_TXID = "aa" * 32
_REFUND_TXID = "bb" * 32
_FUNDING_TXID = "cc" * 32
_FAILED_SWAP_REFUND_LINKED_CSV = (
    "date,txid,direction,asset,amount,fee,fiat_rate,description,swap_refund_funding_txid\n"
    f"2026-02-25T09:00:00Z,{_FUNDING_TXID},inbound,BTC,0.20000000,0,70000,Funding,\n"
    f"2026-03-02T09:00:00Z,{_LOCKUP_TXID},outbound,BTC,0.10000000,0.00010000,72000,Swap lockup,\n"
    f"2026-03-05T11:00:00Z,{_REFUND_TXID},inbound,BTC,0.09980000,0,72000,Refund from failed swap,{_LOCKUP_TXID}\n"
)

# Cross-asset (BTC → LBTC) scenario for the carrying-value rejection +
# taxable acceptance tests.
_CROSS_BTC_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-04-01T10:00:00Z,cross-fund-1,inbound,BTC,0.10000000,0,80000,BTC acquisition
2026-04-15T10:00:00Z,cross-out-leg,outbound,BTC,0.10000000,0.0001,82000,Peg-in to Liquid
"""

_CROSS_BTC_AT_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-04-01T10:00:00Z,cross-fund-1,inbound,BTC,0.10010000,0,80000,BTC acquisition with fee buffer
2026-04-15T10:00:00Z,cross-out-leg,outbound,BTC,0.10000000,0.0001,82000,Peg-in to Liquid
"""

_CROSS_LBTC_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-04-15T10:30:00Z,cross-in-leg,inbound,LBTC,0.10000000,0,82000,Peg-in receive
"""


def _sample_descriptor_pair():
    from embit import bip32

    seed = bytes.fromhex("000102030405060708090a0b0c0d0e0f" * 4)
    root = bip32.HDKey.from_seed(seed)
    account = root.derive("m/84h/0h/0h")
    xpub = account.to_public().to_base58()
    fingerprint = root.my_fingerprint.hex()
    origin = f"[{fingerprint}/84h/0h/0h]"
    return (
        f"wpkh({origin}{xpub}/0/*)",
        f"wpkh({origin}{xpub}/1/*)",
        "m/84'/0'/0'",
        fingerprint,
    )


def _sample_multisig_branch_descriptor():
    from embit import bip32

    keys = []
    for marker in range(1, 5):
        root = bip32.HDKey.from_seed(bytes([marker]) * 64)
        account = root.derive("m/48h/0h/0h/2h")
        keys.append(
            f"[{root.my_fingerprint.hex()}/48h/0h/0h/2h]{account.to_public().to_base58()}/<0;1>/*"
        )
    return "wsh(\n  sortedmulti(\n    2,\n    " + ",\n    ".join(keys) + "\n  )\n)\n"


def _run(data_root, *args, input_text=None):
    """Invoke `python -m kassiber --data-root DATA --machine ARGS...`.

    Returns (payload_dict, returncode). Never raises on non-zero exit; the
    caller asserts on the returncode when an error envelope is expected.
    """
    cmd = [
        sys.executable,
        "-m",
        "kassiber",
        "--data-root",
        str(data_root),
        "--machine",
        *args,
    ]
    result = subprocess.run(
        cmd,
        cwd=ROOT,
        capture_output=True,
        text=True,
        input=input_text,
        check=False,
    )
    stdout = result.stdout.strip()
    if not stdout:
        raise AssertionError(
            f"CLI produced no stdout.\nargs: {args}\nstderr: {result.stderr}"
        )
    try:
        payload = json.loads(stdout)
    except json.JSONDecodeError as exc:
        raise AssertionError(
            f"CLI stdout was not JSON.\nargs: {args}\nstdout: {stdout[:400]}"
        ) from exc
    return payload, result.returncode


def _unescape_xml(text):
    return (
        text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", '"')
        .replace("&apos;", "'")
    )


def _load_xlsx_sheets(path):
    """Parse a workbook into {sheet_name: {row_number: {col_letter: value}}}.

    Numbers become floats; shared/inline strings become Python strings. Used to
    evaluate the verification sheets' reconciliation in Python the way Excel
    would, without a formula engine.
    """
    with zipfile.ZipFile(path) as workbook:
        names = re.findall(r'<sheet name="([^"]+)"', workbook.read("xl/workbook.xml").decode("utf-8"))
        try:
            ss_xml = workbook.read("xl/sharedStrings.xml").decode("utf-8")
        except KeyError:
            ss_xml = ""
        shared = [
            _unescape_xml("".join(re.findall(r"<t[^>]*>(.*?)</t>", si, re.DOTALL)))
            for si in re.findall(r"<si>(.*?)</si>", ss_xml, re.DOTALL)
        ]
        sheets = {}
        for index, name in enumerate(names, start=1):
            xml = workbook.read(f"xl/worksheets/sheet{index}.xml").decode("utf-8")
            rows = {}
            # Handle both self-closing (<c .../>) and content (<c ...>...</c>) cells;
            # merged/blank cells are self-closing and carry no value.
            for cell in re.finditer(r'<c r="([A-Z]+)(\d+)"([^>]*?)(?:/>|>(.*?)</c>)', xml, re.DOTALL):
                col, rownum, attrs, body = cell.group(1), int(cell.group(2)), cell.group(3), cell.group(4)
                if body is None:
                    continue
                vmatch = re.search(r"<v>(.*?)</v>", body, re.DOTALL)
                if vmatch is None:
                    continue
                raw = vmatch.group(1)
                if 't="s"' in attrs:
                    value = shared[int(raw)]
                elif 't="str"' in attrs:
                    value = _unescape_xml(raw)
                else:
                    value = float(raw)
                rows.setdefault(rownum, {})[col] = value
            sheets[_unescape_xml(name)] = rows
    return sheets


class CliSmokeTest(unittest.TestCase):
    """Walks through init → workspace → profile → wallet → Phoenix import →
    journals → reports → rates, asserting envelope shape at each step.

    Tests run in alphabetical order (unittest default); the test_NN_ prefix
    is what sequences them.
    """

    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-smoke-")
        cls.data_root = Path(cls._tmp.name) / "data"
        cls.phoenix_csv = Path(cls._tmp.name) / "phoenix.csv"
        cls.phoenix_csv.write_text(_PHOENIX_CSV, encoding="utf-8")
        cls.cache_pricing_csv = Path(cls._tmp.name) / "cache-pricing.csv"
        cls.cache_pricing_csv.write_text(_CACHE_PRICING_CSV, encoding="utf-8")
        cls.confirmed_pricing_csv = Path(cls._tmp.name) / "confirmed-pricing.csv"
        cls.confirmed_pricing_csv.write_text(_CONFIRMED_PRICING_CSV, encoding="utf-8")
        cls.lbtc_confirmed_pricing_csv = Path(cls._tmp.name) / "lbtc-confirmed-pricing.csv"
        cls.lbtc_confirmed_pricing_csv.write_text(_L_BTC_CONFIRMED_PRICING_CSV, encoding="utf-8")
        cls.cold_transfer_csv = Path(cls._tmp.name) / "cold-transfer.csv"
        cls.cold_transfer_csv.write_text(_COLD_TRANSFER_CSV, encoding="utf-8")
        cls.hot_transfer_csv = Path(cls._tmp.name) / "hot-transfer.csv"
        cls.hot_transfer_csv.write_text(_HOT_TRANSFER_CSV, encoding="utf-8")
        cls.cold_transfer_value_only_csv = Path(cls._tmp.name) / "cold-transfer-value-only.csv"
        cls.cold_transfer_value_only_csv.write_text(_COLD_TRANSFER_VALUE_ONLY_CSV, encoding="utf-8")
        cls.hot_transfer_value_only_csv = Path(cls._tmp.name) / "hot-transfer-value-only.csv"
        cls.hot_transfer_value_only_csv.write_text(_HOT_TRANSFER_VALUE_ONLY_CSV, encoding="utf-8")
        cls.fee_only_consolidation_csv = Path(cls._tmp.name) / "fee-only-consolidation.csv"
        cls.fee_only_consolidation_csv.write_text(_FEE_ONLY_CONSOLIDATION_CSV, encoding="utf-8")
        cls.split_peg_cold_csv = Path(cls._tmp.name) / "split-peg-cold.csv"
        cls.split_peg_cold_csv.write_text(_SPLIT_PEG_COLD_CSV, encoding="utf-8")
        cls.split_peg_hot_csv = Path(cls._tmp.name) / "split-peg-hot.csv"
        cls.split_peg_hot_csv.write_text(_SPLIT_PEG_HOT_CSV, encoding="utf-8")
        cls.oversell_source_csv = Path(cls._tmp.name) / "oversell-source.csv"
        cls.oversell_source_csv.write_text(_OVERSELL_SOURCE_CSV, encoding="utf-8")
        cls.oversell_onchain_csv = Path(cls._tmp.name) / "oversell-onchain.csv"
        cls.oversell_onchain_csv.write_text(_OVERSELL_ONCHAIN_CSV, encoding="utf-8")
        cls.samets_csv = Path(cls._tmp.name) / "samets.csv"
        cls.samets_csv.write_text(_SAMETS_CSV, encoding="utf-8")
        cls.gift_csv = Path(cls._tmp.name) / "gift.csv"
        cls.gift_csv.write_text(_GIFT_CSV, encoding="utf-8")
        cls.reward_csv = Path(cls._tmp.name) / "reward.csv"
        cls.reward_csv.write_text(_REWARD_CSV, encoding="utf-8")
        cls.income_provenance_csv = Path(cls._tmp.name) / "income-provenance.csv"
        cls.income_provenance_csv.write_text(_INCOME_PROVENANCE_CSV, encoding="utf-8")
        cls.gift_debit_csv = Path(cls._tmp.name) / "gift-debit.csv"
        cls.gift_debit_csv.write_text(_GIFT_DEBIT_CSV, encoding="utf-8")
        cls.basis_provenance_csv = Path(cls._tmp.name) / "basis-provenance.csv"
        cls.basis_provenance_csv.write_text(_BASIS_PROVENANCE_CSV, encoding="utf-8")
        cls.split_swap_spend_csv = Path(cls._tmp.name) / "split-swap-spend.csv"
        cls.split_swap_spend_csv.write_text(_SPLIT_SWAP_SPEND_CSV, encoding="utf-8")
        cls.split_swap_keep_csv = Path(cls._tmp.name) / "split-swap-keep.csv"
        cls.split_swap_keep_csv.write_text(_SPLIT_SWAP_KEEP_CSV, encoding="utf-8")
        cls.split_swap_lbtc_csv = Path(cls._tmp.name) / "split-swap-lbtc.csv"
        cls.split_swap_lbtc_csv.write_text(_SPLIT_SWAP_LBTC_CSV, encoding="utf-8")
        cls.manual_from_csv = Path(cls._tmp.name) / "manual-from.csv"
        cls.manual_from_csv.write_text(_MANUAL_FROM_CSV, encoding="utf-8")
        cls.manual_to_csv = Path(cls._tmp.name) / "manual-to.csv"
        cls.manual_to_csv.write_text(_MANUAL_TO_CSV, encoding="utf-8")
        cls.failed_swap_refund_csv = Path(cls._tmp.name) / "failed-swap-refund.csv"
        cls.failed_swap_refund_csv.write_text(_FAILED_SWAP_REFUND_CSV, encoding="utf-8")
        cls.failed_swap_refund_linked_csv = Path(cls._tmp.name) / "failed-swap-refund-linked.csv"
        cls.failed_swap_refund_linked_csv.write_text(
            _FAILED_SWAP_REFUND_LINKED_CSV, encoding="utf-8"
        )
        cls.cross_btc_csv = Path(cls._tmp.name) / "cross-btc.csv"
        cls.cross_btc_csv.write_text(_CROSS_BTC_CSV, encoding="utf-8")
        cls.cross_btc_at_csv = Path(cls._tmp.name) / "cross-btc-at.csv"
        cls.cross_btc_at_csv.write_text(_CROSS_BTC_AT_CSV, encoding="utf-8")
        cls.cross_lbtc_csv = Path(cls._tmp.name) / "cross-lbtc.csv"
        cls.cross_lbtc_csv.write_text(_CROSS_LBTC_CSV, encoding="utf-8")
        cls.attachment_file = Path(cls._tmp.name) / "attachment-note.txt"
        cls.attachment_file.write_text("Signed invoice copy\n", encoding="utf-8")
        (
            cls.sample_descriptor,
            cls.sample_change_descriptor,
            cls.sample_derivation_root,
            cls.sample_fingerprint,
        ) = _sample_descriptor_pair()
        cls.sample_multisig_descriptor_pretty = _sample_multisig_branch_descriptor()
        cls.multisig_descriptor_file = Path(cls._tmp.name) / "multisig-descriptor.txt"
        cls.multisig_descriptor_file.write_text(cls.sample_multisig_descriptor_pretty, encoding="utf-8")

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def _cli(self, *args, input_text=None):
        payload, code = _run(self.data_root, *args, input_text=input_text)
        if code != 0:
            self.fail(
                f"CLI exited {code} for {args!r}; envelope: {json.dumps(payload)[:400]}"
            )
        self.assertEqual(payload.get("schema_version"), 1)
        self.assertIn("data", payload)
        return payload

    def _assert_kind(self, payload, expected):
        self.assertEqual(payload.get("kind"), expected)

    # -- workflow -----------------------------------------------------

    def test_01_init_status(self):
        payload = self._cli("init")
        self._assert_kind(payload, "init")
        self.assertEqual(payload["data"]["state_root"], str(self.data_root.parent))
        self.assertEqual(payload["data"]["config_root"], str(self.data_root.parent / "config"))
        self.assertEqual(payload["data"]["settings_file"], str(self.data_root.parent / "config" / "settings.json"))
        self.assertEqual(payload["data"]["exports_root"], str(self.data_root.parent / "exports"))
        self.assertEqual(payload["data"]["attachments_root"], str(self.data_root.parent / "attachments"))
        self.assertEqual(payload["data"]["env_file"], str(self.data_root.parent / "config" / "backends.env"))

        payload = self._cli("status")
        self._assert_kind(payload, "status")
        auth = payload["data"].get("auth", {})
        self.assertEqual(auth.get("mode"), "local")
        self.assertTrue(auth.get("authenticated"))
        self.assertEqual(payload["data"]["state_root"], str(self.data_root.parent))
        self.assertEqual(payload["data"]["config_root"], str(self.data_root.parent / "config"))
        self.assertEqual(payload["data"]["settings_file"], str(self.data_root.parent / "config" / "settings.json"))
        self.assertEqual(payload["data"]["exports_root"], str(self.data_root.parent / "exports"))
        self.assertEqual(payload["data"]["attachments_root"], str(self.data_root.parent / "attachments"))
        self.assertEqual(payload["data"]["env_file"], str(self.data_root.parent / "config" / "backends.env"))

        payload = self._cli("diagnostics", "collect")
        self._assert_kind(payload, "diagnostics.collect")
        self.assertIsNone(payload["data"]["saved"])
        report = payload["data"]["report"]
        self.assertTrue(report["public_safe"])
        self.assertEqual(report["storage"]["diagnostics_location"], "exports/diagnostics")
        self.assertIn("counts", report["state"])

    def test_01a_backends_batch_size_roundtrip(self):
        payload = self._cli(
            "backends", "create", "bench",
            "--kind", "electrum",
            "--url", "ssl://electrum.example:50002",
            "--batch-size", "25",
        )
        self._assert_kind(payload, "backends.create")
        self.assertEqual(payload["data"]["batch_size"], 25)

        payload = self._cli(
            "backends", "update", "bench",
            "--batch-size", "40",
        )
        self._assert_kind(payload, "backends.update")
        self.assertEqual(payload["data"]["batch_size"], 40)

        payload = self._cli("backends", "get", "bench")
        self._assert_kind(payload, "backends.get")
        self.assertEqual(payload["data"]["batch_size"], 40)

        payload = self._cli("backends", "list")
        self._assert_kind(payload, "backends.list")
        rows = {row["name"]: row for row in payload["data"]}
        self.assertEqual(rows["bench"]["batch_size"], 40)
        self.assertEqual(rows["fulcrum"]["batch_size"], 100)
        self.assertEqual(rows["liquid"]["batch_size"], 100)
        self.assertEqual(rows["liquid-blockstream"]["batch_size"], 100)

    def test_01b_ai_providers_roundtrip(self):
        # Seeded local Ollama row should be present with the local-default
        # marker; api_key is never echoed in any redacted payload.
        payload = self._cli("ai", "providers", "list")
        self._assert_kind(payload, "ai.providers.list")
        self.assertEqual(payload["data"]["default"], "ollama")
        names = [row["name"] for row in payload["data"]["providers"]]
        self.assertIn("ollama", names)
        for row in payload["data"]["providers"]:
            self.assertNotIn("api_key", row)
            self.assertIn("has_api_key", row)
            self.assertIn("kind", row)

        payload = self._cli(
            "ai", "providers", "create", "smoke-remote",
            "--base-url", "https://example.test/v1",
            "--api-key-stdin",
            "--default-model", "test-model",
            "--kind", "remote",
            "--notes", "Smoke test remote",
            input_text="sk-test-secret\n",
        )
        self._assert_kind(payload, "ai.providers.create")
        encoded_payload = json.dumps(payload)
        self.assertNotIn("sk-test-secret", encoded_payload)
        self.assertEqual(payload["data"]["name"], "smoke-remote")
        self.assertEqual(payload["data"]["kind"], "remote")
        self.assertTrue(payload["data"]["has_api_key"])
        self.assertNotIn("api_key", payload["data"])
        self.assertEqual(
            payload["data"]["secret_ref"],
            {"store_id": "sqlcipher_inline", "state": "ok"},
        )
        # Remote providers are not auto-acknowledged.
        self.assertIsNone(payload["data"].get("acknowledged_at"))

        error_payload, code = _run(
            self.data_root,
            "chat", "hello",
            "--provider", "smoke-remote",
            input_text="",
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(error_payload["error"]["code"], "ai_remote_ack_required")

        payload = self._cli("ai", "providers", "get", "smoke-remote")
        self.assertIsNone(payload["data"].get("acknowledged_at"))

        payload = self._cli(
            "ai", "providers", "update", "smoke-remote",
            "--default-model", "test-model-2",
            "--acknowledge",
        )
        self._assert_kind(payload, "ai.providers.update")
        self.assertEqual(payload["data"]["default_model"], "test-model-2")
        self.assertIsNotNone(payload["data"]["acknowledged_at"])

        payload = self._cli("ai", "providers", "set-default", "smoke-remote")
        self._assert_kind(payload, "ai.providers.set-default")
        self.assertEqual(payload["data"]["default"], "smoke-remote")

        payload = self._cli("ai", "providers", "clear-default")
        self._assert_kind(payload, "ai.providers.clear-default")
        self.assertIsNone(payload["data"]["default"])

        payload = self._cli("ai", "providers", "delete", "smoke-remote")
        self._assert_kind(payload, "ai.providers.delete")
        self.assertTrue(payload["data"]["deleted"])

    def test_02_workspace_profile(self):
        payload = self._cli("workspaces", "create", "Main")
        self._assert_kind(payload, "workspaces.create")

        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "Default",
        )
        self._assert_kind(payload, "profiles.create")

        payload = self._cli("profiles", "list")
        self._assert_kind(payload, "profiles.list")
        profiles = payload["data"]
        self.assertIsInstance(profiles, list)
        self.assertEqual(len(profiles), 1)
        prof = profiles[0]
        self.assertIn("tax_country", prof)
        self.assertIn("tax_long_term_days", prof)
        self.assertEqual(prof["tax_country"], "generic")
        self.assertEqual(prof["fiat_currency"], "USD")

    def test_03_wallet_create(self):
        payload = self._cli(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "Phoenix",
            "--kind", "phoenix",
        )
        self._assert_kind(payload, "wallets.create")
        self.assertEqual(payload["data"]["label"], "Phoenix")
        self.assertEqual(payload["data"]["kind"], "phoenix")

    def test_03a_descriptor_derive_exposes_paths(self):
        payload = self._cli(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "Vault",
            "--kind", "descriptor",
            "--descriptor", self.sample_descriptor,
            "--change-descriptor", self.sample_change_descriptor,
            "--gap-limit", "5",
        )
        self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "derive",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Vault",
            "--count", "2",
        )
        self._assert_kind(payload, "wallets.derive")
        rows = payload["data"]
        self.assertEqual(len(rows), 4)

        receive_0 = rows[0]
        self.assertEqual(receive_0["branch_label"], "receive")
        self.assertEqual(receive_0["derivation_path"], f"{self.sample_derivation_root}/0/0")
        self.assertEqual(receive_0["derivation_paths"], [f"{self.sample_derivation_root}/0/0"])
        self.assertEqual(receive_0["key_origins"], [f"[{self.sample_fingerprint}/84'/0'/0'/0/0]"])

        change_0 = rows[2]
        self.assertEqual(change_0["branch_label"], "change")
        self.assertEqual(change_0["derivation_path"], f"{self.sample_derivation_root}/1/0")
        self.assertEqual(change_0["derivation_paths"], [f"{self.sample_derivation_root}/1/0"])
        self.assertEqual(change_0["key_origins"], [f"[{self.sample_fingerprint}/84'/0'/0'/1/0]"])

        payload = self._cli(
            "wallets", "derive",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Vault",
            "--branch", "change",
            "--start", "1",
            "--count", "1",
        )
        self._assert_kind(payload, "wallets.derive")
        change_only = payload["data"]
        self.assertEqual(len(change_only), 1)
        self.assertEqual(change_only[0]["branch_label"], "change")
        self.assertEqual(change_only[0]["derivation_path"], f"{self.sample_derivation_root}/1/1")

    def test_03b_descriptor_file_accepts_pretty_printed_multisig(self):
        payload = self._cli(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "Pretty Vault",
            "--kind", "descriptor",
            "--descriptor-file", str(self.multisig_descriptor_file),
            "--gap-limit", "5",
        )
        self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "derive",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Pretty Vault",
            "--count", "1",
        )
        self._assert_kind(payload, "wallets.derive")
        rows = payload["data"]
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["branch_label"], "receive")
        self.assertEqual(rows[1]["branch_label"], "change")
        self.assertEqual(len(rows[0]["key_origins"]), 4)

    def test_03c_wallet_identify_classifies_ownership(self):
        derived = self._cli(
            "wallets", "derive",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Vault",
            "--branch", "receive",
            "--start", "0",
            "--count", "1",
        )
        owned = derived["data"][0]["address"]
        external = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"

        payload = self._cli(
            "wallets", "identify",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Vault",
            "--address", owned,
            "--address", external,
            "--txid", "not-a-valid-txid",
            "--scan-to-index", "5",
        )
        self._assert_kind(payload, "wallets.identify")
        data = payload["data"]
        self.assertEqual(data["summary"]["owned"], 1)
        self.assertEqual(data["summary"]["external"], 1)
        self.assertEqual(data["summary"]["invalid"], 1)
        self.assertFalse(data["summary"]["verified_on_chain"])

        by_input = {row["input"]: row for row in data["results"]}
        self.assertEqual(by_input[owned]["status"], "owned")
        self.assertEqual(by_input[owned]["matches"][0]["branch"], "receive")
        self.assertEqual(by_input[owned]["matches"][0]["wallet"], "Vault")
        self.assertEqual(by_input[external]["status"], "external")

        # --candidate auto-detects address vs txid, and --file reads a mixed
        # list (with comments/blank lines) — exercise both at the CLI layer.
        list_file = os.path.join(self._tmp.name, "reconcile.txt")
        with open(list_file, "w", encoding="utf-8") as handle:
            handle.write(f"# reconcile list\n{owned}\n\n{external}\n")
        payload = self._cli(
            "wallets", "identify",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Vault",
            "--candidate", owned,
            "--file", list_file,
            "--scan-to-index", "5",
        )
        self._assert_kind(payload, "wallets.identify")
        # owned appears via both --candidate and --file but dedupes to one row.
        candidate_inputs = [row["input"] for row in payload["data"]["results"]]
        self.assertEqual(candidate_inputs.count(owned), 1)
        self.assertEqual(payload["data"]["summary"]["owned"], 1)
        self.assertEqual(payload["data"]["summary"]["external"], 1)

        # --csv smart-imports a messy spreadsheet (semicolon delimiter, noise
        # columns, an oddly-named txid column) and harvests only the tokens.
        csv_file = os.path.join(self._tmp.name, "reconcile.csv")
        with open(csv_file, "w", encoding="utf-8") as handle:
            handle.write("Date;Amount;Wallet Address;Memo;Tx Hash\n")
            handle.write(f"2024-01-01;0.5;{owned};rent;{'a' * 64}\n")
            handle.write(f"2024-02-01;1.0;{external};coffee;\n")
        payload = self._cli(
            "wallets", "identify",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Vault",
            "--csv", csv_file,
            "--scan-to-index", "5",
        )
        self._assert_kind(payload, "wallets.identify")
        csv_summary = payload["data"]["summary"]
        self.assertEqual(csv_summary["owned"], 1)
        self.assertEqual(csv_summary["external"], 1)
        self.assertEqual(csv_summary["unknown"], 1)  # the harvested txid
        csv_by_input = {row["input"]: row for row in payload["data"]["results"]}
        self.assertEqual(csv_by_input[owned]["status"], "owned")
        self.assertNotIn("0.5", csv_by_input)  # amounts/dates/memos are not harvested

        # No candidates is a validation error, not an empty success.
        error_payload, code = _run(
            self.data_root,
            "wallets", "identify",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(error_payload["kind"], "error")
        self.assertEqual(error_payload["error"]["code"], "validation")

    def test_04_phoenix_import(self):
        payload = self._cli(
            "wallets", "import-phoenix",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Phoenix",
            "--file", str(self.phoenix_csv),
        )
        self._assert_kind(payload, "wallets.import-phoenix")
        data = payload["data"]
        self.assertEqual(data["imported"], 4)
        self.assertEqual(data["skipped"], 0)
        self.assertEqual(data["phoenix_notes_set"], 4)
        self.assertEqual(data["phoenix_tags_added"], 4)
        self.assertEqual(data["phoenix_tags_created"], 4)

    def test_05_msat_exposed_on_records(self):
        payload = self._cli(
            "metadata", "records", "list",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_kind(payload, "metadata.records.list")
        records = payload["data"]["records"]
        self.assertEqual(len(records), 4)
        for rec in records:
            # dual BTC/msat fields must be present on every record
            self.assertIn("amount", rec)
            self.assertIn("amount_msat", rec)
            self.assertIsInstance(rec["amount_msat"], int)
            self.assertIn("fee_msat", rec)
            self.assertIsInstance(rec["fee_msat"], int)
        # expected msat totals from the 4-row Phoenix sample
        inbound_msat = sum(r["amount_msat"] for r in records if r["direction"] == "inbound")
        outbound_msat = sum(r["amount_msat"] for r in records if r["direction"] == "outbound")
        self.assertEqual(inbound_msat, 5_000_000_000 + 3_000_000)
        self.assertEqual(outbound_msat, 5_000_000 + 500_000_000)

    def test_05a_attachments_lifecycle(self):
        payload = self._cli(
            "metadata", "records", "list",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_kind(payload, "metadata.records.list")
        tx_ref = payload["data"]["records"][0]["transaction_id"]

        payload = self._cli(
            "attachments", "add",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", tx_ref,
            "--file", str(self.attachment_file),
            "--label", "Invoice copy",
        )
        self._assert_kind(payload, "attachments.add")
        file_attachment = payload["data"]
        self.assertEqual(file_attachment["attachment_type"], "file")
        self.assertEqual(file_attachment["label"], "Invoice copy")
        self.assertTrue(file_attachment["exists"])
        stored_path = self.data_root.parent / "attachments" / file_attachment["stored_relpath"]
        self.assertTrue(stored_path.exists())

        payload = self._cli(
            "attachments", "add",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", tx_ref,
            "--url", "https://docs.google.com/spreadsheets/d/abc123/edit?usp=sharing",
        )
        self._assert_kind(payload, "attachments.add")
        url_attachment = payload["data"]
        self.assertEqual(url_attachment["attachment_type"], "url")
        self.assertIsNone(url_attachment["label"])
        self.assertEqual(url_attachment["display_label"], "Google Sheet")
        self.assertEqual(
            url_attachment["url"],
            "https://docs.google.com/spreadsheets/d/abc123/edit?usp=sharing",
        )
        self.assertFalse(url_attachment["stored_relpath"])

        payload, code = _run(
            self.data_root,
            "attachments", "rename",
            "--workspace", "Main",
            "--profile", "Default",
            file_attachment["id"],
            "--label", "Receipt copy",
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload["kind"], "error")
        self.assertEqual(payload["error"]["code"], "validation")

        payload, code = _run(
            self.data_root,
            "attachments", "rename",
            "--workspace", "Main",
            "--profile", "Default",
            url_attachment["id"],
            "--label", "x" * 201,
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload["kind"], "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertEqual(payload["error"]["details"]["max_length"], 200)

        payload = self._cli(
            "attachments", "rename",
            "--workspace", "Main",
            "--profile", "Default",
            url_attachment["id"],
            "--label", "Support ticket",
        )
        self._assert_kind(payload, "attachments.rename")
        self.assertEqual(payload["data"]["label"], "Support ticket")
        self.assertEqual(payload["data"]["display_label"], "Support ticket")

        payload = self._cli(
            "attachments", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", tx_ref,
        )
        self._assert_kind(payload, "attachments.list")
        rows = payload["data"]
        self.assertEqual(len(rows), 2)
        self.assertEqual(sorted(row["attachment_type"] for row in rows), ["file", "url"])
        by_type = {row["attachment_type"]: row for row in rows}
        self.assertEqual(by_type["url"]["display_label"], "Support ticket")

        payload = self._cli(
            "attachments", "verify",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", tx_ref,
        )
        self._assert_kind(payload, "attachments.verify")
        self.assertEqual(payload["data"]["checked"], 2)
        self.assertEqual(payload["data"]["broken"], 0)
        self.assertEqual(payload["data"]["ok"], 2)
        by_type = {row["attachment_type"]: row for row in payload["data"]["results"]}
        self.assertEqual(by_type["file"]["status"], "ok")
        self.assertEqual(by_type["file"]["issues"], [])
        self.assertEqual(by_type["url"]["status"], "ok")
        self.assertEqual(by_type["url"]["issues"], [])

        payload = self._cli(
            "attachments", "remove",
            "--workspace", "Main",
            "--profile", "Default",
            file_attachment["id"],
        )
        self._assert_kind(payload, "attachments.remove")
        self.assertTrue(payload["data"]["removed"])
        self.assertTrue(payload["data"]["deleted_file"])
        self.assertFalse(stored_path.exists())

        payload = self._cli(
            "attachments", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", tx_ref,
        )
        self._assert_kind(payload, "attachments.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["id"], url_attachment["id"])

    def test_06_journals_process(self):
        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_kind(payload, "journals.process")
        data = payload["data"]
        # 2 acquisitions + 2 disposals, 0 quarantined (fiat_rate derived from value/amount)
        self.assertEqual(data["entries_created"], 4)
        self.assertEqual(data["quarantined"], 0)
        self.assertEqual(data["processed_transactions"], 4)

    def test_07_all_reports_succeed(self):
        for report, kind in [
            ("summary", "reports.summary"),
            ("tax-summary", "reports.tax-summary"),
            ("balance-sheet", "reports.balance-sheet"),
            ("portfolio-summary", "reports.portfolio-summary"),
            ("capital-gains", "reports.capital-gains"),
            ("journal-entries", "reports.journal-entries"),
        ]:
            payload = self._cli(
                "reports", report,
                "--workspace", "Main",
                "--profile", "Default",
            )
            self._assert_kind(payload, kind)
        payload = self._cli(
            "reports", "balance-history",
            "--workspace", "Main",
            "--profile", "Default",
            "--interval", "month",
        )
        self._assert_kind(payload, "reports.balance-history")

    def test_07b_summary_report_rollups(self):
        payload = self._cli(
            "reports", "summary",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_kind(payload, "reports.summary")
        data = payload["data"]
        self.assertEqual(data["workspace"], "Main")
        self.assertEqual(data["profile"], "Default")
        self.assertIsNone(data["wallet"])
        self.assertEqual(data["metrics"]["wallets_in_scope"], 3)
        self.assertEqual(data["metrics"]["active_transactions"], 4)
        self.assertEqual(data["metrics"]["journal_entries"], 4)
        self.assertEqual(data["metrics"]["quarantines"], 0)
        self.assertEqual(len(data["asset_flow"]), 1)
        flow = data["asset_flow"][0]
        self.assertEqual(flow["asset"], "BTC")
        self.assertEqual(flow["fee_amount_msat"], 1800000)
        self.assertAlmostEqual(float(flow["fee_amount"]), 0.000018, places=8)
        self.assertEqual(data["transfer_pairs"], [])

    def test_07a_export_pdf_report(self):
        pdf_path = Path(self._tmp.name) / "kassiber-report.pdf"
        if pdf_path.exists():
            pdf_path.unlink()
        payload = self._cli(
            "reports", "export-pdf",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(pdf_path),
        )
        self._assert_kind(payload, "reports.export-pdf")
        data = payload["data"]
        self.assertEqual(Path(data["file"]), pdf_path.resolve())
        self.assertGreaterEqual(data["pages"], 1)
        self.assertTrue(pdf_path.exists())
        self.assertGreater(pdf_path.stat().st_size, 1000)
        payload_bytes = pdf_path.read_bytes()
        header = payload_bytes[:8]
        self.assertTrue(header.startswith(b"%PDF-1.4"))
        self.assertRegex(payload_bytes, rb"/MediaBox \[0 0 842(?:\.0+)? 595(?:\.0+)?\]")

        summary_pdf_path = Path(self._tmp.name) / "kassiber-summary-report.pdf"
        if summary_pdf_path.exists():
            summary_pdf_path.unlink()
        summary_payload = self._cli(
            "reports", "export-summary-pdf",
            "--workspace", "Main",
            "--profile", "Default",
            "--start", "2026-01-01T00:00:00Z",
            "--end", "2026-12-31T23:59:59Z",
            "--file", str(summary_pdf_path),
        )
        self._assert_kind(summary_payload, "reports.export-summary-pdf")
        summary_data = summary_payload["data"]
        self.assertEqual(Path(summary_data["file"]), summary_pdf_path.resolve())
        self.assertFalse(summary_data["snapshot"])
        self.assertGreaterEqual(len(summary_data["wallets"]), 1)
        self.assertEqual(summary_data["timeframe"]["label"], "2026-01-01 to 2026-12-31")
        self.assertTrue(summary_pdf_path.exists())
        self.assertEqual(summary_pdf_path.read_bytes()[:4], b"%PDF")

    def test_07ab_export_csv_and_xlsx_report(self):
        csv_path = Path(self._tmp.name) / "kassiber-report.csv"
        xlsx_path = Path(self._tmp.name) / "kassiber-report.xlsx"
        for path in (csv_path, xlsx_path):
            if path.exists():
                path.unlink()

        payload = self._cli(
            "reports", "export-csv",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(csv_path),
        )
        self._assert_kind(payload, "reports.export-csv")
        self.assertEqual(Path(payload["data"]["file"]), csv_path.resolve())
        self.assertIn("Overview", payload["data"]["sections"])
        self.assertIn("Transfers & Swaps", payload["data"]["sections"])
        self.assertIn("Transactions", payload["data"]["sections"])
        csv_text = csv_path.read_text(encoding="utf-8")
        self.assertIn("Kassiber Report - Default", csv_text)
        self.assertIn("Wallet Inventory", csv_text)
        self.assertIn("Reviewed Transfers and Swaps", csv_text)
        self.assertIn("Transaction ID", csv_text)
        self.assertIn("Onchain deposit", csv_text)

        payload = self._cli(
            "reports", "export-xlsx",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(xlsx_path),
        )
        self._assert_kind(payload, "reports.export-xlsx")
        self.assertEqual(Path(payload["data"]["file"]), xlsx_path.resolve())
        self.assertEqual(xlsx_path.read_bytes()[:2], b"PK")
        self.assertIn("Overview", payload["data"]["sheets"])
        self.assertIn("Transfers & Swaps", payload["data"]["sheets"])
        self.assertIn("Transactions", payload["data"]["sheets"])
        with zipfile.ZipFile(xlsx_path) as workbook:
            workbook_xml = workbook.read("xl/workbook.xml").decode("utf-8")
            shared_strings = workbook.read("xl/sharedStrings.xml").decode("utf-8")
        self.assertIn('name="Overview"', workbook_xml)
        self.assertIn('name="Transfers &amp; Swaps"', workbook_xml)
        self.assertIn('name="Transactions"', workbook_xml)
        self.assertIn("Executive summary", shared_strings)
        self.assertIn("Wallet Inventory", shared_strings)

    def test_07ac_export_xlsx_self_verifying(self):
        verify_path = Path(self._tmp.name) / "kassiber-report-verify.xlsx"
        plain_path = Path(self._tmp.name) / "kassiber-report-plain.xlsx"
        for path in (verify_path, plain_path):
            if path.exists():
                path.unlink()

        def _read_workbook(path):
            with zipfile.ZipFile(path) as workbook:
                names = re.findall(r'<sheet name="([^"]+)"', workbook.read("xl/workbook.xml").decode("utf-8"))
                sheets = {}
                for index, name in enumerate(names, start=1):
                    sheets[name.replace("&amp;", "&")] = workbook.read(
                        f"xl/worksheets/sheet{index}.xml"
                    ).decode("utf-8")
                shared = workbook.read("xl/sharedStrings.xml").decode("utf-8")
            return sheets, shared

        # Verification is on by default and appends the verify sheets.
        payload = self._cli(
            "reports", "export-xlsx",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(verify_path),
        )
        self._assert_kind(payload, "reports.export-xlsx")
        self.assertTrue(payload["data"]["verified"])
        for sheet in ("Verify", "Acquisitions", "Disposals", "Control"):
            self.assertIn(sheet, payload["data"]["sheets"])

        sheets, shared = _read_workbook(verify_path)
        for sheet in ("Verify", "Acquisitions", "Disposals", "Control"):
            self.assertIn(sheet, sheets)
        # The Control sheet recomputes every figure with live formulas.
        control = sheets["Control"]
        self.assertIn("<f>", control)
        self.assertIn("SUMIFS(", control)
        self.assertIn("Verify!$B$3", control)  # checks reference the tolerance cell
        # Quantities are msat; BTC = msat / 1e11.
        self.assertIn("/100000000000", sheets["Acquisitions"])
        # README guidance, run metadata, and the active lot method are surfaced.
        self.assertIn("How to verify this report", shared)
        self.assertIn("Holdings BTC (recompute)", shared)
        self.assertIn("Active lot-selection method", shared)
        self.assertIn("Verification status", shared)
        self.assertIn("Kassiber version", shared)
        self.assertIn("Pricing Source", shared)  # provenance column on the ledgers
        self.assertIn("Rate Source", shared)  # rate provenance on Control
        self.assertIn("ALL CHECKS OK", sheets["Verify"])  # workbook-level status banner
        # Per-transaction context on the value-layer Transactions sheet.
        self.assertIn("Attachments", shared)
        self.assertIn("Tags", shared)
        self.assertIn("Counterparty", shared)
        # The URL attachment added in the attachments lifecycle is surfaced.
        self.assertIn("docs.google.com", shared)
        # Every linked attachment is a clickable styled link on the Evidence sheet.
        self.assertIn("Evidence", payload["data"]["sheets"])
        self.assertIn("Name (link)", shared)
        with zipfile.ZipFile(verify_path) as workbook:
            names = re.findall(r'<sheet name="([^"]+)"', workbook.read("xl/workbook.xml").decode("utf-8"))
            evidence_rels = workbook.read(
                f"xl/worksheets/_rels/sheet{names.index('Evidence') + 1}.xml.rels"
            ).decode("utf-8")
        self.assertIn("docs.google.com", evidence_rels)  # real hyperlink target

        # Cached results must equal Kassiber's numbers: each Disposals gain cell
        # (column J = proceeds - basis) must match the stored engine gain
        # (column K) within a cent. This guards against the formulas drifting
        # from the report figures.
        disposals = sheets["Disposals"]
        formula_gains = {
            int(row): float(val)
            for row, val in re.findall(r'<c r="J(\d+)"[^>]*><f>.*?</f><v>([^<]*)</v>', disposals)
        }
        kassiber_gains = {
            int(row): float(val)
            for row, val in re.findall(r'<c r="K(\d+)"[^>]*><v>([^<]*)</v>', disposals)
        }
        compared = 0
        for row, gain in formula_gains.items():
            if row in kassiber_gains:
                self.assertAlmostEqual(gain, kassiber_gains[row], places=2)
                compared += 1
        self.assertGreater(compared, 0, "expected at least one disposal gain to reconcile")

        # --no-verify produces the lean workbook: no verify sheets, no formulas.
        payload = self._cli(
            "reports", "export-xlsx",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(plain_path),
            "--no-verify",
        )
        self.assertFalse(payload["data"]["verified"])
        self.assertNotIn("Control", payload["data"]["sheets"])
        self.assertNotIn("Verify", payload["data"]["sheets"])
        plain_sheets, _ = _read_workbook(plain_path)
        self.assertNotIn("Control", plain_sheets)
        self.assertFalse(any("<f>" in xml for xml in plain_sheets.values()))

    def test_07ad_verify_xlsx_reconciles_including_income(self):
        # Independent, formula-engine-free reconciliation on a book that
        # includes an income/earn event. Income is emitted by the engine as BOTH
        # an `acquisition` lot AND an `income` line, so a naive Σacq − Σdisp
        # double-counts it. This test evaluates the verification sheets' SUMIFS
        # logic in Python from the sheet inputs and asserts it reproduces
        # Kassiber's portfolio + capital-gains figures.
        with tempfile.TemporaryDirectory(prefix="kassiber-verify-income-") as tmp:
            root = Path(tmp) / "data"
            csv_path = Path(tmp) / "book.csv"
            csv_path.write_text(
                "date,txid,direction,asset,amount,fee,fiat_rate,description,kind\n"
                "2026-01-01T10:00:00Z,buy-1,inbound,BTC,0.10000000,0,60000,Buy,buy\n"
                "2026-02-01T10:00:00Z,int-1,inbound,BTC,0.01000000,0,65000,Interest,interest\n"
                "2026-03-01T10:00:00Z,sell-1,outbound,BTC,0.02000000,0,70000,Sell,sell\n",
                encoding="utf-8",
            )
            xlsx_path = Path(tmp) / "report.xlsx"

            def run(*args):
                payload, code = _run(root, *args)
                self.assertEqual(code, 0, f"{args} -> {json.dumps(payload)[:300]}")
                return payload

            run("init")
            run("workspaces", "create", "Main")
            run("profiles", "create", "--workspace", "Main", "--fiat-currency", "USD", "--tax-country", "generic", "Default")
            run("wallets", "create", "--workspace", "Main", "--profile", "Default", "--label", "W1", "--kind", "custom")
            run("wallets", "import-csv", "--workspace", "Main", "--profile", "Default", "--wallet", "W1", "--file", str(csv_path))
            # A single Google Docs link on the sale -> styled clickable name.
            sale_doc = "https://docs.google.com/document/d/1sAmPleSaLeReceipt/edit"
            run("attachments", "add", "--workspace", "Main", "--profile", "Default",
                "--transaction", "sell-1", "--url", sale_doc, "--label", "Sale receipt")
            run("journals", "process", "--workspace", "Main", "--profile", "Default")
            export = run("reports", "export-xlsx", "--workspace", "Main", "--profile", "Default", "--file", str(xlsx_path))
            self.assertTrue(export["data"]["verified"])
            self.assertIn("Evidence", export["data"]["sheets"])

            portfolio = run("reports", "portfolio-summary", "--workspace", "Main", "--profile", "Default")["data"]
            capital = run("reports", "capital-gains", "--workspace", "Main", "--profile", "Default")["data"]
            kassiber_qty, kassiber_basis, kassiber_realized = {}, {}, {}
            for row in portfolio:
                kassiber_qty[row["asset"]] = kassiber_qty.get(row["asset"], 0.0) + float(row["quantity"])
                kassiber_basis[row["asset"]] = kassiber_basis.get(row["asset"], 0.0) + float(row["cost_basis"])
            for row in capital:
                kassiber_realized[row["asset"]] = kassiber_realized.get(row["asset"], 0.0) + float(row["gain_loss"])
            # Sanity: the book really exercises income (otherwise the guard is vacuous).
            self.assertAlmostEqual(kassiber_qty["BTC"], 0.09, places=8)
            self.assertGreater(kassiber_realized["BTC"], 600.0)  # 200 disposal + 650 income

            sheets = _load_xlsx_sheets(xlsx_path)
            acq = sheets["Acquisitions"]
            disp = sheets["Disposals"]

            # Pin the load-bearing formula detail: the holdings recompute adds
            # only acquisition + transfer_in (excluding the income lines), or
            # recalc silently double-counts earned coins. Plain equality
            # criteria (no "<>income") keep Apple Numbers happy on import.
            with zipfile.ZipFile(xlsx_path) as workbook:
                names = re.findall(r'<sheet name="([^"]+)"', workbook.read("xl/workbook.xml").decode("utf-8"))
                control_xml = _unescape_xml(
                    workbook.read(f"xl/worksheets/sheet{names.index('Control') + 1}.xml").decode("utf-8")
                )
            self.assertIn('"acquisition"', control_xml)
            self.assertIn('"transfer_in"', control_xml)
            self.assertNotIn("<>income", control_xml)

            # Resolve columns by header label (row 2) so the test survives
            # column additions/reordering.
            def _headers(rows):
                return {label: col for col, label in rows.get(2, {}).items()}

            acq_h = _headers(acq)
            disp_h = _headers(disp)

            def _accumulate(rows, headers, key_fn):
                asset_col = headers["Asset"]
                totals = {}
                for rownum, cells in rows.items():
                    if rownum < 3 or asset_col not in cells:  # skip header / placeholder
                        continue
                    asset = cells[asset_col]
                    totals[asset] = totals.get(asset, 0.0) + key_fn(cells)
                return totals

            def _val(cells, headers, label, default=0.0):
                return cells.get(headers[label], default)

            # Holdings exclude `income` rows on the add side (the paired lot carries them).
            qty_add = _accumulate(
                acq, acq_h,
                lambda c: _val(c, acq_h, "Quantity msat (input)") if _val(c, acq_h, "Type", "") != "income" else 0.0,
            )
            qty_sub = _accumulate(disp, disp_h, lambda c: _val(c, disp_h, "Quantity msat (input)"))
            basis_add = _accumulate(
                acq, acq_h,
                lambda c: _val(c, acq_h, "Fiat Value (input)") if _val(c, acq_h, "Type", "") != "income" else 0.0,
            )
            basis_sub = _accumulate(disp, disp_h, lambda c: _val(c, disp_h, "Cost Basis (input)"))
            realized_disp = _accumulate(
                disp, disp_h,
                lambda c: (_val(c, disp_h, "Proceeds (input)") - _val(c, disp_h, "Cost Basis (input)"))
                if _val(c, disp_h, "Taxable") == 1 else 0.0,
            )
            realized_acq = _accumulate(
                acq, acq_h,
                lambda c: _val(c, acq_h, "Income Gain (input)") if _val(c, acq_h, "Taxable") == 1 else 0.0,
            )

            for asset, expected in kassiber_qty.items():
                recompute = (qty_add.get(asset, 0.0) - qty_sub.get(asset, 0.0)) / 1e11
                self.assertAlmostEqual(recompute, expected, places=8, msg=f"holdings qty {asset}")
            for asset, expected in kassiber_basis.items():
                recompute = basis_add.get(asset, 0.0) - basis_sub.get(asset, 0.0)
                self.assertAlmostEqual(recompute, expected, places=2, msg=f"cost basis {asset}")
            for asset, expected in kassiber_realized.items():
                recompute = realized_disp.get(asset, 0.0) + realized_acq.get(asset, 0.0)
                self.assertAlmostEqual(recompute, expected, places=2, msg=f"realized gain {asset}")

            # The Control sheet's cached recompute values reconcile to Kassiber's.
            control = sheets["Control"]
            ctrl_h = _headers(control)
            label_pairs = [
                ("Holdings BTC (recompute)", "Holdings BTC (Kassiber)"),
                ("Cost Basis (recompute)", "Cost Basis (Kassiber)"),
                ("Avg Price (recompute)", "Avg Price (Kassiber)"),
                ("Market Value (recompute)", "Market Value (Kassiber)"),
                ("Unrealized (recompute)", "Unrealized (Kassiber)"),
                ("Realized Gain (recompute)", "Realized Gain (Kassiber)"),
            ]
            asset_col = ctrl_h["Asset"]
            compared = 0
            for rownum, cells in control.items():
                if rownum < 3 or asset_col not in cells:
                    continue
                for recompute_label, kassiber_label in label_pairs:
                    rc, kc = ctrl_h[recompute_label], ctrl_h[kassiber_label]
                    if rc in cells and kc in cells:
                        self.assertAlmostEqual(cells[rc], cells[kc], places=2)
                        compared += 1
            self.assertGreater(compared, 0, "expected Control rows to reconcile")

            # Rate provenance is surfaced on the Control sheet.
            self.assertIn("Rate Source", ctrl_h)
            self.assertIn("Rate As Of", ctrl_h)
            # Description + tags accompany each ledger row.
            for headers in (acq_h, disp_h):
                self.assertIn("Description", headers)
                self.assertIn("Tags", headers)

            # The single Google Docs link is a real hyperlink: shown behind its
            # name on the Transactions sheet and listed on the Evidence sheet.
            with zipfile.ZipFile(xlsx_path) as workbook:
                names = re.findall(r'<sheet name="([^"]+)"', workbook.read("xl/workbook.xml").decode("utf-8"))
                tx_rels = workbook.read(
                    f"xl/worksheets/_rels/sheet{names.index('Transactions') + 1}.xml.rels"
                ).decode("utf-8")
                ev_rels = workbook.read(
                    f"xl/worksheets/_rels/sheet{names.index('Evidence') + 1}.xml.rels"
                ).decode("utf-8")
            self.assertIn(sale_doc, tx_rels)  # clickable link in the Transactions cell
            self.assertIn(sale_doc, ev_rels)  # clickable link on the Evidence sheet
            # The visible cell text is the name, not the raw URL.
            tx = sheets["Transactions"]
            tx_h = {l: c for c, l in tx.get(2, {}).items()}
            att_values = [tx[r].get(tx_h["Attachments"]) for r in tx if r >= 3 and tx_h["Attachments"] in tx[r]]
            self.assertIn("Sale receipt", att_values)
            self.assertNotIn(sale_doc, att_values)  # URL is the link target, not the shown text

    def test_07ae_transactions_export(self):
        xlsx_path = Path(self._tmp.name) / "kassiber-transactions.xlsx"
        csv_path = Path(self._tmp.name) / "kassiber-transactions.csv"
        for path in (xlsx_path, csv_path):
            if path.exists():
                path.unlink()

        payload = self._cli(
            "transactions", "export",
            "--workspace", "Main",
            "--profile", "Default",
            "--export-format", "xlsx",
            "--file", str(xlsx_path),
        )
        self._assert_kind(payload, "transactions.export")
        self.assertEqual(payload["data"]["sheets"], ["Transactions"])
        self.assertGreater(payload["data"]["rows"], 0)
        self.assertEqual(xlsx_path.read_bytes()[:2], b"PK")
        sheets = _load_xlsx_sheets(xlsx_path)
        headers = {label for label in sheets["Transactions"].get(2, {}).values()}
        for column in ("Wallet", "Direction", "Asset", "Description", "Tags", "Attachments"):
            self.assertIn(column, headers)

        payload = self._cli(
            "transactions", "export",
            "--workspace", "Main",
            "--profile", "Default",
            "--export-format", "csv",
            "--file", str(csv_path),
        )
        self._assert_kind(payload, "transactions.export")
        csv_text = csv_path.read_text(encoding="utf-8")
        self.assertIn("Kassiber Transactions - Default", csv_text)
        self.assertIn("Transaction ID", csv_text)

    def test_07af_verify_transfer_fee_and_traceable_ids(self):
        # A self-transfer with a network fee: the engine records transfer_out for
        # the full sent amount (fee included) plus a separate transfer_fee row.
        # The holdings recompute must not subtract the fee twice, and the ledger
        # Transaction IDs must be the external txids (so they match the
        # Transactions sheet for evidence cross-reference).
        with tempfile.TemporaryDirectory(prefix="kassiber-verify-xfer-") as tmp:
            root = Path(tmp) / "data"
            a_csv = Path(tmp) / "a.csv"
            b_csv = Path(tmp) / "b.csv"
            a_csv.write_text(
                "date,txid,direction,asset,amount,fee,fiat_rate,description,kind\n"
                "2024-01-01T00:00:00Z,buy-001,inbound,BTC,1.00000000,0,40000,Buy,buy\n"
                "2024-06-01T00:00:00Z,xfer-001,outbound,BTC,0.50100000,0,50000,Move to cold,transfer\n",
                encoding="utf-8",
            )
            b_csv.write_text(
                "date,txid,direction,asset,amount,fee,fiat_rate,description,kind\n"
                "2024-06-01T00:05:00Z,xfer-001,inbound,BTC,0.50000000,0,50000,Move to cold,transfer\n",
                encoding="utf-8",
            )
            xlsx_path = Path(tmp) / "report.xlsx"

            def run(*args):
                payload, code = _run(root, *args)
                self.assertEqual(code, 0, f"{args} -> {json.dumps(payload)[:300]}")
                return payload

            run("init")
            run("workspaces", "create", "Main")
            run("profiles", "create", "--workspace", "Main", "--fiat-currency", "USD", "--tax-country", "generic", "Default")
            run("wallets", "create", "--workspace", "Main", "--profile", "Default", "--label", "Hot", "--kind", "custom")
            run("wallets", "create", "--workspace", "Main", "--profile", "Default", "--label", "Cold", "--kind", "custom")
            run("wallets", "import-csv", "--workspace", "Main", "--profile", "Default", "--wallet", "Hot", "--file", str(a_csv))
            run("wallets", "import-csv", "--workspace", "Main", "--profile", "Default", "--wallet", "Cold", "--file", str(b_csv))
            run("rates", "set", "BTC-USD", "2025-06-01T00:00:00Z", "90000")
            run("journals", "process", "--workspace", "Main", "--profile", "Default")
            run("reports", "export-xlsx", "--workspace", "Main", "--profile", "Default", "--file", str(xlsx_path))

            portfolio = run("reports", "portfolio-summary", "--workspace", "Main", "--profile", "Default")["data"]
            kassiber_qty = sum(float(row["quantity"]) for row in portfolio if row["asset"] == "BTC")
            self.assertAlmostEqual(kassiber_qty, 0.999, places=8)  # 1.0 funded − 0.001 fee burned

            sheets = _load_xlsx_sheets(xlsx_path)
            acq = sheets["Acquisitions"]
            disp = sheets["Disposals"]
            acq_h = {l: c for c, l in acq.get(2, {}).items()}
            disp_h = {l: c for c, l in disp.get(2, {}).items()}

            def _by_asset(rows, headers, value_label, *, types=None):
                total = 0.0
                for rownum, cells in rows.items():
                    if rownum < 3 or headers["Asset"] not in cells:
                        continue
                    if types is not None and cells.get(headers["Type"]) not in types:
                        continue
                    total += cells.get(headers[value_label], 0.0)
                return total

            qty = "Quantity msat (input)"
            add = _by_asset(acq, acq_h, qty, types={"acquisition", "transfer_in"})
            sub_all = _by_asset(disp, disp_h, qty)
            sub_fee = _by_asset(disp, disp_h, qty, types={"transfer_fee"})
            # Mirror the Control holdings formula: Σadd − (Σdisp − Σtransfer_fee).
            recompute = (add - (sub_all - sub_fee)) / 1e11
            self.assertAlmostEqual(recompute, kassiber_qty, places=8)

            # The live formula must re-add transfer_fee and use a BTC tolerance.
            with zipfile.ZipFile(xlsx_path) as workbook:
                names = re.findall(r'<sheet name="([^"]+)"', workbook.read("xl/workbook.xml").decode("utf-8"))
                control_xml = _unescape_xml(
                    workbook.read(f"xl/worksheets/sheet{names.index('Control') + 1}.xml").decode("utf-8")
                )
            self.assertIn('"transfer_fee"', control_xml)
            self.assertIn("0.00000001", control_xml)  # balance check uses a BTC tolerance, not the fiat cell

            # Ledger Transaction IDs are the external txids, matching Transactions.
            acq_ids = {acq[r].get(acq_h["Transaction ID"]) for r in acq if r >= 3 and acq_h["Transaction ID"] in acq[r]}
            self.assertIn("buy-001", acq_ids)
            self.assertIn("xfer-001", acq_ids)

    def test_07aa_pdf_writer_reports_actual_page_count(self):
        from kassiber.pdf_report import write_text_pdf

        pdf_path = Path(self._tmp.name) / "kassiber-report-multipage.pdf"
        lines = ["Synthetic Report", "================", ""]
        for section in range(10):
            lines.extend(["", f"Section {section}", "-----------------"])
            lines.append("Date        Wallet          Dir  Asset        Amount           Fee  Description")
            lines.append(
                "----------  --------------  ---  ------  ------------  ------------  ----------------------------"
            )
            for index in range(45):
                lines.append(
                    f"2025-01-{(index % 28) + 1:02d}  Wallet-{section:02d}      out  BTC      "
                    f"{index * 0.12345678:,.8f}    0.00001000  Example row {index}"
                )

        result = write_text_pdf(str(pdf_path), "Synthetic Report", lines)
        payload_bytes = pdf_path.read_bytes()
        actual_pages = len(re.findall(rb"/Type /Page\b", payload_bytes))

        self.assertTrue(pdf_path.exists())
        self.assertGreater(result["pages"], 1)
        self.assertEqual(result["pages"], actual_pages)

    def test_08_capital_gains_msat_and_counts(self):
        payload = self._cli(
            "reports", "capital-gains",
            "--workspace", "Main",
            "--profile", "Default",
        )
        rows = payload["data"]
        self.assertIsInstance(rows, list)
        self.assertEqual(len(rows), 2)
        for row in rows:
            self.assertIn("quantity", row)
            self.assertIn("quantity_msat", row)
            self.assertIsInstance(row["quantity_msat"], int)
            self.assertEqual(row["entry_type"], "disposal")

    def test_09_balance_sheet_totals(self):
        payload = self._cli(
            "reports", "balance-sheet",
            "--workspace", "Main",
            "--profile", "Default",
        )
        rows = payload["data"]
        btc_rows = [r for r in rows if r.get("asset") == "BTC"]
        self.assertEqual(len(btc_rows), 1)
        # Sample math: +0.05 swap_in + 0.00003 ln_received
        #              -(0.00005 + 0.0000005) ln_sent
        #              -(0.005 + 0.000015) channel_close
        # = 0.0449645 BTC
        self.assertAlmostEqual(float(btc_rows[0]["quantity"]), 0.0449645, places=7)

    def test_10_rates_manual_roundtrip(self):
        payload = self._cli("rates", "pairs")
        self._assert_kind(payload, "rates.pairs")
        pairs = {p["pair"] for p in payload["data"]}
        self.assertIn("BTC-USD", pairs)
        self.assertIn("BTC-EUR", pairs)

        payload = self._cli(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "65000",
        )
        self._assert_kind(payload, "rates.set")

        payload = self._cli("rates", "latest", "BTC-USD")
        self._assert_kind(payload, "rates.latest")
        self.assertAlmostEqual(float(payload["data"]["rate"]), 65000.0, places=4)

        payload = self._cli(
            "rates", "range", "BTC-USD",
            "--start", "2024-04-01T00:00:00Z",
        )
        self._assert_kind(payload, "rates.range")
        samples = payload["data"]
        self.assertIsInstance(samples, list)
        self.assertEqual(len(samples), 1)
        self.assertEqual(samples[0]["source"], "manual")

    def test_11_rates_cache_autopricing(self):
        payload = self._cli(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "CachePriced",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "CachePriced",
            "--file", str(self.cache_pricing_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli(
            "rates", "set", "BTC-USD", "2024-05-09T00:00:00Z", "61000",
        )
        self._assert_kind(payload, "rates.set")

        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_kind(payload, "journals.process")
        data = payload["data"]
        self.assertEqual(data["entries_created"], 5)
        self.assertEqual(data["quarantined"], 0)
        self.assertEqual(data["auto_priced"], 1)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "CachePriced",
        )
        self._assert_kind(payload, "transactions.list")
        record = payload["data"][0]
        self.assertAlmostEqual(float(record["fiat_rate"]), 61000.0, places=4)
        self.assertAlmostEqual(float(record["fiat_value"]), 610.0, places=4)

    def test_11a_rates_cache_prefers_confirmed_at_when_present(self):
        workspace = "ConfirmedPricing"
        profile = "ConfirmedPricingDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "ConfirmedPriced",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedPriced",
            "--file", str(self.confirmed_pricing_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        self._cli("rates", "set", "BTC-USD", "2024-05-09T00:00:00Z", "60000")
        self._cli("rates", "set", "BTC-USD", "2024-05-10T00:00:00Z", "62000")

        payload = self._cli(
            "journals", "process",
            "--workspace", workspace,
            "--profile", profile,
        )
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 1)

        payload = self._cli(
            "transactions", "list",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedPriced",
        )
        self._assert_kind(payload, "transactions.list")
        record = payload["data"][0]
        self.assertEqual(record["confirmed_at"], "2024-05-10T12:00:00Z")
        self.assertAlmostEqual(float(record["fiat_rate"]), 62000.0, places=4)
        self.assertAlmostEqual(float(record["fiat_value"]), 620.0, places=4)

    def test_11aa_lbtc_pricing_uses_btc_rate_at_confirmed_at(self):
        workspace = "LiquidConfirmedPricing"
        profile = "LiquidConfirmedPricingDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "LiquidConfirmedPriced",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "LiquidConfirmedPriced",
            "--file", str(self.lbtc_confirmed_pricing_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        self._cli("rates", "set", "BTC-USD", "2024-05-09T00:00:00Z", "60000")
        self._cli("rates", "set", "BTC-USD", "2024-05-10T00:00:00Z", "62000")

        payload = self._cli(
            "journals", "process",
            "--workspace", workspace,
            "--profile", profile,
        )
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 1)

        payload = self._cli(
            "transactions", "list",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "LiquidConfirmedPriced",
        )
        self._assert_kind(payload, "transactions.list")
        record = payload["data"][0]
        self.assertEqual(record["asset"], "LBTC")
        self.assertEqual(record["confirmed_at"], "2024-05-10T12:00:00Z")
        self.assertAlmostEqual(float(record["fiat_rate"]), 62000.0, places=4)
        self.assertAlmostEqual(float(record["fiat_value"]), 620.0, places=4)

    def test_11b_repeat_import_merges_confirmed_at_without_duplicate(self):
        workspace = "ConfirmedMergeSpace"
        profile = "ConfirmedMergeDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "ConfirmedMerge",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        first_csv = Path(self._tmp.name) / "confirmed-merge-first.csv"
        first_csv.write_text(
            "date,txid,direction,asset,amount,fee,description\n"
            "2024-05-10T12:00:00Z,confirmed-merge-1,inbound,BTC,0.01000000,0,First import\n",
            encoding="utf-8",
        )
        second_csv = Path(self._tmp.name) / "confirmed-merge-second.csv"
        second_csv.write_text(
            "date,confirmed_at,txid,direction,asset,amount,fee,description\n"
            "2024-05-10T12:00:00Z,2024-05-10T12:00:00Z,confirmed-merge-1,inbound,BTC,0.01000000,0,Second import\n",
            encoding="utf-8",
        )

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedMerge",
            "--file", str(first_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedMerge",
            "--file", str(second_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["skipped"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        count = conn.execute(
            "SELECT COUNT(*) AS n FROM transactions WHERE wallet_id = (SELECT id FROM wallets WHERE label = 'ConfirmedMerge')"
        ).fetchone()
        record = conn.execute(
            "SELECT occurred_at, confirmed_at FROM transactions WHERE external_id = 'confirmed-merge-1'"
        ).fetchone()
        conn.close()

        self.assertEqual(count["n"], 1)
        self.assertEqual(record["occurred_at"], "2024-05-10T12:00:00Z")
        self.assertEqual(record["confirmed_at"], "2024-05-10T12:00:00Z")

    def test_11c_repeat_import_replaces_unknown_occurred_at_without_duplicate(self):
        workspace = "ConfirmedShiftSpace"
        profile = "ConfirmedShiftDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "ConfirmedShift",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        first_csv = Path(self._tmp.name) / "confirmed-shift-first.csv"
        first_csv.write_text(
            "date,txid,direction,asset,amount,fee,description\n"
            "1970-01-01T00:00:00Z,confirmed-shift-1,inbound,BTC,0.01000000,0,First sync placeholder\n",
            encoding="utf-8",
        )
        second_csv = Path(self._tmp.name) / "confirmed-shift-second.csv"
        second_csv.write_text(
            "date,confirmed_at,txid,direction,asset,amount,fee,description\n"
            "2024-05-10T12:00:00Z,2024-05-10T12:00:00Z,confirmed-shift-1,inbound,BTC,0.01000000,0,Confirmed sync\n",
            encoding="utf-8",
        )

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedShift",
            "--file", str(first_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedShift",
            "--file", str(second_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["skipped"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        count = conn.execute(
            "SELECT COUNT(*) AS n FROM transactions WHERE wallet_id = (SELECT id FROM wallets WHERE label = 'ConfirmedShift')"
        ).fetchone()
        record = conn.execute(
            "SELECT occurred_at, confirmed_at FROM transactions WHERE external_id = 'confirmed-shift-1'"
        ).fetchone()
        conn.close()

        self.assertEqual(count["n"], 1)
        self.assertEqual(record["occurred_at"], "2024-05-10T12:00:00Z")
        self.assertEqual(record["confirmed_at"], "2024-05-10T12:00:00Z")

    def test_11d_confirmed_at_merge_reprices_cache_derived_values(self):
        workspace = "ConfirmedRepriceSpace"
        profile = "ConfirmedRepriceDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "ConfirmedReprice",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        first_csv = Path(self._tmp.name) / "confirmed-reprice-first.csv"
        first_csv.write_text(
            "date,txid,direction,asset,amount,fee,description\n"
            "2024-05-09T09:00:00Z,confirmed-reprice-1,inbound,BTC,0.01000000,0,First unconfirmed copy\n",
            encoding="utf-8",
        )
        second_csv = Path(self._tmp.name) / "confirmed-reprice-second.csv"
        second_csv.write_text(
            "date,confirmed_at,txid,direction,asset,amount,fee,description\n"
            "2024-05-09T09:00:00Z,2024-05-10T12:00:00Z,confirmed-reprice-1,inbound,BTC,0.01000000,0,Confirmed copy\n",
            encoding="utf-8",
        )

        self._cli("rates", "set", "BTC-USD", "2024-05-09T00:00:00Z", "60000")
        self._cli("rates", "set", "BTC-USD", "2024-05-10T00:00:00Z", "62000")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedReprice",
            "--file", str(first_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", profile)
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT fiat_rate, fiat_value, fiat_price_source FROM transactions WHERE external_id = 'confirmed-reprice-1'"
        ).fetchone()
        conn.close()
        self.assertAlmostEqual(row["fiat_rate"], 60000.0, places=4)
        self.assertAlmostEqual(row["fiat_value"], 600.0, places=4)
        self.assertEqual(row["fiat_price_source"], "rates_cache")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedReprice",
            "--file", str(second_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["skipped"], 1)

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", profile)
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 1)

        payload = self._cli(
            "transactions",
            "list",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedReprice",
        )
        self._assert_kind(payload, "transactions.list")
        record = payload["data"][0]
        self.assertEqual(record["confirmed_at"], "2024-05-10T12:00:00Z")
        self.assertAlmostEqual(float(record["fiat_rate"]), 62000.0, places=4)
        self.assertAlmostEqual(float(record["fiat_value"]), 620.0, places=4)

    def test_11e_repeat_import_does_not_desync_fingerprint(self):
        workspace = "FingerprintMergeSpace"
        profile = "FingerprintMergeDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "FingerprintMerge",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        first_csv = Path(self._tmp.name) / "fingerprint-merge-first.csv"
        first_csv.write_text(
            "date,txid,direction,asset,amount,fee,description\n"
            "2024-05-09T09:00:00Z,fingerprint-merge-1,inbound,BTC,0.01000000,0,First copy\n",
            encoding="utf-8",
        )
        second_csv = Path(self._tmp.name) / "fingerprint-merge-second.csv"
        second_csv.write_text(
            "date,txid,direction,asset,amount,fee,description\n"
            "2024-05-10T09:00:00Z,fingerprint-merge-1,inbound,BTC,0.01000000,0,Conflicting timestamp copy\n",
            encoding="utf-8",
        )

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "FingerprintMerge",
            "--file", str(first_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        before = conn.execute(
            "SELECT occurred_at, fingerprint FROM transactions WHERE external_id = 'fingerprint-merge-1'"
        ).fetchone()
        conn.close()

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "FingerprintMerge",
            "--file", str(second_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["skipped"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        count = conn.execute(
            "SELECT COUNT(*) AS n FROM transactions WHERE external_id = 'fingerprint-merge-1'"
        ).fetchone()
        after = conn.execute(
            "SELECT occurred_at, fingerprint FROM transactions WHERE external_id = 'fingerprint-merge-1'"
        ).fetchone()
        conn.close()

        self.assertEqual(count["n"], 1)
        self.assertEqual(after["occurred_at"], before["occurred_at"])
        self.assertEqual(after["fingerprint"], before["fingerprint"])

    def test_11f_confirmed_at_merge_preserves_imported_price(self):
        workspace = "ConfirmedImportedPriceSpace"
        profile = "ConfirmedImportedPriceDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "ConfirmedImportedPrice",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        first_csv = Path(self._tmp.name) / "confirmed-imported-price-first.csv"
        first_csv.write_text(
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2024-05-09T09:00:00Z,confirmed-imported-price-1,inbound,BTC,0.01000000,0,60000,Imported price\n",
            encoding="utf-8",
        )
        second_csv = Path(self._tmp.name) / "confirmed-imported-price-second.csv"
        second_csv.write_text(
            "date,confirmed_at,txid,direction,asset,amount,fee,description\n"
            "2024-05-09T09:00:00Z,2024-05-10T12:00:00Z,confirmed-imported-price-1,inbound,BTC,0.01000000,0,Confirmed copy\n",
            encoding="utf-8",
        )

        self._cli("rates", "set", "BTC-USD", "2024-05-10T00:00:00Z", "62000")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedImportedPrice",
            "--file", str(first_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", profile)
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 0)

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedImportedPrice",
            "--file", str(second_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["skipped"], 1)

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", profile)
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 0)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            """
            SELECT confirmed_at, fiat_rate, fiat_value, fiat_price_source
            FROM transactions
            WHERE external_id = 'confirmed-imported-price-1'
            """
        ).fetchone()
        conn.close()

        self.assertEqual(row["confirmed_at"], "2024-05-10T12:00:00Z")
        self.assertAlmostEqual(row["fiat_rate"], 60000.0, places=4)
        self.assertAlmostEqual(row["fiat_value"], 600.0, places=4)
        self.assertEqual(row["fiat_price_source"], "import")

    def test_12_error_envelope_shape(self):
        # bad pair syntax (no hyphen) → validation error envelope
        payload, code = _run(
            self.data_root,
            "rates", "set", "BTCUSD", "2024-05-01T00:00:00Z", "65000",
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload.get("schema_version"), 1)
        err = payload.get("error")
        self.assertIsInstance(err, dict)
        for field in ("code", "message", "hint", "details", "retryable"):
            self.assertIn(field, err)
        self.assertEqual(err["code"], "validation")

    def test_13_cross_wallet_intra_transfer(self):
        # New profile so the assertions don't tangle with prior tests.
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "Transfer",
        )
        self._assert_kind(payload, "profiles.create")

        for label in ("Cold", "Hot"):
            payload = self._cli(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "Transfer",
                "--label", label,
                "--kind", "custom",
            )
            self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Transfer",
            "--wallet", "Cold",
            "--file", str(self.cold_transfer_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 2)

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Transfer",
            "--wallet", "Hot",
            "--file", str(self.hot_transfer_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        self._assert_kind(payload, "journals.process")
        data = payload["data"]
        # 1 acquisition (cold inbound) + 1 transfer_fee + 1 transfer_out + 1 transfer_in
        self.assertEqual(data["transfers_detected"], 1)
        self.assertEqual(data["entries_created"], 4)
        self.assertEqual(data["quarantined"], 0)
        self.assertEqual(data["processed_transactions"], 3)

        payload = self._cli(
            "reports", "journal-entries",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        self._assert_kind(payload, "reports.journal-entries")
        entries = payload["data"]
        types = sorted(e["entry_type"] for e in entries)
        self.assertEqual(types, ["acquisition", "transfer_fee", "transfer_in", "transfer_out"])

        # The transfer_out / transfer_in pair must zero out across wallets.
        out_entry = next(e for e in entries if e["entry_type"] == "transfer_out")
        in_entry = next(e for e in entries if e["entry_type"] == "transfer_in")
        self.assertEqual(out_entry["wallet"], "Cold")
        self.assertEqual(in_entry["wallet"], "Hot")
        self.assertAlmostEqual(float(out_entry["quantity"]), -0.501, places=8)
        self.assertAlmostEqual(float(in_entry["quantity"]), 0.5, places=8)

        payload = self._cli(
            "journals", "transfers", "list",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        self._assert_kind(payload, "journals.transfers.list")
        audit = payload["data"]
        self.assertEqual(audit["summary"]["same_asset_transfers"], 1)
        self.assertEqual(audit["summary"]["cross_asset_pairs"], 0)
        transfer_row = audit["same_asset_transfers"][0]
        self.assertEqual(transfer_row["from_wallet"], "Cold")
        self.assertEqual(transfer_row["to_wallet"], "Hot")
        self.assertEqual(transfer_row["sent_msat"], 50100000000)
        self.assertEqual(transfer_row["received_msat"], 50000000000)
        self.assertEqual(transfer_row["fee_msat"], 100000000)

        # Network fees are recorded for holdings/audit, but they are not
        # capital-gains disposals.
        payload = self._cli(
            "reports", "capital-gains",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        self.assertEqual(payload["data"], [])

        # Cost basis follows the moved coins to Hot, so both wallets show non-zero
        # holdings with positive average cost.
        payload = self._cli(
            "reports", "portfolio-summary",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        rows = {r["wallet"]: r for r in payload["data"]}
        self.assertEqual(set(rows), {"Cold", "Hot"})
        self.assertAlmostEqual(float(rows["Cold"]["quantity"]), 0.499, places=8)
        self.assertAlmostEqual(float(rows["Hot"]["quantity"]), 0.5, places=8)
        # Average cost is global ($59,940 / 0.999 BTC = $60,000) since the only
        # acquisition was at $60k.
        self.assertAlmostEqual(float(rows["Cold"]["avg_cost"]), 60000.0, places=2)
        self.assertAlmostEqual(float(rows["Hot"]["avg_cost"]), 60000.0, places=2)

        # Aggregate BTC across both wallets: 0.499 + 0.5 = 0.999 BTC.
        payload = self._cli(
            "reports", "balance-sheet",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        btc_rows = [r for r in payload["data"] if r.get("asset") == "BTC"]
        total_qty = sum(float(r["quantity"]) for r in btc_rows)
        self.assertAlmostEqual(total_qty, 0.999, places=8)

        payload = self._cli(
            "reports", "summary",
            "--workspace", "Main",
            "--profile", "Transfer",
            "--wallet", "Hot",
        )
        self._assert_kind(payload, "reports.summary")
        summary = payload["data"]
        self.assertEqual(summary["wallet"], "Hot")
        self.assertEqual(summary["metrics"]["wallets_in_scope"], 1)
        self.assertEqual(summary["metrics"]["active_transactions"], 1)
        self.assertEqual(summary["asset_flow"][0]["fee_amount_msat"], 0)

        payload = self._cli(
            "reports", "tax-summary",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        self._assert_kind(payload, "reports.tax-summary")
        rows = payload["data"]
        detail_rows = [row for row in rows if row["row_type"] == "detail"]
        self.assertEqual(detail_rows, [])

    def test_13a_intra_transfer_fiat_value_spot_price(self):
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "TransferValueOnly",
        )
        self._assert_kind(payload, "profiles.create")

        for label in ("ColdValue", "HotValue"):
            payload = self._cli(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "TransferValueOnly",
                "--label", label,
                "--kind", "custom",
            )
            self._assert_kind(payload, "wallets.create")

        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "TransferValueOnly",
            "--wallet", "ColdValue",
            "--file", str(self.cold_transfer_value_only_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "TransferValueOnly",
            "--wallet", "HotValue",
            "--file", str(self.hot_transfer_value_only_csv),
        )

        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "TransferValueOnly",
        )
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["transfers_detected"], 1)

        payload = self._cli(
            "reports", "capital-gains",
            "--workspace", "Main",
            "--profile", "TransferValueOnly",
        )
        self._assert_kind(payload, "reports.capital-gains")
        rows = payload["data"]
        self.assertEqual(rows, [])

    def test_13c_fee_only_consolidation_is_reported_as_fee(self):
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "FeeOnly",
        )
        self._assert_kind(payload, "profiles.create")

        payload = self._cli(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "FeeOnly",
            "--label", "Wallet",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "FeeOnly",
            "--wallet", "Wallet",
            "--file", str(self.fee_only_consolidation_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 2)

        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "FeeOnly",
        )
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["entries_created"], 2)
        self.assertEqual(payload["data"]["transfers_detected"], 0)
        self.assertEqual(payload["data"]["quarantined"], 0)

        payload = self._cli(
            "reports", "journal-entries",
            "--workspace", "Main",
            "--profile", "FeeOnly",
        )
        self._assert_kind(payload, "reports.journal-entries")
        entries = payload["data"]
        self.assertEqual(sorted(e["entry_type"] for e in entries), ["acquisition", "fee"])
        fee_entry = next(e for e in entries if e["entry_type"] == "fee")
        self.assertEqual(fee_entry["wallet"], "Wallet")
        self.assertAlmostEqual(float(fee_entry["quantity"]), -0.001, places=8)
        self.assertAlmostEqual(float(fee_entry["proceeds"]), 60.0, places=4)
        self.assertAlmostEqual(float(fee_entry["cost_basis"]), 60.0, places=4)
        self.assertAlmostEqual(float(fee_entry["gain_loss"]), 0.0, places=4)

        payload = self._cli(
            "reports", "summary",
            "--workspace", "Main",
            "--profile", "FeeOnly",
        )
        self._assert_kind(payload, "reports.summary")
        flow = payload["data"]["asset_flow"][0]
        self.assertEqual(flow["outbound_amount_msat"], 0)
        self.assertEqual(flow["fee_amount_msat"], 100000000)
        self.assertEqual(payload["data"]["realized"]["gain_loss"], 0)

        payload = self._cli(
            "reports", "tax-summary",
            "--workspace", "Main",
            "--profile", "FeeOnly",
        )
        self._assert_kind(payload, "reports.tax-summary")
        detail_rows = [row for row in payload["data"] if row["row_type"] == "detail"]
        self.assertEqual(detail_rows, [])

    def test_13d_split_peg_implausible_fee_is_quarantined_not_taxed(self):
        # A spend that fans out to an owned wallet + a Liquid peg must NOT be
        # booked as a self-transfer whose ~0.0195 BTC peg residual is taxed as a
        # network fee. The implausible-fee guard quarantines it for review while
        # the rest of the report (the funding acquisition) still computes.
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "SplitPeg",
        )
        self._assert_kind(payload, "profiles.create")
        for label in ("Cold", "Hot"):
            payload = self._cli(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "SplitPeg",
                "--label", label,
                "--kind", "custom",
            )
            self._assert_kind(payload, "wallets.create")
        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", "Main", "--profile", "SplitPeg",
            "--wallet", "Cold", "--file", str(self.split_peg_cold_csv),
        )
        self.assertEqual(payload["data"]["imported"], 2)
        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", "Main", "--profile", "SplitPeg",
            "--wallet", "Hot", "--file", str(self.split_peg_hot_csv),
        )
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli(
            "journals", "process",
            "--workspace", "Main", "--profile", "SplitPeg",
        )
        self._assert_kind(payload, "journals.process")
        data = payload["data"]
        # The bad pair is quarantined, not booked as a transfer; only the
        # funding acquisition survives as an entry.
        self.assertEqual(data["transfers_detected"], 0)
        self.assertGreaterEqual(data["quarantined"], 1)
        self.assertEqual(data["entries_created"], 1)

        # No fee/transfer disposal was created for the pegged residual.
        payload = self._cli(
            "reports", "journal-entries",
            "--workspace", "Main", "--profile", "SplitPeg",
        )
        entry_types = {e["entry_type"] for e in payload["data"]}
        self.assertEqual(entry_types, {"acquisition"})

        # The quarantine names the implausible-fee reason and the right legs.
        payload = self._cli(
            "journals", "quarantined",
            "--workspace", "Main", "--profile", "SplitPeg",
        )
        reasons = [q["reason"] for q in payload["data"]]
        self.assertIn("transfer_fee_implausible", reasons)
        flagged = next(
            q for q in payload["data"] if q["reason"] == "transfer_fee_implausible"
        )
        self.assertAlmostEqual(flagged["detail"]["implied_fee"], 0.01952253, places=8)
        self.assertGreater(
            flagged["detail"]["implied_fee"], flagged["detail"]["fee_ceiling"]
        )

    def test_13e_per_account_oversell_quarantined_not_crashed(self):
        # An account that sells before its funding transfer arrives is quarantined
        # per-account (insufficient_lots), NOT a whole-report crash the way the old
        # global-pool gate caused once rp2's per-account BalanceSet rejected it.
        self._cli("profiles", "create", "--workspace", "Main",
                  "--fiat-currency", "USD", "--tax-country", "generic", "Oversell")
        for label in ("Source", "Onchain"):
            self._cli("wallets", "create", "--workspace", "Main",
                      "--profile", "Oversell", "--label", label, "--kind", "custom")
        self._cli("wallets", "import-csv", "--workspace", "Main", "--profile", "Oversell",
                  "--wallet", "Source", "--file", str(self.oversell_source_csv))
        self._cli("wallets", "import-csv", "--workspace", "Main", "--profile", "Oversell",
                  "--wallet", "Onchain", "--file", str(self.oversell_onchain_csv))
        payload = self._cli("journals", "process", "--workspace", "Main", "--profile", "Oversell")
        # Success envelope, not an app_error — the report computes.
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["transfers_detected"], 1)
        self.assertGreaterEqual(payload["data"]["quarantined"], 1)
        payload = self._cli("journals", "quarantined", "--workspace", "Main", "--profile", "Oversell")
        flagged = [q for q in payload["data"] if q["reason"] == "insufficient_lots"]
        self.assertEqual(len(flagged), 1)
        self.assertEqual(flagged[0]["wallet"], "Onchain")

    def test_13f_same_timestamp_buy_funds_sell(self):
        # A buy and sell at the same timestamp in one wallet: the buy must fund
        # the sell (IN ordered before OUT at equal timestamp), so neither drops.
        self._cli("profiles", "create", "--workspace", "Main",
                  "--fiat-currency", "USD", "--tax-country", "generic", "SameTs")
        self._cli("wallets", "create", "--workspace", "Main",
                  "--profile", "SameTs", "--label", "W", "--kind", "custom")
        self._cli("wallets", "import-csv", "--workspace", "Main", "--profile", "SameTs",
                  "--wallet", "W", "--file", str(self.samets_csv))
        payload = self._cli("journals", "process", "--workspace", "Main", "--profile", "SameTs")
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["quarantined"], 0)
        payload = self._cli("reports", "capital-gains", "--workspace", "Main", "--profile", "SameTs")
        rows = payload["data"]
        self.assertEqual(len(rows), 1)
        self.assertAlmostEqual(float(rows[0]["gain_loss"]), 500.0, places=2)

    def test_13g_gift_disposal_quarantined_not_taxed(self):
        # kind=gift is a disposition but not a market sale; it must be quarantined
        # rather than booked as a full-market-value SELL.
        self._cli("profiles", "create", "--workspace", "Main",
                  "--fiat-currency", "USD", "--tax-country", "generic", "Gift")
        self._cli("wallets", "create", "--workspace", "Main",
                  "--profile", "Gift", "--label", "W", "--kind", "custom")
        self._cli("wallets", "import-csv", "--workspace", "Main", "--profile", "Gift",
                  "--wallet", "W", "--file", str(self.gift_csv))
        payload = self._cli("journals", "process", "--workspace", "Main", "--profile", "Gift")
        self._assert_kind(payload, "journals.process")
        self.assertGreaterEqual(payload["data"]["quarantined"], 1)
        payload = self._cli("reports", "capital-gains", "--workspace", "Main", "--profile", "Gift")
        self.assertEqual(payload["data"], [])
        payload = self._cli("journals", "quarantined", "--workspace", "Main", "--profile", "Gift")
        self.assertIn("non_sale_disposal_kind", [q["reason"] for q in payload["data"]])

    def test_13h_unclassified_income_kind_quarantined(self):
        # kind=reward looks like income but isn't a recognized earn type; it must
        # be quarantined for classification, not silently booked as a plain buy.
        self._cli("profiles", "create", "--workspace", "Main",
                  "--fiat-currency", "USD", "--tax-country", "generic", "Reward")
        self._cli("wallets", "create", "--workspace", "Main",
                  "--profile", "Reward", "--label", "W", "--kind", "custom")
        self._cli("wallets", "import-csv", "--workspace", "Main", "--profile", "Reward",
                  "--wallet", "W", "--file", str(self.reward_csv))
        payload = self._cli("journals", "process", "--workspace", "Main", "--profile", "Reward")
        self._assert_kind(payload, "journals.process")
        self.assertGreaterEqual(payload["data"]["quarantined"], 1)
        payload = self._cli("journals", "quarantined", "--workspace", "Main", "--profile", "Reward")
        self.assertIn("unclassified_income_kind", [q["reason"] for q in payload["data"]])

    def test_13i_dropped_acquisition_starves_later_disposal_basis(self):
        # An early acquisition dropped for coarse pricing contaminates the FIFO:
        # a later sell that is funded per-account but consumes past the priced
        # pre-drop supply must be quarantined, not silently re-based onto a wrong
        # (later) lot.
        self._cli("profiles", "create", "--workspace", "Main",
                  "--fiat-currency", "USD", "--tax-country", "generic", "Basis")
        # This scenario depends on the coarse acquisition being held for review
        # (dropping it from the FIFO), so opt into coarse review for this book.
        self._cli("profiles", "set", "--workspace", "Main", "--profile", "Basis",
                  "--require-coarse-review")
        self._cli("wallets", "create", "--workspace", "Main",
                  "--profile", "Basis", "--label", "W", "--kind", "custom")
        self._cli("wallets", "import-csv", "--workspace", "Main", "--profile", "Basis",
                  "--wallet", "W", "--file", str(self.basis_provenance_csv))
        payload = self._cli("journals", "process", "--workspace", "Main", "--profile", "Basis")
        self._assert_kind(payload, "journals.process")
        payload = self._cli("journals", "quarantined", "--workspace", "Main", "--profile", "Basis")
        reasons = [q["reason"] for q in payload["data"]]
        self.assertIn("pricing_review_required", reasons)  # the coarse acquisition
        self.assertIn("basis_provenance_incomplete", reasons)  # the starved sell
        # The sell did NOT produce a (mis-based) realized gain.
        payload = self._cli("reports", "capital-gains", "--workspace", "Main", "--profile", "Basis")
        self.assertEqual(payload["data"], [])

    def test_13j_unclassified_income_marks_basis_provenance(self):
        # An unclassified income lot dropped before a later sale leaves the FIFO
        # incomplete, so the sale is flagged basis_provenance_incomplete too.
        self._cli("profiles", "create", "--workspace", "Main",
                  "--fiat-currency", "USD", "--tax-country", "generic", "IncomeProv")
        self._cli("wallets", "create", "--workspace", "Main",
                  "--profile", "IncomeProv", "--label", "W", "--kind", "custom")
        self._cli("wallets", "import-csv", "--workspace", "Main", "--profile", "IncomeProv",
                  "--wallet", "W", "--file", str(self.income_provenance_csv))
        payload = self._cli("journals", "process", "--workspace", "Main", "--profile", "IncomeProv")
        self._assert_kind(payload, "journals.process")
        payload = self._cli("journals", "quarantined", "--workspace", "Main", "--profile", "IncomeProv")
        reasons = [q["reason"] for q in payload["data"]]
        self.assertIn("unclassified_income_kind", reasons)
        self.assertIn("basis_provenance_incomplete", reasons)

    def test_13k_quarantined_gift_contaminates_later_disposal(self):
        # A quarantined gift isn't booked into RP2's lots, so a later sale would
        # draw from a lot that should have been consumed. The gift's timestamp
        # contaminates provenance, so the sale is quarantined too (rather than
        # booked against a wrong basis) until the gift is resolved.
        self._cli("profiles", "create", "--workspace", "Main",
                  "--fiat-currency", "USD", "--tax-country", "generic", "GiftDebit")
        self._cli("wallets", "create", "--workspace", "Main",
                  "--profile", "GiftDebit", "--label", "W", "--kind", "custom")
        self._cli("wallets", "import-csv", "--workspace", "Main", "--profile", "GiftDebit",
                  "--wallet", "W", "--file", str(self.gift_debit_csv))
        payload = self._cli("journals", "process", "--workspace", "Main", "--profile", "GiftDebit")
        self._assert_kind(payload, "journals.process")
        payload = self._cli("journals", "quarantined", "--workspace", "Main", "--profile", "GiftDebit")
        reasons = [q["reason"] for q in payload["data"]]
        self.assertIn("non_sale_disposal_kind", reasons)  # the gift
        self.assertIn("basis_provenance_incomplete", reasons)  # the later sale
        # No realized gain booked: the gift is deferred and the sale is gated.
        payload = self._cli("reports", "capital-gains", "--workspace", "Main", "--profile", "GiftDebit")
        self.assertEqual(payload["data"], [])

    def test_13b_pair_by_shared_external_id(self):
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "SharedTxid",
        )
        self._assert_kind(payload, "profiles.create")

        for label in ("ColdShared", "HotShared"):
            payload = self._cli(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "SharedTxid",
                "--label", label,
                "--kind", "custom",
            )
            self._assert_kind(payload, "wallets.create")

        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "SharedTxid",
            "--wallet", "ColdShared",
            "--file", str(self.cold_transfer_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "SharedTxid",
            "--wallet", "HotShared",
            "--file", str(self.hot_transfer_csv),
        )

        payload = self._cli(
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "SharedTxid",
            "--tx-out", "onchain-self-transfer-1",
            "--tx-in", "onchain-self-transfer-1",
            "--policy", "carrying-value",
        )
        self._assert_kind(payload, "transfers.pair")
        pair_id = payload["data"]["id"]
        self.assertNotEqual(payload["data"]["out_transaction_id"], payload["data"]["in_transaction_id"])

        payload = self._cli(
            "transfers", "list",
            "--workspace", "Main",
            "--profile", "SharedTxid",
        )
        self._assert_kind(payload, "transfers.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["id"], pair_id)
        self.assertEqual(payload["data"][0]["out"]["wallet"], "ColdShared")
        self.assertEqual(payload["data"][0]["in"]["wallet"], "HotShared")

    def test_14_manual_same_asset_pairing(self):
        # Auto-detection only fires when external_ids match. The two BTC legs
        # below deliberately have different external_ids; the user pairs them
        # explicitly so the journal pipeline still treats them as an
        # IntraTransaction.
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "ManualPair",
        )
        self._assert_kind(payload, "profiles.create")
        for label in ("From", "To"):
            self._cli(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "ManualPair",
                "--label", label,
                "--kind", "custom",
            )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--wallet", "From",
            "--file", str(self.manual_from_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--wallet", "To",
            "--file", str(self.manual_to_csv),
        )

        # Without a pair, processing books the outbound as a real disposal.
        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "ManualPair",
        )
        self.assertEqual(payload["data"]["transfers_detected"], 0)
        self.assertEqual(payload["data"]["cross_asset_pairs"], 0)

        payload, code = _run(
            self.data_root,
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--tx-out", "manual-out-leg",
            "--tx-in", "manual-in-leg",
            "--kind", "manual",
            "--policy", "taxable",
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("Same-asset taxable", payload["error"]["message"])

        payload = self._cli(
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--tx-out", "manual-out-leg",
            "--tx-in", "manual-in-leg",
            "--kind", "manual",
            "--policy", "carrying-value",
        )
        self._assert_kind(payload, "transfers.pair")
        pair_id = payload["data"]["id"]

        # Listing surfaces both legs with their wallets and assets.
        payload = self._cli("transfers", "list", "--workspace", "Main", "--profile", "ManualPair")
        self._assert_kind(payload, "transfers.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["out"]["wallet"], "From")
        self.assertEqual(payload["data"][0]["in"]["wallet"], "To")

        # Reprocessing now treats the pair as an IntraTransaction: only the
        # 0.0005 BTC fee is realized; the 0.1 BTC carries basis to the To wallet.
        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "ManualPair",
        )
        data = payload["data"]
        self.assertEqual(data["transfers_detected"], 1)
        self.assertEqual(data["cross_asset_pairs"], 0)
        # 1 acquisition + transfer_fee + transfer_out + transfer_in = 4 entries.
        self.assertEqual(data["entries_created"], 4)

        # Excluding a leg of the active pair is refused — it would orphan the
        # other leg into a phantom journal entry. The user must unpair first.
        payload, code = _run(
            self.data_root,
            "metadata", "records", "excluded", "set",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--transaction", "manual-out-leg",
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "conflict")
        self.assertIn(pair_id, payload["error"]["message"])

        # Unpairing reverts behavior to a straight disposal on next process.
        payload = self._cli(
            "transfers", "unpair",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--pair-id", pair_id,
        )
        self._assert_kind(payload, "transfers.unpair")
        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "ManualPair",
        )
        self.assertEqual(payload["data"]["transfers_detected"], 0)
        payload = self._cli(
            "reports", "summary",
            "--workspace", "Main",
            "--profile", "ManualPair",
        )
        self._assert_kind(payload, "reports.summary")
        self.assertEqual(payload["data"]["transfer_pairs"], [])

        # Once unpaired, the leg can be excluded normally.
        payload = self._cli(
            "metadata", "records", "excluded", "set",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--transaction", "manual-out-leg",
        )
        self.assertEqual(payload["data"]["excluded"], True)

    def test_14b_same_wallet_failed_swap_refund_pairing(self):
        workspace = "RefundWorkspace"
        self._cli("init")
        self._cli("workspaces", "create", workspace)
        payload = self._cli(
            "profiles", "create",
            "--workspace", workspace,
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "RefundPair",
        )
        self._assert_kind(payload, "profiles.create")
        self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", "RefundPair",
            "--label", "RefundWallet",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", "RefundPair",
            "--wallet", "RefundWallet",
            "--file", str(self.failed_swap_refund_csv),
        )

        payload = self._cli(
            "transfers", "pair",
            "--workspace", workspace,
            "--profile", "RefundPair",
            "--tx-out", "failed-swap-send",
            "--tx-in", "failed-swap-refund",
            "--kind", "manual",
            "--policy", "carrying-value",
        )
        self._assert_kind(payload, "transfers.pair")
        pair_id = payload["data"]["id"]

        payload = self._cli(
            "transfers", "list",
            "--workspace", workspace,
            "--profile", "RefundPair",
        )
        self._assert_kind(payload, "transfers.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["id"], pair_id)
        self.assertEqual(payload["data"][0]["out"]["wallet"], "RefundWallet")
        self.assertEqual(payload["data"][0]["in"]["wallet"], "RefundWallet")

        payload = self._cli(
            "journals", "process",
            "--workspace", workspace,
            "--profile", "RefundPair",
        )
        data = payload["data"]
        self.assertEqual(data["transfers_detected"], 1)
        self.assertEqual(data["cross_asset_pairs"], 0)
        self.assertEqual(data["quarantined"], 0)
        self.assertEqual(data["entries_created"], 4)

        payload = self._cli(
            "reports", "journal-entries",
            "--workspace", workspace,
            "--profile", "RefundPair",
        )
        entries = payload["data"]
        self.assertEqual(
            sorted(entry["entry_type"] for entry in entries),
            ["acquisition", "transfer_fee", "transfer_in", "transfer_out"],
        )
        out_entry = next(entry for entry in entries if entry["entry_type"] == "transfer_out")
        in_entry = next(entry for entry in entries if entry["entry_type"] == "transfer_in")
        self.assertEqual(out_entry["wallet"], "RefundWallet")
        self.assertEqual(in_entry["wallet"], "RefundWallet")
        self.assertAlmostEqual(float(out_entry["quantity"]), -0.1001, places=8)
        self.assertAlmostEqual(float(in_entry["quantity"]), 0.0998, places=8)

        payload = self._cli(
            "reports", "capital-gains",
            "--workspace", workspace,
            "--profile", "RefundPair",
        )
        gains = payload["data"]
        self.assertEqual(gains, [])

    def test_14c_failed_swap_refund_suggested_by_funding_link(self):
        workspace = "RefundLinkWorkspace"
        self._cli("init")
        self._cli("workspaces", "create", workspace)
        self._cli(
            "profiles", "create",
            "--workspace", workspace,
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "RefundLink",
        )
        self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", "RefundLink",
            "--label", "LinkWallet",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", "RefundLink",
            "--wallet", "LinkWallet",
            "--file", str(self.failed_swap_refund_linked_csv),
        )

        # The send and refund share one wallet and sit 3 days apart, so the
        # time+amount heuristic cannot pair them — only the funding-txid link
        # can. The matcher should surface exactly one exact swap-refund.
        payload = self._cli(
            "transfers", "suggest",
            "--workspace", workspace,
            "--profile", "RefundLink",
        )
        self.assertEqual(payload["kind"], "transfers.suggest")
        candidates = payload["data"]["candidates"]
        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertEqual(candidate["confidence"], "exact")
        self.assertEqual(candidate["method"], "htlc_refund")
        self.assertEqual(candidate["default_kind"], "swap-refund")
        self.assertEqual(candidate["default_policy"], "carrying-value")
        self.assertEqual(candidate["out_wallet_label"], "LinkWallet")
        self.assertEqual(candidate["in_wallet_label"], "LinkWallet")

        # Pair with the dedicated swap-refund kind and confirm the round trip
        # books only the fee, not a disposal.
        payload = self._cli(
            "transfers", "pair",
            "--workspace", workspace,
            "--profile", "RefundLink",
            "--tx-out", _LOCKUP_TXID,
            "--tx-in", _REFUND_TXID,
            "--kind", "swap-refund",
            "--policy", "carrying-value",
        )
        self._assert_kind(payload, "transfers.pair")
        self.assertEqual(payload["data"]["kind"], "swap-refund")

        self._cli(
            "journals", "process",
            "--workspace", workspace,
            "--profile", "RefundLink",
        )
        payload = self._cli(
            "reports", "capital-gains",
            "--workspace", workspace,
            "--profile", "RefundLink",
        )
        gains = payload["data"]
        self.assertEqual(gains, [])

    def test_15_cross_asset_pair_policies(self):
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "CrossAsset",
        )
        self._assert_kind(payload, "profiles.create")
        for label in ("OnchainBTC", "Liquid"):
            self._cli(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "CrossAsset",
                "--label", label,
                "--kind", "custom",
            )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--wallet", "OnchainBTC",
            "--file", str(self.cross_btc_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--wallet", "Liquid",
            "--file", str(self.cross_lbtc_csv),
        )

        # Carrying-value across BTC ↔ LBTC is not yet supported — the CLI must
        # reject the pair creation with a clear validation error envelope.
        payload, code = _run(
            self.data_root,
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--tx-out", "cross-out-leg",
            "--tx-in", "cross-in-leg",
            "--policy", "carrying-value",
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("carrying-value", payload["error"]["message"])

        # Taxable cross-asset pair is accepted and surfaces in the envelope.
        payload = self._cli(
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--tx-out", "cross-out-leg",
            "--tx-in", "cross-in-leg",
            "--kind", "peg-in",
            "--policy", "taxable",
        )
        self._assert_kind(payload, "transfers.pair")
        self.assertEqual(payload["data"]["policy"], "taxable")
        self.assertEqual(payload["data"]["kind"], "peg-in")

        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "CrossAsset",
        )
        data = payload["data"]
        # Cross-asset taxable pair: legs processed independently as SELL+BUY,
        # so transfers_detected stays 0 and cross_asset_pairs reports 1.
        self.assertEqual(data["transfers_detected"], 0)
        self.assertEqual(data["cross_asset_pairs"], 1)

        payload = self._cli(
            "journals", "transfers", "list",
            "--workspace", "Main",
            "--profile", "CrossAsset",
        )
        self._assert_kind(payload, "journals.transfers.list")
        audit = payload["data"]
        self.assertEqual(audit["summary"]["same_asset_transfers"], 0)
        self.assertEqual(audit["summary"]["cross_asset_pairs"], 1)
        pair = audit["cross_asset_pairs"][0]
        self.assertEqual(pair["kind"], "peg-in")
        self.assertEqual(pair["policy"], "taxable")
        self.assertEqual(pair["out_wallet"], "OnchainBTC")
        self.assertEqual(pair["in_wallet"], "Liquid")

        csv_path = Path(self._tmp.name) / "cross-asset-report.csv"
        xlsx_path = Path(self._tmp.name) / "cross-asset-report.xlsx"
        payload = self._cli(
            "reports", "export-csv",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--file", str(csv_path),
        )
        self._assert_kind(payload, "reports.export-csv")
        self.assertIn("Transfers & Swaps", payload["data"]["sections"])
        csv_text = csv_path.read_text(encoding="utf-8")
        self.assertIn("Reviewed Transfers and Swaps", csv_text)
        self.assertIn("Swap Fee msat", csv_text)
        self.assertIn("Swap Fee Kind", csv_text)
        self.assertIn(",swap,peg-in,taxable,", csv_text)
        self.assertIn("cross-out-leg", csv_text)
        self.assertIn("cross-in-leg", csv_text)

        payload = self._cli(
            "reports", "export-xlsx",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--file", str(xlsx_path),
        )
        self._assert_kind(payload, "reports.export-xlsx")
        self.assertIn("Transfers & Swaps", payload["data"]["sheets"])
        with zipfile.ZipFile(xlsx_path) as workbook:
            workbook_xml = workbook.read("xl/workbook.xml").decode("utf-8")
            shared_strings = workbook.read("xl/sharedStrings.xml").decode("utf-8")
        self.assertIn('name="Transfers &amp; Swaps"', workbook_xml)
        self.assertIn("Reviewed Transfers and Swaps", shared_strings)
        self.assertIn("Swap Fee msat", shared_strings)
        self.assertIn("Swap Fee Kind", shared_strings)
        self.assertIn("cross-out-leg", shared_strings)
        self.assertIn("cross-in-leg", shared_strings)

    def test_16_austrian_cross_asset_carrying_value_accepts_same_wallet(self):
        workspace = "CrossAssetAT"
        self._cli("init")
        payload = self._cli("workspaces", "create", workspace)
        self._assert_kind(payload, "workspaces.create")
        payload = self._cli(
            "profiles", "create",
            "--workspace", workspace,
            "--fiat-currency", "EUR",
            "--tax-country", "at",
            "CrossAssetProfile",
        )
        self._assert_kind(payload, "profiles.create")
        self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", "CrossAssetProfile",
            "--label", "Unified",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", "CrossAssetProfile",
            "--wallet", "Unified",
            "--file", str(self.cross_btc_at_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", "CrossAssetProfile",
            "--wallet", "Unified",
            "--file", str(self.cross_lbtc_csv),
        )

        payload = self._cli(
            "transfers", "pair",
            "--workspace", workspace,
            "--profile", "CrossAssetProfile",
            "--tx-out", "cross-out-leg",
            "--tx-in", "cross-in-leg",
            "--kind", "peg-in",
            "--policy", "carrying-value",
        )
        self._assert_kind(payload, "transfers.pair")
        self.assertEqual(payload["data"]["policy"], "carrying-value")

        payload = self._cli(
            "journals", "process",
            "--workspace", workspace,
            "--profile", "CrossAssetProfile",
        )
        data = payload["data"]
        self.assertEqual(data["cross_asset_pairs"], 1)
        self.assertEqual(data["quarantined"], 0)

        payload = self._cli(
            "reports", "summary",
            "--workspace", workspace,
            "--profile", "CrossAssetProfile",
        )
        self._assert_kind(payload, "reports.summary")
        pairs = payload["data"]["transfer_pairs"]
        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["pair_type"], "swap")
        self.assertEqual(pairs[0]["kind"], "peg-in")
        self.assertEqual(pairs[0]["policy"], "carrying-value")
        self.assertEqual(pairs[0]["out_transaction_id"], "cross-out-leg")
        self.assertEqual(pairs[0]["in_transaction_id"], "cross-in-leg")

    def test_17_split_swap_self_transfer_plus_carrying_value_peg(self):
        workspace = "SplitSwapAT"
        self._cli("init")
        self._cli("workspaces", "create", workspace)
        self._cli("profiles", "create", "--workspace", workspace,
                  "--fiat-currency", "EUR", "--tax-country", "at", "SplitSwap")
        for label in ("Spend", "Keep", "Liq"):
            self._cli("wallets", "create", "--workspace", workspace,
                      "--profile", "SplitSwap", "--label", label, "--kind", "custom")
        self._cli("wallets", "import-csv", "--workspace", workspace, "--profile", "SplitSwap",
                  "--wallet", "Spend", "--file", str(self.split_swap_spend_csv))
        self._cli("wallets", "import-csv", "--workspace", workspace, "--profile", "SplitSwap",
                  "--wallet", "Keep", "--file", str(self.split_swap_keep_csv))
        self._cli("wallets", "import-csv", "--workspace", workspace, "--profile", "SplitSwap",
                  "--wallet", "Liq", "--file", str(self.split_swap_lbtc_csv))

        # Unresolved, the 0.05-out / 0.03-in auto-pair has an implausible 0.02
        # "fee" and is quarantined.
        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", "SplitSwap")
        self.assertGreaterEqual(payload["data"]["quarantined"], 1)
        payload = self._cli("journals", "quarantined", "--workspace", workspace, "--profile", "SplitSwap")
        self.assertIn("transfer_fee_implausible", [q["reason"] for q in payload["data"]])

        # Resolve by pairing the BTC spend with the L-BTC peg and declaring the
        # 0.02 BTC that was swapped; the 0.03 remainder is the self-transfer.
        payload = self._cli("transfers", "pair", "--workspace", workspace, "--profile", "SplitSwap",
                            "--tx-out", "splitswap-out", "--tx-in", "splitswap-peg",
                            "--kind", "peg-in", "--policy", "carrying-value", "--out-amount", "0.02")
        self._assert_kind(payload, "transfers.pair")
        self.assertEqual(payload["data"]["out_amount"], 2000000000)
        # Swap fee is the swapped portion (0.02 BTC) minus the L-BTC received
        # (0.0198) = 0.0002 BTC, NOT the full 0.05 outbound minus 0.0198.
        self.assertEqual(payload["data"]["swap_fee_msat"], 20000000)

        # The pair listing must show the SWAPPED portion (consistent with the
        # 0.0002 swap fee), not the full 0.05 outbound, while still exposing the
        # underlying transaction total under full_amount.
        payload = self._cli("transfers", "list", "--workspace", workspace, "--profile", "SplitSwap")
        pair = payload["data"][0]
        self.assertEqual(pair["out"]["amount_msat"], 2000000000)
        self.assertEqual(pair["out"]["full_amount_msat"], 5000000000)

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", "SplitSwap")
        data = payload["data"]
        # Split resolves into a clean self-transfer MOVE + a carrying-value peg;
        # no implausible-fee quarantine remains.
        self.assertEqual(data["quarantined"], 0)
        self.assertEqual(data["transfers_detected"], 1)
        self.assertEqual(data["cross_asset_pairs"], 1)

        # The audit references the REAL BTC out tx (the synthetic split leg is
        # engine-only) and shows both the self-transfer and the peg.
        payload = self._cli("journals", "transfers", "list", "--workspace", workspace, "--profile", "SplitSwap")
        audit = payload["data"]
        self.assertEqual(audit["summary"]["same_asset_transfers"], 1)
        self.assertEqual(audit["summary"]["cross_asset_pairs"], 1)
        self.assertEqual(audit["cross_asset_pairs"][0]["out_wallet"], "Spend")
        self.assertEqual(audit["cross_asset_pairs"][0]["in_wallet"], "Liq")

    def test_18_split_direct_payout_self_transfer_plus_generic_sale(self):
        workspace = "SplitPayoutGeneric"
        self._cli("init")
        self._cli("workspaces", "create", workspace)
        self._cli("profiles", "create", "--workspace", workspace,
                  "--fiat-currency", "USD", "--tax-country", "generic", "SplitPayout")
        for label in ("Spend", "Keep"):
            self._cli("wallets", "create", "--workspace", workspace,
                      "--profile", "SplitPayout", "--label", label, "--kind", "custom")
        self._cli("wallets", "import-csv", "--workspace", workspace, "--profile", "SplitPayout",
                  "--wallet", "Spend", "--file", str(self.split_swap_spend_csv))
        self._cli("wallets", "import-csv", "--workspace", workspace, "--profile", "SplitPayout",
                  "--wallet", "Keep", "--file", str(self.split_swap_keep_csv))

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", "SplitPayout")
        self.assertGreaterEqual(payload["data"]["quarantined"], 1)
        payload = self._cli("journals", "quarantined", "--workspace", workspace, "--profile", "SplitPayout")
        self.assertIn("transfer_fee_implausible", [q["reason"] for q in payload["data"]])

        payload = self._cli("transfers", "payouts", "create",
                            "--workspace", workspace, "--profile", "SplitPayout",
                            "--tx-out", "splitswap-out",
                            "--payout-asset", "BTC", "--payout-amount", "0.0198",
                            "--payout-fiat-value", "1200",
                            "--payout-external-id", "recipient-txid",
                            "--counterparty", "external recipient",
                            "--policy", "taxable",
                            "--out-amount", "0.02")
        self._assert_kind(payload, "transfers.payouts.create")
        self.assertEqual(payload["data"]["out_amount"], 2000000000)
        self.assertEqual(payload["data"]["swap_fee_msat"], 20000000)

        payload = self._cli("transfers", "payouts", "list",
                            "--workspace", workspace, "--profile", "SplitPayout")
        payout = payload["data"][0]
        self.assertEqual(payout["out"]["amount_msat"], 2000000000)
        self.assertEqual(payout["out"]["full_amount_msat"], 5000000000)

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", "SplitPayout")
        self.assertEqual(payload["data"]["quarantined"], 0)
        self.assertEqual(payload["data"]["transfers_detected"], 1)
        self.assertEqual(payload["data"]["direct_swap_payouts"], 1)

        payload = self._cli("reports", "journal-entries",
                            "--workspace", workspace, "--profile", "SplitPayout")
        transfer_fees = [
            entry for entry in payload["data"]
            if entry["entry_type"] == "transfer_fee"
        ]
        self.assertTrue(all(entry["quantity_msat"] < 2000000000 for entry in transfer_fees))
        disposals = [
            entry for entry in payload["data"]
            if entry["entry_type"] == "disposal"
            and entry["asset"] == "BTC"
            and entry["quantity_msat"] == -2000000000
        ]
        self.assertEqual(len(disposals), 1)

    def test_19_split_direct_payout_austrian_carrying_value_neutral_swap(self):
        workspace = "SplitPayoutAT"
        self._cli("init")
        self._cli("workspaces", "create", workspace)
        self._cli("profiles", "create", "--workspace", workspace,
                  "--fiat-currency", "EUR", "--tax-country", "at", "SplitPayoutAT")
        for label in ("Spend", "Keep"):
            self._cli("wallets", "create", "--workspace", workspace,
                      "--profile", "SplitPayoutAT", "--label", label, "--kind", "custom")
        self._cli("wallets", "import-csv", "--workspace", workspace, "--profile", "SplitPayoutAT",
                  "--wallet", "Spend", "--file", str(self.split_swap_spend_csv))
        self._cli("wallets", "import-csv", "--workspace", workspace, "--profile", "SplitPayoutAT",
                  "--wallet", "Keep", "--file", str(self.split_swap_keep_csv))

        self._cli("journals", "process", "--workspace", workspace, "--profile", "SplitPayoutAT")
        payload = self._cli("transfers", "payouts", "create",
                            "--workspace", workspace, "--profile", "SplitPayoutAT",
                            "--tx-out", "splitswap-out",
                            "--payout-asset", "LBTC", "--payout-amount", "0.0198",
                            "--payout-fiat-value", "1188",
                            "--payout-external-id", "external-lbtc-recipient",
                            "--counterparty", "external recipient",
                            "--policy", "carrying-value",
                            "--out-amount", "0.02")
        self._assert_kind(payload, "transfers.payouts.create")

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", "SplitPayoutAT")
        self.assertEqual(payload["data"]["quarantined"], 0)
        self.assertEqual(payload["data"]["transfers_detected"], 1)
        self.assertEqual(payload["data"]["direct_swap_payouts"], 1)

        payload = self._cli("reports", "journal-entries",
                            "--workspace", workspace, "--profile", "SplitPayoutAT")
        neu_swap = [
            entry for entry in payload["data"]
            if entry.get("at_category") == "neu_swap"
            and entry["asset"] == "BTC"
        ]
        self.assertEqual(len(neu_swap), 1)
        self.assertAlmostEqual(float(neu_swap[0]["gain_loss"]), 0.0, places=6)
        taxable = [
            entry for entry in payload["data"]
            if entry.get("at_category") == "neu_gain"
            and entry["asset"] == "LBTC"
        ]
        self.assertEqual(len(taxable), 1)
        self.assertEqual(taxable[0]["asset"], "LBTC")


class AccountBucketBehaviorTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-account-buckets-")
        self.data_root = Path(self._tmp.name) / "data"
        self._cli("init")
        self._cli("workspaces", "create", "Buckets")
        self._cli(
            "profiles", "create",
            "--workspace", "Buckets",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "Default",
        )

    def tearDown(self):
        self._tmp.cleanup()

    def _cli(self, *args):
        payload, code = _run(self.data_root, *args)
        if code != 0:
            self.fail(
                f"CLI exited {code} for {args!r}; envelope: {json.dumps(payload)[:400]}"
            )
        self.assertEqual(payload.get("schema_version"), 1)
        self.assertIn("data", payload)
        return payload

    def _cli_error(self, *args):
        payload, code = _run(self.data_root, *args)
        self.assertNotEqual(code, 0, f"CLI unexpectedly succeeded for {args!r}")
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload.get("schema_version"), 1)
        self.assertIn("error", payload)
        return payload

    def test_new_profiles_seed_only_the_default_reporting_bucket(self):
        payload = self._cli("accounts", "list", "--workspace", "Buckets", "--profile", "Default")
        rows = payload["data"]
        self.assertEqual([row["code"] for row in rows], ["treasury"])
        self.assertEqual(rows[0]["label"], "Treasury")
        self.assertEqual(rows[0]["account_type"], "asset")
        self.assertEqual(rows[0]["asset"], "BTC")

    def test_duplicate_account_label_is_ambiguous_but_code_still_resolves(self):
        for code in ("ops-a", "ops-b"):
            self._cli(
                "accounts", "create",
                "--workspace", "Buckets",
                "--profile", "Default",
                "--code", code,
                "--label", "Operations",
                "--type", "asset",
                "--asset", "BTC",
            )

        payload = self._cli_error(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Ambiguous Wallet",
            "--kind", "custom",
            "--account", "Operations",
        )
        error = payload["error"]
        self.assertEqual(error["code"], "validation")
        self.assertIn("ambiguous", error["message"])
        self.assertEqual(
            [match["code"] for match in error["details"]["matches"]],
            ["ops-a", "ops-b"],
        )

        payload = self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Operations Wallet",
            "--kind", "custom",
            "--account", "ops-a",
        )
        self.assertEqual(payload["data"]["account_code"], "ops-a")

    def test_balance_sheet_groups_holdings_by_wallet_bucket(self):
        events_csv = Path(self._tmp.name) / "events.csv"
        treasury_csv = Path(self._tmp.name) / "treasury.csv"
        events_csv.write_text(
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-01-01T10:00:00Z,events-in,inbound,BTC,0.02000000,0,50000,Event income\n",
            encoding="utf-8",
        )
        treasury_csv.write_text(
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-01-02T10:00:00Z,treasury-in,inbound,BTC,0.10000000,0,51000,Treasury receive\n",
            encoding="utf-8",
        )

        self._cli(
            "accounts", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--code", "events",
            "--label", "Events",
            "--type", "income",
            "--asset", "LBTC",
        )
        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Events Wallet",
            "--kind", "custom",
            "--account", "events",
        )
        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Treasury Wallet",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Events Wallet",
            "--file", str(events_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Treasury Wallet",
            "--file", str(treasury_csv),
        )
        self._cli("journals", "process", "--workspace", "Buckets", "--profile", "Default")

        payload = self._cli("reports", "balance-sheet", "--workspace", "Buckets", "--profile", "Default")
        rows = {row["account"]: row for row in payload["data"]}
        self.assertEqual(set(rows), {"events", "treasury"})
        self.assertAlmostEqual(float(rows["events"]["quantity"]), 0.02, places=8)
        self.assertAlmostEqual(float(rows["treasury"]["quantity"]), 0.1, places=8)
        self.assertEqual(rows["events"]["asset"], "BTC")

    def test_z_river_csv_connection_import(self):
        river_csv = Path(self._tmp.name) / "river-account-activity.csv"
        river_csv.write_text(_RIVER_CSV, encoding="utf-8")

        payload = self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "River",
            "--kind", "river",
            "--source-file", str(river_csv),
            "--source-format", "river_csv",
        )
        self.assertEqual(payload["kind"], "wallets.create")
        self.assertEqual(payload["data"]["source_format"], "river_csv")

        payload = self._cli(
            "wallets", "sync",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "River",
        )
        self.assertEqual(payload["kind"], "wallets.sync")
        self.assertEqual(payload["data"][0]["status"], "synced")
        self.assertEqual(payload["data"][0]["imported"], 3)
        self.assertEqual(payload["data"][0]["unchanged"], 0)
        self.assertEqual(len(payload["data"][0]["inserted_records"]), 3)
        self.assertEqual(payload["data"][0]["inserted_records"][0]["wallet"], "River")
        self.assertEqual(
            payload["data"][0]["inserted_records"][0]["changed_fields"],
            ["metadata", "pricing", "transaction"],
        )
        self.assertEqual(payload["data"][0]["river_notes_set"], 3)
        self.assertEqual(payload["data"][0]["river_tags_added"], 3)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "River",
            "--order", "asc",
        )
        self.assertEqual(payload["kind"], "transactions.list")
        records = payload["data"]
        self.assertEqual(len(records), 3)
        buy = records[0]
        self.assertEqual(buy["kind"], "buy")
        self.assertEqual(buy["direction"], "inbound")
        self.assertEqual(buy["pricing_source_kind"], "exchange_execution")
        self.assertEqual(buy["pricing_provider"], "River")
        self.assertEqual(buy["pricing_method"], "river_csv")
        self.assertEqual(buy["pricing_pair"], "BTC-USD")
        self.assertEqual(buy["fiat_value_exact"], "1005.00")
        self.assertIn({"code": "river:buy", "label": "Buy"}, buy["tags"])

        withdrawal = records[1]
        self.assertEqual(withdrawal["kind"], "withdrawal")
        self.assertEqual(withdrawal["direction"], "outbound")
        self.assertEqual(withdrawal["fee_msat"], 1000000)

        interest = records[2]
        self.assertEqual(interest["kind"], "interest")
        self.assertEqual(interest["pricing_source_kind"], "fmv_provider")
        self.assertEqual(interest["pricing_quality"], "provider_sample")

    def test_z_bullbitcoin_wallet_csv_connection_import(self):
        bull_wallet_csv = Path(self._tmp.name) / "bull-wallet-transactions.csv"
        bull_wallet_csv.write_text(_BULLBITCOIN_WALLET_CSV, encoding="utf-8")

        for label, network in (
            ("Bull Wallet - Bitcoin", "bitcoin"),
            ("Bull Wallet - Liquid", "liquid"),
            ("Bull Wallet - Lightning", "lightning"),
        ):
            payload = self._cli(
                "wallets", "create",
                "--workspace", "Buckets",
                "--profile", "Default",
                "--label", label,
                "--kind", "bullbitcoin",
                "--source-file", str(bull_wallet_csv),
                "--source-format", "bullbitcoin_wallet_csv",
                "--config", json.dumps({"bullbitcoin_wallet_network": network}),
            )
            self.assertEqual(payload["kind"], "wallets.create")
            self.assertEqual(payload["data"]["source_format"], "bullbitcoin_wallet_csv")

        sync_expectations = {
            "Bull Wallet - Bitcoin": ("bitcoin", 2),
            "Bull Wallet - Liquid": ("liquid", 2),
            "Bull Wallet - Lightning": ("lightning", 1),
        }
        for label, (network, rows) in sync_expectations.items():
            payload = self._cli(
                "wallets", "sync",
                "--workspace", "Buckets",
                "--profile", "Default",
                "--wallet", label,
            )
            self.assertEqual(payload["kind"], "wallets.sync")
            sync = payload["data"][0]
            self.assertEqual(sync["status"], "synced")
            self.assertEqual(sync["input_format"], "bullbitcoin_wallet_csv")
            self.assertEqual(sync["bullbitcoin_wallet_network"], network)
            self.assertEqual(sync["bullbitcoin_wallet_rows"], rows)
            self.assertEqual(sync["bullbitcoin_wallet_rows_total"], 5)
            self.assertEqual(sync["imported"], rows)
            self.assertEqual(sync["skipped"], 0)

        bitcoin_records = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Wallet - Bitcoin",
            "--order", "asc",
        )["data"]
        liquid_records = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Wallet - Liquid",
            "--order", "asc",
        )["data"]
        lightning_records = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Wallet - Lightning",
            "--order", "asc",
        )["data"]
        by_external_id = {
            record["external_id"]: record
            for record in [*bitcoin_records, *liquid_records, *lightning_records]
        }
        self.assertEqual(by_external_id["bull-wallet-btc-in"]["asset"], "BTC")
        self.assertEqual(by_external_id["bull-wallet-btc-in"]["direction"], "inbound")
        self.assertEqual(by_external_id["bull-wallet-lbtc-out"]["asset"], "LBTC")
        self.assertEqual(by_external_id["bull-wallet-lbtc-out"]["fee_msat"], 350_000)
        self.assertEqual(by_external_id["bull-chain-send"]["asset"], "BTC")
        self.assertEqual(by_external_id["bull-chain-recv"]["asset"], "LBTC")
        self.assertNotIn("bull-self", by_external_id)
        self.assertNotIn("bull-failed", by_external_id)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        try:
            swap_wallets = conn.execute(
                """
                SELECT t.external_id, w.label AS wallet
                FROM transactions t
                JOIN wallets w ON w.id = t.wallet_id
                WHERE t.external_id IN ('bull-chain-send', 'bull-chain-recv')
                ORDER BY t.external_id
                """
            ).fetchall()
            self.assertEqual(
                {row["external_id"]: row["wallet"] for row in swap_wallets},
                {
                    "bull-chain-recv": "Bull Wallet - Liquid",
                    "bull-chain-send": "Bull Wallet - Bitcoin",
                },
            )
            lightning = conn.execute(
                """
                SELECT payment_hash, payment_hash_source, raw_json
                FROM transactions
                WHERE external_id = ?
                """,
                ("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",),
            ).fetchone()
            self.assertIsNotNone(lightning)
            self.assertEqual(
                lightning["payment_hash"],
                "02d449a31fbb267c8f352e9968a79e3e5fc95c1bbeaa502fd6454ebde5a4bedc",
            )
            self.assertEqual(lightning["payment_hash_source"], "importer")
            self.assertNotIn(
                "1111111111111111111111111111111111111111111111111111111111111111",
                lightning["raw_json"],
            )
            raw = json.loads(lightning["raw_json"])
            self.assertEqual(raw["preimage"], "[redacted]")
            self.assertTrue(raw["preimage_redacted"])

            swap = conn.execute(
                "SELECT raw_json FROM transactions WHERE external_id = 'bull-chain-recv'"
            ).fetchone()
            swap_raw = json.loads(swap["raw_json"])
            self.assertEqual(swap_raw["swap_id"], "swap-chain")
            self.assertEqual(swap_raw["send_network"], "bitcoin")
            self.assertEqual(swap_raw["receive_network"], "liquid")
        finally:
            conn.close()

        payload = self._cli(
            "transfers", "suggest",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--method", "provider_swap_id",
        )
        candidates = payload["data"]["candidates"]
        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertEqual(candidate["method"], "provider_swap_id")
        self.assertEqual(candidate["confidence"], "exact")
        self.assertEqual(candidate["default_kind"], "chain-swap")
        self.assertEqual(candidate["out_asset"], "BTC")
        self.assertEqual(candidate["in_asset"], "LBTC")
        self.assertEqual(candidate["out_wallet_label"], "Bull Wallet - Bitcoin")
        self.assertEqual(candidate["in_wallet_label"], "Bull Wallet - Liquid")
        self.assertEqual(candidate["swap_fee_msat"], 10_500_000)
        self.assertEqual(candidate["evidence"]["provider"], "bullbitcoin")
        self.assertEqual(candidate["evidence"]["id"], "swap-chain")

        existing_btc_csv = Path(self._tmp.name) / "bull-existing-btc.csv"
        existing_btc_csv.write_text(
            "date,txid,direction,asset,amount,fee\n"
            "2026-04-05T09:15:00Z,bull-chain-send,outbound,BTC,0.01000000,0.00000500\n",
            encoding="utf-8",
        )
        # A real descriptor backend stores a receive with fee 0 (the recipient
        # pays no fee), while the Bull export reports a nonzero fee on the same
        # swap row. Enrichment must still match by txid/amount despite the fee
        # mismatch, so keep this descriptor receive fee at 0 on purpose.
        existing_liquid_csv = Path(self._tmp.name) / "bull-existing-liquid.csv"
        existing_liquid_csv.write_text(
            "date,txid,direction,asset,amount,fee\n"
            "2026-04-05T09:20:00Z,bull-chain-recv,inbound,LBTC,0.00990000,0\n",
            encoding="utf-8",
        )
        for label, csv_path in (
            ("Descriptor BTC", existing_btc_csv),
            ("Descriptor Liquid", existing_liquid_csv),
        ):
            self._cli(
                "wallets", "create",
                "--workspace", "Buckets",
                "--profile", "Default",
                "--label", label,
                "--kind", "custom",
                "--source-file", str(csv_path),
                "--source-format", "csv",
            )
            self._cli(
                "wallets", "sync",
                "--workspace", "Buckets",
                "--profile", "Default",
                "--wallet", label,
            )
        for label, network in (("Descriptor BTC", "bitcoin"), ("Descriptor Liquid", "liquid")):
            payload = self._cli(
                "wallets", "attach-bullbitcoin-wallet",
                "--workspace", "Buckets",
                "--profile", "Default",
                "--wallet", label,
                "--file", str(bull_wallet_csv),
                "--network", network,
            )
            self.assertEqual(payload["kind"], "wallets.attach-bullbitcoin-wallet")
            self.assertEqual(payload["data"]["network"], network)

        payload = self._cli(
            "wallets", "sync",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Descriptor BTC",
        )
        self.assertEqual(payload["data"][0]["bullbitcoin_wallet_exports"]["updated"], 1)
        payload = self._cli(
            "wallets", "sync",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Descriptor Liquid",
        )
        self.assertEqual(payload["data"][0]["bullbitcoin_wallet_exports"]["updated"], 1)
        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        try:
            descriptor_rows = conn.execute(
                """
                SELECT w.label AS wallet, t.external_id, t.kind, t.fee, t.raw_json
                FROM transactions t
                JOIN wallets w ON w.id = t.wallet_id
                WHERE w.label IN ('Descriptor BTC', 'Descriptor Liquid')
                ORDER BY w.label
                """
            ).fetchall()
            self.assertEqual(len(descriptor_rows), 2)
            self.assertEqual({row["kind"] for row in descriptor_rows}, {"chain_swap"})
            for row in descriptor_rows:
                raw = json.loads(row["raw_json"])
                self.assertEqual(raw["swap_id"], "swap-chain")
            fees_by_external_id = {
                row["external_id"]: row["fee"] for row in descriptor_rows
            }
            # The matching network-fee send still reconciles exactly, and the
            # fee-0 descriptor receive is enriched without its stored fee being
            # overwritten by the Bull-reported swap fee.
            self.assertEqual(fees_by_external_id["bull-chain-send"], 500_000)
            self.assertEqual(fees_by_external_id["bull-chain-recv"], 0)
        finally:
            conn.close()

    def test_z_bullbitcoin_wallet_csv_refunded_swap_skipped_without_refund_leg(self):
        bull_wallet_csv = Path(self._tmp.name) / "bull-wallet-refund-transactions.csv"
        bull_wallet_csv.write_text(_BULLBITCOIN_WALLET_REFUND_CSV, encoding="utf-8")

        payload = self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Bull Refund Wallet",
            "--kind", "bullbitcoin",
            "--source-file", str(bull_wallet_csv),
            "--source-format", "bullbitcoin_wallet_csv",
            "--config", json.dumps({"bullbitcoin_wallet_network": "bitcoin"}),
        )
        self.assertEqual(payload["kind"], "wallets.create")

        payload = self._cli(
            "wallets", "sync",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Refund Wallet",
        )
        sync = payload["data"][0]
        self.assertEqual(sync["input_format"], "bullbitcoin_wallet_csv")
        self.assertEqual(sync["bullbitcoin_wallet_rows"], 0)
        self.assertEqual(sync["bullbitcoin_wallet_rows_total"], 0)
        self.assertEqual(sync["imported"], 0)
        self.assertEqual(sync["skipped"], 0)

        payload = self._cli(
            "transfers", "suggest",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--method", "provider_swap_id",
        )
        self.assertEqual(payload["data"]["candidates"], [])

    def test_z_bullbitcoin_csv_enriches_existing_wallet_transaction(self):
        existing_csv = Path(self._tmp.name) / "bull-existing-wallet.csv"
        existing_csv.write_text(_BULLBITCOIN_EXISTING_CSV, encoding="utf-8")
        bull_csv = Path(self._tmp.name) / "bull-orders.csv"
        bull_csv.write_text(_BULLBITCOIN_ORDERS_CSV, encoding="utf-8")

        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Bull Matched",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Matched",
            "--file", str(existing_csv),
        )

        payload = self._cli(
            "wallets", "import-bull",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Matched",
            "--file", str(bull_csv),
        )
        self.assertEqual(payload["kind"], "wallets.import-bull")
        self.assertEqual(payload["data"]["input_format"], "bullbitcoin_csv")
        self.assertEqual(payload["data"]["bullbitcoin_rows"], 2)
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["updated"], 1)
        # One completed order belongs to another wallet and row 1003 is canceled.
        self.assertEqual(payload["data"]["skipped"], 2)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Matched",
            "--order", "asc",
        )
        records = payload["data"]
        self.assertEqual(len(records), 1)
        sell = records[0]
        self.assertEqual(sell["kind"], "sell")
        self.assertEqual(sell["fee_msat"], 1413000)
        self.assertEqual(sell["pricing_source_kind"], "exchange_execution")
        self.assertEqual(sell["pricing_provider"], "Bull Bitcoin")
        self.assertEqual(sell["pricing_method"], "bullbitcoin_csv")
        self.assertEqual(sell["pricing_pair"], "BTC-USD")
        self.assertEqual(sell["pricing_external_ref"], "order-1")
        self.assertEqual(sell["fiat_value_exact"], "4202.19")
        self.assertEqual(sell["fiat_rate_exact"], "61694.45")
        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        try:
            row = conn.execute(
                "SELECT counterparty FROM transactions WHERE external_id = 'bull-sell-tx'"
            ).fetchone()
        finally:
            conn.close()
        self.assertEqual(row["counterparty"], "Bull Bitcoin")

    def test_z_bullbitcoin_csv_enriches_unique_book_wide_transaction(self):
        other_existing_csv = Path(self._tmp.name) / "bull-other-wallet.csv"
        other_existing_csv.write_text(_BULLBITCOIN_OTHER_WALLET_EXISTING_CSV, encoding="utf-8")
        bull_csv = Path(self._tmp.name) / "bull-book-orders.csv"
        bull_csv.write_text(_BULLBITCOIN_ORDERS_CSV, encoding="utf-8")

        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Bull Book Primary",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Bull Book Other",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Book Other",
            "--file", str(other_existing_csv),
        )

        payload = self._cli(
            "wallets", "import-bull",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Book Primary",
            "--file", str(bull_csv),
        )
        self.assertEqual(payload["data"]["scope"], "book")
        self.assertEqual(payload["data"]["matched"], 1)
        self.assertEqual(payload["data"]["updated"], 1)
        self.assertEqual(payload["data"]["skipped"], 2)
        self.assertEqual(payload["data"]["skipped_unmatched"], 1)
        self.assertEqual(payload["data"]["skipped_ambiguous"], 0)
        self.assertEqual(payload["data"]["updated_records"][0]["wallet"], "Bull Book Other")
        self.assertEqual(payload["data"]["updated_records"][0]["external_id"], "other-wallet-tx")
        self.assertIn("pricing_provider", payload["data"]["updated_records"][0]["changed_fields"])

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Book Other",
        )
        sell = payload["data"][0]
        self.assertEqual(sell["external_id"], "other-wallet-tx")
        self.assertEqual(sell["pricing_provider"], "Bull Bitcoin")
        self.assertEqual(sell["pricing_method"], "bullbitcoin_csv")
        self.assertEqual(sell["pricing_external_ref"], "order-2")
        self.assertEqual(sell["fiat_value_exact"], "600.00")
        self.assertEqual(sell["fee_msat"], 100000)

    def test_z_bullbitcoin_csv_skips_ambiguous_book_wide_matches(self):
        existing_csv = Path(self._tmp.name) / "bull-ambiguous-wallet.csv"
        existing_csv.write_text(_BULLBITCOIN_OTHER_WALLET_EXISTING_CSV, encoding="utf-8")
        bull_csv = Path(self._tmp.name) / "bull-ambiguous-orders.csv"
        bull_csv.write_text(_BULLBITCOIN_ORDERS_CSV, encoding="utf-8")

        for wallet_label in ("Bull Ambiguous One", "Bull Ambiguous Two"):
            self._cli(
                "wallets", "create",
                "--workspace", "Buckets",
                "--profile", "Default",
                "--label", wallet_label,
                "--kind", "custom",
            )
            self._cli(
                "wallets", "import-csv",
                "--workspace", "Buckets",
                "--profile", "Default",
                "--wallet", wallet_label,
                "--file", str(existing_csv),
            )

        payload = self._cli(
            "wallets", "import-bull",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Ambiguous One",
            "--file", str(bull_csv),
        )
        self.assertEqual(payload["data"].get("updated", 0), 0)
        self.assertEqual(payload["data"]["skipped"], 2)
        self.assertEqual(payload["data"]["matched"], 0)
        self.assertEqual(payload["data"]["skipped_unmatched"], 1)
        self.assertEqual(payload["data"]["skipped_ambiguous"], 1)
        self.assertEqual(payload["data"]["updated_records"], [])

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        try:
            enriched = conn.execute(
                """
                SELECT COUNT(*)
                FROM transactions
                WHERE external_id = 'other-wallet-tx'
                  AND pricing_provider = 'Bull Bitcoin'
                """
            ).fetchone()[0]
        finally:
            conn.close()
        self.assertEqual(enriched, 0)

    def test_z_bullbitcoin_csv_full_import_flags_shared_account_rows(self):
        existing_csv = Path(self._tmp.name) / "bull-full-existing-wallet.csv"
        existing_csv.write_text(_BULLBITCOIN_EXISTING_CSV, encoding="utf-8")
        bull_csv = Path(self._tmp.name) / "bull-full-orders.csv"
        bull_csv.write_text(_BULLBITCOIN_ORDERS_CSV, encoding="utf-8")

        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Operations Wallet",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Operations Wallet",
            "--file", str(existing_csv),
        )

        payload = self._cli(
            "wallets", "import-bull",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--file", str(bull_csv),
            "--mode", "full",
        )
        data = payload["data"]
        self.assertEqual(data["scope"], "book")
        self.assertEqual(data["mode"], "full")
        self.assertEqual(data["wallet"], "Bull Bitcoin")
        self.assertEqual(data["bullbitcoin_rows"], 2)
        self.assertEqual(data["imported"], 2)
        self.assertEqual(data["matched"], 1)
        self.assertEqual(data["unmatched"], 1)
        self.assertEqual(data["ambiguous"], 0)
        self.assertEqual(data["excluded"], 2)
        self.assertEqual(
            sorted(record["status"] for record in data["inserted_records"]),
            ["matched", "unmatched"],
        )

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Bitcoin",
            "--order", "asc",
        )
        records = payload["data"]
        self.assertEqual(len(records), 2)
        self.assertTrue(all(record["excluded"] for record in records))
        tags_by_external_id = {
            record["external_id"]: {tag["code"] for tag in record["tags"]}
            for record in records
        }
        self.assertIn("bullbitcoin-matched", tags_by_external_id["bull-sell-tx"])
        self.assertIn("bullbitcoin-wallet-gap", tags_by_external_id["other-wallet-tx"])

    def test_z_bullbitcoin_csv_preserves_manual_transaction_metadata(self):
        existing_csv = Path(self._tmp.name) / "bull-manual-existing-wallet.csv"
        existing_csv.write_text(_BULLBITCOIN_EXISTING_CSV, encoding="utf-8")
        bull_csv = Path(self._tmp.name) / "bull-manual-orders.csv"
        bull_csv.write_text(_BULLBITCOIN_ORDERS_CSV, encoding="utf-8")

        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Bull Manual",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Manual",
            "--file", str(existing_csv),
        )

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        try:
            conn.execute(
                """
                UPDATE transactions
                SET kind = 'manual_kind',
                    description = 'Manual description',
                    counterparty = 'Manual counterparty'
                WHERE external_id = 'bull-sell-tx'
                """
            )
            conn.commit()
        finally:
            conn.close()

        payload = self._cli(
            "wallets", "import-bull",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Manual",
            "--file", str(bull_csv),
        )
        self.assertEqual(payload["data"]["updated"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        try:
            row = conn.execute(
                """
                SELECT kind, description, counterparty, pricing_provider
                FROM transactions
                WHERE external_id = 'bull-sell-tx'
                """
            ).fetchone()
        finally:
            conn.close()
        self.assertEqual(row["kind"], "manual_kind")
        self.assertEqual(row["description"], "Manual description")
        self.assertEqual(row["counterparty"], "Manual counterparty")
        self.assertEqual(row["pricing_provider"], "Bull Bitcoin")

    def test_z_bullbitcoin_csv_skips_unmatched_foreign_fiat_order(self):
        existing_csv = Path(self._tmp.name) / "bull-foreign-existing-wallet.csv"
        existing_csv.write_text(_BULLBITCOIN_EXISTING_CSV, encoding="utf-8")
        bull_csv = Path(self._tmp.name) / "bull-foreign-orders.csv"
        bull_csv.write_text(
            _BULLBITCOIN_ORDERS_CSV.replace(
                "1002,Fiat Payment,Market Order,,order-2,0.01000000,BTC,600.00,USD,60000.00,USD",
                "1002,Fiat Payment,Market Order,,order-2,0.01000000,BTC,600.00,EUR,60000.00,EUR",
            ),
            encoding="utf-8",
        )

        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Bull Foreign Skip",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Foreign Skip",
            "--file", str(existing_csv),
        )

        payload = self._cli(
            "wallets", "import-bull",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull Foreign Skip",
            "--file", str(bull_csv),
        )
        self.assertEqual(payload["data"]["updated"], 1)
        self.assertEqual(payload["data"]["skipped"], 2)

    def test_z_bullbitcoin_csv_enriches_lightning_method_order(self):
        existing_csv = Path(self._tmp.name) / "bull-ln-existing-wallet.csv"
        existing_csv.write_text(_BULLBITCOIN_LN_EXISTING_CSV, encoding="utf-8")
        bull_csv = Path(self._tmp.name) / "bull-ln-orders.csv"
        bull_csv.write_text(_BULLBITCOIN_LN_ORDERS_CSV, encoding="utf-8")

        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Bull LN Matched",
            "--kind", "phoenix",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull LN Matched",
            "--file", str(existing_csv),
        )

        payload = self._cli(
            "wallets", "import-bull",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull LN Matched",
            "--file", str(bull_csv),
        )
        self.assertEqual(payload["data"]["bullbitcoin_rows"], 1)
        self.assertEqual(payload["data"]["updated"], 1)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Bull LN Matched",
        )
        buy = payload["data"][0]
        self.assertEqual(buy["kind"], "buy")
        self.assertEqual(buy["asset"], "BTC")
        self.assertEqual(buy["pricing_source_kind"], "exchange_execution")
        self.assertEqual(buy["pricing_provider"], "Bull Bitcoin")
        self.assertEqual(buy["pricing_method"], "bullbitcoin_csv")
        self.assertEqual(buy["pricing_external_ref"], "order-ln-1")
        self.assertEqual(buy["fiat_value_exact"], "600.00")
        self.assertEqual(buy["fiat_rate_exact"], "60000.00")

    def test_z_coinfinity_csv_enriches_existing_wallet_transaction(self):
        existing_csv = Path(self._tmp.name) / "coinfinity-existing-wallet.csv"
        existing_csv.write_text(_COINFINITY_EXISTING_CSV, encoding="utf-8")
        coinfinity_csv = Path(self._tmp.name) / "coinfinity-orders.csv"
        coinfinity_csv.write_text(_COINFINITY_ORDERS_CSV, encoding="utf-8")

        self._cli(
            "profiles", "create",
            "--workspace", "Buckets",
            "--fiat-currency", "EUR",
            "--tax-country", "generic",
            "Coinfinity Euro",
        )
        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Euro",
            "--label", "Coinfinity Self Custody",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Euro",
            "--wallet", "Coinfinity Self Custody",
            "--file", str(existing_csv),
        )

        payload = self._cli(
            "wallets", "import-coinfinity",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Euro",
            "--wallet", "Coinfinity Self Custody",
            "--file", str(coinfinity_csv),
        )
        self.assertEqual(payload["kind"], "wallets.import-coinfinity")
        self.assertEqual(payload["data"]["input_format"], "coinfinity_csv")
        self.assertEqual(payload["data"]["coinfinity_rows"], 2)
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["updated"], 1)
        self.assertEqual(payload["data"]["matched"], 1)
        self.assertEqual(payload["data"]["skipped_unmatched"], 1)
        self.assertEqual(payload["data"]["skipped"], 2)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Euro",
            "--wallet", "Coinfinity Self Custody",
        )
        buy = payload["data"][0]
        self.assertEqual(buy["external_id"], "coinfinity-buy-tx")
        self.assertEqual(buy["kind"], "buy")
        self.assertEqual(buy["pricing_source_kind"], "exchange_execution")
        self.assertEqual(buy["pricing_provider"], "Coinfinity")
        self.assertEqual(buy["pricing_method"], "coinfinity_csv")
        self.assertEqual(buy["pricing_pair"], "BTC-EUR")
        self.assertEqual(buy["pricing_external_ref"], "BCBC-229A-AB")
        self.assertEqual(buy["fiat_value_exact"], "101.52")
        self.assertEqual(
            buy["fiat_rate_exact"],
            "68872.400000000000000000000000",
        )

    def test_z_coinfinity_csv_full_import_flags_wallet_gap_rows(self):
        existing_csv = Path(self._tmp.name) / "coinfinity-full-existing-wallet.csv"
        existing_csv.write_text(_COINFINITY_EXISTING_CSV, encoding="utf-8")
        coinfinity_csv = Path(self._tmp.name) / "coinfinity-full-orders.csv"
        coinfinity_csv.write_text(_COINFINITY_ORDERS_CSV, encoding="utf-8")

        self._cli(
            "profiles", "create",
            "--workspace", "Buckets",
            "--fiat-currency", "EUR",
            "--tax-country", "generic",
            "Coinfinity Full Euro",
        )
        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Full Euro",
            "--label", "Coinfinity Receive Wallet",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Full Euro",
            "--wallet", "Coinfinity Receive Wallet",
            "--file", str(existing_csv),
        )

        payload = self._cli(
            "wallets", "import-coinfinity",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Full Euro",
            "--file", str(coinfinity_csv),
            "--mode", "full",
        )
        data = payload["data"]
        self.assertEqual(data["scope"], "book")
        self.assertEqual(data["mode"], "full")
        self.assertEqual(data["wallet"], "Coinfinity")
        self.assertEqual(data["coinfinity_rows"], 2)
        self.assertEqual(data["imported"], 2)
        self.assertEqual(data["matched"], 1)
        self.assertEqual(data["unmatched"], 1)
        self.assertEqual(data["excluded"], 2)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Full Euro",
            "--wallet", "Coinfinity",
            "--order", "asc",
        )
        records = payload["data"]
        self.assertEqual(len(records), 2)
        self.assertTrue(all(record["excluded"] for record in records))
        tags_by_external_id = {
            record["external_id"]: {tag["code"] for tag in record["tags"]}
            for record in records
        }
        self.assertIn(
            "coinfinity-matched",
            tags_by_external_id["coinfinity-buy-tx"],
        )
        self.assertIn(
            "coinfinity-wallet-gap",
            tags_by_external_id["coinfinity-sell-tx"],
        )
        sell = next(
            record for record in records if record["external_id"] == "coinfinity-sell-tx"
        )
        self.assertEqual(sell["kind"], "sell")
        self.assertEqual(sell["fee_msat"], 134000)
        self.assertEqual(sell["fiat_value_exact"], "2954.92")

    def test_z_coinfinity_csv_matches_ln_invoice_before_economics(self):
        existing_csv = Path(self._tmp.name) / "coinfinity-ln-existing-wallet.csv"
        existing_csv.write_text(_COINFINITY_LN_EXISTING_CSV, encoding="utf-8")
        coinfinity_csv = Path(self._tmp.name) / "coinfinity-ln-orders.csv"
        coinfinity_csv.write_text(_COINFINITY_LN_ORDERS_CSV, encoding="utf-8")

        self._cli(
            "profiles", "create",
            "--workspace", "Buckets",
            "--fiat-currency", "EUR",
            "--tax-country", "generic",
            "Coinfinity Lightning Euro",
        )
        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Lightning Euro",
            "--label", "Coinfinity Lightning Wallet",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Lightning Euro",
            "--wallet", "Coinfinity Lightning Wallet",
            "--file", str(existing_csv),
        )

        payload = self._cli(
            "wallets", "import-coinfinity",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Lightning Euro",
            "--wallet", "Coinfinity Lightning Wallet",
            "--file", str(coinfinity_csv),
        )
        self.assertEqual(payload["data"]["matched"], 1)
        self.assertEqual(payload["data"]["updated"], 1)
        self.assertEqual(payload["data"]["skipped_unmatched"], 0)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Coinfinity Lightning Euro",
            "--wallet", "Coinfinity Lightning Wallet",
            "--sort", "occurred-at",
            "--order", "asc",
        )
        matched, other = payload["data"]
        self.assertEqual(matched["external_id"], "lnbc1coinfinityinvoice")
        self.assertEqual(matched["pricing_provider"], "Coinfinity")
        self.assertEqual(matched["pricing_external_ref"], "BCBC-LN-01")
        self.assertEqual(matched["fiat_value_exact"], "505.00")
        self.assertEqual(other["external_id"], "lnbc1otherinvoice")
        self.assertIsNone(other["pricing_provider"])

    def test_z_21bitcoin_csv_enriches_existing_wallet_transaction(self):
        existing_csv = Path(self._tmp.name) / "21bitcoin-existing-wallet.csv"
        existing_csv.write_text(_TWENTYONEBITCOIN_EXISTING_CSV, encoding="utf-8")
        transactions_csv = Path(self._tmp.name) / "21bitcoin-transactions.csv"
        transactions_csv.write_text(_TWENTYONEBITCOIN_TRANSACTIONS_CSV, encoding="utf-8")
        self._cli(
            "profiles", "create",
            "--workspace", "Buckets",
            "--fiat-currency", "EUR",
            "--tax-country", "generic",
            "Euro",
        )

        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Euro",
            "--label", "21bitcoin Matched",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Euro",
            "--wallet", "21bitcoin Matched",
            "--file", str(existing_csv),
        )

        payload = self._cli(
            "wallets", "import-21bitcoin",
            "--workspace", "Buckets",
            "--profile", "Euro",
            "--wallet", "21bitcoin Matched",
            "--file", str(transactions_csv),
            "--mode", "relevant",
        )
        self.assertEqual(payload["kind"], "wallets.import-21bitcoin")
        self.assertEqual(payload["data"]["input_format"], "21bitcoin_csv")
        self.assertEqual(payload["data"]["twentyonebitcoin_rows"], 2)
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"].get("updated", 0), 0)
        self.assertEqual(payload["data"]["unchanged"], 1)
        self.assertEqual(payload["data"]["skipped"], 2)
        self.assertEqual(payload["data"]["matched"], 1)
        self.assertEqual(payload["data"]["skipped_unmatched"], 1)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Euro",
            "--wallet", "21bitcoin Matched",
            "--order", "asc",
        )
        records = {record["external_id"]: record for record in payload["data"]}
        buy = records["21bitcoin:2"]
        self.assertEqual(buy["kind"], "buy")
        self.assertIsNone(buy["pricing_provider"])
        self.assertNotEqual(buy["pricing_method"], "21bitcoin_csv")
        withdrawal = records["l1-withdrawal-tx"]
        self.assertEqual(withdrawal["kind"], "withdrawal")
        self.assertIsNone(withdrawal["pricing_method"])

    def test_z_21bitcoin_csv_full_imports_active_custodial_ledger_rows(self):
        receive_csv = Path(self._tmp.name) / "21bitcoin-full-receive-wallet.csv"
        receive_csv.write_text(_TWENTYONEBITCOIN_RECEIVE_CSV, encoding="utf-8")
        transactions_csv = Path(self._tmp.name) / "21bitcoin-full-transactions.csv"
        transactions_csv.write_text(_TWENTYONEBITCOIN_TRANSACTIONS_CSV, encoding="utf-8")
        self._cli(
            "profiles", "create",
            "--workspace", "Buckets",
            "--fiat-currency", "EUR",
            "--tax-country", "generic",
            "Euro",
        )

        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Euro",
            "--label", "Cold Wallet",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Euro",
            "--wallet", "Cold Wallet",
            "--file", str(receive_csv),
        )

        payload = self._cli(
            "wallets", "import-21bitcoin",
            "--workspace", "Buckets",
            "--profile", "Euro",
            "--file", str(transactions_csv),
        )
        data = payload["data"]
        self.assertEqual(data["mode"], "full")
        self.assertEqual(data["wallet"], "21bitcoin")
        self.assertEqual(data["twentyonebitcoin_rows"], 2)
        self.assertEqual(data["imported"], 2)
        self.assertEqual(data["skipped"], 0)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Euro",
            "--wallet", "21bitcoin",
            "--order", "asc",
        )
        records = payload["data"]
        self.assertEqual(len(records), 2)
        self.assertTrue(all(not record["excluded"] for record in records))
        by_external_id = {record["external_id"]: record for record in records}
        buy = by_external_id["21bitcoin:2"]
        self.assertEqual(buy["kind"], "buy")
        self.assertEqual(buy["pricing_provider"], "21bitcoin")
        self.assertEqual(buy["pricing_method"], "21bitcoin_csv")
        self.assertEqual(buy["fiat_value_exact"], "37.49")
        withdrawal = by_external_id["l1-withdrawal-tx"]
        self.assertEqual(withdrawal["kind"], "withdrawal")
        self.assertIsNone(withdrawal["pricing_method"])
        self.assertEqual(withdrawal["pricing_external_ref"], "16")

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Euro",
            "--wallet", "Cold Wallet",
            "--order", "asc",
        )
        receive = payload["data"][0]
        self.assertEqual(receive["external_id"], "l1-withdrawal-tx")

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        stale = conn.execute(
            """
            SELECT t.id AS transaction_id, t.workspace_id, t.profile_id
            FROM transactions t
            JOIN wallets w ON w.id = t.wallet_id
            WHERE t.external_id = ?
              AND w.label = ?
            """,
            ("21bitcoin:2", "21bitcoin"),
        ).fetchone()
        conn.execute(
            """
            INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at)
            VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                "stale-21bitcoin-gap",
                stale["workspace_id"],
                stale["profile_id"],
                "21bitcoin-wallet-gap",
                "21bitcoin wallet gap",
                "2024-01-01T00:00:00Z",
            ),
        )
        conn.execute(
            "INSERT INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
            (stale["transaction_id"], "stale-21bitcoin-gap"),
        )
        conn.execute(
            "UPDATE transactions SET excluded = 1 WHERE id = ?",
            (stale["transaction_id"],),
        )
        conn.commit()
        conn.close()

        payload = self._cli(
            "wallets", "import-21bitcoin",
            "--workspace", "Buckets",
            "--profile", "Euro",
            "--file", str(transactions_csv),
        )
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["reactivated"], 1)
        self.assertEqual(payload["data"]["reconciliation_flags_cleared"], 1)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Euro",
            "--wallet", "21bitcoin",
            "--order", "asc",
        )
        buy = {record["external_id"]: record for record in payload["data"]}["21bitcoin:2"]
        self.assertFalse(buy["excluded"])
        self.assertNotIn("21bitcoin-wallet-gap", {tag["code"] for tag in buy["tags"]})

    def test_strike_csv_imports_active_custodial_bitcoin_rows(self):
        strike_csv = Path(self._tmp.name) / "strike.csv"
        strike_csv.write_text(_STRIKE_CSV, encoding="utf-8")
        self._cli(
            "profiles", "create",
            "--workspace", "Buckets",
            "--fiat-currency", "EUR",
            "--tax-country", "generic",
            "StrikeEUR",
        )

        payload = self._cli(
            "wallets", "import-strike",
            "--workspace", "Buckets",
            "--profile", "StrikeEUR",
            "--file", str(strike_csv),
        )
        data = payload["data"]
        self.assertEqual(payload["kind"], "wallets.import-strike")
        self.assertEqual(data["mode"], "full")
        self.assertEqual(data["wallet"], "Strike")
        self.assertEqual(data["input_format"], "strike_csv")
        self.assertEqual(data["strike_rows"], 5)
        self.assertEqual(data["imported"], 5)
        self.assertEqual(data["skipped"], 0)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "StrikeEUR",
            "--wallet", "Strike",
            "--order", "asc",
        )
        records = payload["data"]
        self.assertEqual(len(records), 5)
        by_external_id = {record["external_id"]: record for record in records}
        buy = by_external_id["strike:strike-buy-1"]
        self.assertEqual(buy["kind"], "buy")
        self.assertEqual(buy["direction"], "inbound")
        self.assertEqual(buy["amount_msat"], 100000000)
        self.assertEqual(buy["pricing_source_kind"], "exchange_execution")
        self.assertEqual(buy["pricing_provider"], "Strike")
        self.assertEqual(buy["pricing_method"], "strike_csv")
        self.assertEqual(buy["pricing_external_ref"], "strike-buy-1")
        self.assertEqual(buy["fiat_rate_exact"], "100000.00")
        self.assertEqual(buy["fiat_value_exact"], "101.00")
        sell = by_external_id["strike:strike-sell-1"]
        self.assertEqual(sell["kind"], "sell")
        self.assertEqual(sell["direction"], "outbound")
        self.assertEqual(sell["amount_msat"], 50000000)
        self.assertEqual(sell["pricing_source_kind"], "exchange_execution")
        self.assertEqual(sell["pricing_provider"], "Strike")
        self.assertEqual(sell["pricing_method"], "strike_csv")
        self.assertEqual(sell["pricing_external_ref"], "strike-sell-1")
        self.assertEqual(sell["fiat_rate_exact"], "100000.00")
        self.assertEqual(sell["fiat_value_exact"], "48.00")
        lightning = by_external_id["strike:strike-ln-1"]
        self.assertEqual(lightning["kind"], "receive")
        self.assertEqual(lightning["direction"], "inbound")
        self.assertEqual(lightning["amount_msat"], 272794000)
        self.assertIsNone(lightning["pricing_method"])
        price_only = by_external_id["strike:strike-price-only"]
        self.assertEqual(price_only["kind"], "transaction")
        self.assertEqual(price_only["direction"], "inbound")
        self.assertEqual(price_only["amount_msat"], 50000000)
        self.assertEqual(price_only["pricing_source_kind"], "exchange_execution")
        self.assertEqual(price_only["pricing_method"], "strike_csv")
        self.assertEqual(price_only["pricing_external_ref"], "strike-price-only")
        self.assertEqual(price_only["fiat_rate_exact"], "80000.00")
        self.assertEqual(price_only["fiat_value_exact"], "40.0000000000")
        onchain = by_external_id[
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        ]
        self.assertEqual(onchain["kind"], "send")
        self.assertEqual(onchain["direction"], "outbound")
        self.assertEqual(onchain["fee_msat"], 1000000)
        self.assertEqual(onchain["pricing_provider"], "Strike")
        self.assertEqual(onchain["pricing_method"], "strike_csv")
        self.assertEqual(onchain["pricing_external_ref"], "strike-chain-1")
        self.assertEqual(onchain["fiat_rate_exact"], "60000.00")
        self.assertEqual(onchain["fiat_value_exact"], "60.0000000000")

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            """
            SELECT payment_hash, payment_hash_source
            FROM transactions
            WHERE external_id = ?
            """,
            ("strike:strike-ln-1",),
        ).fetchone()
        conn.close()
        self.assertIsNotNone(row)
        self.assertEqual(
            row["payment_hash"],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        self.assertEqual(row["payment_hash_source"], "importer")

    def test_ledgerlive_csv_imports_wallet_movement(self):
        ledger_csv = Path(self._tmp.name) / "ledger-live.csv"
        ledger_csv.write_text(_LEDGERLIVE_CSV, encoding="utf-8")

        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Ledger Live",
            "--kind", "ledgerlive",
            "--source-file", str(ledger_csv),
            "--source-format", "ledgerlive_csv",
        )
        payload = self._cli(
            "wallets", "sync",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Ledger Live",
        )
        self.assertEqual(payload["data"][0]["input_format"], "ledgerlive_csv")
        self.assertEqual(payload["data"][0]["imported"], 2)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Ledger Live",
            "--order", "asc",
        )
        records = payload["data"]
        self.assertEqual(len(records), 2)
        self.assertEqual(records[0]["external_id"], "ledger-in")
        self.assertEqual(records[0]["kind"], "deposit")
        self.assertIsNone(records[0]["pricing_source_kind"])
        self.assertEqual(records[1]["external_id"], "ledger-out")
        self.assertEqual(records[1]["kind"], "withdrawal")
        self.assertEqual(records[1]["fee_msat"], 1000000)

    def test_binance_supplemental_csv_full_imports_exchange_evidence_wallet(self):
        binance_csv = Path(self._tmp.name) / "binance-supplemental.csv"
        binance_csv.write_text(_BINANCE_SUPPLEMENTAL_CSV, encoding="utf-8")

        payload = self._cli(
            "wallets", "import-binance-supplemental",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--file", str(binance_csv),
        )
        data = payload["data"]
        self.assertEqual(payload["kind"], "wallets.import-binance-supplemental")
        self.assertEqual(data["mode"], "full")
        self.assertEqual(data["wallet"], "Binance")
        self.assertEqual(data["input_format"], "binance_supplemental_csv")
        self.assertEqual(data["binance_rows"], 1)
        self.assertEqual(data["imported"], 1)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Binance",
            "--order", "asc",
        )
        records = payload["data"]
        self.assertEqual(len(records), 1)
        buy = records[0]
        self.assertEqual(buy["kind"], "buy")
        self.assertEqual(buy["pricing_source_kind"], "exchange_execution")
        self.assertEqual(buy["pricing_provider"], "Binance")
        self.assertEqual(buy["pricing_method"], "binance_supplemental_csv")
        self.assertEqual(buy["fiat_value_exact"], "101.00")

    def test_z_pocketbitcoin_csv_enriches_existing_wallet_transaction(self):
        existing_csv = Path(self._tmp.name) / "pocket-existing-wallet.csv"
        existing_csv.write_text(_POCKETBITCOIN_EXISTING_CSV, encoding="utf-8")
        pocket_csv = Path(self._tmp.name) / "pocket-orders.csv"
        pocket_csv.write_text(_POCKETBITCOIN_CSV, encoding="utf-8")

        self._cli(
            "profiles", "create",
            "--workspace", "Buckets",
            "--fiat-currency", "EUR",
            "--tax-country", "generic",
            "PocketEUR",
        )
        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "PocketEUR",
            "--label", "Pocket Matched",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "PocketEUR",
            "--wallet", "Pocket Matched",
            "--file", str(existing_csv),
        )

        payload = self._cli(
            "wallets", "import-pocket",
            "--workspace", "Buckets",
            "--profile", "PocketEUR",
            "--wallet", "Pocket Matched",
            "--file", str(pocket_csv),
        )
        self.assertEqual(payload["kind"], "wallets.import-pocket")
        self.assertEqual(payload["data"]["input_format"], "pocketbitcoin_csv")
        self.assertEqual(payload["data"]["pocketbitcoin_rows"], 1)
        self.assertEqual(payload["data"]["matched"], 1)
        self.assertEqual(payload["data"]["updated"], 1)
        self.assertEqual(payload["data"]["skipped"], 1)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "PocketEUR",
            "--wallet", "Pocket Matched",
        )
        buy = payload["data"][0]
        self.assertEqual(buy["external_id"], "pocket-wallet-tx")
        self.assertEqual(buy["kind"], "buy")
        self.assertEqual(buy["fee_msat"], 0)
        self.assertEqual(buy["pricing_source_kind"], "exchange_execution")
        self.assertEqual(buy["pricing_provider"], "Pocket Bitcoin")
        self.assertEqual(buy["pricing_method"], "pocketbitcoin_csv")
        self.assertEqual(buy["pricing_pair"], "BTC-EUR")
        self.assertEqual(buy["pricing_external_ref"], "REF000001")
        self.assertEqual(buy["fiat_value_exact"], "50.00000000")
        self.assertEqual(buy["fiat_rate_exact"], "21586.90000000")

    def test_z_pocketbitcoin_csv_full_import_flags_wallet_gap_rows(self):
        existing_csv = Path(self._tmp.name) / "pocket-full-existing-wallet.csv"
        existing_csv.write_text(_POCKETBITCOIN_EXISTING_CSV, encoding="utf-8")
        pocket_csv = Path(self._tmp.name) / "pocket-full-orders.csv"
        pocket_csv.write_text(_POCKETBITCOIN_CSV, encoding="utf-8")

        self._cli(
            "profiles", "create",
            "--workspace", "Buckets",
            "--fiat-currency", "EUR",
            "--tax-country", "generic",
            "PocketFullEUR",
        )

        payload = self._cli(
            "wallets", "import-pocket",
            "--workspace", "Buckets",
            "--profile", "PocketFullEUR",
            "--file", str(pocket_csv),
            "--mode", "full",
        )
        data = payload["data"]
        self.assertEqual(data["scope"], "book")
        self.assertEqual(data["mode"], "full")
        self.assertEqual(data["wallet"], "Pocket Bitcoin")
        self.assertEqual(data["pocketbitcoin_rows"], 1)
        self.assertEqual(data["imported"], 1)
        self.assertEqual(data["matched"], 0)
        self.assertEqual(data["unmatched"], 1)
        self.assertEqual(data["excluded"], 1)
        self.assertEqual(data["inserted_records"][0]["status"], "unmatched")

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "PocketFullEUR",
            "--wallet", "Pocket Bitcoin",
        )
        buy = payload["data"][0]
        self.assertTrue(buy["excluded"])
        self.assertEqual(buy["amount_msat"], 228101000)
        self.assertEqual(buy["fee_msat"], 46000)
        self.assertEqual(buy["pricing_provider"], "Pocket Bitcoin")
        self.assertEqual(buy["pricing_external_ref"], "REF000001")
        self.assertIn("pocketbitcoin-wallet-gap", {tag["code"] for tag in buy["tags"]})

        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "PocketFullEUR",
            "--label", "Pocket Real Wallet",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "PocketFullEUR",
            "--wallet", "Pocket Real Wallet",
            "--file", str(existing_csv),
        )

        payload = self._cli(
            "wallets", "import-pocket",
            "--workspace", "Buckets",
            "--profile", "PocketFullEUR",
            "--file", str(pocket_csv),
        )
        self.assertEqual(payload["data"]["matched"], 1)
        self.assertEqual(payload["data"]["updated"], 1)
        self.assertEqual(payload["data"].get("skipped_ambiguous", 0), 0)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Buckets",
            "--profile", "PocketFullEUR",
            "--wallet", "Pocket Real Wallet",
        )
        real_buy = payload["data"][0]
        self.assertEqual(real_buy["external_id"], "pocket-wallet-tx")
        self.assertEqual(real_buy["pricing_provider"], "Pocket Bitcoin")
        self.assertFalse(real_buy["excluded"])

    def test_loans_collateral_lock_is_not_a_disposal_end_to_end(self):
        # Drives the whole stack via the CLI: a collateral lock leg must suppress
        # the outbound disposal so the encumbered BTC still shows in the report.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "state" / "kassiber"
            root.mkdir(parents=True)
            csv_path = Path(tmp) / "wallet.csv"
            csv_path.write_text(
                "date,txid,direction,asset,amount,fee,fiat_rate,description,kind\n"
                "2026-01-01T10:00:00Z,buy-1,inbound,BTC,1.00000000,0,60000,Buy,buy\n"
                "2026-02-01T10:00:00Z,lock-1,outbound,BTC,1.00000000,0,65000,Lock to escrow,withdrawal\n",
                encoding="utf-8",
            )

            def run(*args):
                payload, code = _run(root, *args)
                self.assertEqual(code, 0, f"{args} -> {json.dumps(payload)[:300]}")
                return payload

            run("init")
            run("workspaces", "create", "Main")
            run("profiles", "create", "--workspace", "Main", "--fiat-currency", "USD", "--tax-country", "generic", "Default")
            run("wallets", "create", "--workspace", "Main", "--profile", "Default", "--label", "W1", "--kind", "custom")
            run("wallets", "import-csv", "--workspace", "Main", "--profile", "Default", "--wallet", "W1", "--file", str(csv_path))

            def btc_held():
                run("journals", "process", "--workspace", "Main", "--profile", "Default")
                rows = run("reports", "portfolio-summary", "--workspace", "Main", "--profile", "Default")["data"]
                return sum(float(r["quantity"]) for r in rows if r["asset"] == "BTC")

            # Baseline: the outbound is booked as a disposal, so no BTC remains.
            self.assertAlmostEqual(btc_held(), 0.0, places=8)

            mark = run(
                "loans", "mark", "--workspace", "Main", "--profile", "Default",
                "--txid", "lock-1", "--as", "collateral",
            )["data"]
            self.assertEqual(mark["role"], "collateral_lock")

            # With the mark, the disposal is suppressed: the BTC is still held
            # (encumbered), not sold.
            self.assertAlmostEqual(btc_held(), 1.0, places=8)

            # The lock has no offsetting release, so it surfaces as an open lock.
            listing = run("loans", "list", "--workspace", "Main", "--profile", "Default")["data"]
            self.assertEqual(len(listing["open_locks"]), 1)
            self.assertEqual(listing["open_locks"][0]["role"], "collateral_lock")

            # Liquidation path: removing the mark reverts the outbound to the
            # disposal it really was, so the BTC leaves the book again.
            run("loans", "unmark", "--workspace", "Main", "--profile", "Default", "--txid", "lock-1")
            self.assertAlmostEqual(btc_held(), 0.0, places=8)

            # Handler-level error: marking a missing transaction.
            payload, code = _run(
                root, "loans", "mark", "--workspace", "Main", "--profile", "Default",
                "--txid", "does-not-exist", "--as", "collateral",
            )
            self.assertNotEqual(code, 0)
            self.assertEqual(payload.get("kind"), "error")
            self.assertEqual(payload["error"]["code"], "not_found")

    def test_loans_principal_received_and_repaid_are_not_tax_events(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "state" / "kassiber"
            root.mkdir(parents=True)
            csv_path = Path(tmp) / "wallet.csv"
            csv_path.write_text(
                "date,txid,direction,asset,amount,fee,fiat_rate,description,kind\n"
                "2026-01-01T10:00:00Z,principal-in,inbound,BTC,1.00000000,0,60000,Loan principal received,deposit\n"
                "2026-02-01T10:00:00Z,principal-out,outbound,BTC,1.00000000,0,65000,Loan principal repaid,withdrawal\n",
                encoding="utf-8",
            )

            def run(*args):
                payload, code = _run(root, *args)
                self.assertEqual(code, 0, f"{args} -> {json.dumps(payload)[:300]}")
                return payload

            run("init")
            run("workspaces", "create", "Main")
            run(
                "profiles", "create", "--workspace", "Main",
                "--fiat-currency", "USD", "--tax-country", "generic", "Default",
            )
            run(
                "wallets", "create", "--workspace", "Main", "--profile", "Default",
                "--label", "W1", "--kind", "custom",
            )
            run(
                "wallets", "import-csv", "--workspace", "Main", "--profile", "Default",
                "--wallet", "W1", "--file", str(csv_path),
            )

            mark_in = run(
                "loans", "mark", "--workspace", "Main", "--profile", "Default",
                "--txid", "principal-in", "--as", "principal-received",
            )["data"]
            mark_out = run(
                "loans", "mark", "--workspace", "Main", "--profile", "Default",
                "--txid", "principal-out", "--as", "principal-repaid",
            )["data"]
            self.assertEqual(mark_in["role"], "loan_principal_received")
            self.assertEqual(mark_out["role"], "loan_principal_repaid")
            linked = run(
                "loans", "link", "--workspace", "Main", "--profile", "Default",
                "--txid", "principal-in", "--txid", "principal-out", "--loan-id", "loan-test",
            )["data"]
            self.assertEqual(linked["loan_id"], "loan-test")
            self.assertEqual(len(linked["transaction_ids"]), 2)
            listing = run("loans", "list", "--workspace", "Main", "--profile", "Default")["data"]
            self.assertEqual({mark["loan_id"] for mark in listing["marks"]}, {"loan-test"})

            run("journals", "process", "--workspace", "Main", "--profile", "Default")
            gains = run(
                "reports", "capital-gains", "--workspace", "Main", "--profile", "Default",
            )["data"]
            self.assertEqual(gains, [])


_GENERIC_LEDGER_CSV = """Type,Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Fee Amount,Fee Asset,Fiat Value,Counterparty,Note,Tx-ID
Buy,2026-01-15,0.05000000,BTC,3000.00,EUR,3.50,EUR,,Coinfinity,First stack,ledger-buy-1
Sell,2026-02-10,2200.00,EUR,0.03000000,BTC,1.00,EUR,,Kraken,Took some profit,ledger-sell-1
Mining,2026-03-10,0.00050000,BTC,,,,,32.50,Solo pool,Block reward,ledger-mining-1
Income,2026-03-20,250000,SATS,,,,,160.00,Freelance,Invoice in sats,ledger-income-1
Withdrawal,2026-04-01,,,0.02000000,BTC,0.00002000,BTC,,,Moved to cold storage,ledger-withdrawal-1
Gift sent,2026-05-01,,,0.00100000,BTC,,,,,Birthday gift,ledger-gift-1
"""

_GENERIC_LEDGER_BAD_TYPE_CSV = """Type,Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Fee Amount,Fee Asset,Fiat Value,Counterparty,Note,Tx-ID
Margin Trade,2026-01-15,0.05000000,BTC,3000.00,EUR,,,,,,bad-1
"""


class GenericLedgerImportTest(unittest.TestCase):
    """Generic (manual) ledger template generation + .xlsx/CSV import."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-generic-ledger-")
        self.data_root = Path(self._tmp.name) / "data"
        self._cli("init")
        self._cli("workspaces", "create", "Manual")
        self._cli(
            "profiles", "create",
            "--workspace", "Manual",
            "--fiat-currency", "EUR",
            "--tax-country", "generic",
            "Book",
        )
        self._cli(
            "wallets", "create",
            "--workspace", "Manual",
            "--profile", "Book",
            "--label", "Manual Ledger",
            "--kind", "custom",
        )

    def tearDown(self):
        self._tmp.cleanup()

    def _cli(self, *args):
        payload, code = _run(self.data_root, *args)
        if code != 0:
            self.fail(
                f"CLI exited {code} for {args!r}; envelope: {json.dumps(payload)[:400]}"
            )
        self.assertEqual(payload.get("schema_version"), 1)
        self.assertIn("data", payload)
        return payload

    def _import(self, file_path):
        return self._cli(
            "wallets", "import-ledger",
            "--workspace", "Manual",
            "--profile", "Book",
            "--wallet", "Manual Ledger",
            "--file", str(file_path),
        )

    def _records_by_external_id(self):
        payload = self._cli(
            "transactions", "list",
            "--workspace", "Manual",
            "--profile", "Book",
            "--wallet", "Manual Ledger",
        )
        return {row["external_id"]: row for row in payload["data"]}

    def test_template_generates_both_formats_without_a_database(self):
        # ledger-template is a no-DB writer; it must work before any book exists.
        for suffix in ("xlsx", "csv"):
            target = Path(self._tmp.name) / f"template.{suffix}"
            payload = self._cli("wallets", "ledger-template", "--file", str(target))
            self.assertEqual(payload["kind"], "wallets.ledger-template")
            self.assertEqual(payload["data"]["format"], suffix)
            self.assertTrue(target.exists())
            self.assertIn("Type", payload["data"]["columns"])

    def test_xlsx_template_round_trips_through_import(self):
        target = Path(self._tmp.name) / "round-trip.xlsx"
        self._cli("wallets", "ledger-template", "--file", str(target))
        payload = self._import(target)
        # The bundled example rows all import cleanly.
        self.assertEqual(payload["data"]["input_format"], "generic_ledger")
        self.assertEqual(payload["data"]["imported"], 7)

    def test_dry_run_previews_without_importing(self):
        template = Path(self._tmp.name) / "preview.csv"
        self._cli("wallets", "ledger-template", "--file", str(template))
        payload = self._cli(
            "wallets", "import-ledger",
            "--workspace", "Manual", "--profile", "Book", "--wallet", "Manual Ledger",
            "--file", str(template), "--dry-run",
        )
        self.assertEqual(payload["kind"], "wallets.import-ledger")
        self.assertGreater(payload["data"]["mapped"], 0)
        self.assertEqual(payload["data"]["errors"], 0)
        self.assertTrue(payload["data"]["preview"])
        self.assertEqual(self._records_by_external_id(), {})  # nothing persisted

        # A bad row is collected as a row-numbered problem, not aborted, and the
        # whole file still previews (the real importer stops at the first error).
        bad = Path(self._tmp.name) / "bad.csv"
        bad.write_text(
            "Date,Type,Received Asset,Received Amount,Sent Asset,Sent Amount,Fee Amount\n"
            "2026-01-15,Buy,BTC,0.5,EUR,20000,0\n"
            "2026-02-01,Frobnicate,BTC,0.1,,,0\n",
            encoding="utf-8",
        )
        bad_payload = self._cli(
            "wallets", "import-ledger",
            "--workspace", "Manual", "--profile", "Book", "--wallet", "Manual Ledger",
            "--file", str(bad), "--dry-run",
        )
        self.assertEqual(bad_payload["data"]["mapped"], 1)
        self.assertEqual(bad_payload["data"]["errors"], 1)
        self.assertEqual(bad_payload["data"]["problems"][0]["row"], 2)
        self.assertEqual(self._records_by_external_id(), {})

    def test_byo_columns_are_auto_detected(self):
        # An arbitrary export (no Type column; sent/received BTC + a fiat price)
        # imports by auto-detecting the columns onto the ledger shape.
        byo = Path(self._tmp.name) / "byo.csv"
        byo.write_text(
            "Date,Received BTC,Sent BTC,Currency,Price,Note\n"
            "2026-01-15,0.5,,EUR,40000,Bought\n"
            "2026-01-20,,0.1,EUR,42000,Sold\n",
            encoding="utf-8",
        )
        preview = self._cli(
            "wallets", "import-ledger",
            "--workspace", "Manual", "--profile", "Book", "--wallet", "Manual Ledger",
            "--file", str(byo), "--dry-run",
        )
        self.assertTrue(preview["data"]["confident"])
        self.assertEqual(preview["data"]["mapped"], 2)
        fields = {d["field"] for d in preview["data"]["detected"]}
        self.assertIn("received", fields)
        self.assertIn("sent", fields)

        imported = self._import(byo)
        self.assertEqual(imported["data"]["imported"], 2)
        listed = self._cli(
            "transactions", "list",
            "--workspace", "Manual", "--profile", "Book", "--wallet", "Manual Ledger",
        )["data"]
        self.assertEqual(len(listed), 2)
        # The fiat price became exact execution pricing through #244's normalizer.
        self.assertTrue(all(row.get("fiat_value") for row in listed))

    def test_byo_asset_suffixed_cash_legs_keep_trade_kind_and_pricing(self):
        buy_file = Path(self._tmp.name) / "byo-cash-leg-buy.csv"
        buy_file.write_text(
            "Date,Received BTC,Sent EUR,Note,Tx-ID\n"
            "2026-01-15,0.5,20000,Bought,byo-buy-1\n",
            encoding="utf-8",
        )
        sell_file = Path(self._tmp.name) / "byo-cash-leg-sell.csv"
        sell_file.write_text(
            "Date,Received EUR,Sent BTC,Note,Tx-ID\n"
            "2026-01-20,4200,0.1,Sold,byo-sell-1\n",
            encoding="utf-8",
        )
        preview = self._cli(
            "wallets", "import-ledger",
            "--workspace", "Manual", "--profile", "Book", "--wallet", "Manual Ledger",
            "--file", str(buy_file), "--dry-run",
        )
        self.assertTrue(preview["data"]["confident"])
        self.assertEqual(preview["data"]["mapped"], 1)
        detected = {d["column"] for d in preview["data"]["detected"]}
        self.assertIn("Received BTC", detected)
        self.assertIn("Sent EUR", detected)

        self._import(buy_file)
        self._import(sell_file)
        rows = self._records_by_external_id()
        buy = rows["byo-buy-1"]
        self.assertEqual(buy["kind"], "buy")
        self.assertEqual(buy["direction"], "inbound")
        self.assertEqual(buy["fiat_value_exact"], "20000")
        self.assertEqual(buy["pricing_source_kind"], "exchange_execution")
        self.assertEqual(buy["pricing_pair"], "BTC-EUR")
        sell = rows["byo-sell-1"]
        self.assertEqual(sell["kind"], "sell")
        self.assertEqual(sell["direction"], "outbound")
        self.assertEqual(sell["fiat_value_exact"], "4200")
        self.assertEqual(sell["pricing_source_kind"], "exchange_execution")
        self.assertEqual(sell["pricing_pair"], "BTC-EUR")

    def test_byo_amount_asset_columns_are_not_forced_to_crypto_to_crypto(self):
        byo = Path(self._tmp.name) / "byo-amount-assets.csv"
        byo.write_text(
            "Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Note,Tx-ID\n"
            "2026-01-15,0.5,BTC,20000,EUR,Bought,byo-asset-buy-1\n",
            encoding="utf-8",
        )
        preview = self._cli(
            "wallets", "import-ledger",
            "--workspace", "Manual", "--profile", "Book", "--wallet", "Manual Ledger",
            "--file", str(byo), "--dry-run",
        )
        self.assertTrue(preview["data"]["confident"])
        self.assertEqual(preview["data"]["mapped"], 1)
        self.assertEqual(preview["data"]["errors"], 0)

        self._import(byo)
        row = self._records_by_external_id()["byo-asset-buy-1"]
        self.assertEqual(row["kind"], "buy")
        self.assertEqual(row["fiat_value_exact"], "20000")
        self.assertEqual(row["pricing_pair"], "BTC-EUR")

    def test_byo_unrecognized_columns_are_steered_not_imported(self):
        junk = Path(self._tmp.name) / "junk.csv"
        junk.write_text("alpha,beta,gamma\n1,2,3\n", encoding="utf-8")
        preview = self._cli(
            "wallets", "import-ledger",
            "--workspace", "Manual", "--profile", "Book", "--wallet", "Manual Ledger",
            "--file", str(junk), "--dry-run",
        )
        self.assertFalse(preview["data"]["confident"])
        payload, code = _run(
            self.data_root, "wallets", "import-ledger",
            "--workspace", "Manual", "--profile", "Book", "--wallet", "Manual Ledger",
            "--file", str(junk),
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload["error"]["code"], "ledger_unrecognized")

    def test_csv_import_maps_kinds_amounts_and_pricing(self):
        ledger = Path(self._tmp.name) / "ledger.csv"
        ledger.write_text(_GENERIC_LEDGER_CSV, encoding="utf-8")
        payload = self._import(ledger)
        self.assertEqual(payload["data"]["imported"], 6)
        self.assertEqual(payload["data"]["skipped"], 0)

        rows = self._records_by_external_id()
        # Buy: exact exchange execution, fiat fee folded into cost basis.
        buy = rows["ledger-buy-1"]
        self.assertEqual(buy["direction"], "inbound")
        self.assertEqual(buy["kind"], "buy")
        self.assertEqual(buy["amount_msat"], 5_000_000_000)
        self.assertEqual(buy["pricing_source_kind"], "exchange_execution")
        self.assertEqual(buy["pricing_pair"], "BTC-EUR")
        self.assertEqual(buy["fiat_value_exact"], "3003.50")
        # Sell: fiat fee netted out of proceeds.
        sell = rows["ledger-sell-1"]
        self.assertEqual(sell["direction"], "outbound")
        self.assertEqual(sell["kind"], "sell")
        self.assertEqual(sell["fiat_value_exact"], "2199.00")
        # SATS amount converts to BTC; income kept as an earn kind.
        income = rows["ledger-income-1"]
        self.assertEqual(income["kind"], "income")
        self.assertEqual(income["amount_msat"], 250_000_000)
        # Withdrawal carries the on-chain fee on the Bitcoin leg.
        withdrawal = rows["ledger-withdrawal-1"]
        self.assertEqual(withdrawal["kind"], "withdrawal")
        self.assertEqual(withdrawal["fee_msat"], 2_000_000)
        # Gift sent stays a non-sale disposal kind for review.
        self.assertEqual(rows["ledger-gift-1"]["kind"], "gift")

    def test_sats_leg_blank_fee_asset_stays_in_sats(self):
        # Regression: a blank Fee Asset on a SATS-denominated leg must read the
        # fee in sats (the leg's unit), not default to whole BTC. A 500-sat fee
        # is 500_000 msat, not 500 BTC (5e13 msat).
        ledger = Path(self._tmp.name) / "sats-fee.csv"
        ledger.write_text(
            "Type,Date,Sent Amount,Sent Asset,Fee Amount,Tx-ID\n"
            "Withdrawal,2026-04-01,250000,SATS,500,sats-fee-1\n",
            encoding="utf-8",
        )
        self._import(ledger)
        row = self._records_by_external_id()["sats-fee-1"]
        self.assertEqual(row["amount_msat"], 250_000_000)  # 250k sats = 0.0025 BTC
        self.assertEqual(row["fee_msat"], 500_000)  # 500 sats, not 500 BTC

    def test_csv_import_preserves_swap_linkage_columns(self):
        ledger = Path(self._tmp.name) / "swap-linkage.csv"
        payment_hash = "AA" * 32
        refund_funding_txid = "bb" * 32
        ledger.write_text(
            "Type,Date,Sent Amount,Sent Asset,Tx-ID,Payment Hash,Payment Hash Source,Swap Refund Funding Tx-ID\n"
            f"Withdrawal,2026-04-01,0.00100000,LBTC,boltz-lockup-1,{payment_hash},boltz-regtest,{refund_funding_txid}\n",
            encoding="utf-8",
        )
        self._import(ledger)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        try:
            row = conn.execute(
                """
                SELECT asset, payment_hash, payment_hash_source, swap_refund_funding_txid
                FROM transactions
                WHERE external_id = 'boltz-lockup-1'
                """
            ).fetchone()
        finally:
            conn.close()
        self.assertIsNotNone(row)
        self.assertEqual(row["asset"], "LBTC")
        self.assertEqual(row["payment_hash"], payment_hash.lower())
        self.assertEqual(row["payment_hash_source"], "boltz-regtest")
        self.assertEqual(row["swap_refund_funding_txid"], refund_funding_txid)

    def test_reimport_is_idempotent(self):
        ledger = Path(self._tmp.name) / "ledger.csv"
        ledger.write_text(_GENERIC_LEDGER_CSV, encoding="utf-8")
        self._import(ledger)
        again = self._import(ledger)
        self.assertEqual(again["data"]["imported"], 0)
        self.assertEqual(again["data"]["skipped"], 6)

    def test_unknown_type_aborts_with_a_validation_error(self):
        ledger = Path(self._tmp.name) / "bad.csv"
        ledger.write_text(_GENERIC_LEDGER_BAD_TYPE_CSV, encoding="utf-8")
        payload, code = _run(
            self.data_root,
            "wallets", "import-ledger",
            "--workspace", "Manual",
            "--profile", "Book",
            "--wallet", "Manual Ledger",
            "--file", str(ledger),
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("unknown Type", payload["error"]["message"])

    def test_european_decimal_comma_and_date_are_parsed(self):
        # German/Austrian hand entry: comma decimals, dot thousands, DD.MM.YYYY.
        ledger = Path(self._tmp.name) / "de-locale.csv"
        ledger.write_text(
            "Type;Date;Received Amount;Received Asset;Sent Amount;Sent Asset;Fee Amount;Fee Asset;Tx-ID\n"
            "Buy;15.01.2026;0,05000000;BTC;3.000,00;EUR;3,50;EUR;de-buy-1\n",
            encoding="utf-8",
        )
        self._import(ledger)
        row = self._records_by_external_id()["de-buy-1"]
        self.assertEqual(row["amount_msat"], 5_000_000_000)  # 0.05 BTC, not 5,000,000
        self.assertEqual(row["occurred_at"], "2026-01-15T00:00:00Z")
        self.assertEqual(row["fiat_value_exact"], "3003.50")  # 3000,00 + 3,50 fee

    def test_header_aliases_are_accepted(self):
        ledger = Path(self._tmp.name) / "aliases.csv"
        ledger.write_text(
            "Transaction Type,Timestamp,Received Amount,Received Cur.,Sent Amount,Sent Cur.,Tx-ID\n"
            "Buy,2026-01-15,0.05,BTC,3000,EUR,alias-buy-1\n",
            encoding="utf-8",
        )
        payload = self._import(ledger)
        self.assertEqual(payload["data"]["imported"], 1)
        self.assertEqual(self._records_by_external_id()["alias-buy-1"]["kind"], "buy")

    def test_no_txid_rows_reimport_without_duplicates(self):
        ledger = Path(self._tmp.name) / "no-txid.csv"
        ledger.write_text(
            "Type,Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Fiat Value\n"
            "Mining,2026-03-10,0.00050000,BTC,,,32.50\n"
            "Spend,2026-04-15,,,0.00100000,BTC,65.00\n",
            encoding="utf-8",
        )
        first = self._import(ledger)
        self.assertEqual(first["data"]["imported"], 2)
        # Re-importing (even with the rows reordered) must not double-book, since
        # identity falls back to the economic fingerprint, not the row position.
        ledger.write_text(
            "Type,Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Fiat Value\n"
            "Spend,2026-04-15,,,0.00100000,BTC,65.00\n"
            "Mining,2026-03-10,0.00050000,BTC,,,32.50\n",
            encoding="utf-8",
        )
        again = self._import(ledger)
        self.assertEqual(again["data"]["imported"], 0)
        self.assertEqual(again["data"]["skipped"], 2)

    def test_mismatched_fiat_fee_currency_is_rejected(self):
        ledger = Path(self._tmp.name) / "fee-mismatch.csv"
        ledger.write_text(
            "Type,Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Fee Amount,Fee Asset,Tx-ID\n"
            "Buy,2026-01-15,0.05,BTC,3000,EUR,5,USD,fee-bad-1\n",
            encoding="utf-8",
        )
        payload, code = _run(
            self.data_root,
            "wallets", "import-ledger",
            "--workspace", "Manual", "--profile", "Book", "--wallet", "Manual Ledger",
            "--file", str(ledger),
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("does not match", payload["error"]["message"])


if __name__ == "__main__":
    unittest.main()
