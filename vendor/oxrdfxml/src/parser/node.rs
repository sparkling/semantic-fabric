use super::events::InternalRdfXmlParser;
use super::state::{
    RDF_ABOUT, RDF_ANNOTATION, RDF_ANNOTATION_NODE_ID, RDF_BAG_ID, RDF_DATATYPE, RDF_DESCRIPTION,
    RDF_ID, RDF_NODE_ID, RDF_PARSE_TYPE, RDF_RESOURCE, RDF_VERSION, RESERVED_RDF_ATTRIBUTES,
    RdfXmlState,
};
#[cfg(feature = "rdf-12")]
use super::version::RdfVersion;
use crate::error::{RdfXmlParseError, RdfXmlSyntaxError};
use crate::utils::is_nc_name;
use oxilangtag::LanguageTag;
use oxiri::Iri;
#[cfg(feature = "rdf-12")]
use oxrdf::BaseDirection;
use oxrdf::vocab::rdf;
use oxrdf::{BlankNode, NamedNode, NamedOrBlankNode, Triple};
use quick_xml::Error;
use quick_xml::events::BytesStart;
use quick_xml::name::{Namespace, ResolveResult};
use std::borrow::Cow;

#[derive(PartialEq, Eq)]
pub(super) enum RdfXmlParseType {
    Default,
    Collection,
    Literal,
    Resource,
    #[cfg(feature = "rdf-12")]
    Triple,
    Other,
}

pub(super) struct StartAttributes {
    pub(super) language: Option<String>,
    pub(super) base_iri: Option<Iri<String>>,
    pub(super) id_attr: Option<NamedNode>,
    pub(super) node_id_attr: Option<BlankNode>,
    pub(super) about_attr: Option<NamedNode>,
    pub(super) property_attrs: Vec<(NamedNode, String)>,
    pub(super) resource_attr: Option<NamedNode>,
    pub(super) datatype_attr: Option<NamedNode>,
    pub(super) parse_type: RdfXmlParseType,
    pub(super) type_attr: Option<NamedNode>,
    #[cfg(feature = "rdf-12")]
    pub(super) base_direction: Option<BaseDirection>,
    #[cfg(feature = "rdf-12")]
    pub(super) rdf_version: Option<RdfVersion>,
    #[cfg(feature = "rdf-12")]
    pub(super) annotation_attr: Option<NamedNode>,
    #[cfg(feature = "rdf-12")]
    pub(super) annotation_node_id_attr: Option<BlankNode>,
}

