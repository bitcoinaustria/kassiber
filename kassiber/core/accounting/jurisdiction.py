"""Versioned, data-only form registry and exact working-paper arithmetic.

Only bundled, explicitly registered files are loaded. No path, network request,
plugin import, expression interpreter, or personal tax engine is accepted here.
Form preparation is distinct from tax liability determination and submission.
"""
from __future__ import annotations

from copy import deepcopy
import json
from pathlib import Path
from typing import Any, Mapping

from .ledger import digest
from ...errors import AppError

MAX_FORM_MINOR = 999999999999999
AT_PACK_ID = 'AT-K2-2025-v1'
TEST_PACK_ID = 'TEST-NEUTRAL-2025-v1'
FIELD_STATES = ('reviewed_input', 'not_applicable', 'derived', 'blocked')
_XML_PATHS = json.loads((Path(__file__).parent / 'jurisdiction_data' / 'at_k2_2025_xml_paths.json').read_text())


def _pairs(raw):
    return [item.split(':', 1) for item in raw.split('|') if item]


def _field(identifier, label, kind, group, form_id, form):
    paths = _XML_PATHS.get(form_id, {}).get(identifier, [])
    return dict(id=identifier, label=label, type=kind, group=group,
                source_id=form_id, kennzahl=identifier if identifier.isdigit() else None,
                xml_path=paths[0] if len(paths) == 1 else None,
                xml_path_variants=paths,
                pdf_field=form.get('pdf_fields', {}).get(identifier),
                negative_only=identifier in form.get('negative_fields', []),
                required=identifier in ('ENTITY_NAME', 'TAX_NUMBER', 'AUTHORITY'))


def _load_at():
    # Explicit resource name; callers cannot select paths or executable packs.
    data = json.loads((Path(__file__).parent / 'jurisdiction_data' / 'at_k2_2025.json').read_text())
    for form_id, form in data['forms'].items():
        fields = []
        for group, raw in form.get('money_groups', {}).items():
            fields.extend(_field(key, label, 'money', group, form_id, form) for key, label in _pairs(raw))
        for kind in ('extra_money', 'text', 'boolean', 'percent', 'date', 'integer'):
            fields.extend(_field(key, label, 'money' if kind == 'extra_money' else kind,
                                 'Ergänzende Angaben', form_id, form) for key, label in _pairs(form.get(kind, '')))
        for key, label, kind in (
            ('AUTHORITY', 'Finanzamt Österreich / Finanzamt für Großbetriebe', 'text'),
            ('TAX_NUMBER', 'Steuernummer', 'text'), ('ENTITY_NAME', 'Bezeichnung der Körperschaft', 'text'),
            ('REPRESENTATIVE', 'Steuerliche Vertretung', 'text'), ('PREPARED_ON', 'Datum der Vorbereitung', 'date'),
        ):
            fields.append(_field(key, label, kind, 'Identität', form_id, form))
        if len({f['id'] for f in fields}) != len(fields):
            raise RuntimeError(f'Duplicate field in bundled form {form_id}')
        form['fields'] = fields
    data['facts'] = [
        {'id': 'liability', 'label': 'Steuerpflicht', 'type': 'choice', 'choices': ['unlimited', 'limited', 'exempt'], 'required': True},
        {'id': 'section7_3', 'label': 'Körperschaft fällt unter §7 Abs3', 'type': 'boolean', 'required': True},
        {'id': 'entity_type', 'label': 'Rechtsform', 'type': 'choice', 'choices': ['association', 'foundation', 'public_body', 'other_corporation'], 'required': True},
        {'id': 'tax_scope_review', 'label': 'Geprüfte Steuerbefreiungen, Pflichten und Tätigkeitsbereiche', 'type': 'text', 'required': True},
        {'id': 'all_sources_reviewed', 'label': 'Alle Einkunftsquellen und Beilagen geprüft', 'type': 'boolean', 'required': True},
        {'id': 'capital_election', 'label': 'Mitveranlagung endbesteuerter Kapitalerträge', 'type': 'boolean', 'required': True},
        {'id': 'custodian_offsets_reviewed', 'label': 'Bank-/Verwahrer-Verlustausgleich und KESt geprüft', 'type': 'boolean', 'required': True},
        {'id': 'foreign_credits_reviewed', 'label': 'Ausländische Steueranrechnung und DBA geprüft', 'type': 'boolean', 'required': True},
        {'id': 'carryforwards_reviewed', 'label': 'Vorjahresbescheide und Vorträge vollständig geprüft', 'type': 'boolean', 'required': True},
        {'id': 'specialist_review', 'label': 'Prüfung besonderer Sachverhalte und Rechtsgrundlagen', 'type': 'text', 'required': True},
        {'id': 'group_parent', 'label': 'Gruppenträger §9', 'type': 'boolean', 'required': True},
        {'id': 'required_annexes', 'label': 'Erforderliche Beilagen', 'type': 'forms', 'required': True},
    ]
    data['test_only'] = False
    data['field_states'] = list(FIELD_STATES)
    data['max_form_minor'] = MAX_FORM_MINOR
    data['digest'] = digest(data)
    return data


