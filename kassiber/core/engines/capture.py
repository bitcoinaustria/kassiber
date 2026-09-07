"""Cutoff-safe, exact capture of the existing RP2 computation seam.

The explicit prefix bounds retained replay inputs and evidence. Native carry
may also need unexecuted future receipt references, which must not enter the
cutoff inventory. Acquisition carry and report-cutoff position basis use distinct
RP2 contracts; this adapter never implements selection or pool arithmetic.
"""
from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal, ROUND_HALF_EVEN, localcontext
from hashlib import sha256
import inspect
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from ...errors import AppError
from ...msat import MSAT_PER_BTC
from ..custody_tax_projection import FinalizedTaxProjection
from .base import TaxEngineCalculationResult, TaxEngineLedgerInputs


def capturing_accounting_engine(engine, retained):
    """Observe the pinned public RP2 selection contract; never select a lot.

    RP2 TaxEngineCursor constructs ``accounting_engine.__class__(methods)``.
    A per-run subclass closure therefore survives that supported construction
    while sharing only this run's append-only sink. RP2's report cursor replays
    the same asset's ordered prefix on a fresh engine: retain that prefix once,
    and reject changed identities, ordering or values instead of hiding drift.
    No monkeypatch or global state, and the delegated result is unchanged. The true event unit
    basis is intentionally distinct from the display zero-gain override.
    """
    contract = ('self', 'taxable_event', 'acquired_lot', 'taxable_event_amount', 'acquired_lot_amount')
    if tuple(inspect.signature(type(engine).get_acquired_lot_for_taxable_event).parameters) != contract:
        raise AppError('Installed RP2 execution capture contract changed', code='accounting_calculation_dependency')

    selections, identities, owners = {}, set(), {}

    class CapturingAccountingEngine(type(engine)):
        def __init__(self, years_2_methods):
            super().__init__(years_2_methods)
            self._capture_ordinals = {}

        def get_acquired_lot_for_taxable_event(self, taxable_event, acquired_lot, taxable_event_amount, acquired_lot_amount):
            result = super().get_acquired_lot_for_taxable_event(taxable_event, acquired_lot, taxable_event_amount, acquired_lot_amount)
            required = ('taxable_event','acquired_lot','taxable_event_amount','acquired_lot_amount',
                        'unit_cost_basis_override','taxable_event_unit_cost_basis')
            if not all(hasattr(result, field) for field in required):
                raise AppError('Installed RP2 execution result contract changed', code='accounting_calculation_dependency')
            quantity = min(result.taxable_event_amount, result.acquired_lot_amount)
            true_unit = result.taxable_event_unit_cost_basis
            if true_unit is None:
                true_unit = result.unit_cost_basis_override
            if true_unit is None:
                true_unit = result.acquired_lot.fiat_in_with_fee / result.acquired_lot.crypto_in
            record = dict(asset=result.taxable_event.asset, event_id=result.taxable_event.unique_id,
                lot_id=result.acquired_lot.unique_id, quantity_msat=_msat(quantity),
                unit_basis_exact=_decimal(true_unit), basis_exact=_decimal(quantity * true_unit),
                display_unit_basis_override_exact=None if result.unit_cost_basis_override is None else _decimal(result.unit_cost_basis_override),
                carried_unit_basis_exact=None if result.taxable_event_unit_cost_basis is None else _decimal(result.taxable_event_unit_cost_basis))
            asset = result.taxable_event.asset
            identity = (asset, result.taxable_event.internal_id, result.acquired_lot.internal_id)
            sequence = selections.setdefault(asset, [])
            owner = owners.setdefault(asset, self)
            ordinal = self._capture_ordinals.get(asset, 0)
            if ordinal < len(sequence):
                if sequence[ordinal] != (identity, record):
                    raise AppError('RP2 report replay changed its execution prefix', code='accounting_calculation_dependency')
            else:
                if owner is not self:
                    raise AppError('RP2 report replay extended its execution prefix', code='accounting_calculation_dependency')
                # A taxable event can consume several lots, and a lot can serve
                # several events, but each event/lot pair is selected only once.
                if identity in identities:
                    raise AppError('RP2 repeated a selection within execution', code='accounting_calculation_dependency')
                identities.add(identity)
                sequence.append((identity, record))
                retained.append(record)
            self._capture_ordinals[asset] = ordinal + 1
            return result

    return CapturingAccountingEngine(engine.years_2_methods)


def _instant(value):
    try:
        parsed = datetime.fromisoformat(str(value).replace('Z', '+00:00'))
        if parsed.tzinfo is None:
            raise ValueError()
        return parsed.astimezone(timezone.utc)
    except (TypeError, ValueError) as exc:
        raise AppError('Calculation sources require timezone-aware timestamps', code='accounting_calculation_time') from exc


