use crate::utils::{is_name_char, is_name_start_char};
use oxiri::Iri;
#[cfg(feature = "rdf-12")]
use oxrdf::BaseDirection;
use oxrdf::vocab::{rdf, xsd};
use oxrdf::{NamedNodeRef, NamedOrBlankNode, NamedOrBlankNodeRef, TermRef, TripleRef};
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io;

const RESERVED_SYNTAX_TERMS: [&str; 9] = [
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#Description",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#li",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#RDF",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#ID",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#about",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#parseType",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#resource",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#nodeID",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#datatype",
];

pub struct InnerRdfXmlWriter {
    current_subject: Option<NamedOrBlankNode>,
    current_resource_tag: Option<String>,
    custom_default_prefix: bool,
    prefixes_by_iri: BTreeMap<String, String>,
    base_iri: Option<Iri<String>>,
}

impl InnerRdfXmlWriter {
    pub(super) fn new(
        mut prefixes: BTreeMap<String, String>,
        base_iri: Option<Iri<String>>,
    ) -> Self {
        // Makes sure 'rdf' and 'its' are the proper prefixes, by first removing them
        prefixes.remove("rdf");
        prefixes.remove("its");
        let custom_default_prefix = prefixes.contains_key("");
        // The serializer want to have the URL first, we swap
        let mut prefixes = prefixes
            .into_iter()
            .map(|(key, value)| (value, key))
            .collect::<BTreeMap<_, _>>();
        prefixes.insert(
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#".into(),
            "rdf".into(),
        );
        prefixes.insert("http://www.w3.org/2005/11/its".into(), "its".into());
        Self {
            current_subject: None,
            current_resource_tag: None,
            custom_default_prefix,
            prefixes_by_iri: prefixes,
            base_iri,
        }
    }

