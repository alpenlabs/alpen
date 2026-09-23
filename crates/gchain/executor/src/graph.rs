//! The executor's view of the links it has processed.

use std::collections::{HashMap, HashSet, VecDeque};

use strata_gchain_types::*;

/// Index of processed links by the nodes they connect.
///
/// This is how the executor answers the questions the node-centric model
/// raises: which processed links leave a node, whether a node is reachable from
/// the committed node at all, and some path of processed links to it.  Which
/// path doesn't matter to the stages (every path to a node reconstructs the
/// same state there), so the graph just picks one with the fewest links.
pub struct LinkGraph<S: GChainSpec> {
    links: HashMap<LinkRef<S>, LinkEndpoints<S>>,
    forward: HashMap<NodeRef<S>, Vec<LinkRef<S>>>,
}

impl<S: GChainSpec> LinkGraph<S> {
    pub fn new() -> Self {
        Self {
            links: HashMap::new(),
            forward: HashMap::new(),
        }
    }

    /// Records a link.  Recording a link that's already present is a no-op.
    pub fn insert(&mut self, lref: LinkRef<S>, endpoints: LinkEndpoints<S>) {
        if self.links.contains_key(&lref) {
            return;
        }

        self.forward
            .entry(endpoints.origin().clone())
            .or_default()
            .push(lref.clone());
        self.links.insert(lref, endpoints);
    }

    /// Forgets a link, returning where it sat.
    pub fn remove(&mut self, lref: &LinkRef<S>) -> Option<LinkEndpoints<S>> {
        let endpoints = self.links.remove(lref)?;
        if let Some(out) = self.forward.get_mut(endpoints.origin()) {
            out.retain(|l| l != lref);
            if out.is_empty() {
                self.forward.remove(endpoints.origin());
            }
        }
        Some(endpoints)
    }

    pub fn contains(&self, lref: &LinkRef<S>) -> bool {
        self.links.contains_key(lref)
    }

    pub fn endpoints(&self, lref: &LinkRef<S>) -> Option<&LinkEndpoints<S>> {
        self.links.get(lref)
    }