def _msat(value):
    amount = Decimal(value) * MSAT_PER_BTC
    if amount != amount.to_integral_value() or abs(amount) > 2**63 - 1:
        raise AppError('Calculation quantity is not an exact supported atomic amount', code='accounting_calculation_quantity')
    return int(amount)


def _decimal(value):
    return format(Decimal(value), 'f')


def _transaction(transaction, kind):
    result = dict(kind=kind, row=transaction.row, unique_id=transaction.unique_id,
                  timestamp=transaction.timestamp.isoformat(), asset=transaction.asset,
                  transaction_type=transaction.transaction_type.value,
                  spot_price_exact=_decimal(transaction.spot_price), notes=transaction.notes)
    text_fields = ('exchange','holder') if kind != 'INTRA' else ('from_exchange','from_holder','to_exchange','to_holder')
    amount_fields = {
        'IN': ('crypto_in','crypto_fee','fiat_in_no_fee','fiat_in_with_fee','fiat_fee'),
        'OUT': ('crypto_out_no_fee','crypto_fee','crypto_out_with_fee','fiat_out_no_fee','fiat_fee'),
        'INTRA': ('crypto_sent','crypto_received'),
    }[kind]
    result.update({field: getattr(transaction, field) for field in text_fields})
    result.update({field + '_exact': _decimal(getattr(transaction, field)) for field in amount_fields})
    return result


def _source_projection(projection):
    # Prepared transaction values below are the replay inputs. This allowlist
    # retains custody traceability without exporting wallet config/raw node data.
    keys = ('id','journal_transaction_id','wallet_id','asset','occurred_at','direction','amount','fee',
            'amount_includes_fee','custody_quantity_hash','custody_slice_start_msat','custody_slice_end_msat',
            'fiat_currency','fiat_rate_exact','fiat_value_exact','kind')
    return [dict((key, row[key]) for key in keys if key in row.keys()) for row in projection.rows]


