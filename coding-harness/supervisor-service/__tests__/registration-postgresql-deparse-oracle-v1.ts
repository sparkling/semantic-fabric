// SPDX-License-Identifier: MIT

/**
 * Reviewed PostgreSQL 16.15 (`server_version_num = 160015`) deparse facts.
 *
 * The capture ran with `search_path = pg_catalog` and `pretty = false`.
 * Domain and table-check expressions came from `pg_get_expr`; complete table
 * constraint definitions came from `pg_get_constraintdef`. These are literal
 * KAT expectations: no value is derived from the catalogue contract at runtime.
 */

export interface PostgresDeparseFactV1 {
  readonly identity: string;
  readonly byteLength: number;
  readonly sha256: string;
}

export const POSTGRES_16_15_DOMAIN_CHECK_EXPRESSIONS_V1 = Object.freeze([
  Object.freeze({ identity: 'domain-check-expression:sf_supervisor_v1.configuration_bytes_v2.configuration_bytes_v2_check_v1', byteLength: 64, sha256: '8eede7eea742c0c3ac0a3776a880954f15388b95418b5cd41c0d6e594f9246bf' }),
  Object.freeze({ identity: 'domain-check-expression:sf_supervisor_v1.ed25519_spki_der_v1.ed25519_spki_der_v1_check_v1', byteLength: 98, sha256: '8ac6ac659bd050efd4801f414550d2117673a4585c0b2d28d01a5b1b1927fe6c' }),
  Object.freeze({ identity: 'domain-check-expression:sf_supervisor_v1.event_envelope_bytes_v2.event_envelope_bytes_v2_check_v1', byteLength: 63, sha256: '0c0b2750bd4efa9f1d483ffab639d4f08e29f1a80453b7524975234334a7067b' }),
  Object.freeze({ identity: 'domain-check-expression:sf_supervisor_v1.opaque_id_v1.opaque_id_v1_check_v1', byteLength: 100, sha256: '9715c4bf989e1d13c90f2f97a1fa6bbff26ca57a1f479baff745b1704f5830e3' }),
  Object.freeze({ identity: 'domain-check-expression:sf_supervisor_v1.project_scope_role_v1.project_scope_role_v1_check_v1', byteLength: 48, sha256: 'b1c65181f473994035b5852696522fc4b5dfee4b661f91b61db8b84cf06d5518' }),
  Object.freeze({ identity: 'domain-check-expression:sf_supervisor_v1.public_commitment_bytes_v2.public_commitment_bytes_v2_check_v1', byteLength: 62, sha256: '20c1db32bef80b32216ba758c634c0a76980b197c487ccd9f26ca734c37d0c37' }),
  Object.freeze({ identity: 'domain-check-expression:sf_supervisor_v1.registration_result_bytes_v2.registration_result_bytes_v2_check_v1', byteLength: 64, sha256: '2ef6f33c6935716e7c06b27dadc6f4f34cfa8b4ecd826429d6884873feec650d' }),
  Object.freeze({ identity: 'domain-check-expression:sf_supervisor_v1.request_bytes_v2.request_bytes_v2_check_v1', byteLength: 63, sha256: 'f0e3b0e8b07df7b6211c5d9f6c529187ef481692856cf0bb2813785517d2be70' }),
  Object.freeze({ identity: 'domain-check-expression:sf_supervisor_v1.sha256_digest_v1.sha256_digest_v1_check_v1', byteLength: 119, sha256: 'b26a9c45f4ad9111c47a892868d06eaf55a505521262f2facf4ac25f26230379' }),
  Object.freeze({ identity: 'domain-check-expression:sf_supervisor_v1.uint64_v1.uint64_v1_check_v1', byteLength: 95, sha256: '7a9dc019f538ec68bc95b6bee0492075c4fb5197d9796c03950258eed0065f4b' }),
] satisfies readonly PostgresDeparseFactV1[]);

