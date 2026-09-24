//! Graph queries answered by walking the chain provider.
//!
//! The provider knows every link there is; an executor only wants to travel
//! over the ones it can use, so each query takes a predicate saying which
//! those are.  Nothing about the graph is kept between queries, so anything
//! worth caching belongs in the provider.

use std::collections::{HashMap, HashSet, VecDeque};

use strata_gchain_types::*;

use crate::errors::GExecError;

/// Finds a path of usable links from one node to another with the fewest
/// links, if the second is reachable from the first.
///
/// The path is empty when the nodes are the same.  Fewest links is the
/// cheapest pre-state to reconstruct, which naturally prefers a checkpoint
/// link over the run of block links it covers.
pub(crate) fn find_path<S: GChainSpec>(
    provider: &impl ChainProvider<Spec = S>,
    from: &NodeRef<S>,
    to: &NodeRef<S>,
    mut usable: impl FnMut(&LinkRef<S>) -> Result<bool, GExecError>,
) -> Result<Option<LinkPath<S>>, GExecError> {
    // BFS from `from`, remembering the link and origin each node was first
    // reached by.
    let mut arrived_by: HashMap<NodeRef<S>, (LinkRef<S>, NodeRef<S>)> = HashMap::new();
    let mut queue = VecDeque::from([from.clone()]);
    while let Some(node) = queue.pop_front() {
        if node == *to {
            break;
        }

        for lref in provider.fetch_forward_links(&node)? {
            if !usable(&lref)? {
                continue;
            }
            let target = fetch_endpoints(provider, &lref)?.target().clone();
            if target != *from && !arrived_by.contains_key(&target) {
                arrived_by.insert(target.clone(), (lref, node.clone()));
                queue.push_back(target);
            }
        }
    }

    if from != to && !arrived_by.contains_key(to) {
        return Ok(None);
    }

    // Walk the arrival links back from `to`, then lay them down forwards.
    let mut backwards = Vec::new();
    let mut node = to;
    while node != from {
        let (lref, origin) = &arrived_by[node];
        backwards.push((lref.clone(), node.clone()));
        node = origin;
    }
    backwards.reverse();
    Ok(Some(LinkPath::from_steps(from.clone(), backwards)))
}

/// Finds every usable link reachable from a node, following links forwards.
pub(crate) fn find_reachable_links<S: GChainSpec>(
    provider: &impl ChainProvider<Spec = S>,
    from: &NodeRef<S>,
    mut usable: impl FnMut(&LinkRef<S>) -> Result<bool, GExecError>,
) -> Result<HashSet<LinkRef<S>>, GExecError> {
    let mut seen_nodes = HashSet::from([from.clone()]);
    let mut seen_links = HashSet::new();
    let mut queue = VecDeque::from([from.clone()]);
    while let Some(node) = queue.pop_front() {
        for lref in provider.fetch_forward_links(&node)? {
            if !usable(&lref)? {
                continue;
            }
            let target = fetch_endpoints(provider, &lref)?.target().clone();
            seen_links.insert(lref);
            if seen_nodes.insert(target.clone()) {
                queue.push_back(target);
            }
        }
    }
    Ok(seen_links)
}

fn fetch_endpoints<S: GChainSpec>(
    provider: &impl ChainProvider<Spec = S>,
    lref: &LinkRef<S>,
) -> Result<LinkEndpoints<S>, GExecError> {
    provider
        .fetch_link_endpoints(lref)?
        .ok_or_else(|| GExecError::MissingLinkEndpoints(format!("{lref:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn refs(ids: &[u8]) -> Vec<TestRef> {
        ids.iter().copied().map(TestRef).collect()
    }

    /// Blocks 1→2→3→4 as links 10, 11, 12, a checkpoint 1→3 as link 20, and
    /// a side branch 2→9 as link 30.
    fn provider() -> TestProvider {
        let p = TestProvider::new();
        p.add_link(10, 1, 2);
        p.add_link(11, 2, 3);
        p.add_link(12, 3, 4);
        p.add_link(20, 1, 3);
        p.add_link(30, 2, 9);
        p
    }

    fn usable(ids: &[u8]) -> impl FnMut(&TestRef) -> Result<bool, GExecError> {
        let set: HashSet<_> = refs(ids).into_iter().collect();
        move |lref| Ok(set.contains(lref))
    }

    #[test]
    fn test_find_path_takes_fewest_usable_links() {
        let provider = provider();

        let path = find_path(
            &provider,
            &TestRef(1),
            &TestRef(4),
            usable(&[10, 11, 12, 20]),
        )
        .expect("test: find path")
        .expect("test: path exists");
        assert_eq!(path.links(), &refs(&[20, 12]));
        assert_eq!(path.nodes(), &refs(&[1, 3, 4]));

        // Without the checkpoint the blocks are the only way.
        let path = find_path(&provider, &TestRef(1), &TestRef(3), usable(&[10, 11, 12]))
            .expect("test: find path")
            .expect("test: path exists");
        assert_eq!(path.links(), &refs(&[10, 11]));

        // A node reaches itself with no links at all.
        let path = find_path(&provider, &TestRef(1), &TestRef(1), usable(&[]))
            .expect("test: find path")
            .expect("test: path exists");
        assert!(path.is_empty());
        assert_eq!(path.base_node(), &TestRef(1));

        // Links that aren't usable might as well not be there.
        assert!(
            find_path(&provider, &TestRef(1), &TestRef(9), usable(&[10]))
                .expect("test: find path")
                .is_none()
        );
        assert!(
            find_path(&provider, &TestRef(1), &TestRef(4), usable(&[]))
                .expect("test: find path")
                .is_none()
        );
    }

    #[test]
    fn test_reachable_links_follows_only_usable_links() {
        let provider = provider();

        let mut reached: Vec<_> =
            find_reachable_links(&provider, &TestRef(1), usable(&[10, 11, 30, 12]))
                .expect("test: reachable")
                .into_iter()
                .collect();
        reached.sort();
        assert_eq!(reached, refs(&[10, 11, 12, 30]));

        let mut reached: Vec<_> =
            find_reachable_links(&provider, &TestRef(1), usable(&[20, 12, 11]))
                .expect("test: reachable")
                .into_iter()
                .collect();
        reached.sort();
        assert_eq!(reached, refs(&[12, 20]));
    }
}
