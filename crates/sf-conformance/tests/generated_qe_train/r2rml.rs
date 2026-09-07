pub(super) fn build(
    fixture_id: usize,
    item_table: &str,
    group_table: &str,
    language: &str,
) -> String {
    let base = format!("http://example.com/generated/f{fixture_id}/");
    format!(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix qe: <http://example.com/qe/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<#Item>
  rr:logicalTable [ rr:tableName "{item_table}" ] ;
  rr:subjectMap [ rr:template "{base}item/{{id}}" ] ;
  rr:predicateObjectMap [ rr:predicate qe:label ; rr:objectMap [ rr:column "label" ] ] ;
  rr:predicateObjectMap [ rr:predicate qe:bucket ; rr:objectMap [ rr:column "bucket" ] ] ;
  rr:predicateObjectMap [ rr:predicate qe:score ; rr:objectMap [ rr:column "score" ; rr:datatype xsd:integer ] ] ;
  rr:predicateObjectMap [ rr:predicate qe:note ; rr:objectMap [ rr:column "note" ; rr:language "{language}" ] ] ;
  rr:predicateObjectMap [ rr:predicate qe:peer ; rr:objectMap [ rr:template "{base}item/{{peer}}" ] ] ;
  rr:predicateObjectMap [ rr:predicate qe:kind ; rr:objectMap [ rr:constant qe:Entity ] ] ;
  rr:predicateObjectMap [
    rr:predicate qe:group ;
    rr:objectMap [
      rr:parentTriplesMap <#Group> ;
      rr:joinCondition [ rr:child "gid" ; rr:parent "gid" ]
    ]
  ] .
<#Group>
  rr:logicalTable [ rr:tableName "{group_table}" ] ;
  rr:subjectMap [ rr:template "{base}group/{{gid}}" ] ;
  rr:predicateObjectMap [ rr:predicate qe:groupName ; rr:objectMap [ rr:column "title" ] ] .
"#
    )
}
