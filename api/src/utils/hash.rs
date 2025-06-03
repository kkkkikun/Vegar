use core::hash::Hasher;

struct SimpleHasher(u64);
impl Hasher for SimpleHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.wrapping_mul(31).wrapping_add(b as u64);
        }
    }
}

pub fn hash_string(s: &str) -> u64 {
    let mut hasher = SimpleHasher(0);
    hasher.write(s.as_bytes());
    hasher.finish()
}
