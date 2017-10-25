#![deny(missing_copy_implementations,
        trivial_numeric_casts,
        trivial_casts,
        unused_extern_crates,
        unused_import_braces,
        unused_qualifications)]

extern crate xml;

use std::io::prelude::*;
use std::str::FromStr;

use xml::reader::{EventReader, XmlEvent};
use xml::attribute::OwnedAttribute;

pub mod error;
use error::{Error, ErrorReason};

mod elements;
pub use elements::{Coordinate, Id, Role, Bounds, Node, Way, Relation, Member, Tag, Reference,
                   UnresolvedReference, Timestamp, Version, Changeset, User, Visible};
mod polygon;

#[derive(Debug)]
pub struct OSM {
    pub bounds: Option<Bounds>,
    pub nodes: Vec<Node>,
    pub ways: Vec<Way>,
    pub relations: Vec<Relation>,
}

impl OSM {
    fn empty() -> OSM {
        OSM {
            bounds: None,
            nodes: Vec::new(),
            ways: Vec::new(),
            relations: Vec::new(),
        }
    }

    pub fn parse<R: Read>(source: R) -> Result<OSM, Error> {

        let mut osm = OSM::empty();

        let mut parser = EventReader::new(source);

        loop {
            match parse_element_data(&mut parser) {
                Err(Error::XmlParseError(err)) => return Err(Error::XmlParseError(err)),
                Err(Error::BoundsMissing(_)) => osm.bounds = None,
                Err(Error::MalformedTag(_)) |
                Err(Error::MalformedNode(_)) |
                Err(Error::MalformedWay(_)) |
                Err(Error::MalformedRelation(_)) |
                Err(Error::UnknownElement) => continue,
                Ok(data) => {
                    match data {
                        ElementData::EndOfDocument => return Ok(osm),
                        ElementData::Ignored => continue,
                        ElementData::Bounds(minlat, minlon, maxlat, maxlon) => {
                            osm.bounds = Some(Bounds {
                                minlat: minlat,
                                minlon: minlon,
                                maxlat: maxlat,
                                maxlon: maxlon,
                            });
                        }
                        ElementData::Node(id,
                                          lat,
                                          lon,
                                          tags,
                                          version,
                                          timestamp,
                                          user,
                                          changeset,
                                          visible) => {
                            osm.nodes.push(Node {
                                id: id,
                                lat: lat,
                                lon: lon,
                                tags: tags,
                                version: version,
                                timestamp: timestamp,
                                user: user,
                                changeset: changeset,
                                visible: visible,
                            });
                        }
                        ElementData::Way(id,
                                         node_refs,
                                         tags,
                                         version,
                                         timestamp,
                                         user,
                                         changeset,
                                         visible) => {
                            osm.ways.push(Way {
                                id: id,
                                nodes: node_refs,
                                tags: tags,
                                version: version,
                                timestamp: timestamp,
                                user: user,
                                changeset: changeset,
                                visible: visible,
                            });
                        }
                        ElementData::Relation(relation) => {
                            osm.relations.push(relation);
                        }
                    }
                }
            }
        }
    }

    pub fn resolve_reference<'a>(&self, reference: &UnresolvedReference) -> Reference {
        match *reference {
            UnresolvedReference::Node(id) => {
                match self.nodes.iter().find(|node| node.id == id) {
                    Some(node) => Reference::Node(&node),
                    None => Reference::Unresolved,
                }
            }
            UnresolvedReference::Way(id) => {
                match self.ways.iter().find(|way| way.id == id) {
                    Some(way) => Reference::Way(&way),
                    None => Reference::Unresolved,
                }
            }
            UnresolvedReference::Relation(id) => {
                match self.relations.iter().find(|relation| relation.id == id) {
                    Some(relation) => Reference::Relation(&relation),
                    None => Reference::Unresolved,
                }
            }
        }
    }
}

enum ElementType {
    Bounds,
    Node,
    Way,
    Relation,
    Tag,
    NodeRef,
    Member,
}

enum ElementData {
    Bounds(Coordinate, Coordinate, Coordinate, Coordinate),
    Node(Id, Coordinate, Coordinate, Vec<Tag>, Version, Timestamp, User, Changeset, Visible),
    Way(Id, Vec<UnresolvedReference>, Vec<Tag>, Version, Timestamp, User, Changeset, Visible),
    Relation(Relation),
    // These two are here so we can terminate and skip uninteresting data without
    // using error handling.
    EndOfDocument,
    Ignored,
}


impl FromStr for ElementType {
    type Err = Error;

    fn from_str(s: &str) -> Result<ElementType, Error> {
        match s.to_lowercase().as_ref() {
            "bounds" => Ok(ElementType::Bounds),
            "node" => Ok(ElementType::Node),
            "way" => Ok(ElementType::Way),
            "relation" => Ok(ElementType::Relation),
            "tag" => Ok(ElementType::Tag),
            "nd" => Ok(ElementType::NodeRef),
            "member" => Ok(ElementType::Member),
            _ => Err(Error::UnknownElement),
        }
    }
}

