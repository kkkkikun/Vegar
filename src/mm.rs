use axerrno::LinuxError;
use axhal::mem::VirtAddr;
use axhal::paging::MappingFlags;
use axhal::trap::{PAGE_FAULT, register_trap_handler};
use starry_api::imp::task::sys_exit_impl;
use starry_core::mm::is_accessing_user_memory;
use starry_core::task::current_process_data;

#[register_trap_handler(PAGE_FAULT)]
fn handle_page_fault(vaddr: VirtAddr, access_flags: MappingFlags, is_user: bool) -> bool {
    if !is_user && !is_accessing_user_memory() {
        warn!(
            "Page fault at {:#x}, access_flags: {:#x?}",
            vaddr, access_flags
        );
        return false;
    }

    if !current_process_data()
        .addr_space
        .lock()
        .handle_page_fault(vaddr, access_flags)
    {
        warn!(
            "{}: segmentation fault at {:#x}, exit!",
            axtask::current().id_name(),
            vaddr
        );
        // TODO: correct exit code and signal
        sys_exit_impl(LinuxError::EFAULT as _, false);
    }
    true
}