impl<R> InternalRdfXmlParser<R> {
    pub(super) fn parse_start_attributes(
        &mut self,
        event: &BytesStart<'_>,
    ) -> Result<StartAttributes, RdfXmlParseError> {
        let mut language = None;
        let mut base_iri = None;
        let mut id_attr = None;
        let mut node_id_attr = None;
        let mut about_attr = None;
        let mut property_attrs = Vec::new();
        let mut resource_attr = None;
        let mut datatype_attr = None;
        let mut parse_type = RdfXmlParseType::Default;
        let mut type_attr = None;
        #[cfg(feature = "rdf-12")]
        let mut base_direction_attr = None;
        #[cfg(feature = "rdf-12")]
        let mut rdf_version = None;
        #[cfg(feature = "rdf-12")]
        let mut annotation_attr = None;
        #[cfg(feature = "rdf-12")]
        let mut annotation_node_id_attr = None;

        for attribute in event.attributes() {
            let attribute = attribute.map_err(Error::InvalidAttr)?;
            let (attribute_namespace, attribute_local_name) =
                self.reader.resolver().resolve_attribute(attribute.key);
            if attribute_namespace
                == ResolveResult::Bound(Namespace(b"http://www.w3.org/XML/1998/namespace"))
            {
                match attribute.key.local_name().as_ref() {
                    b"lang" => {
                        let tag = self.convert_attribute(&attribute)?.to_ascii_lowercase();
                        language = Some(if self.lenient {
                            tag
                        } else {
                            LanguageTag::parse(tag.clone())
                                .map_err(|error| {
                                    RdfXmlSyntaxError::invalid_language_tag(tag, error)
                                })?
                                .into_inner()
                        });
                    }
                    b"base" => {
                        let iri = self.convert_attribute(&attribute)?;
                        base_iri = Some(if self.lenient {
                            Iri::parse_unchecked(iri.into_owned())
                        } else {
                            Iri::parse(iri.clone().into_owned()).map_err(|error| {
                                RdfXmlSyntaxError::invalid_iri(iri.into(), error)
                            })?
                        })
                    }
                    _ => (), // We ignore other xml attributes
                }
            } else if attribute.key.as_ref().starts_with(b"xml") {
                // We ignore other xml attributes
            } else if cfg!(feature = "rdf-12")
                && attribute_namespace
                    == ResolveResult::Bound(Namespace(b"http://www.w3.org/2005/11/its"))
                && attribute.key.local_name().as_ref() == b"dir"
            {
                #[cfg(feature = "rdf-12")]
                {
                    base_direction_attr = Some(attribute);
                }
            } else if cfg!(feature = "rdf-12")
                && attribute_namespace
                    == ResolveResult::Bound(Namespace(b"http://www.w3.org/2005/11/its"))
                && attribute.key.local_name().as_ref() == b"version"
            {
                // We ignore its:version
            } else {
                let attribute_url =
                    self.resolve_ns_name(attribute_namespace.clone(), attribute_local_name)?;
                if *attribute_url == *RDF_ID {
                    let mut id = self.convert_attribute(&attribute)?.into_owned();
                    if !is_nc_name(&id) {
                        return Err(RdfXmlSyntaxError::msg(format!(
                            "{id} is not a valid rdf:ID value"
                        ))
                        .into());
                    }
                    id.insert(0, '#');
                    id_attr = Some(id);
                } else if *attribute_url == *RDF_BAG_ID {
                    let bag_id = self.convert_attribute(&attribute)?;
                    if !is_nc_name(&bag_id) {
                        return Err(RdfXmlSyntaxError::msg(format!(
                            "{bag_id} is not a valid rdf:bagID value"
                        ))
                        .into());
                    }
                } else if *attribute_url == *RDF_NODE_ID {
                    let id = self.convert_attribute(&attribute)?;
                    if !is_nc_name(&id) {
                        return Err(RdfXmlSyntaxError::msg(format!(
                            "{id} is not a valid rdf:nodeID value"
                        ))
                        .into());
                    }
                    node_id_attr = Some(BlankNode::new_unchecked(id));
                } else if *attribute_url == *RDF_ABOUT {
                    about_attr = Some(attribute);
                } else if *attribute_url == *RDF_RESOURCE {
                    resource_attr = Some(attribute);
                } else if *attribute_url == *RDF_DATATYPE {
                    datatype_attr = Some(attribute);
                } else if *attribute_url == *RDF_PARSE_TYPE {
                    parse_type = match self.convert_attribute(&attribute)?.as_ref() {
                        "Collection" => RdfXmlParseType::Collection,
                        "Literal" => RdfXmlParseType::Literal,
                        "Resource" => RdfXmlParseType::Resource,
                        #[cfg(feature = "rdf-12")]
                        "Triple" => RdfXmlParseType::Triple,
                        _ => RdfXmlParseType::Other,
                    };
                } else if cfg!(feature = "rdf-12") && *attribute_url == *RDF_VERSION {
                    #[cfg(feature = "rdf-12")]
                    {
                        rdf_version =
                            Some(RdfVersion::from_str(&self.convert_attribute(&attribute)?)?);
                    }
                } else if cfg!(feature = "rdf-12") && *attribute_url == *RDF_ANNOTATION {
                    #[cfg(feature = "rdf-12")]
                    {
                        annotation_attr = Some(attribute);
                    }
                } else if cfg!(feature = "rdf-12") && *attribute_url == *RDF_ANNOTATION_NODE_ID {
                    #[cfg(feature = "rdf-12")]
                    {
                        annotation_node_id_attr = Some(attribute);
                    }
                } else if attribute_url == rdf::TYPE.as_str() {
                    type_attr = Some(attribute);
                } else if RESERVED_RDF_ATTRIBUTES.contains(&&*attribute_url) {
                    return Err(RdfXmlSyntaxError::msg(format!(
                        "{attribute_url} is not a valid attribute"
                    ))
                    .into());
                } else {
                    property_attrs.push((
                        self.parse_iri(attribute_url)?,
                        self.convert_attribute(&attribute)?.into(),
                    ));
                }
            }
        }

        // Parsing with the base URI
        let id_attr = if let Some(iri) = id_attr {
            let iri = self.resolve_iri(base_iri.as_ref(), iri.into())?;
            if !self.lenient {
                if self.known_rdf_id.contains(iri.as_str()) {
                    return Err(RdfXmlSyntaxError::msg(format!(
                        "{iri} has already been used as rdf:ID value"
                    ))
                    .into());
                }
                self.known_rdf_id.insert(iri.as_str().into());
            }
            Some(iri)
        } else {
            None
        };
        let about_attr = if let Some(attr) = about_attr {
            Some(self.convert_iri_attribute(base_iri.as_ref(), &attr)?)
        } else {
            None
        };
        let resource_attr = if let Some(attr) = resource_attr {
            Some(self.convert_iri_attribute(base_iri.as_ref(), &attr)?)
        } else {
            None
        };
        let datatype_attr = if let Some(attr) = datatype_attr {
            Some(self.convert_iri_attribute(base_iri.as_ref(), &attr)?)
        } else {
            None
        };
        let type_attr = if let Some(attr) = type_attr {
            Some(self.convert_iri_attribute(base_iri.as_ref(), &attr)?)
        } else {
            None
        };
        #[cfg(feature = "rdf-12")]
        let base_direction = if rdf_version
            .unwrap_or_else(|| self.current_rdf_version())
            .supports_its_dir()
        {
            if let Some(attr) = base_direction_attr {
                Some(match self.convert_attribute(&attr)?.as_ref() {
                    "ltr" => BaseDirection::Ltr,
                    "rtl" => BaseDirection::Rtl,
                    base => {
                        return Err(RdfXmlSyntaxError::msg(format!(
                            "Invalid base direction: '{base}'"
                        ))
                        .into());
                    }
                })
            } else {
                None
            }
        } else {
            None
        };
        #[cfg(feature = "rdf-12")]
        let annotation_attr = if let Some(attr) = annotation_attr {
            Some(self.convert_iri_attribute(base_iri.as_ref(), &attr)?)
        } else {
            None
        };
        #[cfg(feature = "rdf-12")]
        let annotation_node_id_attr = if let Some(attr) = annotation_node_id_attr {
            let id = self.convert_attribute(&attr)?;
            if !is_nc_name(&id) {
                return Err(RdfXmlSyntaxError::msg(format!(
                    "{id} is not a valid rdf:annotationNodeID value"
                ))
                .into());
            }
            Some(BlankNode::new_unchecked(id))
        } else {
            None
        };

        Ok(StartAttributes {
            language,
            base_iri,
            id_attr,
            node_id_attr,
            about_attr,
            property_attrs,
            resource_attr,
            datatype_attr,
            parse_type,
            type_attr,
            #[cfg(feature = "rdf-12")]
            base_direction,
            #[cfg(feature = "rdf-12")]
            rdf_version,
            #[cfg(feature = "rdf-12")]
            annotation_attr,
            #[cfg(feature = "rdf-12")]
            annotation_node_id_attr,
        })
    }

