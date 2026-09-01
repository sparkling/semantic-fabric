// SPDX-License-Identifier: MIT

import { Buffer } from 'node:buffer';
import { isProxy } from 'node:util/types';
import {
  canonicalDigestHexV1,
  closedRecordV1,
  deepFreezeV1,
  exactKeysV1,
  POSTGRES_PROJECT_SCOPE_ROLE_V1,
  parseCanonicalPrettyJsonBytesV1,
  parseDigestV1,
  parseOpaqueIdV1,
  parseUint64V1,
  rawSha256HexV1,
  snapshotBytesV1,
  validateEd25519SpkiV1,
} from './registration-postgresql-canonical-v1.js';
import {
  parsePostgresMigrationJsonBytesV1,
} from './registration-postgresql-migration-json-v1.js';

export const POSTGRES_AUTHORITY_SEED_DOMAIN_V1 =
  'semantic-fabric/programme-capture/supervisor-postgresql-authority-seed-v1';
export const POSTGRES_AUTHORITY_SEED_BYTES_V1 = 11_303;
export const POSTGRES_AUTHORITY_SEED_SHA256_V1 =
  'b6d3b0f77a2b71cb10782152cb74a3f37320e220131a2d3ef29cc455c9a26e4c';

const CONFIGURATION_DIGEST_DOMAIN =
  'semantic-fabric/programme-capture/supervisor-authority-configuration-digest-v2';
const GENESIS_HEAD_DOMAIN =
  'semantic-fabric/programme-capture/supervisor-authority-genesis-head-v2';
