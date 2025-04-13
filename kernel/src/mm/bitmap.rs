use bit_field::BitField;

pub struct PageMan(pub &'static mut [usize], &'static mut [u8]);

#[allow(clippy::len_without_is_empty)]
impl PageMan {
    pub fn calc_size(len: u64) -> (u64, u64, u64) {
        let tmp = len.div_ceil(64);
        (tmp, len, tmp * 8 + len)
    }

    pub fn new(bmp: &'static mut [usize], refcnt: &'static mut [u8]) -> Self {
        bmp.fill(0);
        refcnt.fill(0);
        Self(bmp, refcnt)
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

    pub fn incref(&mut self, pos: usize) {
        self.1[pos] += 1;
    }

    pub fn decref(&mut self, pos: usize) {
        self.1[pos] -= 1;
    }

    pub fn getref(&self, pos: usize) -> u8 {
        self.1[pos]
    }
}
