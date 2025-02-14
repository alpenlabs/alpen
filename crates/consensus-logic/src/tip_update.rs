//! Types relating to updating the tip and planning reorgs.

use strata_primitives::l2::L2BlockId;
use tracing::*;

use crate::{errors::Error, unfinalized_tracker};

/// Describes how we're updating the chain's tip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TipUpdate {
    /// Simply extending the chain tip from a block (left) to the next (right).
    ///
    /// The slot of the first block MUST be lower than the slot of the second
    /// one.
    ExtendTip(L2BlockId, L2BlockId),

    /// A reorg that has to undo some blocks first before extending back up to
    /// the next block.
    Reorg(Reorg),

    /// Just rolling back to an earlier block without playing out new ones.
    ///
    /// This might only happen when we have manual intervention.
    // maybe it'll also happen if we have async subchain updates?
    Revert(L2BlockId, L2BlockId),
}

impl TipUpdate {
    /// Returns the new tip, regardless of the type of change.
    pub fn new_tip(&self) -> &L2BlockId {
        match self {
            Self::ExtendTip(_, new) => new,
            Self::Reorg(reorg) => reorg.new_tip(),
            Self::Revert(_, new) => new,
        }
    }

    /// Returns the old tip, regardless of the type of change.
    pub fn old_tip(&self) -> &L2BlockId {
        match self {
            Self::ExtendTip(old, _) => old,
            Self::Reorg(reorg) => reorg.old_tip(),
            Self::Revert(old, _) => old,
        }
    }

    /// Returns if the tip update is expected to revert blocks.
    pub fn is_reverting(&self) -> bool {
        match self {
            Self::Reorg(reorg) => reorg.revert_iter().next().is_some(),
            Self::Revert(_, _) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reorg {
    /// Blocks we're removing, in the order we're removing them.
    down: Vec<L2BlockId>,

    /// Pivot block that's shared on both chains.
    pivot: L2BlockId,

    /// Blocks we're adding, in the order we're adding them.
    up: Vec<L2BlockId>,
}

impl Reorg {
    pub fn revert_iter(&self) -> impl Iterator<Item = &L2BlockId> {
        self.down.iter()
    }

    pub fn pivot(&self) -> &L2BlockId {
        &self.pivot
    }

    pub fn apply_iter(&self) -> impl Iterator<Item = &L2BlockId> {
        self.up.iter()
    }

    pub fn old_tip(&self) -> &L2BlockId {
        if self.down.is_empty() {
            &self.pivot
        } else {
            &self.down[0]
        }
    }

    pub fn new_tip(&self) -> &L2BlockId {
        if self.up.is_empty() {
            &self.pivot
        } else {
            &self.up[self.up.len() - 1]
        }
    }

    /// If the reorg isn't really a reorg, it's just rolling back blocks or
    /// adding new blocks.
    pub fn is_weird(&self) -> bool {
        self.up.is_empty() || self.down.is_empty()
    }

    /// If the reorg describes no change in tip.
    pub fn is_identity(&self) -> bool {
        self.up.is_empty() && self.down.is_empty()
    }
}

/// Computes the update path from a block to a new tip, aborting at some reorg
/// search depth if necessary.  This behaves sensibly when one block is an
/// ancestor of another or are the same, although that might not be useful.
pub fn compute_tip_update(
    start: &L2BlockId,
    dest: &L2BlockId,
    limit_depth: usize,
    tracker: &unfinalized_tracker::UnfinalizedBlockTracker,
) -> Result<Option<TipUpdate>, Error> {
    // Fast path for when there's no change.
    if start == dest {
        return Ok(None);
    }

    // Fast path for simply extending the tip.
    let dest_parent = tracker
        .get_parent(dest)
        .expect("fcm: chain tracker missing new block");
    if dest_parent == start {
        return Ok(Some(TipUpdate::ExtendTip(*start, *dest)));
    }

    // Create a vec of parents from tip to the beginning(before limit depth) and then move forwards
    // until the blockids don't match
    let mut down_blocks: Vec<_> = std::iter::successors(Some(start), |n| tracker.get_parent(n))
        .take(limit_depth)
        .collect();
    let mut up_blocks: Vec<_> = std::iter::successors(Some(dest), |n| tracker.get_parent(n))
        .take(limit_depth)
        .collect();

    // This shouldn't happen because we probably would have found it on the
    // initial check.  But if the search depth is 0 then maybe.
    if down_blocks.is_empty() || up_blocks.is_empty() {
        return Ok(None);
    }

    // Now trim the vectors such that they start from the same ancestor
    if let Some(pos) = up_blocks
        .iter()
        .position(|&x| x == *down_blocks.last().unwrap())
    {
        up_blocks.drain(pos + 1..);
    } else if let Some(pos) = down_blocks
        .iter()
        .position(|&x| x == *up_blocks.last().unwrap())
    {
        down_blocks.drain(pos + 1..);
    } else {
        return Ok(None);
    }

    // TODO figure out if this is actually just a revert

    // Now reverse so that common ancestor is at the beginning
    down_blocks.reverse();
    up_blocks.reverse();

    // Now move forwards, until the blocks do not match
    let mut pivot_idx = None;
    for (i, (&d, &u)) in down_blocks.iter().zip(&up_blocks).enumerate() {
        if *d != *u {
            break;
        }
        pivot_idx = Some(i);
    }

    // At this point we're sure we can't have anything *but* a reorg.
    if let Some(idx) = pivot_idx {
        let pivot = *up_blocks[idx];
        let down = down_blocks.drain(idx + 1..).copied().rev().collect();
        let up = up_blocks.drain(idx + 1..).copied().collect();
        let reorg = Reorg { pivot, down, up };
        return Ok(Some(TipUpdate::Reorg(reorg)));
    }

    // At this point, we haven't been able to figure it out, abort.
    warn!(%start, %dest, "unable to find tip update path through any normal means");
    Ok(None)
}

#[cfg(test)]
mod tests {
    use rand::{rngs::OsRng, RngCore};
    use strata_state::id::L2BlockId;

    use super::{compute_tip_update, Reorg};
    use crate::unfinalized_tracker;

    fn rand_blkid() -> L2BlockId {
        let mut buf = [0; 32];
        OsRng.fill_bytes(&mut buf);
        L2BlockId::from(strata_primitives::buf::Buf32::from(buf))
    }

    #[test]
    fn test_eq_len() {
        let base = rand_blkid();
        let mut tracker = unfinalized_tracker::UnfinalizedBlockTracker::new_empty(base);

        // Set up the two branches.
        let side_1 = [base, rand_blkid(), rand_blkid(), rand_blkid()];
        let side_2 = [side_1[1], rand_blkid(), rand_blkid()];
        eprintln!("base {base:?}\nside1 {side_1:#?}\nside2 {side_2:#?}");

        let exp_reorg = Reorg {
            down: vec![side_1[3], side_1[2]],
            pivot: side_1[1],
            up: vec![side_2[1], side_2[2]],
        };

        // Insert them.
        side_1
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));
        side_2
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));