const INVALID = 'PostgreSQL authority seed is invalid';
const SEALED_TEXT = `{
  "domain": "semantic-fabric/programme-capture/supervisor-postgresql-authority-seed-v1",
  "schemaVersion": 1,
  "authorityConfiguration": {
    "projectAuthorityDigest": "9ba3490c3ff9becc86163de81360b6aa1ea64e9e2f7098ec72daefa5a66b77bf",
    "projectScopeRole": "sf_supervisor_project_scope_v1",
    "configurationEpoch": "0",
    "configurationDigest": "90ce861103ea0bcacebc37ee97f502fa5080ab5e9ab1b542c15393b96fac4c02",
    "genesisAuthorityHeadDigest": "80b825d44f308c1ef66d92129f2c34e5b05e88ee839ec7ca91bf2301f2013146",
    "serializedConfiguration": "ewogICJzY2hlbWFWZXJzaW9uIjogMiwKICAidHJhbnNhY3Rpb25LaW5kIjogInByb2dyYW1tZS1jYXB0dXJlLXYyIiwKICAicmVjb3JkS2luZCI6ICJzdXBlcnZpc29yLWF1dGhvcml0eS1jb25maWd1cmF0aW9uLXYyIiwKICAiYXV0aG9yaXR5IjogImRldmVsb3BtZW50LW9ubHktbm8tcHJvbW90aW9uIiwKICAiY29uZmlndXJhdGlvbkVwb2NoIjogIjAiLAogICJwcmVkZWNlc3NvciI6IHsKICAgICJraW5kIjogImdlbmVzaXMiLAogICAgImNvbmZpZ3VyYXRpb25EaWdlc3QiOiBudWxsLAogICAgImhlYWREaWdlc3QiOiBudWxsCiAgfSwKICAicHJvamVjdCI6IHsKICAgICJwcm9qZWN0QXV0aG9yaXR5RGlnZXN0IjogIjliYTM0OTBjM2ZmOWJlY2M4NjE2M2RlODEzNjBiNmFhMWVhNjRlOWUyZjcwOThlYzcyZGFlZmE1YTY2Yjc3YmYiLAogICAgInByaW5jaXBhbCI6IHsKICAgICAgInByaW5jaXBhbElkIjogInByb2plY3RfY2xpZW50XzIwMjYwODI5IiwKICAgICAgImtleUVwb2NoIjogIjEiLAogICAgICAia2V5RmluZ2VycHJpbnQiOiAiZmM4YmIzOGMzOWVkNTg4ZDY2MjVkYTkwMjQxMzFjMzllYmRkNzBmNTEwNmVhZDU5MjY1MjhjYWI3MmEwZjk5NiIsCiAgICAgICJwb2xpY3lEaWdlc3QiOiAiOGE3ZTIyOGM3OGM2MWE2YTUxMDgwM2NiNzRkZTRkZDYyOTE3YzgxZDUyN2U1ZTk4MmVlZDg0NzY2OGZlNGNiZSIsCiAgICAgICJhZG1pbmlzdHJhdGlvbkRpZ2VzdCI6ICJlODkwMjQ2MTc5MDBhMjYzOGJlNTg2MjkyY2UxZjMzNjQ5ODQ3YTY4ZmZhMzIyODAyMGY4OWUyOGFiNmI4NThkIgogICAgfSwKICAgICJhdXRoZW50aWNhdGlvblBvbGljeURpZ2VzdCI6ICIzOTg3MjgxNzMzMWVlZDU4N2NhODE1MGU1YzZhOTcxMmUzZjBlOWZiZmM0YWNmYWIxNjIyZDAxODQ5YWY5NWQ1IgogIH0sCiAgInNlcnZpY2UiOiB7CiAgICAicHJpbmNpcGFsIjogewogICAgICAicHJpbmNpcGFsSWQiOiAic3VwZXJ2aXNvcl9zZXJ2aWNlXzIwMjYwODI5IiwKICAgICAgImtleUVwb2NoIjogIjEiLAogICAgICAia2V5RmluZ2VycHJpbnQiOiAiMDZlM2ZkOGZkYTI5YmI2MGFiNTk1NTdkZTYxZWRiMGFlY2RiMjMxMTM0YmUzMGU3NWI0NTVmOGUxYjc5MmZhOSIsCiAgICAgICJwb2xpY3lEaWdlc3QiOiAiNDZlNjYyODJhZGZkODUwNWVhNmFmNzAzMzQ3MDI1YjJjNjc0MTMxZjhmYzdmMjI2M2U1OTVlN2YyOTVmMDY0YSIsCiAgICAgICJhZG1pbmlzdHJhdGlvbkRpZ2VzdCI6ICI0ZmNhYjAyNGI4MmIxZGY3NTE2MDM4NDk2NWUwYTgyZjRlMzI4OTNkOTBmOGYxNTA0YzFlZDM5Nzk5ZjNhNTEyIgogICAgfSwKICAgICJlbmRwb2ludE9yaWdpbiI6ICJodHRwczovL3N1cGVydmlzb3IuZXhhbXBsZS5vcmciLAogICAgInRsc1Nwa2lGaW5nZXJwcmludCI6ICJlNzUxMDc3MGMyYTg0NGFiZGI3NjBmYmYwOThjYjhhMGIyZWU4NTNlNDlkZDM0ODQ5N2IyMWM4NjQ0MjRlOTE0IiwKICAgICJjbGllbnRQb2xpY3lEaWdlc3QiOiAiMGJhMDQ1NDRmYTM4YmQ4N2RiMmQ1OTM1NjE2Nzk1MTZiM2M1MjY4ZjQ0MTUyYzcxYjY3MGE5MTU3MmY1YmE5OSIKICB9LAogICJ0cmFuc3BhcmVuY3lMb2ciOiB7CiAgICAicHJpbmNpcGFsIjogewogICAgICAicHJpbmNpcGFsSWQiOiAidHJhbnNwYXJlbmN5X2xvZ18yMDI2MDgyOSIsCiAgICAgICJrZXlFcG9jaCI6ICIxIiwKICAgICAgImtleUZpbmdlcnByaW50IjogImY1MDhhYTU3NWE3MGRjYjNiNTMwZmUzMTljNjkyMjliYzY4YTkzOTRlZmQ1YTkzMzE1YmE0ZTNjNmExMzhlOWQiLAogICAgICAicG9saWN5RGlnZXN0IjogIjg5MmZiMGRjNTFkYTAyYzA5ZTQ0MmMxZmUyZjg5MjY1Y2E3OWU5MTgzZTgyYzUwZGMxZjEwNDQ4ODI4Yjk5MmIiLAogICAgICAiYWRtaW5pc3RyYXRpb25EaWdlc3QiOiAiZmFkZTRlYWU1NDdkODAyNjVjYjBiZjEwYjdjYjJmYzVmNjg5YzQ1MTQwZDgyYzFlODMxY2FiZDlhMDM5NmFiOSIKICAgIH0sCiAgICAiZW5kcG9pbnRPcmlnaW4iOiAiaHR0cHM6Ly9sb2cuZXhhbXBsZS5vcmciLAogICAgInRsc1Nwa2lGaW5nZXJwcmludCI6ICI3ZWYwMGZlYzIyMTY5ZDE4NDYzNWMwNjIyNTVkMmNlY2E5YzI2ZDY1NGMyYzYxN2IyMjk2YjJlNzQyN2FkOTYyIiwKICAgICJwdWJsaWNDb21taXRtZW50UG9saWN5RGlnZXN0IjogImFlN2VjZDYyZDkzZjM4YTBkOGY5ZjRkOTdmNjAxNzM0Yjg4NjhkZGVkOGRmYjUyMjk4NDI1NjYxOGI3ZTYxOTciCiAgfSwKICAiY2hlY2twb2ludFdpdG5lc3NlcyI6IHsKICAgICJwb2xpY3lJZCI6ICJjaGVja3BvaW50X3dpdG5lc3NfcG9saWN5XzIwMjYwODI5IiwKICAgICJmYXVsdFRocmVzaG9sZCI6ICIxIiwKICAgICJxdW9ydW1UaHJlc2hvbGQiOiAiMyIsCiAgICAibWVtYmVycyI6IFsKICAgICAgewogICAgICAgICJwcmluY2lwYWxJZCI6ICJjaGVja3BvaW50X3dpdG5lc3NfMDBfMjAyNjA4MjkiLAogICAgICAgICJrZXlFcG9jaCI6ICIxIiwKICAgICAgICAia2V5RmluZ2VycHJpbnQiOiAiYTg3MmIxZDA2ZDMxYjgzYTlkMjc0ZmRhYjlmNTQ0Y2FiOGNmMDEyYjA0ZTI4MTVlZWYwM2ZjNWYyNTYzNTViYSIsCiAgICAgICAgInBvbGljeURpZ2VzdCI6ICI3YTYxMmNlNGM3OTZhZjQ2MDhhNmQ5ZTk4ZmY2NGU5M2EzYjJiM2JiZDg5NzBhZmY1ZWY3NjRlNzEwZGVlNTg5IiwKICAgICAgICAiYWRtaW5pc3RyYXRpb25EaWdlc3QiOiAiNmZjOWU0ZTVkMGU3YTU4OWJjNTM5Yjk3ZThjMzU2YzdhNmM5Y2IzNDcxODk3NjhjYTllZjUyMGNiZDY2ZGJlNiIKICAgICAgfSwKICAgICAgewogICAgICAgICJwcmluY2lwYWxJZCI6ICJjaGVja3BvaW50X3dpdG5lc3NfMDFfMjAyNjA4MjkiLAogICAgICAgICJrZXlFcG9jaCI6ICIxIiwKICAgICAgICAia2V5RmluZ2VycHJpbnQiOiAiZWJkMzNjZjBjMGNkMjNjZjg3NDgzYmQzMzUwYWYxYjA4ZWE4NWZhYjBiODQwODk0ZjhiMjNmYzBkNmQwMDBhYyIsCiAgICAgICAgInBvbGljeURpZ2VzdCI6ICI1ZjZkNGZhNmJjOTUzMjc2N2UzMjFhYWMwNDdmNDBkNmFmNzllNzA3MTBhMTE5MjY4M2MzNWQwMTcyNjY2YzBhIiwKICAgICAgICAiYWRtaW5pc3RyYXRpb25EaWdlc3QiOiAiYzM1OGI5ZjQ3NWNmYTk1NTNhMjNiM2ZhODBkYjRhNDg5OGNmYjA1NDE3NTAyMzBlOTJjOWFmM2I5OWFjYzk5NSIKICAgICAgfSwKICAgICAgewogICAgICAgICJwcmluY2lwYWxJZCI6ICJjaGVja3BvaW50X3dpdG5lc3NfMDJfMjAyNjA4MjkiLAogICAgICAgICJrZXlFcG9jaCI6ICIxIiwKICAgICAgICAia2V5RmluZ2VycHJpbnQiOiAiYTJiYjQxYWY1OTQ4MDU0N2EyOTZiZjliYzI1NmI2NDg0NTM0YmFiMmMwZDdlNWViOTBhMmU4MTJhM2RmNWU5MyIsCiAgICAgICAgInBvbGljeURpZ2VzdCI6ICI3YTZhODhkOWY3YWU0Y2M5ODEzYTRiZTM4YzdjNTc2MjQ3MjRjMDhlNGMzMWM1YmNiZmI5MGU0MTQ5NTM4N2E3IiwKICAgICAgICAiYWRtaW5pc3RyYXRpb25EaWdlc3QiOiAiZjAwNWY0ODBlMWVhNTczNWI1NmI0Y2Q4NDE0ZmZlZWY5ZTRiZTk0NDU2NjI4MDJmYzNmY2Q0MzAxMmI2NjU3YSIKICAgICAgfSwKICAgICAgewogICAgICAgICJwcmluY2lwYWxJZCI6ICJjaGVja3BvaW50X3dpdG5lc3NfMDNfMjAyNjA4MjkiLAogICAgICAgICJrZXlFcG9jaCI6ICIxIiwKICAgICAgICAia2V5RmluZ2VycHJpbnQiOiAiNmU0MzQ1ZjJkZWQ5NDgwZDg1ZTQ1ZDhmMWY4NzQ5NzZkMmY3NDY4N2U0M2I1ODhmMzczNzc1MTM1ZmZmYTAzZSIsCiAgICAgICAgInBvbGljeURpZ2VzdCI6ICI1NzVjNjg0YzI3MGM3MjBkNmQ4ZWE0ZGY3MTFhZjU0YjVkNzE2NDNjZmE2NGNhYTIyODEwZDJhNWU3ODhiN2ExIiwKICAgICAgICAiYWRtaW5pc3RyYXRpb25EaWdlc3QiOiAiMTMwNGE5NWZjMGQxZDE3OGYzNDZlNWUxNWU2NDI2ZDY3YjEwY2NkYzZhNTllYjU0ODEyMzYwODdjNWIzMmM2YyIKICAgICAgfQogICAgXQogIH0sCiAgInNlbWFudGljV2l0bmVzc2VzIjogewogICAgInBvbGljeUlkIjogInNlbWFudGljX3dpdG5lc3NfcG9saWN5XzIwMjYwODI5IiwKICAgICJmYXVsdFRocmVzaG9sZCI6ICIxIiwKICAgICJxdW9ydW1UaHJlc2hvbGQiOiAiMyIsCiAgICAibWVtYmVycyI6IFsKICAgICAgewogICAgICAgICJwcmluY2lwYWxJZCI6ICJzZW1hbnRpY193aXRuZXNzXzAwXzIwMjYwODI5IiwKICAgICAgICAia2V5RXBvY2giOiAiMSIsCiAgICAgICAgImtleUZpbmdlcnByaW50IjogIjBiMWExY2U3NjMzMjc0YTcwMjNiOTQyMDViMzgxMmUzMWQwYmRhYzViY2E0NWEzMzliNzU3Y2QxMzQ0NTRkOWEiLAogICAgICAgICJwb2xpY3lEaWdlc3QiOiAiOTBlYzA0MTU3Y2QzZmU0NmE3YjM0MzE1MzU1ZmQ3MDk0ODQxOTdjNDUzYjUwMjUxYTU0MDI1OWM1ZmZjZjNkMCIsCiAgICAgICAgImFkbWluaXN0cmF0aW9uRGlnZXN0IjogIjdhOGY5YjBiN2IyMzFjNTE0M2NhNzFhMmFkZmI1NjY0MTgxOWExZjdhOTE3YzRjMmJlZGRlYTY5YjhlZGEwN2UiCiAgICAgIH0sCiAgICAgIHsKICAgICAgICAicHJpbmNpcGFsSWQiOiAic2VtYW50aWNfd2l0bmVzc18wMV8yMDI2MDgyOSIsCiAgICAgICAgImtleUVwb2NoIjogIjEiLAogICAgICAgICJrZXlGaW5nZXJwcmludCI6ICJjYTU5MTkwYmY0YWUyMjE2MjU5ZmFlMTY4MDcwODBhZjVkNDhmMjBiMmM3NTE4MDVkMWUyMTQ0ZjAzZjVhYjUzIiwKICAgICAgICAicG9saWN5RGlnZXN0IjogImVhYWE5MTI1YTFhNGE0MjViMDRiY2M3NTUzYmM5ZDg0OTA4NzZkZDE1NGJkMjQwZDg1YWQwZmQyYjg3OGM5NDMiLAogICAgICAgICJhZG1pbmlzdHJhdGlvbkRpZ2VzdCI6ICJmN2ZiZWQ3OTVhYjk2MDI3OWY2NjkzYzA4YmI3YzczMzQ2ZTFiNWZlMDE4MTUwZGMzOTYxZDVlNTk1NTNkMjE3IgogICAgICB9LAogICAgICB7CiAgICAgICAgInByaW5jaXBhbElkIjogInNlbWFudGljX3dpdG5lc3NfMDJfMjAyNjA4MjkiLAogICAgICAgICJrZXlFcG9jaCI6ICIxIiwKICAgICAgICAia2V5RmluZ2VycHJpbnQiOiAiY2MxZDA0NjhiNGViNTgyY2FjY2EyNGQ0MjBhYzQ0ZTA0MTQ2MmM1NDY5ZDU4MjU2NGQwNzQ2ODA3MjFmYWMzMiIsCiAgICAgICAgInBvbGljeURpZ2VzdCI6ICI3ZTA3OWUxMTU2MzM3ZjNlNjAyNmM5YjFkNDdmMDBiM2E2NTNkN2ZmNGYwZjQzODk2OGJkYzY1MjA3YjdlM2IzIiwKICAgICAgICAiYWRtaW5pc3RyYXRpb25EaWdlc3QiOiAiZTdlZDQ4NTZhMjA0NGRlZGNlZTNjYWZjZDRkNDMwZTc1NTBiMTViMjI1NDg5MDUwMTRjODU0YzQ2N2M0ZTYyOSIKICAgICAgfSwKICAgICAgewogICAgICAgICJwcmluY2lwYWxJZCI6ICJzZW1hbnRpY193aXRuZXNzXzAzXzIwMjYwODI5IiwKICAgICAgICAia2V5RXBvY2giOiAiMSIsCiAgICAgICAgImtleUZpbmdlcnByaW50IjogIjA3NDc4YWUwZjg0NmYzNjA0OWVjMmI4YjUzZmNjMmZmNDdjYmRiZTFmZmM1NDAyMjcxMDQxNjI0M2QyMWY4MWEiLAogICAgICAgICJwb2xpY3lEaWdlc3QiOiAiZDc4M2I4MjllYTdlMGU2YzJlODA2ZTA2ZGVlNTgxM2ZlNDNiMmZmNmRkZjUxMDdhNmQ5NjIyZjM5ZDBkZGU2MSIsCiAgICAgICAgImFkbWluaXN0cmF0aW9uRGlnZXN0IjogImM1YmNkMmZjNzU2MjQyMDIyZjM3YmZhZGRmMTIxMmQ3NGVlYjc0Zjg0MjA4YjYxZjY5ODNiNDM2YWJjMDE3YzUiCiAgICAgIH0KICAgIF0KICB9LAogICJpbml0aWFsaXphdGlvbkFuY2hvciI6IHsKICAgICJwcmluY2lwYWxJZCI6ICJpbml0aWFsaXphdGlvbl9hbmNob3JfMjAyNjA4MjkiLAogICAgImtleUVwb2NoIjogIjEiLAogICAgImtleUZpbmdlcnByaW50IjogIjQ5M2Y3M2JlMDhiYzQ4MzI2YTA1ZGQxYWYzODNhMWVlNGFiNGE3ZjEzNTM4ZWFmZGI4YjUzNGE1ZWU5ZGE3ZmUiLAogICAgInBvbGljeURpZ2VzdCI6ICJkOWFiZDA0ZTY0MTVjNGYzM2NmMDNjNmNlZDRhZDUxOGRlMWZhZmUwZmYzYjM1MzRkY2E3NWRmZDk0OGZhYzJiIiwKICAgICJhZG1pbmlzdHJhdGlvbkRpZ2VzdCI6ICJiNTg0OWNlYjhkZTU0M2RiOGM3N2Y1ZGVkMWZiYWQ2YTIxZjNlMjBjMmI2ZTliMmFmY2EyYjE0NzY1NzlkOGNjIgogIH0sCiAgInJ1bm5lckVucm9sbG1lbnQiOiB7CiAgICAicHJpbmNpcGFsSWQiOiAicnVubmVyX2Vucm9sbG1lbnRfMjAyNjA4MjkiLAogICAgImtleUVwb2NoIjogIjEiLAogICAgImtleUZpbmdlcnByaW50IjogImY5MDZmNDU2MTkyMWQzNDQ1ZGI0MjM5M2M4NDhiMDA4ZjhhOWM1MzJkNWE4OTU5YWZiYzczNDYwNjY1MjdjODEiLAogICAgInBvbGljeURpZ2VzdCI6ICJlMGUyODI1NDk1YzVlMTdiNjMxMmZjMTAwZDNjNTVjNmEyM2Y4ZWExZjczYzU4OWNmZmNkZDg4ZTk2ZWE5OGE1IiwKICAgICJhZG1pbmlzdHJhdGlvbkRpZ2VzdCI6ICJiZmI2ZDRlYzRkYWRkNzIyMmMxZjY3MjljNjk2NmMxYjFkNTU1YzQ2ZDgzODBkNTEyZDdjMDBiYTZkYzFjYmVhIgogIH0sCiAgImRlcGxveW1lbnRBdHRlc3RvciI6IHsKICAgICJwcmluY2lwYWxJZCI6ICJkZXBsb3ltZW50X2F0dGVzdG9yXzIwMjYwODI5IiwKICAgICJrZXlFcG9jaCI6ICIxIiwKICAgICJrZXlGaW5nZXJwcmludCI6ICI0MWQyMjcwZDZhNTNmNDM5ZmJmMWUzZjQ5ZWUxYzFlNzM3MTQzNGIwZWVmMGUzMDNlMzBkNzRkZjlkNzcyMGQ2IiwKICAgICJwb2xpY3lEaWdlc3QiOiAiOTcxZGRmNWYyN2UwMTIxNDc4OTNkYzgyMmNhYWJhNTEzMDIyOGY4MDAwNWZkZDE4NGE3NDUyNWE0ZTdjMmNhYSIsCiAgICAiYWRtaW5pc3RyYXRpb25EaWdlc3QiOiAiMzRiNDFiYTFlYmIzOWRjMTcyNTUyODZjM2YwMTE1Zjk3OTRlMDA1ZWZiZGM4M2I0ZDNmYzk2ZTZkNDgwYTVjYSIKICB9LAogICJyZWFkaW5lc3NQb2xpY3lEaWdlc3QiOiAiZWI2NzUyODRkM2VmZWQ3ODY2NGI4MDM5MjE4YTk1NGFjMWM5OWI4NzQ1ODE3ZjI5MmMzZGZjNTcyOGYxYzBjMyIsCiAgInZlcmlmaWNhdGlvblNjb3BlIjogInRydXN0LXBpbnMtYW5kLXF1b3J1bS1tYXRoLW9ubHkiLAogICJleHRlcm5hbEFkbWluaXN0cmF0aW9uVmVyaWZpZWQiOiBmYWxzZSwKICAiZGVwbG95bWVudEF0dGVzdGF0aW9uVmVyaWZpZWQiOiBmYWxzZSwKICAiY2hlY2twb2ludFdpdG5lc3NRdW9ydW1WZXJpZmllZCI6IGZhbHNlLAogICJzZW1hbnRpY1dpdG5lc3NRdW9ydW1WZXJpZmllZCI6IGZhbHNlLAogICJzdGF0ZVRyYW5zaXRpb25BdXRob3JpemVkIjogZmFsc2UsCiAgImF0dGVtcHRTdGFydEF1dGhvcml6ZWQiOiBmYWxzZSwKICAiY2FwdHVyZUF1dGhvcml6ZWQiOiBmYWxzZSwKICAiY29uZmlndXJhdGlvbkRpZ2VzdCI6ICI5MGNlODYxMTAzZWEwYmNhY2ViYzM3ZWU5N2Y1MDJmYTUwODBhYjVlOWFiMWI1NDJjMTUzOTNiOTZmYWM0YzAyIgp9Cg",
    "serializedConfigurationSha256": "4152175917e9441505ac17611dbd064181defe6fbe4620bd550d773fcbf59d48",
    "projectPrincipalId": "project_client_20260829",
    "projectAuthenticationPolicyDigest": "39872817331eed587ca8150e5c6a9712e3f0e9fbfc4acfab1622d01849af95d5",
    "servicePrincipalId": "supervisor_service_20260829",
    "serviceKeyEpoch": "1",
    "serviceKeyFingerprint": "06e3fd8fda29bb60ab59557de61edb0aecdb231134be30e75b455f8e1b792fa9",
    "serviceSigningSpkiDer": "MCowBQYDK2VwAyEA11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo",
    "genesisSemanticReceiptDigest": "8ab65ed0931fcb6b341cec2e22ce75ce131954c40822987815a3904eb5cebd91"
  },
  "authorityStateIdentity": {
    "projectAuthorityDigest": "9ba3490c3ff9becc86163de81360b6aa1ea64e9e2f7098ec72daefa5a66b77bf",
    "projectScopeRole": "sf_supervisor_project_scope_v1",
    "singletonKey": true,
    "activeConfigurationEpoch": "0",
    "activeConfigurationDigest": "90ce861103ea0bcacebc37ee97f502fa5080ab5e9ab1b542c15393b96fac4c02",
    "authorityHeadDigest": "80b825d44f308c1ef66d92129f2c34e5b05e88ee839ec7ca91bf2301f2013146"
  }
}
`;
const EXPECTED_RECORD = deepFreezeV1(JSON.parse(SEALED_TEXT) as Record<string, unknown>);