_AT = _load_at()
_TEST = dict(pack_id=TEST_PACK_ID, country='TEST', tax_year=2025, version=1,
             currency='EUR', minor_unit_exponent=2, test_only=True,
             purpose='Synthetic jurisdiction contract tests only; never a filing form',
             sources=[], law_sources=[], source_resolutions=[], facts=[],
             field_states=list(FIELD_STATES), max_form_minor=MAX_FORM_MINOR,
             forms={'TEST': {'title': 'Synthetic neutral form', 'fields': [
                 dict(id='RESULT', label='Reviewed result', type='money', group='Result',
                      source_id='TEST', required=False, negative_only=False, kennzahl=None,
                      xml_path=None, pdf_field=None)]}})
_TEST['digest'] = digest(_TEST)


def list_packs(*, include_test=False):
    return [dict(pack_id=p['pack_id'], country=p['country'], tax_year=p['tax_year'],
                 version=p['version'], digest=p['digest'], test_only=p['test_only'], purpose=p['purpose'])
            for p in ([_AT, _TEST] if include_test else [_AT])]


def get_pack(pack_id, *, allow_test=False):
    if pack_id == AT_PACK_ID:
        return deepcopy(_AT)
    if pack_id == TEST_PACK_ID and allow_test:
        return deepcopy(_TEST)
    raise AppError('Unknown or unavailable jurisdiction pack', code='accounting_tax_pack_unavailable')


def fields_for(pack, form_id):
    if form_id not in pack['forms']:
        raise AppError('Form is not part of this jurisdiction pack', code='accounting_tax_validation')
    return {field['id']: field for field in pack['forms'][form_id]['fields']}


