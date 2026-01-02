use core::alloc::Layout;

use alloc::alloc::{alloc, dealloc};
use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use nvme::{Allocator, Device, IoQueuePair, Namespace};
use spin::{Lazy, Mutex};
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{
    Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB, Translate,
};
use x86_64::{PhysAddr, VirtAddr};

use crate::mm::page_alloc::{dealloc_physical_page, find_continuous_mem, DLOSFrameAllocator};
use crate::mm::phys_to_virt;
use crate::pcie::enumrate::doit;

type SharedNvmeDevice = Arc<Mutex<Device<NvmeAllocator>>>;
type LockedQueuePair = Mutex<IoQueuePair<NvmeAllocator>>;

pub struct NvmeAllocator;

impl Allocator for NvmeAllocator {
    unsafe fn allocate(&self, size: usize) -> usize {
        // println!("[DEBUG] NvmeAllocator got a request of {size} bytes");
        let res = phys_to_virt(find_continuous_mem(size.div_ceil(4096))) as usize;
        (res as *mut u8).write_bytes(0, 4096);
        res
    }

    unsafe fn deallocate(&self, addr: usize) {
        dealloc_physical_page(addr as u64 - phys_to_virt(0));
    }

    fn translate(&self, addr: usize) -> usize {
        let pgt = unsafe {
            OffsetPageTable::new(
                &mut *(phys_to_virt(Cr3::read().0.start_address().as_u64()) as *mut PageTable),
                VirtAddr::new(crate::mm::phys_to_virt(0)),
            )
        };
        pgt.translate_addr(VirtAddr::new(addr as u64))
            .unwrap()
            .as_u64() as usize
    }
}

#[derive(Clone)]
pub struct NvmeBlockDevice {
    pub namespace: Namespace,
    pub qpairs: BTreeMap<u16, Arc<LockedQueuePair>>,
    pub model_number: alloc::string::String,
    cur_pos: usize,
}

impl fatfs::IoBase for NvmeBlockDevice {
    type Error = ();
}

impl fatfs::Read for NvmeBlockDevice {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let t = self.qpairs.first_entry().ok_or(())?;
        let mut qp = t.get().lock();
        let block_size = self.namespace.block_size() as usize;
        let size = buf.len();
        let sector = self.cur_pos / block_size;
        let buf2 =
            unsafe { alloc(Layout::from_size_align(block_size, block_size).map_err(|_| ())?) };
        qp.read(buf2, block_size, sector as u64).map_err(|_| ())?;
        let (t1, t2) = match self.cur_pos % block_size {
            0 => (block_size, 0),
            o => (o, block_size - o),
        };
        let will_read = core::cmp::min(size, t1);
        let buf2_slice = unsafe { core::slice::from_raw_parts(buf2, block_size) };
        buf[..will_read].copy_from_slice(&buf2_slice[t2..(t2 + will_read)]);
        unsafe {
            dealloc(
                buf2,
                Layout::from_size_align(block_size, block_size).map_err(|_| ())?,
            );
        }
        Ok(will_read)
    }
}

