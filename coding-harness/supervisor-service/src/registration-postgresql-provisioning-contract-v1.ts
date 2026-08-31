// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import {
  deepFreezeV1,
  snapshotBytesV1,
} from './registration-postgresql-canonical-v1.js';
import {
  parsePostgresMigrationJsonBytesV1,
} from './registration-postgresql-migration-json-v1.js';

export const POSTGRES_PROVISIONING_CONTRACT_BYTES_V1 = 8_657;
export const POSTGRES_PROVISIONING_CONTRACT_SHA256_V1 =
  '71e4bafda6f97f44b54f28903363fe4ff88f3199a2d08b4dc4bc9060c33e55a9';

const INVALID = 'PostgreSQL provisioning contract is invalid';
const EXPECTED_TEXT = `{
  "domain": "semantic-fabric/programme-capture/supervisor-postgresql-provisioning-contract-v1",
  "schemaVersion": 1,
  "authority": "none",
  "readinessAuthorized": false,
  "database": {
    "name": "sf_supervisor_v1",
    "ownerRole": "sf_supervisor_owner_v1",
    "aclState": "explicit",
    "privileges": [
      {
        "grantorRole": "sf_supervisor_owner_v1",
        "granteeKind": "role",
        "granteeRole": "sf_supervisor_migration_login_v1",
        "privilege": "CONNECT",
        "grantable": false
      },
      {
        "grantorRole": "sf_supervisor_owner_v1",
        "granteeKind": "role",
        "granteeRole": "sf_supervisor_owner_v1",
        "privilege": "CONNECT",
        "grantable": false
      },
      {
        "grantorRole": "sf_supervisor_owner_v1",
        "granteeKind": "role",
        "granteeRole": "sf_supervisor_owner_v1",
        "privilege": "CREATE",
        "grantable": false
      },
      {
        "grantorRole": "sf_supervisor_owner_v1",
        "granteeKind": "role",
        "granteeRole": "sf_supervisor_owner_v1",
        "privilege": "TEMPORARY",
        "grantable": false
      },
      {
        "grantorRole": "sf_supervisor_owner_v1",
        "granteeKind": "role",
        "granteeRole": "sf_supervisor_readiness_login_v1",
        "privilege": "CONNECT",
        "grantable": false
      },
      {
        "grantorRole": "sf_supervisor_owner_v1",
        "granteeKind": "role",
        "granteeRole": "sf_supervisor_recovery_login_v1",
        "privilege": "CONNECT",
        "grantable": false
      },
      {
        "grantorRole": "sf_supervisor_owner_v1",
        "granteeKind": "role",
        "granteeRole": "sf_supervisor_writer_login_v1",
        "privilege": "CONNECT",
        "grantable": false
      }
    ]
  },
  "dedicatedSchema": {
    "name": "sf_supervisor_v1",
    "ownerRole": "sf_supervisor_owner_v1"
  },
  "publicSchema": {
    "name": "public",
    "ownerRole": "pg_database_owner",
    "aclState": "explicit",
    "privileges": [
      {
        "grantorRole": "pg_database_owner",
        "granteeKind": "role",
        "granteeRole": "pg_database_owner",
        "privilege": "CREATE",
        "grantable": false
      },
      {
        "grantorRole": "pg_database_owner",
        "granteeKind": "role",
        "granteeRole": "pg_database_owner",
        "privilege": "USAGE",
        "grantable": false
      }
    ]
  },
  "roles": [
    {
      "name": "sf_supervisor_migration_login_v1",
      "login": true,
      "superuser": false,
      "inherit": false,
      "createRole": false,
      "createDatabase": false,
      "replication": false,
      "bypassRls": false,
      "connectionLimit": -1,
      "validUntil": null,
      "settings": []
    },
    {
      "name": "sf_supervisor_owner_v1",
      "login": false,
      "superuser": false,
      "inherit": false,
      "createRole": false,
      "createDatabase": false,
      "replication": false,
      "bypassRls": false,
      "connectionLimit": -1,
      "validUntil": null,
      "settings": []
    },
    {
      "name": "sf_supervisor_project_scope_v1",
      "login": false,
      "superuser": false,
      "inherit": false,
      "createRole": false,
      "createDatabase": false,
      "replication": false,
      "bypassRls": false,
      "connectionLimit": -1,
      "validUntil": null,
      "settings": []
    },
    {
      "name": "sf_supervisor_readiness_capability_v1",
      "login": false,
      "superuser": false,
      "inherit": false,
      "createRole": false,
      "createDatabase": false,
      "replication": false,
      "bypassRls": false,
      "connectionLimit": -1,
      "validUntil": null,
      "settings": []
    },
    {
      "name": "sf_supervisor_readiness_login_v1",
      "login": true,
      "superuser": false,
      "inherit": false,
      "createRole": false,
      "createDatabase": false,
      "replication": false,
      "bypassRls": false,
      "connectionLimit": -1,
      "validUntil": null,
      "settings": []
    },
    {
      "name": "sf_supervisor_recovery_capability_v1",
      "login": false,
      "superuser": false,
      "inherit": false,
      "createRole": false,
      "createDatabase": false,
      "replication": false,
      "bypassRls": false,
      "connectionLimit": -1,
      "validUntil": null,
      "settings": []
    },
    {
      "name": "sf_supervisor_recovery_login_v1",
      "login": true,
      "superuser": false,
      "inherit": false,
      "createRole": false,
      "createDatabase": false,
      "replication": false,
      "bypassRls": false,
      "connectionLimit": -1,
      "validUntil": null,
      "settings": []
    },
    {
      "name": "sf_supervisor_writer_capability_v1",
      "login": false,
      "superuser": false,
      "inherit": false,
      "createRole": false,
      "createDatabase": false,
      "replication": false,
      "bypassRls": false,
      "connectionLimit": -1,
      "validUntil": null,
      "settings": []
    },
    {
      "name": "sf_supervisor_writer_login_v1",
      "login": true,
      "superuser": false,
      "inherit": false,
      "createRole": false,
      "createDatabase": false,
      "replication": false,
      "bypassRls": false,
      "connectionLimit": -1,
      "validUntil": null,
      "settings": []
    }
  ],
  "memberships": [
    {
      "grantedRole": "sf_supervisor_owner_v1",
      "memberRole": "sf_supervisor_migration_login_v1",
      "grantorRole": "sf_supervisor_bootstrap_admin_v1",
      "adminOption": false,
      "inheritOption": false,
      "setOption": true
    },
    {
      "grantedRole": "sf_supervisor_project_scope_v1",
      "memberRole": "sf_supervisor_readiness_login_v1",
      "grantorRole": "sf_supervisor_bootstrap_admin_v1",
      "adminOption": false,
      "inheritOption": false,
      "setOption": false
    },
    {
      "grantedRole": "sf_supervisor_project_scope_v1",
      "memberRole": "sf_supervisor_recovery_login_v1",
      "grantorRole": "sf_supervisor_bootstrap_admin_v1",
      "adminOption": false,
      "inheritOption": false,
      "setOption": false
    },
    {
      "grantedRole": "sf_supervisor_project_scope_v1",
      "memberRole": "sf_supervisor_writer_login_v1",
      "grantorRole": "sf_supervisor_bootstrap_admin_v1",
      "adminOption": false,
      "inheritOption": false,
      "setOption": false
    },
    {
      "grantedRole": "sf_supervisor_readiness_capability_v1",
      "memberRole": "sf_supervisor_readiness_login_v1",
      "grantorRole": "sf_supervisor_bootstrap_admin_v1",
      "adminOption": false,
      "inheritOption": false,
      "setOption": false
    },
    {
      "grantedRole": "sf_supervisor_recovery_capability_v1",
      "memberRole": "sf_supervisor_recovery_login_v1",
      "grantorRole": "sf_supervisor_bootstrap_admin_v1",
      "adminOption": false,
      "inheritOption": false,
      "setOption": false
    },
    {
      "grantedRole": "sf_supervisor_writer_capability_v1",
      "memberRole": "sf_supervisor_writer_login_v1",
      "grantorRole": "sf_supervisor_bootstrap_admin_v1",
      "adminOption": false,
      "inheritOption": false,
      "setOption": false
    }
  ],
  "clusterGuards": {
    "forbiddenExplicitMembershipRolePrefix": "pg_",
    "implicitDatabaseOwnerRole": "pg_database_owner",
    "parameterPrivileges": [],
    "tablespacePrivileges": [],
    "systemPrivilegeBaseline": {
      "profile": "postgresql-16.15-clean-template0-public-object-acl-v1",
      "recordCount": 4059,
      "recordsBytes": 860988,
      "recordsSha256": "a108e05f9cfd6d6485a86fe198a87b3800e21986b5c62e6251519de6577d05be"
    },
    "databasePrivilegeRule": "exact-database-privileges-v1",
    "serviceOwnershipRule": "database-and-dedicated-schema-only-v1",
    "effectiveObjectPrivilegeRule": "system-baseline-or-dedicated-schema-only-v1",
    "defaultAclRule": "catalogue-default-acls-only-v1",
    "inboundGrantRule": "no-inbound-grants-outside-dedicated-schema-v1"
  },
  "runtimeCredentialAbsence": {
    "migrationRole": "sf_supervisor_migration_login_v1",
    "runtimeProfileIds": [
      "semantic-fabric-parent-runtime-v1",
      "semantic-fabric-supervisor-readiness-runtime-v1",
      "semantic-fabric-supervisor-recovery-runtime-v1",
      "semantic-fabric-supervisor-writer-runtime-v1"
    ],
    "evidenceRule": "external-profile-observation-required-v1"
  },
  "limits": {
    "maximumBytes": 65536,
    "maximumDepth": 8,
    "maximumNodes": 2048,
    "maximumRecords": 512,
    "maximumCollectionWidth": 64,
    "maximumObjectKeys": 16,
    "maximumStringBytes": 4096,
    "maximumIdentifierBytes": 63
  }
}
`;
const EXPECTED_RECORD = deepFreezeV1(JSON.parse(EXPECTED_TEXT) as Record<string, unknown>);
const HANDLES = new WeakMap<object, Readonly<{ bytes: Uint8Array }>>();