    pub(super) fn build_node_elt(
        &mut self,
        iri: NamedNode,
        attributes: StartAttributes,
        results: &mut Vec<Triple>,
    ) -> Result<RdfXmlState, RdfXmlSyntaxError> {
        let StartAttributes {
            language,
            base_iri,
            id_attr,
            node_id_attr,
            about_attr,
            type_attr,
            property_attrs,
            #[cfg(feature = "rdf-12")]
            base_direction,
            #[cfg(feature = "rdf-12")]
            rdf_version,
            ..
        } = attributes;
        let subject = match (id_attr, node_id_attr, about_attr) {
            (Some(id_attr), None, None) => NamedOrBlankNode::from(id_attr),
            (None, Some(node_id_attr), None) => node_id_attr.into(),
            (None, None, Some(about_attr)) => about_attr.into(),
            (None, None, None) => BlankNode::default().into(),
            (Some(_), Some(_), _) => {
                return Err(RdfXmlSyntaxError::msg(
                    "Not both rdf:ID and rdf:nodeID could be set at the same time",
                ));
            }
            (_, Some(_), Some(_)) => {
                return Err(RdfXmlSyntaxError::msg(
                    "Not both rdf:nodeID and rdf:resource could be set at the same time",
                ));
            }
            (Some(_), _, Some(_)) => {
                return Err(RdfXmlSyntaxError::msg(
                    "Not both rdf:ID and rdf:resource could be set at the same time",
                ));
            }
        };

        self.emit_property_attrs(
            &subject,
            property_attrs,
            language.as_deref(),
            #[cfg(feature = "rdf-12")]
            base_direction,
            results,
        );

        if let Some(type_attr) = type_attr {
            self.emit_triple(results, Triple::new(subject.clone(), rdf::TYPE, type_attr));
        }

        if iri != *RDF_DESCRIPTION {
            self.emit_triple(results, Triple::new(subject.clone(), rdf::TYPE, iri));
        }
        Ok(RdfXmlState::NodeElt {
            base_iri,
            language,
            #[cfg(feature = "rdf-12")]
            base_direction,
            subject,
            li_counter: 0,
            #[cfg(feature = "rdf-12")]
            rdf_version,
        })
    }

    pub(super) fn convert_iri_attribute(
        &self,
        base_iri: Option<&Iri<String>>,
        attribute: &quick_xml::events::attributes::Attribute<'_>,
    ) -> Result<NamedNode, RdfXmlParseError> {
        Ok(self.resolve_iri(base_iri, self.convert_attribute(attribute)?)?)
    }

    pub(super) fn resolve_iri(
        &self,
        base_iri: Option<&Iri<String>>,
        relative_iri: Cow<'_, str>,
    ) -> Result<NamedNode, RdfXmlSyntaxError> {
        if let Some(base_iri) = base_iri.or_else(|| self.current_base_iri()) {
            Ok(NamedNode::new_unchecked(
                if self.lenient {
                    base_iri.resolve_unchecked(&relative_iri)
                } else {
                    base_iri.resolve(&relative_iri).map_err(|error| {
                        RdfXmlSyntaxError::invalid_iri(relative_iri.into(), error)
                    })?
                }
                .into_inner(),
            ))
        } else {
            self.parse_iri(relative_iri.into())
        }
    }

    pub(super) fn parse_iri(&self, relative_iri: String) -> Result<NamedNode, RdfXmlSyntaxError> {
        Ok(NamedNode::new_unchecked(if self.lenient {
            relative_iri
        } else {
            Iri::parse(relative_iri.clone())
                .map_err(|error| RdfXmlSyntaxError::invalid_iri(relative_iri, error))?
                .into_inner()
        }))
    }
}
