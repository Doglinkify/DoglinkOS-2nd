use x86_64::VirtAddr;

use crate::{
    mm::{page_alloc::dealloc_continuous_mem, phys_to_virt},
    task::process::ProcessContext,
};

pub(super) const KERNEL_STACK_SIZE: usize = 16; // 64 KiB

pub(super) struct KernelStack {
    start_paddr: u64,
    pub current_vaddr: VirtAddr,
}

impl KernelStack {
    pub(super) fn new() -> Self {
        let start_paddr = crate::mm::page_alloc::find_continuous_mem(KERNEL_STACK_SIZE)
            .expect("task: unable to reserve kernel stack");
        Self {
            start_paddr,
            current_vaddr: VirtAddr::new(phys_to_virt(
                start_paddr + (KERNEL_STACK_SIZE << 12) as u64,
            )),
        }
    }

    pub(super) fn fake(context: &ProcessContext) -> Self {
        let mut tmp = Self::new();

        #[repr(C)]
        pub struct OpaqueSymbol {
            _data: [u8; 0],
            _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
        }

        unsafe extern "C" {
            pub static __asm_syscall_ret: OpaqueSymbol;
        }

        unsafe {
            let mut p = tmp.current_vaddr.as_mut_ptr::<u8>();
            // push context
            p = p.sub(size_of::<ProcessContext>());
            *p.cast() = *context;
            // push fake caller (__asm_syscall_ret)
            p = p.sub(size_of::<VirtAddr>());
            *p.cast() = &raw const __asm_syscall_ret;
            // push placeholder for switch_to_asm
            for _ in 0..6 {
                p = p.sub(size_of::<u64>());
                *p.cast() = 0u64;
            }
            tmp.current_vaddr = VirtAddr::from_ptr(p);
        }

        tmp
    }

    pub(super) fn get_top(&self) -> VirtAddr {
        VirtAddr::new(phys_to_virt(
            self.start_paddr + (KERNEL_STACK_SIZE << 12) as u64,
        ))
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        dealloc_continuous_mem(self.start_paddr, KERNEL_STACK_SIZE);
    }
}
