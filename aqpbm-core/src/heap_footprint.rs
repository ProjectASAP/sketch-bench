//! What asap_sketchlib 0.3's `HHHeap` (behind `CMSHeap`, `CSHeap` and
//! UnivMon's layers) holds for its residents: the heap array of `HHItem`s, a
//! parallel digest column (`slots: Vec<u64>`), and a position index
//! `HashMap<u64, SmallVec<[usize; 2]>>` that hashbrown sizes in power-of-two
//! buckets at 7/8 load. The residents' own key bytes, for string keys, are the
//! caller's to add.

/// One index bucket: the `u64` digest, the inline `SmallVec<[usize; 2]>`
/// (two positions plus its length word) and hashbrown's control byte.
pub const HH_HEAP_INDEX_ENTRY_BYTES: usize = 8 + 3 * std::mem::size_of::<usize>() + 1;

/// Buckets hashbrown allocates for `capacity` entries: at 7/8 load, rounded
/// up to a power of two.
pub fn hash_buckets(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    (capacity.div_ceil(7) * 8).next_power_of_two()
}

/// Bytes an `HHHeap` holding `capacity` residents keeps, given the size of
/// one `HHItem`: the item, its digest, and its share of the index.
pub fn hh_heap_bytes(capacity: usize, item_bytes: usize) -> usize {
    capacity * (item_bytes + std::mem::size_of::<u64>())
        + hash_buckets(capacity) * HH_HEAP_INDEX_ENTRY_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resident_costs_more_than_its_item() {
        let item = 40;
        for (small, large) in [(32, 128), (128, 2048)] {
            let grown = hh_heap_bytes(large, item) - hh_heap_bytes(small, item);
            assert!(grown > (large - small) * item, "{small}->{large}: {grown}");
        }
        assert_eq!(hash_buckets(32), 64);
        assert_eq!(hh_heap_bytes(0, item), 0);
    }
}
