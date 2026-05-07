use zeroize::{Zeroize, ZeroizeOnDrop};

/// 256-bit symmetric key, zeroized on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; 32]);

impl MasterKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_bytes() {
        let bytes = [9u8; 32];
        let key = MasterKey::new(bytes);
        assert_eq!(key.as_bytes(), &bytes);
    }

    #[test]
    fn clone_independent() {
        let key = MasterKey::new([1u8; 32]);
        let clone = key.clone();
        assert_eq!(key.as_bytes(), clone.as_bytes());
    }
}