fn parse_element_data<R: Read>(parser: &mut EventReader<R>) -> Result<ElementData, Error> {
    let element = try!(parser.next());
    match element {
        XmlEvent::EndDocument => Ok(ElementData::EndOfDocument),
        XmlEvent::StartElement { name, attributes, .. } => {
            let element_type = try!(ElementType::from_str(&name.local_name));

            match element_type {
                ElementType::Bounds => parse_bounds(&attributes),
                ElementType::Node => parse_node(parser, &attributes),
                ElementType::Way => parse_way(parser, &attributes),
                ElementType::Relation => parse_relation(parser, &attributes),
                _ => Err(Error::UnknownElement),
            }
        }
        _ => Ok(ElementData::Ignored),
    }
}

fn parse_relation<R: Read>(
    parser: &mut EventReader<R>,
    attrs: &Vec<OwnedAttribute>,
) -> Result<ElementData, Error> {
    let id = try!(find_attribute("id", attrs).map_err(
        Error::MalformedRelation,
    ));
    let version = try!(find_attribute("version", attrs).map_err(
        Error::MalformedRelation,
    ));
    let timestamp = try!(find_attribute_uncasted("timestamp", attrs).map_err(
        Error::MalformedRelation,
    ));
    let user = try!(find_attribute_uncasted("user", attrs).map_err(
        Error::MalformedRelation,
    ));
    let changeset = try!(find_attribute("changeset", attrs).map_err(
        Error::MalformedRelation,
    ));
    let visible = try!(find_attribute_uncasted("visible", attrs).map_err(
        Error::MalformedRelation,
    ));
    let mut members = Vec::new();
    let mut tags = Vec::new();

    loop {
        match try!(parser.next()) {
            XmlEvent::EndElement { name } => {
                let element_type = try!(ElementType::from_str(&name.local_name));

                match element_type {
                    ElementType::Relation => {
                        return Ok(ElementData::Relation(Relation {
                            id: id,
                            members: members,
                            tags: tags,
                            version: version,
                            timestamp: timestamp,
                            user: user,
                            changeset: changeset,
                            visible: visible,
                        }));
                    }
                    _ => continue,
                }
            }
            XmlEvent::StartElement { name, attributes, .. } => {
                let element_type = try!(ElementType::from_str(&name.local_name));

                match element_type {
                    ElementType::Tag => {
                        if let Ok(tag) = parse_tag(&attributes) {
                            tags.push(tag);
                        }
                    }
                    ElementType::Member => {
                        let el_type = try!(find_attribute_uncasted("type", &attributes).map_err(
                            Error::MalformedRelation,
                        ));
                        let el_ref = try!(find_attribute("ref", &attributes).map_err(
                            Error::MalformedRelation,
                        ));
                        let el_role = try!(find_attribute_uncasted("role", &attributes).map_err(
                            Error::MalformedRelation,
                        ));

                        let el = match el_type.to_lowercase().as_ref() {
                            "node" => Member::Node(UnresolvedReference::Node(el_ref), el_role),
                            "way" => Member::Way(UnresolvedReference::Way(el_ref), el_role),
                            "relation" => {
                                Member::Relation(UnresolvedReference::Relation(el_ref), el_role)
                            }
                            _ => return Err(Error::MalformedRelation(ErrorReason::Missing)),
                        };

                        members.push(el);
                    }
                    ElementType::Bounds | ElementType::Node | ElementType::Relation |
                    ElementType::Way | ElementType::NodeRef => {
                        return Err(Error::MalformedRelation(ErrorReason::IllegalNesting))
                    }
                }
            }
            _ => continue,
        }
    }
}

fn parse_way<R: Read>(
    parser: &mut EventReader<R>,
    attrs: &Vec<OwnedAttribute>,
) -> Result<ElementData, Error> {
    let id = try!(find_attribute("id", attrs).map_err(Error::MalformedWay));
    let version = try!(find_attribute("version", attrs).map_err(
        Error::MalformedWay,
    ));
    let timestamp = try!(find_attribute_uncasted("timestamp", attrs).map_err(
        Error::MalformedWay,
    ));
    let user = try!(find_attribute_uncasted("user", attrs).map_err(
        Error::MalformedRelation,
    ));
    let changeset = try!(find_attribute("changeset", attrs).map_err(
        Error::MalformedRelation,
    ));
    let visible = try!(find_attribute_uncasted("visible", attrs).map_err(
        Error::MalformedRelation,
    ));

    let mut node_refs = Vec::new();
    let mut tags = Vec::new();

    loop {
        match try!(parser.next()) {
            XmlEvent::EndElement { name } => {
                let element_type = try!(ElementType::from_str(&name.local_name));

                match element_type {
                    ElementType::Way => {
                        return Ok(ElementData::Way(
                            id,
                            node_refs,
                            tags,
                            version,
                            timestamp,
                            user,
                            changeset,
                            visible,
                        ))
                    }
                    _ => continue,
                }
            }
            XmlEvent::StartElement { name, attributes, .. } => {
                let element_type = try!(ElementType::from_str(&name.local_name));

                match element_type {
                    ElementType::Tag => {
                        if let Ok(tag) = parse_tag(&attributes) {
                            tags.push(tag);
                        }
                    }
                    ElementType::NodeRef => {
                        let node_ref = try!(find_attribute("ref", &attributes).map_err(
                            Error::MalformedWay,
                        ));
                        node_refs.push(UnresolvedReference::Node(node_ref));
                    }
                    ElementType::Bounds | ElementType::Node | ElementType::Relation |
                    ElementType::Way | ElementType::Member => {
                        return Err(Error::MalformedWay(ErrorReason::IllegalNesting))
                    }
                }
            }
            _ => continue,
        }
    }

}

