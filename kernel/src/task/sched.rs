use core::sync::atomic::{AtomicUsize, Ordering};
use spin::lazy::Lazy;
use spin::mutex::Mutex;
use x86_64::PhysAddr;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::PhysFrame;

use crate::task::process::TASKS;
use crate::task::sched::cfs::Cfs;
use crate::task::sched::cfs::WEIGHT_NORMAL;

use super::process::ProcessContext;
use super::process::ProcessState;
use super::process::WaitReason;

pub static CURRENT_TASK_ID: AtomicUsize = AtomicUsize::new(0);
pub static TOTAL_TICKS: AtomicUsize = AtomicUsize::new(0);
const IDLE_TASK_ID: usize = 0;

pub fn block_current(context: &mut ProcessContext, reason: WaitReason) {
    let current = CURRENT_TASK_ID.load(Ordering::Relaxed);
    {
        let mut tasks = super::process::TASKS.lock();
        if let Some(task) = tasks[current].as_mut() {
            task.state = ProcessState::Blocked(reason);
        }
    }
    del_process(current);
    schedule(context, false);
}

fn check_and_unblock() {
    let mut tasks = TASKS.lock();
    for tid in 0..tasks.len() {
        if tasks[tid].is_some() {
            let should_unblock = matches!(
                tasks[tid].as_ref().unwrap().state,
                ProcessState::Blocked(WaitReason::WaitPid(target_pid)) if tasks[target_pid].is_none()
            );
            if should_unblock {
                tasks[tid].as_mut().unwrap().state = ProcessState::Runnable;
                add_process(tid);
            }
        }
    }
}

fn switch_to(
    context: &mut super::process::ProcessContext,
    next: usize,
    current_process_exited: bool,
) {
    let current = CURRENT_TASK_ID.load(Ordering::Relaxed);
    // crate::println!("scheduler: switching from {current} to {next}");
    let flags = Cr3::read().1;
    let new_cr3_va;
    {
        let tasks = super::process::TASKS.lock();
        new_cr3_va = tasks[next].as_ref().unwrap().page_table.level_4_table() as *const _ as u64;
    }
    let new_cr3 =
        PhysFrame::from_start_address(PhysAddr::new(new_cr3_va - crate::mm::phys_to_virt(0)))
            .unwrap();
    unsafe {
        Cr3::write(new_cr3, flags);
    }
    {
        let mut tasks = super::process::TASKS.lock();
        if !current_process_exited {
            let cur = tasks[current].as_mut().unwrap();
            cur.context = *context;
            cur.fs = x86_64::registers::model_specific::FsBase::read();
            unsafe {
                core::arch::x86_64::_fxsave64((&mut cur.fpu_state) as *mut _ as *mut u8);
            }
        }
        let nxt = tasks[next].as_ref().unwrap();
        *context = nxt.context;
        x86_64::registers::model_specific::FsBase::write(nxt.fs);
        unsafe {
            core::arch::x86_64::_fxrstor64((&nxt.fpu_state) as *const _ as *const u8);
        }
        CURRENT_TASK_ID.store(next, Ordering::Relaxed);
    }
}

pub(crate) extern "C" fn timer(context: *mut super::process::ProcessContext) {
    crate::apic::local::eoi();
    x86_64::instructions::interrupts::disable();
    check_and_unblock();
    schedule(unsafe { &mut *context }, false);
    TOTAL_TICKS.fetch_add(1, Ordering::Relaxed);
    x86_64::instructions::interrupts::enable();
}

static GLOBAL_SCHEDULER: Lazy<Mutex<Cfs>> = Lazy::new(|| Mutex::new(Cfs::new()));

pub fn add_process(tid: usize) {
    GLOBAL_SCHEDULER.lock().enqueue(tid, WEIGHT_NORMAL);
}

pub fn del_process(tid: usize) {
    GLOBAL_SCHEDULER.lock().dequeue(tid);
}

pub fn schedule(context: &mut super::process::ProcessContext, current_process_exited: bool) {
    let current_tid = CURRENT_TASK_ID.load(Ordering::Relaxed);
    let mut global_scheduler = GLOBAL_SCHEDULER.lock();
    if !current_process_exited {
        let tasks = TASKS.lock();
        if tasks[current_tid].as_ref().unwrap().state == ProcessState::Runnable {
            global_scheduler.update_vruntime(current_tid, 10_000_000);
        }
    }
    if let Some(next) = global_scheduler.pick_next() {
        if next.id != current_tid {
            drop(global_scheduler);
            switch_to(context, next.id, current_process_exited);
        } else {
            global_scheduler.update_vruntime(next.id, 0);
        }
    } else {
        drop(global_scheduler);
        switch_to(context, IDLE_TASK_ID, current_process_exited);
    }
}

#[allow(unused)]
mod cfs {
    use alloc::collections::{BTreeMap, BTreeSet};
    use core::cmp::Ordering;

    use crate::println;

    pub type TaskId = usize;

    /// Reference weight (equivalent to Linux's NICE_0_LOAD).
    pub const WEIGHT_NORMAL: u32 = 1024;

    #[derive(Clone, Debug)]
    pub struct Task {
        pub id: TaskId,
        pub vruntime: u64,
        pub weight: u32,
    }