def round_ratio(value: int, numerator: int, denominator: int = 100):
    """Exact commercial rounding at the form-cent boundary, never binary floats."""
    sign = -1 if value < 0 else 1
    return sign * ((abs(value) * numerator * 2 + denominator) // (denominator * 2))


def derive_fields(form_id: str, values: Mapping[str, Any], *, not_applicable=()) -> dict[str, dict[str, Any]]:
    """Known arithmetic only; absence remains absence, not zero.

    Callers supply zero only for explicitly reviewed not-applicable fields.
    Results include replayable operand traces. No rule infers legal eligibility.
    """
    current = dict(values)
    output = {}

    def formula(target, terms, *, operation='sum', numerator=100):
        if any(type(current.get(key)) is not int for key, _ in terms):
            return
        operands = [{'field': key, 'value_minor': current[key], 'coefficient': coefficient} for key, coefficient in terms]
        result = sum(current[key] * coefficient for key, coefficient in terms)
        if operation == 'positive':
            result = max(0, result)
        elif operation == 'negative_addback':
            result = max(0, -result)
        result = round_ratio(result, numerator)
        current[target] = result
        output[target] = dict(value_minor=result, operation=operation, numerator=numerator,
                              denominator=100, operands=operands)

    def total(target, plus, minus=''):
        formula(target, [(key, 1) for key in plus.split()] + [(key, -1) for key in minus.split()])

    if form_id == 'K2kv':
        total('POOL_WITH_KEST', '862 864 891 893 895 897 936 171 173 175 189')
        total('POOL_WITHOUT_KEST', '863 865 892 894 896 898 937 172 174 176')
    elif form_id == 'K2a':
        total('REVENUE_SUM', '9040 9060 9070 9080 9081 9090 9093')
        total('EXPENSE_SUM', '9100 9110 9120 9130 9134 9135 9140 9142 9150 9160 9170 9180 9190 9200 9210 9220 9258 9248 9243 9244 9245 9246 9206 9207 9208 9209 9261 9262 9230 9233 9259')
        total('BOOK_PROFIT', 'REVENUE_SUM 9237', 'EXPENSE_SUM')
        total('SALDO_1', 'SUBGEW_1 SUBVER_1')
        total('SALDO_2', 'SUBGEW_2 SUBVER_2')
        formula('9301', [('SALDO_1', 1)], operation='negative_addback', numerator=45)
        formula('9309', [('SALDO_2', 1)], operation='negative_addback', numerator=40)
        # Absolute-value deductions are explicit in the BMF income validation.
        deductions = '9276 9277 9344 9345 9279 9339'.split()
        if all(type(current.get(k)) is int for k in deductions):
            for key in deductions:
                current['ABS_' + key] = abs(current[key])
            total('MWR_TOTAL', '9337 9338 9240 9268 9269 9273 9274 9260 9270 9280 9317 9322 9325 9257 9333 9334 9247 9267 9305 9301 9285 9309 9326 9290',
                  '9299 ABS_9276 ABS_9277 ABS_9344 ABS_9345 ABS_9279 ABS_9339')
        # BMF income-validation 2025 distinguishes an absent disposal field
        # from an explicitly submitted zero: a negative transition result is
        # ignored only when KZ9020 is absent. Preserve that reviewed distinction.
        transition = current.get('9010')
        absent_disposal = '9020' in not_applicable or '9020' not in current
        if type(transition) is int and transition < 0 and absent_disposal:
            current['9010'] = 0
        total('ADJUSTED_PROFIT', 'BOOK_PROFIT MWR_TOTAL 9010', '9242')
        if 'ADJUSTED_PROFIT' in output and current.get('9010') != transition:
            output['ADJUSTED_PROFIT']['conditional_operand'] = dict(
                field='9010', input_value_minor=transition, effective_value_minor=0,
                rule='negative_transition_without_disposal_9020')
        total('SMALL_PROFIT', '9027', '9028')
        total('TAX_PROFIT', 'ADJUSTED_PROFIT 9020 SMALL_PROFIT 9006', '9021 9030 SMALL_FLAT_ALLOWANCE')
    elif form_id == 'K2b':
        total('EXPENSE_SUM', '9470 9480 9490 9500 9134 9135 9505 9510 9520 9521 9522 9530')
        total('TAX_PROFIT', '9460 9414', 'EXPENSE_SUM 9030')
    elif form_id == 'K11':
        total('BUSINESS_RESULT', 'BETRAG_B LTGBET_B', 'KAPVM_B')
        total('RENT_RESULT', 'BETRAG_B LTGVV_B')
    elif form_id == 'K12a':
        total('ZUEB_12A', 'ZAW_12A', 'ZER_12A')
        formula('ZIUEBERH', [('ZUEB_12A', 1), ('ZIVORVJ', 1), ('UMUEBZI', 1), ('UMUNTZI', -1)], operation='positive')
        total('ZU_ZINS', 'ZAW_12A')
        total('AB_ZINS', 'ZER_12A')
        total('ST_EBIT', 'GBE_12A ZU_AFA ZU_TEILW ZU_ZINS', 'AB_ZUSCH AB_ZINS AB_INFRA')
        formula('VER_EBIT', [('ST_EBIT', 1)], operation='positive', numerator=30)
        total('ZINS_WJA', 'ZIVORVJ UMUEBZI', 'UMUNTZI')
        total('ZINS_WJE', 'NAB_ZINS ZINS_WJA', 'ABZ_ZIVT')
        total('EBIT_END', 'EBIT_VJ UMUEB_EB EBIT_K12', 'UMUNT_EB VERR_EVT WEG_WJ5')
        # Nonlinear17/18/20/25/26 are specialist-reviewed inputs: allowance
        # sharing and reorganization prevent a universal per-annex formula.
    elif form_id == 'K2':
        total('610', 'LF_A LF_B KV_LF 917')
        total('636', 'GW_A GW_B KV_GW 919')
        total('650', 'VV_A VV_B 546 547 818')
        total('PROPERTY_SUM', '572 573 574')
        total('INCOME_AFTER_INTEREST', 'INCOME_BEFORE_INTEREST 168', '177')
    return output