export const POSTGRES_16_15_CONSTRAINT_DEFINITIONS_V1 = Object.freeze([
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_configurations.authority_configurations_digest_uq_v1', byteLength: 75, sha256: '5ed6d593783b9d60b6bc1cf13016c9c0f6d251e036170ca81aba961a8d09ae54' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_configurations.authority_configurations_epoch_zero_check_v1', byteLength: 55, sha256: 'f27ce96b9a4661df8c91d59216c4fb0ad0d6e9caa7fe67008c5c6589ecb8a8d8' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_configurations.authority_configurations_genesis_receipt_uq_v1', byteLength: 129, sha256: '0d132b236a6b871652c8893bc14af91d65e50cb1877581696a9ba6af60e309f1' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_configurations.authority_configurations_head_uq_v1', byteLength: 127, sha256: '94d48866451d78313009dac285b49aa6acb5f70ba85c07cc76c2583ef1086601' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_configurations.authority_configurations_pk_v1', byteLength: 79, sha256: '3c59bdc38b26035e96681c7ac5ebed4d4589626c6e133172f38516c944f556f7' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_configurations.authority_configurations_service_key_epoch_check_v1', byteLength: 53, sha256: '932d5f860c7713cf27a632ff5484fdb5121c121e4c03755145bf1806a74ece93' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_state.authority_state_active_configuration_fk_v1', byteLength: 349, sha256: 'eb261285a6186e5ae670f2fc89cf13468a90648a67bb92f31be1e780aea8ebf5' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_state.authority_state_last_event_check_v1', byteLength: 183, sha256: 'ff5bfbf2f585d9f53a5b9ec903adac8d097cdf4557c33095cbdffefca35b5bd0' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_state.authority_state_last_event_fk_v1', byteLength: 288, sha256: '7f9ec190b3d8c16c90af175051a6fe6feb828d7a6d1f683f7cd46bf4198e10a3' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_state.authority_state_pk_v1', byteLength: 58, sha256: 'dd17cd6596147d891cfdf2f345f2620d30638d7e41cf46cdce9ead2568f000d1' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_state.authority_state_sequence_successor_check_v1', byteLength: 92, sha256: '62c91890c15212dbe437145bea4bdadeace78674bfd0400ffe683f6c8e233c82' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_state.authority_state_singleton_true_check_v1', byteLength: 31, sha256: 'e4bdd775f5f0c4851759440d55855dab508c3bbdeb97624a6cd598d73086a5fa' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.authority_state.authority_state_singleton_uq_v1', byteLength: 22, sha256: 'ccf04784b844fd519c44168ba82369217af8c7f764aa6e2db88cb5c05e64034a' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.publication_outbox.publication_outbox_commitment_uq_v1', byteLength: 79, sha256: '07153ae96193883a5f44120641ecb099d336d5181034c365ab1c5957c7ed327c' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.publication_outbox.publication_outbox_event_fk_v1', byteLength: 214, sha256: '0e982f2b45ad1384ed50da6f5c53f55a24ba72f58843b8c0291db50b926e37da' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.publication_outbox.publication_outbox_pk_v1', byteLength: 72, sha256: 'f5ff0b6cdf4e4382e703fc03431f08b23254e2f248cdb392cc86215434879a10' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.publication_outbox.publication_outbox_scope_fk_v1', byteLength: 186, sha256: '46d52d719a873ca258989e537233935da8ca162c0f468f0114312da58f1237f4' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.publication_outbox.publication_outbox_state_check_v1', byteLength: 45, sha256: '079a0ddff4f114f45abd9842fbbbe50d44e162f4fbcc8235c572a20a927b3579' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_results.registration_results_content_type_check_v1', byteLength: 73, sha256: '7d849e0b731841960bc340c6de33c3ccf8ae4d75b4ecbf71b10a895fb2b512c2' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_results.registration_results_current_event_fk_v1', byteLength: 318, sha256: 'ddba25be3217a02773ab346d14ff2aac3c6424d4f7454d378917c1ec876f8f11' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_results.registration_results_current_event_uq_v1', byteLength: 75, sha256: '1feda22a69b40c89b3524925360de9cce30cbb87bfe326e0adb1590c74b2c5f3' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_results.registration_results_pk_v1', byteLength: 83, sha256: '7ce4ebf9a2b5c3a749cfaa8d5c87760693d3de16066e94751698724aa8b93e48' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_results.registration_results_response_status_check_v1', byteLength: 60, sha256: 'e8465a99015012f15700f412ab825977956df022dd3659e673c54aad2ef3b63e' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_results.registration_results_run_provenance_fk_v1', byteLength: 458, sha256: '3ec02bd4c7f9fc6755c9fdb7a360b35d0d792356c2438ec9ebb488f805ebbc0a' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_results.registration_results_run_request_uq_v1', byteLength: 86, sha256: '9a0a57afb563c61a8bb96108edc79148077542f1c9ab9c56499501afd44f760d' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_results.registration_results_run_status_uq_v1', byteLength: 78, sha256: 'ee9952df03d564a53189e2adb84022c1acf2821763e5ad17f9dcfdde9ab0ac98' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_results.registration_results_scope_fk_v1', byteLength: 186, sha256: '46d52d719a873ca258989e537233935da8ca162c0f468f0114312da58f1237f4' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_results.registration_results_status_provenance_check_v1', byteLength: 498, sha256: 'b98e76c9192adf9d88502227ed5ebeabc22c1dd39dc4c8ebc3183ef1bb5c4c40' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_runs.registration_runs_first_changed_result_fk_v1', byteLength: 299, sha256: 'ce3beee9245cd65a4a71a675a0dfe0f05d434e62988dfc77a3c1c52882080c73' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_runs.registration_runs_last_event_snapshot_fk_v1', byteLength: 423, sha256: '5dc608d6b9bd235997ced3af83a47c462cfcafc3a597a8846c2ed1f3f9c90b28' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_runs.registration_runs_original_event_fk_v1', byteLength: 345, sha256: '361058af2fd37f1035c4146f74a6bece3392ad77b6aa53bc2f74308f926c43ac' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_runs.registration_runs_original_provenance_uq_v1', byteLength: 173, sha256: '8992b02ec46d2dd9d9e71f36f3ced865af0b8b558a626507cedf58c81df6c0ed' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_runs.registration_runs_original_result_fk_v1', byteLength: 300, sha256: 'f9f43a09550320ef6dfd259cada56386575d0eebafeea5a8a76446e919f0f5ce' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_runs.registration_runs_pk_v1', byteLength: 66, sha256: '5241a3cb8d70ef77aeddf4e1bc970a49f8b0e344698446e92985371c0d50416d' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_runs.registration_runs_scope_fk_v1', byteLength: 186, sha256: '46d52d719a873ca258989e537233935da8ca162c0f468f0114312da58f1237f4' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.registration_runs.registration_runs_state_check_v1', byteLength: 479, sha256: 'a0131bab8aacfa2eeca0ce485ce046e502083efd9b5155a20568adb8d96c1ed2' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.schema_migrations.schema_migrations_pk_v1', byteLength: 31, sha256: '9fe63aab5b28d082b628da96dac3691ceb39299fbd90ab1d20ff4ce2cb8aaaf2' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.schema_migrations.schema_migrations_version_positive_check_v1', byteLength: 31, sha256: '5c41a70cf37d0b18de0b6177d92d2006ec2e67e80db701502fae24fd673cd3fe' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_configuration_fk_v1', byteLength: 385, sha256: 'a9ba859e599637f51ef823666e027dbff9c2020c4157f70424d4f2b3007ed6f7' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_genesis_receipt_fk_v1', byteLength: 432, sha256: 'a1de2cc8698a6bb862b23b454f4d5bf698a89d2f495abf0070170b7eca5d5709' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_global_reference_uq_v1', byteLength: 84, sha256: '8aabce7628d8c5669eccdb0c313f742ba2ea92904f9f615a9557b8b3b035f88e' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_global_sequence_check_v1', byteLength: 52, sha256: '3c286426eb48222315fe32a0f3efa0a33a89812e1ba7418d16ba11bf4aab5cc4' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_global_sequence_uq_v1', byteLength: 70, sha256: 'bf9fad66449362549d3274ba8962f395e1a93382b522782994d620993720cb7f' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_kind_run_sequence_check_v1', byteLength: 196, sha256: '3b4e3ef57ee77e5167f2a609b84bb7983c293c5954277fd6a3094596111cb174' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_pk_v1', byteLength: 72, sha256: 'f5ff0b6cdf4e4382e703fc03431f08b23254e2f248cdb392cc86215434879a10' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_previous_global_check_v1', byteLength: 886, sha256: '74cdb4c5bbb78007265ccd0cffffead75b721fda490ef92bdc39f37bc8d5819b' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_previous_global_fk_v1', byteLength: 303, sha256: '89418bc3a889d73c2122f6f9fbdc1eb0a49a9b617d3edac1d5fdbdaa020b86a5' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_previous_receipt_fk_v1', byteLength: 325, sha256: '4df2cdb78465f53acd65f909a8cd1e316a463f2cc6d956acfd69c820455cc060' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_previous_run_check_v1', byteLength: 527, sha256: 'a5dd8830c03dbb4090cba6fcdfd2ea28332db66ed5bda86ca537f400a431e614' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_previous_run_fk_v1', byteLength: 433, sha256: 'fec930a3b1b5c3b57c8f911e47f3962a89f05d77316161b3ed31531bb864c289' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_request_provenance_uq_v1', byteLength: 100, sha256: 'ba4c38d957f65457cb9eeb397f5d80bb1c0e0949f533a9dc9c15fcbfd320f8b6' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_run_fk_v1', byteLength: 234, sha256: 'b46b4fb270632cb27eb3e6238a0f5320b02ab30808f866952bda5cc1662d0aff' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_run_sequence_uq_v1', byteLength: 75, sha256: 'a5d58f1772fa616c05b8e0bba9329f362ff443618d927154c6fcfbc0be065ad2' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_run_snapshot_uq_v1', byteLength: 146, sha256: 'ac24d79ac4f6c9c0779816bdb1efff2f9f588507968ed331af37aa12f41e08f4' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_events.semantic_events_scope_fk_v1', byteLength: 186, sha256: '46d52d719a873ca258989e537233935da8ca162c0f468f0114312da58f1237f4' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_receipts.semantic_receipts_digest_uq_v1', byteLength: 78, sha256: '5dcbf4563e2c3c9204967d80cdd1c594522ed2799f1a3f3164e8d33e6bc62893' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_receipts.semantic_receipts_event_fk_v1', byteLength: 214, sha256: '0e982f2b45ad1384ed50da6f5c53f55a24ba72f58843b8c0291db50b926e37da' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_receipts.semantic_receipts_event_receipt_uq_v1', byteLength: 92, sha256: '132e64cbcd87377f057f200eb5fcad1354d2a9f4b811898fcff9bc60d7fed333' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_receipts.semantic_receipts_pk_v1', byteLength: 72, sha256: 'f5ff0b6cdf4e4382e703fc03431f08b23254e2f248cdb392cc86215434879a10' }),
  Object.freeze({ identity: 'constraint-definition:sf_supervisor_v1.semantic_receipts.semantic_receipts_scope_fk_v1', byteLength: 186, sha256: '46d52d719a873ca258989e537233935da8ca162c0f468f0114312da58f1237f4' }),
] satisfies readonly PostgresDeparseFactV1[]);