impl fatfs::Write for NvmeBlockDevice {
    fn write(&mut self, _buf: &[u8]) -> Result<usize, Self::Error> {
        Err(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl fatfs::Seek for NvmeBlockDevice {
    fn seek(&mut self, pos: fatfs::SeekFrom) -> Result<u64, Self::Error> {
        let new_pos = match pos {
            fatfs::SeekFrom::Start(offset) => offset as i64,
            fatfs::SeekFrom::End(offset) => {
                (self.namespace.block_count() as usize * self.namespace.block_size() as usize)
                    as i64
                    + offset
            }
            fatfs::SeekFrom::Current(offset) => self.cur_pos as i64 + offset,
        };
        if new_pos < 0
            || new_pos
                > (self.namespace.block_count() as usize * self.namespace.block_size() as usize)
                    as i64
        {
            Err(())
        } else {
            self.cur_pos = new_pos as usize;
            Ok(self.cur_pos as u64)
        }
    }
}

impl crate::vfs::VfsFile for NvmeBlockDevice {
    fn size(&mut self) -> usize {
        self.namespace.block_count() as usize * self.namespace.block_size() as usize
    }

    fn read(&mut self, buf: &mut [u8]) -> usize {
        <Self as fatfs::Read>::read(self, buf).unwrap()
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        <Self as fatfs::Write>::write(self, buf).unwrap()
    }

    fn seek(&mut self, pos: crate::vfs::SeekFrom) -> usize {
        <Self as fatfs::Seek>::seek(
            self,
            match pos {
                crate::vfs::SeekFrom::End(x) => fatfs::SeekFrom::End(x as i64),
                crate::vfs::SeekFrom::Current(x) => fatfs::SeekFrom::Current(x as i64),
                crate::vfs::SeekFrom::Start(x) => fatfs::SeekFrom::Start(x as u64),
            },
        )
        .unwrap() as usize
    }
}

pub struct NvmeManager(Vec<SharedNvmeDevice>);

impl NvmeManager {
    pub fn iter(&self) -> impl Iterator<Item = Vec<NvmeBlockDevice>> + use<'_> {
        self.0.iter().map(|device| {
            let mut controller = device.lock();
            let namespaces = controller.identify_namespaces(0).unwrap();

            let mapper = |namespace: Namespace| {
                // Some(NvmeBlockDevice {
                //     namespace,
                //     qpairs: BTreeMap::new(),
                // })

                let qpair = controller
                    .create_io_queue_pair(namespace.clone(), 64)
                    .ok()?;

                Some(NvmeBlockDevice {
                    namespace,
                    qpairs: BTreeMap::from([(*qpair.id(), Arc::new(Mutex::new(qpair)))]),
                    model_number: controller.controller_data().model_number.clone(),
                    cur_pos: 0,
                })
            };

            namespaces.into_iter().filter_map(mapper).collect()
        })
    }
}

pub static NVME: Lazy<NvmeManager> = Lazy::new(|| {
    // crate::println!("[INFO] nvme: initializer called");

    let mut connections = Vec::new();

    doit(|_, _, _, config| {
        if config.class_code == 1 && config.subclass == 8 {
            let physical_address =
                (config.bar[0] & 0xfffffff0u32) as u64 + ((config.bar[1] as u64) << 32);
            let virtual_address = phys_to_virt(physical_address);
            let mut pgt = unsafe {
                OffsetPageTable::new(
                    &mut *(phys_to_virt(Cr3::read().0.start_address().as_u64()) as *mut PageTable),
                    VirtAddr::new(crate::mm::phys_to_virt(0)),
                )
            };
            for pg in 0..32 {
                unsafe {
                    let _ = pgt
                        .map_to(
                            Page::<Size4KiB>::containing_address(VirtAddr::new(
                                virtual_address + pg * 4096,
                            )),
                            PhysFrame::containing_address(PhysAddr::new(
                                physical_address + pg * 4096,
                            )),
                            PageTableFlags::WRITABLE
                                | PageTableFlags::PRESENT
                                | PageTableFlags::NO_CACHE,
                            &mut DLOSFrameAllocator,
                        )
                        .map(|f| f.flush());
                }
            }

            let virtual_address = virtual_address as usize;
            let device = Device::init(virtual_address, NvmeAllocator).unwrap();
            connections.push(Arc::new(Mutex::new(device)));
        }
    });

    // crate::println!("[INFO] nvme: initializer returned");

    NvmeManager(connections)
});

pub fn init() {
    for device in NVME.iter() {
        for namespace in device.iter() {
            let res = crate::mm::convert_unit(
                namespace.namespace.block_count() * namespace.namespace.block_size(),
            );
            crate::println!(
                "[INFO] blockdev: nvme: found {}, size = {} {}",
                namespace.model_number,
                res.0,
                res.1
            );
        }
    }
}