        let reorg =
            compute_tip_update(side_1.last().unwrap(), side_2.last().unwrap(), 10, &tracker);

        let reorg = reorg.expect("test: reorg not found");
        eprintln!("expected {exp_reorg:#?}\nfound {reorg:#?}");
        assert_eq!(reorg, exp_reorg);
    }

    #[test]
    fn test_longer_down() {
        let base = rand_blkid();
        let mut tracker = unfinalized_tracker::UnfinalizedBlockTracker::new_empty(base);

        // Set up the two branches.
        let side_1 = [base, rand_blkid(), rand_blkid(), rand_blkid(), rand_blkid()];
        let side_2 = [side_1[1], rand_blkid(), rand_blkid()];
        eprintln!("base {base:?}\nside1 {side_1:#?}\nside2 {side_2:#?}");

        let exp_reorg = Reorg {
            down: vec![side_1[4], side_1[3], side_1[2]],
            pivot: side_1[1],
            up: vec![side_2[1], side_2[2]],
        };

        // Insert them.
        side_1
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));
        side_2
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));

        let reorg =
            compute_tip_update(side_1.last().unwrap(), side_2.last().unwrap(), 10, &tracker);

        let reorg = reorg.expect("test: reorg not found");
        eprintln!("expected {exp_reorg:#?}\nfound {reorg:#?}");
        assert_eq!(reorg, exp_reorg);
    }

    #[test]
    fn test_longer_up() {
        let base = rand_blkid();
        let mut tracker = unfinalized_tracker::UnfinalizedBlockTracker::new_empty(base);

        // Set up the two branches.
        let side_1 = [base, rand_blkid(), rand_blkid(), rand_blkid()];
        let side_2 = [
            side_1[1],
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
        ];
        eprintln!("base {base:?}\nside1 {side_1:#?}\nside2 {side_2:#?}");

        let exp_reorg = Reorg {
            down: vec![side_1[3], side_1[2]],
            pivot: side_1[1],
            up: vec![side_2[1], side_2[2], side_2[3], side_2[4]],
        };

        // Insert them.
        side_1
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));
        side_2
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));

        let reorg =
            compute_tip_update(side_1.last().unwrap(), side_2.last().unwrap(), 10, &tracker);

        let reorg = reorg.expect("test: reorg not found");
        eprintln!("expected {exp_reorg:#?}\nfound {reorg:#?}");
        assert_eq!(reorg, exp_reorg);
    }

    #[test]
    fn test_too_deep() {
        let base = rand_blkid();
        let mut tracker = unfinalized_tracker::UnfinalizedBlockTracker::new_empty(base);

        // Set up the two branches.
        let side_1 = [
            base,
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
        ];
        let side_2 = [
            side_1[1],
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
        ];
        eprintln!("base {base:?}\nside1 {side_1:#?}\nside2 {side_2:#?}");

        // Insert them.
        side_1
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));
        side_2
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));

        let reorg = compute_tip_update(side_1.last().unwrap(), side_2.last().unwrap(), 3, &tracker);

        if let Some(reorg) = reorg {
            eprintln!("reorg found wrongly {reorg:#?}");
            panic!("reorg found wrongly");
        }
    }

    #[test]
    fn test_start_ancestor() {
        let base = rand_blkid();
        let mut tracker = unfinalized_tracker::UnfinalizedBlockTracker::new_empty(base);

        // Set up the two branches.
        let chain = [
            base,
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
        ];
        eprintln!("base {base:?}\nchain {chain:#?}");

        // Insert them.
        chain
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));

        let src = &chain[3];
        let dest = chain.last().unwrap();
        let reorg = compute_tip_update(src, dest, 10, &tracker);

        let exp_reorg = Reorg {
            down: Vec::new(),
            pivot: *src,
            up: vec![chain[4], chain[5], chain[6]],
        };

        let reorg = reorg.expect("test: reorg not found");
        eprintln!("expected {exp_reorg:#?}\nfound {reorg:#?}");
        assert_eq!(reorg, exp_reorg);
        assert!(reorg.is_weird());
    }

    #[test]
    fn test_end_ancestor() {
        let base = rand_blkid();
        let mut tracker = unfinalized_tracker::UnfinalizedBlockTracker::new_empty(base);

        // Set up the two branches.
        let chain = [
            base,
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
        ];
        eprintln!("base {base:?}\nchain {chain:#?}");

        // Insert them.
        chain
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));

        let src = chain.last().unwrap();
        let dest = &chain[3];
        let reorg = compute_tip_update(src, dest, 10, &tracker);

        let exp_reorg = Reorg {
            down: vec![chain[6], chain[5], chain[4]],
            pivot: *dest,
            up: Vec::new(),
        };

        let reorg = reorg.expect("test: reorg not found");
        eprintln!("expected {exp_reorg:#?}\nfound {reorg:#?}");
        assert_eq!(reorg, exp_reorg);
        assert!(reorg.is_weird());
    }

    #[test]
    fn test_identity() {
        let base = rand_blkid();
        let mut tracker = unfinalized_tracker::UnfinalizedBlockTracker::new_empty(base);

        // Set up the two branches.
        let chain = [
            base,
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
        ];
        eprintln!("base {base:?}\nchain {chain:#?}");

        // Insert them.
        chain
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));

        let src = chain.last().unwrap();
        let dest = src;
        let reorg = compute_tip_update(src, dest, 10, &tracker);
        eprintln!("reorg found wrongly {reorg:#?}");

        let exp_reorg = Reorg {
            down: Vec::new(),
            pivot: *dest,
            up: Vec::new(),
        };

        let reorg = reorg.expect("test: reorg not found");
        eprintln!("expected {exp_reorg:#?}\nfound {reorg:#?}");
        assert_eq!(reorg, exp_reorg);
        assert!(reorg.is_identity());
    }

    #[test]
    fn test_longer_down_depth_less_than_chain_length() {
        let base = rand_blkid();
        let mut tracker = unfinalized_tracker::UnfinalizedBlockTracker::new_empty(base);

        // Set up the two branches.
        let side_1 = [base, rand_blkid(), rand_blkid(), rand_blkid(), rand_blkid()];
        let side_2 = [side_1[1], rand_blkid(), rand_blkid()];
        let limit_depth = 4; // This is not larger than the longest chain length
        eprintln!("base {base:?}\nside1 {side_1:#?}\nside2 {side_2:#?}");

        let exp_reorg = Reorg {
            down: vec![side_1[4], side_1[3], side_1[2]],
            pivot: side_1[1],
            up: vec![side_2[1], side_2[2]],
        };

        // Insert them.
        side_1
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));
        side_2
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));

        let reorg = compute_tip_update(
            side_1.last().unwrap(),
            side_2.last().unwrap(),
            limit_depth,
            &tracker,
        );

        let reorg = reorg.expect("test: reorg not found");
        eprintln!("expected {exp_reorg:#?}\nfound {reorg:#?}");
        assert_eq!(reorg, exp_reorg);
    }

    #[test]
    fn test_longer_up_depth_less_than_chain_length() {
        let base = rand_blkid();
        let mut tracker = unfinalized_tracker::UnfinalizedBlockTracker::new_empty(base);

        // Set up the two branches.
        let side_1 = [base, rand_blkid(), rand_blkid(), rand_blkid()];
        let side_2 = [
            side_1[1],
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
            rand_blkid(),
        ];
        let limit_depth = 5; // This is not larger than the longest chain length
        eprintln!("base {base:?}\nside1 {side_1:#?}\nside2 {side_2:#?}");

        let exp_reorg = Reorg {
            down: vec![side_1[3], side_1[2]],
            pivot: side_1[1],
            up: vec![side_2[1], side_2[2], side_2[3], side_2[4]],
        };

        // Insert them.
        side_1
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));
        side_2
            .windows(2)
            .for_each(|pair| tracker.insert_fake_block(pair[1], pair[0]));

        let reorg = compute_tip_update(
            side_1.last().unwrap(),
            side_2.last().unwrap(),
            limit_depth,
            &tracker,
        );

        let reorg = reorg.expect("test: reorg not found");
        eprintln!("expected {exp_reorg:#?}\nfound {reorg:#?}");
        assert_eq!(reorg, exp_reorg);
    }
}