def _bounded_inputs(inputs, cutoff):
    projection = inputs.finalized_tax_projection
    if not isinstance(projection, FinalizedTaxProjection):
        raise TypeError('Calculation capture requires a FinalizedTaxProjection')
    selected = {str(row['id']): row for row in projection.rows if _instant(row['occurred_at']) < cutoff}
    all_rows = {str(row['id']): row for row in projection.rows}
    blockers, retained_intra, retained_cross, relations = [], [], [], []
    wallet_refs = dict(inputs.wallet_refs_by_id)
    for pairs, retained in ((projection.intra_pairs, retained_intra), (projection.cross_asset_pairs, retained_cross)):
        for pair in pairs:
            out_id = str(pair['out']['id']) if pair.get('out') is not None else str(pair.get('out_id') or '')
            in_id = str(pair['in']['id']) if pair.get('in') is not None else str(pair.get('in_id') or '')
            if out_id in selected and in_id in selected:
                retained.append(pair)
            elif (pairs is projection.cross_asset_pairs and out_id in selected and in_id not in selected
                  and pair.get('policy') == 'carrying-value'
                  and {all_rows[out_id]['asset'], all_rows[in_id]['asset']} <= {'BTC', 'LBTC', 'L-BTC'}):
                # The original future receipt is a relation reference, NOT an
                # economic prefix event. RP2's native runner needs this object
                # to retain the dispatched basis; no future taxable event may
                # admit it into the destination pool. Capture filters this one
                # explicitly identified reference from holdings/acquisitions.
                selected[in_id] = all_rows[in_id]
                retained.append(pair)
                relations.append(dict(relation_id=str(pair.get('pair_id') or f'{out_id}:{in_id}'),
                    out_id=out_id, in_id=in_id, departed_at=all_rows[out_id]['occurred_at'],
                    arrives_at=all_rows[in_id]['occurred_at'], future_reference=True,
                    from_asset=all_rows[out_id]['asset'], to_asset=all_rows[in_id]['asset'],
                    from_wallet_id=all_rows[out_id]['wallet_id'], to_wallet_id=all_rows[in_id]['wallet_id'],
                    quantity_sent_msat=int(all_rows[out_id]['amount']),
                    quantity_received_msat=int(all_rows[in_id]['amount']),
                    dispatch_fee_msat=int(all_rows[out_id]['fee'] or 0)))
            elif pairs is projection.intra_pairs and out_id in selected and in_id not in selected:
                # A reviewed MOVE owns its quantity even while the destination
                # observation has not arrived. Execute only its dispatch into
                # an engine-local transit custody location, not the future
                # receipt. Original relation identities and dates remain data.
                outgoing, incoming = pair['out'], pair['in']
                identity = sha256(f'{out_id}:{in_id}'.encode()).hexdigest()
                transit_id = f'accounting-transit:{identity}'
                wallet_refs[transit_id] = dict(inputs.wallet_refs_by_id[outgoing['wallet_id']],
                    id=transit_id, label=f'Accounting transit {identity}')
                synthetic = dict(outgoing, id=transit_id, direction='inbound', fee=0,
                    wallet_id=transit_id, wallet_label=wallet_refs[transit_id]['label'],
                    description='Reviewed transfer in transit at calculation cutoff',
                    amount_includes_fee=False)
                # Do not import a future fee/valuation. Only principal observed
                # leaving now and its explicitly attributed dispatch fee apply.
                selected[transit_id] = synthetic
                retained.append(dict(pair, out=outgoing, **{'in': synthetic}))
                relations.append(dict(relation_id=str(pair.get('pair_id') or identity),
                    out_id=out_id, in_id=in_id, departed_at=outgoing['occurred_at'],
                    arrives_at=incoming['occurred_at'], transit_wallet_id=transit_id,
                    from_wallet_id=outgoing['wallet_id'], to_wallet_id=incoming['wallet_id'],
                    asset=outgoing['asset'], quantity_msat=int(outgoing['amount']),
                    dispatch_fee_msat=int(outgoing['fee'] or 0)))
            elif out_id in selected or in_id in selected:
                blockers.append(dict(code='accounting_cross_cutoff_relation', out_id=out_id, in_id=in_id,
                                     relation_id=str(pair.get('id') or pair.get('pair_id') or '')))
                selected.pop(out_id, None)
                selected.pop(in_id, None)
    # A group can have more than one relation: after one pair is excluded,
    # iteratively block every dependent pair rather than retaining a half group.
    changed = True
    while changed:
        changed = False
        for retained in (retained_intra, retained_cross):
            for pair in retained[:]:
                out_id = str(pair['out']['id']) if pair.get('out') is not None else str(pair.get('out_id') or '')
                in_id = str(pair['in']['id']) if pair.get('in') is not None else str(pair.get('in_id') or '')
                if out_id not in selected or in_id not in selected:
                    retained.remove(pair)
                    selected.pop(out_id, None)
                    selected.pop(in_id, None)
                    blockers.append(dict(code='accounting_cross_cutoff_relation', out_id=out_id, in_id=in_id,
                                         relation_id=str(pair.get('id') or pair.get('pair_id') or '')))
                    changed = True
    quarantines = tuple(item for item in projection.quarantines if not item.get('occurred_at') or _instant(item['occurred_at']) < cutoff)
    prefix = FinalizedTaxProjection(rows=tuple(selected.values()), intra_pairs=tuple(retained_intra),
                                    cross_asset_pairs=tuple(retained_cross), quarantines=quarantines,
                                    selected_move_ids=projection.selected_move_ids)
    return TaxEngineLedgerInputs(prefix, wallet_refs), blockers, relations


def _positions_without_future_references(prepared, computed, future_refs, profile):
    """Replay only observed inventory with RP2's already resolved native carry.

    Future receipt references are required for cross-asset validation/carry, not
    acquisitions at this cutoff. Input filtering is exact-instant upstream, so
    removing these explicit references also handles a cutoff within a UTC date.
    """
    from rp2.input_data import InputData
    from rp2.tax_engine import TaxEngineCursor
    from rp2.transaction_set import TransactionSet
    from .rp2 import _build_rp2_accounting_engine

    original = prepared.input_data
    configuration = original.unfiltered_in_transaction_set.configuration
    eligible = [item for item in original.unfiltered_in_transaction_set if item.unique_id not in future_refs]
    if not eligible:
        return {}
    acquisitions = TransactionSet(configuration, 'IN', prepared.asset)
    for item in eligible:
        acquisitions.add_entry(item)
    inputs = InputData(prepared.asset, acquisitions, original.unfiltered_out_transaction_set,
        original.unfiltered_intra_transaction_set,
        in_transaction_2_fiat_in_with_fee_override={item: computed.get_in_transaction_fiat_in_with_fee(item) for item in eligible})
    # An uncaptured engine prevents this report-only replay from creating
    # additional retained execution evidence. RP2 still owns every calculation.
    cursor = TaxEngineCursor(configuration, _build_rp2_accounting_engine(profile), inputs)
    while cursor.has_next():
        cursor.consume_next_taxable_event()
    return cursor.get_open_position_basis()