export interface PostgresAuthoritySeedConfigurationInsertProjectionV1 {
  readonly projectAuthorityDigest: string;
  readonly projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1;
  readonly configurationEpoch: string;
  readonly configurationDigest: string;
  readonly genesisAuthorityHeadDigest: string;
  readonly serializedConfiguration: string;
  readonly serializedConfigurationSha256: string;
  readonly projectPrincipalId: string;
  readonly projectAuthenticationPolicyDigest: string;
  readonly servicePrincipalId: string;
  readonly serviceKeyEpoch: string;
  readonly serviceKeyFingerprint: string;
  readonly serviceSigningSpkiDer: string;
  readonly genesisSemanticReceiptDigest: string;
}

export interface PostgresAuthoritySeedStateInsertProjectionV1 {
  readonly projectAuthorityDigest: string;
  readonly projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1;
  readonly singletonKey: true;
  readonly activeConfigurationEpoch: string;
  readonly activeConfigurationDigest: string;
  readonly authorityHeadDigest: string;
}

export interface PostgresAuthoritySeedInsertProjectionV1 {
  readonly authorityConfiguration: PostgresAuthoritySeedConfigurationInsertProjectionV1;
  readonly authorityStateIdentity: PostgresAuthoritySeedStateInsertProjectionV1;
}