export interface ParsedPostgresProvisioningContractV1 {
  readonly contractKind: 'postgresql-provisioning-contract-v1';
  readonly rawByteLength: typeof POSTGRES_PROVISIONING_CONTRACT_BYTES_V1;
  readonly rawSha256: typeof POSTGRES_PROVISIONING_CONTRACT_SHA256_V1;
  readonly authority: 'none';
  readonly readinessAuthorized: false;
}

export function parsePostgresProvisioningContractV1(
  value: unknown,
): ParsedPostgresProvisioningContractV1 {
  try {
    const bytes = snapshotBytesV1(
      value, 'PostgreSQL provisioning contract',
      POSTGRES_PROVISIONING_CONTRACT_BYTES_V1,
      POSTGRES_PROVISIONING_CONTRACT_BYTES_V1,
    );
    const parsed = parsePostgresMigrationJsonBytesV1(bytes, 'provisioning');
    if (parsed.sha256 !== POSTGRES_PROVISIONING_CONTRACT_SHA256_V1
      || JSON.stringify(parsed.record) !== JSON.stringify(EXPECTED_RECORD)) throw new TypeError();
    const handle = Object.freeze({
      contractKind: 'postgresql-provisioning-contract-v1' as const,
      rawByteLength: POSTGRES_PROVISIONING_CONTRACT_BYTES_V1,
      rawSha256: POSTGRES_PROVISIONING_CONTRACT_SHA256_V1,
      authority: 'none' as const,
      readinessAuthorized: false as const,
    });
    HANDLES.set(handle, Object.freeze({ bytes }));
    return handle;
  } catch {
    throw new TypeError(INVALID);
  }
}

export function assertPostgresProvisioningContractHandleV1(
  value: unknown,
): asserts value is ParsedPostgresProvisioningContractV1 {
  try {
    if (isProxy(value) || value === null || typeof value !== 'object'
      || !HANDLES.has(value)) throw new TypeError();
  } catch {
    throw new TypeError(INVALID);
  }
}

export function copyPostgresProvisioningContractBytesV1(value: unknown): Uint8Array {
  assertPostgresProvisioningContractHandleV1(value);
  return Uint8Array.from(HANDLES.get(value)!.bytes);
}
