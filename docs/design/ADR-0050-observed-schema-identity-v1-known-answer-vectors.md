# ADR-0050 Appendix B: Observed Schema Identity V1 known-answer vectors

**Normative authority:** This document is Appendix B of
[ADR-0050](../adr/ADR-0050-verified-source-generation-leases-schema-identity-and-atomic-runtime-activation.md).
It has no independent status. [Appendix A](./ADR-0050-observed-schema-identity-v1-contract.md)
defines every notation and encoding used below.

Hex strings are lowercase; presentation line breaks and the `||` notation are
not bytes.

## B.1 Empty observation

```text
structural profile = "portable-base-v1"
type profile       = "portable-type-v1"
constraint profile = "portable-constraint-v1"
relations          = []
constraints        = []
```

Each body is exactly `TXT(profile) || U32(0)`.

| Digest | Body bytes | Preimage bytes | Expected SHA-256 |
|---|---:|---:|---|
| Structural | 24 | 78 | `4934b444efc6178b895c553448fb486c50ff6c16feaf3179535d184bf885f1c5` |
| Type | 24 | 72 | `bcf3964b5b48a6c5a6fc2786dd15cf333a8f193756b5b8a7b282665544369486` |
| Constraint | 30 | 84 | `0f8a5a297f308b392f87d9359aafa0a67fd3bc71385448524471e8de671b2884` |

## B.2 Non-empty observation

Supply input deliberately in noncanonical order:

```text
profiles = {
  structural: "kat-struct-v1",
  types: "kat-type-v1",
  constraints: "kat-constraint-v1"
}

relations = [
  app.staff {
    1: id      -> std.i32, SignedInteger, facets { bits: U64(32) }
    2: dept_id -> std.i32, SignedInteger, facets { bits: U64(32) }
  },
  app.dept {
    1: id      -> std.i32, SignedInteger, facets { bits: U64(32) }
  }
]

constraints = [
  FK app.staff(dept_id) -> app.dept(id), validated, enforced, SIMPLE,
  UNIQUE app.staff(dept_id), validated, enforced, NULLs distinct,
  PK app.staff(id), validated, enforced,
  NOT NULL app.staff.id, validated, enforced,
  PK app.dept(id), validated, enforced,
  NOT NULL app.dept.id, validated, enforced
]
```

Every qualified name has `catalog=None`; relations have `schema=Some("app")`;
types have `schema=Some("std")`. All relations are base tables.

### Structural preimage

```text
STRUCTURAL_DOMAIN
|| 000000000000006a
|| 0000000d6b61742d7374727563742d763100000002110001000000036170700000000464657074010000000112000000010000000269641100010000000361707000000005737461666601000000021200000001000000026964120000000200000007646570745f6964
```

Expected preimage length: 160 bytes. Expected SHA-256:

```text
ddd247fe2adf8831e101a1273e3692c5586370da31c8877d403f75e948fa4f38
```

### Type preimage

```text
TYPE_DOMAIN
|| 00000000000000e6
|| 0000000b6b61742d747970652d763100000003210001000000036170700000000464657074010000000100000002696400010000000373746400000003693332020000000122000000046269747302000000000000002021000100000003617070000000057374616666010000000100000002696400010000000373746400000003693332020000000122000000046269747302000000000000002021000100000003617070000000057374616666010000000200000007646570745f6964000100000003737464000000036933320200000001220000000462697473020000000000000020
```

Expected preimage length: 278 bytes. Expected SHA-256:

```text
9b6ef1560bbe702ceb44b5bf0d2c46a2e1c5e88ceb1c134c334c3910f84e9574
```

### Constraint preimage

```text
CONSTRAINT_DOMAIN
|| 000000000000010f
|| 000000116b61742d636f6e73747261696e742d76310000000631000100000003617070000000046465707401000000010000000269640101310001000000036170700000000573746166660100000001000000026964010132000100000003617070000000046465707401010100000001000000010000000269643200010000000361707000000005737461666601010100000001000000010000000269643300010000000361707000000005737461666601010101000000010000000200000007646570745f69643400010000000361707000000005737461666601000100000003617070000000046465707401010101000000010000000200000007646570745f696400000001000000026964
```

Expected preimage length: 325 bytes. Expected SHA-256:

```text
478af9a49cbceddc91b2e0b75cdfc1fc88a4a02f166bb6ee4b1b1005870beeb2
```

## B.3 Required test shape

Tests must check each vector both as whole preimage bytes and as streamed encoder
output. Mutation tests cover every typed-enum tag, boolean, option, integer
endian, list count, ordering rule, cap, duplicate rule and expected-preimage
mismatch; V1 has no raw-byte decoder or trailing-byte input. The non-empty input
must also be permuted to prove only the specified unordered inputs are
invariant; PK order, FK pair order and facet-list order remain semantic.
Additional vectors cover `Some(catalog)`, false state, negative `I64`, every
facet-value variant, and facet keys `b`/`aa` to distinguish raw-key order from
length-prefixed order. All six profile IDs used above are test-only reservations
and are not product profile registrations.
