use std::time::{Duration, Instant};

use super::{Edit, Selection};

pub(super) struct Transaction {
    pub before: Vec<Selection>,
    pub after: Vec<Selection>,
    pub forward: Vec<Edit>,
    pub backward: Vec<Edit>,
}

#[derive(Default)]
struct Node {
    parent: usize,
    children: Vec<usize>,
    preferred_child: usize,
    transaction: Option<Transaction>,
    revision: u64,
}

pub(super) struct History {
    nodes: Vec<Node>,
    current: usize,
    revision: u64,
    typing: Option<Instant>,
}

impl Default for History {
    fn default() -> Self {
        Self {
            nodes: vec![Node::default()],
            current: 0,
            revision: 0,
            typing: None,
        }
    }
}

impl History {
    pub fn revision(&self) -> u64 {
        self.nodes[self.current].revision
    }

    pub fn end_typing(&mut self) {
        self.typing = None;
    }

    pub fn record(&mut self, mut next: Transaction, typing: Option<Instant>) {
        self.revision += 1;
        let next_index = self.nodes.len();
        let current = &mut self.nodes[self.current];
        if self
            .typing
            .zip(typing)
            .is_some_and(|(from, now)| now - from <= Duration::from_millis(450))
            && current.children.is_empty()
            && let Some(transaction) = &mut current.transaction
            && transaction.after == next.before
        {
            transaction.forward.append(&mut next.forward);
            transaction.backward.append(&mut next.backward);
            transaction.after = next.after;
            current.revision = self.revision;
        } else {
            let parent = self.current;
            current.preferred_child = current.children.len();
            current.children.push(next_index);
            self.current = next_index;
            self.nodes.push(Node {
                parent,
                transaction: Some(next),
                revision: self.revision,
                ..Node::default()
            });
        }
        self.typing = typing;
    }

    pub fn undo(&mut self) -> Option<&Transaction> {
        self.end_typing();
        let index = self.current;
        let transaction = self.nodes[index].transaction.as_ref()?;
        self.current = self.nodes[index].parent;
        Some(transaction)
    }

    pub fn redo(&mut self, alternate: bool) -> Option<&Transaction> {
        self.end_typing();
        let current = &mut self.nodes[self.current];
        if alternate && !current.children.is_empty() {
            current.preferred_child =
                (current.preferred_child + current.children.len() - 1) % current.children.len();
        }
        self.current = *current.children.get(current.preferred_child)?;
        self.nodes[self.current].transaction.as_ref()
    }
}
