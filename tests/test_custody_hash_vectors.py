"""Stored custody hashes and ids stay byte-identical.

The vectors in tests/fixtures/custody_hash_vectors/ pin what the product
functions compute today. A difference means a stored commitment or id would
no longer match its recomputation; regenerate the fixtures only together with
a migration for books that stored the old value.
"""

import hashlib
import json
import unittest
import uuid

from tests import custody_hash_vectors as vectors


MIN_VECTORS_PER_FAMILY = 15


def _stored():
    return {
        family.name: vectors.load_vectors(vectors.fixture_path(family.name))
        for family in vectors.FAMILIES
        if vectors.fixture_path(family.name).exists()
    }


def _outputs(output):
    """Every output mapping in a vector, including nested observation lists."""

    if isinstance(output, dict):
        yield output
        for value in output.values():
            if isinstance(value, list):
                for item in value:
                    yield from _outputs(item)


def _dumps_kwargs(rule):
    kwargs = dict(rule["json.dumps"])
    if "separators" in kwargs:
        kwargs["separators"] = tuple(kwargs["separators"])
    if kwargs.get("default") == "str":
        kwargs["default"] = str
    return kwargs


class CustodyHashVectorTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.stored = _stored()

    def test_every_family_has_a_fixture_and_enough_vectors(self):
        on_disk = {path.stem for path in vectors.FIXTURE_DIR.glob("*.jsonl")}
        self.assertEqual(on_disk, set(vectors.FAMILIES_BY_NAME))
        for family in vectors.FAMILIES:
            with self.subTest(family=family.name):
                self.assertGreaterEqual(len(self.stored[family.name]), MIN_VECTORS_PER_FAMILY)

    def test_product_functions_reproduce_every_vector(self):
        for name, stored in self.stored.items():
            family = vectors.FAMILIES_BY_NAME[name]
            for vector in stored:
                with self.subTest(family=name, case=vector["case"]):
                    self.assertEqual(vector["function"], family.function)
                    self.assertEqual(vectors.recompute(vector), vector["output"])

    def test_preimages_hash_to_the_stored_values(self):
        checked = 0
        for name, stored in self.stored.items():
            for vector in stored:
                for output in _outputs(vectors.decode(vector["output"])):
                    for key, preimage in output.items():
                        if not key.endswith("_preimage"):
                            continue
                        base = key[: -len("_preimage")]
                        hash_key = next(
                            candidate
                            for candidate in (base, f"{base}_hash", f"{base}_id", f"{base}_fingerprint")
                            if candidate in output
                        )
                        stored_hash = output[hash_key]
                        digest = hashlib.sha256(preimage.encode("utf-8")).hexdigest()
                        if stored_hash.startswith("cg_"):
                            digest = "cg_" + digest[:24]
                        elif stored_hash.startswith("native-htlc-"):
                            digest = "native-htlc-" + digest
                        with self.subTest(family=name, case=vector["case"], field=hash_key):
                            self.assertEqual(digest, stored_hash)
                        checked += 1
        self.assertGreater(checked, 500)

    def test_preimages_follow_the_recorded_json_dumps_arguments(self):
        checked = 0
        for name, stored in self.stored.items():
            for vector in stored:
                rules = [
                    rule
                    for rule in vector["serialization"].values()
                    if isinstance(rule, dict) and "json.dumps" in rule and "preimage" in rule
                ]
                for rule in rules:
                    kwargs = _dumps_kwargs(rule)
                    for output in _outputs(vectors.decode(vector["output"])):
                        preimage = output.get(rule["preimage"])
                        if preimage is None:
                            continue
                        with self.subTest(family=name, case=vector["case"], field=rule["preimage"]):
                            self.assertEqual(json.dumps(json.loads(preimage), **kwargs), preimage)
                        checked += 1
        self.assertGreater(checked, 400)

    def test_uuid5_ids_follow_their_recorded_names(self):
        checked = 0
        for name, stored in self.stored.items():
            for vector in stored:
                output = vectors.decode(vector["output"])
                namespace = uuid.UUID(output["namespace"]) if "namespace" in output else uuid.NAMESPACE_URL
                pairs = []
                if "name" in output and "id" in output:
                    pairs.append((output["id"], output["name"]))
                for ids, names in (("leg_ids", "leg_names"), ("allocation_ids", "allocation_names")):
                    pairs.extend(zip(output.get(ids, ()), output.get(names, ())))
                for value, uuid_name in pairs:
                    with self.subTest(family=name, case=vector["case"]):
                        self.assertEqual(str(uuid.uuid5(namespace, uuid_name)), value)
                    checked += 1
        self.assertGreater(checked, 80)

    def test_fixtures_are_what_the_seeded_generator_writes(self):
        generated = vectors.generate()
        for name, fresh in generated.items():
            with self.subTest(family=name):
                self.assertEqual(
                    [vectors.vector_line(item) for item in fresh],
                    [vectors.vector_line(item) for item in self.stored[name]],
                )


if __name__ == "__main__":
    unittest.main()
