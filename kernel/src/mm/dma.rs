//! DMA and MMIO helpers for devices using the HHDM direct map.
//!
//! There is no IOMMU support yet. DMA addresses are therefore physical
//! addresses and device drivers must keep a `DmaBuffer` alive until hardware
//! has stopped accessing it.

use core::ptr::NonNull;

use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{
    Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size2MiB, Size4KiB,
    mapper::FlagUpdateError,
};
use x86_64::{PhysAddr, VirtAddr};

use crate::mm::page_alloc::{
    DLOSFrameAllocator, PAGE_SIZE, dealloc_continuous_mem, find_aligned_continuous_mem,
};
use crate::mm::phys_to_virt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmaError {
    Empty,
    InvalidAlignment,
    OutOfMemory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MmioError {
    Empty,
    AddressOverflow,
    MappingMissing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PageRange {
    base: u64,
    pages: usize,
    offset: usize,
}

impl PageRange {
    fn new(address: u64, len: usize) -> Result<Self, MmioError> {
        if len == 0 {
            return Err(MmioError::Empty);
        }
        let base = address & !(PAGE_SIZE as u64 - 1);
        let offset = (address - base) as usize;
        let span = offset.checked_add(len).ok_or(MmioError::AddressOverflow)?;
        let pages = span
            .checked_add(PAGE_SIZE - 1)
            .ok_or(MmioError::AddressOverflow)?
            / PAGE_SIZE;
        base.checked_add((pages - 1) as u64 * PAGE_SIZE as u64)
            .ok_or(MmioError::AddressOverflow)?;
        Ok(Self {
            base,
            pages,
            offset,
        })
    }
}

/// Physically contiguous, zeroed memory suitable for DMA.
pub struct DmaBuffer {
    physical_address: u64,
    virtual_address: NonNull<u8>,
    pages: usize,
    alignment: usize,
}

impl DmaBuffer {
    pub fn new(len: usize, alignment: usize) -> Result<Self, DmaError> {
        if len == 0 {
            return Err(DmaError::Empty);
        }
        if !alignment.is_power_of_two() {
            return Err(DmaError::InvalidAlignment);
        }
        let alignment = alignment.max(PAGE_SIZE);
        let pages = len
            .checked_add(PAGE_SIZE - 1)
            .ok_or(DmaError::OutOfMemory)?
            / PAGE_SIZE;
        let physical_address =
            find_aligned_continuous_mem(pages, alignment).ok_or(DmaError::OutOfMemory)?;
        let virtual_address = NonNull::new(phys_to_virt(physical_address) as *mut u8)
            .expect("HHDM virtual address must not be null");
        unsafe { core::ptr::write_bytes(virtual_address.as_ptr(), 0, pages * PAGE_SIZE) };
        Ok(Self {
            physical_address,
            virtual_address,
            pages,
            alignment,
        })
    }

    pub fn physical_address(&self) -> u64 {
        self.physical_address
    }

    pub fn virtual_address(&self) -> u64 {
        self.virtual_address.as_ptr() as u64
    }

    pub fn pages(&self) -> usize {
        self.pages
    }

    pub fn len(&self) -> usize {
        self.pages * PAGE_SIZE
    }

    pub fn alignment(&self) -> usize {
        self.alignment
    }

    pub fn as_ptr(&self) -> *mut u8 {
        self.virtual_address.as_ptr()
    }
}

impl Drop for DmaBuffer {
    fn drop(&mut self) {
        dealloc_continuous_mem(self.physical_address, self.pages);
    }
}

/// An HHDM MMIO range with all constituent pages marked uncacheable.
pub struct MmioMapping {
    physical_address: u64,
    virtual_address: NonNull<u8>,
    len: usize,
    page_base: u64,
    pages: usize,
}

impl MmioMapping {
    /// # Safety
    ///
    /// `physical_address..physical_address + len` must identify a device MMIO
    /// range. It remains mapped until the kernel changes the active page table.
    pub unsafe fn map(physical_address: u64, len: usize) -> Result<Self, MmioError> {
        let range = PageRange::new(physical_address, len)?;
        let mut mapper = unsafe {
            OffsetPageTable::new(
                &mut *(phys_to_virt(Cr3::read().0.start_address().as_u64()) as *mut PageTable),
                VirtAddr::new(phys_to_virt(0)),
            )
        };
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_CACHE;
        for page_index in 0..range.pages {
            let virtual_address = phys_to_virt(range.base + page_index as u64 * PAGE_SIZE as u64);
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virtual_address));
            let result = unsafe { mapper.update_flags(page, flags) };
            match result {
                Ok(flush) => flush.flush(),
                // Limine commonly maps the HHDM with 2 MiB pages. A 4 KiB
                // flag update cannot split such a mapping, so mark the
                // containing huge page uncached instead.
                Err(FlagUpdateError::ParentEntryHugePage) => {
                    let huge = Page::<Size2MiB>::containing_address(VirtAddr::new(virtual_address));
                    unsafe { mapper.update_flags(huge, flags) }
                        .map_err(|_| MmioError::MappingMissing)?
                        .flush();
                }
                Err(FlagUpdateError::PageNotMapped) => {
                    let physical_address = range.base + page_index as u64 * PAGE_SIZE as u64;
                    unsafe {
                        mapper
                            .map_to(
                                page,
                                PhysFrame::containing_address(PhysAddr::new(physical_address)),
                                flags,
                                &mut DLOSFrameAllocator,
                            )
                            .map_err(|_| MmioError::MappingMissing)?
                            .flush();
                    }
                }
            }
        }
        let virtual_address = NonNull::new(phys_to_virt(physical_address) as *mut u8)
            .expect("HHDM virtual address must not be null");
        Ok(Self {
            physical_address,
            virtual_address,
            len,
            page_base: range.base,
            pages: range.pages,
        })
    }

    pub fn physical_address(&self) -> u64 {
        self.physical_address
    }

    pub fn virtual_address(&self) -> u64 {
        self.virtual_address.as_ptr() as u64
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn pages(&self) -> usize {
        self.pages
    }

    pub fn page_base(&self) -> u64 {
        self.page_base
    }

    pub fn as_ptr(&self) -> *mut u8 {
        self.virtual_address.as_ptr()
    }
}

pub fn test() {
    assert_eq!(
        PageRange::new(0x1234, PAGE_SIZE).unwrap(),
        PageRange {
            base: 0x1000,
            pages: 2,
            offset: 0x234,
        }
    );
    assert_eq!(PageRange::new(0, 0), Err(MmioError::Empty));
    assert_eq!(
        PageRange::new(u64::MAX - 1, PAGE_SIZE),
        Err(MmioError::AddressOverflow)
    );
    assert!(matches!(DmaBuffer::new(0, PAGE_SIZE), Err(DmaError::Empty)));
    assert!(matches!(
        DmaBuffer::new(PAGE_SIZE, 3),
        Err(DmaError::InvalidAlignment)
    ));

    let alignment = PAGE_SIZE * 2;
    let dma =
        DmaBuffer::new(PAGE_SIZE + 1, alignment).expect("mm: DMA self-test allocation failed");
    assert_eq!(dma.pages(), 2);
    assert_eq!(dma.len(), 2 * PAGE_SIZE);
    assert_eq!(dma.physical_address() as usize % alignment, 0);
    assert!(
        unsafe { core::slice::from_raw_parts(dma.as_ptr(), dma.len()) }
            .iter()
            .all(|byte| *byte == 0)
    );
    let physical_address = dma.physical_address();
    drop(dma);

    let replacement =
        DmaBuffer::new(PAGE_SIZE + 1, alignment).expect("mm: DMA self-test re-allocation failed");
    assert_eq!(replacement.physical_address(), physical_address);
    drop(replacement);
    crate::println!("[INFO] mm: DMA/MMIO self-test passed");
}
