// SPDX-License-Identifier: MIT

//! Bounded event preflight before allocating the YAML document tree.

use std::collections::BTreeSet;
use yaml_rust2::Yaml;
use yaml_rust2::parser::{Event, Parser};
use yaml_rust2::scanner::TScalarStyle;

const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_DEPTH: usize = 32;
const MAX_EVENTS: usize = 8192;

enum Container {
    Mapping {
        keys: BTreeSet<String>,
        key_next: bool,
    },
    Sequence,
}

pub(super) fn unambiguous_bounded_document(source: &str) -> bool {
    if source.len() > MAX_SOURCE_BYTES {
        return false;
    }
    let mut parser = Parser::new(source.chars());
    let mut stack = Vec::new();
    let mut documents = 0;
    for _ in 0..MAX_EVENTS {
        let Ok((event, _)) = parser.next_token() else {
            return false;
        };
        match event {
            Event::StreamEnd => return documents == 1 && stack.is_empty(),
            Event::DocumentStart => {
                documents += 1;
                if documents > 1 || !stack.is_empty() {
                    return false;
                }
            }
            Event::DocumentEnd if !stack.is_empty() => return false,
            Event::StreamStart | Event::DocumentEnd => {}
            event => {
                if !admit_node(event, &mut stack) {
                    return false;
                }
            }
        }
    }
    false
}

fn admit_node(event: Event, stack: &mut Vec<Container>) -> bool {
    match event {
        Event::Scalar(value, style, 0, None) => admit_scalar(value, style, stack),
        Event::MappingStart(0, None) => push_container(
            Container::Mapping {
                keys: BTreeSet::new(),
                key_next: true,
            },
            stack,
        ),
        Event::SequenceStart(0, None) => push_container(Container::Sequence, stack),
        Event::MappingEnd => {
            matches!(stack.pop(), Some(Container::Mapping { key_next: true, .. }))
        }
        Event::SequenceEnd => matches!(stack.pop(), Some(Container::Sequence)),
        // Aliases, anchors and explicit tags are outside the supported workflow subset.
        // In particular, do not let an alias expand before the allocation bounds apply.
        _ => false,
    }
}

fn admit_scalar(value: String, style: TScalarStyle, stack: &mut [Container]) -> bool {
    if let Some(Container::Mapping { keys, key_next }) = stack.last_mut() {
        if *key_next
            && (value == "<<"
                || (style == TScalarStyle::Plain
                    && !matches!(Yaml::from_str(&value), Yaml::String(_)))
                || !keys.insert(value))
        {
            return false;
        }
        *key_next = !*key_next;
    }
    true
}

fn push_container(container: Container, stack: &mut Vec<Container>) -> bool {
    if stack.len() >= MAX_DEPTH {
        return false;
    }
    if let Some(Container::Mapping { key_next, .. }) = stack.last_mut() {
        if *key_next {
            return false; // Complex keys are not GitHub workflow field names.
        }
        *key_next = true;
    }
    stack.push(container);
    true
}