    /// The processed links departing from a node.
    pub fn links_from(&self, node: &NodeRef<S>) -> &[LinkRef<S>] {
        self.forward.get(node).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn iter_links(&self) -> impl Iterator<Item = (&LinkRef<S>, &LinkEndpoints<S>)> {
        self.links.iter()
    }

    pub fn len(&self) -> usize {
        self.links.len()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// Finds a path of processed links from one node to another with the
    /// fewest links, if the second is reachable from the first.
    ///
    /// The path is empty when the nodes are the same.  Fewest links is the
    /// cheapest pre-state to reconstruct, which naturally prefers a checkpoint
    /// link over the run of block links it covers.
    pub fn find_path(&self, from: &NodeRef<S>, to: &NodeRef<S>) -> Option<LinkPath<S>> {
        // BFS from `from`, remembering the link each node was first reached by.
        let mut arrived_by: HashMap<NodeRef<S>, LinkRef<S>> = HashMap::new();
        let mut queue = VecDeque::from([from.clone()]);
        while let Some(node) = queue.pop_front() {
            if node == *to {
                break;
            }

            for lref in self.links_from(&node) {
                let target = self.links[lref].target();
                if target != from && !arrived_by.contains_key(target) {
                    arrived_by.insert(target.clone(), lref.clone());
                    queue.push_back(target.clone());
                }
            }
        }

        if from != to && !arrived_by.contains_key(to) {
            return None;
        }

        // Walk the arrival links back from `to`, then lay them down forwards.
        let mut backwards = Vec::new();
        let mut node = to;
        while node != from {
            let lref = &arrived_by[node];
            node = self.links[lref].origin();
            backwards.push(lref);
        }

        let mut path = LinkPath::new_at(from.clone());
        for lref in backwards.into_iter().rev() {
            let pushed = path.try_push_link(lref.clone(), &self.links[lref]);
            debug_assert!(pushed, "gchain: BFS path must be contiguous");
        }
        Some(path)
    }

    /// Every link reachable from a node, following links forwards.
    pub fn reachable_links_from(&self, node: &NodeRef<S>) -> HashSet<LinkRef<S>> {
        let mut seen_nodes = HashSet::from([node.clone()]);
        let mut seen_links = HashSet::new();
        let mut queue = VecDeque::from([node.clone()]);
        while let Some(node) = queue.pop_front() {
            for lref in self.links_from(&node) {
                seen_links.insert(lref.clone());
                let target = self.links[lref].target();
                if seen_nodes.insert(target.clone()) {
                    queue.push_back(target.clone());
                }
            }
        }
        seen_links
    }
}

impl<S: GChainSpec> Default for LinkGraph<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn endpoints(origin: u8, target: u8) -> LinkEndpoints<TestSpec> {
        LinkEndpoints::new(TestRef(origin), TestRef(target))
    }

    /// Blocks 1→2→3→4 as links 10, 11, 12, a checkpoint 1→3 as link 20, and
    /// a side branch 2→9 as link 30.
    fn graph() -> LinkGraph<TestSpec> {
        let mut g = LinkGraph::new();
        g.insert(TestRef(10), endpoints(1, 2));
        g.insert(TestRef(11), endpoints(2, 3));
        g.insert(TestRef(12), endpoints(3, 4));
        g.insert(TestRef(20), endpoints(1, 3));
        g.insert(TestRef(30), endpoints(2, 9));
        g
    }

    #[test]
    fn test_find_path_to_same_node_is_empty() {
        let path = graph()
            .find_path(&TestRef(1), &TestRef(1))
            .expect("test: find path");
        assert!(path.is_empty());
        assert_eq!(path.terminal_node(), &TestRef(1));
    }

    #[test]
    fn test_find_path_prefers_fewest_links() {
        let path = graph()
            .find_path(&TestRef(1), &TestRef(4))
            .expect("test: find path");
        assert_eq!(path.links(), &[TestRef(20), TestRef(12)]);
        assert_eq!(path.terminal_node(), &TestRef(4));
    }

    #[test]
    fn test_find_path_follows_branches() {
        let path = graph()
            .find_path(&TestRef(1), &TestRef(9))
            .expect("test: find path");
        assert_eq!(path.links(), &[TestRef(10), TestRef(30)]);
    }

    /// Links only go forwards, so a node behind the start is unreachable.
    #[test]
    fn test_find_path_backwards_or_to_unknown_node_is_none() {
        let g = graph();
        assert!(g.find_path(&TestRef(3), &TestRef(1)).is_none());
        assert!(g.find_path(&TestRef(1), &TestRef(7)).is_none());
    }

    #[test]
    fn test_remove_detaches_link_from_its_origin() {
        let mut g = graph();
        assert_eq!(g.remove(&TestRef(20)), Some(endpoints(1, 3)));

        assert!(!g.contains(&TestRef(20)));
        assert_eq!(g.links_from(&TestRef(1)), &[TestRef(10)]);
        let path = g
            .find_path(&TestRef(1), &TestRef(3))
            .expect("test: find path");
        assert_eq!(path.links(), &[TestRef(10), TestRef(11)]);
    }

    #[test]
    fn test_insert_existing_link_is_noop() {
        let mut g = graph();
        g.insert(TestRef(10), endpoints(1, 2));
        assert_eq!(g.links_from(&TestRef(1)), &[TestRef(10), TestRef(20)]);
    }

    #[test]
    fn test_reachable_links_covers_every_branch() {
        let g = graph();
        let from_two = g.reachable_links_from(&TestRef(2));
        assert_eq!(
            from_two,
            HashSet::from([TestRef(11), TestRef(12), TestRef(30)])
        );
        assert!(g.reachable_links_from(&TestRef(4)).is_empty());
    }
}
