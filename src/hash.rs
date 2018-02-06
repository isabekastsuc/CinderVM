//! Small hashes for the journal chain, snapshot integrity, and the image
//! checksum. Deliberately not a cryptographic hash: these protect against
//! corruption and replay drift, not adversaries.

/// FNV-1a 64 over bytes.
#[must_use]
pub fn hash_bytes(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Chain two hashes: `hash_bytes` of the previous hash and the next block.
#[must_use]
pub fn chain(prev: u64, data: &[u8]) -> u64 {
    let mut buf = Vec::with_capacity(8 + data.len());
    buf.extend_from_slice(&prev.to_le_bytes());
    buf.extend_from_slice(data);
