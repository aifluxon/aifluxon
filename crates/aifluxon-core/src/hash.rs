pub fn content_hash(content: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_for_the_same_payload() {
        assert_eq!(content_hash("echo hi"), content_hash("echo hi"));
        assert_ne!(content_hash("echo hi"), content_hash("echo bye"));
        assert!(content_hash("echo hi").starts_with("fnv1a64:"));
    }
}
