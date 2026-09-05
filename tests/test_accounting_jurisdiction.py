import pytest

from kassiber.core.accounting import jurisdiction as j
from kassiber.errors import AppError


def test_registry_is_explicit_data_only_year_bound_and_not_mutable():
    pack = j.get_pack(j.AT_PACK_ID)
    assert pack['tax_year'] == 2025
    assert pack['currency'] == 'EUR'
    assert set(pack['forms']) == {'K2', 'K2kv', 'K2a', 'K2b', 'K11', 'K12', 'K12a'}
    assert len(pack['sources']) == 11
    pack['forms'].clear()
    assert j.get_pack(j.AT_PACK_ID)['forms']
    assert len(j.list_packs()) == 1
    for unavailable in ('../../evil.py', 'AT-K2-2026-v1', j.TEST_PACK_ID):
        with pytest.raises(AppError):
            j.get_pack(unavailable)
    test = j.get_pack(j.TEST_PACK_ID, allow_test=True)
    assert test['country'] == 'TEST' and test['test_only']


def test_k2kv_complete_2025_crypto_and_source_mappings():
    pack = j.get_pack(j.AT_PACK_ID)
    fields = j.fields_for(pack, 'K2kv')
    expected = set('856 857 929 858 859 860 861 934 935 862 863 864 865 891 892 893 894 895 896 897 898 936 937 189 171 172 173 174 175 176 899 900 901 902 297 298 299'.split())
    assert {key for key in fields if key.isdigit()} == expected
    assert all(fields[key]['xml_path'] == f'EINKUENFTE_KAPITALVERMOEGEN_K2/KZ{key}' for key in expected)
    assert all(fields[key]['pdf_field'] for key in expected)
    assert fields['175']['negative_only']
    assert fields['175']['pdf_field'] == 'Zahl106d.7.0'
    assert '179' not in fields
    assert '179' in j.fields_for(pack, 'K2')
    assert '832' not in j.fields_for(pack, 'K2')


def test_full_k2a_k2b_k12a_field_inventory_and_correct_labels():
    pack = j.get_pack(j.AT_PACK_ID)
    a = j.fields_for(pack, 'K2a')
    expected = set('9040 9060 9070 9080 9081 9090 9093 9088 9089 9100 9110 9120 9130 9134 9135 9140 9142 9150 9160 9170 9180 9190 9200 9210 9220 9258 9248 9243 9244 9245 9246 9206 9207 9208 9209 9261 9262 9230 9233 9259 9237 9249 9276 9277 9344 9345 9337 9338 9240 9269 9268 9273 9274 9260 9270 9280 9317 9322 9325 9257 9333 9334 9279 9339 9299 9305 9301 9285 9309 9326 9010 9242 9247 9267 9290 9020 9021 9030 9300 9310 9320 9330 9340 9350 9360 9363 9370 9027 9028 9006'.split())
    assert {key for key in a if key.isdigit()} == expected
    assert 'Umsatzsteuer' in a['9093']['label']
    assert a['9040']['xml_path'] == 'EINZELUNTERNEHMER_K2/ERTRAEGE_EINNAHMEN/KZ9040'
    assert a['9006']['xml_path'] is None  # Printed form lane; not a guessed XSD node.
    assert 'Kfz' in a['9170']['label']
    assert a['9310']['label'] == 'Grund und Boden'
    b = j.fields_for(pack, 'K2b')
    assert {key for key in b if key.isdigit()} == set('9030 9407 9409 9410 9416 9417 9430 9440 9450 9460 9470 9480 9490 9500 9134 9135 9505 9510 9520 9521 9522 9530 9414'.split())
    z = j.fields_for(pack, 'K12a')
    assert len([f for f in z.values() if f['type'] == 'money']) == 28
    assert z['NAB_ZINS']['xml_path'] == 'ZINSSCHRANKE_K12A/NAB_ZINS'


def test_exact_crypto_pool_signed_losses_and_unknown_not_zero():
    assert j.derive_fields('K2kv', {'173': 10000}) == {}
    values = dict.fromkeys('862 864 891 893 895 897 936 171 173 175 189'.split(), 0)
    values.update({'171': 100, '173': 900, '175': -300, '189': 50})
    out = j.derive_fields('K2kv', values)
    assert out['POOL_WITH_KEST']['value_minor'] == 750
    assert 'POOL_WITHOUT_KEST' not in out
    assert out['POOL_WITH_KEST']['operands'][-2]['value_minor'] == -300


def test_exact_loss_addback_and_no_personal_tax_rate():
    out = j.derive_fields('K2a', {'SUBGEW_1': 100, 'SUBVER_1': -201, 'SUBGEW_2': 0, 'SUBVER_2': -101})
    assert out['9301']['value_minor'] == 45
    assert out['9309']['value_minor'] == 40
    assert j.round_ratio(1, 50) == 1
    assert j.round_ratio(-1, 50) == -1
    assert j.derive_fields('K2a', {'SUBGEW_1': 100, 'SUBVER_1': -10})['9301']['value_minor'] == 0
    assert not j.derive_fields('arbitrary_code', {'__import__': 'evil'})


def test_rental_formula_excludes_9030_once_and_specialist_interest_not_guessed():
    values = dict.fromkeys('9470 9480 9490 9500 9134 9135 9505 9510 9520 9521 9522 9530'.split(), 0)
    values.update({'9460': 1000, '9414': -30, '9030': 100, '9500': 200})
    assert j.derive_fields('K2b', values)['TAX_PROFIT']['value_minor'] == 670
    assert 'NABZ_VOR' not in j.derive_fields('K12a', {'ZAW_12A': 400000000, 'ZER_12A': 0})


def test_business_transition_absence_is_not_explicit_zero_and_flat_lanes_roll_up():
    values = dict(BOOK_PROFIT=1000, MWR_TOTAL=0, SMALL_FLAT_ALLOWANCE=0)
    values.update({'9010': -300, '9242': 0, '9020': 0, '9021': 0, '9030': 0,
                   '9027': 200, '9028': 50, '9006': 400})
    explicit = j.derive_fields('K2a', values)
    absent = j.derive_fields('K2a', values, not_applicable={'9020'})
    assert explicit['TAX_PROFIT']['value_minor'] == 1250
    assert absent['TAX_PROFIT']['value_minor'] == 1550
    assert absent['ADJUSTED_PROFIT']['conditional_operand']['input_value_minor'] == -300
    assert 'conditional_operand' not in explicit['ADJUSTED_PROFIT']