interface AuthoritySeedStateV1 {
  readonly bytes: Uint8Array;
  readonly insertProjection: PostgresAuthoritySeedInsertProjectionV1;
}

const HANDLES = new WeakMap<object, Readonly<AuthoritySeedStateV1>>();

export interface ParsedPostgresAuthoritySeedV1 {
  readonly seedKind: 'postgresql-authority-seed-v1';
  readonly rawByteLength: typeof POSTGRES_AUTHORITY_SEED_BYTES_V1;
  readonly rawSha256: typeof POSTGRES_AUTHORITY_SEED_SHA256_V1;
  readonly authority: 'none';
  readonly readinessAuthorized: false;
}

export function parseSealedPostgresAuthoritySeedV1(): ParsedPostgresAuthoritySeedV1 {
  return parsePostgresAuthoritySeedCandidateV1(new TextEncoder().encode(SEALED_TEXT));
}

export function parsePostgresAuthoritySeedCandidateV1(
  value: unknown,
): ParsedPostgresAuthoritySeedV1 {
  try {
    const bytes = snapshotBytesV1(
      value, 'PostgreSQL authority seed',
      POSTGRES_AUTHORITY_SEED_BYTES_V1,
      POSTGRES_AUTHORITY_SEED_BYTES_V1,
    );
    const parsed = parsePostgresMigrationJsonBytesV1(bytes, 'seed');
    if (parsed.sha256 !== POSTGRES_AUTHORITY_SEED_SHA256_V1
      || JSON.stringify(parsed.record) !== JSON.stringify(EXPECTED_RECORD)) throw new TypeError();
    const insertProjection = validateSeed(parsed.record);
    const handle = Object.freeze({
      seedKind: 'postgresql-authority-seed-v1' as const,
      rawByteLength: POSTGRES_AUTHORITY_SEED_BYTES_V1,
      rawSha256: POSTGRES_AUTHORITY_SEED_SHA256_V1,
      authority: 'none' as const,
      readinessAuthorized: false as const,
    });
    HANDLES.set(handle, Object.freeze({ bytes, insertProjection }));
    return handle;
  } catch {
    throw new TypeError(INVALID);
  }
}

