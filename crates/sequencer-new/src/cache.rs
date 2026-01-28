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
/// Templates automatically expire after a TTL and are cleaned up periodically.
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

        // Clean up expired entries while we're here
        self.cleanup_expired();

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