    pub(super) fn serialize_triple<'a>(
        &mut self,
        t: impl Into<TripleRef<'a>>,
        output: &mut Vec<Event<'a>>,
    ) -> io::Result<()> {
        if self.current_subject.is_none() {
            self.write_start(output);
        }

        let triple = t.into();
        // We open a new rdf:Description if useful
        if self.current_subject.as_ref().map(NamedOrBlankNode::as_ref) != Some(triple.subject) {
            if self.current_subject.is_some() {
                output.push(Event::End(
                    self.current_resource_tag
                        .take()
                        .map_or_else(|| BytesEnd::new("rdf:Description"), BytesEnd::new),
                ));
            }
            self.current_subject = Some(triple.subject.into_owned());

            let (mut description_start, with_type_tag) = if triple.predicate == rdf::TYPE {
                if let TermRef::NamedNode(t) = triple.object {
                    if RESERVED_SYNTAX_TERMS.contains(&t.as_str()) {
                        (BytesStart::new("rdf:Description"), false)
                    } else {
                        let (prop_qname, prop_xmlns) = self.uri_to_qname_and_xmlns(t);
                        let mut description_start = BytesStart::new(prop_qname.clone());
                        if let Some(prop_xmlns) = prop_xmlns {
                            description_start.push_attribute(prop_xmlns);
                        }
                        self.current_resource_tag = Some(prop_qname.into_owned());
                        (description_start, true)
                    }
                } else {
                    (BytesStart::new("rdf:Description"), false)
                }
            } else {
                (BytesStart::new("rdf:Description"), false)
            };
            description_start.push_attribute(match triple.subject {
                NamedOrBlankNodeRef::NamedNode(node) => {
                    ("rdf:about", relative_iri(node.as_str(), &self.base_iri))
                }
                NamedOrBlankNodeRef::BlankNode(node) => ("rdf:nodeID", node.as_str().into()),
            });
            output.push(Event::Start(description_start));
            if with_type_tag {
                return Ok(()); // No need for a value
            }
        }
        self.write_predicate_object(triple.predicate, triple.object, output)
    }

    fn write_predicate_object<'a>(
        &mut self,
        predicate: NamedNodeRef<'a>,
        object: TermRef<'a>,
        output: &mut Vec<Event<'a>>,
    ) -> io::Result<()> {
        if RESERVED_SYNTAX_TERMS.contains(&predicate.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "RDF/XML reserved syntax term is not allowed as a predicate",
            ));
        }

        let (prop_qname, prop_xmlns) = self.uri_to_qname_and_xmlns(predicate);
        let mut property_open = BytesStart::new(prop_qname.clone());
        if let Some(prop_xmlns) = prop_xmlns {
            property_open.push_attribute(prop_xmlns);
        }
        #[allow(
            unreachable_patterns,
            clippy::match_wildcard_for_single_variants,
            clippy::allow_attributes
        )]
        match object {
            TermRef::NamedNode(node) => {
                property_open
                    .push_attribute(("rdf:resource", relative_iri(node.as_str(), &self.base_iri)));
                output.push(Event::Empty(property_open));
            }
            TermRef::BlankNode(node) => {
                property_open.push_attribute(("rdf:nodeID", node.as_str()));
                output.push(Event::Empty(property_open));
            }
            TermRef::Literal(literal) => {
                if let Some(language) = literal.language() {
                    property_open.push_attribute(("xml:lang", language));
                } else if literal.datatype() != xsd::STRING {
                    property_open.push_attribute((
                        "rdf:datatype",
                        relative_iri(literal.datatype().as_str(), &self.base_iri),
                    ));
                }
                #[cfg(feature = "rdf-12")]
                if let Some(base_direction) = literal.direction() {
                    property_open.push_attribute(("rdf:version", "1.2-basic"));
                    property_open.push_attribute(("its:version", "2.0"));
                    property_open.push_attribute((
                        "its:dir",
                        match base_direction {
                            BaseDirection::Ltr => "ltr",
                            BaseDirection::Rtl => "rtl",
                        },
                    ));
                }
                output.push(Event::Start(property_open));
                // Raw CR is normalized by XML readers; character references preserve it.
                let text = if literal.value().contains('\r') {
                    BytesText::from_escaped(
                        quick_xml::escape::escape(literal.value()).replace('\r', "&#13;"),
                    )
                } else {
                    BytesText::new(literal.value())
                };
                output.push(Event::Text(text));
                output.push(Event::End(BytesEnd::new(prop_qname)));
            }
            #[cfg(feature = "rdf-12")]
            TermRef::Triple(triple) => {
                property_open.push_attribute(("rdf:version", "1.2"));
                property_open.push_attribute(("rdf:parseType", "Triple"));
                output.push(Event::Start(property_open));
                let mut subject_start = BytesStart::new("rdf:Description");
                subject_start.push_attribute(match triple.subject.as_ref() {
                    NamedOrBlankNodeRef::NamedNode(node) => {
                        ("rdf:about", relative_iri(node.as_str(), &self.base_iri))
                    }
                    NamedOrBlankNodeRef::BlankNode(node) => ("rdf:nodeID", node.as_str().into()),
                });
                output.push(Event::Start(subject_start));
                self.write_predicate_object(
                    triple.predicate.as_ref(),
                    triple.object.as_ref(),
                    output,
                )?;
                output.push(Event::End(BytesEnd::new("rdf:Description")));
                output.push(Event::End(BytesEnd::new(prop_qname)));
            }
        }
        Ok(())
    }

    fn write_start(&self, output: &mut Vec<Event<'_>>) {
        output.push(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)));
        let mut rdf_open = BytesStart::new("rdf:RDF");
        if let Some(base_iri) = &self.base_iri {
            rdf_open.push_attribute(("xml:base", base_iri.as_str()));
        }
        for (prefix_value, prefix_name) in &self.prefixes_by_iri {
            rdf_open.push_attribute((
                if prefix_name.is_empty() {
                    "xmlns".into()
                } else {
                    format!("xmlns:{prefix_name}")
                }
                .as_str(),
                prefix_value.as_str(),
            ));
        }
        output.push(Event::Start(rdf_open))
    }

    pub(super) fn finish(&mut self, output: &mut Vec<Event<'static>>) {
        if self.current_subject.is_some() {
            output.push(Event::End(
                self.current_resource_tag
                    .take()
                    .map_or_else(|| BytesEnd::new("rdf:Description"), BytesEnd::new),
            ));
        } else {
            self.write_start(output);
        }
        output.push(Event::End(BytesEnd::new("rdf:RDF")));
    }

    fn uri_to_qname_and_xmlns<'a>(
        &self,
        uri: NamedNodeRef<'a>,
    ) -> (Cow<'a, str>, Option<(&'a str, &'a str)>) {
        let (prop_prefix, prop_value) = split_iri(uri.as_str());
        if let Some(prop_prefix) = self.prefixes_by_iri.get(prop_prefix) {
            (
                if prop_prefix.is_empty() {
                    Cow::Borrowed(prop_value)
                } else {
                    Cow::Owned(format!("{prop_prefix}:{prop_value}"))
                },
                None,
            )
        } else if prop_prefix == "http://www.w3.org/2000/xmlns/" {
            (Cow::Owned(format!("xmlns:{prop_value}")), None)
        } else if !prop_value.is_empty() && !self.custom_default_prefix {
            (Cow::Borrowed(prop_value), Some(("xmlns", prop_prefix)))
        } else {
            // TODO: does not work on recursive elements
            (
                Cow::Owned(format!("oxprefix:{prop_value}")),
                Some(("xmlns:oxprefix", prop_prefix)),
            )
        }
    }
}

pub(super) fn split_iri(iri: &str) -> (&str, &str) {
    if let Some(position_base) = iri.rfind(|c| !is_name_char(c) || c == ':') {
        if let Some(position_add) = iri[position_base..].find(|c| is_name_start_char(c) && c != ':')
        {
            (
                &iri[..position_base + position_add],
                &iri[position_base + position_add..],
            )
        } else {
            (iri, "")
        }
    } else {
        (iri, "")
    }
}

fn relative_iri<'a>(iri: &'a str, base_iri: &Option<Iri<String>>) -> Cow<'a, str> {
    if let Some(base_iri) = base_iri {
        if let Ok(relative) = base_iri.relativize(&Iri::parse_unchecked(iri)) {
            return relative.into_inner().into();
        }
    }
    iri.into()
}
