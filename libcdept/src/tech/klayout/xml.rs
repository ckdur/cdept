//! A small XML tree, enough for KLayout's `.lyt`, `.lyp` and `.lym` files.

use quick_xml::events::Event;

/// An element: its name, the text directly in it and its child elements in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    /// The first child of that name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every child of that name.
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The trimmed text of the first child of that name, when it has any.
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name)
            .map(|c| c.text.trim())
            .filter(|t| !t.is_empty())
    }

    /// The text of a child as `true`/`false`.
    pub fn flag(&self, name: &str) -> Option<bool> {
        match self.text_of(name)? {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        }
    }

    /// The text of a child as a number.
    pub fn number<T: std::str::FromStr>(&self, name: &str) -> Option<T> {
        self.text_of(name)?.parse().ok()
    }

    /// The child at the end of a path of names.
    pub fn at(&self, path: &[&str]) -> Option<&Node> {
        path.iter().try_fold(self, |node, name| node.child(name))
    }
}

/// Read a document into its root element.
pub fn parse(xml: &str) -> Result<Node, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let close = |stack: &mut Vec<Node>, root: &mut Option<Node>| {
        if let Some(node) = stack.pop() {
            match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None => *root = Some(node),
            }
        }
    };
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("XML error at {}: {e}", reader.buffer_position()))?;
        match event {
            Event::Start(e) => stack.push(Node {
                name: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(),
                ..Node::default()
            }),
            Event::Empty(e) => {
                stack.push(Node {
                    name: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(),
                    ..Node::default()
                });
                close(&mut stack, &mut root);
            }
            Event::End(_) => close(&mut stack, &mut root),
            Event::Text(e) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&e.decode().map_err(|e| e.to_string())?);
                }
            }
            Event::CData(e) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&e.decode().map_err(|e| e.to_string())?);
                }
            }
            Event::GeneralRef(e) => {
                let Some(node) = stack.last_mut() else {
                    continue;
                };
                if let Some(c) = e.resolve_char_ref().map_err(|e| e.to_string())? {
                    node.text.push(c);
                } else {
                    let name = e.decode().map_err(|e| e.to_string())?;
                    match name.as_ref() {
                        "lt" => node.text.push('<'),
                        "gt" => node.text.push('>'),
                        "amp" => node.text.push('&'),
                        "quot" => node.text.push('"'),
                        "apos" => node.text.push('\''),
                        other => {
                            node.text.push('&');
                            node.text.push_str(other);
                            node.text.push(';');
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    root.ok_or_else(|| "the document has no root element".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_text_and_references() {
        let root = parse(
            "<?xml version=\"1.0\"?><!-- c --><a><b>x &lt; y &amp; z</b><c/><b> 2 </b>\
             <d><e>true</e></d></a>",
        )
        .unwrap();
        assert_eq!(root.name, "a");
        assert_eq!(root.text_of("b"), Some("x < y & z"));
        assert_eq!(root.all("b").count(), 2);
        assert_eq!(root.child("c").unwrap().text, "");
        assert_eq!(root.at(&["d", "e"]).unwrap().text, "true");
        assert_eq!(root.child("d").unwrap().flag("e"), Some(true));
        assert_eq!(root.text_of("c"), None);
        assert!(parse("").is_err());
    }
}