export const POSTGRES_16_15_TABLE_CHECK_EXPRESSIONS_V1 = Object.freeze([
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.authority_configurations.authority_configurations_epoch_zero_check_v1', byteLength: 47, sha256: '40a460d3563f1f2396c4437e252d96c756c6475141328f2aa5d95eee00b9cf69' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.authority_configurations.authority_configurations_service_key_epoch_check_v1', byteLength: 45, sha256: '72f01f6de36e3af3824e91b870b4a71c3cee95bd807d9378d9a9246111d6204a' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.authority_state.authority_state_last_event_check_v1', byteLength: 175, sha256: 'ad6aa597137fe0314be632ed7d4a32ce18119a4a63bf01f4fb4444ab04906cc2' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.authority_state.authority_state_sequence_successor_check_v1', byteLength: 84, sha256: '07c0960138abf025df25f894fcb8074f595fe143e5bcb1586bf9c90df8f2fe74' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.authority_state.authority_state_singleton_true_check_v1', byteLength: 23, sha256: '985652eadc048d79c977ac2a19d34d157c435282b6c413453fbd0c98ff8eb66a' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.publication_outbox.publication_outbox_state_check_v1', byteLength: 37, sha256: 'f7a6679caf8280312bb93510453b09fa2ca33593b0995f1036c940c283261b45' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.registration_results.registration_results_content_type_check_v1', byteLength: 65, sha256: '1e01b6028808ee86c9e4c7ecb847852fb79ec718c2d9552722c8575c2000d754' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.registration_results.registration_results_response_status_check_v1', byteLength: 52, sha256: 'de3de9cc12d25a1b0f9ed4a94fb3bc209611f02f91e4ee2fb2d7a4e9bfc85772' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.registration_results.registration_results_status_provenance_check_v1', byteLength: 490, sha256: '8702c92ced7d57e928ddf0365753b6b05e86c4bcca7c1ca88fed7820c960f6d2' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.registration_runs.registration_runs_state_check_v1', byteLength: 471, sha256: 'd7de0ec5f4754885cc32031328069eed0af3fd23b001dd352c4402c920d19232' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.schema_migrations.schema_migrations_version_positive_check_v1', byteLength: 23, sha256: '9d7d6163aef033ba0fdf6bdc43ac779b02189b2658301e2f0f2cdcd1301730f6' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.semantic_events.semantic_events_global_sequence_check_v1', byteLength: 44, sha256: '2b6782a2159ced108a796cf02fa6954d56fb409d0ec3d4ed04b20074909de039' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.semantic_events.semantic_events_kind_run_sequence_check_v1', byteLength: 188, sha256: '0437e4fdfe33587ac7518b3b6998255df265e4cbe9ff295c784d4e00b90233d2' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.semantic_events.semantic_events_previous_global_check_v1', byteLength: 878, sha256: '91d437c843ab0f18dbeb39ea861a3ada7c37c6586ba441d89f481077299d2701' }),
  Object.freeze({ identity: 'table-check-expression:sf_supervisor_v1.semantic_events.semantic_events_previous_run_check_v1', byteLength: 519, sha256: 'd172daad392a05a079e124ed55253e5179a1080bd134eed92c1c6e2e6655b944' }),
] satisfies readonly PostgresDeparseFactV1[]);

export const POSTGRES_16_15_DEPARSE_ORACLE_V1 = Object.freeze({
  serverVersionNum: 160_015,
  searchPath: 'pg_catalog',
  pretty: false,
  domainCheckExpressions: POSTGRES_16_15_DOMAIN_CHECK_EXPRESSIONS_V1,
  constraintDefinitions: POSTGRES_16_15_CONSTRAINT_DEFINITIONS_V1,
  tableCheckExpressions: POSTGRES_16_15_TABLE_CHECK_EXPRESSIONS_V1,
});
