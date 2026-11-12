-- Read-only guards for data-dependent ensure_schema_compat branches. These are
-- acceptance probes, never migration SQL: a book they flag still needs its
-- migrating open (legacy_pair_components: a legacy review Python's migration
-- would still pick up, neither linked to a component nor held by an issue).
-- probe: legacy_source_links
SELECT EXISTS(SELECT 1 FROM source_funds_links WHERE method IN ('transaction_pair','same_onchain_scope','payment_hash'));
-- probe: source_funds_target_snapshot
SELECT EXISTS(SELECT 1 FROM source_funds_cases c JOIN transactions t ON t.id=c.target_transaction_id WHERE c.target_external_id IS NULL AND t.external_id IS NOT NULL);
-- probe: provider_display_names
SELECT EXISTS(SELECT 1 FROM ai_providers WHERE display_name IS NULL OR trim(display_name)='');
-- probe: wallet_scoped_labels
SELECT EXISTS(SELECT 1 FROM bip329_labels WHERE wallet_id IS NOT NULL);
-- probe: legacy_pair_components
SELECT EXISTS(SELECT 1 FROM transaction_pairs p JOIN transactions o ON o.id=p.out_transaction_id JOIN wallets ow ON ow.id=o.wallet_id JOIN transactions i ON i.id=p.in_transaction_id JOIN wallets iw ON iw.id=i.wallet_id WHERE p.component_id IS NULL AND NOT EXISTS(SELECT 1 FROM custody_authored_migration_issues m WHERE m.legacy_table='transaction_pairs' AND m.legacy_source_id=p.id AND m.resolved_at IS NULL)) OR EXISTS(SELECT 1 FROM direct_swap_payouts p JOIN transactions o ON o.id=p.out_transaction_id JOIN wallets ow ON ow.id=o.wallet_id WHERE p.component_id IS NULL AND NOT EXISTS(SELECT 1 FROM custody_authored_migration_issues m WHERE m.legacy_table='direct_swap_payouts' AND m.legacy_source_id=p.id AND m.resolved_at IS NULL));
-- probe: missing_durable_anchors
SELECT EXISTS(SELECT 1 FROM custody_component_legs WHERE anchor_transaction_id IS NULL AND transaction_id IS NOT NULL);
-- probe: evidence_commitments
SELECT EXISTS(SELECT 1 FROM custody_components c WHERE expected_evidence_count IS NULL AND (EXISTS(SELECT 1 FROM custody_authored_evidence_snapshots s WHERE s.profile_id=c.profile_id AND s.subject_kind='custody_component' AND s.subject_id=c.id) OR (c.state='active' AND NOT EXISTS(SELECT 1 FROM custody_component_legs l WHERE l.component_id=c.id AND coalesce(l.anchor_transaction_id,l.transaction_id) IS NOT NULL))));