    impl Task {
        #[inline]
        fn sort_key(&self) -> (u64, TaskId) {
            (self.vruntime, self.id)
        }
    }

    // Equality is by id — a task is identified by its id.
    impl PartialEq for Task {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id
        }
    }
    impl Eq for Task {}

    // Ordering is by (vruntime, id) so the BTreeSet keeps the runqueue sorted.
    impl PartialOrd for Task {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Task {
        fn cmp(&self, other: &Self) -> Ordering {
            self.sort_key().cmp(&other.sort_key())
        }
    }

    /// A Completely Fair Scheduler.
    ///
    /// Usage pattern per scheduling round:
    ///   1. `pick_next()`       -> removes the min-vruntime task (it is now "running").
    ///   2. task runs for `delta_ns`.
    ///   3. `update_vruntime(id, delta_ns)` -> requeues it with advanced vruntime.
    ///
    /// To remove a task entirely (sleep/exit), call `dequeue(id)`.
    pub struct Cfs {
        /// Ordered runqueue.
        runqueue: BTreeSet<Task>,
        /// Id -> current Task snapshot (source of truth for per-task state).
        by_id: BTreeMap<TaskId, Task>,
        /// Monotonic floor used when enqueuing new tasks so they don't get
        /// starved or unfairly boosted.
        min_vruntime: u64,
    }

    impl Cfs {
        pub fn new() -> Self {
            Self {
                runqueue: BTreeSet::new(),
                by_id: BTreeMap::new(),
                min_vruntime: 0,
            }
        }

        /// Enqueue a new task. Its vruntime is clamped to `min_vruntime` so a
        /// late-joining task doesn't get punished or rewarded unfairly.
        pub fn enqueue(&mut self, id: TaskId, weight: u32) {
            if self.by_id.contains_key(&id) {
                return;
            }
            let task = Task {
                id,
                vruntime: self.min_vruntime,
                weight: weight.max(1),
            };
            self.by_id.insert(id, task.clone());
            self.runqueue.insert(task);
        }

        /// Remove a task entirely (e.g. it blocked or exited).
        pub fn dequeue(&mut self, id: TaskId) -> Option<Task> {
            let task = self.by_id.remove(&id)?;
            self.runqueue.remove(&task);
            self.refresh_min_vruntime();
            Some(task)
        }

        pub fn contains(&self, id: TaskId) -> bool {
            self.by_id.contains_key(&id)
        }

        pub fn len(&self) -> usize {
            self.runqueue.len()
        }

        pub fn is_empty(&self) -> bool {
            self.runqueue.is_empty()
        }

        /// Peek at the next task to run without removing it.
        pub fn peek_next(&self) -> Option<&Task> {
            self.runqueue.iter().next()
        }

        /// Pick the next task to run, removing it from the runqueue.
        /// The task remains in the id-index until `dequeue` or `update_vruntime`
        /// requeues it.
        pub fn pick_next(&mut self) -> Option<Task> {
            self.runqueue.pop_first()
        }

        /// Advance a task's vruntime after it has run for `delta_ns` and requeue it.
        ///
        /// `vruntime += delta_ns * (WEIGHT_NORMAL / weight)`
        ///
        /// Returns the new vruntime, or `None` if the task is unknown.
        pub fn update_vruntime(&mut self, id: TaskId, delta_ns: u64) -> Option<u64> {
            let mut task = self.by_id.get(&id)?.clone();

            // Remove old entry (no-op if it was picked and already absent).
            self.runqueue.remove(&task);

            // Scale by inverse weight using 128-bit math to avoid overflow.
            let scaled = (delta_ns as u128)
                .checked_mul(WEIGHT_NORMAL as u128)?
                .checked_div(task.weight.max(1) as u128)? as u64;
            task.vruntime = task.vruntime.saturating_add(scaled);

            // min_vruntime only ever moves forward.
            if let Some(first) = self.runqueue.iter().next() {
                self.min_vruntime = self.min_vruntime.max(first.vruntime);
            } else {
                self.min_vruntime = self.min_vruntime.max(task.vruntime);
            }

            self.by_id.insert(id, task.clone());
            self.runqueue.insert(task.clone());
            Some(task.vruntime)
        }

        /// Change a task's weight (nice) without disturbing its vruntime.
        pub fn set_weight(&mut self, id: TaskId, weight: u32) -> Option<()> {
            let mut task = self.by_id.get(&id)?.clone();
            self.runqueue.remove(&task);
            task.weight = weight.max(1);
            self.by_id.insert(id, task.clone());
            self.runqueue.insert(task);
            Some(())
        }

        pub fn vruntime(&self, id: TaskId) -> Option<u64> {
            self.by_id.get(&id).map(|t| t.vruntime)
        }

        pub fn iter_ordered(&self) -> impl Iterator<Item = &Task> {
            self.runqueue.iter()
        }

        fn refresh_min_vruntime(&mut self) {
            self.min_vruntime = self
                .runqueue
                .iter()
                .next()
                .map(|t| t.vruntime)
                .unwrap_or(self.min_vruntime);
        }
    }

    impl Default for Cfs {
        fn default() -> Self {
            Self::new()
        }
    }
}
