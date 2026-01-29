use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use strata_ol_block_assembly::FullBlockTemplate;
use strata_primitives::OLBlockId;

use crate::types::BlockTemplateExt;

/// A cache for block templates with time-based expiration.
///
/// This replaces the old worker's HashMap that never cleaned up templates.
/// Templates automatically expire after a TTL and are cleaned up during insertions.
pub struct TemplateCache {
    templates: HashMap<OLBlockId, CachedTemplate>,
    ttl: Duration,
}

struct CachedTemplate {
    template: FullBlockTemplate,
    created_at: Instant,
}

impl TemplateCache {
    /// Creates a new cache with the specified TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            templates: HashMap::new(),
            ttl,
        }
    }

    /// Inserts a template into the cache.
    pub fn insert(&mut self, template: FullBlockTemplate) {
        let id = template.template_id();
        self.templates.insert(
            id,
            CachedTemplate {
                template,
                created_at: Instant::now(),
            },
        );
        eprintln!("inserted templates {:?}", self.templates.len());

        // Clean up expired entries while we're here
        self.cleanup_expired();
    }

    /// Gets a template by parent block ID if it exists and hasn't expired.
    pub fn get_by_parent(&mut self, parent_id: &OLBlockId) -> Option<FullBlockTemplate> {
        // Find template with matching parent
        let template = self
            .templates
            .values()
            .find(|cached| {
                cached.template.parent() == *parent_id && cached.created_at.elapsed() < self.ttl
            })
            .map(|cached| cached.template.clone());

        template
    }

    /// Removes and returns a template if it exists.
    pub fn remove(&mut self, id: &OLBlockId) -> Option<FullBlockTemplate> {
        self.templates.remove(id).map(|cached| cached.template)
    }

    /// Removes expired templates from the cache.
    pub fn cleanup_expired(&mut self) {
        let now = Instant::now();
        self.templates
            .retain(|_, cached| now.duration_since(cached.created_at) < self.ttl);
    }

    /// Returns the number of cached templates (including expired ones).
    pub fn len(&self) -> usize {
        self.templates.len()
    }
}

#[cfg(test)]
mod tests {
    use strata_ol_chain_types_new::{BlockFlags, OLBlockBody, OLBlockHeader, OLTxSegment};
    use strata_primitives::Buf32;

    use super::*;

    fn create_test_template(parent_id: OLBlockId) -> FullBlockTemplate {
        let header = OLBlockHeader {
            parent_blkid: parent_id,
            timestamp: 1000,
            slot: 1,
            epoch: 0,
            flags: BlockFlags::from(0),
            body_root: [0u8; 32].into(),
            state_root: [0u8; 32].into(),
            logs_root: [0u8; 32].into(),
        };

        let body = OLBlockBody {
            tx_segment: Some(OLTxSegment { txs: vec![].into() }).into(),
            l1_update: None.into(),
        };

        FullBlockTemplate::new(header, body)
    }

    #[test]
    fn test_insert_and_get_by_parent() {
        let mut cache = TemplateCache::new(Duration::from_secs(60));
        let parent_id = OLBlockId::from(Buf32([1u8; 32]));
        let template = create_test_template(parent_id);

        cache.insert(template.clone());

        let retrieved = cache.get_by_parent(&parent_id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().header(), template.header());
    }

    #[test]
    fn test_ttl_expiration_on_get() {
        let mut cache = TemplateCache::new(Duration::from_millis(50));
        let parent_id = OLBlockId::from(Buf32([1u8; 32]));
        let template = create_test_template(parent_id);

        cache.insert(template.clone());
        assert_eq!(cache.len(), 1);

        // Should be present immediately
        assert!(cache.get_by_parent(&parent_id).is_some());

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(100));

        // get_by_parent checks expiration but doesn't clean up
        assert!(cache.get_by_parent(&parent_id).is_none());
        // Template is still in map until cleanup
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_cleanup_on_insert() {
        let mut cache = TemplateCache::new(Duration::from_millis(50));

        // Insert first template
        let parent1 = OLBlockId::from(Buf32([1u8; 32]));
        let template1 = create_test_template(parent1);
        cache.insert(template1);
        assert_eq!(cache.len(), 1);

        // Wait for it to expire
        std::thread::sleep(Duration::from_millis(100));

        // Insert second template - should trigger cleanup
        let parent2 = OLBlockId::from(Buf32([3u8; 32]));
        let template2 = create_test_template(parent2);
        cache.insert(template2);

        // First should be cleaned up, only second remains
        assert_eq!(cache.len(), 1);
        assert!(cache.get_by_parent(&parent1).is_none());
        assert!(cache.get_by_parent(&parent2).is_some());
    }

    #[test]
    fn test_remove_template() {
        let mut cache = TemplateCache::new(Duration::from_secs(60));
        let parent_id = OLBlockId::from(Buf32([1u8; 32]));
        let template = create_test_template(parent_id);
        let template_id = template.template_id();

        cache.insert(template.clone());
        assert_eq!(cache.len(), 1);

        let removed = cache.remove(&template_id);
        eprintln!("{:?}", removed);
        assert!(removed.is_some());
        assert_eq!(cache.len(), 0);

        // Should not find it anymore
        assert!(cache.get_by_parent(&parent_id).is_none());
    }

    #[test]
    fn test_multiple_templates_different_parents() {
        let mut cache = TemplateCache::new(Duration::from_secs(60));

        let parent1 = OLBlockId::from(Buf32([1u8; 32]));
        let parent2 = OLBlockId::from(Buf32([2u8; 32]));

        let template1 = create_test_template(parent1);
        let template2 = create_test_template(parent2);

        cache.insert(template1.clone());
        cache.insert(template2.clone());

        // Each parent should find its own template
        let found1 = cache.get_by_parent(&parent1);
        assert!(found1.is_some());
        assert_eq!(found1.unwrap().parent(), parent1);

        let found2 = cache.get_by_parent(&parent2);
        assert!(found2.is_some());
        assert_eq!(found2.unwrap().parent(), parent2);

        // Non-existent parent should return None
        let parent3 = OLBlockId::from(Buf32([5u8; 32]));
        assert!(cache.get_by_parent(&parent3).is_none());
    }

    #[test]
    fn test_explicit_cleanup_expired() {
        let mut cache = TemplateCache::new(Duration::from_millis(50));

        // Insert multiple templates at different times
        let template1 = create_test_template(OLBlockId::from(Buf32([1u8; 32])));
        cache.insert(template1);

        std::thread::sleep(Duration::from_millis(30));

        let template2 = create_test_template(OLBlockId::from(Buf32([3u8; 32])));
        cache.insert(template2);
        // Insert triggers cleanup but nothing expired yet
        assert_eq!(cache.len(), 2);

        // Wait for first to expire but not second
        std::thread::sleep(Duration::from_millis(30));

        // Manually trigger cleanup
        cache.cleanup_expired();
        assert_eq!(cache.len(), 1);
        assert!(cache
            .get_by_parent(&OLBlockId::from(Buf32([1u8; 32])))
            .is_none());
        assert!(cache
            .get_by_parent(&OLBlockId::from(Buf32([3u8; 32])))
            .is_some());
    }
}