def _freeze(capture, prefix, result, blockers, relations, profile):
    prepared_rows, assets, source_map, overrides = [], [], {}, {}
    future_refs = {item['in_id']: item for item in relations if item.get('future_reference')}
    for row in prefix.finalized_tax_projection.rows:
        source_map[str(row['id'])] = {key: row[key] for key in ('journal_transaction_id','custody_quantity_hash',
            'custody_slice_start_msat','custody_slice_end_msat') if key in row.keys()}
    refs = {str(ref['label']): ref for ref in prefix.wallet_refs_by_id.values()}
    for normalized, prepared in capture.get('prepared', []):
        if prepared.input_data is None:
            continue
        for kind, transactions in (('IN', prepared.input_data.unfiltered_in_transaction_set),
                                   ('OUT', prepared.input_data.unfiltered_out_transaction_set),
                                   ('INTRA', prepared.input_data.unfiltered_intra_transaction_set)):
            prepared_rows.extend(_transaction(item, kind) for item in transactions)
        computed = capture['states'][prepared.asset].computed_data
        if computed is None:
            continue
        position_overrides = (
            _positions_without_future_references(prepared, computed, future_refs, profile)
            if any(item.unique_id in future_refs for item in prepared.input_data.unfiltered_in_transaction_set)
            else None
        )
        acquisitions, gains, positions, balances = [], [], [], []
        for item in computed.open_position_in_transaction_set:
            effective = computed.get_in_transaction_fiat_in_with_fee(item)
            if item.unique_id in future_refs:
                future_refs[item.unique_id]['basis_carried_exact'] = _decimal(effective)
                continue
            sold = computed.get_open_position_in_lot_sold_percentage(item)
            source_ids = [str(item.unique_id)]
            acquisitions.append(dict(event_id=f'{prepared.asset}:{item.unique_id}', source_ids=source_ids,
                rp2_unique_id=item.unique_id, quantity_msat=_msat(item.crypto_in),
                original_fiat_value_exact=_decimal(item.fiat_in_with_fee), effective_basis_exact=_decimal(effective)))
            overrides[f'{prepared.asset}:{item.unique_id}'] = _decimal(effective)
            actual = computed.get_in_transaction_actual_amount(item)
            retained_quantity = item.crypto_in if actual is None else actual
            remaining = Decimal(retained_quantity) * (Decimal(1) - Decimal(sold))
            position_basis = (computed.get_open_position_in_transaction_fiat_in_with_fee(item)
                              if position_overrides is None else position_overrides[item])
            basis = Decimal(position_basis) * remaining / Decimal(item.crypto_in)
            positions.append(dict(lot_id=item.unique_id, pool_id='global', quantity_msat=_msat(remaining), basis_exact=_decimal(basis)))
        for gain in computed.gain_loss_set:
            event, lot = gain.taxable_event, gain.acquired_lot
            gains.append(dict(row_id=f'{prepared.asset}:{gain.internal_id}', event_id=event.unique_id,
                lot_id=lot.unique_id if lot else None, quantity_msat=_msat(gain.crypto_amount),
                basis_exact=_decimal(gain.fiat_cost_basis), proceeds_exact=_decimal(gain.taxable_event_fiat_amount_with_fee_fraction),
                gain_exact=_decimal(gain.fiat_gain), unit_basis_override_exact=None if gain.unit_cost_basis_override is None else _decimal(gain.unit_cost_basis_override),
                transaction_type=event.transaction_type.value))
        for balance in computed.balance_set:
            reference_quantity = sum(item['quantity_received_msat'] for item in future_refs.values()
                if item['to_asset'] == prepared.asset and item['to_wallet_id'] == refs[balance.exchange]['id'])
            balances.append(dict(wallet_id=refs[balance.exchange]['id'], quantity_msat=_msat(balance.final_balance) - reference_quantity))
        # Same-asset MOVE changes custody, not the global basis pool. Original
        # prepared MOVE transactions retain fee quantity and exact provenance.
        assets.append(dict(asset=prepared.asset, acquisitions=acquisitions, gain_losses=gains,
                           open_positions=positions, custody_balances=balances, transfers=[]))
    for transaction in prepared_rows:
        if transaction['unique_id'] in future_refs:
            transaction['execution_role'] = 'unexecuted_receipt_reference'
    for pair in prefix.finalized_tax_projection.cross_asset_pairs:
        if pair.get('policy') != 'carrying-value':
            continue
        outgoing = next((item for item in prepared_rows if item['unique_id'] == pair['out_id'] and item['kind'] == 'OUT'), None)
        incoming = next((item for item in prepared_rows if item['unique_id'] == pair['in_id'] and item['kind'] == 'IN'), None)
        if outgoing is None or incoming is None:
            continue  # Existing engine quarantine is retained below.
        execution = [item for item in capture.get('execution', []) if item['asset'] == outgoing['asset'] and item['event_id'] == pair['out_id']]
        units = {item['carried_unit_basis_exact'] for item in execution if item['carried_unit_basis_exact'] is not None}
        if len(units) > 1:
            raise AppError('RP2 transfer execution returned inconsistent carried basis', code='accounting_calculation_dependency')
        carried = (Decimal(next(iter(units))) * Decimal(outgoing['crypto_out_no_fee_exact'])
                   if units else Decimal(incoming['fiat_in_with_fee_exact']))
        source_asset = next(item for item in assets if item['asset'] == outgoing['asset'])
        source_asset['transfers'].append(dict(event_id=pair['out_id'], from_asset=outgoing['asset'],
            to_asset=incoming['asset'], quantity_sent_msat=_msat(outgoing['crypto_out_no_fee_exact']),
            quantity_received_msat=_msat(incoming['crypto_in_exact']), fee_msat=_msat(outgoing['crypto_fee_exact']),
            basis_carried_exact=_decimal(carried)))
    for relation in future_refs.values():
        # Neu swap GainLoss deliberately sets its displayed cost basis equal
        # to market proceeds for zero gain. That is NOT the carrying value.
        # Only RP2's destination acquisition-basis override owns this number.
        if relation.get('basis_carried_exact') is None:
            blockers.append(dict(code='accounting_pending_basis_unreconciled', relation_id=relation['relation_id']))
        else:
            relation['basis_authority'] = 'rp2_destination_acquisition_override'
    for quarantine in result.quarantines:
        blockers.append({key: quarantine[key] for key in ('reason','transaction_id','occurred_at','asset') if key in quarantine})
    inputs = dict(finalized_projection=_source_projection(prefix.finalized_tax_projection),
                  prepared_transactions=prepared_rows, source_event_map=source_map, basis_overrides=overrides)
    inputs['cutoff_relations'] = relations
    inputs['execution_basis'] = capture.get('execution', [])
    inputs['custody_relations'] = [dict((key, pair[key]) for key in
        ('pair_id','out_id','in_id','out_asset','in_asset','policy','kind','component_id') if key in pair)
        for pair in prefix.finalized_tax_projection.cross_asset_pairs]
    inputs['custody_relations'].extend(dict(pair_id=pair.get('pair_id'), out_id=pair['out']['id'],
        in_id=pair['in']['id'], out_asset=pair['out']['asset'], in_asset=pair['in']['asset'], policy=pair.get('policy'))
        for pair in prefix.finalized_tax_projection.intra_pairs)
    inputs['same_asset_moves'] = [{key: item[key] for key in ('out_id','in_id','rp2_unique_id','asset',
        'crypto_sent_msat','crypto_received_msat','crypto_fee_msat','from_wallet_id','to_wallet_id','occurred_at') if key in item}
        for item in result.intra_audit]
    return inputs, assets, blockers


