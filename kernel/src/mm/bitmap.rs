use bit_field::BitField;

pub struct Bitmap(&'static mut [usize]);

impl Bitmap {
    pub fn new(inner: &'static mut [usize]) -> Self {
        inner.fill(0);
        Self(inner)
    }

    pub fn len(&self) -> usize {
        self.0.len() * 64
    }

    pub fn set(&mut self, pos: usize, value: bool) {
        let byte_pos = pos / 64;
        let bit_pos = pos % 64;
        self.0[byte_pos].set_bit(bit_pos, value);
    }

    pub fn get(&self, pos: usize) -> bool {
        let byte_pos = pos / 64;
        let bit_pos = pos % 64;
        self.0[byte_pos].get_bit(bit_pos)
    }

    pub fn set_range(&mut self, l: usize, r: usize, value: bool) {
        for pos in l..r {
            self.set(pos, value);
        }
    }
}
