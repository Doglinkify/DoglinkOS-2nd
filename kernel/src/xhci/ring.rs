//! In-memory producer and consumer rings with an explicit cycle state.

use super::trb::{TRB_CYCLE, Trb, link_trb};
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RingError {
    Full,
    Empty,
}

pub struct ProducerRing {
    entries: Vec<Trb>,
    index: usize,
    cycle: bool,
    len: usize,
}

impl ProducerRing {
    pub fn new(usable_entries: usize) -> Self {
        assert!(usable_entries >= 2);
        let mut entries = vec![Trb::default(); usable_entries + 1];
        entries[usable_entries] = link_trb(0, true);
        Self {
            entries,
            index: 0,
            cycle: true,
            len: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.entries.len() - 1
    }

    pub fn push(&mut self, mut trb: Trb) -> Result<usize, RingError> {
        if self.len == self.capacity() {
            return Err(RingError::Full);
        }
        trb.control = (trb.control & !TRB_CYCLE) | self.cycle as u32;
        let at = self.index;
        self.entries[at] = trb;
        self.len += 1;
        let next = (self.index + 1) % self.capacity();
        self.index = next;
        if self.index == 0 {
            self.cycle = !self.cycle;
        }
        let capacity = self.capacity();
        self.entries[capacity] = link_trb(0, self.cycle);
        Ok(at)
    }

    pub fn get(&self, index: usize) -> Option<Trb> {
        self.entries.get(index).copied()
    }

    pub fn cycle(&self) -> bool {
        self.cycle
    }
}

pub struct EventRing {
    entries: Vec<Trb>,
    consumer: usize,
    consumer_cycle: bool,
    producer: usize,
    producer_cycle: bool,
    used: usize,
}

impl EventRing {
    pub fn new(entries: usize) -> Self {
        assert!(entries > 0);
        Self {
            entries: vec![Trb::default(); entries],
            consumer: 0,
            consumer_cycle: true,
            producer: 0,
            producer_cycle: true,
            used: 0,
        }
    }

    pub fn publish(&mut self, trb: Trb) -> Result<(), RingError> {
        if self.used == self.entries.len() {
            return Err(RingError::Full);
        }
        self.entries[self.producer] = Trb {
            control: trb.control | self.producer_cycle as u32,
            ..trb
        };
        self.producer = (self.producer + 1) % self.entries.len();
        if self.producer == 0 {
            self.producer_cycle = !self.producer_cycle;
        }
        self.used += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Result<Trb, RingError> {
        if self.used == 0 {
            return Err(RingError::Empty);
        }
        let trb = self.entries[self.consumer];
        if (trb.control & TRB_CYCLE != 0) != self.consumer_cycle {
            return Err(RingError::Empty);
        }
        self.entries[self.consumer] = Trb::default();
        self.consumer += 1;
        if self.consumer == self.entries.len() {
            self.consumer = 0;
            self.consumer_cycle = !self.consumer_cycle;
        }
        self.used -= 1;
        Ok(trb)
    }

    pub fn consumer_index(&self) -> usize {
        self.consumer
    }
}

pub fn test() {
    let mut ring = ProducerRing::new(3);
    assert_eq!(ring.push(Trb::default()), Ok(0));
    assert_eq!(ring.push(Trb::default()), Ok(1));
    assert_eq!(ring.push(Trb::default()), Ok(2));
    assert_eq!(ring.push(Trb::default()), Err(RingError::Full));
    assert!(!ring.cycle());
    assert_eq!(ring.get(3).unwrap().control & TRB_CYCLE, 0);

    let mut events = EventRing::new(2);
    assert_eq!(events.pop(), Err(RingError::Empty));
    events.publish(Trb::default()).unwrap();
    assert!(events.pop().is_ok());
    assert_eq!(events.consumer_index(), 1);
}