export function assertPostgresAuthoritySeedHandleV1(
  value: unknown,
): asserts value is ParsedPostgresAuthoritySeedV1 {
  try {
    if (isProxy(value) || value === null || typeof value !== 'object'
      || !HANDLES.has(value)) throw new TypeError();
  } catch {
    throw new TypeError(INVALID);
  }
}

export function copyPostgresAuthoritySeedBytesV1(value: unknown): Uint8Array {
  assertPostgresAuthoritySeedHandleV1(value);
  return Uint8Array.from(HANDLES.get(value)!.bytes);
}

export function postgresAuthoritySeedInsertProjectionV1(
  value: unknown,
): PostgresAuthoritySeedInsertProjectionV1 {
  assertPostgresAuthoritySeedHandleV1(value);
  return HANDLES.get(value)!.insertProjection;
}

function validateSeed(
  root: Readonly<Record<string, unknown>>,
): PostgresAuthoritySeedInsertProjectionV1 {
  exactKeysV1(root, [
    'domain', 'schemaVersion', 'authorityConfiguration', 'authorityStateIdentity',
  ], 'authority seed');
  if (root.domain !== POSTGRES_AUTHORITY_SEED_DOMAIN_V1 || root.schemaVersion !== 1) {
    throw new TypeError();
  }
  const configurationRow = closedRecordV1(root.authorityConfiguration, 'authority seed config');
  exactKeysV1(configurationRow, [
    'projectAuthorityDigest', 'projectScopeRole', 'configurationEpoch',
    'configurationDigest', 'genesisAuthorityHeadDigest', 'serializedConfiguration',
    'serializedConfigurationSha256', 'projectPrincipalId',
    'projectAuthenticationPolicyDigest', 'servicePrincipalId', 'serviceKeyEpoch',
    'serviceKeyFingerprint', 'serviceSigningSpkiDer', 'genesisSemanticReceiptDigest',
  ], 'authority seed config');
  const state = closedRecordV1(root.authorityStateIdentity, 'authority seed state');
  exactKeysV1(state, [
    'projectAuthorityDigest', 'projectScopeRole', 'singletonKey',
    'activeConfigurationEpoch', 'activeConfigurationDigest', 'authorityHeadDigest',
  ], 'authority seed state');

  const serializedConfiguration = decodeBase64Url(
    configurationRow.serializedConfiguration, 131_072,
  );
  const configuration = parseCanonicalPrettyJsonBytesV1(
    serializedConfiguration.bytes, 'sealed authority configuration', 131_072,
  );
  const configurationDigest = parseDigestV1(
    configurationRow.configurationDigest, 'seed configuration digest',
  );
  const serializedConfigurationSha256 = parseDigestV1(
    configurationRow.serializedConfigurationSha256, 'config byte digest',
  );
  const configurationEpoch = parseUint64V1(
    configurationRow.configurationEpoch, 'seed epoch',
  );
  const claimedConfigurationDigest = parseDigestV1(
    configuration.configurationDigest, 'configuration digest',
  );
  const configurationBody = Object.fromEntries(
    Object.entries(configuration).filter(([key]) => key !== 'configurationDigest'),
  );
  if (configurationDigest !== claimedConfigurationDigest
    || configurationDigest !== canonicalDigestHexV1({
      domain: CONFIGURATION_DIGEST_DOMAIN,
      configuration: configurationBody,
    })
    || rawSha256HexV1(serializedConfiguration.bytes) !== serializedConfigurationSha256
    || configurationEpoch !== '0') {
    throw new TypeError();
  }

  const project = closedRecordV1(configuration.project, 'configuration project');
  const principal = closedRecordV1(project.principal, 'configuration project principal');
  const service = closedRecordV1(configuration.service, 'configuration service');
  const servicePrincipal = closedRecordV1(service.principal, 'configuration service principal');
  const projectDigest = parseDigestV1(
    configurationRow.projectAuthorityDigest, 'seed project digest',
  );
  const projectPrincipalId = parseOpaqueIdV1(
    configurationRow.projectPrincipalId, 'seed project principal',
  );
  const projectAuthenticationPolicyDigest = parseDigestV1(
    configurationRow.projectAuthenticationPolicyDigest, 'seed auth policy',
  );
  const servicePrincipalId = parseOpaqueIdV1(
    configurationRow.servicePrincipalId, 'seed service principal',
  );
  const serviceKeyEpoch = parseUint64V1(
    configurationRow.serviceKeyEpoch, 'seed service epoch', 1n,
  );
  const serviceFingerprint = parseDigestV1(
    configurationRow.serviceKeyFingerprint, 'seed service fingerprint',
  );
  const serviceSigningSpkiDer = decodeBase64Url(
    configurationRow.serviceSigningSpkiDer, 44, 44,
  );
  validateEd25519SpkiV1(
    serviceSigningSpkiDer.bytes, serviceFingerprint,
  );
  const genesisSemanticReceiptDigest = parseDigestV1(
    configurationRow.genesisSemanticReceiptDigest, 'seed genesis receipt',
  );
  if (projectDigest !== parseDigestV1(project.projectAuthorityDigest, 'configuration project')
    || projectPrincipalId
      !== parseOpaqueIdV1(principal.principalId, 'configuration project principal')
    || projectAuthenticationPolicyDigest
      !== parseDigestV1(project.authenticationPolicyDigest, 'configuration auth policy')
    || servicePrincipalId
      !== parseOpaqueIdV1(servicePrincipal.principalId, 'configuration service principal')
    || serviceKeyEpoch
      !== parseUint64V1(servicePrincipal.keyEpoch, 'configuration service epoch', 1n)
    || serviceFingerprint
      !== parseDigestV1(servicePrincipal.keyFingerprint, 'configuration service fingerprint')) {
    throw new TypeError();
  }

  const head = canonicalDigestHexV1({
    domain: GENESIS_HEAD_DOMAIN,
    configurationEpoch: '0',
    configurationDigest,
  });
  const genesisAuthorityHeadDigest = parseDigestV1(
    configurationRow.genesisAuthorityHeadDigest, 'seed head',
  );
  const stateProjectDigest = parseDigestV1(
    state.projectAuthorityDigest, 'seed state project digest',
  );
  const stateActiveConfigurationEpoch = parseUint64V1(
    state.activeConfigurationEpoch, 'seed state active epoch',
  );
  const stateActiveConfigurationDigest = parseDigestV1(
    state.activeConfigurationDigest, 'seed state active configuration digest',
  );
  const stateAuthorityHeadDigest = parseDigestV1(
    state.authorityHeadDigest, 'seed state authority head digest',
  );
  const projectScopeRole = configurationRow.projectScopeRole;
  const stateProjectScopeRole = state.projectScopeRole;
  const singletonKey = state.singletonKey;
  if (head !== genesisAuthorityHeadDigest
    || singletonKey !== true
    || projectScopeRole !== POSTGRES_PROJECT_SCOPE_ROLE_V1
    || stateProjectScopeRole !== POSTGRES_PROJECT_SCOPE_ROLE_V1
    || stateProjectDigest !== projectDigest
    || stateActiveConfigurationEpoch !== '0'
    || stateActiveConfigurationDigest !== configurationDigest
    || stateAuthorityHeadDigest !== head) throw new TypeError();

  return deepFreezeV1({
    authorityConfiguration: {
      projectAuthorityDigest: projectDigest,
      projectScopeRole,
      configurationEpoch,
      configurationDigest,
      genesisAuthorityHeadDigest,
      serializedConfiguration: serializedConfiguration.text,
      serializedConfigurationSha256,
      projectPrincipalId,
      projectAuthenticationPolicyDigest,
      servicePrincipalId,
      serviceKeyEpoch,
      serviceKeyFingerprint: serviceFingerprint,
      serviceSigningSpkiDer: serviceSigningSpkiDer.text,
      genesisSemanticReceiptDigest,
    },
    authorityStateIdentity: {
      projectAuthorityDigest: stateProjectDigest,
      projectScopeRole: stateProjectScopeRole,
      singletonKey,
      activeConfigurationEpoch: stateActiveConfigurationEpoch,
      activeConfigurationDigest: stateActiveConfigurationDigest,
      authorityHeadDigest: stateAuthorityHeadDigest,
    },
  });
}

function decodeBase64Url(
  value: unknown,
  maximum: number,
  exact?: number,
): Readonly<{ text: string; bytes: Uint8Array }> {
  if (typeof value !== 'string' || value.length === 0
    || !/^[A-Za-z0-9_-]+$/.test(value)) throw new TypeError();
  const decoded = Uint8Array.from(Buffer.from(value, 'base64url'));
  if (decoded.byteLength > maximum || (exact !== undefined && decoded.byteLength !== exact)
    || Buffer.from(decoded).toString('base64url') !== value) throw new TypeError();
  return Object.freeze({ text: value, bytes: decoded });
}
