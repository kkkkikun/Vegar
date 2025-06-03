use axerrno::LinuxResult;
use starry_core::task::current_process_data;
use syscall_trace::syscall_trace;

#[syscall_trace]
pub fn sys_brk(addr: usize) -> LinuxResult<isize> {
    let mut return_val: isize = current_process_data().get_heap_top() as isize;
    let heap_bottom = current_process_data().get_heap_bottom();
    if addr != 0 && addr >= heap_bottom && addr <= heap_bottom + axconfig::plat::USER_HEAP_SIZE {
        current_process_data().set_heap_top(addr);
        return_val = addr as isize;
    }
    Ok(return_val)
}