def capture_calculation(engine, inputs, *, cutoff_exclusive_utc, calculation_timezone):
    from .rp2 import _rp2_configuration

    cutoff = _instant(cutoff_exclusive_utc)
    try:
        ZoneInfo(calculation_timezone)
    except (TypeError, ZoneInfoNotFoundError) as exc:
        raise AppError('Calculation calendar needs a valid timezone', code='accounting_calculation_time') from exc
    prefix, blockers, relations = _bounded_inputs(inputs, cutoff)
    captured = {}
    with localcontext() as context:
        context.prec, context.rounding = 32, ROUND_HALF_EVEN
        if prefix.finalized_tax_projection.rows:
            labels = [str(ref['label']) for ref in prefix.wallet_refs_by_id.values()]
            names = {str(row['asset']) for row in prefix.finalized_tax_projection.rows}
            with _rp2_configuration(engine.profile, labels, names) as configuration:
                result = engine._build_finalized_ledger_state(prefix, configuration, calculation_capture=captured)
                retained_inputs, assets, blockers = _freeze(captured, prefix, result, blockers, relations, engine.profile)
        else:
            result = engine._build_finalized_ledger_state(prefix, None)
            retained_inputs, assets, blockers = _freeze(captured, prefix, result, blockers, relations, engine.profile)
    return TaxEngineCalculationResult(cutoff.isoformat().replace('+00:00', 'Z'), calculation_timezone,
                                       retained_inputs, assets, blockers)
