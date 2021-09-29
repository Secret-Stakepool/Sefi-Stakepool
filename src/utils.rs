use secret_toolkit::crypto::sha_256;
use subtle::ConstantTimeEq;

use crate::{viewing_keys::VIEWING_KEY_SIZE};

pub fn ct_slice_compare(s1: &[u8], s2: &[u8]) -> bool {
    bool::from(s1.ct_eq(s2))
}

pub fn create_hashed_password(s1: &str) -> [u8; VIEWING_KEY_SIZE] {
    sha_256(s1.as_bytes())
}