fn parse_node<R: Read>(
    parser: &mut EventReader<R>,
    attrs: &Vec<OwnedAttribute>,
) -> Result<ElementData, Error> {
    let id = try!(find_attribute("id", attrs).map_err(Error::MalformedNode));
    let lat = try!(find_attribute("lat", attrs).map_err(Error::MalformedNode));
    let lon = try!(find_attribute("lon", attrs).map_err(Error::MalformedNode));
    let version = try!(find_attribute("version", attrs).map_err(
        Error::MalformedNode,
    ));
    let timestamp = try!(find_attribute_uncasted("timestamp", attrs).map_err(
        Error::MalformedNode,
    ));
    let user = try!(find_attribute_uncasted("user", attrs).map_err(
        Error::MalformedRelation,
    ));
    let changeset = try!(find_attribute("changeset", attrs).map_err(
        Error::MalformedRelation,
    ));
    let visible = try!(find_attribute_uncasted("visible", attrs).map_err(
        Error::MalformedRelation,
    ));
    let mut tags = Vec::new();

    loop {
        match try!(parser.next()) {
            XmlEvent::EndElement { name } => {
                let element_type = try!(ElementType::from_str(&name.local_name));

                match element_type {
                    ElementType::Node => {
                        return Ok(ElementData::Node(
                            id,
                            lat,
                            lon,
                            tags,
                            version,
                            timestamp,
                            user,
                            changeset,
                            visible,
                        ))
                    }
                    _ => continue,
                }
            }
            XmlEvent::StartElement { name, attributes, .. } => {
                let element_type = try!(ElementType::from_str(&name.local_name));

                match element_type {
                    ElementType::Tag => {
                        if let Ok(tag) = parse_tag(&attributes) {
                            tags.push(tag);
                        }
                    }
                    ElementType::Bounds | ElementType::Node | ElementType::Relation |
                    ElementType::Way | ElementType::NodeRef | ElementType::Member => {
                        return Err(Error::MalformedNode(ErrorReason::IllegalNesting))
                    }
                }
            }
            _ => continue,
        }
    }
}

fn parse_tag(attributes: &Vec<OwnedAttribute>) -> Result<Tag, Error> {
    let key = try!(find_attribute_uncasted("k", attributes).map_err(
        Error::MalformedTag,
    ));
    let val = try!(find_attribute_uncasted("v", attributes).map_err(
        Error::MalformedTag,
    ));
    Ok(Tag { key: key, val: val })
}

fn parse_bounds(attrs: &Vec<OwnedAttribute>) -> Result<ElementData, Error> {
    let minlat = try!(find_attribute("minlat", attrs).map_err(
        Error::BoundsMissing,
    ));
    let minlon = try!(find_attribute("minlon", attrs).map_err(
        Error::BoundsMissing,
    ));
    let maxlat = try!(find_attribute("maxlat", attrs).map_err(
        Error::BoundsMissing,
    ));
    let maxlon = try!(find_attribute("maxlon", attrs).map_err(
        Error::BoundsMissing,
    ));

    Ok(ElementData::Bounds(minlat, minlon, maxlat, maxlon))
}

fn find_attribute<T>(name: &str, attrs: &Vec<OwnedAttribute>) -> Result<T, ErrorReason>
where
    ErrorReason: From<<T as std::str::FromStr>::Err>,
    T: FromStr,
{
    let val_raw = try!(find_attribute_uncasted(name, attrs));
    let val = try!(val_raw.parse::<T>());
    Ok(val)
}

fn find_attribute_uncasted(name: &str, attrs: &Vec<OwnedAttribute>) -> Result<String, ErrorReason> {
    let attr = attrs.iter().find(|attr| attr.name.local_name == name);
    match attr {
        Some(a) => Ok(a.value.clone()),
        None => Err(ErrorReason::Missing),
    }
}
